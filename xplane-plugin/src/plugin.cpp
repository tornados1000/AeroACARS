// =============================================================================
// AeroACARS X-Plane Premium Plugin
// =============================================================================
//
// SDK: X-Plane Plugin SDK 4.3.0 (BSD-licensed, vendored under
//      third_party/XPSDK430/).
//
// Purpose: read a curated set of DataRefs at flight-loop frequency and
// forward them to the AeroACARS desktop client over a UDP loopback socket
// (port 49001 by default). Plus a one-shot "touchdown" event packet at the
// physical moment of wheel-runway contact (fnrml_gear edge) — captured with
// frame-perfect timing, no UDP-eviction race, no VSI-smoothing artifacts.
//
// Design constraints (NON-NEGOTIABLE — see xplane-plugin/README.md):
//
//   1. NEVER crash X-Plane.
//      - Every XPLMFindDataRef result is NULL-checked before use.
//      - All errors are caught + logged via XPLMDebugString, never propagated.
//      - No C++ exceptions cross the C-ABI plugin boundary (-fno-exceptions).
//
//   2. NEVER stall the flight loop.
//      - The flight-loop callback runs on X-Plane's render thread.
//      - We read ~15 DataRefs (microseconds), build a small JSON string
//        (microseconds), and call sendto() on a non-blocking UDP socket
//        (microseconds when the buffer's empty, ECONNREFUSED-ignored
//        when the client isn't listening).
//      - No filesystem I/O, no malloc inside the hot path.
//
//   3. NEVER persist state outside the plugin's address space.
//      - No file writes, no registry edits, no env-var tweaks.
//      - Plugin is purely read-only against X-Plane state.
//
//   4. CLEAN SHUTDOWN on plugin reload.
//      - XPluginStop unregisters the flight loop, closes the socket,
//        zeros every DataRef handle. A second XPluginStart afterwards
//        starts from a known-good slate.
//
// Wire format: line-delimited JSON over UDP. Every packet is a single line
// terminated with `\n`. Schema versioned via "v":1. See README.md §"Wire
// Format" for details.
// =============================================================================

#include <XPLM/XPLMDataAccess.h>
#include <XPLM/XPLMDefs.h>
#include <XPLM/XPLMProcessing.h>
#include <XPLM/XPLMUtilities.h>

#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>

#if IBM
    #include <winsock2.h>
    #include <ws2tcpip.h>
    using socket_t = SOCKET;
    constexpr socket_t INVALID_SOCK = INVALID_SOCKET;
    static inline void close_sock(socket_t s) { closesocket(s); }
    static inline int sock_err() { return WSAGetLastError(); }
#else
    #include <arpa/inet.h>
    #include <fcntl.h>
    #include <netinet/in.h>
    #include <sys/socket.h>
    #include <sys/types.h>
    #include <unistd.h>
    using socket_t = int;
    constexpr socket_t INVALID_SOCK = -1;
    static inline void close_sock(socket_t s) { close(s); }
    static inline int sock_err() { return errno; }
#endif

// =============================================================================
// Configuration constants (compile-time)
// =============================================================================

// UDP destination — loopback only. The AeroACARS desktop client binds this
// port and listens for our packets. Mismatched port = silent no-op (sendto
// gets ECONNREFUSED which we ignore), no crash.
static constexpr const char* AEROACARS_UDP_HOST = "127.0.0.1";
static constexpr uint16_t AEROACARS_UDP_PORT = 49001;

// Flight-loop callback interval (in seconds). Negative = "every N frames".
// We use 0.05 s (= 20 Hz) as the baseline — matches xgs Landing Speed
// Plugin's cadence. At low AGL the touchdown sampler tightens to per-frame
// (-1.0f) for sub-frame accuracy at the moment of contact.
static constexpr float FLIGHT_LOOP_BASE_INTERVAL_S = 0.05f;
static constexpr float FLIGHT_LOOP_FAST_INTERVAL = -1.0f;  // every frame
static constexpr float FAST_AGL_THRESHOLD_FT = 200.0f;

// Touchdown-edge threshold on `fnrml_gear` (Newtons). xgs uses != 0.0,
// we choose 1.0 N as a tiny safety margin against potential float-noise
// with absolutely no risk of missing real touchdowns (a 60-tonne airliner
// at the moment of wheel contact spikes to >> 100,000 N immediately).
static constexpr float GEAR_TOUCHDOWN_THRESHOLD_N = 1.0f;

// VS lookback window for capturing the descent peak just before contact —
// matches the AeroACARS-Rust-side sampler's window so the data semantics
// are identical between premium-mode and UDP-fallback-mode.
static constexpr int64_t VS_LOOKBACK_MS = 500;

// Plugin metadata — matches X-Plane's plugin browser UI.
static constexpr const char* PLUGIN_NAME = "AeroACARS Premium";
static constexpr const char* PLUGIN_SIG  = "com.aeroacars.xplane.premium";
static constexpr const char* PLUGIN_DESC =
    "Native frame-rate telemetry bridge for the AeroACARS desktop client. "
    "Optional companion to the standard UDP integration — no extra config "
    "needed when both are running.";

// =============================================================================
// Plugin globals (zero-initialized)
// =============================================================================
//
// All globals start at safe defaults. If XPluginStart fails partway through
// (e.g. socket creation fails, or we're on a system where the SDK headers
// don't match the installed X-Plane), the rest of the lifecycle still runs
// without crashing — we just don't send packets.

namespace {

// DataRef handles. NULL = not found = silently skip that field in packets.
struct DataRefs {
    XPLMDataRef latitude          = nullptr;  // sim/flightmodel/position/latitude (deg)
    XPLMDataRef longitude         = nullptr;  // sim/flightmodel/position/longitude (deg)
    XPLMDataRef agl_m             = nullptr;  // sim/flightmodel/position/y_agl (meters)
    XPLMDataRef vertical_velocity = nullptr;  // sim/flightmodel/position/local_vy (m/s, raw, no smoothing)
    XPLMDataRef gear_fnrml_n      = nullptr;  // sim/flightmodel/forces/fnrml_gear (Newtons)
    XPLMDataRef on_ground_any     = nullptr;  // sim/flightmodel/failures/onground_any (0/1)
    XPLMDataRef gforce_normal     = nullptr;  // sim/flightmodel2/misc/gforce_normal (g's)
    XPLMDataRef pitch_deg         = nullptr;  // sim/flightmodel/position/theta (deg)
    XPLMDataRef bank_deg          = nullptr;  // sim/flightmodel/position/phi (deg)
    XPLMDataRef heading_deg_true  = nullptr;  // sim/flightmodel/position/psi (deg)
    XPLMDataRef ias_kt            = nullptr;  // sim/cockpit2/gauges/indicators/airspeed_kts_pilot (kt)
    XPLMDataRef gs_ms             = nullptr;  // sim/flightmodel/position/groundspeed (m/s)
    XPLMDataRef sim_paused        = nullptr;  // sim/time/paused (0/1)
    XPLMDataRef sim_in_replay     = nullptr;  // sim/time/is_in_replay (0/1)
};

DataRefs g_drefs;

// UDP socket state.
socket_t g_sock = INVALID_SOCK;
sockaddr_in g_dest{};

// Per-tick state for touchdown detection.
//
// `prev_in_air` tracks the previous tick's "are we airborne" state, used
// to detect the false→true→false edge (in_air→on_ground transition).
// `touchdown_captured` is a one-shot guard so we only emit the touchdown
// event packet once per landing. Both reset to safe defaults when the
// plugin reloads.
bool prev_in_air = true;
bool touchdown_captured = false;

// Sequence counter for outgoing packets (monotonic). Resets on plugin
// reload, which is acceptable — the client uses it for diagnostics
// (gap detection) only, not for ordering.
uint32_t g_seq = 0;

// VS lookback ring buffer. Stores the last N (timestamp, vs_fpm) pairs
// so when the touchdown edge fires we can find the peak descent VS in
// the last VS_LOOKBACK_MS milliseconds.
struct VSSample {
    double t_sec;     // X-Plane elapsed sim time (sim/time/total_running_time_sec)
    float vs_fpm;
    float pitch_deg;
};
constexpr size_t VS_BUFFER_CAP = 64;  // 64 samples × ~50ms = ~3.2s history
VSSample g_vs_buffer[VS_BUFFER_CAP];
size_t g_vs_buffer_head = 0;  // next write index (ring)
size_t g_vs_buffer_count = 0; // valid samples (caps at VS_BUFFER_CAP)

// =============================================================================
// Logging — XPLM-safe, never blocks
// =============================================================================
//
// X-Plane's Log.txt is the canonical log destination for plugins.
// XPLMDebugString() is the only thread-safe sync logger. We prefix every
// line with "[AeroACARS]" so we're easy to grep.

void log_msg(const char* msg) noexcept {
    if (!msg) return;
    char buf[1024];
    std::snprintf(buf, sizeof(buf), "[AeroACARS] %s\n", msg);
    XPLMDebugString(buf);
}

void log_msgf(const char* fmt, ...) noexcept {
    if (!fmt) return;
    char body[896];
    va_list ap;
    va_start(ap, fmt);
    std::vsnprintf(body, sizeof(body), fmt, ap);
    va_end(ap);
    char line[1024];
    std::snprintf(line, sizeof(line), "[AeroACARS] %s\n", body);
    XPLMDebugString(line);
}

// =============================================================================
// DataRef helpers (NULL-safe)
// =============================================================================

float read_float(XPLMDataRef ref, float fallback = 0.0f) noexcept {
    if (ref == nullptr) return fallback;
    return XPLMGetDataf(ref);
}

double read_double(XPLMDataRef ref, double fallback = 0.0) noexcept {
    if (ref == nullptr) return fallback;
    return XPLMGetDatad(ref);
}

int read_int(XPLMDataRef ref, int fallback = 0) noexcept {
    if (ref == nullptr) return fallback;
    return XPLMGetDatai(ref);
}

XPLMDataRef find_ref(const char* path) noexcept {
    XPLMDataRef ref = XPLMFindDataRef(path);
    if (ref == nullptr) {
        log_msgf("warn: DataRef not found: %s (will use fallback values)", path);
    }
    return ref;
}

// =============================================================================
// UDP transport — non-blocking sendto, errors silently ignored
// =============================================================================

void make_socket_nonblocking(socket_t s) noexcept {
#if IBM
    u_long mode = 1;
    ioctlsocket(s, FIONBIO, &mode);
#else
    int flags = fcntl(s, F_GETFL, 0);
    if (flags >= 0) fcntl(s, F_SETFL, flags | O_NONBLOCK);
#endif
}

bool open_socket() noexcept {
#if IBM
    WSADATA wsa{};
    if (WSAStartup(MAKEWORD(2, 2), &wsa) != 0) {
        log_msg("error: WSAStartup failed; UDP transport disabled");
        return false;
    }
#endif
    g_sock = ::socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP);
    if (g_sock == INVALID_SOCK) {
        log_msgf("error: socket() failed (errno=%d); UDP transport disabled", sock_err());
        return false;
    }
    make_socket_nonblocking(g_sock);

    g_dest = {};
    g_dest.sin_family = AF_INET;
    g_dest.sin_port = htons(AEROACARS_UDP_PORT);
    if (inet_pton(AF_INET, AEROACARS_UDP_HOST, &g_dest.sin_addr) != 1) {
        log_msgf("error: inet_pton failed for %s", AEROACARS_UDP_HOST);
        close_sock(g_sock);
        g_sock = INVALID_SOCK;
        return false;
    }
    log_msgf("UDP socket open: forwarding to %s:%u",
             AEROACARS_UDP_HOST, (unsigned)AEROACARS_UDP_PORT);
    return true;
}

void close_socket() noexcept {
    if (g_sock != INVALID_SOCK) {
        close_sock(g_sock);
        g_sock = INVALID_SOCK;
    }
#if IBM
    WSACleanup();
#endif
}

// Fire-and-forget UDP send. Failures are silently ignored — the client may
// not be running, and that's a normal state for us (pilot has X-Plane open
// but AeroACARS Tauri client closed). NEVER throws, NEVER blocks.
void send_packet(const char* payload, size_t len) noexcept {
    if (g_sock == INVALID_SOCK || payload == nullptr || len == 0) return;
    ::sendto(g_sock, payload, static_cast<int>(len), 0,
             reinterpret_cast<const sockaddr*>(&g_dest), sizeof(g_dest));
    // Intentionally no error-checking. ECONNREFUSED, EAGAIN, etc. are all
    // ignored — the next tick will try again with fresh data.
}

// =============================================================================
// JSON building — printf-style with bounds checking
// =============================================================================
//
// We deliberately avoid heap allocation in the flight loop. Each packet is
// built into a fixed 2 KB stack buffer; if for some reason we overflow,
// we truncate cleanly (vsnprintf is bounded). 2 KB is well above any
// realistic packet size — a fully-populated telemetry frame is ~600 bytes.

constexpr size_t PACKET_BUF_SIZE = 2048;

// Format a double cleanly into the JSON. NaN/Inf become null (per JSON
// spec — those values aren't representable). All numbers go via "%g"
// which is compact + readable.
int jw_double(char* buf, size_t cap, double v) noexcept {
    if (!std::isfinite(v)) {
        return std::snprintf(buf, cap, "null");
    }
    return std::snprintf(buf, cap, "%g", v);
}

// =============================================================================
// Flight-loop callback — the hot path
// =============================================================================
//
// Called by X-Plane every FLIGHT_LOOP_BASE_INTERVAL_S seconds (or every
// frame when low). MUST be fast — no I/O, no waiting, no malloc in here.
//
// Returns the seconds-until-next-call. We tighten the rate when at low
// AGL so the touchdown edge gets sub-frame resolution.

float flight_loop_cb(float, float, int, void*) noexcept {
    // Skip work entirely while the sim is paused or in replay — those
    // states give us frozen / weird telemetry that the AeroACARS client
    // wouldn't know how to interpret. Sim/replay-aware code is the
    // client's job, not the plugin's.
    if (read_int(g_drefs.sim_paused) != 0 || read_int(g_drefs.sim_in_replay) != 0) {
        return FLIGHT_LOOP_BASE_INTERVAL_S;
    }

    // -- Read DataRefs (NULL-safe, fast) ---------------------------------
    const double sim_t       = static_cast<double>(XPLMGetElapsedTime());
    const double lat         = read_double(g_drefs.latitude);
    const double lon         = read_double(g_drefs.longitude);
    const float  agl_m       = read_float(g_drefs.agl_m);
    const float  agl_ft      = agl_m * 3.28084f;
    const float  vy_ms       = read_float(g_drefs.vertical_velocity);
    const float  vs_fpm_raw  = vy_ms * 196.8504f;  // m/s → fpm
    const float  pitch_deg   = read_float(g_drefs.pitch_deg);
    const float  pitch_rad   = pitch_deg * 0.0174533f;
    const float  pitch_cos   = std::cos(pitch_rad);
    // Pitch-corrected VS (xgs convention) — cos(pitch) projects world-
    // vertical Y-velocity to the body-axial direction. For typical
    // touchdowns at 3-5° pitch this is a 0.1-0.4% adjustment; for
    // STOL-style 10° flares ~1.5%. Free accuracy.
    const float  vs_fpm      = vs_fpm_raw * pitch_cos;
    const float  fnrml_n     = read_float(g_drefs.gear_fnrml_n);
    const int    on_ground   = read_int(g_drefs.on_ground_any);
    const float  gnorm       = read_float(g_drefs.gforce_normal, 1.0f);
    const float  bank_deg    = read_float(g_drefs.bank_deg);
    const float  hdg_true    = read_float(g_drefs.heading_deg_true);
    const float  ias_kt      = read_float(g_drefs.ias_kt);
    const float  gs_ms       = read_float(g_drefs.gs_ms);
    const float  gs_kt       = gs_ms * 1.94384f;

    // -- Push to VS ring buffer (always, regardless of touchdown) --------
    {
        VSSample& slot = g_vs_buffer[g_vs_buffer_head];
        slot.t_sec     = sim_t;
        slot.vs_fpm    = vs_fpm;
        slot.pitch_deg = pitch_deg;
        g_vs_buffer_head = (g_vs_buffer_head + 1) % VS_BUFFER_CAP;
        if (g_vs_buffer_count < VS_BUFFER_CAP) g_vs_buffer_count++;
    }

    // -- Touchdown-edge detection (fnrml_gear-based) --------------------
    //
    // Definition: in_air = gear-normal-force below threshold. xgs uses
    // != 0; we use > 1 N as a tiny noise filter. Edge fires when we
    // transition from "in air" to "on ground" — and only ONCE per
    // landing (touchdown_captured guard). The guard clears when AGL
    // climbs above 50 ft so a go-around resets us cleanly.
    const bool in_air_now = (fnrml_n < GEAR_TOUCHDOWN_THRESHOLD_N);
    if (touchdown_captured && agl_ft > 50.0f) {
        // Got back airborne — re-arm.
        touchdown_captured = false;
    }
    const bool edge = prev_in_air && !in_air_now && !touchdown_captured;

    // -- Build + send the per-tick telemetry packet ----------------------
    {
        char buf[PACKET_BUF_SIZE];
        int n = std::snprintf(buf, sizeof(buf),
            "{"
            "\"v\":1,"
            "\"type\":\"telemetry\","
            "\"seq\":%u,"
            "\"ts\":%.6f,"
            "\"lat\":%.7f,"
            "\"lon\":%.7f,"
            "\"agl_ft\":%.2f,"
            "\"vs_fpm_raw\":%.2f,"
            "\"vs_fpm\":%.2f,"
            "\"fnrml_gear_n\":%.2f,"
            "\"on_ground\":%s,"
            "\"g_normal\":%.4f,"
            "\"pitch_deg\":%.3f,"
            "\"bank_deg\":%.3f,"
            "\"hdg_true\":%.3f,"
            "\"ias_kt\":%.2f,"
            "\"gs_kt\":%.2f"
            "}\n",
            ++g_seq,
            sim_t,
            lat, lon,
            static_cast<double>(agl_ft),
            static_cast<double>(vs_fpm_raw),
            static_cast<double>(vs_fpm),
            static_cast<double>(fnrml_n),
            on_ground != 0 ? "true" : "false",
            static_cast<double>(gnorm),
            static_cast<double>(pitch_deg),
            static_cast<double>(bank_deg),
            static_cast<double>(hdg_true),
            static_cast<double>(ias_kt),
            static_cast<double>(gs_kt));
        if (n > 0 && static_cast<size_t>(n) < sizeof(buf)) {
            send_packet(buf, static_cast<size_t>(n));
        }
    }

    // -- Touchdown event packet (one-shot, frame-perfect) ---------------
    if (edge) {
        // Find the most-negative VS in the lookback window. This is the
        // peak descent rate just before contact — the value the client
        // wants for "landing rate fpm".
        const double cutoff_t = sim_t - (VS_LOOKBACK_MS / 1000.0);
        float captured_vs = vs_fpm;
        for (size_t i = 0; i < g_vs_buffer_count; ++i) {
            const VSSample& s = g_vs_buffer[i];
            if (s.t_sec >= cutoff_t && s.vs_fpm < captured_vs) {
                captured_vs = s.vs_fpm;
            }
        }
        char buf[PACKET_BUF_SIZE];
        int n = std::snprintf(buf, sizeof(buf),
            "{"
            "\"v\":1,"
            "\"type\":\"touchdown\","
            "\"seq\":%u,"
            "\"ts\":%.6f,"
            "\"lat\":%.7f,"
            "\"lon\":%.7f,"
            "\"captured_vs_fpm\":%.2f,"
            "\"captured_g_normal\":%.4f,"
            "\"captured_pitch_deg\":%.3f,"
            "\"captured_bank_deg\":%.3f,"
            "\"captured_ias_kt\":%.2f,"
            "\"captured_gs_kt\":%.2f,"
            "\"captured_heading_deg\":%.3f,"
            "\"fnrml_gear_n\":%.2f,"
            "\"agl_ft\":%.2f"
            "}\n",
            ++g_seq,
            sim_t,
            lat, lon,
            static_cast<double>(captured_vs),
            static_cast<double>(gnorm),
            static_cast<double>(pitch_deg),
            static_cast<double>(bank_deg),
            static_cast<double>(ias_kt),
            static_cast<double>(gs_kt),
            static_cast<double>(hdg_true),
            static_cast<double>(fnrml_n),
            static_cast<double>(agl_ft));
        if (n > 0 && static_cast<size_t>(n) < sizeof(buf)) {
            send_packet(buf, static_cast<size_t>(n));
        }
        log_msgf("touchdown captured: vs=%.1f fpm  g=%.2f  ias=%.1f kt",
                 static_cast<double>(captured_vs),
                 static_cast<double>(gnorm),
                 static_cast<double>(ias_kt));
        touchdown_captured = true;
    }

    // -- Update edge state for next tick --------------------------------
    prev_in_air = in_air_now;

    // -- Adaptive interval: faster when close to ground -----------------
    if (agl_ft < FAST_AGL_THRESHOLD_FT) {
        return FLIGHT_LOOP_FAST_INTERVAL;  // every frame
    }
    return FLIGHT_LOOP_BASE_INTERVAL_S;
}

}  // namespace

// =============================================================================
// XPLM Plugin entry points
// =============================================================================
//
// These four callbacks form the entire X-Plane plugin contract. We use the
// stable XPLM extern-C exports + PLUGIN_API decorators per SDK convention.

PLUGIN_API int XPluginStart(char* outName, char* outSig, char* outDesc) {
    // Defensive: copy our metadata into the SDK's caller-owned buffers.
    // Format docs say these are 256-byte buffers; strncpy + force-null is
    // the canonical safe pattern.
    std::strncpy(outName, PLUGIN_NAME, 255); outName[255] = '\0';
    std::strncpy(outSig,  PLUGIN_SIG,  255); outSig[255]  = '\0';
    std::strncpy(outDesc, PLUGIN_DESC, 255); outDesc[255] = '\0';

    log_msgf("starting AeroACARS X-Plane Plugin v0.5.0 (SDK 4.3.0)");

    // Resolve all DataRef handles. NULL is acceptable for any of these
    // — the read_*-helpers fall back to a sensible default. We log
    // each warning so the pilot can see in Log.txt if their X-Plane
    // version is missing something we expect.
    g_drefs.latitude          = find_ref("sim/flightmodel/position/latitude");
    g_drefs.longitude         = find_ref("sim/flightmodel/position/longitude");
    g_drefs.agl_m             = find_ref("sim/flightmodel/position/y_agl");
    g_drefs.vertical_velocity = find_ref("sim/flightmodel/position/local_vy");
    g_drefs.gear_fnrml_n      = find_ref("sim/flightmodel/forces/fnrml_gear");
    g_drefs.on_ground_any     = find_ref("sim/flightmodel/failures/onground_any");
    g_drefs.gforce_normal     = find_ref("sim/flightmodel2/misc/gforce_normal");
    g_drefs.pitch_deg         = find_ref("sim/flightmodel/position/theta");
    g_drefs.bank_deg          = find_ref("sim/flightmodel/position/phi");
    g_drefs.heading_deg_true  = find_ref("sim/flightmodel/position/psi");
    g_drefs.ias_kt            = find_ref("sim/cockpit2/gauges/indicators/airspeed_kts_pilot");
    g_drefs.gs_ms             = find_ref("sim/flightmodel/position/groundspeed");
    g_drefs.sim_paused        = find_ref("sim/time/paused");
    g_drefs.sim_in_replay     = find_ref("sim/time/is_in_replay");

    // Open UDP socket. Failure here is non-fatal — we just won't send
    // packets, but the plugin still loads cleanly.
    if (!open_socket()) {
        log_msg("warn: UDP socket setup failed; plugin loaded but inert");
    }

    // Register the flight-loop callback. Returning 1 = plugin started OK.
    XPLMRegisterFlightLoopCallback(flight_loop_cb, FLIGHT_LOOP_BASE_INTERVAL_S, nullptr);

    log_msg("AeroACARS X-Plane Plugin started successfully");
    return 1;
}

PLUGIN_API void XPluginStop(void) {
    log_msg("stopping AeroACARS X-Plane Plugin");

    // Reverse-order cleanup:
    //   1. Unregister flight-loop callback FIRST so we stop sending.
    //   2. Close the socket.
    //   3. Zero DataRef handles (defensive — plugin reload will re-find).
    XPLMUnregisterFlightLoopCallback(flight_loop_cb, nullptr);
    close_socket();

    g_drefs = DataRefs{};
    g_vs_buffer_head = 0;
    g_vs_buffer_count = 0;
    prev_in_air = true;
    touchdown_captured = false;
    g_seq = 0;

    log_msg("AeroACARS X-Plane Plugin stopped cleanly");
}

PLUGIN_API int XPluginEnable(void) {
    // Nothing to do — we run continuously while loaded. X-Plane only
    // calls Disable/Enable on user request, not normally.
    return 1;
}

PLUGIN_API void XPluginDisable(void) {
    // Same — no-op. State stays valid until XPluginStop.
}

PLUGIN_API void XPluginReceiveMessage(XPLMPluginID, int, void*) {
    // We don't accept inter-plugin messages. Silent acknowledge is fine.
}
