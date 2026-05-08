//! AeroACARS — Tauri application root.
//!
//! Holds the active `api_client::Client` in shared state, exposes auth commands
//! to the UI (login, logout, session restore), and persists the site URL to a
//! per-user config dir. The API key itself is stored via `secrets` (OS keyring),
//! never on disk in plaintext.

mod discord;
mod runway;
mod xplane_plugin_install;

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use api_client::{
    Airport, ApiError, Bid, Client, Connection, FareEntry, FileBody, PositionEntry, PrefileBody,
    Profile, UpdateBody,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use metar::{MetarError, MetarSnapshot};
use recorder::{FlightLogEvent, FlightOutcome, FlightRecorder};
use storage::{
    ApproachSample, LandingProfilePoint, LandingRecord, LandingRunwayMatch, LandingStore,
    PositionQueue, QueuedPosition,
};
use sim_core::{FlightPhase, SimKind, SimSnapshot, Simulator};
use tauri::{AppHandle, Manager};
use tracing_subscriber::EnvFilter;

#[cfg(target_os = "windows")]
use sim_msfs::MsfsAdapter;

const KEYRING_ACCOUNT: &str = "primary";
const SITE_CONFIG_FILE: &str = "site.json";
const SIM_CONFIG_FILE: &str = "sim.json";
/// File holding the current in-progress flight, written on flight_start and
/// removed on flight_end / flight_cancel. Lets us resume after a client crash.
const ACTIVE_FLIGHT_FILE: &str = "active_flight.json";
/// Persisted activity-log dump — survives app restarts so the pilot
/// sees the full flight history when they re-open Tauri mid-flight.
/// Capped at `ACTIVITY_LOG_CAPACITY` entries (same as in-memory).
const ACTIVITY_LOG_FILE: &str = "activity_log.json";

/// Anything older than this is considered stale and discarded on resume.
const RESUME_MAX_AGE_HOURS: i64 = 12;

/// Phase-dependent position-post cadence. Ground manoeuvres (taxi, takeoff,
/// landing rollout) and the approach are where pilots want a precise trail
/// on the live map; cruise is straight-and-level so a sparse sample is fine
/// and saves API budget. Spec §10 ("configurable intervals") — these are
/// the defaults; a future settings panel can override.
/// v0.5.21: MQTT live-tracking publish interval. Constant 3 s
/// regardless of phase — gives the live-tracking server 10x more
/// position points in cruise (was 30 s) for smoother live maps and
/// finer post-flight analytics. Decoupled from phpVMS POST cadence
/// (which stays phase-aware via `position_interval`) so the phpVMS
/// DB doesn't get hammered.
///
/// Per-pilot bandwidth at 3 s: ~1.3 MB/h with WSS+TLS overhead;
/// for a 5 h flight that's less than 1 minute of YouTube traffic.
/// VPS handles it trivially even with 200 concurrent pilots.
const MQTT_PUBLISH_INTERVAL_SECS: u64 = 3;

fn position_interval(phase: FlightPhase) -> Duration {
    let secs = match phase {
        // Brief, critical events — sample a touch faster so the touchdown
        // point and the actual liftoff don't get smeared between two posts.
        FlightPhase::Takeoff | FlightPhase::Landing => 5,
        // Boarding + Pushback need fast ticks because a real pushback
        // can be over in 8–15 s; if the streamer only fires every 10 s
        // we miss the entire phase between two snapshots and the
        // dashboard jumps straight from Boarding to Taxi.
        FlightPhase::Boarding | FlightPhase::Pushback => 4,
        // On the ground (taxi, takeoff roll) — 10 s is plenty;
        // movements are slow and the live map just needs a clean trail.
        FlightPhase::TaxiOut | FlightPhase::TakeoffRoll | FlightPhase::TaxiIn => 10,
        // Approach / final — pilot wants the inbound track precise (ILS,
        // localizer, glideslope) without overdoing samples.
        FlightPhase::Approach | FlightPhase::Final => 8,
        // Climb / descent: moderate change rate.
        FlightPhase::Climb | FlightPhase::Descent => 10,
        // Cruise: long straight legs, sparse samples are enough — capped
        // at 30 s so the live map never goes more than half a minute stale.
        FlightPhase::Cruise => 30,
        // v0.5.11: Holding — same cadence as Cruise (slow track update,
        // we're circling at constant altitude).
        FlightPhase::Holding => 30,
        // Parked / pre-/post-flight — keep a 30 s heartbeat so phpVMS
        // sees the PIREP is alive while the pilot files.
        FlightPhase::Preflight
        | FlightPhase::BlocksOn
        | FlightPhase::Arrived
        | FlightPhase::PirepSubmitted => 30,
    };
    Duration::from_secs(secs)
}

/// Minimum great-circle distance between two consecutive samples before we
/// add it to the running total. Filters out GPS jitter while parked.
const DISTANCE_EPSILON_M: f64 = 5.0;

/// Kilograms → pounds. We collect fuel in kg internally because every
/// SimConnect adapter normalises to SI, but phpVMS-Core's `acars` table
/// and the PIREP `file` endpoint expect pounds — convert at the boundary.
const KG_TO_LB: f64 = 2.20462262;

/// Universal "we're done here" fallback for the FSM. Catches helicopters
/// (no taxi-out / taxi-in / parking-brake convention), short hops,
/// emergency landings near the destination, and anything else where the
/// normal Pushback → Cruise → BlocksOn chain doesn't fire cleanly.
///
/// When the aircraft is on-ground with engines off, sitting within
/// `ARRIVED_FALLBACK_RADIUS_NM` of the destination airport for at
/// least `ARRIVED_FALLBACK_DWELL_SECS`, the FSM jumps straight to
/// `Arrived` regardless of what phase it's currently in (apart from
/// pre-block-off phases — we won't accidentally end a flight that
/// hasn't started yet).
const ARRIVED_FALLBACK_RADIUS_NM: f64 = 2.0;
const ARRIVED_FALLBACK_DWELL_SECS: i64 = 30;

// ---- Touch-and-Go / Go-Around detection (Stage 3) ----
//
// These thresholds were chosen by working backwards from real-world
// pilot intent rather than from sim-specific telemetry shapes:
//
// * 100 ft AGL threshold for T&G — comfortably above gear-strut /
//   bounce-rebound height (which is at most ~40 ft on the worst
//   bounces), but low enough that a deliberate climb-out from a
//   training landing crosses it quickly.
// * 30 s observation window — even a slow GA-pattern aircraft
//   (172, J3 Cub) climbs past 100 ft in less than that.
// * 200 ft recovery for GA — a missed approach typically initiates
//   at ≥200 ft above the lowest observed altitude, and 200 ft is
//   the standard Cat I missed-approach decision height. Sustained
//   8 s prevents the detector firing on a single climb-correction.
// * +500 fpm V/S filter on GA — pilots really go around with positive
//   V/S, no false positives from a flare round-out.

/// Time after a touchdown during which we watch for the aircraft
/// climbing back out (= touch-and-go). After this window expires
/// the touchdown is finalised as a regular landing.
const TOUCH_AND_GO_WATCH_SECS: i64 = 30;
/// Aircraft must climb above this AGL after a touchdown to count
/// as a touch-and-go (not a tall bounce or runway-end pop-up).
const TOUCH_AND_GO_AGL_THRESHOLD_FT: f32 = 100.0;
/// Aircraft must be above the AGL threshold for this long for the
/// T&G to be confirmed — short spike doesn't count.
const TOUCH_AND_GO_DWELL_SECS: i64 = 1;
/// AGL increase from `lowest_agl_during_approach_ft` that signals a
/// possible go-around in progress.
const GO_AROUND_AGL_RECOVERY_FT: f32 = 200.0;

// ---- Holding-pattern detection (v0.5.11) ------------------------------
//
// A holding pattern is a circle at constant altitude. We detect it via
// sustained turn (banked) + level flight (low VS). Real ICAO holds:
//   * 1-min legs at <FL140, 1.5-min legs above → 4-6 min full circuit
//   * Standard 3°/sec turn rate = full 360° in 2 min
//   * Constant altitude (±100 ft)
//
// Entry threshold: 90 s of sustained bank > 15° + |VS| < 200 fpm.
// This filters out:
//   * 90° heading change at standard rate (~30 s) — won't trigger
//   * Procedure turn / teardrop entry (~60 s) — won't trigger
//   * Long ATC vector with sustained 30° bank (~2 min) — WILL trigger,
//     which is fine: the FSM simply shows "Holding" briefly and then
//     exits when bank levels off
//
// Exit threshold: 30 s of sustained bank < 5° (level flight). Or
// active descent (< -300 fpm + AGL < 5000) which means ATC cleared
// us out of the hold for the approach.
const HOLDING_ENTRY_DWELL_SECS: i64 = 90;
const HOLDING_EXIT_DWELL_SECS: i64 = 30;
const HOLDING_BANK_THRESHOLD_DEG: f32 = 15.0;
const HOLDING_VS_THRESHOLD_FPM: f32 = 200.0;
/// Sustained climb time required before we classify a Go-Around.
const GO_AROUND_DWELL_SECS: i64 = 8;
/// Minimum positive V/S at which the GA detector accepts a sample
/// as "actually climbing" (filters out flare round-out / level-off).
const GO_AROUND_MIN_VS_FPM: f32 = 500.0;
/// AGL threshold above which we mark the flight as having actually
/// flown. Powers the `was_airborne` gate that prevents both the
/// Arrived-fallback and the divert-detection from firing pre-flight
/// (GSX repositioning, ground-handling wackeln, etc.). 50 ft is well
/// above gear-strut oscillation and runway-end short-hop noise.
const WAS_AIRBORNE_AGL_FT: f32 = 50.0;
/// Upper bound on AGL we'll trust for the `was_airborne` mark. Live bug
/// 2026-05-03 (PMDG B738, GSX pushback): MSFS reported AGL=53819 ft
/// for ~60 s right after the SimConnect handshake while the terrain
/// engine was still loading — the aircraft was sitting on a stand
/// at EDDH but the SimVar said it was at FL538 with on_ground=false.
/// `was_airborne` flipped true from that loading glitch, and 30 min
/// later the universal Arrived-fallback (gated by was_airborne)
/// fired during the GSX pushback because pushback motion happens to
/// match the conditions. The fix is to ignore obvious garbage AGL
/// values: real pre-flight terrain data is bounded; nothing in the
/// real world makes a parked aircraft suddenly "airborne at FL538".
const WAS_AIRBORNE_AGL_MAX_FT: f32 = 30000.0;
/// Minimum number of consecutive ticks the airborne conditions must
/// hold before we flip `was_airborne`. Gates against single-tick
/// glitches where MSFS briefly reports !on_ground during scenery
/// load even with sane AGL. 2 ticks ≈ 5-10 s at the streamer cadence.
const WAS_AIRBORNE_DWELL_TICKS: u8 = 2;

/// If at the moment the FSM reaches Arrived the aircraft is farther
/// than this from the planned `arr_airport`, we treat it as a divert
/// candidate and surface a banner asking the pilot to confirm the
/// actual destination. Same threshold as ARRIVED_FALLBACK_RADIUS_NM
/// so we don't paint divert banners on perfectly-normal arrivals
/// where the FSM happened to fire from the slightly-larger fallback
/// instead of the strict on-block path.
const DIVERT_DETECT_RADIUS_NM: f64 = 2.0;
/// How far from the actual touchdown point we'll search the local
/// runways DB for a matching airport. 50 nmi covers any sensible
/// real-world divert (typical divert distances: 20-100 nmi). Larger
/// than 50 likely means we either missed the airport in our DB or
/// the pilot landed somewhere genuinely off-grid (private strip).
const DIVERT_NEAREST_SEARCH_RADIUS_NM: f64 = 50.0;

/// How often we POST `/pireps/{id}/update` purely to bump `pireps.updated_at`
/// and keep phpVMS's `RemoveExpiredLiveFlights` cron from soft-deleting
/// the in-flight PIREP. The cron runs hourly and looks at `updated_at`,
/// NOT at the latest position row — without this heartbeat, a long
/// cruise leg with no phase changes gets killed after `acars.live_time`
/// hours (default 2h on most installs). vmsACARS uses 30 s by default
/// (`acars_update_timer`); we match.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);

/// v0.4.1: nach wie vielen Sekunden ohne brauchbaren Sim-Snapshot
/// wir den Flug als „pausiert" markieren (Sim-Crash, Quit, Timeout).
/// 30 s ist ein guter Kompromiss: lang genug dass kurze FPS-Drops oder
/// Pause-Menü-Ausflüge keinen False-Positive triggern, kurz genug dass
/// der Pilot den Disconnect zeitnah mitkriegt und re-positionieren kann
/// bevor phpVMS' Live-Tracking-Cron (default ~2h) den PIREP killt.
const SIM_DISCONNECT_THRESHOLD_S: i64 = 30;

/// v0.4.1: Repositions-Distanz (in NM) ab der wir den Resume-Eintrag
/// mit WARN-Level statt INFO im Activity-Log loggen.
///
/// Schwelle absichtlich großzügig: 500 nm berücksichtigt legitime
/// Pilot-Workflows wie „Sim kracht mid-Atlantik, ich will die letzten
/// 4 h nicht nochmal fliegen, lade kurz vor Approach". Sub-500 nm ist
/// gängig und harmlos; > 500 nm fällt im VA-Audit zu Recht auf
/// (Teleport zur Destination, andere Welt-Hemisphäre, etc.).
const REPOSITION_WARN_DELTA_NM: f64 = 500.0;

/// How close (in nautical miles) the aircraft must be to the departure airport
/// to start the flight. Generous enough to cover taxi positions and remote
/// stands; tight enough to reject "I'm at EDDF instead of EDDP".
const MAX_START_DISTANCE_NM: f64 = 5.0;

/// Symmetric to MAX_START_DISTANCE_NM but applied at touchdown / file:
/// the pilot has to actually be on (or at) the planned arrival airport,
/// not somewhere random. Diverts are still possible via
/// `flight_end_manual` with a divert ICAO + reason. EDDP→EDDP plans are
/// fine — same airport, same check.
const MAX_FILE_DISTANCE_NM: f64 = 5.0;

/// MSFS often returns SimVar values as localization keys, not plain text.
/// The ATC MODEL var is one of them — e.g. `TT:ATCCOM.AC_MODEL_A320.0.text`
/// or `ATCCOM.AC_MODEL A320.0.text`. Pull out the readable code, or return
/// `None` if the input is an unresolved key we can't decode.
fn clean_atc_model(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    if let Some(start) = s.find("AC_MODEL") {
        let after = &s[start + "AC_MODEL".len()..];
        let after = after.trim_start_matches(|c: char| c == '_' || c == ' ');
        if let Some(end) = after.find('.') {
            let model = &after[..end];
            if !model.is_empty() {
                return Some(model.to_uppercase());
            }
        }
    }
    let upper = s.to_uppercase();
    if upper.starts_with("TT:") || upper.contains("ATCCOM.") || upper.ends_with(".TEXT") {
        return None;
    }
    Some(upper)
}

/// Loose check: does the aircraft title from MSFS appear to mention the given
/// ICAO code? Used as a permissive backup when ATC MODEL parses to one code
/// but the title says something completely different.
///
/// "Mention" includes the ICAO code AND any of its known long-form
/// aliases — e.g. for `icao = "A359"` the title "Airbus A350-900"
/// counts as a match. Without this, a pilot on a real GSG flight
/// (Emirates UAE770 EK770, A359 bid, A350-900 sim) got blocked
/// with "Aircraft mismatch" even though it's the same aircraft.
fn title_mentions_icao(title: &str, icao: &str) -> bool {
    let title_upper = title.to_uppercase();
    let icao_upper = icao.to_uppercase();
    if title_upper.contains(&icao_upper) {
        return true;
    }
    // Long-form aliases. If any alias is in the title, accept.
    aircraft_aliases(&icao_upper)
        .iter()
        .any(|alias| title_upper.contains(alias))
}

/// Bidirectional alias table for aircraft type identification.
/// phpVMS stores `aircraft.icao` as the ICAO 4-letter type code
/// (`A359`, `B738`, `B77W`), but MSFS's `ATC MODEL` and `TITLE`
/// SimVars often expose the marketing/long form (`A350-900`,
/// `737-800`, `777-300ER`). Both sides refer to the same airframe;
/// our match logic must accept either.
///
/// Returns the list of long-form aliases for the given ICAO code,
/// PLUS the inverse — if `icao` looks like a marketing form, the
/// aliases include the corresponding ICAO code so a sim that
/// reports the ICAO can still match a long-form title and vice
/// versa. Empty list = no known aliases (fall through to
/// strict-string comparison).
///
/// Live bug 2026-05-04: Emirates A359 bid blocked because sim
/// loaded "A350-900 (No Cabin)" — same aircraft, different name.
fn aircraft_aliases(code: &str) -> &'static [&'static str] {
    // Match against the uppercased input. Long-form aliases are
    // partial substrings (we only need ONE to be present in the
    // sim title) — so "A350-900" alone catches "Airbus A350-900",
    // "A350-900 No Cabin", "Asobo A350-900", etc.
    match code {
        // ---- Airbus ----
        // A220 family
        "BCS1" => &["A220-100", "CS100"],
        "BCS3" => &["A220-300", "CS300"],
        // A320 family
        "A318" => &["A318"],
        "A319" => &["A319"],
        "A20N" => &["A320NEO", "A320-NEO", "A320 NEO"],
        "A320" => &["A320"],
        "A21N" => &["A321NEO", "A321-NEO", "A321 NEO"],
        "A321" => &["A321"],
        // A330 family
        "A332" => &["A330-200"],
        "A333" => &["A330-300"],
        "A338" => &["A330-800"],
        "A339" => &["A330-900"],
        // A340 family
        "A342" => &["A340-200"],
        "A343" => &["A340-300"],
        "A345" => &["A340-500"],
        "A346" => &["A340-600"],
        // A350 family — the bug
        "A359" => &["A350-900", "A350"],
        "A35K" => &["A350-1000"],
        // A380
        "A388" => &["A380-800", "A380"],

        // ---- Boeing ----
        // 717
        "B712" => &["717-200", "717"],
        // 737 NG family
        "B736" => &["737-600"],
        "B737" => &["737-700"],
        "B738" => &["737-800"],
        "B739" => &["737-900"],
        // 737 MAX
        "B37M" => &["737 MAX 7", "737-7", "737MAX-7"],
        "B38M" => &["737 MAX 8", "737-8", "737MAX-8"],
        "B39M" => &["737 MAX 9", "737-9", "737MAX-9"],
        "B3XM" => &["737 MAX 10", "737-10"],
        // 747
        "B741" => &["747-100"],
        "B742" => &["747-200"],
        "B744" => &["747-400"],
        "B748" => &["747-8"],
        // 757
        "B752" => &["757-200"],
        "B753" => &["757-300"],
        // 767
        "B762" => &["767-200"],
        "B763" => &["767-300"],
        "B764" => &["767-400"],
        // 777
        "B772" => &["777-200"],
        "B77L" => &["777-200LR", "777-200 LR"],
        "B773" => &["777-300"],
        "B77W" => &["777-300ER", "777-300 ER"],
        "B77F" => &["777F", "777-200F"],
        // 787
        "B788" => &["787-8"],
        "B789" => &["787-9"],
        "B78X" => &["787-10"],

        // ---- Embraer ----
        "E170" => &["170"],
        "E175" => &["175"],
        "E190" => &["190"],
        "E195" => &["195"],
        "E290" => &["E190-E2", "E2-190"],
        "E295" => &["E195-E2", "E2-195"],

        // No alias known — fall through to strict comparison.
        _ => &[],
    }
}

/// Symmetric: do these two aircraft type strings refer to the
/// same airframe? Tries both directions: are `actual`'s long-form
/// aliases mentioned by `expected`? And are `expected`'s aliases
/// mentioned by `actual`? Plus strict equality fallback.
fn aircraft_types_match(expected: &str, actual: &str) -> bool {
    let exp = expected.to_uppercase();
    let act = actual.to_uppercase();
    if exp == act {
        return true;
    }
    // Either side might be the short ICAO form, the other the
    // long marketing form. Check both directions.
    if aircraft_aliases(&exp).iter().any(|alias| act.contains(alias))
        || aircraft_aliases(&act).iter().any(|alias| exp.contains(alias))
    {
        return true;
    }
    false
}

/// Shared application state — wraps the currently-authenticated client (if any)
/// and (on Windows) the MSFS adapter.
/// Cap on retained activity-log entries. The frontend displays the most
/// recent N — older lines fall off so the buffer doesn't grow forever
/// during a long flight (~5 h cruise = ~600 phase-tagged position posts
/// in the log; 1000 leaves comfortable headroom for transitions, errors
/// and bid-list updates on top).
const ACTIVITY_LOG_CAPACITY: usize = 1000;

/// Severity of an activity-log entry. Drives the frontend's icon/colour.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ActivityLevel {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ActivityEntry {
    timestamp: DateTime<Utc>,
    level: ActivityLevel,
    /// Stable English label for the event (e.g. "PIREP prefiled").
    /// Frontend may translate via i18n if a key matches; otherwise
    /// renders as-is.
    message: String,
    /// Optional context — flight phase, PIREP id, error code, etc.
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

#[derive(Default)]
struct AppState {
    client: Mutex<Option<Client>>,
    #[cfg(target_os = "windows")]
    msfs: Mutex<MsfsAdapter>,
    /// X-Plane adapter — cross-platform (UDP). Co-exists with the
    /// MSFS adapter; only one is `started` at a time, dictated by
    /// the persisted `SimKind`.
    xplane: Mutex<sim_xplane::XPlaneAdapter>,
    active_flight: Mutex<Option<Arc<ActiveFlight>>>,
    /// Ring buffer of pilot-visible activity events. Surfaced via the
    /// `activity_log_get` Tauri command; the dashboard renders them in
    /// the new "ACARS-Log" tab — same idea as the smartcars activity
    /// feed (login → bids loaded → prefiled → boarding → …).
    activity_log: Mutex<VecDeque<ActivityEntry>>,
    /// Atomic guard for `flight_start` / `flight_adopt`. Both functions await
    /// network calls *between* checking that no flight is active and writing
    /// the new ActiveFlight into state. Without this guard, two concurrent
    /// invokes (StrictMode double-mount, double-click, resume-banner re-render)
    /// would both pass the initial check and the second would silently
    /// overwrite the first — losing the streamer reference and leaving
    /// phpVMS with two adopt attempts. Acquire with `compare_exchange`,
    /// release in *every* exit path (success and error).
    flight_setup_in_progress: AtomicBool,
    /// In-process airport-coords cache. Keyed by ICAO uppercase. Populated on
    /// first lookup so we don't re-fetch on every snapshot tick.
    airports: Mutex<HashMap<String, Airport>>,
    /// Auto-start watcher state. When true, a background task polls
    /// `current_snapshot()` and the user's bids; if the aircraft is
    /// parked at one of the bid's departure airports AND the loaded
    /// aircraft matches the bid's planned aircraft, it auto-fires
    /// `flight_start` for that bid. Persisted via the same
    /// `aeroacars.autoStart` localStorage key the React side reads.
    auto_start_enabled: AtomicBool,
    /// Marker so the watcher doesn't re-trigger after a successful
    /// auto-start while the resulting flight is still active. Set to
    /// the Bid ID we last fired for; cleared when the flight ends.
    /// Without this, a user that cancels the auto-started flight
    /// while still parked at the gate would get an instant re-trigger.
    auto_start_last_bid_id: Mutex<Option<i64>>,
    /// v0.3.0: Letzter Auto-Start-Skip-Grund. Wenn die UI fragt, wir
    /// den ausgeben können statt der Pilot grübelt warum nichts
    /// passiert. Format: "engines_on" / "moving" / "airborne" /
    /// "no_matching_bid" / "no_bids" — mit Timestamp damit die UI
    /// erkennen kann ob's gerade aktuell ist oder uralt.
    auto_start_skip_reason: Mutex<Option<(DateTime<Utc>, String)>>,
    /// When `true`, intercept the main window's CloseRequested event
    /// and `hide()` the window instead of letting it close. The user
    /// gets to it again via the system-tray icon (Win) / menubar
    /// item (Mac). When `false`, close behaves normally and the app
    /// quits. Toggle lives in Settings → Verhalten; persisted via
    /// the React side's `aeroacars.minimizeToTray` localStorage key
    /// and synced into here on every mount + change.
    minimize_to_tray_enabled: AtomicBool,
    /// v0.4.0: Cached pilot identity (e.g. `("GSG0001", "Thomas K")`)
    /// für Discord-Webhook-Posts. Wird beim Profile-Refresh gefüllt
    /// (`get_profile()` returns it), bleibt für die Lebenszeit der
    /// AeroACARS-Session. Wenn kein Profile geladen → `None`, der
    /// Discord-Embed fällt auf "AeroACARS Pilot" zurück.
    cached_pilot: Mutex<Option<(String, String)>>,
    /// v0.5.11: MQTT live-tracking publisher handle. Connects to
    /// the aeroacars-live VPS (live.kant.ovh) via auto-provisioning
    /// the first time AeroACARS sees a logged-in pilot's API key.
    /// Pure background feature — pilot-invisible, no UI. Failure is
    /// non-fatal: AeroACARS works exactly as before if MQTT is
    /// unreachable. tokio::sync::Mutex (not std) because Handle is
    /// only ever accessed from async contexts (hook points run
    /// inside the streamer's async tasks).
    mqtt: tokio::sync::Mutex<Option<aeroacars_mqtt::Handle>>,
}

/// RAII guard for `AppState::flight_setup_in_progress`. Acquire it at the
/// start of `flight_start` / `flight_adopt`; the in-progress flag is cleared
/// automatically when the guard goes out of scope (any return path), unless
/// `disarm()` was called to keep the slot reserved (e.g. because the new
/// ActiveFlight has been written into state and the `active_flight` mutex
/// now serves as the conflict guard).
struct FlightSetupGuard<'a> {
    flag: &'a AtomicBool,
    armed: bool,
}

impl<'a> FlightSetupGuard<'a> {
    fn try_acquire(flag: &'a AtomicBool) -> Result<Self, UiError> {
        flag.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| {
                UiError::new(
                    "flight_setup_in_progress",
                    "another flight start or adopt is already in progress — please wait",
                )
            })?;
        Ok(Self { flag, armed: true })
    }

    /// Tell the guard to NOT release the in-progress flag on drop. Use this
    /// once we've successfully written the new ActiveFlight into state — at
    /// that point the `active_flight` mutex is the source of truth.
    fn disarm(mut self) {
        self.armed = false;
        // Manually release the in-progress flag right now; from here the
        // `active_flight` mutex blocks duplicate setups.
        self.flag.store(false, Ordering::SeqCst);
    }
}

impl Drop for FlightSetupGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.flag.store(false, Ordering::SeqCst);
        }
    }
}

/// In-memory record of an in-progress flight. Held inside an `Arc` so the
/// background streaming task can hold a reference without going through the
/// AppState mutex.
struct ActiveFlight {
    pirep_id: String,
    bid_id: i64,
    started_at: DateTime<Utc>,
    /// ICAO of the operating airline (e.g. "DLH"). Combined with
    /// `flight_number` to produce the full callsign on the dashboard
    /// ("DLH155" instead of just "155"). Empty string when unknown.
    airline_icao: String,
    /// v0.4.0: phpVMS-hosted Airline-Logo-URL (von `bid.flight.airline.logo`,
    /// z.B. `https://german-sky-group.eu/storage/uploads/airlines/4/.../logo.png`).
    /// Discord-Webhook-Embeds nutzen das als großes Bild unten —
    /// Pilot sieht in seinem Channel das Logo der gerade geflogenen
    /// Airline ohne dass wir es im Repo selbst hosten müssen.
    /// `None` wenn die VA das Logo-Feld nicht gepflegt hat.
    airline_logo_url: Option<String>,
    /// Registration phpVMS assigned to this flight (e.g. "D-AIUV").
    /// Looked up via `get_aircraft(bid.flight.aircraft_id)` at start
    /// time. Compared against the live `ATC ID` SimVar in the activity
    /// log so the pilot sees immediately if they loaded the wrong tail
    /// number in MSFS. Empty string when unknown (fresh-PIREP / disk-
    /// resume edge cases where we couldn't match a bid).
    planned_registration: String,
    /// Aircraft ICAO der Bid (z.B. "B738"). Wird im PIREP-Custom-Field
    /// "Aircraft Type" rausgegeben damit der VA-Admin auf der phpVMS-
    /// Detail-Seite ohne extra Lookup weiß was geflogen wurde.
    /// v0.3.0: vorher nur `planned_registration`, neuer `aircraft_icao`
    /// kommt aus dem gleichen `client.get_aircraft(id)`-Call.
    aircraft_icao: String,
    /// Aircraft-Name (z.B. "Boeing 737-800"). Komplettiert die ICAO-
    /// Anzeige. Empty wenn nicht ermittelbar.
    aircraft_name: String,
    flight_number: String,
    dpt_airport: String,
    arr_airport: String,
    /// Final loads (per fare-class id) captured at flight start so we can
    /// include them in the filed PIREP — even if the bid is gone by then.
    fares: Vec<(i64, i32)>,
    /// Mutable running stats updated by the streamer task.
    stats: Mutex<FlightStats>,
    stop: AtomicBool,
    /// True until the resume banner is dismissed (confirmed or cancelled).
    /// Drives the resume modal on the dashboard.
    was_just_resumed: AtomicBool,
    /// Compare-and-swap guard for `spawn_position_streamer`. Several UI paths
    /// (resume confirm, StrictMode double-mount, retry on transient error)
    /// could end up calling spawn more than once for the same ActiveFlight;
    /// this flag ensures only ONE streamer task is ever live per flight.
    streamer_spawned: AtomicBool,
    /// Set once we've detected that phpVMS soft-deleted the PIREP under us
    /// (any live POST endpoint returning 404). Idempotency guard so the
    /// "PIREP cancelled remotely" activity entry / UI event fire exactly
    /// once even if positions and the heartbeat both get 404s in the
    /// same cycle.
    cancelled_remotely: AtomicBool,
}

/// On-disk representation of an active flight, used for resume after a client
/// crash. Not the same as `ActiveFlight` because we only persist serializable,
/// non-Mutex fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedFlight {
    pirep_id: String,
    bid_id: i64,
    started_at: DateTime<Utc>,
    #[serde(default)]
    airline_icao: String,
    /// v0.4.0: phpVMS-Airline-Logo-URL (für Discord-Webhook-Embeds).
    /// `#[serde(default)]` damit Snapshots aus älteren Builds beim
    /// Resume nicht crashen — fehlendes Feld wird zu `None`.
    #[serde(default)]
    airline_logo_url: Option<String>,
    #[serde(default)]
    planned_registration: String,
    flight_number: String,
    dpt_airport: String,
    arr_airport: String,
    fares: Vec<(i64, i32)>,
    /// Snapshot of running flight statistics (distance, fuel, phase
    /// FSM state, captured timestamps). Written by the position
    /// streamer every `STATS_PERSIST_EVERY_TICKS` posts; read by
    /// `try_resume_flight` so a Tauri restart mid-flight doesn't lose
    /// fuel-burn or distance numbers — that was the root cause of the
    /// "0 distance / 0 fuel" PIREPs we saw before Phase H.4.
    #[serde(default)]
    stats: PersistedFlightStats,
}

/// Persistable subset of `FlightStats`. The activity-log edge-detector
/// state (`last_logged_*`) is intentionally NOT persisted — after a
/// resume we restart the diff detection cleanly.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct PersistedFlightStats {
    #[serde(default)]
    distance_nm: f64,
    #[serde(default)]
    position_count: u32,
    #[serde(default)]
    phase: FlightPhase,
    #[serde(default)]
    block_off_at: Option<DateTime<Utc>>,
    #[serde(default)]
    takeoff_at: Option<DateTime<Utc>>,
    #[serde(default)]
    landing_at: Option<DateTime<Utc>>,
    #[serde(default)]
    block_on_at: Option<DateTime<Utc>>,
    #[serde(default)]
    takeoff_weight_kg: Option<f64>,
    #[serde(default)]
    takeoff_fuel_kg: Option<f32>,
    /// v0.5.16: pitch / bank at takeoff (rotation moment).
    #[serde(default)]
    takeoff_pitch_deg: Option<f32>,
    #[serde(default)]
    takeoff_bank_deg: Option<f32>,
    #[serde(default)]
    landing_rate_fpm: Option<f32>,
    #[serde(default)]
    landing_g_force: Option<f32>,
    #[serde(default)]
    landing_pitch_deg: Option<f32>,
    /// v0.5.16: bank angle at touchdown for wing-strike detection.
    #[serde(default)]
    landing_bank_deg: Option<f32>,
    #[serde(default)]
    landing_speed_kt: Option<f32>,
    /// v0.5.17: groundspeed at touchdown.
    #[serde(default)]
    landing_groundspeed_kt: Option<f32>,
    #[serde(default)]
    landing_weight_kg: Option<f64>,
    #[serde(default)]
    landing_heading_deg: Option<f32>,
    #[serde(default)]
    landing_fuel_kg: Option<f32>,
    #[serde(default)]
    block_fuel_kg: Option<f32>,
    #[serde(default)]
    last_fuel_kg: Option<f32>,
    #[serde(default)]
    last_lat: Option<f64>,
    #[serde(default)]
    last_lon: Option<f64>,
    #[serde(default)]
    landing_peak_vs_fpm: Option<f32>,
    #[serde(default)]
    landing_peak_g_force: Option<f32>,
    #[serde(default)]
    bounce_count: u8,
    #[serde(default)]
    landing_score: Option<LandingScore>,
    #[serde(default)]
    landing_score_announced: bool,
    #[serde(default)]
    dep_gate: Option<String>,
    #[serde(default)]
    arr_gate: Option<String>,
    #[serde(default)]
    approach_runway: Option<String>,
    #[serde(default)]
    cruise_peak_msl: Option<f32>,
    #[serde(default)]
    climb_peak_msl: Option<f32>,
    #[serde(default)]
    peak_altitude_ft: Option<f32>,
    #[serde(default)]
    aircraft_banner_logged: bool,
    // ---- Tier 1/2/3 (BeatMyLanding-aligned) touchdown extras ----
    #[serde(default)]
    touchdown_sideslip_deg: Option<f32>,
    #[serde(default)]
    touchdown_profile: Vec<TouchdownProfilePoint>,
    #[serde(default)]
    runway_match: Option<runway::RunwayMatch>,
    #[serde(default)]
    landing_lat: Option<f64>,
    #[serde(default)]
    landing_lon: Option<f64>,
    #[serde(default)]
    landing_heading_true_deg: Option<f32>,
    /// Headwind component at touchdown (knots). Positive = wind from
    /// the front (= reduces required ground roll). Negative = tailwind.
    /// Sourced from `AIRCRAFT WIND Z` (negated, since +Z is tailwind
    /// in MSFS body-axis convention).
    #[serde(default)]
    landing_headwind_kt: Option<f32>,
    /// Crosswind component at touchdown (knots). Positive = from the
    /// right side; negative = from the left. Sourced from
    /// `AIRCRAFT WIND X` (positive X is crosswind from right per MSFS).
    #[serde(default)]
    landing_crosswind_kt: Option<f32>,
    // ---- Landing Analyzer (Stage 1) ----
    #[serde(default)]
    approach_vs_stddev_fpm: Option<f32>,
    #[serde(default)]
    approach_bank_stddev_deg: Option<f32>,
    #[serde(default)]
    rollout_distance_m: Option<f64>,
    // ---- Landing Analyzer (Stage 2): SimBrief OFP plan ----
    #[serde(default)]
    planned_block_fuel_kg: Option<f32>,
    #[serde(default)]
    planned_burn_kg: Option<f32>,
    #[serde(default)]
    planned_reserve_kg: Option<f32>,
    #[serde(default)]
    planned_zfw_kg: Option<f32>,
    #[serde(default)]
    planned_tow_kg: Option<f32>,
    #[serde(default)]
    planned_ldw_kg: Option<f32>,
    #[serde(default)]
    planned_route: Option<String>,
    #[serde(default)]
    planned_alternate: Option<String>,
    // v0.3.0: MAX-Werte aus dem OFP für Overweight-Detection.
    #[serde(default)]
    planned_max_zfw_kg: Option<f32>,
    #[serde(default)]
    planned_max_tow_kg: Option<f32>,
    #[serde(default)]
    planned_max_ldw_kg: Option<f32>,
    // ---- Touch-and-Go + Go-Around tracking (v0.1.26) ----
    // Persisted so a Tauri restart mid-flight (or a planned resume
    // after the pilot closed the app for lunch) doesn't wipe the
    // training-flight audit trail. `pending_acars_logs` is NOT
    // persisted on purpose — it's a transient queue between FSM
    // tick and streamer tick; if the streamer didn't drain it
    // before shutdown, those log lines are simply lost. Resume
    // semantics for the GA detector: lowest_agl is preserved so a
    // pilot who quits the app on short final and resumes can still
    // get their GA correctly classified.
    #[serde(default)]
    touchdown_events: Vec<TouchdownEvent>,
    #[serde(default)]
    touch_and_go_pending_since: Option<DateTime<Utc>>,
    #[serde(default)]
    go_around_count: u32,
    #[serde(default)]
    lowest_agl_during_approach_ft: Option<f32>,
    #[serde(default)]
    go_around_climb_pending_since: Option<DateTime<Utc>>,
    // v0.5.11 holding-pattern tracking
    #[serde(default)]
    holding_pending_since: Option<DateTime<Utc>>,
    #[serde(default)]
    holding_exit_pending_since: Option<DateTime<Utc>>,
    #[serde(default)]
    previous_phase_before_holding: Option<FlightPhase>,
}

impl PersistedFlightStats {
    fn snapshot_from(stats: &FlightStats) -> Self {
        Self {
            distance_nm: stats.distance_nm,
            position_count: stats.position_count,
            phase: stats.phase,
            block_off_at: stats.block_off_at,
            takeoff_at: stats.takeoff_at,
            landing_at: stats.landing_at,
            block_on_at: stats.block_on_at,
            takeoff_weight_kg: stats.takeoff_weight_kg,
            takeoff_fuel_kg: stats.takeoff_fuel_kg,
            takeoff_pitch_deg: stats.takeoff_pitch_deg,
            takeoff_bank_deg: stats.takeoff_bank_deg,
            landing_rate_fpm: stats.landing_rate_fpm,
            landing_g_force: stats.landing_g_force,
            landing_pitch_deg: stats.landing_pitch_deg,
            landing_bank_deg: stats.landing_bank_deg,
            landing_speed_kt: stats.landing_speed_kt,
            landing_groundspeed_kt: stats.landing_groundspeed_kt,
            landing_weight_kg: stats.landing_weight_kg,
            landing_heading_deg: stats.landing_heading_deg,
            landing_fuel_kg: stats.landing_fuel_kg,
            block_fuel_kg: stats.block_fuel_kg,
            last_fuel_kg: stats.last_fuel_kg,
            last_lat: stats.last_lat,
            last_lon: stats.last_lon,
            landing_peak_vs_fpm: stats.landing_peak_vs_fpm,
            landing_peak_g_force: stats.landing_peak_g_force,
            bounce_count: stats.bounce_count,
            landing_score: stats.landing_score,
            landing_score_announced: stats.landing_score_announced,
            dep_gate: stats.dep_gate.clone(),
            arr_gate: stats.arr_gate.clone(),
            approach_runway: stats.approach_runway.clone(),
            cruise_peak_msl: stats.cruise_peak_msl,
            climb_peak_msl: stats.climb_peak_msl,
            peak_altitude_ft: stats.peak_altitude_ft,
            aircraft_banner_logged: stats.aircraft_banner_logged,
            touchdown_sideslip_deg: stats.touchdown_sideslip_deg,
            touchdown_profile: stats.touchdown_profile.clone(),
            runway_match: stats.runway_match.clone(),
            landing_lat: stats.landing_lat,
            landing_lon: stats.landing_lon,
            landing_heading_true_deg: stats.landing_heading_true_deg,
            landing_headwind_kt: stats.landing_headwind_kt,
            landing_crosswind_kt: stats.landing_crosswind_kt,
            approach_vs_stddev_fpm: stats.approach_vs_stddev_fpm,
            approach_bank_stddev_deg: stats.approach_bank_stddev_deg,
            rollout_distance_m: stats.rollout_distance_m,
            planned_block_fuel_kg: stats.planned_block_fuel_kg,
            planned_burn_kg: stats.planned_burn_kg,
            planned_reserve_kg: stats.planned_reserve_kg,
            planned_zfw_kg: stats.planned_zfw_kg,
            planned_tow_kg: stats.planned_tow_kg,
            planned_ldw_kg: stats.planned_ldw_kg,
            planned_route: stats.planned_route.clone(),
            planned_alternate: stats.planned_alternate.clone(),
            planned_max_zfw_kg: stats.planned_max_zfw_kg,
            planned_max_tow_kg: stats.planned_max_tow_kg,
            planned_max_ldw_kg: stats.planned_max_ldw_kg,
            touchdown_events: stats.touchdown_events.clone(),
            touch_and_go_pending_since: stats.touch_and_go_pending_since,
            go_around_count: stats.go_around_count,
            lowest_agl_during_approach_ft: stats.lowest_agl_during_approach_ft,
            go_around_climb_pending_since: stats.go_around_climb_pending_since,
            holding_pending_since: stats.holding_pending_since,
            holding_exit_pending_since: stats.holding_exit_pending_since,
            previous_phase_before_holding: stats.previous_phase_before_holding,
        }
    }

    fn apply_to(self, stats: &mut FlightStats) {
        stats.distance_nm = self.distance_nm;
        stats.position_count = self.position_count;
        // Don't clobber the freshly-initialised Boarding phase with the
        // serde-default Preflight when the persisted JSON predates the
        // stats sidecar (no `phase` field → FlightPhase::default() →
        // Preflight, which has no matching arm in step_flight, so the
        // FSM would freeze and never advance past it).
        if self.phase != FlightPhase::Preflight {
            stats.phase = self.phase;
        }
        stats.block_off_at = self.block_off_at;
        stats.takeoff_at = self.takeoff_at;
        stats.landing_at = self.landing_at;
        stats.block_on_at = self.block_on_at;
        stats.takeoff_weight_kg = self.takeoff_weight_kg;
        stats.takeoff_fuel_kg = self.takeoff_fuel_kg;
        stats.takeoff_pitch_deg = self.takeoff_pitch_deg;
        stats.takeoff_bank_deg = self.takeoff_bank_deg;
        stats.landing_rate_fpm = self.landing_rate_fpm;
        stats.landing_g_force = self.landing_g_force;
        stats.landing_pitch_deg = self.landing_pitch_deg;
        stats.landing_bank_deg = self.landing_bank_deg;
        stats.landing_speed_kt = self.landing_speed_kt;
        stats.landing_groundspeed_kt = self.landing_groundspeed_kt;
        stats.landing_weight_kg = self.landing_weight_kg;
        stats.landing_heading_deg = self.landing_heading_deg;
        stats.landing_fuel_kg = self.landing_fuel_kg;
        stats.block_fuel_kg = self.block_fuel_kg;
        stats.last_fuel_kg = self.last_fuel_kg;
        stats.last_lat = self.last_lat;
        stats.last_lon = self.last_lon;
        stats.landing_peak_vs_fpm = self.landing_peak_vs_fpm;
        stats.landing_peak_g_force = self.landing_peak_g_force;
        stats.bounce_count = self.bounce_count;
        stats.landing_score = self.landing_score;
        stats.landing_score_announced = self.landing_score_announced;
        stats.dep_gate = self.dep_gate;
        stats.arr_gate = self.arr_gate;
        stats.approach_runway = self.approach_runway;
        stats.cruise_peak_msl = self.cruise_peak_msl;
        stats.climb_peak_msl = self.climb_peak_msl;
        stats.peak_altitude_ft = self.peak_altitude_ft;
        stats.aircraft_banner_logged = self.aircraft_banner_logged;
        stats.touchdown_sideslip_deg = self.touchdown_sideslip_deg;
        stats.touchdown_profile = self.touchdown_profile;
        stats.runway_match = self.runway_match;
        stats.landing_lat = self.landing_lat;
        stats.landing_lon = self.landing_lon;
        stats.landing_heading_true_deg = self.landing_heading_true_deg;
        stats.landing_headwind_kt = self.landing_headwind_kt;
        stats.landing_crosswind_kt = self.landing_crosswind_kt;
        stats.approach_vs_stddev_fpm = self.approach_vs_stddev_fpm;
        stats.approach_bank_stddev_deg = self.approach_bank_stddev_deg;
        stats.rollout_distance_m = self.rollout_distance_m;
        stats.planned_block_fuel_kg = self.planned_block_fuel_kg;
        stats.planned_burn_kg = self.planned_burn_kg;
        stats.planned_reserve_kg = self.planned_reserve_kg;
        stats.planned_zfw_kg = self.planned_zfw_kg;
        stats.planned_tow_kg = self.planned_tow_kg;
        stats.planned_ldw_kg = self.planned_ldw_kg;
        stats.planned_route = self.planned_route;
        stats.planned_alternate = self.planned_alternate;
        stats.planned_max_zfw_kg = self.planned_max_zfw_kg;
        stats.planned_max_tow_kg = self.planned_max_tow_kg;
        stats.planned_max_ldw_kg = self.planned_max_ldw_kg;
        stats.touchdown_events = self.touchdown_events;
        stats.touch_and_go_pending_since = self.touch_and_go_pending_since;
        stats.go_around_count = self.go_around_count;
        stats.lowest_agl_during_approach_ft = self.lowest_agl_during_approach_ft;
        stats.go_around_climb_pending_since = self.go_around_climb_pending_since;
        stats.holding_pending_since = self.holding_pending_since;
        stats.holding_exit_pending_since = self.holding_exit_pending_since;
        stats.previous_phase_before_holding = self.previous_phase_before_holding;
    }
}

/// One entry in the touchdown ring buffer (see `FlightStats::snapshot_buffer`).
///
/// Carries enough fields for the post-touchdown analyzer (V/S, G,
/// on-ground edge, AGL for bounce detection) and the touchdown profile
/// reconstruction (heading, GS, IAS, lat/lon for sideslip + runway
/// correlation). Sampled at ~30 Hz so a 5-second buffer holds ~150
/// entries — cheap, but rich enough to produce a per-Touchdown-ms
/// V/S curve identical to what BeatMyLanding ships.
// Several fields here aren't read post-Tier-3 refactor — they were
// the inputs to the old `compute_sideslip_at_touchdown` helper which
// we replaced with the native `atan2(VEL_BODY_X, VEL_BODY_Z)` path.
// We keep them in the buffer because the touchdown_profile mapping
// in step_flight reads them when freezing the profile points (see
// `t_ms`, `vs_fpm`, `g_force`, `agl_ft` etc. all consumed there),
// and `lat`/`lon` are kept as a fallback hook in case we ever need
// to derive sideslip from successive positions for a sim that
// doesn't expose body velocity.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
struct TelemetrySample {
    at: DateTime<Utc>,
    vs_fpm: f32,
    g_force: f32,
    on_ground: bool,
    /// AGL altitude — drives bounce detection (35 ft up / 5 ft return,
    /// BeatMyLanding-aligned). Sourced from `altitude_agl_ft` directly.
    agl_ft: f32,
    /// True heading at the moment of sample. Used at touchdown to
    /// reconstruct ground track from successive lat/lon pairs and
    /// derive sideslip / crab angle.
    heading_true_deg: f32,
    groundspeed_kt: f32,
    indicated_airspeed_kt: f32,
    lat: f64,
    lon: f64,
    pitch_deg: f32,
    bank_deg: f32,
}

/// Maximum age of any entry in the touchdown ring buffer. 5 s gives
/// us roughly the last final-approach segment plus the touchdown
/// itself. Longer than ~6 s and we'd start picking up Cruise data;
/// shorter than ~3 s and we'd miss the descent rate moments before
/// flare.
const TOUCHDOWN_BUFFER_SECS: i64 = 5;

/// One frozen subsample around the touchdown moment, surfaced in the
/// PIREP notes block as a tiny V/S / G curve. Modelled after
/// BeatMyLanding's `LandingTouchdownProfilePoint`. Times are in ms
/// relative to the on-ground edge — negative = before, positive =
/// after.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct TouchdownProfilePoint {
    /// Milliseconds relative to the touchdown edge. Range roughly
    /// −5000..+0 in the captured slice (only the buffer's history).
    t_ms: i32,
    vs_fpm: f32,
    g_force: f32,
    agl_ft: f32,
    on_ground: bool,
    heading_true_deg: f32,
    groundspeed_kt: f32,
    indicated_airspeed_kt: f32,
    pitch_deg: f32,
    bank_deg: f32,
}


/// v0.4.1: Snapshot der letzten bekannten Sim-Werte zum Zeitpunkt
/// als der Streamer den Sim-Disconnect detektiert hat. Gezeigt im
/// Cockpit-Banner + Activity-Log + bei Bedarf für Reposition.
#[derive(Debug, Clone, serde::Serialize)]
struct PausedSnapshot {
    pub lat: f64,
    pub lon: f64,
    pub heading_deg: f32,
    pub altitude_ft: f64,
    pub fuel_total_kg: f32,
    pub zfw_kg: Option<f32>,
}

#[derive(Default)]
struct FlightStats {
    // Position tracking.
    last_lat: Option<f64>,
    last_lon: Option<f64>,
    distance_nm: f64,
    position_count: u32,

    // ---- Phase-FSM state ----
    /// Current flight phase. Starts at Boarding when flight_start fires.
    phase: FlightPhase,
    /// Recent transitions for the flight log.
    transitions: Vec<(DateTime<Utc>, FlightPhase)>,
    /// Snapshot of the previous tick — used to detect on_ground / parking
    /// brake transitions cleanly.
    was_on_ground: Option<bool>,
    was_parking_brake: Option<bool>,

    // ---- Block / takeoff / landing timestamps (real-time UTC) ----
    block_off_at: Option<DateTime<Utc>>,
    takeoff_at: Option<DateTime<Utc>>,
    landing_at: Option<DateTime<Utc>>,
    block_on_at: Option<DateTime<Utc>>,

    /// First tick at which the universal "we're done" fallback saw all
    /// the conditions satisfied (on-ground, engines off, within 2 nmi
    /// of arrival). Once now − this ≥ `ARRIVED_FALLBACK_DWELL_SECS`
    /// the FSM jumps to Arrived regardless of prior phase. Cleared
    /// the moment any condition stops being true (so a brief engine
    /// restart or move resets the dwell). Powers helicopter flights,
    /// short hops, and emergency landings near destination.
    arrived_fallback_pending_since: Option<DateTime<Utc>>,

    /// Detected divert situation — populated once when the FSM reaches
    /// Arrived OR the universal fallback fires AND the aircraft is
    /// >= `DIVERT_DETECT_RADIUS_NM` from the planned `arr_airport`.
    /// Surfaced via `flight_status` so the cockpit can render a
    /// "Divert detected" banner and let the pilot file with the
    /// correct `arr_airport_id`. None when we landed at the planned
    /// destination as expected.
    divert_hint: Option<DivertHint>,

    /// v0.4.1: Sim-Disconnect-Pause-State.
    ///
    /// Wenn der Simulator mid-flight wegbricht (Crash, Quit, FPS auf 0,
    /// SimConnect/UDP timeout) und mehr als `SIM_DISCONNECT_THRESHOLD_S`
    /// Sekunden ohne brauchbaren Snapshot vergangen sind, friert der
    /// Streamer den Flug ein und wartet auf manuellen Resume-Klick.
    ///
    /// Während Pause:
    ///   * keine Position-Posts mehr an phpVMS (sonst Stale Data)
    ///   * Heartbeat (`/update`) läuft weiter — sonst killt phpVMS' cron
    ///     den PIREP nach `acars.live_time` (~2h)
    ///   * Phase-FSM friert auf dem Stand vor Disconnect
    ///   * Activity-Log + Discord-Webhook deaktiviert für die Dauer
    ///
    /// Beim Disconnect-Übergang loggen wir die letzte bekannte Position
    /// (Lat/Lon/HDG/Alt/Fuel/ZFW) damit der Pilot weiß wohin er nach
    /// dem Sim-Restart re-positionieren soll. Kein 5-NM-Restriction wie
    /// bei smartCARS — der Pilot entscheidet wo er wieder einsteigt.
    paused_since: Option<DateTime<Utc>>,
    /// Snapshot der letzten Werte zum Pause-Zeitpunkt — wird in der UI
    /// + Activity-Log angezeigt damit der Pilot weiß was er für die
    /// Repositionierung braucht. None solange der Flug nicht pausiert.
    paused_last_known: Option<PausedSnapshot>,

    /// True once the aircraft has actually been airborne above
    /// `WAS_AIRBORNE_AGL_FT` ft AGL since the flight started. Used to
    /// gate the universal Arrived-fallback and divert-detection so
    /// they NEVER fire pre-flight — caught by a real bug at KFLL
    /// where GSX repositioned the aircraft a few meters at the gate,
    /// `block_off_at` got stamped from the resulting >0.5 kt motion,
    /// and 9 minutes later (engines never started) the fallback
    /// fired with phase=Arrived + a divert banner pointing at
    /// "you're in KFLL, planned was MKJS — divert?". Aircraft had
    /// never even left the gate.
    was_airborne: bool,
    /// Consecutive-tick counter for the airborne dwell filter. Bumps
    /// on every tick where the airborne conditions hold; clears the
    /// moment any condition fails. Hits `WAS_AIRBORNE_DWELL_TICKS`
    /// → `was_airborne` flips true. Live bug 2026-05-03 (B738/GSX
    /// pushback): a single glitch tick at FL538 was enough to flip
    /// the flag. Sustained filter hardens that.
    airborne_dwell_ticks: u8,

    /// Wann das Flugzeug zum ersten Mal nach `tug_done` zum Stehen
    /// kam (gs < 0.5 kt). Triggert das 10-Sekunden-Dwell-Fenster
    /// bevor die Phase auf TaxiOut wechselt — entspricht dem echten
    /// Workflow "Tug ab, Park-Bremse, Funk, dann anrollen". Reset
    /// auf None sobald die Phase auf TaxiOut springt; nicht
    /// persistiert (Pushback überlebt keinen Restart).
    pushback_stopped_at: Option<DateTime<Utc>>,

    // ---- Capture at takeoff ----
    takeoff_weight_kg: Option<f64>,
    takeoff_fuel_kg: Option<f32>,
    /// v0.5.16: pitch / bank at takeoff (rotation moment). Submitted
    /// as numeric PIREP custom fields `takeoff-pitch` / `takeoff-roll`
    /// — DisposableSpecial dmaintenance reads these for tail-strike
    /// (pitch > 12.5° default) and wing-strike (roll > 15° default)
    /// detection. Captured the moment `takeoff_at` is stamped.
    takeoff_pitch_deg: Option<f32>,
    takeoff_bank_deg: Option<f32>,

    // ---- Capture at touchdown ----
    landing_rate_fpm: Option<f32>,
    landing_g_force: Option<f32>,
    landing_pitch_deg: Option<f32>,
    /// v0.5.16: bank angle at touchdown — submitted as numeric PIREP
    /// custom field `landing-roll` for the wing-strike maintenance
    /// check (default >15° = strike).
    landing_bank_deg: Option<f32>,
    landing_speed_kt: Option<f32>,
    /// v0.5.17: groundspeed at touchdown. Sent in MQTT touchdown
    /// payload as `gs_kt` so the live-tracking server can compute
    /// real-world rollout distance vs. IAS-derived rollout (which
    /// distorts when there's strong head/tailwind).
    landing_groundspeed_kt: Option<f32>,
    landing_weight_kg: Option<f64>,
    landing_heading_deg: Option<f32>,
    landing_fuel_kg: Option<f32>,

    // ---- Landing analyzer (Phase I) ----
    /// Most negative VS observed in the touchdown window. The first
    /// on-ground snapshot already gives a good number, but the spike
    /// often lands 1–2 ticks later — we keep refining for ~5 s after
    /// touchdown so the score reflects the worst case.
    landing_peak_vs_fpm: Option<f32>,
    /// Highest G-force observed in the touchdown window.
    landing_peak_g_force: Option<f32>,
    /// How many bounces (on_ground → !on_ground → on_ground) we counted
    /// within the touchdown window. >0 implies the pilot didn't put it
    /// down clean.
    bounce_count: u8,
    /// Categorised score, computed once when the window closes. The
    /// activity log emits this and we ship it in the PIREP custom fields.
    landing_score: Option<LandingScore>,
    /// True once the score has been announced to the activity log, so
    /// repeated streamer ticks (or a Tauri restart that resumes after
    /// touchdown) don't re-fire the entry.
    landing_score_announced: bool,

    /// Edge-tracking flag for AGL-based bounce detection. Set to true
    /// once the aircraft climbs above `BOUNCE_AGL_THRESHOLD_FT` after
    /// the initial touchdown; a subsequent drop below
    /// `BOUNCE_AGL_RETURN_FT` (within `BOUNCE_WINDOW_SECS`) increments
    /// `bounce_count` and clears this flag, ready to arm again.
    /// Replaces the noisy `was_on_ground && !on_ground` flicker we
    /// used pre-Tier-1, which was tripping on gear-strut oscillation.
    bounce_armed_above_threshold: bool,
    /// Sideslip / crab angle at the moment of touchdown, in degrees.
    /// Computed from `heading_true_deg − groundtrack` where the
    /// ground track is reconstructed from the last few ring-buffer
    /// samples just before touchdown. Positive = aircraft nose right
    /// of track (right crosswind crab); negative = nose left.
    /// `None` until we capture it; `None` also when the buffer can't
    /// produce a track (e.g. taxi-speed touchdown after a roll-out).
    touchdown_sideslip_deg: Option<f32>,
    /// v0.4.4: Sampler-side touchdown capture. Der 50Hz-Touchdown-Sampler
    /// detektiert die `on_ground=false→true`-Flanke direkt (innerhalb
    /// von 20ms) und merkt sich VS am Edge plus VS-Min in den letzten
    /// 500ms davor (= echter Touchdown-Sinkflug, bevor Rollout den
    /// Wert kaputt-mittelt). Wenn step_flight später (bis zu 5s
    /// verspätet im Streamer-Tick) die Edge auch detektiert, nimmt es
    /// diese pre-captured Werte statt den Buffer-Window-Scan zu
    /// versuchen — dann sind die Pre-TD-Samples nämlich schon evicted.
    ///
    /// Live-Bug Pilot-Test 2026-05-06 (DAL93 EDDB→KJFK): X-Plane Land-
    /// ung mit echtem -300 fpm → AeroACARS scorte +35 fpm (smooth)
    /// weil Streamer 5s nach Touchdown wachte und Buffer dann nur
    /// noch Rollout-Samples mit VS≈0 enthielt.
    sampler_touchdown_at: Option<DateTime<Utc>>,
    sampler_touchdown_vs_fpm: Option<f32>,
    sampler_touchdown_g_force: Option<f32>,
    /// v0.5.5: running peak-descent VS tracker. Updated every sampler
    /// tick while AGL ≤ 250 ft (low-altitude / final approach territory).
    /// Picks the most negative pitch-corrected VS seen ONLY in the
    /// touchdown footprint, NOT across the whole approach.
    ///
    /// v0.5.11 hardening (renamed from `approach_vs_min_fpm`):
    /// the previous variant tracked min across the whole Approach +
    /// Final segment. Pilot's deep analysis exposed the bug class:
    /// a pre-flare descent at -1346 fpm @ 943 ft AGL would win over
    /// the actual gentle touchdown. AGL ≤ 250 ft restricts tracking
    /// to the meaningful zone.
    ///
    /// Used ONLY as a fallback when the AGL-derivative estimator
    /// fails (very sparse RREF, no usable AGL samples). Never wins
    /// against a real AGL-Δ estimate.
    /// Reset on Approach entry so go-arounds don't carry stale data.
    low_agl_vs_min_fpm: Option<f32>,
    // ─── v0.5.25 Approach-Stability v2 ─────────────────────────────────
    //
    // Pre-v0.5.25: einfache Standardabweichung ueber Approach+Final-
    // Buffer (5000 ft AGL bis 0). Probleme:
    //   - Vectoring-Bank-Spikes von ATC bestrafen Pilot fuer die
    //     ausgefuehrte Anweisung
    //   - Step-Down-Descents waehrend Initial-Approach (Flaps/Speed-
    //     Down) verfaelschen V/S-Stddev
    //   - σ um Mittelwert misst nicht Glide-Slope-Abweichung
    //   - RWY-Wechsel mid-final bestraft Pilot fuer ATC-Action
    //
    // v0.5.25: scharfes Stable-Approach-Gate (= AGL ≤ 1000 ft) +
    // Glide-Slope-Deviation + Vector-Window-Filter.
    /// Mittlere Abweichung |actual_vs − target_vs(3°)| im 1000-ft-Gate.
    approach_vs_deviation_fpm: Option<f32>,
    /// Maximale Abweichung |actual_vs − target_vs(3°)| im LETZTEN
    /// 500-ft-Gate (= unter 500 ft AGL). Critical-zone Indikator.
    approach_max_vs_deviation_below_500_fpm: Option<f32>,
    /// Bank-Stddev ueber 1000-ft-Gate, gefiltert: Samples aus Vector-
    /// Windows (= 5 sec nach RWY-Change) ausgenommen.
    approach_bank_stddev_filtered_deg: Option<f32>,
    /// True wenn waehrend Approach/Final eine RWY-Aenderung beobachtet
    /// wurde mit AGL < 1500 ft (= "late RWY change", Pilot wurde zu
    /// Maneuver gezwungen, score-relevante Beruecksichtigung).
    approach_runway_changed_late: bool,
    /// True wenn beim Erreichen 1000 ft AGL: VS-Deviation < 200 fpm
    /// AND Bank-Mittelwert < 5°. = Stable-Approach-Gate erreicht.
    approach_stable_at_gate: Option<bool>,
    /// Anzahl Samples die im 1000-ft-Window lagen (= Konfidenz-
    /// Indikator, < 5 Samples = niedrige Konfidenz).
    approach_window_sample_count: Option<u32>,
    /// Arrival-Airport-Elevation aus phpVMS-Cache. Ermoeglicht HAT
    /// (Height Above Touchdown) statt AGL fuer Stable-Approach-Gate-
    /// Window. Kritisch fuer Mountain-Airports (LSGS, LFKB, …) wo AGL
    /// ueber Bergrueeken fluktuiert. None → Fallback auf AGL (mit
    /// niedrigerem Confidence-Score).
    arr_airport_elevation_ft: Option<f32>,
    /// V/S-Jerk: mean |Δvs| zwischen aufeinanderfolgenden Samples im
    /// Gate. Sim-/Aircraft-agnostisches Stabilitaets-Maß (jet vs. GA).
    /// < 100 fpm/tick = stable, > 300 fpm/tick = unstable.
    approach_vs_jerk_fpm: Option<f32>,
    /// IAS-Stddev im Gate-Window. Speed-Stability-Indikator.
    /// < 5 kt = on-target, > 15 kt = unstabil.
    approach_ias_stddev_kt: Option<f32>,
    /// Excessive Sink Flag: True wenn IRGENDEIN Sample im Gate
    /// V/S < -1000 fpm hatte. FAA Sink-Rate-Limit-Verletzung.
    approach_excessive_sink: bool,
    /// Stable-Configuration-Flag: Gear voll runter (≥99%) AND Flaps
    /// in Landing-Position (≥70%) am Gate. None bei Konfig-Sample fehlt.
    approach_stable_config: Option<bool>,
    /// "Nutzte HAT statt AGL?" — fuer UI-Confidence-Indikator.
    approach_used_hat: bool,
    // ─── v0.5.24 Takeoff-Edge-Capture (50Hz Sampler) ───────────────────
    //
    // Frueher: stats.takeoff_pitch_deg / takeoff_bank_deg wurden im
    // step_flight-Streamer-Tick (3-30s Cadence) gestempelt — also
    // potenziell mehrere Sekunden NACH dem echten Wheels-Up-Frame.
    // Resultat: Pitch wird zu hoch erfasst (= Initial-Climb-Pitch statt
    // Rotations-Pitch), bei tail-strike-empfindlichen Aircraft wie der
    // A321 ist das ein realistischer 2-3°-Versatz der False-Positive-
    // Tail-Strike-Checks im phpVMS DisposableSpecial-Modul triggert.
    //
    // Neu: der 50Hz-Touchdown-Sampler wurde erweitert um auch die
    // umgekehrte Edge (on_ground=true → false = Wheels-Up) zu fangen.
    // Capture im Frame des physischen Lift-Off (binnen 20ms). Werden
    // im step_flight-Phase-Transition als bevorzugte Quelle genutzt
    // statt snap.pitch_deg.
    sampler_takeoff_at: Option<DateTime<Utc>>,
    sampler_takeoff_pitch_deg: Option<f32>,
    sampler_takeoff_bank_deg: Option<f32>,
    // ─── v0.5.23 Touchdown-Forensik ───────────────────────────────────
    //
    // Bei jedem Touchdown laufen MSFS- und X-Plane-Schaetzer parallel
    // (siehe lib.rs ~Zeile 8254). Wir merken uns hier ALLE relevanten
    // Zwischenergebnisse damit der TouchdownPayload-Build sie ans
    // aeroacars-live-Monitor weitergeben kann fuer Forensik-Vergleiche.
    /// "msfs" / "xplane" / "other" — gestempelt im Touchdown-Frame.
    landing_simulator: Option<&'static str>,
    /// Lua-Style 30-Sample-Schaetzung in fpm. None wenn Pfad nicht lief.
    landing_vs_estimate_xp_fpm: Option<i32>,
    /// Time-Tier-Schaetzung in fpm. None wenn Pfad nicht lief.
    landing_vs_estimate_msfs_fpm: Option<i32>,
    /// Welcher Pfad hat den finalen vs_fpm geliefert.
    landing_vs_source: Option<&'static str>,
    /// X-Plane Gear-Sampler peak gear_normal_force_n. None auf MSFS.
    landing_gear_force_peak_n: Option<f32>,
    /// Lua-Schaetzer Window-Groesse in ms (None wenn Pfad nicht gewann).
    landing_estimate_window_ms: Option<i32>,
    /// Lua-Schaetzer Sample-Count im Window.
    landing_estimate_sample_count: Option<u32>,
    /// Frozen subsample of the ring buffer covering ±2 s around touchdown,
    /// for V/S-curve reconstruction in the PIREP notes block. Captured
    /// once when the on-ground edge fires; surviving across a Tauri
    /// restart so a resumed flight still has the curve to ship.
    touchdown_profile: Vec<TouchdownProfilePoint>,
    /// Runway-correlation result from the OurAirports CSV lookup,
    /// computed once at the touchdown edge from `(landing_lat,
    /// landing_lon, landing_heading_true_deg)`. Drives the runway
    /// distance / centerline-offset fields in the PIREP. None until
    /// computed; None also when no runway is within ~3 km of the
    /// touchdown coordinate.
    runway_match: Option<runway::RunwayMatch>,
    /// Latitude at the touchdown edge — captured separately from
    /// `last_lat` so a resume mid-rollout doesn't overwrite it.
    landing_lat: Option<f64>,
    landing_lon: Option<f64>,
    /// True heading at touchdown (used for runway-end disambiguation
    /// in the runway lookup; `landing_heading_deg` is *magnetic*).
    landing_heading_true_deg: Option<f32>,
    /// Headwind (positive) / tailwind (negative) at touchdown in knots.
    /// Sourced from `AIRCRAFT WIND Z` (negated). None when the SimVar
    /// isn't wired or the flight resumed after touchdown.
    landing_headwind_kt: Option<f32>,
    /// Crosswind at touchdown in knots. Positive = from the right.
    /// Sourced from `AIRCRAFT WIND X`.
    landing_crosswind_kt: Option<f32>,

    // ---- Landing Analyzer (Stage 1) ----
    /// Rolling buffer of (V/S, bank) samples collected during the
    /// Approach + Final phases. Capped at ~120 entries (≈ 10-15 min
    /// of data at the position-streamer's 5-8 s cadence). Drained
    /// at the touchdown edge into stddev metrics.
    /// Pre-v0.5.25: nur (vs_fpm, bank_deg). Ab v0.5.25 reicht ein
    /// reicheres Sample fuer korrekte Stable-Approach-Gate-Auswertung
    /// (siehe ApproachSample-Struct). Beide Felder bleiben in den
    /// Touchdown-Stats gepflegt; die alten approach_*_stddev werden
    /// fuer Backward-Compat weiter geliefert, das richtige
    /// Stable-Approach-Maß ist `approach_vs_deviation_fpm`.
    approach_buffer: std::collections::VecDeque<ApproachBufferSample>,
    /// V/S standard deviation (fpm) over the approach window.
    /// Lower = more stable. Computed once at touchdown.
    approach_vs_stddev_fpm: Option<f32>,
    /// Bank-angle standard deviation (degrees) over the approach
    /// window. Lower = smoother flying.
    approach_bank_stddev_deg: Option<f32>,
    /// Rollout distance in meters: accumulated great-circle distance
    /// from the touchdown point until groundspeed first drops below
    /// 5 kt. None until first touchdown; finalised once GS<5 kt is
    /// observed. Resumed flights mid-rollout finalise on next
    /// stop or never (we accept the imprecision).
    rollout_distance_m: Option<f64>,
    /// True once `rollout_distance_m` has been finalised. Stops the
    /// per-tick accumulation in step_flight from continuing past
    /// the actual stop.
    rollout_finalized: bool,
    /// Last (lat, lon) we accumulated into the rollout distance.
    /// Used to compute the great-circle delta to the next position.
    rollout_last_lat: Option<f64>,
    rollout_last_lon: Option<f64>,

    // ---- Multi-touchdown / Touch-and-Go / Go-Around tracking (Stage 3) ----
    /// Chronological log of every touchdown during the flight.
    ///
    /// On a normal one-landing flight: 1 entry (kind=FinalLanding).
    /// On a training flight with N touch-and-goes: N+1 entries — the
    /// last one is always the FinalLanding, earlier ones are
    /// TouchAndGo. The PIREP `landing_*` native fields and score are
    /// derived from the LAST entry (i.e. the actual final landing),
    /// so a T&G doesn't drag down the score.
    ///
    /// Empty until the first touchdown. Touchdowns are SNAPSHOTS
    /// taken at classification time — the in-progress
    /// `landing_peak_vs_fpm` etc. fields keep being refined for the
    /// CURRENT (= newest) touchdown until the bounce window closes.
    touchdown_events: Vec<TouchdownEvent>,
    /// Watcher state for the touch-and-go classifier. Holds the AGL
    /// excursion starting from a Touchdown event so we can decide
    /// (within `TOUCH_AND_GO_WATCH_SECS`) whether the aircraft is
    /// climbing back out for real (= T&G) or just bouncing/rolling
    /// out (= regular landing). `None` between touchdowns.
    touch_and_go_pending_since: Option<DateTime<Utc>>,
    /// Counter of go-arounds (rejected approaches without touchdown
    /// followed by climb-back-out). Surfaced as a custom PIREP field
    /// "Go-Arounds: N". Most flights = 0.
    go_around_count: u32,
    /// Lowest AGL observed during the current Approach/Final phase.
    /// Used by the go-around detector — once we see a sustained
    /// climb-back from this minimum past `GO_AROUND_AGL_RECOVERY_FT`,
    /// we classify it as a GA. Reset whenever Phase enters Climb
    /// (so each new approach gets a fresh minimum to compare against).
    lowest_agl_during_approach_ft: Option<f32>,
    /// First tick where the GA detector saw AGL climbing back past
    /// the recovery threshold. We need this sustained for
    /// `GO_AROUND_DWELL_SECS` before firing — otherwise every brief
    /// climb-correction during a normal approach would count as a GA.
    go_around_climb_pending_since: Option<DateTime<Utc>>,

    // ---- v0.5.11 holding-pattern tracking ----
    /// Timestamp of the first tick where the aircraft satisfied
    /// holding-pattern conditions (banked turn at constant altitude).
    /// Cleared whenever conditions break — only when sustained for
    /// `HOLDING_ENTRY_DWELL_SECS` does the FSM transition to Holding.
    holding_pending_since: Option<DateTime<Utc>>,
    /// Timestamp of the first tick where the aircraft has level wings
    /// while in Holding. Cleared if bank rises again. Sustained for
    /// `HOLDING_EXIT_DWELL_SECS` triggers the exit back to the
    /// previous phase.
    holding_exit_pending_since: Option<DateTime<Utc>>,
    /// What phase we entered Holding from (Cruise or Approach).
    /// Restored on Holding exit unless an active descent has begun
    /// (then we go directly to Approach).
    previous_phase_before_holding: Option<FlightPhase>,

    /// Free-form ACARS log lines that `step_flight` wants posted to
    /// phpVMS' `/acars/logs` on the next streamer tick — primarily
    /// used by the Touch-and-Go and Go-Around classifiers since
    /// they fire from inside the FSM (not on the streamer's normal
    /// phase-change branch). Drained empty by the streamer after
    /// each successful POST. Keeps the FSM decoupled from the HTTP
    /// client.
    pending_acars_logs: Vec<String>,

    // ---- Landing Analyzer (Stage 2): SimBrief OFP plan ----
    /// Planned block (= ramp) fuel from the SimBrief OFP, in kg.
    /// None when the bid had no SimBrief OFP attached or the fetch
    /// failed. Captured once at flight_start; never updated mid-flight.
    planned_block_fuel_kg: Option<f32>,
    /// Planned trip burn (takeoff → touchdown) from the OFP. Used at
    /// PIREP build to compute fuel-efficiency vs the actual burn.
    planned_burn_kg: Option<f32>,
    /// Planned reserve fuel (alternate + holding).
    planned_reserve_kg: Option<f32>,
    /// Planned zero-fuel weight (kg).
    planned_zfw_kg: Option<f32>,
    /// Planned takeoff weight (kg).
    planned_tow_kg: Option<f32>,
    /// Planned landing weight (kg).
    planned_ldw_kg: Option<f32>,
    /// Planned route string from the OFP.
    planned_route: Option<String>,
    /// Planned alternate ICAO from the OFP.
    planned_alternate: Option<String>,
    // ---- v0.3.0: MAX-Werte aus dem OFP für Overweight-Detection ----
    /// Maximum Zero-Fuel Weight (Strukturlimit). None wenn das OFP
    /// keinen `<max_zfw>`-Eintrag hatte (kommt bei Custom-Subfleets vor).
    planned_max_zfw_kg: Option<f32>,
    /// Maximum Takeoff Weight. Drives Overweight-Warnung im Live-
    /// Loadsheet vor Pushback und Score-Penalty im Landung-Tab.
    planned_max_tow_kg: Option<f32>,
    /// Maximum Landing Weight. Bei Overshoot droht Overweight-
    /// Landing-Inspektion in der echten Welt.
    planned_max_ldw_kg: Option<f32>,

    // ---- METAR snapshots (Phase J.2) ----
    /// Raw METAR text captured at takeoff for the departure airport.
    /// Filled by a fire-and-forget async fetch from the streamer when
    /// `step_flight` transitions into `Takeoff`. None until the fetch
    /// returns (and stays None if the airport isn't in NOAA's set).
    dep_metar_raw: Option<String>,
    /// Same for the arrival airport at touchdown (Final → Landing).
    arr_metar_raw: Option<String>,
    /// Markers so the streamer only kicks off one fetch per direction
    /// per flight. Set immediately when the spawn fires; cleared never.
    dep_metar_requested: bool,
    arr_metar_requested: bool,

    // ---- Fuel tracking ----
    block_fuel_kg: Option<f32>,
    last_fuel_kg: Option<f32>,
    // ---- Live-Loadsheet (v0.3.0) ----
    /// Letzter beobachteter ZFW-Wert vom Sim. Treibt das Live-
    /// Loadsheet-Display im Cockpit-Tab während der Boarding-Phase
    /// (Pilot sieht wie ZFW während des Boardings hochläuft).
    /// None wenn das Aircraft-Profil keinen ZFW-SimVar hat (Fenix).
    last_zfw_kg: Option<f32>,
    /// Letzter beobachteter Total-Weight-Wert vom Sim (= TOW während
    /// der Boarding-Phase, sobald komplett beladen).
    last_total_weight_kg: Option<f32>,
    /// True wenn der Loadsheet-Activity-Log-Eintrag beim Block-off
    /// schon emittet wurde — verhindert Duplikate beim Resume.
    loadsheet_logged_at_blockoff: bool,

    /// Highest MSL altitude we've seen while in Cruise (or any step
    /// climb during Cruise). Drives the Cruise → Descent guard so
    /// short ATC step-downs (FL380 → FL360) don't flip the phase to
    /// Descent — only a real TOD drop of >5000 ft from this peak
    /// counts.
    cruise_peak_msl: Option<f32>,
    /// v0.5.9: peak MSL altitude during the Climb phase. Reset on
    /// Climb entry. Used by the Climb → Descent guard — require
    /// >200 ft loss from this peak before transitioning, so single
    /// VS spikes (turbulence, level-off) don't flip the FSM.
    climb_peak_msl: Option<f32>,
    /// Peak MSL altitude across the entire flight, regardless of
    /// phase. Reported as the PIREP `level` field — phpVMS shows it
    /// as "Flt.Level". Updated on every snapshot during Climb /
    /// Cruise / Descent.
    peak_altitude_ft: Option<f32>,

    /// Rolling 5-second ring buffer of (timestamp, V/S, G, on-ground)
    /// for recovering the *true* touchdown values. The single-tick
    /// "first on_ground=true" snapshot routinely caught a bounce
    /// rebound (positive V/S, 1.0 G), missing the actual impact. The
    /// buffer lets us scan the last few seconds and pick the worst
    /// V/S / G that the aircraft actually saw — independent of the
    /// snapshot rate or SimConnect's PLANE TOUCHDOWN * latching.
    snapshot_buffer: std::collections::VecDeque<TelemetrySample>,

    /// True once the "Aircraft: {title}" banner has been emitted to
    /// the activity log for this flight. Persisted across resumes
    /// so a Tauri restart mid-flight doesn't re-fire the banner — it
    /// belongs to the *flight*, not the session.
    aircraft_banner_logged: bool,

    /// When the streamer last successfully posted a position to phpVMS.
    /// Drives the cockpit's LIVE / REC indicator — a long gap means
    /// the network is having trouble (or the streamer is dead) and
    /// the dashboard should warn the pilot before they file a PIREP
    /// without recent telemetry.
    last_position_at: Option<DateTime<Utc>>,
    /// When the streamer last successfully POSTed `/pireps/{id}/update`.
    /// Surfaced via `flight_status` so a debug panel can show "last
    /// keep-alive Xs ago" — useful to diagnose if the heartbeat that
    /// prevents phpVMS's RemoveExpiredLiveFlights cron from killing the
    /// PIREP is actually getting through.
    last_heartbeat_at: Option<DateTime<Utc>>,
    /// Number of positions currently sitting in the offline queue
    /// waiting to be replayed. >0 means we lost the network briefly
    /// and the dashboard should surface that.
    queued_position_count: u32,

    // ---- Gates / runway (from MSFS ATC SimVars) ----
    /// Parking stand the pilot pushed back from. Captured the first
    /// time `step_flight` sees a non-empty `parking_name` while still
    /// in Boarding. Survives across the whole flight.
    dep_gate: Option<String>,
    /// Stand the pilot ended up parked at after arrival. Captured at
    /// the Boarding-of-block transition (`BlocksOn`).
    arr_gate: Option<String>,
    /// `ATC RUNWAY SELECTED` snapshotted at touchdown. Useful for VAs
    /// that grade "did the pilot land on the right runway".
    approach_runway: Option<String>,

    // ---- Edge detector for activity log (Phase H.3) ----
    /// Last value we logged to the activity feed. Used to detect when the
    /// pilot changes a knob and emit one log entry per change rather than
    /// repeating the current state every tick.
    last_logged_squawk: Option<u16>,
    last_logged_com1: Option<f32>,
    last_logged_com2: Option<f32>,
    last_logged_nav1: Option<f32>,
    last_logged_nav2: Option<f32>,
    last_logged_lights: Option<LightsState>,
    /// 3-state strobe selector (0=OFF, 1=AUTO, 2=ON) for aircraft
    /// addons that distinguish AUTO from ON. Lives separate from
    /// `last_logged_lights.strobe` so we don't log a "Strobe lights
    /// ON" entry on top of the more informative "Strobe lights AUTO"
    /// one. Not persisted across resumes — change-detection state
    /// always rebuilds from the first post-resume tick.
    last_logged_strobe_state: Option<u8>,
    last_logged_ap: Option<ApState>,
    /// Debounce: when did we first observe the *current* AP master state?
    /// We only emit a "Autopilot ENGAGED/OFF" log entry if the new state
    /// has held for `AP_DEBOUNCE_SECS`. Stops a misbehaving LVar (pulsed
    /// momentary buttons, sim-engine restarts) from flooding the log.
    pending_ap_master: Option<bool>,
    pending_ap_master_since: Option<DateTime<Utc>>,
    last_logged_parking_brake: Option<bool>,
    last_logged_engines_running: Option<u8>,
    /// Pending engine-count change waiting for stability — same
    /// debounce trick as AP_DEBOUNCE_SECS. Fenix in particular
    /// flashes the GENERAL ENG COMBUSTION SimVar during engine
    /// starts and at idle, which fired phantom "Engine shutdown /
    /// started" pairs in the log.
    pending_engines_running: Option<u8>,
    pending_engines_running_since: Option<DateTime<Utc>>,
    last_logged_stall: Option<bool>,
    last_logged_overspeed: Option<bool>,
    /// Last aircraft profile we logged. If the pilot loads a different
    /// airframe mid-session (e.g. swaps from default A20N to Fenix), the
    /// detector flips and we want to emit a one-line announcement so the
    /// user sees the LVar mapping changed.
    last_logged_profile: Option<sim_core::AircraftProfile>,
    /// Flaps-handle detent the last time we emitted a log entry. Stored
    /// as the integer detent (0–5 on Airbus, 0/1/2/3/4 on Boeing —
    /// derived from `flaps_position * 5` and rounded). One log per real
    /// detent change rather than every 0.001 of jitter.
    last_logged_flaps_detent: Option<u8>,
    /// PMDG-Premium-First flap label the last time we emitted a log
    /// entry. Stored as the cockpit-exact Boeing label ("UP"/"1"/"5"/
    /// "15"/"30"/etc.) when a PMDG aircraft is loaded; cleared when
    /// the pilot switches to a non-PMDG aircraft so the legacy
    /// `last_logged_flaps_detent` path takes back over without
    /// firing a duplicate "Flaps ↓ X" entry.
    last_logged_flap_label: Option<String>,
    /// Gear command position discretised — 0 = up, 1 = down. The
    /// 0..1 SimVar value is rounded so a moving gear-handle doesn't
    /// flood the log mid-cycle.
    last_logged_gear_down: Option<bool>,
    /// Speed-brake handle deployed (>5%) — distinct from "armed".
    last_logged_spoilers_deployed: Option<bool>,
    last_logged_spoilers_armed: Option<bool>,
    last_logged_apu_running: Option<bool>,
    last_logged_battery_master: Option<bool>,
    last_logged_avionics_master: Option<bool>,
    last_logged_pitot_heat: Option<bool>,
    last_logged_engine_anti_ice: Option<bool>,
    last_logged_wing_anti_ice: Option<bool>,
    /// Seat belts sign (0=OFF, 1=AUTO, 2=ON). Logged on transitions.
    last_logged_seatbelts_sign: Option<u8>,
    /// No smoking sign (0=OFF, 1=AUTO, 2=ON).
    last_logged_no_smoking_sign: Option<u8>,
    /// Autobrake setting label ("OFF" / "LO" / "MED" / "MAX") for
    /// the activity log. Reset on every flight.
    last_logged_autobrake: Option<String>,

    // ---- PMDG SDK tracking (Phase H.4 / v0.2.0) ----
    /// Once-per-flight banner: "PMDG 737-800 detected — premium
    /// telemetry active". Fires the first tick we see PmdgState
    /// in the snapshot, not before (so e.g. the user starts
    /// AeroACARS, then loads PMDG → the banner appears at PMDG
    /// load, not at app start).
    pmdg_detected_logged: bool,
    /// Once-per-flight V-speeds banner — fires when the FMC
    /// finally has all four (V1, VR, V2, VREF) populated. We log
    /// them once at that moment so the timeline shows when the
    /// pilot completed the perf-init.
    pmdg_v_speeds_logged: bool,
    /// FMA mode tracking — combined string ("N1 / VNAV / LNAV")
    /// so a single change in any sub-mode produces one log entry
    /// rather than three.
    last_logged_pmdg_fma: Option<String>,
    /// MCP selected speed — tracked rounded to nearest knot.
    last_logged_pmdg_mcp_speed: Option<i16>,
    /// MCP selected heading.
    last_logged_pmdg_mcp_heading: Option<u16>,
    /// MCP selected altitude.
    last_logged_pmdg_mcp_altitude: Option<u16>,
    /// MCP selected V/S.
    last_logged_pmdg_mcp_vs: Option<i16>,
    /// AT armed state.
    last_logged_pmdg_at_armed: Option<bool>,
    /// AP engaged state (CMD A or B).
    last_logged_pmdg_ap_engaged: Option<bool>,
    /// Takeoff config warning (one-shot — only logs the on-edge,
    /// not the off-edge).
    last_logged_pmdg_to_warning: Option<bool>,

    // ---- v0.2.2 — wider PMDG integration ----
    /// FMC thrust limit mode label tracking (777-specific). Logged
    /// on every change — TO → CLB → CRZ → CON arc tells the VA
    /// admin the pilot worked the thrust schedule properly.
    last_logged_pmdg_thrust_mode: Option<String>,
    /// 777 ECL phase completion tracking — one slot per phase.
    /// Logged on rising edge (false→true) so the activity log
    /// shows "ECL: Preflight ✓ complete" once per phase.
    last_logged_pmdg_ecl: [bool; 10],
    /// PMDG-authoritative APU-running bit (777). Distinct from
    /// the standard-SimVar APU tracking which uses RPM heuristics.
    last_logged_pmdg_apu: Option<bool>,
    /// Wheel chocks set (777). Pre-flight ground state.
    last_logged_pmdg_chocks: Option<bool>,
    /// PMDG transponder mode label (v0.2.4). Cockpit-exact mode
    /// selector position — STBY/ALT-OFF/XPNDR/TA/TA-RA — which the
    /// standard `TRANSPONDER STATE` SimVar can't expose.
    last_logged_pmdg_xpdr_mode: Option<String>,

    // ---- PMDG PIREP captures (Phase 5.6 / v0.2.0) ----
    /// PMDG aircraft variant label captured once at flight start.
    /// "737-800" / "737-800 SSW" / "777-300ER" / etc. Surfaced in
    /// the PIREP custom fields as "Aircraft Variant".
    pmdg_variant_label: Option<String>,
    /// V-speeds captured at takeoff roll start (= once
    /// `takeoff_at` is stamped) so the PIREP records the values
    /// the pilot actually rotated against. (V1, VR, V2)
    pmdg_v_speeds_takeoff: Option<(u8, u8, u8)>,
    /// VREF captured at the touchdown moment for the PIREP.
    pmdg_vref_at_landing: Option<u8>,
    /// FMC TO-flaps degrees captured at takeoff roll start.
    pmdg_takeoff_flaps_planned: Option<u8>,
    /// FMC LDG-flaps degrees captured at landing entry.
    pmdg_landing_flaps_planned: Option<u8>,
    /// Takeoff-config-warning was active at any point during
    /// TakeoffRoll. PIREP flags this as a discipline issue.
    pmdg_takeoff_config_warning_seen: bool,
    /// Autobrake setting at touchdown.
    pmdg_autobrake_at_landing: Option<String>,
    /// FMC flight number captured at takeoff (for sanity check
    /// against the bid's flight number).
    pmdg_fmc_flight_number: Option<String>,
    /// 777-specific: ECL phases marked complete during the flight.
    /// Captured continuously — final value is the union of all
    /// phases that were ever ticked (NG3 stays None).
    pmdg_ecl_phases_complete: Option<[bool; 10]>,
    // FCU debounce state — kept around for the planned switch to the
    // standard `AUTOPILOT * VAR` SimVars (the Fenix LVar variant
    // proved unreliable as encoder click counters; we don't log
    // them for now). The four `last_logged_*` + `pending_*` fields
    // get re-wired when fcu_debounce() is called again.
    #[allow(dead_code)]
    last_logged_fcu_alt: Option<i32>,
    #[allow(dead_code)]
    last_logged_fcu_hdg: Option<i32>,
    #[allow(dead_code)]
    last_logged_fcu_spd: Option<i32>,
    #[allow(dead_code)]
    last_logged_fcu_vs: Option<i32>,
    #[allow(dead_code)]
    pending_fcu_alt: Option<(i32, DateTime<Utc>)>,
    #[allow(dead_code)]
    pending_fcu_hdg: Option<(i32, DateTime<Utc>)>,
    #[allow(dead_code)]
    pending_fcu_spd: Option<(i32, DateTime<Utc>)>,
    #[allow(dead_code)]
    pending_fcu_vs: Option<(i32, DateTime<Utc>)>,
}

/// What kind of touchdown this is from the pilot's perspective.
/// Discriminates real landings from training touch-and-goes so the
/// PIREP score doesn't get unfairly dragged down by deliberate
/// multi-touch flights.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TouchdownKind {
    /// The actual end-of-flight landing. The PIREP `landing_*`
    /// native fields and `landing_score` are derived from this one.
    /// Always the LAST entry in `touchdown_events` when a flight
    /// has been filed normally.
    FinalLanding,
    /// Training / planned multi-touch — aircraft touched the runway
    /// briefly, then climbed back out under power. Recorded for the
    /// audit trail but does NOT influence the score and does NOT
    /// count toward `bounce_count`.
    TouchAndGo,
}

/// One touchdown event during a flight. Captured at the moment the
/// touchdown is classified (= when either the bounce window closes
/// for a normal landing, or the T&G watcher confirms a climb-back).
/// Snapshot semantics — the values reflect the analyzer state at
/// classification time and don't update afterwards.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TouchdownEvent {
    pub timestamp: DateTime<Utc>,
    pub kind: TouchdownKind,
    /// Worst (= most negative) V/S observed in the touchdown window.
    pub peak_vs_fpm: f32,
    /// Highest G-force in the touchdown window.
    pub peak_g: f32,
    /// Touchdown coordinates so the PIREP-detail map can plot all
    /// touchdowns separately.
    pub lat: f64,
    pub lon: f64,
    /// Number of sub-bounces observed for THIS touchdown (= AGL
    /// excursions above the bounce threshold within the bounce
    /// window). A clean T&G has 0 sub-bounces; a sloppy flare
    /// might have 1-2.
    pub sub_bounces: u8,
}

/// Categorised assessment of a touchdown. Computed from peak descent
/// rate, peak G-force and bounce count once the touchdown window
/// closes. Stored on `FlightStats` and shipped both to the activity log
/// and the PIREP custom-fields map for VA review.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum LandingScore {
    /// |V/S| < 60 fpm, G < 1.2, no bounce — butter.
    Smooth,
    /// |V/S| < 240 fpm, G < 1.4 — typical good landing.
    Acceptable,
    /// |V/S| < 600 fpm or G < 1.8 — firm but not damaging.
    Firm,
    /// |V/S| < 1000 fpm or G < 1.8–2.5 — hard landing, inspection due.
    Hard,
    /// |V/S| ≥ 1000 fpm or extreme G — likely damaged.
    Severe,
}

impl LandingScore {
    fn classify(peak_vs_fpm: f32, peak_g: f32, bounces: u8) -> Self {
        let vs = peak_vs_fpm.abs();
        // Severity climbs with either V/S OR G — whichever is worse wins.
        let by_vs = if vs >= TOUCHDOWN_VS_SEVERE_FPM {
            Self::Severe
        } else if vs >= TOUCHDOWN_VS_HARD_FPM {
            Self::Hard
        } else if vs >= TOUCHDOWN_VS_FIRM_FPM {
            Self::Firm
        } else if vs >= TOUCHDOWN_VS_SMOOTH_FPM {
            Self::Acceptable
        } else {
            Self::Smooth
        };
        let by_g = if peak_g >= TOUCHDOWN_G_SEVERE {
            Self::Severe
        } else if peak_g >= TOUCHDOWN_G_HARD {
            Self::Hard
        } else if peak_g >= TOUCHDOWN_G_FIRM {
            Self::Firm
        } else if peak_g >= TOUCHDOWN_G_SMOOTH {
            Self::Acceptable
        } else {
            Self::Smooth
        };
        let mut score = by_vs.max(by_g);
        // Bounces bump the score one step down (Smooth → Acceptable etc.)
        // unless we're already at Severe.
        if bounces > 0 && score < Self::Severe {
            score = match score {
                Self::Smooth => Self::Acceptable,
                Self::Acceptable => Self::Firm,
                Self::Firm => Self::Hard,
                Self::Hard | Self::Severe => Self::Severe,
            };
        }
        score
    }

    fn label(self) -> &'static str {
        match self {
            Self::Smooth => "smooth",
            Self::Acceptable => "acceptable",
            Self::Firm => "firm",
            Self::Hard => "hard",
            Self::Severe => "severe",
        }
    }

    /// Severity ordering — for `max()` comparisons in `classify`.
    fn severity(self) -> u8 {
        match self {
            Self::Smooth => 0,
            Self::Acceptable => 1,
            Self::Firm => 2,
            Self::Hard => 3,
            Self::Severe => 4,
        }
    }

    /// Numeric score 0..100 for the phpVMS `score` field. Higher is
    /// better. Calibrated so a perfectly butter-smooth touchdown
    /// scores 100 and a structural-damage event scores ~0 — VAs that
    /// publish leaderboards then sort by score in the obvious way.
    fn numeric(self) -> i32 {
        match self {
            Self::Smooth => 100,
            Self::Acceptable => 80,
            Self::Firm => 60,
            Self::Hard => 30,
            Self::Severe => 0,
        }
    }
}

impl PartialOrd for LandingScore {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for LandingScore {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.severity().cmp(&other.severity())
    }
}

/// Snapshot of the exterior lights we track. Compared as a whole so we
/// can emit one log entry per "lights change" rather than one-per-light
/// on a config transition.
///
/// Wing + wheel-well were added in v0.2.4 as part of the PMDG Premium-
/// First sweep. They're optional in spirit because most generic SimVar
/// profiles don't expose them — the activity-log path checks the
/// underlying `light_wing` / `light_wheel_well` for `Some(_)` before
/// emitting a transition entry to avoid noise on aircraft that don't
/// report those switches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct LightsState {
    landing: bool,
    beacon: bool,
    strobe: bool,
    taxi: bool,
    nav: bool,
    logo: bool,
    /// PMDG-only on most installations — generic SimVars don't expose
    /// the wing-illumination switch separately on Boeings.
    wing: bool,
    /// NG3-only bonus (737 has a dedicated wheel-well light switch).
    wheel_well: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct ApState {
    master: bool,
    heading: bool,
    altitude: bool,
    nav: bool,
    approach: bool,
}

impl FlightStats {
    fn new() -> Self {
        Self {
            phase: FlightPhase::Boarding,
            ..Self::default()
        }
    }
}

/// Cached divert detection result, surfaced via `flight_status` so the
/// cockpit UI can render a "you landed at X, not the planned Y"
/// banner with action buttons. Populated once per flight when the FSM
/// reaches Arrived AND the aircraft is too far from `arr_airport`.
#[derive(Debug, Clone, Serialize)]
pub struct DivertHint {
    /// Best-guess actual landing airport ICAO. None when the local
    /// runways DB found nothing within `DIVERT_NEAREST_SEARCH_RADIUS_NM`
    /// (private strip, off-DB military, scenery-only field).
    pub actual_icao: Option<String>,
    /// What the bid had as the planned destination.
    pub planned_arr_icao: String,
    /// What the bid had as the planned alternate, if any. Used to
    /// boost UI confidence — when actual_icao == planned_alt_icao
    /// we say "diverted to planned alternate" with high confidence.
    pub planned_alt_icao: Option<String>,
    /// Distance from the touchdown point to the planned arrival,
    /// in nautical miles. Used in the banner copy.
    pub distance_to_planned_nmi: f64,
    /// One of: "alternate" (matched planned alt), "nearest" (closest
    /// airport in DB), "unknown" (nothing found, manual override).
    pub kind: &'static str,
}

#[derive(Serialize)]
pub struct ActiveFlightInfo {
    pirep_id: String,
    bid_id: i64,
    started_at: String,
    airline_icao: String,
    planned_registration: String,
    flight_number: String,
    dpt_airport: String,
    arr_airport: String,
    distance_nm: f64,
    position_count: u32,
    /// snake_case name of the current `FlightPhase` (e.g. "boarding", "climb").
    phase: String,
    /// ISO-8601 timestamps captured at major flight events. Each is `null`
    /// until the corresponding transition fires.
    block_off_at: Option<String>,
    takeoff_at: Option<String>,
    landing_at: Option<String>,
    block_on_at: Option<String>,
    landing_rate_fpm: Option<f32>,
    landing_g_force: Option<f32>,
    /// Set to `true` exactly ONCE — the first time the UI reads the flight
    /// status after an automatic resume (disk or remote). Lets the dashboard
    /// surface a one-time banner with a 10-second cancel window.
    was_just_resumed: bool,
    /// Departure stand from MSFS `ATC PARKING NAME` (snapshotted at
    /// the start of the flight). Empty until captured.
    dep_gate: Option<String>,
    /// Arrival stand from MSFS `ATC PARKING NAME` after BlocksOn.
    arr_gate: Option<String>,
    /// `ATC RUNWAY SELECTED` snapshotted while on Final.
    approach_runway: Option<String>,
    /// ISO-8601 UTC timestamp of the last successful position-post.
    /// Powers the cockpit's LIVE recording indicator — "X seconds
    /// ago" derived client-side.
    last_position_at: Option<String>,
    /// ISO-8601 UTC timestamp of the last successful PIREP heartbeat
    /// (`POST /pireps/{id}/update`). Surfaced for the debug panel so
    /// the pilot can see the keep-alive is firing — without it, phpVMS
    /// soft-deletes the PIREP after `acars.live_time` hours of inactive
    /// `updated_at`.
    last_heartbeat_at: Option<String>,
    /// Number of positions sitting in the offline queue. Non-zero
    /// means the streamer hit a network failure recently and the
    /// dashboard should warn the pilot.
    queued_position_count: u32,
    /// v0.4.1: ISO-8601 UTC timestamp wann der Streamer den Sim-
    /// Disconnect detektiert und den Flug pausiert hat. None = Flug
    /// läuft normal. Some(...) = Cockpit-Tab zeigt Resume-Banner.
    paused_since: Option<String>,
    /// v0.4.1: Snapshot der letzten bekannten Sim-Werte (LAT/LON/HDG/
    /// ALT/Fuel/ZFW) vor dem Disconnect. Pilot nutzt das um sein
    /// Flugzeug nach Sim-Restart wieder an die richtige Stelle zu
    /// setzen. None solange nicht pausiert.
    paused_last_known: Option<PausedSnapshot>,
    /// Divert hint when the FSM noticed the aircraft landed somewhere
    /// other than the planned `arr_airport`. The cockpit renders a
    /// banner ("you landed at LFBO, planned was LEBL — file as divert
    /// to LFBO?") with action buttons. None for normal arrivals.
    divert_hint: Option<DivertHint>,
    /// Number of touch-and-go events recorded so far. Always 0 on a
    /// routine A→B; non-zero on training flights or unstable approaches
    /// where the pilot bounced and went around. Surfaced as a small
    /// counter chip in the cockpit during/after Landing phase.
    touch_and_go_count: u32,
    /// Number of confirmed go-arounds (sustained climb-back from low
    /// approach). Independent of T&G — a missed-approach without
    /// ground contact only bumps this counter, not the T&G one.
    go_around_count: u32,
    // ---- v0.3.0 — SimBrief OFP Plan-Werte für Soll/Ist-Vergleich ----
    /// Plan-Block-Fuel aus dem SimBrief OFP (kg). None wenn der Pilot
    /// keine SimBrief-Verbindung im phpVMS-Profil hat oder das OFP
    /// nicht abgerufen werden konnte.
    planned_block_fuel_kg: Option<f32>,
    /// Plan-Trip-Burn aus dem SimBrief OFP (kg).
    planned_burn_kg: Option<f32>,
    /// Plan-Reserve-Fuel (kg).
    planned_reserve_kg: Option<f32>,
    /// Plan-ZFW (kg).
    planned_zfw_kg: Option<f32>,
    /// Plan-TOW (kg).
    planned_tow_kg: Option<f32>,
    /// Plan-LDW (kg).
    planned_ldw_kg: Option<f32>,
    /// Plan-Route aus dem OFP, ICAO-codiert. Wird im Briefing-Tab
    /// als monospace-string angezeigt.
    planned_route: Option<String>,
    /// Geplanter Alternate-Flughafen (ICAO).
    planned_alternate: Option<String>,
    // ---- v0.3.0: MAX-Werte für Overweight-Detection ----
    /// Maximum Zero-Fuel Weight (Strukturlimit). None bei Custom-
    /// Subfleets ohne MAX-Daten im OFP.
    planned_max_zfw_kg: Option<f32>,
    /// Maximum Takeoff Weight.
    planned_max_tow_kg: Option<f32>,
    /// Maximum Landing Weight.
    planned_max_ldw_kg: Option<f32>,
    // ---- v0.3.0: Live-Loadsheet-Werte aus dem aktuellen Sim-Snapshot ----
    /// Aktuelles Block-Fuel im Tank (kg). Live-Wert aus dem Sim,
    /// updated bei jedem Snapshot. Driver für die "Tank-Vorgang läuft"-
    /// Anzeige im Cockpit-Tab vor Pushback.
    sim_fuel_kg: Option<f32>,
    /// Aktuelles ZFW (kg). None wenn der Sim/das Aircraft-Profil das
    /// nicht meldet (z.B. Fenix mit Standard-SimVars).
    sim_zfw_kg: Option<f32>,
    /// Aktuelles Total-Weight (= TOW wenn am Boden voll beladen).
    /// None wenn der Sim das nicht meldet.
    sim_tow_kg: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SiteConfig {
    url: String,
}

#[derive(Serialize)]
pub struct LoginResult {
    profile: Profile,
    base_url: String,
}

/// Errors returned to the UI in a serializable shape.
/// `code` is a stable, machine-readable identifier the frontend uses for i18n.
/// `details` carries optional structured payload (e.g. list of missing
/// validation fields) so the UI can render a richer error than just a string.
#[derive(Debug, Serialize)]
pub struct UiError {
    code: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<serde_json::Value>,
}

impl UiError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: None,
        }
    }

    fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }
}

impl From<ApiError> for UiError {
    fn from(err: ApiError) -> Self {
        Self {
            code: err.code().to_string(),
            message: err.to_string(),
            details: None,
        }
    }
}

// ---- Helpers ----

/// Translate a flaps detent (0..5) into the Airbus-style label most
/// virtual airlines (and the activity-log readers) recognise. ONLY
/// used when no PMDG snapshot is available — for PMDG aircraft we
/// take the cockpit-exact Boeing label directly from
/// `pmdg.flap_handle_label` (see flap-detent change-detection block).
fn detent_label(detent: u8) -> &'static str {
    match detent {
        0 => "UP",
        1 => "1",
        2 => "1+F",
        3 => "2",
        4 => "3",
        _ => "FULL",
    }
}

/// Order Boeing flap-handle labels for direction-of-change detection.
/// Boeing 737 detents: UP/1/2/5/10/15/25/30/40 (rank 0..8).
/// Boeing 777 detents: UP/1/5/15/20/25/30 (rank 0..6, sharing the
/// numeric ordering with NG3 — both share the rank table since UP=0,
/// 1=1, 5=3, 15=5 etc., and the comparison only cares about
/// monotonicity, not exact values).
fn boeing_detent_rank(label: &str) -> u8 {
    match label {
        "UP" => 0,
        "1" => 1,
        "2" => 2,
        "5" => 3,
        "10" => 4,
        "15" => 5,
        "20" => 6,
        "25" => 7,
        "30" => 8,
        "40" => 9,
        _ => 0,
    }
}

// (Old position-derived sideslip helper deleted — replaced by direct
// `atan2(VEL_BODY_X, VEL_BODY_Z)` computation at the touchdown edge,
// matching GEES. The native body-frame velocities are accurate to
// the floating-point precision MSFS reports them at, no need to
// reconstruct anything from successive lat/lon.)

/// Push the current snapshot's V/S + bank into the approach buffer.
/// Called every step_flight tick during Approach + Final phases. The
/// buffer is capped at APPROACH_BUFFER_MAX so a 30-min hold doesn't
/// blow up memory; oldest entries get dropped first.
/// v0.5.25: erweitertes Approach-Sample mit allen Feldern die für
/// korrekte Stable-Approach-Gate-Analyse gebraucht werden:
///   * `agl_ft`: für Window-Filter (= nur AGL ≤ 1000 ft zählt fuer
///     den Stable-Approach-Gate-Score, ueber 1000 ft ist Vectoring
///     legitim und sollte nicht Pilot-bestrafen)
///   * `gs_kt`: für Soll-Glide-Slope-Berechnung
///     (target_vs ≈ gs_kt × 5.31 für 3°-Glide-Slope)
///   * `selected_runway`: für Late-RWY-Change-Detection. Wenn ATC
///     mid-final die Bahn wechselt, war Pilot zu Maneuver gezwungen,
///     das ist nicht "instabil"
///
/// Andere Naming-Konvention als das `ApproachSample` aus dem
/// `storage`-Crate (welches nur (vs_fpm, bank_deg) ist und an die
/// PIREP-Notes-Sparkline serialisiert wird) — dieses hier ist das
/// reichere internal Buffer-Sample.
#[derive(Debug, Clone)]
pub struct ApproachBufferSample {
    pub at: DateTime<Utc>,
    pub agl_ft: f32,
    pub msl_ft: f32,
    pub gs_kt: f32,
    pub ias_kt: f32,
    pub vs_fpm: f32,
    pub bank_deg: f32,
    pub heading_true_deg: f32,
    pub gear_position: f32,
    pub flaps_position: f32,
    pub selected_runway: Option<String>,
}

fn push_approach_sample(stats: &mut FlightStats, snap: &SimSnapshot) {
    stats.approach_buffer.push_back(ApproachBufferSample {
        at: Utc::now(),
        agl_ft: snap.altitude_agl_ft as f32,
        msl_ft: snap.altitude_msl_ft as f32,
        gs_kt: snap.groundspeed_kt,
        ias_kt: snap.indicated_airspeed_kt,
        vs_fpm: snap.vertical_speed_fpm,
        bank_deg: snap.bank_deg,
        heading_true_deg: snap.heading_deg_true,
        gear_position: snap.gear_position,
        flaps_position: snap.flaps_position,
        selected_runway: snap.selected_runway.clone(),
    });
    while stats.approach_buffer.len() > APPROACH_BUFFER_MAX {
        stats.approach_buffer.pop_front();
    }
}

/// Track the lowest AGL observed during the current Approach/Final
/// segment. The go-around detector compares the *current* AGL against
/// this minimum to decide whether a sustained climb-back has begun.
///
/// Reset to `None` when a go-around is classified (climb starts a
/// fresh approach window) and when the FSM exits Final into Landing
/// (a successful touchdown invalidates the approach minimum).
fn update_lowest_approach_agl(stats: &mut FlightStats, snap: &SimSnapshot) {
    let agl = snap.altitude_agl_ft as f32;
    // Only track positive AGL — a brief negative reading from a sim
    // glitch (terrain mesh hiccup) would poison the minimum and make
    // every subsequent sample look like a 200 ft go-around climb.
    if agl < 0.0 {
        return;
    }
    stats.lowest_agl_during_approach_ft = Some(
        stats
            .lowest_agl_during_approach_ft
            .map_or(agl, |prev| prev.min(agl)),
    );
}

/// Detect a go-around in progress: aircraft has climbed
/// `GO_AROUND_AGL_RECOVERY_FT` above the lowest AGL seen during this
/// approach, sustained `GO_AROUND_DWELL_SECS` of positive V/S above
/// `GO_AROUND_MIN_VS_FPM`, while airborne with engines running.
///
/// Returns `Some(FlightPhase::Climb)` when classified — the caller
/// should swap `next_phase` to that value so the FSM bumps back to
/// Climb. Side effects on confirmation: bumps `stats.go_around_count`,
/// pushes a human-readable line into `stats.pending_acars_logs`,
/// clears the lowest-AGL tracker (next descent starts a fresh
/// window), and clears the dwell timer.
fn check_go_around(
    stats: &mut FlightStats,
    snap: &SimSnapshot,
    now: DateTime<Utc>,
) -> Option<FlightPhase> {
    let lowest = stats.lowest_agl_during_approach_ft?;
    let agl = snap.altitude_agl_ft as f32;
    // Need at least *some* descent to have happened — otherwise a
    // pilot intercepting the glideslope from above would trip the
    // detector the moment we entered Approach.
    if lowest > 1500.0 {
        return None;
    }
    let conds = agl > lowest + GO_AROUND_AGL_RECOVERY_FT
        && snap.vertical_speed_fpm > GO_AROUND_MIN_VS_FPM
        && !snap.on_ground
        && snap.engines_running > 0;
    if conds {
        let pending = stats.go_around_climb_pending_since.get_or_insert(now);
        if (now - *pending).num_seconds() >= GO_AROUND_DWELL_SECS {
            stats.go_around_count = stats.go_around_count.saturating_add(1);
            tracing::info!(
                count = stats.go_around_count,
                agl_ft = agl,
                lowest_ft = lowest,
                vs_fpm = snap.vertical_speed_fpm,
                "go-around classified — phase reverting to Climb"
            );
            stats.pending_acars_logs.push(format!(
                "Go-around #{} at {:.0} ft AGL (V/S {:.0} fpm)",
                stats.go_around_count, agl, snap.vertical_speed_fpm
            ));
            stats.lowest_agl_during_approach_ft = None;
            stats.go_around_climb_pending_since = None;
            // v0.5.11: reset climb_peak_msl so the missed-approach
            // climb back up tracks fresh peaks. Without this, the
            // prior peak (the cruise altitude) stays as reference,
            // and the moment the aircraft descends back through it
            // for the second approach attempt, the low-altitude
            // Climb→Descent trigger could fire prematurely against
            // a stale peak.
            stats.climb_peak_msl = None;
            return Some(FlightPhase::Climb);
        }
    } else {
        // Conditions broke — reset dwell so the next sustained climb
        // starts a fresh timer. Without this, brief bumps would
        // accumulate seconds and falsely confirm the go-around.
        stats.go_around_climb_pending_since = None;
    }
    None
}

/// v0.5.11: detect entry into a holding pattern.
///
/// Triggered from Cruise or Approach when the aircraft sustains a
/// turn at constant altitude for `HOLDING_ENTRY_DWELL_SECS`.
/// Returns `true` once the dwell elapses; the caller then transitions
/// the FSM to `FlightPhase::Holding`.
///
/// Detection criteria each tick:
///   * |bank| > 15° (banked turn — standard rate is ~25-30°)
///   * |VS| < 200 fpm (level — real holds are within ±100 ft)
///
/// Once both fail (e.g. straight-and-level resumed), the dwell timer
/// resets so brief intermediate level segments during a 360° turn
/// don't accidentally fire (banked → level → banked is normal at
/// hold turn entry/exit).
fn check_holding_entry(
    stats: &mut FlightStats,
    snap: &SimSnapshot,
    now: DateTime<Utc>,
) -> bool {
    let in_hold_pattern = snap.bank_deg.abs() > HOLDING_BANK_THRESHOLD_DEG
        && snap.vertical_speed_fpm.abs() < HOLDING_VS_THRESHOLD_FPM;
    if in_hold_pattern {
        let pending = stats.holding_pending_since.get_or_insert(now);
        if (now - *pending).num_seconds() >= HOLDING_ENTRY_DWELL_SECS {
            tracing::info!(
                bank_deg = snap.bank_deg,
                vs_fpm = snap.vertical_speed_fpm,
                agl_ft = snap.altitude_agl_ft,
                "holding pattern detected — entering Holding phase"
            );
            stats.holding_pending_since = None;
            return true;
        }
    } else {
        stats.holding_pending_since = None;
    }
    false
}

// ---- v0.5.11: AGL-derivative touchdown VS estimator -------------------
//
// Pilot's deep analysis pinpointed the bug class in v0.5.5+:
// "approach_vs_min_fpm" picks the most-negative VS across the WHOLE
// approach. A pre-flare descent at -1346 fpm @ 943 ft AGL would win
// over the actual gentle touchdown at -200 fpm @ 0 ft. Result:
// "phantom hard landing" reports.
//
// Correct approach: ONLY count samples close to the ground for
// touchdown VS. The window-tier strategy (LandingRate-1.lua, Volanta)
// uses geometric AGL change over a tight time window centred on the
// actual touchdown moment. Pre-flare descent gets ignored entirely.
//
// Window tiers (most preferred first):
//   1. 750 ms   — dense plugin-fed samples (≥5 samples)
//   2. 1000 ms  — typical RREF cadence
//   3. 1500 ms  — slower RREF (still inside flare)
//   4. 2000 ms  — wider for very sparse RREF
//   5. 3000 ms  — last "real" tier
//   6. 12000 ms — sparse fallback (the famous 9-sec gap in pilot
//                 reports). Lower confidence but better than nothing.
//
// All tiers must satisfy:
//   * Last sample AGL ≤ 5 ft  (genuine touchdown frame, not Sim-glitch)
//   * Earliest sample AGL ≤ 250 ft (= we're below pattern altitude)
//   * Sample count ≥ MIN_SAMPLES (5 for short tiers, 3 for sparse)
//   * Result < 0 fpm (no positive "landing rate" is physical)

const TD_WINDOW_TIERS_MS: &[(i64, usize)] = &[
    (750, 5),
    (1000, 5),
    (1500, 5),
    (2000, 4),
    (3000, 3),
    (12000, 3),
];
const TD_AGL_MAX_AT_TOUCHDOWN_FT: f32 = 5.0;
const TD_AGL_MAX_AT_WINDOW_START_FT: f32 = 250.0;

/// Result of a touchdown VS estimation, including diagnostics so the
/// rest of the system can log which window won and its sample
/// density.
#[derive(Debug, Clone, Copy)]
pub struct TouchdownVsEstimate {
    /// Negative fpm (descent rate). Always strictly < 0.
    pub fpm: f32,
    pub source: &'static str,
    pub window_ms: i64,
    pub sample_count: usize,
}

/// Estimate touchdown VS from snapshot_buffer AGL samples.
///
/// Walks the window tiers in order (shortest first); returns the
/// first tier that satisfies all guards. Returns `None` when no
/// window has enough samples or all windows give a non-negative
/// (= unphysical) result.
fn estimate_xplane_touchdown_vs_from_agl(
    buffer: &std::collections::VecDeque<TelemetrySample>,
    touchdown_at: DateTime<Utc>,
) -> Option<TouchdownVsEstimate> {
    for (window_ms, min_samples) in TD_WINDOW_TIERS_MS {
        let win_start = touchdown_at - chrono::Duration::milliseconds(*window_ms);
        let samples: Vec<&TelemetrySample> = buffer
            .iter()
            .filter(|s| s.at >= win_start && s.at <= touchdown_at)
            .collect();
        if samples.len() < *min_samples {
            continue;
        }
        // Touchdown sample (latest) must really be at the ground.
        // v0.5.12: accept on_ground=true OR AGL ≤ 5 ft. MSFS reports
        // AGL ≈ 9.4 ft (sometimes 13 ft) even when on_ground=true at
        // the physical contact frame — this is a sim quirk, not a
        // pre-touchdown sample. Strict AGL ≤ 5 ft alone would reject
        // every MSFS touchdown.
        let last = samples.last()?;
        let is_at_touchdown =
            last.on_ground || last.agl_ft <= TD_AGL_MAX_AT_TOUCHDOWN_FT;
        if !is_at_touchdown {
            continue;
        }
        // Window-start sample must already be near the ground (i.e.
        // we're not picking up a high-altitude pre-flare descent).
        let first = samples.first()?;
        if first.agl_ft > TD_AGL_MAX_AT_WINDOW_START_FT {
            continue;
        }
        // Geometric descent rate via avg-AGL midpoint trick
        // (LandingRate-1.lua method). For a linear descent the avg
        // is the time-midpoint; the rate from midpoint → now over
        // (timespan / 2) is the geometric descent rate.
        let avg_agl: f32 =
            samples.iter().map(|s| s.agl_ft).sum::<f32>() / samples.len() as f32;
        let timespan_sec =
            (last.at - first.at).num_milliseconds() as f32 / 1000.0;
        if timespan_sec < 0.2 {
            continue;
        }
        let agl_midpoint = last.agl_ft - avg_agl;
        let fpm = (agl_midpoint / (timespan_sec / 2.0)) * 60.0;
        // Reject positive (climbing) results — physically wrong for
        // a touchdown estimate.
        if !fpm.is_finite() || fpm >= 0.0 {
            continue;
        }
        return Some(TouchdownVsEstimate {
            fpm,
            source: match *window_ms {
                750 => "agl_750ms",
                1000 => "agl_1000ms",
                1500 => "agl_1500ms",
                2000 => "agl_2000ms",
                3000 => "agl_3000ms",
                12000 => "agl_sparse_12s",
                _ => "agl_unknown",
            },
            window_ms: *window_ms,
            sample_count: samples.len(),
        });
    }
    None
}

/// Discard non-negative VS values. Used as a safety filter on every
/// fallback source (sampler, plugin, buffer-min). A positive
/// "landing rate" is physically impossible — the airframe is by
/// definition descending (or settled at 0) at the touchdown frame.
/// Plugin / buffer values can briefly read positive due to gear
/// oscillation rebound; we never want those as the published
/// landing rate.
fn negative_only(value: Option<f32>) -> Option<f32> {
    value.filter(|v| v.is_finite() && *v < 0.0)
}

// ---- v0.5.13: Lua-style adaptive 30-sample AGL-Δ estimator ------------
//
// Direct port of LandingRate-1.lua's algorithm (Dan Berry, 2014+,
// X-Plane.org "A New Landing Rate Display"). Matches what Volanta uses
// internally for X-Plane touchdown capture.
//
// Key insight: instead of fixed time windows (750ms/1s/1.5s/...) that
// can pick up pre-flare contamination on sparse RREF feeds, take a
// FIXED COUNT of recent samples (30) and adapt the time window to
// however dense the buffer is.
//
// Effect:
//   * High-fps sim (60+ fps): 30 samples ≈ 0.5 s window — tight, captures
//     just the flare's final phase
//   * Mid-fps sim (30 fps): 30 samples ≈ 1 s window — Lua's reference point
//   * Low-fps sim (10 fps): 30 samples ≈ 3 s window — wider window to
//     compensate for sparse data
//
// AGL guards (TD ≤ 5 ft / on_ground=true, window-start ≤ 250 ft) are
// identical to the time-tier estimator — they're what prevents
// pre-flare descent contamination, regardless of window size.
//
// Used ONLY for the X-Plane priority path. MSFS keeps the older
// time-tier `estimate_xplane_touchdown_vs_from_agl` as fallback (when
// the latched MSFS SimVar is null) — that path is GEES-aligned and
// validated against pilot reports; we don't change MSFS behaviour.
const LUA_STYLE_SAMPLE_COUNT: usize = 30;
const LUA_STYLE_MIN_SAMPLES: usize = 5;

fn estimate_xplane_touchdown_vs_lua_style(
    buffer: &std::collections::VecDeque<TelemetrySample>,
    touchdown_at: DateTime<Utc>,
) -> Option<TouchdownVsEstimate> {
    // Collect samples up to (and including) touchdown_at, in chronological order.
    let mut samples: Vec<&TelemetrySample> =
        buffer.iter().filter(|s| s.at <= touchdown_at).collect();
    samples.sort_by_key(|s| s.at);
    let n = samples.len();
    if n < LUA_STYLE_MIN_SAMPLES {
        return None;
    }
    let take = n.min(LUA_STYLE_SAMPLE_COUNT);
    let recent = &samples[n - take..];

    // Touchdown sample (latest) must really be at the ground.
    let last = *recent.last()?;
    let is_at_touchdown =
        last.on_ground || last.agl_ft <= TD_AGL_MAX_AT_TOUCHDOWN_FT;
    if !is_at_touchdown {
        return None;
    }
    // Window-start sample must already be in approach footprint
    // (no high-altitude pre-flare contamination).
    let first = *recent.first()?;
    if first.agl_ft > TD_AGL_MAX_AT_WINDOW_START_FT {
        return None;
    }

    let avg_agl: f32 =
        recent.iter().map(|s| s.agl_ft).sum::<f32>() / take as f32;
    let timespan_sec =
        (last.at - first.at).num_milliseconds() as f32 / 1000.0;
    if timespan_sec < 0.2 {
        return None;
    }
    let agl_midpoint = last.agl_ft - avg_agl;
    let fpm = (agl_midpoint / (timespan_sec / 2.0)) * 60.0;
    if !fpm.is_finite() || fpm >= 0.0 {
        return None;
    }
    Some(TouchdownVsEstimate {
        fpm,
        source: "lua_30_sample",
        window_ms: (timespan_sec * 1000.0) as i64,
        sample_count: take,
    })
}

/// Compute population standard deviations of V/S and bank over the
/// approach buffer. Returns `(None, None)` when fewer than 3 samples
/// (insufficient signal). Single-pass formula (Welford's algorithm
/// would be marginally more numerically stable but at this scale it
/// makes no difference).
fn compute_approach_stddev(
    buf: &std::collections::VecDeque<ApproachBufferSample>,
) -> (Option<f32>, Option<f32>) {
    let n = buf.len();
    if n < 3 {
        return (None, None);
    }
    let mut sum_vs = 0.0_f64;
    let mut sum_bank = 0.0_f64;
    for s in buf.iter() {
        sum_vs += s.vs_fpm as f64;
        sum_bank += s.bank_deg as f64;
    }
    let mean_vs = sum_vs / n as f64;
    let mean_bank = sum_bank / n as f64;
    let mut sq_vs = 0.0_f64;
    let mut sq_bank = 0.0_f64;
    for s in buf.iter() {
        let dv = s.vs_fpm as f64 - mean_vs;
        let db = s.bank_deg as f64 - mean_bank;
        sq_vs += dv * dv;
        sq_bank += db * db;
    }
    let var_vs = sq_vs / n as f64;
    let var_bank = sq_bank / n as f64;
    (Some(var_vs.sqrt() as f32), Some(var_bank.sqrt() as f32))
}

/// v0.5.25 Approach-Stability v2 — kompletter Result-Struct.
#[derive(Debug, Clone, Default)]
pub struct ApproachStabilityV2 {
    pub vs_deviation_fpm: Option<f32>,
    pub max_vs_deviation_below_500_fpm: Option<f32>,
    pub bank_stddev_filtered_deg: Option<f32>,
    pub runway_changed_late: bool,
    pub stable_at_gate: Option<bool>,
    pub window_sample_count: u32,
    /// V/S-Jerk: mean |Δvs| sample-to-sample. Sim/Aircraft-agnostic.
    pub vs_jerk_fpm: Option<f32>,
    /// IAS-Stddev im Gate (Speed-Stability).
    pub ias_stddev_kt: Option<f32>,
    /// Mind. ein Sample im Gate hatte V/S < -1000 fpm.
    pub excessive_sink: bool,
    /// Gear+Flaps am 1000-ft-Sample in Landing-Konfig?
    pub stable_config: Option<bool>,
    /// HAT (statt AGL) als Window-Filter genutzt?
    pub used_hat: bool,
}

/// v0.5.25: Stable-Approach-Gate-konformes Stability-Maß.
///
/// FAA AC 120-71B / EASA SUPP-32 definieren Stable-Approach-Gate als
/// 1000 ft HAT (Height Above Touchdown). Innerhalb des Gates muessen
/// alle Parameter im Toleranzband sein, sonst go-around.
///
/// Window-Logik:
///   * Wenn `arr_airport_elevation_ft` bekannt: HAT = msl_ft − elevation,
///     Filter auf HAT ≤ 1000 ft (= ueber Mountain-Airports korrekt)
///   * Sonst: Fallback auf AGL ≤ 1000 ft (= AGL fluktuiert ueber Berg-
///     ruecken aber funktioniert für Flachland-Airports)
///
/// Stability-Metrics (alle im Gate-Window):
///   1. V/S-Jerk = mean |vs[i]−vs[i−1]| (PRIMAER, sim/aircraft-agnostic)
///   2. Bank-σ filtered (Vector-Windows ausgenommen, 5s nach RWY-Change)
///   3. IAS-σ (Speed-Stability)
///   4. Excessive-Sink-Flag (≥1 Sample mit V/S < -1000 fpm)
///   5. Stable-Config-Flag (Gear≥99% AND Flaps≥70% am gate-Eintritt)
///
/// Sekundaer (informativ, NICHT score-relevant):
///   6. V/S-Deviation vs 3°-ILS-Profil (target_vs = -gs_kt × 5.31)
///      — falsch fuer GA-Visual-Approach, deshalb nur informativ
fn compute_approach_stability_v2(
    buf: &std::collections::VecDeque<ApproachBufferSample>,
    arr_elevation_ft: Option<f32>,
) -> ApproachStabilityV2 {
    let mut out = ApproachStabilityV2::default();

    // 1) Window-Filter: HAT bevorzugt, AGL als Fallback.
    let use_hat = arr_elevation_ft.is_some();
    out.used_hat = use_hat;
    let height_for = |s: &ApproachBufferSample| -> f32 {
        match arr_elevation_ft {
            Some(elev) => s.msl_ft - elev,
            None => s.agl_ft,
        }
    };
    let gate_samples: Vec<&ApproachBufferSample> = buf.iter()
        .filter(|s| {
            let h = height_for(s);
            h > 0.0 && h <= 1000.0
        })
        .collect();
    out.window_sample_count = gate_samples.len() as u32;
    if gate_samples.len() < 3 {
        return out;
    }

    // 2) RWY-Change-Detection ueber GANZEN Buffer.
    let runway_changes: Vec<DateTime<Utc>> = {
        let mut changes = Vec::new();
        let mut prev_rwy: Option<&str> = None;
        for s in buf.iter() {
            let curr = s.selected_runway.as_deref();
            if let (Some(p), Some(c)) = (prev_rwy, curr) {
                if p != c {
                    changes.push(s.at);
                    if height_for(s) < 1500.0 {
                        out.runway_changed_late = true;
                    }
                }
            }
            if curr.is_some() {
                prev_rwy = curr;
            }
        }
        changes
    };
    let in_vector_window = |t: DateTime<Utc>| -> bool {
        runway_changes.iter().any(|&change| {
            (t - change).num_milliseconds().abs() <= 5_000
        })
    };

    // 3) V/S-Jerk (PRIMAER): mean |Δvs| sample-to-sample.
    //    Sim/Aircraft-agnostic — funktioniert für Jet, Turboprop, GA gleich.
    if gate_samples.len() >= 2 {
        let mut jerk_sum = 0.0_f64;
        for w in gate_samples.windows(2) {
            jerk_sum += (w[1].vs_fpm - w[0].vs_fpm).abs() as f64;
        }
        out.vs_jerk_fpm = Some((jerk_sum / (gate_samples.len() - 1) as f64) as f32);
    }

    // 4) IAS-Stddev (Speed-Stability).
    let n = gate_samples.len() as f64;
    let mean_ias = gate_samples.iter().map(|s| s.ias_kt as f64).sum::<f64>() / n;
    let var_ias = gate_samples.iter()
        .map(|s| (s.ias_kt as f64 - mean_ias).powi(2))
        .sum::<f64>() / n;
    out.ias_stddev_kt = Some(var_ias.sqrt() as f32);

    // 5) Excessive-Sink-Flag.
    out.excessive_sink = gate_samples.iter().any(|s| s.vs_fpm < -1000.0);

    // 6) Stable-Config: Gear+Flaps am 1000-ft-Sample (= aeltester
    //    Sample im Gate, = der mit hoechster Hoehe).
    if let Some(gate_entry) = gate_samples.iter()
        .max_by(|a, b| height_for(a).partial_cmp(&height_for(b)).unwrap())
    {
        let gear_ok = gate_entry.gear_position >= 0.99;
        let flaps_ok = gate_entry.flaps_position >= 0.70;
        out.stable_config = Some(gear_ok && flaps_ok);
    }

    // 7) V/S-Deviation vs 3° ILS-Profil (sekundaer, nur informativ).
    let mut sum_dev = 0.0_f64;
    let mut max_dev_below_500 = 0.0_f32;
    for s in &gate_samples {
        let target_vs = -(s.gs_kt as f64) * 5.31;
        let dev = (s.vs_fpm as f64 - target_vs).abs();
        sum_dev += dev;
        if height_for(s) <= 500.0 {
            let dev_f32 = dev as f32;
            if dev_f32 > max_dev_below_500 {
                max_dev_below_500 = dev_f32;
            }
        }
    }
    let mean_dev = sum_dev / gate_samples.len() as f64;
    out.vs_deviation_fpm = Some(mean_dev as f32);
    if max_dev_below_500 > 0.0 {
        out.max_vs_deviation_below_500_fpm = Some(max_dev_below_500);
    }

    // 8) Bank-Stddev filtered.
    let bank_filtered: Vec<f32> = gate_samples
        .iter()
        .filter(|s| !in_vector_window(s.at))
        .map(|s| s.bank_deg)
        .collect();
    if bank_filtered.len() >= 3 {
        let n = bank_filtered.len() as f64;
        let mean = bank_filtered.iter().map(|&b| b as f64).sum::<f64>() / n;
        let var = bank_filtered.iter()
            .map(|&b| (b as f64 - mean).powi(2))
            .sum::<f64>() / n;
        out.bank_stddev_filtered_deg = Some(var.sqrt() as f32);
    }

    // 9) Composite Stable-At-Gate Indikator.
    //    PRIMARY-Maße: jerk < 100 AND bank_sd < 5 AND ias_sd < 10
    //                AND no_excessive_sink AND stable_config
    let jerk_ok = out.vs_jerk_fpm.map(|j| j < 100.0).unwrap_or(false);
    let bank_ok = out.bank_stddev_filtered_deg.map(|b| b < 5.0).unwrap_or(true);
    let ias_ok = out.ias_stddev_kt.map(|i| i < 10.0).unwrap_or(true);
    let config_ok = out.stable_config.unwrap_or(true); // None = unbekannt, kein blocker
    let stable = jerk_ok && bank_ok && ias_ok && !out.excessive_sink && config_ok;
    out.stable_at_gate = Some(stable);

    out
}

/// Map a numeric landing score (0-100, finer granularity than the
/// 5-tier `LandingScore`) to a letter grade. Same A+/A/B+/B/C/D/F
/// scale as the Landing Analyzer reference tool. Boundaries chosen
/// so a clean butter touchdown (LandingScore::Smooth = 100) earns
/// A+, an Acceptable (80) earns B, Firm (60) earns C, etc.
fn letter_grade(numeric: i32) -> &'static str {
    match numeric {
        95..=100 => "A+",
        88..=94 => "A",
        82..=87 => "B+",
        75..=81 => "B",
        65..=74 => "C",
        50..=64 => "D",
        _ => "F",
    }
}

/// Render the full callsign as "DLH 155" (airline + space + number) when
/// the airline ICAO is known, falling back to bare "155" otherwise.
fn format_callsign(airline_icao: &str, flight_number: &str) -> String {
    if airline_icao.is_empty() {
        flight_number.to_string()
    } else {
        format!("{} {}", airline_icao, flight_number)
    }
}

/// Render an Option<f32> as kg with sensible "n/a" fallback for the
/// activity-log detail string. Used by the fuel/weight diagnostics.
fn fmt_kg(v: Option<f32>) -> String {
    match v {
        Some(n) if n > 0.0 => format!("{:.0} kg", n),
        Some(_) | None => "n/a".to_string(),
    }
}
fn fmt_kg_f64(v: Option<f64>) -> String {
    match v {
        Some(n) if n > 0.0 => format!("{:.0} kg", n),
        Some(_) | None => "n/a".to_string(),
    }
}

/// Emit a fuel/weight diagnostic line into the activity log at the
/// phase transitions where AeroACARS captures values into PIREP
/// fields. Lets the pilot verify after the fact what the app saw.
///
/// The four meaningful transitions:
///   * Pushback / TaxiOut → "Block fuel" capture window closes (well,
///     it's a running peak, but this is when the pilot rolls off the
///     stand and the value should be locked in).
///   * Takeoff           → TOW + takeoff fuel snapshotted into stats.
///   * Landing           → LDW + landing fuel snapshotted.
///   * BlocksOn          → final fuel + computed Fuel Used.
///
/// Other phases produce no entry.
fn log_fuel_weight_at_phase(
    app: &AppHandle,
    flight: &ActiveFlight,
    phase: FlightPhase,
    snap: &SimSnapshot,
) {
    // Take ALL values out under the lock, then drop it before calling
    // log_activity_handle (which reaches into AppState for the activity
    // log mutex — never hold two mutexes across an external call).
    let (
        block_fuel,
        takeoff_fuel,
        takeoff_weight,
        landing_fuel,
        landing_weight,
        last_fuel,
    ) = {
        let stats = flight.stats.lock().expect("flight stats");
        (
            stats.block_fuel_kg,
            stats.takeoff_fuel_kg,
            stats.takeoff_weight_kg,
            stats.landing_fuel_kg,
            stats.landing_weight_kg,
            stats.last_fuel_kg,
        )
    };

    let (message, detail) = match phase {
        FlightPhase::Pushback | FlightPhase::TaxiOut => {
            // Block fuel is a running peak — at this point we expect
            // it to roughly match `snap.fuel_total_kg` unless the
            // pilot just defueled. ZFW/payload/empty come straight
            // from the SimVar block.
            (
                "Fuel & Weight @ Block-off".to_string(),
                Some(format!(
                    "Block fuel {} | Live fuel {:.0} kg | ZFW {} | Payload {} | OEW {} | Total {}",
                    fmt_kg(block_fuel),
                    snap.fuel_total_kg,
                    fmt_kg(snap.zfw_kg),
                    fmt_kg(snap.payload_kg),
                    fmt_kg(snap.empty_weight_kg),
                    fmt_kg(snap.total_weight_kg),
                )),
            )
        }
        FlightPhase::Takeoff => {
            (
                "Fuel & Weight @ Takeoff".to_string(),
                Some(format!(
                    "Takeoff fuel {} | TOW {} | Block fuel was {}",
                    fmt_kg(takeoff_fuel),
                    fmt_kg_f64(takeoff_weight),
                    fmt_kg(block_fuel),
                )),
            )
        }
        FlightPhase::Landing => {
            (
                "Fuel & Weight @ Landing".to_string(),
                Some(format!(
                    "Landing fuel {} | LDW {} | TOW was {}",
                    fmt_kg(landing_fuel),
                    fmt_kg_f64(landing_weight),
                    fmt_kg_f64(takeoff_weight),
                )),
            )
        }
        FlightPhase::BlocksOn => {
            // Fuel Used = block fuel (peak before takeoff) minus
            // current fuel. Only show when both are valid and
            // block > current — otherwise the math is meaningless
            // (e.g. defuel during taxi, fuel SimVar missing).
            let fuel_used = match (block_fuel, last_fuel) {
                (Some(b), Some(c)) if b > 0.0 && b > c => Some(b - c),
                _ => None,
            };
            (
                "Fuel & Weight @ Block-on".to_string(),
                Some(format!(
                    "Final fuel {} | Fuel used {} | Landing fuel was {}",
                    fmt_kg(last_fuel),
                    fmt_kg(fuel_used),
                    fmt_kg(landing_fuel),
                )),
            )
        }
        // No diagnostic for other phase transitions.
        _ => return,
    };

    log_activity_handle(app, ActivityLevel::Info, message, detail);
}

// ---- Activity log ----

/// How many position-streamer ticks between persistence flushes. Stats
/// are snapshotted to disk every Nth post so a Tauri crash mid-flight
/// can resume with reasonably fresh distance/fuel/phase data instead of
/// the zeros we wrote at flight_start. 5 ticks ≈ every ~25–150 s
/// depending on phase cadence — fine for the cost of a small JSON write.
const STATS_PERSIST_EVERY_TICKS: u32 = 5;

/// Touchdown analyzer window in seconds — how long after the first
/// on-ground tick we keep refining peak G-force and peak descent rate
/// before locking in the final score. 5 s is enough to catch the spike
/// (which can lag the initial contact by a tick or two) without bleeding
/// into the rollout. Bounded above by `TOUCHDOWN_BUFFER_SECS` (the ring
/// buffer's reach) and below by `TOUCHDOWN_G_WINDOW_MS`.
const TOUCHDOWN_WINDOW_SECS: i64 = 5;

/// V/S sampling window around the on-ground edge, in milliseconds.
/// We pick the worst-case (most negative) V/S from buffered samples
/// within ±`/2` of the touchdown timestamp instead of using a single
/// tick. 500 ms matches BeatMyLanding's `TouchdownWindowMs` — large
/// enough that the actual contact frame falls inside even when the
/// edge detection is one tick late, small enough not to pull in
/// pre-flare descent or post-bounce rebound values.
const TOUCHDOWN_VS_WINDOW_MS: i64 = 500;

/// Peak-G search window, in milliseconds AFTER the on-ground edge.
/// G spike from gear strut compression typically peaks 100–300 ms
/// after first contact and is over within ~600 ms. We use 800 ms —
/// tighter than BeatMyLanding's `MaxGWindowMs` (1500) because the
/// later ms reach into strut REBOUND territory which inflates G with
/// pseudo-impact spikes, hurting the score on otherwise-clean
/// landings (the 1.24 G "acceptable" downgrade we kept seeing on
/// pilot reports of butter touchdowns).
const TOUCHDOWN_G_WINDOW_MS: i64 = 800;

/// Maximum window in seconds during which we still count subsequent
/// liftoffs as bounces of the original touchdown (rather than a fresh
/// landing event). Matches BeatMyLanding's `BounceWindow`.
const BOUNCE_WINDOW_SECS: i64 = 8;

/// AGL altitude (ft) the aircraft must climb above before we can
/// detect a bounce. Below this, on-ground flickers are noise (gear
/// strut oscillation, sloppy SimVar updates). Matches BeatMyLanding's
/// `BounceRadioAltThresholdFeet`. f64 to match `SimSnapshot::altitude_agl_ft`.
const BOUNCE_AGL_THRESHOLD_FT: f64 = 35.0;

/// AGL altitude (ft) the aircraft must come back below to count one
/// bounce. The detector arms when AGL crosses up through THRESHOLD
/// and fires when it crosses back down through RETURN. Matches
/// BeatMyLanding's `BounceRadioAltReturnFeet`.
const BOUNCE_AGL_RETURN_FT: f64 = 5.0;

/// Max samples retained in `FlightStats::approach_buffer`. Position
/// streamer ticks every 5-8 s during Approach/Final, so 120 samples
/// ≈ 10-15 min — plenty to cover even a long ILS approach.
const APPROACH_BUFFER_MAX: usize = 120;

/// Primary rollout-end trigger: groundspeed at which we consider the
/// pilot has finished using the runway and is about to clear at a
/// high-speed taxiway exit. Real pilots almost never decelerate to a
/// full stop on the runway — they brake to ~40 kt and turn off at the
/// next exit. Using a 40 kt threshold gives a "runway distance used"
/// metric that matches what other ACARS tools (BeatMyLanding,
/// vmsACARS) and FOQA reports show.
///
/// Source for 40 kt: ICAO Doc 9981 (Procedures for Air Navigation
/// Services — Aerodromes) recommends rapid-exit taxiways be designed
/// for a 65 km/h (≈ 35 kt) exit speed; FAA AC 150/5300-13B uses 60 kt
/// for high-speed RETs but pilots routinely brake further to ~40 kt
/// before committing to the turn.
const ROLLOUT_EXIT_GS_KT: f32 = 40.0;

/// Secondary trigger: full stop on the runway (rare in practice — long
/// rwys at uncontrolled fields, GA touch-and-go's gone wrong). Kept
/// as a hard floor so a pilot who really does brake to a stop still
/// gets a finalised metric.
const ROLLOUT_STOP_GS_KT: f32 = 5.0;

/// Tertiary trigger: heading has rotated this many degrees from the
/// touchdown heading, meaning the aircraft has clearly turned onto a
/// taxiway and is no longer using the runway. Stops the accumulator
/// from counting the taxi-out distance against the rollout figure.
/// Computed signed (wraparound-safe) and compared in absolute value.
const ROLLOUT_HEADING_DEVIATION_DEG: f32 = 30.0;

/// Hard-landing thresholds, ordered worst-first. The first row that
/// the peak |V/S| or G-force breaches wins; combined with
/// `bounce_count` this maps to a `LandingScore`.
///
/// Sources verified rather than guessed (the older 60 / 240 / 600 fpm
/// table downgraded clean -91 fpm landings to Acceptable, which the
/// pilot rightly called out as unrealistic):
///
///   * **Boeing 737 FCOM**: Hard-Landing-Inspection > 600 fpm OR
///     > 1.7 G. Severe > 1000 fpm OR > 2.6 G.
///   * **Airbus A320 FCOM**: Max recommended TD sink 360 fpm; hard-
///     landing inspection ≈ 600 fpm OR 2.6 G.
///   * **vmsACARS default rules.yml** (extracted from the shipped PHP
///     module): single `HARD_LANDING` rule at parameter = 500 fpm.
///   * **Lufthansa FOQA category bands** (publicly documented):
///       Soft   < 200 fpm
///       Normal 200–400 fpm
///       Firm   400–600 fpm
///       Hard   > 600 fpm
///       Heavy  > 1000 fpm
///   * **Community ACARS conventions** (Smartcars, BeatMyLanding
///     toast colour bands, LandingRate.com): butter < 200 fpm,
///     good < 400, ok < 600, bad above.
///
/// Consensus → these tiers:
///
/// ```text
///                  V/S        G
///   Smooth         < 200 fpm  < 1.20 G   (butter / greaser)
///   Acceptable     < 400 fpm  < 1.40 G   (normal LH FOQA)
///   Firm           < 600 fpm  < 1.70 G   (firm but accepted)
///   Hard           < 1000 fpm < 2.10 G   (FCOM inspection trigger)
///   Severe         ≥ 1000 fpm ≥ 2.10 G   (structural concern)
/// ```
// V/S boundaries — fpm, |abs| at touchdown:
const TOUCHDOWN_VS_SEVERE_FPM: f32 = 1000.0;
const TOUCHDOWN_VS_HARD_FPM: f32 = 600.0;
const TOUCHDOWN_VS_FIRM_FPM: f32 = 400.0;
const TOUCHDOWN_VS_SMOOTH_FPM: f32 = 200.0;
// G boundaries — peak G in the touchdown window:
const TOUCHDOWN_G_SEVERE: f32 = 2.10;
const TOUCHDOWN_G_HARD: f32 = 1.70;
const TOUCHDOWN_G_FIRM: f32 = 1.40;
const TOUCHDOWN_G_SMOOTH: f32 = 1.20;

/// AP master toggles only emit a log entry once they've held for this
/// many seconds. Stops a flickering / pulsed LVar (Fenix's momentary
/// `S_FCU_AP*` was the original culprit) from spamming the log with
/// alternating ENGAGED / OFF lines.
const AP_DEBOUNCE_SECS: i64 = 2;

/// Suppress repeated "same" activity entries fired within this window.
/// React StrictMode in dev double-mounts effects, so the login + session-
/// restore commands run twice on startup; we don't want the user to see
/// every event duplicated. Five seconds is wide enough to dedupe those
/// without merging genuine repeats during a long flight.
const ACTIVITY_DEDUPE_WINDOW_SECS: i64 = 5;

fn would_dedupe(log: &VecDeque<ActivityEntry>, now: DateTime<Utc>, message: &str) -> bool {
    log.back().is_some_and(|prev| {
        prev.message == message
            && (now - prev.timestamp).num_seconds() < ACTIVITY_DEDUPE_WINDOW_SECS
    })
}

/// Push an entry into the in-memory activity log. Caps the buffer at
/// `ACTIVITY_LOG_CAPACITY` so a long-running session doesn't leak memory.
/// Also mirrors the entry to `tracing` at the matching level so command-
/// line debugging stays consistent. Identical messages within
/// `ACTIVITY_DEDUPE_WINDOW_SECS` of each other collapse to a single line.
fn log_activity(
    state: &tauri::State<'_, AppState>,
    level: ActivityLevel,
    message: impl Into<String>,
    detail: Option<String>,
) {
    let now = Utc::now();
    let message = message.into();
    {
        let log = state.activity_log.lock().expect("activity_log lock");
        if would_dedupe(&log, now, &message) {
            return;
        }
    }
    let entry = ActivityEntry {
        timestamp: now,
        level,
        message,
        detail,
    };
    match level {
        ActivityLevel::Info => {
            tracing::info!(message = %entry.message, detail = ?entry.detail, "activity")
        }
        ActivityLevel::Warn => {
            tracing::warn!(message = %entry.message, detail = ?entry.detail, "activity")
        }
        ActivityLevel::Error => {
            tracing::error!(message = %entry.message, detail = ?entry.detail, "activity")
        }
    }
    let mut log = state.activity_log.lock().expect("activity_log lock");
    log.push_back(entry);
    while log.len() > ACTIVITY_LOG_CAPACITY {
        log.pop_front();
    }
    save_activity_log(&log);
}

/// Same as `log_activity` but takes an `AppHandle` for use inside the
/// streamer task (which has no `tauri::State` handle).
fn log_activity_handle(
    app: &AppHandle,
    level: ActivityLevel,
    message: impl Into<String>,
    detail: Option<String>,
) {
    let now = Utc::now();
    let message = message.into();
    let state = app.state::<AppState>();
    {
        let log = state.activity_log.lock().expect("activity_log lock");
        if would_dedupe(&log, now, &message) {
            return;
        }
    }
    let entry = ActivityEntry {
        timestamp: now,
        level,
        message,
        detail,
    };
    match level {
        ActivityLevel::Info => {
            tracing::info!(message = %entry.message, detail = ?entry.detail, "activity")
        }
        ActivityLevel::Warn => {
            tracing::warn!(message = %entry.message, detail = ?entry.detail, "activity")
        }
        ActivityLevel::Error => {
            tracing::error!(message = %entry.message, detail = ?entry.detail, "activity")
        }
    }
    let mut log = state.activity_log.lock().expect("activity_log lock");
    log.push_back(entry);
    while log.len() > ACTIVITY_LOG_CAPACITY {
        log.pop_front();
    }
    save_activity_log(&log);
}

/// `GET` the entire activity log. Frontend polls this every couple of
/// seconds; `ACTIVITY_LOG_CAPACITY` keeps the payload bounded.
#[tauri::command]
fn activity_log_get(state: tauri::State<'_, AppState>) -> Vec<ActivityEntry> {
    let log = state.activity_log.lock().expect("activity_log lock");
    log.iter().cloned().collect()
}

/// Wipe the activity log. Useful when the pilot starts a fresh session
/// and doesn't want the previous flight's chatter cluttering the panel.
#[tauri::command]
fn activity_log_clear(state: tauri::State<'_, AppState>) {
    let mut log = state.activity_log.lock().expect("activity_log lock");
    log.clear();
    // Also persist the cleared state — otherwise a restart would
    // restore the pre-clear contents from disk.
    save_activity_log(&log);
}

// ---- Landing-history commands (Landing tab) ----

/// List every persisted landing record, newest first. Used by the
/// Landing tab's list view.
#[tauri::command]
fn landing_list(app: AppHandle) -> Vec<LandingRecord> {
    open_landing_store(&app)
        .and_then(|s| s.list().ok())
        .unwrap_or_default()
}

/// Fetch a single landing record by PIREP id.
#[tauri::command]
fn landing_get(app: AppHandle, pirep_id: String) -> Option<LandingRecord> {
    open_landing_store(&app)
        .and_then(|s| s.get(&pirep_id).ok())
        .flatten()
}

/// Build a *preview* landing record from the currently-active flight.
/// Used by the Landing tab during the rollout / before file: shows the
/// pilot what their landing looks like *right now* without having to
/// wait for the PIREP to be filed. Returns None when there is no
/// active flight or when the touchdown hasn't happened yet.
#[tauri::command]
fn landing_get_current(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
) -> Option<LandingRecord> {
    let flight = {
        let guard = state.active_flight.lock().expect("active_flight lock");
        guard.as_ref().cloned()
    }?;
    let stats = flight.stats.lock().expect("flight stats");
    let snapshot = current_snapshot(&app);
    let sim_kind = read_sim_config(&app).kind;
    let sim_label = match sim_kind {
        SimKind::Off => None,
        SimKind::Msfs2020 | SimKind::Msfs2024 => Some("MSFS"),
        SimKind::XPlane11 | SimKind::XPlane12 => Some("X-PLANE"),
    };
    let aircraft_icao = snapshot.as_ref().and_then(|s| s.aircraft_icao.as_deref());
    let aircraft_title = snapshot.as_ref().and_then(|s| s.aircraft_title.as_deref());
    build_landing_record(&flight, &stats, sim_label, aircraft_icao, aircraft_title)
}

/// Delete a landing record. Lets the user clean up bad/test entries
/// from the Landing tab. Best-effort — returns Ok(()) even if the
/// record didn't exist.
#[tauri::command]
fn landing_delete(app: AppHandle, pirep_id: String) -> Result<(), UiError> {
    let Some(store) = open_landing_store(&app) else {
        return Err(UiError::new("landing_store", "could not open landing store"));
    };
    let mut all = store
        .list()
        .map_err(|e| UiError::new("landing_read", format!("{e}")))?;
    let before = all.len();
    all.retain(|r| r.pirep_id != pirep_id);
    if all.len() == before {
        return Ok(());
    }
    // upsert one-by-one isn't quite right (it appends); easier: rewrite
    // the file via a private helper. We don't have one — fall back to
    // upserting each remaining row in order. Since LandingStore::upsert
    // dedupes by pirep_id, an upsert chain produces the same set.
    // BUT it doesn't drop rows. Hack: write empty first, then re-add.
    // The store doesn't expose a clear/replace yet — we add one.
    if let Err(e) = store.replace_all(&all) {
        return Err(UiError::new("landing_write", format!("{e}")));
    }
    Ok(())
}

// ---- Site config persistence ----

fn site_config_path(app: &AppHandle) -> Result<PathBuf, UiError> {
    app.path()
        .app_config_dir()
        .map(|dir| dir.join(SITE_CONFIG_FILE))
        .map_err(|e| UiError::new("config_path", e.to_string()))
}

fn read_site_config(app: &AppHandle) -> Result<Option<SiteConfig>, UiError> {
    let path = site_config_path(app)?;
    if !path.exists() {
        return Ok(None);
    }
    let bytes =
        std::fs::read(&path).map_err(|e| UiError::new("config_read", e.to_string()))?;
    let cfg: SiteConfig = serde_json::from_slice(&bytes)
        .map_err(|e| UiError::new("config_parse", e.to_string()))?;
    Ok(Some(cfg))
}

fn write_site_config(app: &AppHandle, cfg: &SiteConfig) -> Result<(), UiError> {
    let path = site_config_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| UiError::new("config_write", e.to_string()))?;
    }
    let json = serde_json::to_vec_pretty(cfg)
        .map_err(|e| UiError::new("config_serialize", e.to_string()))?;
    std::fs::write(&path, json).map_err(|e| UiError::new("config_write", e.to_string()))
}

fn clear_site_config(app: &AppHandle) -> Result<(), UiError> {
    let path = site_config_path(app)?;
    if path.exists() {
        std::fs::remove_file(&path)
            .map_err(|e| UiError::new("config_remove", e.to_string()))?;
    }
    Ok(())
}

// ---- Tauri commands ----

#[derive(Serialize)]
pub struct AppInfo {
    pub name: &'static str,
    pub version: &'static str,
    pub commit: Option<&'static str>,
    /// Small credit line for the About / Settings footer.
    pub credit: &'static str,
}

#[tauri::command]
fn app_info() -> AppInfo {
    AppInfo {
        name: "AeroACARS",
        version: env!("CARGO_PKG_VERSION"),
        commit: option_env!("AEROACARS_GIT_SHA"),
        credit: "Made with ❤️ in Gifhorn — by Thomas Kant",
    }
}

/// Hardcoded phpVMS host this build is locked to. Pre-1.0 we ship
/// AeroACARS as a German Sky Group internal beta; opening the
/// app to other VAs (config-driven URL) is a Phase-3 task once
/// the core is hardened. The login UI ignores whatever URL the
/// user types and always uses this — the input field stays for
/// continuity but the value is overwritten before validation.
const ALLOWED_PHPVMS_HOST: &str = "german-sky-group.eu";

/// Authenticate against a phpVMS site. On success: stores key in OS keyring,
/// writes URL to site config, and caches the live `Client` in `AppState`.
///
/// Domain is locked to `ALLOWED_PHPVMS_HOST` for this build. The
/// `url` argument is rewritten to `https://{ALLOWED_PHPVMS_HOST}`
/// regardless of what the UI sent. Pre-1.0 we don't want pilots
/// pointing AeroACARS at random phpVMS instances and reporting
/// bugs that aren't ours.
#[tauri::command]
async fn phpvms_login(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    url: String,
    api_key: String,
) -> Result<LoginResult, UiError> {
    // Hard-lock the host. We allow http/https variants and any path
    // suffix from the input but always force the canonical hostname.
    let _ = url; // input field is decorative for now
    let locked_url = format!("https://{ALLOWED_PHPVMS_HOST}");
    let conn = Connection::new(&locked_url, api_key.trim())?;
    let client = Client::new(conn)?;
    let profile = client.get_profile().await?;

    secrets::store_api_key(KEYRING_ACCOUNT, api_key.trim())
        .map_err(|e| UiError::new("keyring", e.to_string()))?;
    write_site_config(&app, &SiteConfig { url: locked_url.clone() })?;

    // v0.5.11: kick off live-tracking provisioning in the background.
    // Non-blocking — login completes regardless of whether the
    // provision call succeeds. Pure background feature, no UI.
    {
        let app_for_mqtt = app.clone();
        tauri::async_runtime::spawn(async move {
            init_mqtt_publisher_via_provisioning(app_for_mqtt).await;
        });
    }

    let base_url = client.connection().base_url().to_string();
    *state.client.lock().expect("client mutex") = Some(client.clone());
    cache_pilot(&state, &profile);

    // Auto-start the simulator adapter using the persisted selection.
    let saved_kind = read_sim_config(&app).kind;
    apply_sim_kind(&state, saved_kind);

    // Try to resume an in-progress flight (e.g. after a client crash).
    try_resume_flight(&app, &state, &client).await;

    log_activity(
        &state,
        ActivityLevel::Info,
        format!("Logged in as {}", profile.name),
        Some(format!("Sim: {:?}", saved_kind)),
    );
    Ok(LoginResult { profile, base_url })
}

/// Cache pilot ident + name on AppState (für Discord-Webhook-Posts).
/// Wird nach jedem get_profile()-Call aufgerufen — die phpVMS-API
/// liefert beides als Teil von /api/user.
fn cache_pilot(state: &tauri::State<'_, AppState>, profile: &api_client::Profile) {
    let ident = profile.ident.clone().unwrap_or_default();
    if !ident.is_empty() || !profile.name.is_empty() {
        *state.cached_pilot.lock().expect("cached_pilot lock") =
            Some((ident, profile.name.clone()));
    }
}

// ---- v0.5.11: MQTT live-tracking auto-provisioning --------------------
//
// Pure background feature — pilot-invisible, no UI. Provisioning is
// optional: failure is non-fatal, AeroACARS continues to function as
// before. Spec: aeroacars-live/docs/aeroacars-integration-spec-v2.md.

const MQTT_KEYRING_USERNAME: &str = "mqtt-username";
const MQTT_KEYRING_PASSWORD: &str = "mqtt-password";
const MQTT_KEYRING_VA: &str = "mqtt-va";
const MQTT_KEYRING_PILOT_ID: &str = "mqtt-pilot-id";
const MQTT_KEYRING_BROKER: &str = "mqtt-broker-url";

/// Try to start the MQTT live-tracking publisher.
///
/// Two-stage flow:
///   1. If MQTT credentials are already cached in the keyring (from
///      a previous successful provision), use them directly.
///   2. Otherwise, read the phpVMS API key from the keyring (must be
///      present — the user has logged in at least once) and call
///      live.kant.ovh's `/api/provision` endpoint with it. The
///      server returns MQTT credentials which we cache and use.
///
/// All failures are logged via `tracing::warn!` and ignored — this
/// is a non-fatal background feature. If the user's never logged in,
/// or the VPS is down, or the API key is invalid, the function just
/// returns without starting MQTT.
async fn init_mqtt_publisher_via_provisioning(app: AppHandle) {
    use aeroacars_mqtt::{provision::provision, start, MqttConfig};

    let state = app.state::<AppState>();

    // v0.5.14: Idempotency-Guard. Diese Funktion wird aus drei Stellen
    // aufgerufen — Setup-Hook, cmd_login, phpvms_load_session. Beim
    // App-Start mit gespeicherter Session feuern Setup + load_session
    // praktisch zeitgleich → ohne Guard zwei Clients mit gleichem
    // (alten v0.5.13) bzw. fast-gleichem (v0.5.14, ms-Timestamp)
    // client_id, die sich gegenseitig vom Broker kicken. Ergebnis war
    // Reconnect-Cycle alle 1-10 Sekunden.
    //
    // Mit Guard: nur der erste Call macht `start()`, alle weiteren
    // Aufrufe sind no-op. `phpvms_logout` setzt `state.mqtt = None`
    // sauber zurück, sodass der nächste Login wieder startet.
    {
        let guard = state.mqtt.lock().await;
        if guard.is_some() {
            tracing::debug!(
                "live-tracking: publisher already running, skipping re-init"
            );
            return;
        }
    }

    // Check cache first.
    let cached = (|| -> Option<MqttConfig> {
        let user = secrets::load_api_key(MQTT_KEYRING_USERNAME).ok().flatten()?;
        let pw = secrets::load_api_key(MQTT_KEYRING_PASSWORD).ok().flatten()?;
        let va = secrets::load_api_key(MQTT_KEYRING_VA).ok().flatten()?;
        let pilot_id = secrets::load_api_key(MQTT_KEYRING_PILOT_ID).ok().flatten()?;
        let broker = secrets::load_api_key(MQTT_KEYRING_BROKER).ok().flatten()?;
        Some(MqttConfig {
            broker_url: broker,
            username: user,
            password: pw,
            va_prefix: va,
            pilot_id,
        })
    })();

    let cfg = if let Some(c) = cached {
        tracing::info!("live-tracking: using cached MQTT credentials");
        c
    } else {
        // No cache — provision from server.
        let api_key = match secrets::load_api_key(KEYRING_ACCOUNT) {
            Ok(Some(k)) => k,
            Ok(None) => {
                tracing::debug!(
                    "live-tracking: no phpVMS API key in keyring yet — \
                     will retry after login"
                );
                return;
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "live-tracking: keyring read failed — skipping"
                );
                return;
            }
        };

        let resp = match provision(&api_key, None).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "live-tracking: provision call failed (non-fatal)"
                );
                return;
            }
        };

        // Cache for next launch. Failures here are non-fatal —
        // we'll just provision again next time.
        let _ = secrets::store_api_key(MQTT_KEYRING_USERNAME, &resp.username);
        let _ = secrets::store_api_key(MQTT_KEYRING_PASSWORD, &resp.password);
        let _ = secrets::store_api_key(MQTT_KEYRING_VA, &resp.va_prefix);
        let _ = secrets::store_api_key(MQTT_KEYRING_PILOT_ID, &resp.pilot_id);
        let _ = secrets::store_api_key(MQTT_KEYRING_BROKER, &resp.broker_url);
        tracing::info!(
            pilot_id = %resp.pilot_id,
            va = %resp.va_prefix,
            newly_created = resp.newly_created,
            "live-tracking provisioned"
        );
        resp.into()
    };

    let handle = match start(cfg) {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!(error = %e, "live-tracking: MQTT start failed");
            return;
        }
    };

    *state.mqtt.lock().await = Some(handle);
    tracing::info!("live-tracking publisher running");
}

/// Forget cached MQTT credentials. Called from logout — next session
/// re-provisions cleanly. The phpVMS API key in `KEYRING_ACCOUNT`
/// already gets cleared by the existing logout flow.
fn clear_mqtt_credentials_cache() {
    let _ = secrets::delete_api_key(MQTT_KEYRING_USERNAME);
    let _ = secrets::delete_api_key(MQTT_KEYRING_PASSWORD);
    let _ = secrets::delete_api_key(MQTT_KEYRING_VA);
    let _ = secrets::delete_api_key(MQTT_KEYRING_PILOT_ID);
    let _ = secrets::delete_api_key(MQTT_KEYRING_BROKER);
}

/// Forget the current session. Removes the keyring entry and site config,
/// clears the in-memory client.
#[tauri::command]
async fn phpvms_logout(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), UiError> {
    *state.client.lock().expect("client mutex") = None;
    secrets::delete_api_key(KEYRING_ACCOUNT)
        .map_err(|e| UiError::new("keyring", e.to_string()))?;
    // v0.5.11: stop the MQTT publisher and forget cached credentials
    // so the next login provisions fresh (handles the case where
    // a different pilot logs in on the same machine).
    if let Some(handle) = state.mqtt.lock().await.take() {
        handle.shutdown();
    }
    clear_mqtt_credentials_cache();
    clear_site_config(&app)?;
    tracing::info!("logged out");
    Ok(())
}

/// On app launch: try to restore the previous session from disk + keyring.
/// Returns `None` if no session is stored or stored key is now invalid.
#[tauri::command]
async fn phpvms_load_session(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<Option<LoginResult>, UiError> {
    let Some(_cfg) = read_site_config(&app)? else {
        return Ok(None);
    };
    let Some(api_key) = secrets::load_api_key(KEYRING_ACCOUNT)
        .map_err(|e| UiError::new("keyring", e.to_string()))?
    else {
        return Ok(None);
    };

    // Force the locked host even if the persisted config has an old
    // URL from a development build that allowed arbitrary phpVMS
    // instances. See `ALLOWED_PHPVMS_HOST` for the rationale.
    let locked_url = format!("https://{ALLOWED_PHPVMS_HOST}");
    let conn = Connection::new(&locked_url, &api_key)?;
    let client = Client::new(conn)?;
    match client.get_profile().await {
        Ok(profile) => {
            let base_url = client.connection().base_url().to_string();
            *state.client.lock().expect("client mutex") = Some(client.clone());
            cache_pilot(&state, &profile);
            // Auto-start the simulator adapter when we restore an existing session.
            let saved_kind = read_sim_config(&app).kind;
            apply_sim_kind(&state, saved_kind);
            try_resume_flight(&app, &state, &client).await;
            // Throttle "Session restored" entries: at most once per
            // 60 s. A session-restore is benign noise on rapid Tauri
            // restarts (debug cycles, dev HMR rebuilds) and the
            // activity log was filling up with 4-6 of these in a
            // row from yesterday's testing. Real users restart the
            // app maybe twice a day, so 60 s is plenty.
            use std::sync::atomic::{AtomicI64, Ordering};
            static LAST_SESSION_LOG_S: AtomicI64 = AtomicI64::new(0);
            let now_s = Utc::now().timestamp();
            let prev_s = LAST_SESSION_LOG_S.load(Ordering::Relaxed);
            if now_s - prev_s >= 60 {
                LAST_SESSION_LOG_S.store(now_s, Ordering::Relaxed);
                log_activity(
                    &state,
                    ActivityLevel::Info,
                    format!("Session restored — {}", profile.name),
                    Some(format!("Sim: {:?}", saved_kind)),
                );
            }
            // v0.5.12: belt-and-suspenders — kick off MQTT live-tracking
            // provisioning here too, in case the setup-hook race lost
            // out to a not-yet-ready keyring on this platform. The
            // function is idempotent: if MQTT is already running this
            // is a no-op (well, almost — it may try to overwrite the
            // handle, but the existing one keeps working). If keyring
            // wasn't ready at setup time but is now, this is the catch.
            {
                let app_for_mqtt = app.clone();
                tauri::async_runtime::spawn(async move {
                    init_mqtt_publisher_via_provisioning(app_for_mqtt).await;
                });
            }
            Ok(Some(LoginResult { profile, base_url }))
        }
        // Stored key was rejected — drop it so the next login goes via the form.
        Err(ApiError::Unauthenticated) => {
            let _ = secrets::delete_api_key(KEYRING_ACCOUNT);
            let _ = clear_site_config(&app);
            Ok(None)
        }
        Err(other) => Err(other.into()),
    }
}

/// Pull the active client out of state, or fail with `not_logged_in`.
fn current_client(state: &tauri::State<'_, AppState>) -> Result<Client, UiError> {
    let guard = state.client.lock().expect("client mutex");
    guard
        .as_ref()
        .cloned()
        .ok_or_else(|| UiError::new("not_logged_in", "no active session"))
}

/// Read the current minimize-to-tray setting from backend state.
/// Used by the React side after mount to surface the toggle's
/// initial value when localStorage was wiped.
#[tauri::command]
fn get_minimize_to_tray(state: tauri::State<'_, AppState>) -> bool {
    state
        .minimize_to_tray_enabled
        .load(std::sync::atomic::Ordering::Relaxed)
}

/// Flip the minimize-to-tray flag. The React Settings panel calls
/// this whenever the user toggles the checkbox AND on every mount
/// (so the persisted localStorage value flows back into the backend
/// after a restart). Cheap atomic flip — the actual close-handler
/// just reads the flag at the moment a CloseRequested fires.
#[tauri::command]
fn set_minimize_to_tray(
    state: tauri::State<'_, AppState>,
    enabled: bool,
) -> Result<(), UiError> {
    let was = state
        .minimize_to_tray_enabled
        .swap(enabled, std::sync::atomic::Ordering::Relaxed);
    if was != enabled {
        tracing::info!(enabled, "minimize_to_tray toggled");
    }
    Ok(())
}

/// Find the N nearest airports to the active flight's CURRENT
/// position. Powers the manual-divert modal in the cockpit so the
/// pilot can pick from a list of nearby fields (or type a custom
/// ICAO if their actual landing strip isn't in our local DB).
///
/// Returns an empty vec when no flight is active or no airport is
/// within `DIVERT_NEAREST_SEARCH_RADIUS_NM` of the current position.
#[tauri::command]
fn divert_nearest_airports(
    state: tauri::State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<runway::NearestAirport>, UiError> {
    let limit = limit.unwrap_or(5).clamp(1, 20);
    let (lat, lon) = {
        let guard = state.active_flight.lock().expect("active_flight lock");
        let Some(flight) = guard.as_ref() else {
            return Ok(Vec::new());
        };
        let stats = flight.stats.lock().expect("flight stats");
        match (stats.last_lat, stats.last_lon) {
            (Some(la), Some(lo)) => (la, lo),
            _ => return Ok(Vec::new()),
        }
    };
    Ok(runway::find_nearest_airports(
        lat,
        lon,
        DIVERT_NEAREST_SEARCH_RADIUS_NM * 1852.0,
        limit,
    ))
}

/// Re-fetch the pilot profile from phpVMS. Returns the fresh profile
/// (or None if not logged in / fetch failed). Mostly used by the UI to
/// pick up `curr_airport` after a PIREP files — phpVMS updates the
/// pilot's current airport server-side once a flight is accepted, but
/// our cached LoginResult never sees it without an explicit re-fetch.
#[tauri::command]
async fn phpvms_refresh_profile(
    state: tauri::State<'_, AppState>,
) -> Result<Option<Profile>, UiError> {
    let client = match current_client(&state) {
        Ok(c) => c,
        Err(_) => return Ok(None),
    };
    match client.get_profile().await {
        Ok(p) => {
            cache_pilot(&state, &p);
            Ok(Some(p))
        }
        Err(e) => {
            tracing::warn!(error = %e, "profile refresh failed");
            Ok(None)
        }
    }
}

/// Single GitHub release record — the subset we render in the
/// in-app "What's new" modal. Source: anonymous GET against
/// <https://api.github.com/repos/MANFahrer-GF/AeroACARS/releases/tags/v{version}>
/// (GitHub allows 60 anonymous req/h per IP, way more than we'd ever
/// hit since we cache the result for the modal's lifetime).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseNotes {
    /// Release name (typically "AeroACARS v0.1.23"). Used as the
    /// modal header.
    pub name: String,
    /// Tag name ("v0.1.23"). Used as a stable identifier so the
    /// frontend can store "last-seen" in localStorage.
    pub tag_name: String,
    /// Markdown body — bilingual format with `## 🇩🇪 Deutsch` and
    /// `## 🇬🇧 English` section markers. Frontend splits and renders
    /// just the matching language.
    pub body: String,
    /// ISO-8601 publish timestamp.
    pub published_at: String,
    /// Direct link to the GitHub release page — used as a fallback
    /// "View on GitHub" button in the modal.
    pub html_url: String,
}

/// Fetch the GitHub release notes for a specific version tag. Used by
/// the in-app "What's new" modal that fires once per version after
/// the auto-updater swaps the binary in.
///
/// Returns `Err(...)` for network failures or non-200 responses;
/// frontend falls back to "open on GitHub" in those cases. We
/// deliberately don't authenticate the request — at one fetch per
/// app-start the 60-req/h anonymous limit is irrelevant, and pulling
/// secrets into a release-notes feature would be silly.
#[tauri::command]
async fn fetch_release_notes(version: String) -> Result<ReleaseNotes, UiError> {
    let tag = if version.starts_with('v') {
        version.clone()
    } else {
        format!("v{version}")
    };
    let url = format!(
        "https://api.github.com/repos/MANFahrer-GF/AeroACARS/releases/tags/{tag}"
    );
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .user_agent(concat!("AeroACARS/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| UiError::new("network", e.to_string()))?;
    let response = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| UiError::new("network", e.to_string()))?;
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(UiError::new(
            "not_found",
            format!("no release with tag {tag}"),
        ));
    }
    if !status.is_success() {
        return Err(UiError::new(
            "github_error",
            format!("GitHub returned HTTP {}", status.as_u16()),
        ));
    }
    response
        .json::<ReleaseNotes>()
        .await
        .map_err(|e| UiError::new("parse", e.to_string()))
}

/// `GET /api/user/bids` — the pilot's open bids.
#[tauri::command]
async fn phpvms_get_bids(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Bid>, UiError> {
    let client = current_client(&state)?;
    match client.get_bids().await {
        Ok(bids) => Ok(bids),
        Err(e) => {
            // Log to the activity feed so the pilot can paste the
            // technical detail without having to dig through console
            // logs. Particularly useful for the BadResponse case where
            // a single malformed bid breaks the whole list — we show
            // exactly which field/path tripped the decoder.
            log_activity(
                &state,
                ActivityLevel::Error,
                "Bids konnten nicht geladen werden",
                Some(format!("{e}")),
            );
            Err(e.into())
        }
    }
}

/// SimBrief OFP-Preview (v0.3.0) — fetcht das letzte SimBrief XML
/// für eine OFP-ID und liefert die wesentlichen Plan-Werte. Wird vom
/// v0.3.2: Refresh den SimBrief-OFP für den GERADE LAUFENDEN Flug.
/// Real-Pilot-Workflow: Pilot regeneriert auf simbrief.com einen neuen
/// OFP nachdem AeroACARS schon den alten beim Flight-Start gefangen
/// hat — z.B. weil sich Pax-Bestand, Cargo, oder Reserve-Strategie
/// geändert haben. Vorher musste der Pilot „Discard flight" und neu
/// starten; jetzt zieht der Cockpit-Refresh-Button den frischen OFP
/// und überschreibt die `planned_*`-Felder im aktiven Flug.
///
/// Wir hängen am Bid (statt nur am gespeicherten ofp_id), weil der
/// SimBrief-Sync auf phpVMS-Seite die Relation aktualisiert wenn
/// der Pilot regeneriert — der Bid trägt dann die neue OFP-ID.
/// Fallback: wenn der Bid keinen SimBrief-Link mehr hat, error.
#[tauri::command]
async fn flight_refresh_simbrief(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<SimBriefOfpDto, UiError> {
    // Snapshot bid_id under the lock; release it before any await.
    let bid_id = {
        let guard = state.active_flight.lock().expect("active_flight lock");
        guard
            .as_ref()
            .ok_or_else(|| UiError::new("no_active_flight", "no flight is active"))?
            .bid_id
    };
    let client = current_client(&state)?;
    // Pull the up-to-date bid list — the pilot's OFP regeneration would
    // have updated the bid->simbrief relation server-side. We don't have
    // a "GET single bid by id" endpoint, so we list all and filter.
    let bids = client.get_bids().await.map_err(|e| {
        UiError::new(
            "bids_fetch_failed",
            format!("could not refresh bid list: {e}"),
        )
    })?;
    let bid = bids.into_iter().find(|b| b.id == bid_id).ok_or_else(|| {
        UiError::new(
            "bid_not_found",
            "current bid is no longer in your bid list — cannot refresh OFP",
        )
    })?;
    let sb_id = bid.flight.simbrief.as_ref().map(|s| s.id.clone()).ok_or_else(|| {
        UiError::new(
            "no_simbrief_link",
            "bid has no SimBrief OFP linked — generate one on simbrief.com first",
        )
    })?;
    let ofp = client.fetch_simbrief_ofp(&sb_id).await.map_err(|e| {
        UiError::new("ofp_fetch_failed", format!("SimBrief OFP fetch failed: {e}"))
    })?;
    let ofp = ofp.ok_or_else(|| {
        UiError::new(
            "ofp_unusable",
            "SimBrief returned no usable OFP — check your simbrief.com OFP",
        )
    })?;
    // Mutate the active flight's planned_* fields. We re-acquire the
    // lock here (post-await) and verify the flight is still the same
    // bid — protects against the user discarding mid-fetch.
    {
        let guard = state.active_flight.lock().expect("active_flight lock");
        let flight = guard.as_ref().ok_or_else(|| {
            UiError::new(
                "no_active_flight",
                "flight was discarded during OFP refresh",
            )
        })?;
        if flight.bid_id != bid_id {
            return Err(UiError::new(
                "flight_changed",
                "active flight changed during OFP refresh — try again",
            ));
        }
        let mut stats = flight.stats.lock().expect("flight stats lock");
        stats.planned_block_fuel_kg = Some(ofp.planned_block_fuel_kg).filter(|&v| v > 0.0);
        stats.planned_burn_kg = Some(ofp.planned_burn_kg).filter(|&v| v > 0.0);
        stats.planned_reserve_kg = Some(ofp.planned_reserve_kg).filter(|&v| v > 0.0);
        stats.planned_zfw_kg = Some(ofp.planned_zfw_kg).filter(|&v| v > 0.0);
        stats.planned_tow_kg = Some(ofp.planned_tow_kg).filter(|&v| v > 0.0);
        stats.planned_ldw_kg = Some(ofp.planned_ldw_kg).filter(|&v| v > 0.0);
        stats.planned_route = ofp.route.clone();
        stats.planned_alternate = ofp.alternate.clone();
        stats.planned_max_zfw_kg = Some(ofp.max_zfw_kg).filter(|&v| v > 0.0);
        stats.planned_max_tow_kg = Some(ofp.max_tow_kg).filter(|&v| v > 0.0);
        stats.planned_max_ldw_kg = Some(ofp.max_ldw_kg).filter(|&v| v > 0.0);
        drop(stats);
        save_active_flight(&app, flight);
    }
    log_activity(
        &state,
        ActivityLevel::Info,
        "OFP refreshed".to_string(),
        Some(format!(
            "Plan-Werte aktualisiert aus SimBrief — Block {:.0} kg, TOW {:.0} kg, LDW {:.0} kg",
            ofp.planned_block_fuel_kg, ofp.planned_tow_kg, ofp.planned_ldw_kg
        )),
    );
    Ok(SimBriefOfpDto {
        planned_block_fuel_kg: ofp.planned_block_fuel_kg,
        planned_burn_kg: ofp.planned_burn_kg,
        planned_reserve_kg: ofp.planned_reserve_kg,
        planned_zfw_kg: ofp.planned_zfw_kg,
        planned_tow_kg: ofp.planned_tow_kg,
        planned_ldw_kg: ofp.planned_ldw_kg,
        route: ofp.route,
        alternate: ofp.alternate,
        ofp_flight_number: ofp.ofp_flight_number,
        ofp_origin_icao: ofp.ofp_origin_icao,
        ofp_destination_icao: ofp.ofp_destination_icao,
        ofp_generated_at: ofp.ofp_generated_at,
    })
}

/// Frontend in der ausgeklappten Bid-Card aufgerufen, damit der Pilot
/// die Plan-Werte (Block-Fuel, Trip-Burn, TOW, LDW etc.) **vor**
/// dem Flight-Start sieht.
///
/// Im Gegensatz zum Auto-Fetch beim flight_start() (der die Werte
/// in die FlightStats schreibt), liefert dieser Command sie nur
/// transient an die UI zurück. Kein State-Mutation.
#[tauri::command]
async fn fetch_simbrief_preview(
    state: tauri::State<'_, AppState>,
    ofp_id: String,
) -> Result<Option<SimBriefOfpDto>, UiError> {
    let client = current_client(&state)?;
    match client.fetch_simbrief_ofp(&ofp_id).await {
        Ok(Some(ofp)) => Ok(Some(SimBriefOfpDto {
            planned_block_fuel_kg: ofp.planned_block_fuel_kg,
            planned_burn_kg: ofp.planned_burn_kg,
            planned_reserve_kg: ofp.planned_reserve_kg,
            planned_zfw_kg: ofp.planned_zfw_kg,
            planned_tow_kg: ofp.planned_tow_kg,
            planned_ldw_kg: ofp.planned_ldw_kg,
            route: ofp.route,
            alternate: ofp.alternate,
            ofp_flight_number: ofp.ofp_flight_number,
            ofp_origin_icao: ofp.ofp_origin_icao,
            ofp_destination_icao: ofp.ofp_destination_icao,
            ofp_generated_at: ofp.ofp_generated_at,
        })),
        Ok(None) => Ok(None), // OFP nicht abrufbar (Netz-Fehler / 404 / ...)
        Err(e) => Err(e.into()),
    }
}

/// DTO für den SimBrief-Preview-Tauri-Command. Bewusst flach gehalten,
/// die `waypoints` aus `SimBriefOfp` ignoriert die UI hier (zu sperrig
/// für die kompakte Bid-Card-Vorschau).
#[derive(Debug, Clone, Serialize)]
struct SimBriefOfpDto {
    planned_block_fuel_kg: f32,
    planned_burn_kg: f32,
    planned_reserve_kg: f32,
    planned_zfw_kg: f32,
    planned_tow_kg: f32,
    planned_ldw_kg: f32,
    route: Option<String>,
    alternate: Option<String>,
    // v0.3.0: OFP-Identität für Mismatch-Detection im Frontend.
    ofp_flight_number: String,
    ofp_origin_icao: String,
    ofp_destination_icao: String,
    ofp_generated_at: String,
}

/// Aircraft-Details für die Bid-Card (v0.3.0). Liefert die Registrierung
/// + ICAO + Name eines Aircraft, sodass die Bid-Card "RYR-B738-WL · EI-ENI"
/// anzeigen kann statt nur den Subfleet-Namen.
#[tauri::command]
async fn phpvms_get_aircraft(
    state: tauri::State<'_, AppState>,
    aircraft_id: i64,
) -> Result<AircraftInfoDto, UiError> {
    let client = current_client(&state)?;
    let details = client.get_aircraft(aircraft_id).await?;
    Ok(AircraftInfoDto {
        id: details.id,
        registration: details.registration,
        icao: details.icao,
        name: details.name,
    })
}

/// Schmale Aircraft-Info für die Frontend-Anzeige. Nur die Felder die
/// wir wirklich rendern — Status/Airport-ID lassen wir weg.
#[derive(Debug, Clone, Serialize)]
struct AircraftInfoDto {
    id: i64,
    registration: Option<String>,
    icao: Option<String>,
    name: Option<String>,
}

// ---- Active-flight persistence (for resume after crash/restart) ----

fn active_flight_path(app: &AppHandle) -> Result<PathBuf, UiError> {
    app.path()
        .app_config_dir()
        .map(|dir| dir.join(ACTIVE_FLIGHT_FILE))
        .map_err(|e| UiError::new("config_path", e.to_string()))
}

fn write_persisted_flight(app: &AppHandle, flight: &PersistedFlight) -> Result<(), UiError> {
    let path = active_flight_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| UiError::new("config_write", e.to_string()))?;
    }
    let bytes = serde_json::to_vec_pretty(flight)
        .map_err(|e| UiError::new("config_serialize", e.to_string()))?;
    std::fs::write(&path, bytes).map_err(|e| UiError::new("config_write", e.to_string()))
}

fn read_persisted_flight(app: &AppHandle) -> Option<PersistedFlight> {
    let path = active_flight_path(app).ok()?;
    if !path.exists() {
        return None;
    }
    let bytes = std::fs::read(&path).ok()?;
    serde_json::from_slice::<PersistedFlight>(&bytes).ok()
}

// ---- Activity-log persistence ----
//
// Dump the in-memory activity log to disk after every mutation so
// an app restart mid-flight doesn't lose the running event history.
// Read once at boot to pre-populate AppState. Cheap: 1000-entry log
// is roughly 100–200 KB, well within "instant" sync write cost.
//
// The path is resolved once on the first call (with the AppHandle
// available there) and cached in a OnceLock so the lock-free
// log_activity helper that doesn't have an AppHandle can still
// persist without re-resolving on every call.

use std::sync::OnceLock;
static ACTIVITY_LOG_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Resolve and cache the activity-log path on first use. Called once
/// from the Tauri setup hook with the AppHandle, then read from the
/// OnceLock everywhere afterwards.
fn init_activity_log_path(app: &AppHandle) {
    let path = app
        .path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join(ACTIVITY_LOG_FILE));
    let _ = ACTIVITY_LOG_PATH.set(path);
}

/// Reset the activity log for a fresh recording. Called from
/// `flight_start` (NOT `flight_adopt`) so each new flight starts
/// with a clean feed in the UI and on disk. Per-PIREP JSONL flight-
/// event logs are unaffected — they remain on disk forever for
/// review / debugging.
fn clear_activity_log_for_new_flight(app: &AppHandle) {
    let state = app.state::<AppState>();
    let mut log = state.activity_log.lock().expect("activity_log lock");
    log.clear();
    save_activity_log(&log);
    tracing::info!("activity log cleared for new flight");
}

/// Best-effort persist of the entire activity-log VecDeque to disk.
/// Failures are logged at warn level but never propagated — the
/// activity log is informational, not safety-critical, and we'd
/// rather drop a write than crash the streamer.
fn save_activity_log(log: &VecDeque<ActivityEntry>) {
    let Some(Some(path)) = ACTIVITY_LOG_PATH.get() else { return };
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            tracing::warn!(error = %e, "could not ensure activity-log parent dir");
            return;
        }
    }
    let entries: Vec<&ActivityEntry> = log.iter().collect();
    match serde_json::to_vec_pretty(&entries) {
        Ok(bytes) => {
            if let Err(e) = std::fs::write(path, bytes) {
                tracing::warn!(error = %e, "could not write activity log");
            }
        }
        Err(e) => tracing::warn!(error = %e, "could not serialize activity log"),
    }
}

/// Read the persisted activity log on first boot — used to seed the
/// AppState's VecDeque before Tauri's setup hook runs. We don't have
/// an AppHandle yet at this point, so we resolve the standard Tauri
/// config-dir layout manually via APPDATA. Stays best-effort: any
/// failure just yields an empty log.
fn load_activity_log_at_boot() -> VecDeque<ActivityEntry> {
    let appdata = match std::env::var_os("APPDATA") {
        Some(v) => v,
        None => return VecDeque::new(),
    };
    // Tauri's identifier-based default for app_config_dir on Windows.
    // Identifier is set in `tauri.conf.json` as `com.aeroacars.app`.
    let path = std::path::Path::new(&appdata)
        .join("com.aeroacars.app")
        .join(ACTIVITY_LOG_FILE);
    if !path.exists() {
        return VecDeque::new();
    }
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "could not read persisted activity log");
            return VecDeque::new();
        }
    };
    match serde_json::from_slice::<Vec<ActivityEntry>>(&bytes) {
        Ok(entries) => {
            tracing::info!(restored = entries.len(), "activity log restored from disk");
            entries.into_iter().collect()
        }
        Err(e) => {
            tracing::warn!(error = %e, "could not parse persisted activity log");
            VecDeque::new()
        }
    }
}

/// Drop any queued positions for the given PIREP. Called when the user
/// cancels or forgets a flight — replaying those rows would attach
/// them to a PIREP that's already gone or moved on, so we clear them
/// out cleanly. Other flights' queued rows stay put.
fn discard_queued_positions_for(app: &AppHandle, pirep_id: &str) {
    let Some(q) = open_position_queue(app) else { return; };
    let items = match q.read_all() {
        Ok(v) => v,
        Err(_) => return,
    };
    let kept: Vec<_> = items.into_iter().filter(|i| i.pirep_id != pirep_id).collect();
    let _ = q.replace(&kept);
}

fn clear_persisted_flight(app: &AppHandle) {
    let Ok(path) = active_flight_path(app) else { return };
    if path.exists() {
        let _ = std::fs::remove_file(&path);
    }
}

fn save_active_flight(app: &AppHandle, flight: &ActiveFlight) {
    // Snapshot stats inside a short-lived guard so we don't hold the
    // mutex while doing I/O.
    let stats_snapshot = {
        let guard = flight.stats.lock().expect("flight stats");
        PersistedFlightStats::snapshot_from(&guard)
    };
    let persisted = PersistedFlight {
        pirep_id: flight.pirep_id.clone(),
        bid_id: flight.bid_id,
        started_at: flight.started_at,
        airline_icao: flight.airline_icao.clone(),
        airline_logo_url: flight.airline_logo_url.clone(),
        planned_registration: flight.planned_registration.clone(),
        flight_number: flight.flight_number.clone(),
        dpt_airport: flight.dpt_airport.clone(),
        arr_airport: flight.arr_airport.clone(),
        fares: flight.fares.clone(),
        stats: stats_snapshot,
    };
    if let Err(e) = write_persisted_flight(app, &persisted) {
        tracing::warn!(error = ?e, "could not persist active flight");
    }
}

// ---- Airport cache ----

#[derive(Serialize)]
pub struct AirportInfo {
    icao: String,
    name: Option<String>,
    lat: Option<f64>,
    lon: Option<f64>,
}

/// Fetch an airport by ICAO, caching the result so we don't re-hit the network
/// on each sim snapshot.
#[tauri::command]
async fn airport_get(
    state: tauri::State<'_, AppState>,
    icao: String,
) -> Result<AirportInfo, UiError> {
    let key = icao.trim().to_uppercase();
    // Block-scope the lock so the MutexGuard is dropped before any `await`,
    // keeping the future `Send`.
    let cached: Option<Airport> = {
        let guard = state.airports.lock().expect("airports lock");
        guard.get(&key).cloned()
    };
    if let Some(c) = cached {
        return Ok(AirportInfo {
            icao: key,
            name: c.name,
            lat: c.lat,
            lon: c.lon,
        });
    }
    let client = current_client(&state)?;
    let airport = client.get_airport(&key).await?;
    let info = AirportInfo {
        icao: key.clone(),
        name: airport.name.clone(),
        lat: airport.lat,
        lon: airport.lon,
    };
    {
        let mut guard = state.airports.lock().expect("airports lock");
        guard.insert(key, airport);
    }
    Ok(info)
}

// ---- Flight workflow ----

fn flight_info(flight: &ActiveFlight) -> ActiveFlightInfo {
    let stats = flight.stats.lock().expect("flight stats");
    // Don't consume here — the flag stays true until the resume banner has
    // run its course (flight_resume_confirm or flight_cancel clears it).
    // This avoids a race with React StrictMode's double-mount where the
    // first poll consumes the flag before the UI can latch it.
    let was_just_resumed = flight.was_just_resumed.load(Ordering::Relaxed);
    ActiveFlightInfo {
        pirep_id: flight.pirep_id.clone(),
        bid_id: flight.bid_id,
        started_at: flight.started_at.to_rfc3339(),
        airline_icao: flight.airline_icao.clone(),
        planned_registration: flight.planned_registration.clone(),
        flight_number: flight.flight_number.clone(),
        dpt_airport: flight.dpt_airport.clone(),
        arr_airport: flight.arr_airport.clone(),
        distance_nm: stats.distance_nm,
        position_count: stats.position_count,
        phase: phase_to_snake(stats.phase).to_string(),
        block_off_at: stats.block_off_at.map(|t| t.to_rfc3339()),
        takeoff_at: stats.takeoff_at.map(|t| t.to_rfc3339()),
        landing_at: stats.landing_at.map(|t| t.to_rfc3339()),
        block_on_at: stats.block_on_at.map(|t| t.to_rfc3339()),
        landing_rate_fpm: stats.landing_rate_fpm,
        landing_g_force: stats.landing_g_force,
        was_just_resumed,
        dep_gate: stats.dep_gate.clone(),
        arr_gate: stats.arr_gate.clone(),
        approach_runway: stats.approach_runway.clone(),
        last_position_at: stats.last_position_at.map(|t| t.to_rfc3339()),
        last_heartbeat_at: stats.last_heartbeat_at.map(|t| t.to_rfc3339()),
        queued_position_count: stats.queued_position_count,
        paused_since: stats.paused_since.map(|t| t.to_rfc3339()),
        paused_last_known: stats.paused_last_known.clone(),
        divert_hint: stats.divert_hint.clone(),
        touch_and_go_count: stats
            .touchdown_events
            .iter()
            .filter(|e| matches!(e.kind, TouchdownKind::TouchAndGo))
            .count() as u32,
        go_around_count: stats.go_around_count,
        // ---- v0.3.0 — SimBrief Plan-Werte für Soll/Ist-Vergleich ----
        // Werden vom flight_start gefüllt (über fetch_simbrief_ofp).
        // None bleiben sie wenn der Pilot keine SimBrief-Verbindung
        // im phpVMS-Profil hat.
        planned_block_fuel_kg: stats.planned_block_fuel_kg,
        planned_burn_kg: stats.planned_burn_kg,
        planned_reserve_kg: stats.planned_reserve_kg,
        planned_max_zfw_kg: stats.planned_max_zfw_kg,
        planned_max_tow_kg: stats.planned_max_tow_kg,
        planned_max_ldw_kg: stats.planned_max_ldw_kg,
        // v0.3.0 — Live-Loadsheet-Werte für Boarding-Phase. Pulled
        // aus dem letzten beobachteten Sim-Snapshot — wird beim
        // Block-off "eingefroren" (last_block_fuel_kg etc.).
        sim_fuel_kg: stats.last_fuel_kg,
        sim_zfw_kg: stats.last_zfw_kg,
        sim_tow_kg: stats.last_total_weight_kg,
        planned_zfw_kg: stats.planned_zfw_kg,
        planned_tow_kg: stats.planned_tow_kg,
        planned_ldw_kg: stats.planned_ldw_kg,
        planned_route: stats.planned_route.clone(),
        planned_alternate: stats.planned_alternate.clone(),
    }
}

fn phase_to_snake(phase: FlightPhase) -> &'static str {
    match phase {
        FlightPhase::Preflight => "preflight",
        FlightPhase::Boarding => "boarding",
        FlightPhase::Pushback => "pushback",
        FlightPhase::TaxiOut => "taxi_out",
        FlightPhase::TakeoffRoll => "takeoff_roll",
        FlightPhase::Takeoff => "takeoff",
        FlightPhase::Climb => "climb",
        FlightPhase::Cruise => "cruise",
        FlightPhase::Holding => "holding",
        FlightPhase::Descent => "descent",
        FlightPhase::Approach => "approach",
        FlightPhase::Final => "final",
        FlightPhase::Landing => "landing",
        FlightPhase::TaxiIn => "taxi_in",
        FlightPhase::BlocksOn => "blocks_on",
        FlightPhase::Arrived => "arrived",
        FlightPhase::PirepSubmitted => "pirep_submitted",
    }
}

#[tauri::command]
fn flight_status(state: tauri::State<'_, AppState>) -> Option<ActiveFlightInfo> {
    let guard = state.active_flight.lock().expect("active_flight lock");
    guard.as_ref().map(|f| flight_info(f.as_ref()))
}

#[derive(Serialize)]
struct FlightLogStatsDto {
    count: u32,
    total_bytes: u64,
}

/// Disk usage of the per-flight JSONL recorder files. Powers the
/// Settings → Speicher section so the user knows how much is on disk
/// before they hit "alle löschen".
#[tauri::command]
fn flight_logs_stats(app: AppHandle) -> Result<FlightLogStatsDto, UiError> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|_| UiError::new("no_app_data_dir", "no app data dir"))?;
    let s = recorder::flight_logs_stats(dir).map_err(|e| {
        UiError::new("flight_logs_stats_failed", format!("could not read flight logs dir: {e}"))
    })?;
    Ok(FlightLogStatsDto {
        count: s.count,
        total_bytes: s.total_bytes,
    })
}

#[derive(Serialize)]
struct DeletedDto {
    deleted: u32,
}

/// Delete every per-flight JSONL recorder file. Triggered manually from
/// Settings — never automatically — so the active flight's log (if it
/// exists) gets removed too. The streamer just recreates it on the
/// next event append.
#[tauri::command]
fn flight_logs_delete_all(app: AppHandle) -> Result<DeletedDto, UiError> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|_| UiError::new("no_app_data_dir", "no app data dir"))?;
    let n = recorder::flight_logs_delete_all(dir).map_err(|e| {
        UiError::new("flight_logs_delete_failed", format!("could not delete flight logs: {e}"))
    })?;
    tracing::info!(deleted = n, "flight logs purged (manual)");
    Ok(DeletedDto { deleted: n })
}

/// Delete per-flight JSONL files older than `older_than_days` (mtime).
/// Called from the JS layer once per app launch when the user has the
/// auto-purge toggle on (default 30 days).
#[tauri::command]
fn flight_logs_purge_older_than(app: AppHandle, older_than_days: u32) -> Result<DeletedDto, UiError> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|_| UiError::new("no_app_data_dir", "no app data dir"))?;
    let n = recorder::flight_logs_purge_older_than(dir, older_than_days).map_err(|e| {
        UiError::new("flight_logs_purge_failed", format!("could not purge flight logs: {e}"))
    })?;
    if n > 0 {
        tracing::info!(deleted = n, days = older_than_days, "flight logs purged (auto)");
    }
    Ok(DeletedDto { deleted: n })
}

#[derive(Serialize)]
pub struct ResumableFlight {
    pirep_id: String,
    flight_number: String,
    dpt_airport: String,
    arr_airport: String,
    status: Option<String>,
}

/// Look on phpVMS for in-progress PIREPs the user could resume. Used to drive
/// the auto-detection banner on dashboard mount: if the answer is non-empty,
/// the UI offers to adopt with a 10s countdown.
///
/// Skipped (returns empty) if a flight is already attached locally —
/// either in memory (Disk-Resume already finished) OR still pending on
/// disk (Disk-Resume hasn't finished its async backfill yet but will
/// install the same PIREP shortly). Without the disk-file check, the
/// frontend's mount-time discovery race could find a separate
/// in-progress PIREP on phpVMS and offer it as a discovered flight,
/// then `flight_adopt` would crash with "another flight is already
/// active" the moment Disk-Resume committed.
#[tauri::command]
async fn flight_discover_resumable(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ResumableFlight>, UiError> {
    {
        let guard = state.active_flight.lock().expect("active_flight lock");
        if guard.is_some() {
            return Ok(Vec::new());
        }
    }
    if let Some(persisted) = read_persisted_flight(&app) {
        let age = Utc::now() - persisted.started_at;
        if age <= chrono::Duration::hours(RESUME_MAX_AGE_HOURS) {
            tracing::debug!(
                pirep_id = %persisted.pirep_id,
                "discover_resumable: disk-resume pending — returning empty so the disk flight wins"
            );
            return Ok(Vec::new());
        }
    }
    let client = current_client(&state)?;
    let pireps = client.get_user_pireps().await?;
    Ok(pireps
        .into_iter()
        .filter(|p| p.state == Some(0)) // IN_PROGRESS
        .filter_map(|p| {
            Some(ResumableFlight {
                pirep_id: p.id,
                flight_number: p.flight_number?,
                dpt_airport: p.dpt_airport_id.unwrap_or_default(),
                arr_airport: p.arr_airport_id.unwrap_or_default(),
                status: p.status,
            })
        })
        .collect())
}

/// Adopt a specific in-progress PIREP — creates the local ActiveFlight,
/// persists it to disk, starts the position streamer. Used by the resume
/// banner after the 10s countdown elapses (or the user clicks Resume now).
#[tauri::command]
async fn flight_adopt(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    pirep_id: String,
) -> Result<ActiveFlightInfo, UiError> {
    // Reserve the setup slot atomically — released automatically (via the
    // guard's Drop) on any error path, or disarmed once we've committed the
    // new ActiveFlight into state. Without this, two concurrent adopts
    // would both see active_flight=None and one would silently overwrite
    // the other (we observed this in production logs: StrictMode +
    // resume-banner re-render produced four "flight adopted" lines for a
    // single PIREP).
    let setup_guard = FlightSetupGuard::try_acquire(&state.flight_setup_in_progress)?;

    {
        let guard = state.active_flight.lock().expect("active_flight lock");
        if guard.is_some() {
            return Err(UiError::new(
                "flight_already_active",
                "another flight is already active",
            ));
        }
    }

    let client = current_client(&state)?;
    let pireps = client.get_user_pireps().await?;
    let pirep = pireps
        .into_iter()
        .find(|p| p.id == pirep_id && p.state == Some(0))
        .ok_or_else(|| {
            UiError::new(
                "pirep_not_resumable",
                "PIREP not found or no longer in progress",
            )
        })?;

    // Best-effort: look for a matching bid so we can carry fares forward to
    // the eventual file. If no bid matches, file with no fares — phpVMS keeps
    // whatever was set during prefile.
    let flight_number = pirep
        .flight_number
        .clone()
        .ok_or_else(|| UiError::new("pirep_invalid", "PIREP has no flight number"))?;
    let dpt_airport = pirep.dpt_airport_id.clone().unwrap_or_default();
    let arr_airport = pirep.arr_airport_id.clone().unwrap_or_default();

    let bids = client.get_bids().await.unwrap_or_default();
    let matching_bid = bids
        .iter()
        .find(|b| b.flight.flight_number == flight_number);
    let bid_id = matching_bid.map(|b| b.id).unwrap_or(0);
    let fares: Vec<(i64, i32)> = matching_bid
        .and_then(|b| b.flight.simbrief.as_ref())
        .and_then(|sb| sb.subfleet.as_ref())
        .map(|sf| {
            sf.fares
                .iter()
                .filter_map(|f| f.count.map(|c| (f.id, c)))
                .collect()
        })
        .unwrap_or_default();

    let airline_icao = matching_bid
        .and_then(|b| b.flight.airline.as_ref())
        .map(|a| a.icao.clone())
        .unwrap_or_default();
    let airline_logo_url = matching_bid
        .and_then(|b| b.flight.airline.as_ref())
        .and_then(|a| a.logo.clone())
        .filter(|s| !s.is_empty());

    // Look up planned registration so the activity log can compare it
    // against the live `ATC ID` SimVar — pilot sees instantly if the
    // wrong tail number is loaded in MSFS.
    // Bid.flight has no direct aircraft_id; the chosen aircraft lives
    // on the SimBrief OFP. If the pilot hasn't generated an OFP, we
    // simply leave planned_registration empty.
    // v0.3.0: zusätzlich Aircraft-ICAO + Name aus dem gleichen Call
    // ziehen. Damit der PIREP-Custom-Field "Aircraft Type" gefüllt
    // werden kann ohne extra Lookup auf der phpVMS-Detail-Seite.
    let aircraft_details = match matching_bid
        .and_then(|b| b.flight.simbrief.as_ref())
        .and_then(|sb| sb.aircraft_id)
    {
        Some(id) => client.get_aircraft(id).await.ok(),
        None => None,
    };
    let planned_registration = aircraft_details
        .as_ref()
        .and_then(|a| a.registration.clone())
        .unwrap_or_default()
        .trim()
        .to_string();
    let aircraft_icao = aircraft_details
        .as_ref()
        .and_then(|a| a.icao.clone())
        .unwrap_or_default()
        .trim()
        .to_string();
    let aircraft_name = aircraft_details
        .as_ref()
        .and_then(|a| a.name.clone())
        .unwrap_or_default()
        .trim()
        .to_string();

    let flight = Arc::new(ActiveFlight {
        pirep_id: pirep.id.clone(),
        bid_id,
        // We don't know the original prefile time; treat "now" as the start
        // for our counters. The PIREP's actual times are intact server-side.
        started_at: Utc::now(),
        airline_icao,
        airline_logo_url,
        planned_registration,
        aircraft_icao,
        aircraft_name,
        flight_number,
        dpt_airport,
        arr_airport,
        fares,
        stats: Mutex::new(FlightStats::new()),
        stop: AtomicBool::new(false),
        // Surfaced via flight_status to trigger the resume banner.
        was_just_resumed: AtomicBool::new(true),
        streamer_spawned: AtomicBool::new(false),
        cancelled_remotely: AtomicBool::new(false),
    });

    save_active_flight(&app, &flight);
    {
        let mut guard = state.active_flight.lock().expect("active_flight lock");
        *guard = Some(Arc::clone(&flight));
    }
    // ActiveFlight is now committed — release the in-progress flag so the
    // active_flight mutex alone guards subsequent adopts.
    setup_guard.disarm();
    // Don't spawn the streamer yet — the resume banner is in countdown mode
    // and the user can still cancel. `flight_resume_confirm` spawns it.
    let _ = client;

    let info = flight_info(flight.as_ref());
    log_activity(
        &state,
        ActivityLevel::Info,
        format!(
            "Adopted in-progress flight {} ({} → {})",
            format_callsign(&flight.airline_icao, &flight.flight_number),
            flight.dpt_airport,
            flight.arr_airport
        ),
        Some(format!("PIREP {}", flight.pirep_id)),
    );
    record_event(
        &app,
        &flight.pirep_id,
        &FlightLogEvent::FlightStarted {
            timestamp: Utc::now(),
            pirep_id: flight.pirep_id.clone(),
            airline_icao: flight.airline_icao.clone(),
            flight_number: flight.flight_number.clone(),
            dpt_airport: flight.dpt_airport.clone(),
            arr_airport: flight.arr_airport.clone(),
        },
    );
    Ok(info)
}

/// Start tracking a flight: prefile a PIREP and begin position streaming.
#[tauri::command]
async fn flight_start(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    bid_id: i64,
) -> Result<ActiveFlightInfo, UiError> {
    // Same race protection as flight_adopt: a double-click on "Start flight"
    // would otherwise prefile two PIREPs against the same bid.
    let setup_guard = FlightSetupGuard::try_acquire(&state.flight_setup_in_progress)?;

    {
        let guard = state.active_flight.lock().expect("active_flight lock");
        if guard.is_some() {
            return Err(UiError::new(
                "flight_already_active",
                "another flight is already active",
            ));
        }
    }

    let client = current_client(&state)?;
    let bids = client.get_bids().await?;
    let bid = bids
        .into_iter()
        .find(|b| b.id == bid_id)
        .ok_or_else(|| UiError::new("bid_not_found", "bid not found in current bids"))?;

    // ---- Pre-flight gating: must be on the ground at the departure airport ----
    let snapshot = current_snapshot(&app).ok_or_else(|| {
        UiError::new("no_sim_snapshot", "no sim snapshot yet — is the simulator connected?")
    })?;
    if !snapshot.on_ground {
        return Err(UiError::new(
            "not_on_ground",
            "you must be on the ground to start a flight",
        ));
    }

    // Cached or live fetch of the departure airport. The lock is taken in a
    // narrow scope each time so the MutexGuard never crosses an `await`.
    let dpt_icao = bid.flight.dpt_airport_id.trim().to_uppercase();
    let cached_dpt: Option<Airport> = {
        let guard = state.airports.lock().expect("airports lock");
        guard.get(&dpt_icao).cloned()
    };
    let dpt_airport = match cached_dpt {
        Some(a) => a,
        None => {
            let fetched = client.get_airport(&dpt_icao).await?;
            let mut guard = state.airports.lock().expect("airports lock");
            guard.insert(dpt_icao.clone(), fetched.clone());
            fetched
        }
    };
    if let (Some(lat), Some(lon)) = (dpt_airport.lat, dpt_airport.lon) {
        let distance_nm =
            ::geo::distance_m(snapshot.lat, snapshot.lon, lat, lon) / 1852.0;
        if distance_nm > MAX_START_DISTANCE_NM {
            return Err(UiError::new(
                "not_at_departure",
                format!(
                    "you are {:.1} nm from {} — start the flight at the departure airport",
                    distance_nm, dpt_icao
                ),
            ));
        }
        tracing::info!(
            dpt = %dpt_icao,
            distance_nm,
            "preflight gate passed"
        );
    } else {
        tracing::warn!(
            dpt = %dpt_icao,
            "no coordinates for departure airport — skipping distance check"
        );
    }

    let airline_id = bid.flight.airline.as_ref().map(|a| a.id).ok_or_else(|| {
        UiError::new("missing_airline", "bid has no airline relation")
    })?;
    let aircraft_id = bid
        .flight
        .simbrief
        .as_ref()
        .map(|sb| sb.aircraft_id)
        .flatten()
        .ok_or_else(|| {
            UiError::new(
                "missing_aircraft",
                "no aircraft on this bid — please prepare a SimBrief OFP first",
            )
        })?;

    // ---- Aircraft-mismatch gate (spec §7) ----
    // Compare the aircraft type the bid expects (from get_aircraft) to what's
    // loaded in the simulator (parsed ATC MODEL or, as a backup, the TITLE).
    // Permissive: only block when both sides resolve to clearly different codes.
    let expected_aircraft = client.get_aircraft(aircraft_id).await?;
    let expected_icao = expected_aircraft
        .icao
        .as_ref()
        .map(|s| s.trim().to_uppercase())
        .filter(|s| !s.is_empty());
    let sim_icao = snapshot
        .aircraft_icao
        .as_deref()
        .and_then(clean_atc_model);
    let sim_title = snapshot
        .aircraft_title
        .as_deref()
        .unwrap_or("")
        .to_string();
    if let (Some(expected), Some(actual)) = (expected_icao.as_ref(), sim_icao.as_ref()) {
        let title_supports_expected = title_mentions_icao(&sim_title, expected);
        // Use alias-aware comparison so e.g. "A359" (ICAO) matches
        // "A350-900" (sim long-form). See `aircraft_types_match`.
        // Live bug 2026-05-04: Emirates UAE770 A359 bid blocked
        // because sim loaded "A350-900 (No Cabin)" — same aircraft.
        let types_match_loose = aircraft_types_match(expected, actual);
        if !types_match_loose && !title_supports_expected {
            let registration = expected_aircraft
                .registration
                .as_deref()
                .unwrap_or("?");
            tracing::warn!(
                expected = %expected,
                actual = %actual,
                title = %sim_title,
                registration = %registration,
                "aircraft type mismatch — blocking flight start"
            );
            return Err(UiError::new(
                "aircraft_mismatch",
                format!(
                    "Aircraft mismatch: bid wants {expected} ({registration}), sim has {actual} (title \"{sim_title}\"). Load the correct aircraft type in the sim or pick a matching bid.",
                ),
            ));
        }
    }

    let body = PrefileBody {
        airline_id,
        aircraft_id: aircraft_id.to_string(),
        flight_number: bid.flight.flight_number.clone(),
        dpt_airport_id: bid.flight.dpt_airport_id.clone(),
        arr_airport_id: bid.flight.arr_airport_id.clone(),
        alt_airport_id: bid.flight.alt_airport_id.clone(),
        flight_type: bid.flight.flight_type.clone(),
        route_code: bid.flight.route_code.clone(),
        route_leg: bid.flight.route_leg.clone(),
        level: bid.flight.level.filter(|&l| l > 0),
        planned_distance: bid.flight.distance.as_ref().and_then(|d| d.nmi),
        planned_flight_time: bid.flight.flight_time,
        route: bid.flight.route.clone().filter(|s| !s.is_empty()),
        source_name: format!("AeroACARS/{}", env!("CARGO_PKG_VERSION")),
        notes: None,
    };

    // Before trying a fresh prefile, see if the user already has an in-progress
    // PIREP for this flight. This handles the "client crashed / persistence
    // file gone, but phpVMS still has the active PIREP" case — we adopt the
    // existing PIREP instead of trying to create a new one (which would fail
    // with aircraft-not-available because the aircraft is already "in use" by
    // the orphaned PIREP).
    let existing = match client.get_user_pireps().await {
        Ok(list) => list,
        Err(e) => {
            tracing::warn!(error = %e, "could not list user PIREPs to check for resume");
            Vec::new()
        }
    };
    let adoptable = existing.into_iter().find(|p| {
        // phpVMS PirepState IN_PROGRESS = 0.
        p.state == Some(0)
            && p.flight_number.as_deref() == Some(body.flight_number.as_str())
            && (p.airline_id.is_none() || p.airline_id == Some(airline_id))
    });
    if let Some(p) = &adoptable {
        tracing::info!(pirep_id = %p.id, "adopting existing in-progress PIREP");
    }

    tracing::info!(
        airline_id,
        aircraft_id,
        flight_number = body.flight_number.as_str(),
        adopting = adoptable.is_some(),
        "prefiling PIREP"
    );
    let pirep = if let Some(adopt) = adoptable {
        api_client::PirepCreated { id: adopt.id }
    } else {
    match client.prefile_pirep(&body).await {
        Ok(p) => p,
        Err(ApiError::Server { status: 400, body: err_body })
            if err_body.contains("aircraft-not-available") =>
        {
            // Diagnose: fetch aircraft details to tell the user *why* it's
            // unavailable (wrong airport, "in use" by an orphan PIREP, etc.).
            let detail = match client.get_aircraft(aircraft_id).await {
                Ok(a) => {
                    let reg = a
                        .registration
                        .as_deref()
                        .or(a.name.as_deref())
                        .unwrap_or("?");
                    let where_ = a.airport_id.as_deref().unwrap_or("?");
                    let state = match a.state {
                        Some(0) => "parked",
                        Some(1) => "in use",
                        Some(2) => "in flight",
                        _ => "unknown",
                    };
                    format!(
                        "{reg} (id {}): currently at {where_}, state '{state}'. Wanted at {dpt_icao}.",
                        a.id
                    )
                }
                Err(e) => format!(
                    "could not fetch aircraft {} details: {e}",
                    aircraft_id
                ),
            };
            tracing::warn!(aircraft_id, %detail, "aircraft not available");
            return Err(UiError::new(
                "aircraft_not_available",
                format!("Aircraft not available — {detail}"),
            ));
        }
        Err(ApiError::Server { status, body: err_body }) => {
            // Try to extract a human-readable message from a phpVMS JSON error body.
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&err_body) {
                if let Some(title) = json.get("title").and_then(|v| v.as_str()) {
                    return Err(UiError::new(
                        "phpvms_error",
                        format!("phpVMS rejected the flight (HTTP {status}): {title}"),
                    ));
                }
            }
            return Err(UiError::new(
                "phpvms_error",
                format!("phpVMS rejected the flight (HTTP {status})"),
            ));
        }
        Err(e) => return Err(e.into()),
        }
    };

    // Advance the PIREP status to BOARDING and ensure state is IN_PROGRESS so
    // it appears in phpVMS's "Aktive Flüge" view.
    //
    // phpVMS 7 PirepState values: REJECTED = -1, IN_PROGRESS = 0, PENDING = 1,
    // ACCEPTED = 2. We send 0 explicitly so this also recovers any PIREP that
    // accidentally got pushed to PENDING earlier (e.g. by a buggier client).
    let update_body = UpdateBody {
        state: Some(0),
        status: Some("BST".to_string()),
        source_name: Some(format!("AeroACARS/{}", env!("CARGO_PKG_VERSION"))),
        ..Default::default()
    };
    if let Err(e) = client.update_pirep(&pirep.id, &update_body).await {
        tracing::warn!(
            pirep_id = %pirep.id,
            error = %e,
            "could not advance PIREP status to BOARDING (flight will still be tracked)"
        );
    } else {
        tracing::info!(pirep_id = %pirep.id, "PIREP status set to BOARDING");
    }

    // Capture fares from the SimBrief OFP so we can file accurate loads
    // even if the bid is gone by the time we end the flight.
    let fares: Vec<(i64, i32)> = bid
        .flight
        .simbrief
        .as_ref()
        .and_then(|sb| sb.subfleet.as_ref())
        .map(|sf| {
            sf.fares
                .iter()
                .filter_map(|f| f.count.map(|c| (f.id, c)))
                .collect()
        })
        .unwrap_or_default();

    let planned_registration = expected_aircraft
        .registration
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_string();
    // v0.3.0: Aircraft-ICAO + Name aus dem expected_aircraft ziehen für
    // den PIREP-Custom-Field "Aircraft Type".
    let aircraft_icao = expected_aircraft
        .icao
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_string();
    let aircraft_name = expected_aircraft
        .name
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_string();

    // ---- Stage 2: SimBrief OFP fetch for planned-fuel comparison ----
    // The bid carries a SimBrief id when the pilot has prepared an OFP.
    // We fetch the OFP XML now (best-effort, never blocks the flight) so
    // the PIREP can compare actual fuel burn against the dispatcher's
    // plan.
    //
    // v0.4.2: Fetch-Fehler werden zusätzlich ins Activity-Log geschrieben
    // (vorher nur Tracing-Log → unsichtbar für Pilot). Pilot-Report von
    // heute: Flug hatte OFP, aber Landung-Tab zeigte trotzdem keine SOLL-
    // Werte → Fetch war silently fehlgeschlagen, niemand merkte was. Mit
    // Activity-Log-Eintrag sieht der Pilot's beim nächsten Mal sofort.
    let planned_ofp = if let Some(sb) = bid.flight.simbrief.as_ref() {
        match client.fetch_simbrief_ofp(&sb.id).await {
            Ok(Some(ofp)) => {
                tracing::info!(
                    sb_id = %sb.id,
                    plan_burn_kg = ofp.planned_burn_kg,
                    plan_block_kg = ofp.planned_block_fuel_kg,
                    plan_tow_kg = ofp.planned_tow_kg,
                    "SimBrief OFP fetched"
                );
                log_activity(
                    &state,
                    ActivityLevel::Info,
                    "SimBrief OFP geladen".to_string(),
                    Some(format!(
                        "Plan-Block {:.0} kg · Trip {:.0} kg · TOW {:.0} kg",
                        ofp.planned_block_fuel_kg,
                        ofp.planned_burn_kg,
                        ofp.planned_tow_kg
                    )),
                );
                Some(ofp)
            }
            Ok(None) => {
                tracing::warn!(sb_id = %sb.id, "SimBrief OFP fetch returned no usable data");
                log_activity(
                    &state,
                    ActivityLevel::Warn,
                    "SimBrief-OFP konnte nicht geladen werden".to_string(),
                    Some(format!(
                        "OFP-ID {} liefert keine brauchbaren Daten — Landung-Tab zeigt deshalb keine SOLL-Werte. Bitte einen frischen OFP auf simbrief.com erstellen.",
                        sb.id
                    )),
                );
                None
            }
            Err(e) => {
                tracing::warn!(sb_id = %sb.id, error = %e, "SimBrief OFP fetch failed");
                log_activity(
                    &state,
                    ActivityLevel::Warn,
                    "SimBrief-OFP-Fetch fehlgeschlagen".to_string(),
                    Some(format!(
                        "OFP-ID {} konnte nicht geladen werden ({}). Landung-Tab zeigt deshalb keine SOLL-Werte. Probier später nochmal Refresh in My-Flights, oder generier einen neuen OFP.",
                        sb.id, e
                    )),
                );
                None
            }
        }
    } else {
        // Bid hatte überhaupt keine SimBrief-Verbindung — kein Fehler,
        // aber wir sagen es dem Piloten damit er weiß warum keine
        // SOLL-Werte kommen.
        log_activity(
            &state,
            ActivityLevel::Info,
            "Kein SimBrief-OFP für diesen Flug".to_string(),
            Some("Der Bid hat keinen SimBrief-OFP gelinkt. Du kannst trotzdem fliegen — der Landung-Tab zeigt dann nur die IST-Werte ohne SOLL/Δ-Vergleich.".to_string()),
        );
        None
    };

    let flight = Arc::new(ActiveFlight {
        pirep_id: pirep.id.clone(),
        bid_id,
        started_at: Utc::now(),
        airline_icao: bid
            .flight
            .airline
            .as_ref()
            .map(|a| a.icao.clone())
            .unwrap_or_default(),
        airline_logo_url: bid
            .flight
            .airline
            .as_ref()
            .and_then(|a| a.logo.clone())
            .filter(|s| !s.is_empty()),
        planned_registration,
        aircraft_icao,
        aircraft_name,
        flight_number: bid.flight.flight_number.clone(),
        dpt_airport: bid.flight.dpt_airport_id.clone(),
        arr_airport: bid.flight.arr_airport_id.clone(),
        fares,
        stats: Mutex::new(FlightStats::new()),
        stop: AtomicBool::new(false),
        // Fresh start triggered by user — no banner needed.
        was_just_resumed: AtomicBool::new(false),
        streamer_spawned: AtomicBool::new(false),
        cancelled_remotely: AtomicBool::new(false),
    });

    save_active_flight(&app, &flight);

    {
        let mut guard = state.active_flight.lock().expect("active_flight lock");
        *guard = Some(Arc::clone(&flight));
    }
    // ActiveFlight is committed; release the setup-in-progress flag.
    setup_guard.disarm();

    // Stage 2: write the SimBrief OFP plan into FlightStats so the
    // streamer + PIREP builder can read it later. Done after commit
    // so stats already exists; the lock is short-held and never
    // crosses an await.
    if let Some(ofp) = planned_ofp {
        // Pull the navlog out before moving the rest into FlightStats so
        // we can post it to phpVMS without holding a lock across an await.
        let waypoints = ofp.waypoints.clone();
        let mut stats = flight.stats.lock().expect("flight stats lock");
        stats.planned_block_fuel_kg = Some(ofp.planned_block_fuel_kg).filter(|&v| v > 0.0);
        stats.planned_burn_kg = Some(ofp.planned_burn_kg).filter(|&v| v > 0.0);
        stats.planned_reserve_kg = Some(ofp.planned_reserve_kg).filter(|&v| v > 0.0);
        stats.planned_zfw_kg = Some(ofp.planned_zfw_kg).filter(|&v| v > 0.0);
        stats.planned_tow_kg = Some(ofp.planned_tow_kg).filter(|&v| v > 0.0);
        stats.planned_ldw_kg = Some(ofp.planned_ldw_kg).filter(|&v| v > 0.0);
        stats.planned_route = ofp.route;
        stats.planned_alternate = ofp.alternate;
        // v0.3.0: MAX-Werte für Overweight-Detection im Live-Loadsheet
        // + Score-Penalty im Landung-Tab. Filter Null-Werte aus —
        // Custom-Subfleets ohne MAX-Daten kriegen einfach keine
        // Overweight-Anzeige.
        stats.planned_max_zfw_kg = Some(ofp.max_zfw_kg).filter(|&v| v > 0.0);
        stats.planned_max_tow_kg = Some(ofp.max_tow_kg).filter(|&v| v > 0.0);
        stats.planned_max_ldw_kg = Some(ofp.max_ldw_kg).filter(|&v| v > 0.0);
        drop(stats);
        // Persist immediately so a Tauri restart mid-flight doesn't
        // lose the plan.
        save_active_flight(&app, &flight);

        // Push the planned waypoints to phpVMS so the live map / PIREP
        // detail can draw the planned track alongside the flown one.
        // Best-effort: failure here doesn't abort the flight setup.
        if !waypoints.is_empty() {
            let route: Vec<api_client::RouteWaypoint> = waypoints
                .iter()
                .enumerate()
                .map(|(i, fix)| api_client::RouteWaypoint {
                    name: fix.ident.clone(),
                    order: i as i32,
                    nav_type: simbrief_kind_to_nav_type(&fix.kind),
                    lat: fix.lat,
                    lon: fix.lon,
                })
                .collect();
            let pirep_id = pirep.id.clone();
            let route_client = client.clone();
            tauri::async_runtime::spawn(async move {
                match route_client.post_route(&pirep_id, &route).await {
                    Ok(()) => tracing::info!(
                        pirep_id = %pirep_id,
                        waypoint_count = route.len(),
                        "planned route uploaded"
                    ),
                    Err(e) => tracing::warn!(
                        pirep_id = %pirep_id,
                        error = %e,
                        "planned route upload failed"
                    ),
                }
            });
        }
    }

    spawn_position_streamer(app.clone(), Arc::clone(&flight), client.clone());
    spawn_touchdown_sampler(app.clone(), Arc::clone(&flight));

    // Fire an explicit "Phase: Boarding" entry into phpVMS' acars/logs
    // for the PIREP detail's Fluglogbuch tab. The streamer only emits
    // log lines on phase TRANSITIONS, but Boarding is the initial
    // state — there's no transition INTO it, so without this kick
    // the Fluglogbuch tab starts at "Phase: Pushback" with no record
    // of the boarding period. Best-effort, never blocks flight start.
    {
        let pirep_id = flight.pirep_id.clone();
        let log_client = client.clone();
        tauri::async_runtime::spawn(async move {
            let entry = api_client::LogEntry {
                log: "Phase: Boarding".to_string(),
                lat: None,
                lon: None,
                created_at: Some(Utc::now().to_rfc3339()),
            };
            if let Err(e) = log_client.post_acars_logs(&pirep_id, &[entry]).await {
                tracing::warn!(
                    pirep_id = %pirep_id,
                    error = %e,
                    "could not push initial Boarding ACARS log line"
                );
            }
        });
    }

    // New flight = fresh activity log. Pilots were getting confused
    // when the previous flight's events (or yesterday's) lingered at
    // the top of the log. The JSONL flight-event log on disk still
    // captures everything per-PIREP for review, but the in-app feed
    // resets here for a clean recording. Resume (`flight_adopt`)
    // skips this on purpose — there the existing log is the
    // continuation history we want to keep.
    clear_activity_log_for_new_flight(&app);

    let info = flight_info(flight.as_ref());
    log_activity(
        &state,
        ActivityLevel::Info,
        format!(
            "Flight started: {} {} → {}",
            format_callsign(&flight.airline_icao, &flight.flight_number),
            flight.dpt_airport,
            flight.arr_airport
        ),
        Some(format!("PIREP prefiled as {}", flight.pirep_id)),
    );
    // Announce the initial Boarding phase explicitly. Without this
    // entry the activity log shows the flight jumping straight from
    // "Flight started" to "Phase: Pushback" later, leaving the
    // Boarding stage of the timeline unrepresented in the textual
    // log even though the UI marks it as the active checkpoint.
    log_activity(
        &state,
        ActivityLevel::Info,
        "Phase: Boarding".to_string(),
        None,
    );
    record_event(
        &app,
        &flight.pirep_id,
        &FlightLogEvent::FlightStarted {
            timestamp: Utc::now(),
            pirep_id: flight.pirep_id.clone(),
            airline_icao: flight.airline_icao.clone(),
            flight_number: flight.flight_number.clone(),
            dpt_airport: flight.dpt_airport.clone(),
            arr_airport: flight.arr_airport.clone(),
        },
    );
    Ok(info)
}

/// Minimum thresholds a flight has to pass before we'll let it file as a clean
/// ACARS PIREP. Anything below these is either a buggy run, a half-baked test,
/// or a flight that never actually flew — phpVMS shouldn't get those.
const MIN_FLIGHT_TIME_MIN: i32 = 1;
const MIN_DISTANCE_NM: f64 = 1.0;
const MIN_POSITION_COUNT: u32 = 5;

/// Names of fields that failed validation, returned to the UI as the
/// `details` payload of the `flight_validation_failed` UiError. The UI looks
/// up `flight.validation.<key>` for the localized message and decides
/// whether the user can still file as a manual PIREP.
fn validate_for_filing(
    flight: &ActiveFlight,
    stats: &FlightStats,
    elapsed_minutes: i32,
) -> Vec<&'static str> {
    let mut missing = Vec::new();
    if elapsed_minutes < MIN_FLIGHT_TIME_MIN {
        missing.push("flight_time");
    }
    if stats.distance_nm < MIN_DISTANCE_NM {
        missing.push("distance");
    }
    if stats.position_count < MIN_POSITION_COUNT {
        missing.push("position_count");
    }
    if stats.block_fuel_kg.is_none() || stats.last_fuel_kg.is_none() {
        missing.push("fuel");
    }
    // Must have actually arrived at a parking position. Anything before
    // BlocksOn means engines weren't shut down on the ground at the gate.
    if !matches!(
        stats.phase,
        FlightPhase::BlocksOn | FlightPhase::Arrived | FlightPhase::PirepSubmitted
    ) {
        missing.push("not_arrived");
    }
    // Bid is required for a clean ACARS PIREP. `flight_adopt` falls back to
    // bid_id=0 when no matching bid is found server-side, which means the
    // pilot is flying a PIREP that no longer has a booking attached. That
    // can happen legitimately (admin removed the bid) but it's never an
    // ACARS-clean situation — admin must review.
    if flight.bid_id <= 0 {
        missing.push("no_bid");
    }
    missing
}

/// Open (or create) the offline position queue under the OS-appropriate
/// app data directory. None when we can't resolve the dir — caller
/// treats that as "no queue available, drop the offline-resilience
/// feature for this tick" rather than blowing up the streamer.
fn open_position_queue(app: &AppHandle) -> Option<PositionQueue> {
    let dir = app.path().app_data_dir().ok()?;
    PositionQueue::open(dir).ok()
}

/// Open (or create) the per-flight JSONL recorder. None when we can't
/// resolve the app data dir — caller treats it as "skip recording for
/// this tick" rather than failing the streamer.
fn open_flight_recorder(app: &AppHandle, pirep_id: &str) -> Option<FlightRecorder> {
    let dir = app.path().app_data_dir().ok()?;
    FlightRecorder::open(dir, pirep_id).ok()
}

/// Open the landing-history store. None when the app data dir can't be
/// resolved — caller treats that as "skip recording, history is best-
/// effort" rather than failing the PIREP file.
fn open_landing_store(app: &AppHandle) -> Option<LandingStore> {
    let dir = app.path().app_data_dir().ok()?;
    LandingStore::open(dir).ok()
}

/// Build a `LandingRecord` from the current flight + stats. Returns None
/// when there's no usable touchdown captured (e.g. PIREP filed before
/// landing — synthetic / bug). The record is the immutable snapshot we
/// show in the Landing tab; once written it never changes.
fn build_landing_record(
    flight: &ActiveFlight,
    stats: &FlightStats,
    sim_kind_label: Option<&str>,
    aircraft_icao: Option<&str>,
    aircraft_title: Option<&str>,
) -> Option<LandingRecord> {
    let touchdown_at = stats.landing_at?;
    let landing_rate_fpm = stats.landing_rate_fpm?;
    let score = stats.landing_score?;
    let grade = letter_grade(score.numeric());

    // Compute fuel-efficiency once so the Landing tab doesn't have to
    // redo the formula. Same shape as build_pirep_fields.
    let actual_burn = match (stats.takeoff_fuel_kg, stats.landing_fuel_kg) {
        (Some(toff), Some(land)) if toff > land && toff > 0.0 && land >= 0.0 => Some(toff - land),
        _ => None,
    };
    let (fuel_diff_kg, fuel_pct) = match (stats.planned_burn_kg, actual_burn) {
        (Some(plan), Some(actual)) if plan > 0.0 => {
            let d = actual - plan;
            (Some(d), Some((d / plan) * 100.0))
        }
        _ => (None, None),
    };

    let runway_match = stats.runway_match.as_ref().map(|m| LandingRunwayMatch {
        airport_ident: m.airport_ident.clone(),
        runway_ident: m.runway_ident.clone(),
        surface: m.surface.clone(),
        length_ft: m.length_ft as f64,
        centerline_distance_m: m.centerline_distance_m,
        centerline_distance_abs_ft: m.centerline_distance_abs_ft,
        side: m.side.clone(),
        touchdown_distance_from_threshold_ft: m.touchdown_distance_from_threshold_ft,
    });

    let touchdown_profile = stats
        .touchdown_profile
        .iter()
        .map(|p| LandingProfilePoint {
            t_ms: p.t_ms,
            vs_fpm: p.vs_fpm,
            g_force: p.g_force,
            agl_ft: p.agl_ft,
            on_ground: p.on_ground,
            heading_true_deg: p.heading_true_deg,
            groundspeed_kt: p.groundspeed_kt,
            indicated_airspeed_kt: p.indicated_airspeed_kt,
            pitch_deg: p.pitch_deg,
            bank_deg: p.bank_deg,
        })
        .collect();

    let approach_samples = stats
        .approach_buffer
        .iter()
        .map(|s| ApproachSample {
            vs_fpm: s.vs_fpm,
            bank_deg: s.bank_deg,
        })
        .collect();

    Some(LandingRecord {
        pirep_id: flight.pirep_id.clone(),
        touchdown_at,
        recorded_at: Utc::now(),
        flight_number: flight.flight_number.clone(),
        airline_icao: flight.airline_icao.clone(),
        dpt_airport: flight.dpt_airport.clone(),
        arr_airport: flight.arr_airport.clone(),
        aircraft_registration: Some(flight.planned_registration.clone())
            .filter(|s| !s.is_empty()),
        aircraft_icao: aircraft_icao.map(|s| s.to_string()).filter(|s| !s.is_empty()),
        aircraft_title: aircraft_title.map(|s| s.to_string()).filter(|s| !s.is_empty()),
        sim_kind: sim_kind_label.map(|s| s.to_string()),

        score_numeric: score.numeric(),
        score_label: score.label().to_string(),
        grade_letter: grade.to_string(),

        landing_rate_fpm,
        landing_peak_vs_fpm: stats.landing_peak_vs_fpm,
        landing_g_force: stats.landing_g_force,
        landing_peak_g_force: stats.landing_peak_g_force,
        landing_pitch_deg: stats.landing_pitch_deg,
        // Bank at touchdown isn't a top-level FlightStats field; pull
        // the sample closest to t=0 from the touchdown profile.
        landing_bank_deg: stats
            .touchdown_profile
            .iter()
            .min_by_key(|p| p.t_ms.abs())
            .map(|p| p.bank_deg),
        landing_speed_kt: stats.landing_speed_kt,
        landing_heading_deg: stats.landing_heading_deg,
        landing_weight_kg: stats.landing_weight_kg,
        touchdown_sideslip_deg: stats.touchdown_sideslip_deg,
        bounce_count: stats.bounce_count,

        headwind_kt: stats.landing_headwind_kt,
        crosswind_kt: stats.landing_crosswind_kt,

        approach_vs_stddev_fpm: stats.approach_vs_stddev_fpm,
        approach_bank_stddev_deg: stats.approach_bank_stddev_deg,
        rollout_distance_m: stats.rollout_distance_m,

        planned_block_fuel_kg: stats.planned_block_fuel_kg,
        planned_burn_kg: stats.planned_burn_kg,
        planned_tow_kg: stats.planned_tow_kg,
        planned_ldw_kg: stats.planned_ldw_kg,
        planned_zfw_kg: stats.planned_zfw_kg,
        actual_trip_burn_kg: actual_burn,
        fuel_efficiency_kg_diff: fuel_diff_kg,
        fuel_efficiency_pct: fuel_pct,
        takeoff_weight_kg: stats.takeoff_weight_kg,
        takeoff_fuel_kg: stats.takeoff_fuel_kg,
        landing_fuel_kg: stats.landing_fuel_kg,
        block_fuel_kg: stats.block_fuel_kg,

        runway_match,
        touchdown_profile,
        approach_samples,
    })
}

/// Persist a landing record from a flight that just filed its PIREP.
/// Best-effort — failure is logged and ignored so the file path
/// doesn't fail because the history disk write blew up.
fn record_landing_for_filed_flight(
    app: &AppHandle,
    flight: &ActiveFlight,
    stats: &FlightStats,
) {
    let snapshot = current_snapshot(app);
    let sim_kind_label = read_sim_config(app).kind;
    let sim_label = match sim_kind_label {
        SimKind::Off => None,
        SimKind::Msfs2020 | SimKind::Msfs2024 => Some("MSFS"),
        SimKind::XPlane11 | SimKind::XPlane12 => Some("X-PLANE"),
    };
    let aircraft_icao = snapshot.as_ref().and_then(|s| s.aircraft_icao.as_deref());
    let aircraft_title = snapshot.as_ref().and_then(|s| s.aircraft_title.as_deref());

    let Some(record) = build_landing_record(
        flight,
        stats,
        sim_label,
        aircraft_icao,
        aircraft_title,
    ) else {
        tracing::debug!(
            pirep_id = %flight.pirep_id,
            "no touchdown captured — skipping landing-history record"
        );
        return;
    };
    let Some(store) = open_landing_store(app) else {
        tracing::warn!("landing store unavailable — landing not recorded");
        return;
    };
    if let Err(e) = store.upsert(record) {
        tracing::warn!(error = ?e, "could not persist landing record");
    } else {
        tracing::info!(pirep_id = %flight.pirep_id, "landing record persisted");
    }
}

/// Best-effort append to the flight log. Swallows errors so a missing
/// disk doesn't surface as a UI failure — the log is purely for replay
/// and post-flight analysis.
fn record_event(app: &AppHandle, pirep_id: &str, event: &FlightLogEvent) {
    if let Some(rec) = open_flight_recorder(app, pirep_id) {
        if let Err(e) = rec.append(event) {
            tracing::warn!(error = ?e, "could not append to flight log");
        }
    }
}

/// Drain queued positions for `pirep_id` by replaying each one through
/// the live phpVMS client. Stops at the first failure and writes the
/// remaining rows back to the queue file so the next tick can retry.
/// Older rows for *other* PIREPs are kept in place (so they replay
/// once the matching flight resumes); only matching rows are touched.
async fn drain_position_queue(queue: &PositionQueue, client: &Client, pirep_id: &str) {
    let items = match queue.read_all() {
        Ok(v) if !v.is_empty() => v,
        _ => return,
    };
    let mut still_pending: Vec<QueuedPosition> = Vec::new();
    let mut drained_now: usize = 0;
    let mut hit_failure = false;
    for q in items {
        if hit_failure || q.pirep_id != pirep_id {
            still_pending.push(q);
            continue;
        }
        // Round-trip through PositionEntry so we use the same client
        // helper — keeps the wire format identical to fresh posts.
        let position: PositionEntry = match serde_json::from_value(q.position.clone()) {
            Ok(p) => p,
            Err(e) => {
                // Bad row — drop it so we don't block forever.
                tracing::warn!(error = %e, "discarding malformed queued position");
                continue;
            }
        };
        match client.post_positions(&q.pirep_id, &[position]).await {
            Ok(()) => {
                drained_now += 1;
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    pending = still_pending.len(),
                    "queue drain failed; will retry next tick"
                );
                still_pending.push(q);
                hit_failure = true;
            }
        }
    }
    if let Err(e) = queue.replace(&still_pending) {
        tracing::warn!(error = ?e, "could not rewrite position queue");
    } else if drained_now > 0 {
        tracing::info!(drained_now, remaining = still_pending.len(), "drained queued positions");
    }
}

/// `GET /metar/{icao}` — fetch the current METAR for an airport.
/// Cached per-flight in `FlightStats` (departure + arrival), so the
/// dashboard / future briefing panel can reuse it without round-trips.
/// Errors map to a serializable `UiError` so the frontend can show a
/// localized message instead of a Rust enum.
#[tauri::command]
async fn metar_get(icao: String) -> Result<MetarSnapshot, UiError> {
    metar::fetch_metar(&icao).await.map_err(|e| match e {
        MetarError::NotFound(_) => UiError::new("metar_not_found", e.to_string()),
        MetarError::Network(_) => UiError::new("metar_network", e.to_string()),
        MetarError::Status(_) => UiError::new("metar_upstream", e.to_string()),
        MetarError::Parse(_) => UiError::new("metar_parse", e.to_string()),
    })
}

/// Compute the great-circle distance (nm) from the live sim position to
/// the airport with the given ICAO. Returns `None` when we don't have
/// a sim snapshot yet, can't resolve the airport, or the airport
/// coordinates are zero (typical for stub records). Used by
/// `flight_end` to enforce the "you have to actually be there to file"
/// rule. Reads the airport from the in-memory cache first; only goes
/// to the network if we haven't seen the ICAO yet.
async fn compute_distance_to_airport(
    app: &AppHandle,
    state: &tauri::State<'_, AppState>,
    icao: &str,
) -> Option<f64> {
    let snap = current_snapshot(app)?;
    let key = icao.trim().to_uppercase();

    // Cache hit: synchronous lookup, no network call.
    let cached: Option<Airport> = {
        let guard = state.airports.lock().expect("airports lock");
        guard.get(&key).cloned()
    };
    let airport = match cached {
        Some(a) => a,
        None => {
            // Cache miss: fetch via phpVMS. Best-effort — if the lookup
            // fails (network down, airport not in DB) we skip the check
            // rather than blocking the file.
            let client = state.client.lock().expect("client mutex").clone()?;
            let fetched = client.get_airport(&key).await.ok()?;
            let mut guard = state.airports.lock().expect("airports lock");
            guard.insert(key.clone(), fetched.clone());
            fetched
        }
    };

    let lat = airport.lat?;
    let lon = airport.lon?;
    if lat == 0.0 && lon == 0.0 {
        return None;
    }
    let meters = ::geo::distance_m(snap.lat, snap.lon, lat, lon);
    Some(meters / 1852.0)
}

/// File the active PIREP with computed final stats. Refuses to file if any
/// of the minimum-quality checks in `validate_for_filing` fail — the UI then
/// surfaces a dialog letting the user cancel the flight or file as a manual
/// PIREP via [`flight_end_manual`].
/// `divert_to` is optional and carries the ICAO of the actual landing
/// airport when the pilot is filing a divert. JS side passes this when
/// the user clicks "Submit as divert to X" on the divert-detected
/// banner. When `Some`, the proximity-to-planned-arrival validation is
/// skipped, the FileBody includes `arr_airport_id` to override the
/// bid's planned destination, and a "DIVERT: X → Y" line is prepended
/// to the notes.
#[tauri::command]
async fn flight_end(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    divert_to: Option<String>,
) -> Result<(), UiError> {
    let divert_to = divert_to
        .as_deref()
        .map(|s| s.trim().to_uppercase())
        .filter(|s| !s.is_empty());

    // Pull the arrival ICAO so we can resolve its coords without holding
    // the active_flight lock across the network call.
    let arr_icao = {
        let guard = state.active_flight.lock().expect("active_flight lock");
        let flight = guard
            .as_ref()
            .ok_or_else(|| UiError::new("no_active_flight", "no flight is active"))?;
        flight.arr_airport.clone()
    };
    // Compute distance to the planned arrival. Used as an extra
    // pre-flight-filing gate ("not_at_arrival") so the pilot can't
    // file a flight from 200 nm out — they have to taxi to the gate or
    // file as a manual divert via flight_end_manual.
    //
    // SKIPPED entirely when filing as a divert — by definition the
    // pilot is NOT at the planned arrival, that's the whole point.
    let distance_to_arr_nm = if divert_to.is_some() {
        None
    } else {
        compute_distance_to_airport(&app, &state, &arr_icao).await
    };

    // Validate WITHOUT removing the flight from state, so a failed validation
    // leaves the user able to retry, edit-and-file-manual, or cancel.
    {
        let guard = state.active_flight.lock().expect("active_flight lock");
        let flight = guard
            .as_ref()
            .ok_or_else(|| UiError::new("no_active_flight", "no flight is active"))?;
        let stats = flight.stats.lock().expect("flight stats");
        let elapsed_minutes = (Utc::now() - flight.started_at).num_minutes() as i32;
        let mut missing = validate_for_filing(flight, &stats, elapsed_minutes);
        if let Some(d) = distance_to_arr_nm {
            if d > MAX_FILE_DISTANCE_NM {
                missing.push("not_at_arrival");
            }
        }
        if !missing.is_empty() {
            tracing::warn!(
                pirep_id = %flight.pirep_id,
                missing = ?missing,
                distance_to_arr_nm,
                "rejecting PIREP filing — validation failed"
            );
            return Err(UiError::new(
                "flight_validation_failed",
                "flight does not meet minimum requirements for an ACARS PIREP",
            )
            .with_details(serde_json::json!({ "missing": missing })));
        }
    }

    // Validation passed — now actually take the flight out of state and file.
    let flight = {
        let mut guard = state.active_flight.lock().expect("active_flight lock");
        guard
            .take()
            .ok_or_else(|| UiError::new("no_active_flight", "no flight is active"))?
    };
    flight.stop.store(true, Ordering::Relaxed);

    let client = current_client(&state)?;

    // Snapshot all stats inside a single short-lived guard to avoid holding
    // the Mutex across an `await`.
    let (body, block_on_iso) = {
        let stats = flight.stats.lock().expect("flight stats");
        // Flight time = takeoff → landing (when both timestamps were
        // captured by the FSM). Falls back to the started_at → now
        // window only if takeoff/landing weren't observed (e.g.
        // manual file before the FSM advanced through Takeoff). The
        // takeoff→landing range matches what phpVMS expects in its
        // native Flt.Time column.
        let flight_time = match (stats.takeoff_at, stats.landing_at) {
            (Some(t), Some(l)) if l > t => Some((l - t).num_minutes() as i32),
            _ => Some(((Utc::now() - flight.started_at).num_minutes() as i32).max(0)),
        };

        let fares = if flight.fares.is_empty() {
            None
        } else {
            Some(
                flight
                    .fares
                    .iter()
                    .map(|(id, count)| FareEntry {
                        id: *id,
                        count: *count,
                    })
                    .collect(),
            )
        };

        // Block→remaining diff in kg, converted to pounds for phpVMS.
        // We round to whole kilograms BEFORE the lb conversion so the
        // PIREP-detail page shows clean integer values (`9733 kg`
        // instead of `9733.45 kg`). Real-world refuelling has no
        // sub-kg precision anyway, and a half-kg loss is invisible
        // to pilots — what's NOT invisible is `9733.45 kg` looking
        // like a unit-mix-up bug. Same logic for fuel_used.
        let fuel_used = match (stats.block_fuel_kg, stats.last_fuel_kg) {
            (Some(b), Some(c)) if b > c => Some(((b - c) as f64).round() * KG_TO_LB),
            _ => None,
        };
        // Block fuel sent natively so phpVMS computes "Verbleibender
        // Treibstoff" correctly (= block_fuel - fuel_used). Without
        // this the dashboard shows "-fuel_used kg" because the
        // missing block_fuel defaults to 0. Same unit as fuel_used.
        let block_fuel = stats
            .block_fuel_kg
            .filter(|kg| *kg > 0.0)
            .map(|kg| (kg as f64).round() * KG_TO_LB);
        // Cruise level = peak MSL altitude observed. phpVMS column
        // `Flt.Level`. Rounded to the nearest 100 ft so the value
        // matches the conventional FL display.
        let level = stats.peak_altitude_ft.map(|ft| {
            let rounded = ((ft / 100.0).round() * 100.0) as i32;
            rounded.max(0)
        });
        // Native landing rate field — same value as the custom field
        // but on the native column phpVMS shows on the PIREP
        // overview.
        let landing_rate = stats.landing_rate_fpm.map(|v| v as f64);
        let score = stats.landing_score.map(|s| s.numeric());
        let distance_nm = stats.distance_nm;
        let fields = build_pirep_fields(&flight, &stats);
        let mut notes = build_pirep_notes(&flight, &stats);
        // Prepend a divert banner to the notes so the VA admin sees
        // immediately on the PIREP page that this wasn't a normal
        // arrival. Format mirrors what most ACARS clients write
        // ("DIVERT: <planned> → <actual>") so admins can grep.
        if let Some(actual) = divert_to.as_deref() {
            notes = format!(
                "DIVERT: {} → {} (planned destination not reached)\n\n{}",
                arr_icao, actual, notes
            );
        }

        let body = FileBody {
            flight_time,
            fuel_used,
            block_fuel,
            distance: Some(distance_nm),
            level,
            landing_rate,
            score,
            source_name: Some(format!("AeroACARS/{}", env!("CARGO_PKG_VERSION"))),
            notes: Some(notes),
            fares,
            fields: Some(fields),
            arr_airport_id: divert_to.clone(),
        };
        // Block-on time = touchdown timestamp captured by the FSM.
        // Needed by the divert-finalize path because we skip /file
        // entirely there, so phpVMS won't auto-set this column.
        let block_on_iso = stats.landing_at.map(|t| t.to_rfc3339());
        (body, block_on_iso)
    };
    // v0.3.1: Divert finalization bypasses /file entirely.
    //
    // Background: earlier versions tried to route diverts into PENDING via
    // (a) a pre-file source=MANUAL "smuggle" through the update endpoint,
    // and (b) a post-file state=PENDING update. Both are dead on real
    // phpVMS deployments:
    //   * Acars\PirepController::file() ignores the stored source field
    //     and always evaluates auto_approve_acars on the rank, so (a)
    //     does not route through auto_approve_manual.
    //   * Once the PIREP is ACCEPTED, PirepController::checkReadOnly()
    //     blocks any further state-update, so (b) returns "PIREP is
    //     read-only" (verified against german-sky-group.eu, 2026-05-04).
    //
    // The path that actually works: while the PIREP is still IN_PROGRESS
    // (not read-only), mass-assign EVERYTHING /file would have written —
    // including state=PENDING, source=MANUAL, arr_airport_id, all final
    // stats — through a single update_pirep call. This bypasses
    // PirepService::submit() and the auto-approve check entirely. The
    // PIREP shows up in the admin's PENDING queue with the correct
    // arrival airport and divert notes.
    //
    // Strategy verified against phpvms@dev: PirepController::update +
    // parsePirep() pass the full request payload to mass-assignment, and
    // (source, state, arr_airport_id, landing_rate, score, submitted_at,
    // block_on_time) are all in Pirep $fillable. Acars\UpdateRequest
    // doesn't strip non-validated keys.
    if divert_to.is_some() {
        let now_iso = Utc::now().to_rfc3339();
        let finalize = api_client::UpdateBody {
            state: Some(1),                                   // PirepState::PENDING
            source: Some(api_client::pirep_source::MANUAL),   // 1
            status: Some("ONB".to_string()),                  // PirepStatus::ARRIVED
            flight_time: body.flight_time,
            distance: body.distance,
            fuel_used: body.fuel_used,
            block_fuel: body.block_fuel,
            level: body.level,
            landing_rate: body.landing_rate,
            score: body.score,
            source_name: body.source_name.clone(),
            notes: body.notes.clone(),
            arr_airport_id: divert_to.clone(),
            submitted_at: Some(now_iso.clone()),
            block_on_time: block_on_iso.clone(),
            updated_at: Some(now_iso),
        };
        match client.update_pirep(&flight.pirep_id, &finalize).await {
            Ok(()) => tracing::info!(
                pirep_id = %flight.pirep_id,
                "divert finalize update OK — PIREP mass-assigned to MANUAL/PENDING"
            ),
            Err(e) => {
                log_activity(
                    &state,
                    ActivityLevel::Error,
                    "Divert: Finalisierung fehlgeschlagen".to_string(),
                    Some(format!("{} — Flug bleibt aktiv für Retry", e)),
                );
                let mut guard = state.active_flight.lock().expect("active_flight lock");
                *guard = Some(flight);
                return Err(e.into());
            }
        }
        // Custom PIREP fields live in `pirep_field_values` (separate
        // table from `pireps`) — they don't go through Pirep
        // mass-assignment. Push them via the dedicated endpoint.
        // Failures are non-fatal: the main PIREP is already in PENDING.
        if let Some(fields_map) = body.fields.as_ref() {
            if let Err(e) = client.post_pirep_fields(&flight.pirep_id, fields_map).await {
                tracing::warn!(
                    pirep_id = %flight.pirep_id,
                    error = %e,
                    "post_pirep_fields after divert finalize failed (non-fatal)"
                );
            }
        }
        // Verify the PIREP actually landed in PENDING. The mass-assign
        // strategy is well-trodden but the verification is cheap and
        // gives us loud feedback if a future phpVMS upgrade changes the
        // semantics. Failure here doesn't unwind the file — it just
        // surfaces a warning so the pilot knows to ping the VA admin.
        match client.get_pirep(&flight.pirep_id).await {
            Ok(p) => {
                let s = p.state.unwrap_or(-1);
                if s == 1 {
                    tracing::info!(
                        pirep_id = %flight.pirep_id,
                        "verified: divert PIREP state == PENDING"
                    );
                } else {
                    tracing::warn!(
                        pirep_id = %flight.pirep_id,
                        actual_state = s,
                        "divert PIREP did NOT land in PENDING — phpVMS semantics changed?"
                    );
                    log_activity(
                        &state,
                        ActivityLevel::Error,
                        "Divert: konnte NICHT für Admin-Review markiert werden".to_string(),
                        Some(format!(
                            "Server-Status nach Divert-Finalize: {} (erwartet: 1=PENDING). Bitte VA-Admin manuell informieren.",
                            s
                        )),
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    pirep_id = %flight.pirep_id,
                    error = %e,
                    "post-divert state verify failed"
                );
            }
        }
        // Local snapshot + activity log + bid cleanup, mirroring the
        // /file success path so downstream UI (Landung-Tab, Activity)
        // sees the same state for divert and normal arrivals.
        {
            let stats = flight.stats.lock().expect("flight stats");
            record_landing_for_filed_flight(&app, &flight, &stats);
        }
        clear_persisted_flight(&app);
        log_activity(
            &state,
            ActivityLevel::Info,
            format!(
                "PIREP filed: {} {} → {} (DIVERT, planned {})",
                format_callsign(&flight.airline_icao, &flight.flight_number),
                flight.dpt_airport,
                divert_to.as_deref().unwrap_or(&flight.arr_airport),
                flight.arr_airport,
            ),
            {
                let dist = body.distance.unwrap_or(0.0);
                let fuel = body.fuel_used.unwrap_or(0.0);
                let stats_line = if fuel > 0.0 {
                    format!("Distance {dist:.1} nm, fuel {fuel:.0} lb")
                } else {
                    format!("Distance {dist:.1} nm")
                };
                Some(format!("{stats_line} · MANUAL/PENDING (admin review)"))
            },
        );
        record_event(
            &app,
            &flight.pirep_id,
            &FlightLogEvent::FlightEnded {
                timestamp: Utc::now(),
                pirep_id: flight.pirep_id.clone(),
                outcome: FlightOutcome::Filed,
            },
        );
        // v0.4.0: Discord-Webhook für Divert. Fire-and-forget — wir
        // wollen Discord NIE im File-Pfad warten lassen.
        {
            let actual_arr = divert_to
                .as_deref()
                .unwrap_or(&flight.arr_airport)
                .to_string();
            let cached_pilot = state
                .cached_pilot
                .lock()
                .expect("cached_pilot lock")
                .clone();
            let (pilot_ident, pilot_name) = match cached_pilot {
                Some((id, name)) => (Some(id), Some(name)),
                None => (None, None),
            };
            let stats_for_post = flight.stats.lock().expect("flight stats");
            let ctx = discord::EventContext {
                callsign: format_callsign(&flight.airline_icao, &flight.flight_number),
                airline_icao: flight.airline_icao.clone(),
                airline_logo_url: flight.airline_logo_url.clone(),
                dpt_icao: flight.dpt_airport.clone(),
                arr_icao: actual_arr,
                planned_arr_icao: Some(flight.arr_airport.clone()),
                aircraft_type: Some(flight.aircraft_icao.clone()).filter(|s| !s.is_empty()),
                aircraft_reg: Some(flight.planned_registration.clone()).filter(|s| !s.is_empty()),
                pilot_ident,
                pilot_name,
                distance_nm: body.distance,
                flight_time_min: body.flight_time,
                score: stats_for_post.landing_score.map(|s| s.numeric()),
                ..Default::default()
            };
            drop(stats_for_post);
            tokio::spawn(discord::post_event(discord::EventKind::Divert, ctx));
        }
        consume_bid_best_effort(&client, flight.bid_id).await;
        // Drop the in-memory active flight: divert is finalized.
        let _ = state.active_flight.lock().expect("active_flight lock").take();
        return Ok(());
    }
    tracing::info!(
        pirep_id = %flight.pirep_id,
        flight_time = body.flight_time.unwrap_or(0),
        distance = body.distance.unwrap_or(0.0),
        fuel_used = body.fuel_used.unwrap_or(0.0),
        fare_classes = flight.fares.len(),
        custom_fields = body.fields.as_ref().map(|f| f.len()).unwrap_or(0),
        "filing PIREP"
    );
    // Non-divert path. (Diverts return early above via the dedicated
    // mass-assign-to-PENDING flow; only normal arrivals reach /file.)
    match client.file_pirep(&flight.pirep_id, &body).await {
        Ok(()) => {
            // Snapshot the landing into the local history file BEFORE
            // we drop the persisted flight — gives the new "Landung"
            // tab something to render even after the PIREP is filed
            // and the in-memory FlightStats goes out of scope.
            {
                let stats = flight.stats.lock().expect("flight stats");
                record_landing_for_filed_flight(&app, &flight, &stats);
            }
            clear_persisted_flight(&app);
            log_activity(
                &state,
                ActivityLevel::Info,
                format!(
                    "PIREP filed: {} {} → {}",
                    format_callsign(&flight.airline_icao, &flight.flight_number),
                    flight.dpt_airport,
                    flight.arr_airport,
                ),
                {
                    let dist = body.distance.unwrap_or(0.0);
                    let fuel = body.fuel_used.unwrap_or(0.0);
                    if fuel > 0.0 {
                        Some(format!("Distance {dist:.1} nm, fuel {fuel:.0} lb"))
                    } else {
                        Some(format!("Distance {dist:.1} nm"))
                    }
                },
            );
            record_event(
                &app,
                &flight.pirep_id,
                &FlightLogEvent::FlightEnded {
                    timestamp: Utc::now(),
                    pirep_id: flight.pirep_id.clone(),
                    outcome: FlightOutcome::Filed,
                },
            );
            // v0.4.0: Discord-Webhook für regulären File-Erfolg.
            // Fire-and-forget. Divert hat einen eigenen Hook oben
            // (anderes Embed mit DIVERT-Banner).
            {
                let cached_pilot = state
                    .cached_pilot
                    .lock()
                    .expect("cached_pilot lock")
                    .clone();
                let (pilot_ident, pilot_name) = match cached_pilot {
                    Some((id, name)) => (Some(id), Some(name)),
                    None => (None, None),
                };
                let stats_for_post = flight.stats.lock().expect("flight stats");
                let ctx = discord::EventContext {
                    callsign: format_callsign(&flight.airline_icao, &flight.flight_number),
                    airline_icao: flight.airline_icao.clone(),
                    airline_logo_url: flight.airline_logo_url.clone(),
                    dpt_icao: flight.dpt_airport.clone(),
                    arr_icao: flight.arr_airport.clone(),
                    aircraft_type: Some(flight.aircraft_icao.clone()).filter(|s| !s.is_empty()),
                    aircraft_reg: Some(flight.planned_registration.clone()).filter(|s| !s.is_empty()),
                    pilot_ident,
                    pilot_name,
                    distance_nm: body.distance,
                    flight_time_min: body.flight_time,
                    score: stats_for_post.landing_score.map(|s| s.numeric()),
                    ..Default::default()
                };
                drop(stats_for_post);
                tokio::spawn(discord::post_event(discord::EventKind::PirepFiled, ctx));
            }
            // v0.5.11: MQTT live-tracking PIREP publish. Best-effort,
            // fire-and-forget. Monitor uses this to mark a flight
            // as completed in the live history.
            {
                let mqtt = state.mqtt.lock().await;
                if let Some(handle) = mqtt.as_ref() {
                    // Snapshot the rich stats inside a short-lived scope so
                    // the std::sync::MutexGuard doesn't span any later
                    // .await — same pattern as the touchdown publish.
                    let pirep_payload = {
                        let stats = flight.stats.lock().expect("flight stats");
                        let touchdown_count = stats.touchdown_events.len() as u32;
                        aeroacars_mqtt::PirepPayload {
                            ts: Utc::now().timestamp_millis(),
                            pirep_id: flight.pirep_id.clone(),
                            flight_number: format_callsign(
                                &flight.airline_icao,
                                &flight.flight_number,
                            ),
                            dep: flight.dpt_airport.clone(),
                            arr: flight.arr_airport.clone(),
                            block_time_min: body.flight_time,
                            flight_time_min: body.flight_time,
                            distance_nm: body.distance.map(|d| d as f32),
                            fuel_used_kg: body.fuel_used.map(|kg| kg as f32),
                            planned_burn_kg: stats.planned_burn_kg,
                            block_fuel_kg: stats.block_fuel_kg,
                            takeoff_fuel_kg: stats.takeoff_fuel_kg,
                            landing_fuel_kg: stats.landing_fuel_kg,
                            takeoff_weight_kg: stats.takeoff_weight_kg.map(|w| w as f32),
                            landing_weight_kg: stats.landing_weight_kg.map(|w| w as f32),
                            planned_tow_kg: stats.planned_tow_kg,
                            planned_ldw_kg: stats.planned_ldw_kg,
                            peak_altitude_ft: stats.peak_altitude_ft.map(|v| v.round() as i32),
                            landing_vs_fpm: body.landing_rate.map(|r| r as i32),
                            landing_score: stats.landing_score.map(|s| s.numeric()),
                            go_around_count: Some(stats.go_around_count),
                            touchdown_count: Some(touchdown_count),
                            dep_gate: stats.dep_gate.clone(),
                            arr_gate: stats.arr_gate.clone(),
                            approach_runway: stats.approach_runway.clone(),
                            divert: stats.divert_hint.as_ref().map(|_| true),
                            diverted_to: stats
                                .divert_hint
                                .as_ref()
                                .and_then(|h| h.actual_icao.clone()),
                            notes: None,
                        }
                    };
                    handle.pirep(pirep_payload);
                }
            }
            // v0.5.23 Forensik-Upload: gzip + POST des kompletten JSONL-
            // Logfiles an aeroacars-live damit der VA-Owner ohne den
            // Piloten zu kontaktieren das vollstaendige SimSnapshot-Log
            // (80 Felder) + Activity-Log + PhaseChanged-Stream pro
            // Session abrufen kann. Fire-and-forget, blockt PIREP-File
            // nicht — Failure landet nur im Log.
            spawn_flight_log_upload(&app, flight.pirep_id.clone());
            consume_bid_best_effort(&client, flight.bid_id).await;
            Ok(())
        }
        Err(e) => {
            // File call failed — don't drop the flight on the floor. Put it
            // back in state so the user can retry instead of orphaning the
            // PIREP server-side.
            log_activity(
                &state,
                ActivityLevel::Error,
                "PIREP file failed",
                Some(format!("{} — flight kept in state for retry", e)),
            );
            let mut guard = state.active_flight.lock().expect("active_flight lock");
            *guard = Some(flight);
            Err(e.into())
        }
    }
}

/// Best-effort: drop the bid that was used for this flight. phpVMS does NOT
/// auto-consume bids on PIREP file, so without this the booked flight stays
/// in the pilot's bid list even after Accepted. A failure here is non-fatal
/// — the user can clean up via the phpVMS UI if needed.
async fn consume_bid_best_effort(client: &Client, bid_id: i64) {
    if bid_id <= 0 {
        return;
    }
    match client.delete_bid(bid_id).await {
        Ok(()) => tracing::info!(bid_id, "bid removed after PIREP filing"),
        Err(e) => tracing::warn!(
            bid_id,
            error = %e,
            "could not remove bid after PIREP filing — please clean up manually"
        ),
    }
}

/// File the active PIREP as a *manual* report — bypasses the validation in
/// `flight_end` and tags the source as manual so the VA admin knows the data
/// wasn't 100% machine-collected.
///
/// Use cases:
///   * Flight ended with broken stats (e.g. resumed run that lost block_fuel)
///     but pilot still wants to file rather than cancel.
///   * Divert: planned EDDP→EDDF, actually landed EDDV. The pilot supplies
///     `divert_to = "EDDV"` and a `reason`; admin sees the divert tag in the
///     notes and adjusts the PIREP airports manually.
///   * Sim disconnect mid-flight: the FSM never saw landing, so flight_time /
///     fuel_used / landing_rate are all wrong. Pilot supplies the override
///     fields below and we ship those instead of the (broken) FSM values.
///
/// Every override field is `Option<...>`; `None` falls back to whatever the
/// FSM captured. All overrides also get tagged in the notes so the admin
/// can see WHICH fields the pilot edited by hand.
///
/// `block_off_at_iso` / `block_on_at_iso` accept any RFC-3339 timestamp
/// (typically `2026-05-02T15:30:00Z`); blank uses the FSM-captured time.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn flight_end_manual(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    notes_override: Option<String>,
    divert_to: Option<String>,
    reason: Option<String>,
    flight_time_minutes: Option<i32>,
    block_fuel_kg: Option<f32>,
    fuel_used_kg: Option<f32>,
    distance_nm: Option<f64>,
    cruise_level_ft: Option<i32>,
    landing_rate_fpm: Option<f32>,
    block_off_at_iso: Option<String>,
    block_on_at_iso: Option<String>,
) -> Result<(), UiError> {
    let flight = {
        let mut guard = state.active_flight.lock().expect("active_flight lock");
        guard
            .take()
            .ok_or_else(|| UiError::new("no_active_flight", "no flight is active"))?
    };
    flight.stop.store(true, Ordering::Relaxed);
    let client = current_client(&state)?;

    // Parse RFC-3339 block-off/on overrides if present. Anything that
    // doesn't parse cleanly is dropped — we don't want a typo to
    // silently file a PIREP with a bogus "1970" timestamp.
    let block_off_override: Option<DateTime<Utc>> = block_off_at_iso
        .as_deref()
        .and_then(|s| {
            if s.trim().is_empty() {
                None
            } else {
                DateTime::parse_from_rfc3339(s.trim())
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            }
        });
    let block_on_override: Option<DateTime<Utc>> = block_on_at_iso
        .as_deref()
        .and_then(|s| {
            if s.trim().is_empty() {
                None
            } else {
                DateTime::parse_from_rfc3339(s.trim())
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            }
        });

    let body = {
        let mut stats = flight.stats.lock().expect("flight stats");

        // Apply block-time overrides BEFORE building the body so the
        // notes block + custom fields pick them up.
        if let Some(off) = block_off_override {
            stats.block_off_at = Some(off);
        }
        if let Some(on) = block_on_override {
            stats.block_on_at = Some(on);
        }
        if let Some(kg) = block_fuel_kg.filter(|v| *v > 0.0) {
            stats.block_fuel_kg = Some(kg);
        }

        // Same flight-time / block-fuel / level / landing-rate / score
        // mapping as the regular file path, with manual-overrides
        // taking precedence. phpVMS skips missing values cleanly
        // thanks to `skip_serializing_if`.
        let flight_time = flight_time_minutes
            .filter(|m| *m >= 0)
            .or_else(|| match (stats.takeoff_at, stats.landing_at) {
                (Some(t), Some(l)) if l > t => Some((l - t).num_minutes() as i32),
                _ => Some(
                    ((Utc::now() - flight.started_at).num_minutes() as i32).max(0),
                ),
            });
        let fares = if flight.fares.is_empty() {
            None
        } else {
            Some(
                flight
                    .fares
                    .iter()
                    .map(|(id, count)| FareEntry {
                        id: *id,
                        count: *count,
                    })
                    .collect(),
            )
        };
        // Block→remaining diff in kg, rounded to whole kg before lb
        // conversion (see flight_end for the cleanup rationale).
        // Manual override (kg) wins over the FSM-derived diff.
        let fuel_used = fuel_used_kg
            .filter(|v| *v > 0.0)
            .map(|kg| (kg as f64).round() * KG_TO_LB)
            .or_else(|| match (stats.block_fuel_kg, stats.last_fuel_kg) {
                (Some(b), Some(c)) if b > c => Some(((b - c) as f64).round() * KG_TO_LB),
                _ => None,
            });
        let block_fuel = stats
            .block_fuel_kg
            .filter(|kg| *kg > 0.0)
            .map(|kg| (kg as f64).round() * KG_TO_LB);
        let level = cruise_level_ft.filter(|ft| *ft > 0).or_else(|| {
            stats.peak_altitude_ft.map(|ft| {
                let rounded = ((ft / 100.0).round() * 100.0) as i32;
                rounded.max(0)
            })
        });
        let landing_rate = landing_rate_fpm
            .map(|v| v as f64)
            .or_else(|| stats.landing_rate_fpm.map(|v| v as f64));
        let score = stats.landing_score.map(|s| s.numeric());
        let resolved_distance = distance_nm
            .filter(|d| *d > 0.0)
            .unwrap_or(stats.distance_nm);
        let fields = build_pirep_fields(&flight, &stats);
        let mut notes = build_pirep_notes(&flight, &stats);
        notes.push_str("\n\n[MANUAL FILE — auto-validation bypassed by pilot.]");
        if let Some(divert) = divert_to
            .as_ref()
            .map(|s| s.trim().to_uppercase())
            .filter(|s| !s.is_empty())
        {
            notes.push_str(&format!(
                "\n\n[DIVERT] Planned arrival: {planned}. Actual landing: {actual}.",
                planned = flight.arr_airport,
                actual = divert,
            ));
        }
        if let Some(r) = reason
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            notes.push_str("\n\nReason: ");
            notes.push_str(r);
        }

        // List which fields the pilot manually overrode so the admin
        // can review at a glance. Helps avoid silently mis-attributing
        // FSM stats to manual entry.
        let mut overrides: Vec<&'static str> = Vec::new();
        if flight_time_minutes.is_some() {
            overrides.push("flight_time");
        }
        if block_fuel_kg.is_some() {
            overrides.push("block_fuel");
        }
        if fuel_used_kg.is_some() {
            overrides.push("fuel_used");
        }
        if distance_nm.is_some() {
            overrides.push("distance");
        }
        if cruise_level_ft.is_some() {
            overrides.push("cruise_level");
        }
        if landing_rate_fpm.is_some() {
            overrides.push("landing_rate");
        }
        if block_off_override.is_some() {
            overrides.push("block_off_time");
        }
        if block_on_override.is_some() {
            overrides.push("block_on_time");
        }
        if !overrides.is_empty() {
            notes.push_str(&format!(
                "\n\n[MANUAL OVERRIDES] {}",
                overrides.join(", ")
            ));
        }

        if let Some(extra) = notes_override
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            notes.push_str("\n\nPilot notes: ");
            notes.push_str(extra);
        }
        FileBody {
            flight_time,
            fuel_used,
            block_fuel,
            distance: Some(resolved_distance),
            level,
            landing_rate,
            score,
            source_name: Some(format!(
                "AeroACARS/{} (manual)",
                env!("CARGO_PKG_VERSION")
            )),
            notes: Some(notes),
            fares,
            fields: Some(fields),
            // The manual flow already had a `divert_to` parameter for
            // notes — pre-fix it only annotated the notes block and left
            // the admin to update arr_airport_id by hand. Now we override
            // the field directly too, same way the auto-divert flow does.
            arr_airport_id: divert_to
                .as_ref()
                .map(|s| s.trim().to_uppercase())
                .filter(|s| !s.is_empty()),
        }
    };
    // Flip the PIREP `source` to MANUAL (1) before submitting. PhpVMS's
    // PirepService::submit() decides ACCEPTED-vs-PENDING based on
    // (source, rank.auto_approve_acars, rank.auto_approve_manual). With
    // source=MANUAL and the VA having `auto_approve_manual=false` on the
    // pilot's rank, the PIREP lands in PENDING for admin review instead
    // of being auto-accepted as a normal ACARS submission.
    //
    // The Acars/UpdateRequest validation rules don't include `source`,
    // but the controller's parsePirep() forwards EVERY request input
    // to mass-assignment, and `source` is in the Pirep $fillable. So
    // the smuggle works as long as upstream phpVMS doesn't change it.
    // Verified against phpvms@dev on 2026-05-03.
    let source_marker = UpdateBody {
        source: Some(api_client::pirep_source::MANUAL),
        ..Default::default()
    };
    if let Err(e) = client.update_pirep(&flight.pirep_id, &source_marker).await {
        tracing::warn!(
            pirep_id = %flight.pirep_id,
            error = %e,
            "could not flip PIREP source to MANUAL — VA admin must filter on source_name"
        );
    } else {
        tracing::info!(pirep_id = %flight.pirep_id, "PIREP source flipped to MANUAL");
    }
    tracing::info!(pirep_id = %flight.pirep_id, "filing PIREP (manual)");
    match client.file_pirep(&flight.pirep_id, &body).await {
        Ok(()) => {
            // Same landing-history snapshot as the regular file path.
            {
                let stats = flight.stats.lock().expect("flight stats");
                record_landing_for_filed_flight(&app, &flight, &stats);
            }
            clear_persisted_flight(&app);
            log_activity(
                &state,
                ActivityLevel::Warn,
                {
                    let actual_arr = divert_to.as_deref().unwrap_or(&flight.arr_airport);
                    let suffix = if divert_to.is_some() {
                        format!(" (DIVERT, planned {})", flight.arr_airport)
                    } else {
                        String::new()
                    };
                    format!(
                        "Manual PIREP filed: {} {} → {}{}",
                        format_callsign(&flight.airline_icao, &flight.flight_number),
                        flight.dpt_airport,
                        actual_arr,
                        suffix
                    )
                },
                Some("Source tagged 'manual' — admin will review".into()),
            );
            record_event(
                &app,
                &flight.pirep_id,
                &FlightLogEvent::FlightEnded {
                    timestamp: Utc::now(),
                    pirep_id: flight.pirep_id.clone(),
                    outcome: FlightOutcome::Manual,
                },
            );
            consume_bid_best_effort(&client, flight.bid_id).await;
            Ok(())
        }
        Err(e) => {
            tracing::warn!(
                pirep_id = %flight.pirep_id,
                error = %e,
                "manual PIREP file failed — restoring flight to state for retry"
            );
            let mut guard = state.active_flight.lock().expect("active_flight lock");
            *guard = Some(flight);
            Err(e.into())
        }
    }
}

/// Cancel the active PIREP without filing it.
#[tauri::command]
async fn flight_cancel(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), UiError> {
    let flight = {
        let mut guard = state.active_flight.lock().expect("active_flight lock");
        guard
            .take()
            .ok_or_else(|| UiError::new("no_active_flight", "no flight is active"))?
    };
    flight.stop.store(true, Ordering::Relaxed);
    let client = current_client(&state)?;
    let result = client.cancel_pirep(&flight.pirep_id).await;
    // Clear local persistence regardless — the user wants this gone.
    clear_persisted_flight(&app);
    discard_queued_positions_for(&app, &flight.pirep_id);
    result?;
    log_activity(
        &state,
        ActivityLevel::Warn,
        format!(
            "Flight cancelled: {} {} → {}",
            format_callsign(&flight.airline_icao, &flight.flight_number),
            flight.dpt_airport,
            flight.arr_airport
        ),
        Some(format!("PIREP {}", flight.pirep_id)),
    );
    record_event(
        &app,
        &flight.pirep_id,
        &FlightLogEvent::FlightEnded {
            timestamp: Utc::now(),
            pirep_id: flight.pirep_id.clone(),
            outcome: FlightOutcome::Cancelled,
        },
    );
    Ok(())
}

/// Confirm an auto-resumed flight: now actually spawn the position streamer.
/// Called by the resume banner when its 10-second countdown elapses (or the
/// user clicks "Resume now"). Until this fires, no position posts go out, so
/// the user can still cancel without leaving footprints on phpVMS.
#[tauri::command]
async fn flight_resume_confirm(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), UiError> {
    let flight = {
        let guard = state.active_flight.lock().expect("active_flight lock");
        guard
            .as_ref()
            .cloned()
            .ok_or_else(|| UiError::new("no_active_flight", "no flight to resume"))?
    };
    // Resume confirmed → clear the banner flag.
    flight.was_just_resumed.store(false, Ordering::Relaxed);
    let client = current_client(&state)?;
    spawn_position_streamer(app.clone(), Arc::clone(&flight), client);
    spawn_touchdown_sampler(app, flight);
    Ok(())
}

/// Drop local active-flight state without contacting phpVMS. Useful when the
/// stored PIREP is orphaned/dead on the server side and the user wants a
/// clean slate.
#[tauri::command]
async fn flight_forget(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), UiError> {
    if let Some(flight) = state
        .active_flight
        .lock()
        .expect("active_flight lock")
        .take()
    {
        flight.stop.store(true, Ordering::Relaxed);
        tracing::info!(pirep_id = %flight.pirep_id, "active flight forgotten (no phpVMS call)");
        discard_queued_positions_for(&app, &flight.pirep_id);
        record_event(
            &app,
            &flight.pirep_id,
            &FlightLogEvent::FlightEnded {
                timestamp: Utc::now(),
                pirep_id: flight.pirep_id.clone(),
                outcome: FlightOutcome::Forgotten,
            },
        );
    }
    clear_persisted_flight(&app);
    Ok(())
}

/// v0.4.1: Pilot klickt „Flug wiederaufnehmen" nachdem der Sim wegbrach
/// und neu positioniert wurde. Hebt den Pause-State in den FlightStats auf
/// — der Streamer fängt im nächsten Tick wieder an Position-Posts an
/// phpVMS zu schicken, der Phase-FSM nimmt die Verarbeitung auf den
/// neuen Sim-Werten wieder auf.
///
/// Loggt eine Repositions-Audit-Zeile mit der Distanz zwischen letzter
/// bekannter Position (vor Disconnect) und der aktuellen Sim-Position
/// (nach Reposition). Bei großen Sprüngen (> REPOSITION_WARN_DELTA_NM)
/// als WARN-Level damit's bei VA-Audits sichtbar bleibt.
///
/// Bewusst KEINE 5-NM/2000-ft-Restriktion wie bei smartCARS — der Pilot
/// entscheidet wo er weitermacht, der Audit-Log macht's nachvollziehbar.
#[tauri::command]
async fn flight_resume_after_disconnect(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), UiError> {
    let flight = state
        .active_flight
        .lock()
        .expect("active_flight lock")
        .as_ref()
        .map(Arc::clone)
        .ok_or_else(|| UiError::new("no_active_flight", "no flight is active"))?;

    // Snapshot grab the pre-pause position before clearing state.
    let prev_known = {
        let stats = flight.stats.lock().expect("flight stats");
        if stats.paused_since.is_none() {
            return Err(UiError::new(
                "not_paused",
                "flight is not currently paused — nothing to resume",
            ));
        }
        stats.paused_last_known.clone()
    };

    // Read the current sim snapshot (best-effort) for the delta calculation.
    let current_snap = current_snapshot(&app);

    // Clear pause state. Streamer will resume normal posting from next tick.
    //
    // **Wichtig:** `last_lat/last_lon` wird auf None gesetzt — das ist der
    // Anker für die Distance-Akkumulation in `step_flight`. Ohne den Reset
    // würde der nächste Tick die volle Reposition-Strecke (z.B. 800 nm
    // wenn der Pilot zur Approach gesprungen ist) als geflogene Distanz
    // ins `distance_nm` reinschieben. Der Pilot hätte dann einen PIREP
    // mit künstlich aufgeblähter Distanz — das wäre auch dann „Cheating
    // im PIREP" wenn der Pilot es nicht beabsichtigt.
    //
    // Mit None startet der nächste Tick mit einer frischen Distance-
    // Baseline, der Reposition-Sprung fließt **nicht** in die geloggte
    // Flugdistanz ein. Die Reposition-Distanz selbst wird separat in
    // einer Activity-Log-Zeile festgehalten („▶ Flug wiederaufgenommen
    // — Repositioniert X nm") und ist damit für VA-Audits nachvoll-
    // ziehbar, ohne den PIREP-Distance-Counter zu verfälschen.
    {
        let mut stats = flight.stats.lock().expect("flight stats");
        stats.paused_since = None;
        stats.paused_last_known = None;
        stats.last_lat = None;
        stats.last_lon = None;
    }
    save_active_flight(&app, &flight);

    // Build the resume audit log entry.
    let delta_nm = match (&prev_known, &current_snap) {
        (Some(prev), Some(cur)) => {
            let d_m = ::geo::distance_m(prev.lat, prev.lon, cur.lat, cur.lon);
            Some(d_m / 1852.0)
        }
        _ => None,
    };

    let (level, msg) = match delta_nm {
        Some(d) if d >= REPOSITION_WARN_DELTA_NM => (
            ActivityLevel::Warn,
            format!(
                "▶ Flug wiederaufgenommen — Repositioniert {:.1} nm (auffällig groß)",
                d
            ),
        ),
        Some(d) => (
            ActivityLevel::Info,
            format!("▶ Flug wiederaufgenommen — Repositioniert {:.1} nm", d),
        ),
        None => (
            ActivityLevel::Info,
            "▶ Flug wiederaufgenommen".to_string(),
        ),
    };
    let detail = match (&prev_known, &current_snap) {
        (Some(prev), Some(cur)) => Some(format!(
            "Vorher: LAT {:.4}° · LON {:.4}° · ALT {:.0} ft\nJetzt:  LAT {:.4}° · LON {:.4}° · ALT {:.0} ft",
            prev.lat, prev.lon, prev.altitude_ft,
            cur.lat, cur.lon, cur.altitude_msl_ft,
        )),
        _ => None,
    };
    log_activity(&state, level, msg, detail);

    tracing::info!(
        pirep_id = %flight.pirep_id,
        delta_nm = ?delta_nm,
        "flight resumed after disconnect"
    );
    Ok(())
}

/// Spawn the background task that pushes the latest sim snapshot to phpVMS at
/// `POSITION_INTERVAL_SECS`. Stops when `flight.stop` is set or the active
/// flight is replaced.
///
/// CAS-guarded: at most one streamer task per ActiveFlight. Repeat calls are
/// no-ops, which makes it safe to invoke from multiple UI paths
/// (`flight_resume_confirm` racing with itself, StrictMode double-mount, etc.).
/// Spawn a high-rate (~30 Hz) sampler dedicated to populating the
/// touchdown ring buffer in `FlightStats`. Runs alongside the
/// position streamer (which posts to phpVMS at 1–30 s cadence,
/// way too slow for sub-second touchdown capture).
///
/// The actual telemetry rate is bounded by SimConnect's
/// `SIMCONNECT_PERIOD_VISUAL_FRAME` updates (~30 Hz typical) and
/// the adapter worker's 50 ms drain sleep — so in practice the
/// buffer accumulates ~20 samples/second, which still gives 100
/// entries in the 5-second look-back window. That is more than
/// enough to capture the actual touchdown subframe instead of
/// the bounce rebound that the single 1 Hz sample was catching.
///
/// Exits when `flight.stop` is set, just like the streamer.
/// v0.5.23 Forensik-Upload. Spawnt einen async-Task der das per-Flug
/// JSONL-Logfile `<app_data>/flight_logs/<pirep_id>.jsonl` gzippt und an
/// aeroacars-live POSTet. Endpoint: `/api/flight-logs/upload`. Auth via
/// HTTP Basic gegen die provisioned_pilots-Tabelle (gleiche Cred-Pair wie
/// Mosquitto-MQTT-Login).
///
/// Failure-Modi (alle non-fatal):
///   * Log-Datei nicht vorhanden — z.B. Recording war disabled oder
///     PIREP wurde manuell gefilet ohne dass das Recorder-Modul den
///     Flug initialisiert hat. → tracing::debug, kein Retry.
///   * Keyring-Read fehlgeschlagen — auch non-fatal. Pilot kann beim
///     naechsten App-Start neu provisionieren.
///   * Server gibt non-2xx zurueck (401 / 403 / 5xx) — gelogged, keine
///     Retry-Queue heute (kann spaeter mit Pending-Folder kommen).
fn spawn_flight_log_upload(app: &AppHandle, pirep_id: String) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        // 1. Pfad zur JSONL-Datei zusammensetzen — gleiche Logik wie der
        //    Recorder selbst (sanitize_pirep_id).
        let app_data_dir = match app.path().app_data_dir() {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(error = %e, "log-upload: no app_data_dir");
                return;
            }
        };
        let safe_pirep = pirep_id
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
            .collect::<String>();
        let log_path = app_data_dir
            .join("flight_logs")
            .join(format!("{safe_pirep}.jsonl"));
        if !log_path.exists() {
            tracing::debug!(path = ?log_path, "log-upload: file missing — skipping");
            return;
        }

        // 2. MQTT-Credentials aus Keyring (= identisch zu HTTP-Basic).
        let username = match secrets::load_api_key(MQTT_KEYRING_USERNAME) {
            Ok(Some(u)) => u,
            _ => {
                tracing::warn!("log-upload: no MQTT username in keyring — skip");
                return;
            }
        };
        let password = match secrets::load_api_key(MQTT_KEYRING_PASSWORD) {
            Ok(Some(p)) => p,
            _ => {
                tracing::warn!("log-upload: no MQTT password in keyring — skip");
                return;
            }
        };

        // 3. Hochladen.
        match aeroacars_mqtt::log_upload::upload_flight_log(
            &log_path,
            &pirep_id,
            &username,
            &password,
            None, // default endpoint = https://live.kant.ovh/api/flight-logs/upload
        ).await {
            Ok(stats) => {
                tracing::info!(
                    pirep_id = %pirep_id,
                    raw_kb = stats.raw_size / 1024,
                    gzip_kb = stats.compressed_size / 1024,
                    "flight log uploaded",
                );
                // v0.5.23: Pilot-UI-Feedback im Activity-Log damit der
                // Pilot sieht dass der Upload geklappt hat. Detail-Spalte
                // zeigt die Groessen-Statistik fuer Debugging.
                log_activity_handle(
                    &app,
                    ActivityLevel::Info,
                    "Flight log uploaded to live-tracking server",
                    Some(format!(
                        "{} KB raw → {} KB gzip ({}% Kompression)",
                        stats.raw_size / 1024,
                        stats.compressed_size / 1024,
                        ((stats.compressed_size as f64 / stats.raw_size as f64) * 100.0) as i32,
                    )),
                );
            }
            Err(e) => {
                tracing::warn!(
                    pirep_id = %pirep_id,
                    error = %e,
                    "flight log upload failed (non-fatal)",
                );
                // Fehler ist non-fatal — JSONL bleibt lokal verfuegbar.
                // Wir loggen mit ActivityLevel::Warn damit der Pilot
                // weiss dass Forensik-Upload nicht klappte (= bei
                // Bug-Reports den Pfad zur lokalen Datei nennen).
                log_activity_handle(
                    &app,
                    ActivityLevel::Warn,
                    "Flight log upload failed (non-fatal)",
                    Some(format!("{} — Log liegt lokal in flight_logs/{}.jsonl", e, pirep_id)),
                );
            }
        }
    });
}

fn spawn_touchdown_sampler(app: AppHandle, flight: Arc<ActiveFlight>) {
    tauri::async_runtime::spawn(async move {
        tracing::info!(pirep_id = %flight.pirep_id, "touchdown sampler started");
        // v0.4.4: lokale Edge-Tracking-State.
        //
        // X-Plane: bevorzugt `gear_normal_force_n` (Newton, spikt
        // präzise im Frame des physischen Touchdowns) — das macht
        // auch xgs (etabliertes X-Plane-Landing-Speed-Plugin seit ~10
        // Jahren). xgs nutzt exakt `force != 0.0` als Trigger; wir
        // wählen `> 1.0 N` als minimaler Filter gegen Float-Noise und
        // catchen damit immer noch den ersten Kontakt-Frame (echte
        // Touchdowns gehen blitzartig auf mehrere kN — ein 60-300t
        // Airliner mit 1.0g Bremsmoment = mindestens 588-2940 kN auf
        // dem Gear). 1.0 N = ~100g Auflagedruck, also unter jeder
        // physikalisch plausiblen Touchdown-Schwelle.
        //
        // MSFS: gear_normal_force_n ist None → fallback auf
        // `on_ground`-Edge wie vorher (MSFS hat eh den separaten
        // `PLANE TOUCHDOWN NORMAL VELOCITY`-SimVar als Primary).
        let mut prev_in_air: Option<bool> = None;
        const GEAR_TOUCHDOWN_THRESHOLD_N: f32 = 1.0;
        loop {
            // 20 ms = 50 Hz target — matches GEES (`SAMPLE_RATE = 20`),
            // the only open-source reference impl that publishes its
            // sample cadence. The actual upper bound comes from how
            // fast `current_snapshot` returns fresh data; on a
            // typical PC the SimConnect adapter ticks at the rendered
            // frame rate (60-120 fps) so we never under-sample here.
            tokio::time::sleep(Duration::from_millis(20)).await;
            if flight.stop.load(Ordering::Relaxed) {
                break;
            }
            let Some(snap) = current_snapshot(&app) else {
                continue;
            };
            let now = Utc::now();
            let mut stats = flight.stats.lock().expect("flight stats");
            stats.snapshot_buffer.push_back(TelemetrySample {
                at: now,
                vs_fpm: snap.vertical_speed_fpm,
                g_force: snap.g_force,
                on_ground: snap.on_ground,
                agl_ft: snap.altitude_agl_ft as f32,
                heading_true_deg: snap.heading_deg_true,
                groundspeed_kt: snap.groundspeed_kt,
                indicated_airspeed_kt: snap.indicated_airspeed_kt,
                lat: snap.lat,
                lon: snap.lon,
                pitch_deg: snap.pitch_deg,
                bank_deg: snap.bank_deg,
            });

            // v0.5.11: running peak-descent VS in the LOW-ALTITUDE zone
            // only (AGL ≤ 250 ft). Earlier versions (v0.5.5+) tracked
            // this across the whole Approach + Final segment, which
            // produced phantom hard-landing reports when a steep
            // pre-flare descent (e.g. -1346 fpm @ 943 ft AGL) won
            // over the actual gentle touchdown.
            //
            // Now: ONLY samples from the low-altitude footprint count.
            // This is a fallback only — the AGL-derivative estimator
            // (estimate_xplane_touchdown_vs_from_agl) is the primary
            // source. low_agl_vs_min_fpm is used when the estimator
            // can't find a valid window (extremely sparse RREF, etc.).
            let approach_or_final = matches!(
                stats.phase,
                FlightPhase::Approach | FlightPhase::Final
            );
            if approach_or_final
                && !snap.on_ground
                && snap.altitude_agl_ft <= 250.0
            {
                let pitch_rad = (snap.pitch_deg as f32) * std::f32::consts::PI / 180.0;
                let vs_corrected = snap.vertical_speed_fpm * pitch_rad.cos();
                let curr_min = stats.low_agl_vs_min_fpm.unwrap_or(f32::INFINITY);
                if vs_corrected < curr_min {
                    stats.low_agl_vs_min_fpm = Some(vs_corrected);
                }
            }

            // v0.4.4: Edge-Detection direkt im Sampler.
            //
            // Primary signal: `gear_normal_force_n` (X-Plane-only) —
            // spikt im Frame des physischen Touchdowns (xgs-Methode).
            // Fallback: `on_ground`-Flag (für MSFS und falls X-Plane-
            // DataRef mal None liefert).
            //
            // Wir halten `prev_in_air` als rolling-state. Der Edge
            // ist `true → false` (vom in-air zu am-Boden).
            //
            // Capture nur ONCE: sampler_touchdown_at bleibt Some bis
            // Flight-Reset → kein Re-Trigger bei Bouncing nach dem
            // initialen Touchdown.
            let in_air_now = match snap.gear_normal_force_n {
                Some(force) => force < GEAR_TOUCHDOWN_THRESHOLD_N,
                None => !snap.on_ground,
            };
            let edge_detected = matches!(prev_in_air, Some(true)) && !in_air_now;

            // v0.5.0: Premium-Plugin-Override.
            //
            // Wenn der AeroACARS X-Plane Plugin (optional, v0.5.0+) ein
            // Touchdown-Event gesendet hat, übernimmt es den Capture
            // unabhängig von der RREF-Edge — der Plugin sieht das Edge
            // mit Frame-Genauigkeit und kennt das tatsächliche peak
            // descent VS aus seinem eigenen 500ms-Lookback-Buffer
            // (gleicher Algorithmus wie unten, aber INNERHALB von
            // X-Plane gemessen, also keine UDP-Verzögerung / Eviction-
            // Race). Wir brauchen lokal den `prev_in_air`-Reset
            // trotzdem, damit das `bouncing`-Re-Trigger-Guard greift
            // wenn das Aircraft kurz wieder abhebt.
            //
            // `stats.sampler_touchdown_at.is_none()`-Guard: identisch
            // zur RREF-Edge — nur ein Capture pro Landing.
            if let Some(td) = current_premium_touchdown(&app) {
                if stats.sampler_touchdown_at.is_none() {
                    stats.sampler_touchdown_at = Some(now);
                    stats.sampler_touchdown_vs_fpm = Some(td.captured_vs_fpm);
                    stats.sampler_touchdown_g_force = Some(td.captured_g_normal);
                    tracing::info!(
                        pirep_id = %flight.pirep_id,
                        captured_vs_fpm = td.captured_vs_fpm,
                        captured_g = td.captured_g_normal,
                        captured_pitch_deg = td.captured_pitch_deg,
                        captured_ias_kt = td.captured_ias_kt,
                        source = "x-plane-plugin-premium",
                        "premium touchdown event captured (frame-perfect)"
                    );
                    prev_in_air = Some(in_air_now);
                    let cutoff = now - chrono::Duration::seconds(TOUCHDOWN_BUFFER_SECS);
                    while stats
                        .snapshot_buffer
                        .front()
                        .is_some_and(|s| s.at < cutoff)
                    {
                        stats.snapshot_buffer.pop_front();
                    }
                    continue;
                }
            }

            // v0.5.24: Takeoff-Edge (Wheels-Up) — opposite of touchdown.
            // Wenn `prev_in_air=false` und jetzt `in_air_now=true`, ist
            // der Flieger gerade abgehoben. Wir capturen Pitch + Bank
            // exakt im Frame (50Hz, <20ms-Genauigkeit). Guard
            // `sampler_takeoff_at.is_none()` verhindert Re-Trigger bei
            // Touch-and-Go im Pattern (= zweiter Wheels-Up nach erstem
            // Touchdown ueberschreibt nicht den initialen Takeoff-Wert).
            let takeoff_edge_detected =
                matches!(prev_in_air, Some(false)) && in_air_now;
            if takeoff_edge_detected && stats.sampler_takeoff_at.is_none() {
                stats.sampler_takeoff_at = Some(now);
                stats.sampler_takeoff_pitch_deg = Some(snap.pitch_deg);
                stats.sampler_takeoff_bank_deg = Some(snap.bank_deg);
                tracing::info!(
                    pirep_id = %flight.pirep_id,
                    captured_pitch_deg = snap.pitch_deg,
                    captured_bank_deg = snap.bank_deg,
                    on_ground = snap.on_ground,
                    fnrml_n = snap.gear_normal_force_n,
                    "sampler-side takeoff edge detected (wheels-up frame)"
                );
            }

            if edge_detected && stats.sampler_touchdown_at.is_none() {
                // Pitch-Korrektur: world-frame Y-Velocity → body-axial
                // (xgs-Pattern). Bei typischem Touchdown-Pitch ~3-5°
                // ist cos(pitch)≈0.998, also <0.5% Unterschied. Bei
                // steilen Flares (STOL-Aircraft, ~10°) bis 1.5%.
                // Konsistent mit xgs's `vy * cos(theta * deg2rad)`.
                let pitch_rad = (snap.pitch_deg as f32) * std::f32::consts::PI / 180.0;
                let pitch_cos = pitch_rad.cos();
                let current_vs = snap.vertical_speed_fpm * pitch_cos;
                // VS-min der letzten 2 s — fängt den echten Sinkflug
                // ein bevor Rollout-Damping zugeschlagen hat. v0.5.5
                // erweitert das Fenster von 500 ms auf 2 s nachdem ein
                // Pilot-Test gezeigt hat dass 500 ms bei aggressivem
                // Flare nur Post-Touchdown-Rebound-Samples sieht (alle
                // VS-Werte positiv). 2 s deckt sicher die letzte
                // Flare-Phase ab; falls X-Plane uns RREF mit niedriger
                // Rate liefert haben wir trotzdem genug Samples drin.
                let lookback_start = now - chrono::Duration::milliseconds(2000);
                let recent_vs_min: f32 = stats
                    .snapshot_buffer
                    .iter()
                    .filter(|s| s.at >= lookback_start)
                    .map(|s| {
                        // Pitch-Korrektur auch auf Buffer-Samples — die
                        // sind je nach Aircraft beim Flare auch unter
                        // Nase-hoch-Bedingungen erfasst worden.
                        let p_rad = (s.pitch_deg as f32) * std::f32::consts::PI / 180.0;
                        s.vs_fpm * p_rad.cos()
                    })
                    .fold(f32::INFINITY, f32::min);
                let captured_vs = if recent_vs_min.is_finite() {
                    recent_vs_min.min(current_vs)
                } else {
                    current_vs
                };
                // Peak G im gleichen Window
                let recent_g_peak: f32 = stats
                    .snapshot_buffer
                    .iter()
                    .filter(|s| s.at >= lookback_start)
                    .map(|s| s.g_force)
                    .fold(0.0, f32::max);
                let captured_g = recent_g_peak.max(snap.g_force);

                stats.sampler_touchdown_at = Some(now);
                stats.sampler_touchdown_vs_fpm = Some(captured_vs);
                stats.sampler_touchdown_g_force = Some(captured_g);
                tracing::info!(
                    pirep_id = %flight.pirep_id,
                    captured_vs_fpm = captured_vs,
                    current_vs_fpm = current_vs,
                    captured_g = captured_g,
                    fnrml_n = snap.gear_normal_force_n,
                    on_ground = snap.on_ground,
                    "sampler-side touchdown edge detected"
                );
            }
            prev_in_air = Some(in_air_now);

            let cutoff = now - chrono::Duration::seconds(TOUCHDOWN_BUFFER_SECS);
            while stats
                .snapshot_buffer
                .front()
                .is_some_and(|s| s.at < cutoff)
            {
                stats.snapshot_buffer.pop_front();
            }
        }
        tracing::info!(pirep_id = %flight.pirep_id, "touchdown sampler stopped");
    });
}

fn spawn_position_streamer(app: AppHandle, flight: Arc<ActiveFlight>, client: Client) {
    if flight
        .streamer_spawned
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        tracing::debug!(
            pirep_id = %flight.pirep_id,
            "position streamer already running, skipping spawn"
        );
        return;
    }
    tauri::async_runtime::spawn(async move {
        tracing::info!(pirep_id = %flight.pirep_id, "position streamer started");
        // Heartbeat tracker: ensures `POST /pireps/{id}/update` fires at
        // least every `HEARTBEAT_INTERVAL` so phpVMS's RemoveExpiredLiveFlights
        // cron never reaches the inactivity threshold. Initialised one
        // interval in the past so the first eligible tick triggers it.
        let mut last_heartbeat: std::time::Instant =
            std::time::Instant::now() - HEARTBEAT_INTERVAL;
        // Phase-adaptive cadence: re-read the current phase on every tick so
        // the sleep matches what the aircraft is actually doing right now
        // (5 s during takeoff/landing, 8 s on approach/final, 10 s on the
        // ground / climb / descent, 30 s in cruise — capped at 30 s so the
        // live map never goes more than half a minute stale).
        // v0.4.1: track last good snapshot so we can detect Sim-Disconnect
        // (kein Snapshot mehr für > SIM_DISCONNECT_THRESHOLD_S s) und den
        // letzten bekannten Stand für Repositions-Anzeige im Cockpit-Banner
        // einfrieren.
        let mut last_good_snap: Option<SimSnapshot> = None;
        let mut last_good_snap_at: Option<std::time::Instant> = None;
        // v0.5.21: track last phpVMS POST so we can keep its cadence
        // phase-aware (4-30 s) while running the loop body itself —
        // and the MQTT publish — at 3 s.
        let mut last_phpvms_post_at: Option<std::time::Instant> = None;
        loop {
            let current_phase = {
                let stats = flight.stats.lock().expect("flight stats");
                stats.phase
            };
            // v0.5.21: loop runs at MQTT cadence (constant 3 s).
            // phpVMS POST throttled separately further down.
            tokio::time::sleep(Duration::from_secs(MQTT_PUBLISH_INTERVAL_SECS)).await;
            if flight.stop.load(Ordering::Relaxed) {
                break;
            }

            let snapshot = current_snapshot(&app);
            if let Some(ref s) = snapshot {
                last_good_snap = Some(s.clone());
                last_good_snap_at = Some(std::time::Instant::now());
            }

            // v0.4.1: paused state check — if we're paused, skip everything
            // (kein Position-Post, kein Phase-FSM-Step, kein Activity-Log).
            // Heartbeat unten läuft trotzdem damit phpVMS' Cron den PIREP
            // nicht killt während der Pilot den Sim neu startet. **Kein**
            // Auto-Resume — selbst wenn der Sim wieder Daten liefert, der
            // Streamer wartet auf den Resume-Klick (Tauri-Command
            // `flight_resume_after_disconnect`).
            let is_paused = flight
                .stats
                .lock()
                .expect("flight stats")
                .paused_since
                .is_some();
            if is_paused {
                // Heartbeat trotzdem feuern (siehe weiter unten — wir lassen
                // den Heartbeat-Code-Pfad ungestört durchlaufen, müsste hier
                // also eigentlich `continue` sein. Der Heartbeat fired
                // unten am Schleifen-Ende, deshalb erstmal next-iteration.
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }

            // Sim-Disconnect-Detection: hatten irgendwann mal Daten, aber
            // der letzte gute Snapshot ist > THRESHOLD her → Pause auslösen.
            let disconnect_detected = match (last_good_snap_at, snapshot.is_some()) {
                (Some(t), false) => {
                    t.elapsed() > Duration::from_secs(SIM_DISCONNECT_THRESHOLD_S as u64)
                }
                _ => false,
            };
            if disconnect_detected {
                if let Some(ref last) = last_good_snap {
                    let paused = PausedSnapshot {
                        lat: last.lat,
                        lon: last.lon,
                        heading_deg: last.heading_deg_true,
                        altitude_ft: last.altitude_msl_ft,
                        fuel_total_kg: last.fuel_total_kg,
                        zfw_kg: last.zfw_kg,
                    };
                    {
                        let mut stats = flight.stats.lock().expect("flight stats");
                        if stats.paused_since.is_none() {
                            stats.paused_since = Some(Utc::now());
                            stats.paused_last_known = Some(paused.clone());
                        }
                    }
                    let detail = format!(
                        "Letzte bekannte Position: LAT {:.4}° · LON {:.4}° · HDG {:.0}° · ALT {:.0} ft · Fuel {:.0} kg · ZFW {}",
                        paused.lat,
                        paused.lon,
                        paused.heading_deg,
                        paused.altitude_ft,
                        paused.fuel_total_kg,
                        paused
                            .zfw_kg
                            .map(|v| format!("{:.0} kg", v))
                            .unwrap_or_else(|| "—".to_string())
                    );
                    log_activity_handle(
                        &app,
                        ActivityLevel::Warn,
                        "⏸ Sim getrennt — Flug pausiert. Klicke „Flug wiederaufnehmen\" sobald du repositioniert hast.".to_string(),
                        Some(detail),
                    );
                    save_active_flight(&app, &flight);
                    tracing::warn!(
                        pirep_id = %flight.pirep_id,
                        "sim disconnect detected, flight paused"
                    );
                }
                continue;
            }

            // Wenn weder pausiert noch frischer Snapshot da, einfach weiter
            // warten — bis zum Threshold poll'n wir geduldig.
            let Some(snap) = snapshot else {
                tracing::debug!(
                    pirep_id = %flight.pirep_id,
                    "no sim snapshot yet — waiting (will pause after {}s)",
                    SIM_DISCONNECT_THRESHOLD_S
                );
                continue;
            };

            // Snapshot the current phase BEFORE stepping so we can pass
            // the from→to pair to the recorder when it changes.
            // Plus snapshot Takeoff/Landing-at flags so we can fire
            // Discord-Webhook-Posts beim None→Some-Übergang nach
            // step_flight (v0.4.0).
            let (prev_phase, prev_block_off_at, prev_takeoff_at, prev_landing_at) = {
                let stats = flight.stats.lock().expect("flight stats");
                (
                    stats.phase,
                    stats.block_off_at,
                    stats.takeoff_at,
                    stats.landing_at,
                )
            };
            // Update running stats AND step the flight-phase FSM.
            // v0.5.25: arr_airport_elevation_ft fuer HAT-basiertes
            // Stable-Approach-Gate. Lazy lookup beim ersten Tick wo wir
            // die Elevation noch nicht haben — phpVMS-API-Cache aus
            // state.airports liefert sie sobald airport_get aufgerufen
            // wurde (passiert beim Bid-Pickup ueblicherweise schon).
            // Ohne Elevation: Fallback auf AGL-Filter, used_hat=false.
            {
                let mut stats_g = flight.stats.lock().expect("flight stats");
                if stats_g.arr_airport_elevation_ft.is_none() {
                    let app_state = app.state::<AppState>();
                    let airports = app_state.airports.lock().expect("airports lock");
                    if let Some(arr) = airports.get(&flight.arr_airport.to_uppercase()) {
                        if let Some(elev) = arr.elevation {
                            stats_g.arr_airport_elevation_ft = Some(elev as f32);
                            tracing::info!(
                                arr = %flight.arr_airport,
                                elevation_ft = elev,
                                "arr airport elevation cached for HAT-based Stable-Approach-Gate"
                            );
                        }
                    }
                }
            }
            let phase_change = step_flight(&flight, &snap);
            // v0.4.0: Discord-Webhook-Posts für Takeoff + Landing.
            // Wir feuern fire-and-forget (tokio::spawn) damit der
            // Streamer-Tick nie auf Discord wartet — ein langsamer
            // Webhook würde sonst die Position-Post-Frequenz killen.
            // Filter: Übergang None→Some am Stats-Feld, nicht Phase
            // — verhindert Doppel-Posts wenn der FSM zwischen Phasen
            // hin- und herflattert.
            {
                let cached_pilot = app
                    .state::<AppState>()
                    .cached_pilot
                    .lock()
                    .expect("cached_pilot lock")
                    .clone();
                let (pilot_ident, pilot_name) = match cached_pilot {
                    Some((id, name)) => (Some(id), Some(name)),
                    None => (None, None),
                };
                let stats = flight.stats.lock().expect("flight stats");
                if prev_takeoff_at.is_none() && stats.takeoff_at.is_some() {
                    let ctx = discord::EventContext {
                        callsign: format_callsign(&flight.airline_icao, &flight.flight_number),
                        airline_icao: flight.airline_icao.clone(),
                        airline_logo_url: flight.airline_logo_url.clone(),
                        dpt_icao: flight.dpt_airport.clone(),
                        arr_icao: flight.arr_airport.clone(),
                        aircraft_type: Some(flight.aircraft_icao.clone()).filter(|s| !s.is_empty()),
                        aircraft_reg: Some(flight.planned_registration.clone()).filter(|s| !s.is_empty()),
                        pilot_ident: pilot_ident.clone(),
                        pilot_name: pilot_name.clone(),
                        block_fuel_kg: stats.block_fuel_kg,
                        planned_block_fuel_kg: stats.planned_block_fuel_kg,
                        tow_kg: stats.takeoff_weight_kg.map(|w| w as f32),
                        ..Default::default()
                    };
                    tokio::spawn(discord::post_event(discord::EventKind::Takeoff, ctx));
                }
                if prev_landing_at.is_none() && stats.landing_at.is_some() {
                    let ctx = discord::EventContext {
                        callsign: format_callsign(&flight.airline_icao, &flight.flight_number),
                        airline_icao: flight.airline_icao.clone(),
                        airline_logo_url: flight.airline_logo_url.clone(),
                        dpt_icao: flight.dpt_airport.clone(),
                        arr_icao: flight.arr_airport.clone(),
                        aircraft_type: Some(flight.aircraft_icao.clone()).filter(|s| !s.is_empty()),
                        aircraft_reg: Some(flight.planned_registration.clone()).filter(|s| !s.is_empty()),
                        pilot_ident: pilot_ident.clone(),
                        pilot_name: pilot_name.clone(),
                        landing_rate_fpm: stats.landing_rate_fpm,
                        score: stats.landing_score.map(|s| s.numeric()),
                        distance_nm: Some(stats.distance_nm),
                        ..Default::default()
                    };
                    tokio::spawn(discord::post_event(discord::EventKind::Landing, ctx));
                }
            }
            // v0.5.14: MQTT block + takeoff snapshots. Fire on the same
            // None→Some transitions used by the Discord webhook block
            // above. Block fires when block_off_at is stamped (=
            // pushback / first taxi-out motion); takeoff fires when
            // takeoff_at is stamped (= aircraft has lifted off).
            // Both use retain=true so a Monitor that joins mid-flight
            // sees the snapshot.
            {
                let block_payload_opt: Option<aeroacars_mqtt::BlockPayload> = {
                    let stats = flight.stats.lock().expect("flight stats");
                    if prev_block_off_at.is_none() && stats.block_off_at.is_some() {
                        Some(aeroacars_mqtt::BlockPayload {
                            ts: stats
                                .block_off_at
                                .map(|t| t.timestamp_millis())
                                .unwrap_or(0),
                            block_fuel_kg: stats.block_fuel_kg,
                            planned_block_fuel_kg: stats.planned_block_fuel_kg,
                            planned_burn_kg: stats.planned_burn_kg,
                            planned_reserve_kg: stats.planned_reserve_kg,
                            planned_zfw_kg: stats.planned_zfw_kg,
                            planned_tow_kg: stats.planned_tow_kg,
                            planned_ldw_kg: stats.planned_ldw_kg,
                            planned_max_zfw_kg: stats.planned_max_zfw_kg,
                            planned_max_tow_kg: stats.planned_max_tow_kg,
                            planned_max_ldw_kg: stats.planned_max_ldw_kg,
                            planned_route: stats.planned_route.clone(),
                            planned_alternate: stats.planned_alternate.clone(),
                            dep_gate: stats.dep_gate.clone(),
                            dep_metar: stats.dep_metar_raw.clone(),
                        })
                    } else {
                        None
                    }
                };
                let takeoff_payload_opt: Option<aeroacars_mqtt::TakeoffPayload> = {
                    let stats = flight.stats.lock().expect("flight stats");
                    if prev_takeoff_at.is_none() && stats.takeoff_at.is_some() {
                        Some(aeroacars_mqtt::TakeoffPayload {
                            ts: stats
                                .takeoff_at
                                .map(|t| t.timestamp_millis())
                                .unwrap_or(0),
                            takeoff_weight_kg: stats.takeoff_weight_kg.map(|w| w as f32),
                            takeoff_fuel_kg: stats.takeoff_fuel_kg,
                            takeoff_lat: Some(snap.lat),
                            takeoff_lon: Some(snap.lon),
                            dep_metar: stats.dep_metar_raw.clone(),
                            dep_runway: snap.selected_runway.clone(),
                        })
                    } else {
                        None
                    }
                };
                if block_payload_opt.is_some() || takeoff_payload_opt.is_some() {
                    let app_state = app.state::<AppState>();
                    let mqtt = app_state.mqtt.lock().await;
                    if let Some(handle) = mqtt.as_ref() {
                        if let Some(p) = block_payload_opt {
                            handle.block(p);
                        }
                        if let Some(p) = takeoff_payload_opt {
                            handle.takeoff(p);
                        }
                    }
                }
            }
            // Detect when the touchdown-analyzer window has just locked
            // in a final score so we can emit the activity-log entry
            // exactly once. `landing_score` flips from None to Some
            // inside `step_flight` after TOUCHDOWN_WINDOW_SECS.
            // Returns Some(message) the first time a score is announced
            // so the streamer can mirror it into `acars/logs`.
            let landing_log_message = announce_landing_score(&app, &flight);
            // v0.5.11: MQTT touchdown event. Fires once per landing
            // when the score message gets generated (= touchdown was
            // captured + scored). Snapshot relevant fields from
            // FlightStats inside a short-lived block so the
            // std::sync::MutexGuard (not Send) doesn't span the
            // tokio Mutex `.await` below.
            if landing_log_message.is_some() {
                let payload_opt: Option<aeroacars_mqtt::TouchdownPayload> = {
                    let stats = flight.stats.lock().expect("flight stats");
                    stats.landing_at.map(|landing_at| {
                        let rwy_match = stats.runway_match.as_ref();
                        aeroacars_mqtt::TouchdownPayload {
                            ts: landing_at.timestamp_millis(),
                            vs_fpm: stats
                                .landing_rate_fpm
                                .or(stats.landing_peak_vs_fpm)
                                .map(|v| v.round() as i32)
                                .unwrap_or(0),
                            ias_kt: stats
                                .landing_speed_kt
                                .map(|v| v.round() as i32)
                                .unwrap_or(0),
                            // v0.5.17: GS captured separately from IAS,
                            // falls aware of head/tailwind.
                            gs_kt: stats
                                .landing_groundspeed_kt
                                .map(|v| v.round() as i32),
                            pitch_deg: stats.landing_pitch_deg,
                            // v0.5.17: bank at touchdown (= "Landing
                            // Roll" in maintenance-plugin lingo,
                            // captured in v0.5.16 alongside pitch).
                            bank_deg: stats.landing_bank_deg,
                            g_load: stats.landing_g_force,
                            peak_g_load: stats.landing_peak_g_force,
                            sideslip_deg: stats.touchdown_sideslip_deg,
                            headwind_kt: stats.landing_headwind_kt,
                            crosswind_kt: stats.landing_crosswind_kt,
                            score: stats.landing_score.map(|s| s.numeric()),
                            bounce: Some(stats.bounce_count > 0),
                            bounce_count: Some(stats.bounce_count),
                            runway: stats.approach_runway.clone(),
                            airport: Some(flight.arr_airport.clone()),
                            lat: stats.landing_lat,
                            lon: stats.landing_lon,
                            heading_true_deg: stats.landing_heading_true_deg,
                            heading_mag_deg: stats.landing_heading_deg,
                            landing_weight_kg: stats.landing_weight_kg.map(|w| w as f32),
                            landing_fuel_kg: stats.landing_fuel_kg,
                            rollout_distance_m: stats.rollout_distance_m.map(|d| d as f32),
                            approach_vs_stddev_fpm: stats.approach_vs_stddev_fpm,
                            approach_bank_stddev_deg: stats.approach_bank_stddev_deg,
                            go_around_count: Some(stats.go_around_count),
                            arr_metar: stats.arr_metar_raw.clone(),
                            runway_match_icao: rwy_match.map(|m| m.airport_ident.clone()),
                            runway_match_ident: rwy_match.map(|m| m.runway_ident.clone()),
                            runway_match_distance_m: rwy_match
                                .map(|m| (m.touchdown_distance_from_threshold_ft as f32) * 0.3048),
                            runway_match_centerline_offset_m: rwy_match
                                .map(|m| m.centerline_distance_m as f32),
                            // v0.5.23: Touchdown-Forensik — alle Schaetzer-
                            // Zwischenergebnisse fuer Server-seitige
                            // Algorithmus-Vergleiche (siehe stats Felder
                            // landing_vs_estimate_*_fpm + landing_vs_source).
                            simulator: stats.landing_simulator
                                .map(|s| s.to_string()),
                            vs_estimate_xp_fpm: stats.landing_vs_estimate_xp_fpm,
                            vs_estimate_msfs_fpm: stats.landing_vs_estimate_msfs_fpm,
                            vs_source: stats.landing_vs_source
                                .map(|s| s.to_string()),
                            gear_force_peak_n: stats.landing_gear_force_peak_n,
                            estimate_window_ms: stats.landing_estimate_window_ms,
                            estimate_sample_count: stats.landing_estimate_sample_count,
                            // v0.5.25: Approach-Stability v2 — Stable-Approach-
                            // Gate-konform mit HAT-statt-AGL, V/S-Jerk,
                            // IAS-σ, Excessive-Sink, Stable-Config.
                            approach_vs_deviation_fpm: stats.approach_vs_deviation_fpm,
                            approach_max_vs_deviation_below_500_fpm: stats
                                .approach_max_vs_deviation_below_500_fpm,
                            approach_bank_stddev_filtered_deg: stats
                                .approach_bank_stddev_filtered_deg,
                            approach_runway_changed_late: stats.approach_runway_changed_late,
                            approach_stable_at_gate: stats.approach_stable_at_gate,
                            approach_window_sample_count: stats.approach_window_sample_count,
                            approach_vs_jerk_fpm: stats.approach_vs_jerk_fpm,
                            approach_ias_stddev_kt: stats.approach_ias_stddev_kt,
                            approach_excessive_sink: Some(stats.approach_excessive_sink),
                            approach_stable_config: stats.approach_stable_config,
                            approach_used_hat: Some(stats.approach_used_hat),
                            // v0.5.22: feeds the live-monitor's "Bahn-
                            // Auslastung"-sub-score so it matches the
                            // in-app PIREP value 1:1.
                            runway_length_m: rwy_match
                                .map(|m| m.length_ft * 0.3048),
                            // v0.5.22: actual_burn − planned_burn over
                            // planned_burn × 100. Mirrors the client's
                            // `LandingRecord.fuel_efficiency_pct`.
                            // actual_burn = block_fuel − landing_fuel.
                            fuel_efficiency_pct: match (
                                stats.block_fuel_kg,
                                stats.landing_fuel_kg,
                                stats.planned_burn_kg,
                            ) {
                                (Some(block), Some(landing), Some(plan))
                                    if plan > 0.0 =>
                                {
                                    let actual = block - landing;
                                    Some(((actual - plan) / plan) * 100.0)
                                }
                                _ => None,
                            },
                        }
                    })
                };
                if let Some(payload) = payload_opt {
                    let app_state = app.state::<AppState>();
                    let mqtt = app_state.mqtt.lock().await;
                    if let Some(handle) = mqtt.as_ref() {
                        handle.touchdown(payload);
                    }
                }
            }
            // Diff cockpit knobs against last-seen values and log changes
            // to the activity feed. One entry per change, not per tick.
            detect_telemetry_changes(&app, &flight, &snap);
            let position = snapshot_to_position(&snap);

            // Collect any text-log entries we want to mirror into phpVMS's
            // `/acars/logs` for the PIREP detail page. Phase changes get a
            // generic "Phase: <name>" line; landings get the touchdown
            // summary. Posted in a single batch at the end of the tick to
            // minimise HTTP round-trips.
            let mut acars_log_entries: Vec<api_client::LogEntry> = Vec::new();
            if let Some(new_phase) = phase_change {
                acars_log_entries.push(api_client::LogEntry {
                    log: format!("Phase: {}", phase_human_label(new_phase)),
                    lat: Some(snap.lat),
                    lon: Some(snap.lon),
                    created_at: Some(Utc::now().to_rfc3339()),
                });
                // v0.5.11: live-tracking phase publish. Retained
                // message — Monitor sees current phase on connect
                // even when joining mid-flight.
                {
                    let app_state = app.state::<AppState>();
                    let mqtt = app_state.mqtt.lock().await;
                    if let Some(handle) = mqtt.as_ref() {
                        handle.phase(new_phase, Utc::now());
                    }
                }
            }
            if let Some(msg) = landing_log_message {
                acars_log_entries.push(api_client::LogEntry {
                    log: msg,
                    lat: Some(snap.lat),
                    lon: Some(snap.lon),
                    created_at: Some(Utc::now().to_rfc3339()),
                });
            }

            // Drain any T&G / go-around log lines that the FSM
            // queued during this tick (or accumulated since the
            // last tick — the touchdown sampler runs at 30 Hz, the
            // streamer ticks at ~5 s, so multiple events can stack
            // up between drains). Order is preserved so the PIREP
            // detail page reads chronologically.
            {
                let mut stats = flight.stats.lock().expect("flight stats");
                if !stats.pending_acars_logs.is_empty() {
                    let drained: Vec<String> =
                        std::mem::take(&mut stats.pending_acars_logs);
                    drop(stats);
                    let now_ts = Utc::now().to_rfc3339();
                    for line in drained {
                        // Mirror to the in-app activity log so the
                        // pilot sees the T&G / go-around immediately
                        // in the cockpit dashboard, not just after the
                        // PIREP is filed and they look at phpVMS. The
                        // 5 s dedupe window is fine here — successive
                        // T&Gs are >1 min apart (FSM has to descend
                        // back to Approach in between).
                        log_activity_handle(
                            &app,
                            ActivityLevel::Info,
                            line.clone(),
                            None,
                        );
                        acars_log_entries.push(api_client::LogEntry {
                            log: line,
                            lat: Some(snap.lat),
                            lon: Some(snap.lon),
                            created_at: Some(now_ts.clone()),
                        });
                    }
                }
            }

            // Try to drain any positions we couldn't ship in earlier
            // ticks before sending the new one — keeps phpVMS's row
            // ordering chronological even after a network gap.
            let queue = open_position_queue(&app);
            if let Some(q) = &queue {
                drain_position_queue(q, &client, &flight.pirep_id).await;
            }

            // v0.5.11: MQTT live-tracking publish (best-effort,
            // independent of phpVMS post). Publishing happens on a
            // bounded mpsc — try_send drops at full buffer so a
            // stalled broker can NEVER stall the streamer's hot
            // path. Position is the high-frequency channel; the
            // crate marks it QoS 0 retained for live-map snap-on-
            // connect.
            {
                let app_state = app.state::<AppState>();
                    let mqtt = app_state.mqtt.lock().await;
                if let Some(handle) = mqtt.as_ref() {
                    let meta = aeroacars_mqtt::FlightMeta {
                        callsign: format_callsign(
                            &flight.airline_icao,
                            &flight.flight_number,
                        ),
                        aircraft_icao: flight.aircraft_icao.clone(),
                        dep_icao: flight.dpt_airport.clone(),
                        arr_icao: flight.arr_airport.clone(),
                        // v0.5.19: phpVMS-side registration overrides
                        // sim's ATC-ID for the live-tracking stream.
                        planned_registration: flight.planned_registration.clone(),
                    };
                    // v0.5.14: pass current phase so it's inlined into
                    // the position payload — Monitor doesn't have to
                    // wait for a separate phase-topic delivery.
                    let live_phase = phase_change.unwrap_or(prev_phase);
                    handle.position(&snap, &meta, live_phase);
                }
            }

            // v0.5.21: phpVMS POST throttled to phase-aware cadence
            // (`position_interval(phase)`, 4-30 s). The streamer-tick
            // itself runs at 3 s for MQTT — without this gate every
            // tick would slam phpVMS, causing 10x DB-row growth in
            // `pirep_positions` plus matching cron-load increase.
            //
            // First post fires immediately (last_phpvms_post_at is
            // None); subsequent posts wait until the phase-specific
            // interval has elapsed since the previous successful or
            // attempted post.
            let should_post_phpvms = match last_phpvms_post_at {
                None => true,
                Some(t) => t.elapsed() >= position_interval(current_phase),
            };
            if !should_post_phpvms {
                // Skip phpVMS this tick — the MQTT publish above
                // already went out at 3 s cadence; phpVMS will catch
                // up on the next eligible tick.
            } else {
                last_phpvms_post_at = Some(std::time::Instant::now());
                match client
                .post_positions(&flight.pirep_id, &[position.clone()])
                .await
            {
                Ok(()) => {
                    tracing::info!(
                        pirep_id = %flight.pirep_id,
                        lat = snap.lat,
                        lon = snap.lon,
                        alt_msl_ft = snap.altitude_msl_ft,
                        gs_kt = snap.groundspeed_kt,
                        "position posted"
                    );
                    // Stamp the success time + clear any stale queue
                    // count so the dashboard's LIVE indicator stays
                    // green and back-fills the "last send" line.
                    {
                        let mut stats = flight.stats.lock().expect("flight stats");
                        stats.last_position_at = Some(Utc::now());
                        let queue_len = queue
                            .as_ref()
                            .and_then(|q| q.len().ok())
                            .unwrap_or(0) as u32;
                        stats.queued_position_count = queue_len;
                    }
                    // Mirror the snapshot into the per-flight JSONL log
                    // for offline replay / debugging. Best-effort.
                    record_event(
                        &app,
                        &flight.pirep_id,
                        &FlightLogEvent::Position {
                            timestamp: Utc::now(),
                            snapshot: snap.clone(),
                        },
                    );
                }
                Err(ApiError::NotFound) => {
                    // phpVMS soft-deleted the PIREP under us (most likely
                    // the RemoveExpiredLiveFlights hourly cron). Queueing
                    // would just spam 404s forever — handle it as a hard
                    // remote cancellation instead.
                    handle_remote_cancellation(&app, &flight, "POST positions");
                    break;
                }
                Err(e) => {
                    tracing::warn!(
                        pirep_id = %flight.pirep_id,
                        error = %e,
                        "position post failed; queueing for later replay"
                    );
                    if let Some(q) = &queue {
                        let queued = QueuedPosition {
                            pirep_id: flight.pirep_id.clone(),
                            position: serde_json::to_value(&position).unwrap_or_default(),
                        };
                        match q.enqueue(queued) {
                            Ok(len) => {
                                {
                                    let mut stats =
                                        flight.stats.lock().expect("flight stats");
                                    stats.queued_position_count = len as u32;
                                }
                                log_activity_handle(
                                    &app,
                                    ActivityLevel::Warn,
                                    "Position queued (offline)".to_string(),
                                    Some(format!("{} pending", len)),
                                );
                            }
                            Err(qe) => tracing::warn!(error = ?qe, "could not enqueue position"),
                        }
                    }
                }
            }
            } // end `if should_post_phpvms`

            // Periodically flush running stats to disk so a Tauri restart
            // (or crash) doesn't lose distance/fuel/phase data. The
            // identification fields are written every time too — cheap,
            // and keeps the file consistent if any of them changed via
            // backfill (airline_icao / planned_registration on resume).
            let should_persist = {
                let stats = flight.stats.lock().expect("flight stats");
                stats.position_count % STATS_PERSIST_EVERY_TICKS == 0
            };
            // Re-check stop *before* writing — `flight_end` may have run
            // since the top-of-loop check, taken the flight, called
            // `clear_persisted_flight`, and we'd otherwise resurrect the
            // disk file. Without this, the resume banner pops on the next
            // launch for a flight the user already filed.
            if should_persist && !flight.stop.load(Ordering::Relaxed) {
                save_active_flight(&app, &flight);
            }

            // On phase change, push the new status to phpVMS so the
            // live-map and PIREP detail page reflect the current phase.
            if let Some(new_phase) = phase_change {
                log_activity_handle(
                    &app,
                    ActivityLevel::Info,
                    format!("Phase: {:?}", new_phase),
                    Some(format!(
                        "Alt {:.0} ft, GS {:.0} kt, AGL {:.0} ft",
                        snap.altitude_msl_ft, snap.groundspeed_kt, snap.altitude_agl_ft
                    )),
                );
                // Diagnostic fuel/weight snapshot at the four phases
                // where the values get captured into PIREP fields.
                // Lets the pilot verify what the app saw — especially
                // useful with Fenix where TOTAL WEIGHT / ZFW / payload
                // sometimes report 0 and the pilot can't tell from
                // the PIREP alone whether the SimVar was missing or
                // the addon hadn't finished loading at capture time.
                log_fuel_weight_at_phase(&app, &flight, new_phase, &snap);
                record_event(
                    &app,
                    &flight.pirep_id,
                    &FlightLogEvent::PhaseChanged {
                        timestamp: Utc::now(),
                        from: prev_phase,
                        to: new_phase,
                        altitude_msl_ft: snap.altitude_msl_ft,
                        groundspeed_kt: snap.groundspeed_kt,
                        altitude_agl_ft: snap.altitude_agl_ft,
                    },
                );
                // METAR snapshot at the two phase transitions where it
                // matters most: just after takeoff for the departure
                // weather, and just before/at touchdown for the arrival
                // weather. Spawned async so a flaky aviationweather.gov
                // doesn't stall the streamer.
                maybe_spawn_metar_fetch(&app, &flight, new_phase);
            }

            // Unified heartbeat-and-phase-update: POST `/pireps/{id}/update`
            // either when the phase just changed (so the live map sees the
            // new status immediately) OR after every `HEARTBEAT_INTERVAL`
            // (so phpVMS's RemoveExpiredLiveFlights cron never fires
            // mid-flight). Both purposes use the same payload — sending
            // monotonic flight_time / distance also guarantees the row is
            // dirty so Eloquent always writes UPDATE and bumps updated_at.
            let phase_for_heartbeat = phase_change.unwrap_or(current_phase);
            let due_for_heartbeat = last_heartbeat.elapsed() >= HEARTBEAT_INTERVAL;
            if phase_change.is_some() || due_for_heartbeat {
                let body = {
                    let stats = flight.stats.lock().expect("flight stats");
                    build_heartbeat_body(&snap, &stats, phase_for_heartbeat)
                };
                match client.update_pirep(&flight.pirep_id, &body).await {
                    Ok(()) => {
                        last_heartbeat = std::time::Instant::now();
                        {
                            let mut stats = flight.stats.lock().expect("flight stats");
                            stats.last_heartbeat_at = Some(Utc::now());
                        }
                        tracing::debug!(
                            pirep_id = %flight.pirep_id,
                            phase = ?phase_for_heartbeat,
                            phase_changed = phase_change.is_some(),
                            flight_time_min = body.flight_time.unwrap_or(0),
                            distance_nm = body.distance.unwrap_or(0.0),
                            "PIREP heartbeat sent"
                        );
                    }
                    Err(ApiError::NotFound) => {
                        handle_remote_cancellation(&app, &flight, "POST update");
                        break;
                    }
                    Err(e) => {
                        tracing::warn!(
                            pirep_id = %flight.pirep_id,
                            phase = ?phase_for_heartbeat,
                            error = %e,
                            "PIREP heartbeat failed"
                        );
                    }
                }
            }

            // Mirror collected text events into phpVMS's `/acars/logs` for
            // the PIREP detail page. Best-effort: a transient network
            // error here doesn't matter — the activity log + recorder
            // already have the durable copy. 404 still means the PIREP
            // is gone, so handle that the same way as the heartbeat.
            if !acars_log_entries.is_empty() {
                match client
                    .post_acars_logs(&flight.pirep_id, &acars_log_entries)
                    .await
                {
                    Ok(()) => {
                        tracing::debug!(
                            pirep_id = %flight.pirep_id,
                            count = acars_log_entries.len(),
                            "ACARS log lines pushed"
                        );
                    }
                    Err(ApiError::NotFound) => {
                        handle_remote_cancellation(&app, &flight, "POST acars/logs");
                        break;
                    }
                    Err(e) => {
                        tracing::warn!(
                            pirep_id = %flight.pirep_id,
                            count = acars_log_entries.len(),
                            error = %e,
                            "ACARS log push failed"
                        );
                    }
                }
            }
        }
        tracing::info!(pirep_id = %flight.pirep_id, "position streamer stopped");
    });
}

/// Map SimBrief's text fix `<type>` ("apt", "wpt", "vor", "ndb", "ltlg")
/// to phpVMS's numeric `nav_type` for `POST /pireps/{id}/route`. phpVMS
/// is permissive (`nav_type` is `sometimes|int`); when we can't classify
/// a fix we omit the field and let the server render it generically.
fn simbrief_kind_to_nav_type(kind: &str) -> Option<i32> {
    match kind.to_ascii_lowercase().as_str() {
        "wpt" => Some(1),
        "ndb" => Some(2),
        "vor" => Some(3),
        "apt" => Some(4),
        _ => None,
    }
}

/// Human-readable single-word label for a `FlightPhase`. Used in the
/// `/acars/logs` text we ship to phpVMS. Pilots already see these in
/// the German activity feed; matching them here keeps the PIREP detail
/// consistent with what was on screen during the flight.
fn phase_human_label(phase: FlightPhase) -> &'static str {
    match phase {
        FlightPhase::Preflight => "Preflight",
        FlightPhase::Boarding => "Boarding",
        FlightPhase::Pushback => "Pushback",
        FlightPhase::TaxiOut => "Taxi Out",
        FlightPhase::TakeoffRoll => "Takeoff Roll",
        FlightPhase::Takeoff => "Takeoff",
        FlightPhase::Climb => "Initial Climb",
        FlightPhase::Cruise => "Cruise",
        FlightPhase::Holding => "Holding",
        FlightPhase::Descent => "Descent",
        FlightPhase::Approach => "Approach",
        FlightPhase::Final => "Final",
        FlightPhase::Landing => "Landing",
        FlightPhase::TaxiIn => "Taxi In",
        FlightPhase::BlocksOn => "On Blocks",
        FlightPhase::Arrived => "Arrived",
        FlightPhase::PirepSubmitted => "PIREP Submitted",
    }
}

/// Read a snapshot from whichever adapter is currently active per
/// the persisted `SimKind`. We try the active adapter first; if it
/// hasn't seen any data yet (e.g. just after startup), we still
/// return None — caller is expected to handle "no snapshot yet".
fn current_snapshot(app: &AppHandle) -> Option<SimSnapshot> {
    let state = app.state::<AppState>();
    let kind = read_sim_config(app).kind;
    if kind.is_xplane() {
        let adapter = state.xplane.lock().expect("xplane lock");
        return adapter.snapshot();
    }
    #[cfg(target_os = "windows")]
    {
        if kind.is_msfs() {
            let adapter = state.msfs.lock().expect("msfs lock");
            return adapter.snapshot();
        }
    }
    None
}

/// Drain any pending touchdown event from the AeroACARS X-Plane Plugin
/// (v0.5.0+ premium mode). Returns `Some` exactly once per landing
/// when the plugin is installed and detected a touchdown edge in its
/// flight-loop callback. `None` if the plugin isn't running, or if
/// the active sim isn't X-Plane, or if no touchdown is pending. The
/// touchdown sampler calls this each tick and prefers premium values
/// over its own RREF-derived edge detection when available.
fn current_premium_touchdown(app: &AppHandle) -> Option<sim_xplane::PremiumTouchdown> {
    let state = app.state::<AppState>();
    let kind = read_sim_config(app).kind;
    if !kind.is_xplane() {
        return None;
    }
    let adapter = state.xplane.lock().expect("xplane lock");
    adapter.take_premium_touchdown()
}

/// Status of the AeroACARS X-Plane Plugin connection. Returns
/// `(ever_seen, active)` for the X-Plane sim path; `(false, false)`
/// for any other sim. UI uses `active` to show the "X-PLANE PREMIUM"
/// badge.
fn current_premium_status(app: &AppHandle) -> sim_xplane::PremiumStatus {
    let state = app.state::<AppState>();
    let kind = read_sim_config(app).kind;
    if !kind.is_xplane() {
        return sim_xplane::PremiumStatus {
            ever_seen: false,
            active: false,
            packet_count: 0,
        };
    }
    let adapter = state.xplane.lock().expect("xplane lock");
    adapter.premium_status()
}

/// Update running stats AND step the flight-phase FSM. Returns the new phase
/// when a transition fires, otherwise `None`.
fn step_flight(flight: &ActiveFlight, snap: &SimSnapshot) -> Option<FlightPhase> {
    let mut stats = flight.stats.lock().expect("flight stats");

    // Distance accounting.
    if let (Some(prev_lat), Some(prev_lon)) = (stats.last_lat, stats.last_lon) {
        let d_m = ::geo::distance_m(prev_lat, prev_lon, snap.lat, snap.lon);
        if d_m > DISTANCE_EPSILON_M {
            stats.distance_nm += d_m / 1852.0;
        }
    }
    stats.last_lat = Some(snap.lat);
    stats.last_lon = Some(snap.lon);
    stats.position_count = stats.position_count.saturating_add(1);
    let prev_fuel_kg = stats.last_fuel_kg;
    stats.last_fuel_kg = Some(snap.fuel_total_kg);
    // v0.3.0: ZFW + Total-Weight für Live-Loadsheet (Cockpit-Tab
    // während Boarding-Phase). Updates jeden Tick. None bleiben sie
    // wenn das Aircraft-Profil die SimVars nicht meldet (z.B. Fenix
    // ohne erweiterte Profile).
    stats.last_zfw_kg = snap.zfw_kg;
    stats.last_total_weight_kg = snap.total_weight_kg;

    // Block fuel = peak fuel observed across the flight, with a
    // defuel guard. Rationale:
    //   * Tracking the peak captures the loaded amount no matter
    //     when refuelling completes — Fenix's async EFB load,
    //     APU burn during boarding, etc. all become non-issues
    //     because the peak naturally settles on the loaded value.
    //   * BUT pure peak doesn't handle defuelling: pilot loads
    //     4000 kg, realises overload, removes 1000 kg → peak
    //     stays at 4000, Used Fuel = 4000 − landing reads ~1000
    //     kg too high. So we detect a sudden large drop (well
    //     above any realistic engine burn over a tick interval)
    //     and reset the peak to the current value. Threshold
    //     200 kg covers even A380 at takeoff thrust over a 30 s
    //     cruise tick (~210 kg max), so no false positives from
    //     normal operations.
    let live_fuel = snap.fuel_total_kg;
    if live_fuel > 0.0 {
        const DEFUEL_THRESHOLD_KG: f32 = 200.0;
        let is_defuel = prev_fuel_kg
            .map(|prev| prev - live_fuel > DEFUEL_THRESHOLD_KG)
            .unwrap_or(false);
        if is_defuel {
            tracing::info!(
                prev = prev_fuel_kg,
                live = live_fuel,
                drop = prev_fuel_kg.unwrap_or(0.0) - live_fuel,
                "defuel detected — resetting block_fuel peak baseline"
            );
            stats.block_fuel_kg = Some(live_fuel);
        } else {
            stats.block_fuel_kg = Some(
                stats
                    .block_fuel_kg
                    .map_or(live_fuel, |peak| peak.max(live_fuel)),
            );
        }
    }

    // Peak altitude tracker — every tick, take the max with the live
    // MSL altitude. Reported as the PIREP `level` field.
    let alt = snap.altitude_msl_ft as f32;
    stats.peak_altitude_ft = Some(stats.peak_altitude_ft.map_or(alt, |p| p.max(alt)));

    // Mark the flight as having actually been airborne. Sticky once
    // set — even if the aircraft comes back down (final approach,
    // touch-and-go) the flag stays true. Gates the universal
    // Arrived-fallback so neither GSX repositioning nor a few-meter
    // wackeln at the gate accidentally promotes the FSM to Arrived
    // pre-flight. See WAS_AIRBORNE_AGL_FT for the threshold rationale.
    // `was_airborne` flag — three-layer defense against false positives
    // (live bug 2026-05-03: PMDG B738, MSFS reported AGL=53819 ft for
    // 60 s during scenery load and the flag flipped true; 30 min later
    // the GSX pushback fired the universal Arrived-fallback through
    // the now-poisoned gate):
    //
    //   1. AGL must be in a sane range — > 50 ft AND < 30000 ft.
    //      53819 ft would have been ignored.
    //   2. `block_off_at` must be set — chronologically the aircraft
    //      can't be airborne before it's even been block-off.
    //      Belt-and-braces against any other future loading glitch
    //      that produces sane-looking but pre-flight airborne values.
    //   3. Conditions must hold for WAS_AIRBORNE_DWELL_TICKS in a
    //      row — single-tick glitches don't poison the flag.
    let airborne_now = !snap.on_ground
        && stats.block_off_at.is_some()
        && {
            let agl = snap.altitude_agl_ft as f32;
            agl > WAS_AIRBORNE_AGL_FT && agl < WAS_AIRBORNE_AGL_MAX_FT
        };
    if airborne_now {
        stats.airborne_dwell_ticks =
            stats.airborne_dwell_ticks.saturating_add(1);
        if stats.airborne_dwell_ticks >= WAS_AIRBORNE_DWELL_TICKS {
            stats.was_airborne = true;
        }
    } else {
        stats.airborne_dwell_ticks = 0;
    }

    // Touchdown ring-buffer is now populated by the dedicated 30 Hz
    // sampler (`spawn_touchdown_sampler`) — the streamer ticks every
    // 5–8 s during Final / Landing, way too sparse to reliably catch
    // the actual touchdown subframe. The sampler shares the same
    // `stats.snapshot_buffer` so the FSM still reads the buffer when
    // it transitions to Landing, just with 30× more samples to pick
    // the worst V/S and peak G from.

    let now = Utc::now();
    let prev_phase = stats.phase;
    let mut next_phase = prev_phase;
    let was_on_ground = stats.was_on_ground.unwrap_or(snap.on_ground);
    // was_parking_brake is no longer consulted by any phase transition
    // (we removed the brake-release-only Pushback trigger), but the
    // tracking field stays — the activity-log change detector below
    // and any future transition that wants brake-state history will
    // pick it up. Acknowledge to silence the unused-variable warning.
    let _ = stats.was_parking_brake;

    // Capture the departure gate as soon as MSFS gives us a parking
    // name — that means the aircraft is still on the named stand and
    // hasn't pushed back yet. Set once and stay.
    if stats.dep_gate.is_none() {
        if let Some(name) = snap.parking_name.as_ref().filter(|s| !s.is_empty()) {
            let label = match snap.parking_number.as_ref() {
                Some(num) if !num.is_empty() => format!("{name} {num}"),
                _ => name.clone(),
            };
            stats.dep_gate = Some(label);
        }
    }

    // v0.5.11: freeze FSM transitions during sim pause / slew mode.
    //
    // Position recording, distance tracking, fuel tracking, peak-altitude
    // tracking, and the phpVMS heartbeat above all continue running.
    // Only the phase classifier pauses with the sim — otherwise:
    //
    //   * Pause: a frozen snapshot with bank > 15° and |VS| < 200 fpm
    //     would let the wall-clock dwell timer falsely classify the
    //     pilot as "in Holding" after 90 real-world seconds (pilot
    //     making coffee while paused mid-turn = falsely identified as
    //     a holding pattern).
    //   * Slew: teleporting from FL340 → 500 ft AGL produces a
    //     massive lost_altitude spike in one tick → would falsely
    //     fire Climb→Descent / Heli-vertical transitions / mid-air
    //     ground-edges. Slew is a debug/positioning tool, not real
    //     flight motion.
    //
    // When the pilot un-pauses or exits slew, FSM resumes from the
    // current state with current snapshot values. Any pending dwell
    // timers (Holding, Touch-and-Go, Go-Around) keep their last
    // value and continue from there.
    if snap.paused || snap.slew_mode {
        return None;
    }

    // Match on a local Copy so the rest of the body is free to mutate `stats`.
    match prev_phase {
        FlightPhase::Boarding => {
            // Pushback / departure detection: actual movement is the
            // only reliable trigger. Brake-release alone is NOT enough
            // — pilots routinely release the brake to test controls,
            // prep for taxi, or because GSX flips it during boarding.
            // Without movement, that's still Boarding.
            //
            // Branches:
            //   * Moving with engines OFF → tug is pushing → Pushback
            //   * Moving with engines ON  → straight to TaxiOut
            //                                (powered movement, no push)
            //
            // Both flows MSFS supports get covered: the native Ctrl+P
            // push (often doesn't touch parking brake) and the GSX
            // / hand-flown push (brake released first, then truck).
            // v0.5.11: accept water surface as ground-equivalent for
            // seaplanes. Some sims (MSFS especially) report
            // on_ground=false when an aircraft is sitting on water,
            // because the gear-on-pavement contact model returns
            // false on liquid surfaces. The aircraft is functionally
            // on the surface (AGL ≈ 0, VS ≈ 0) and water-taxi
            // movement is real motion. Without this, seaplanes that
            // boot on water would stay stuck in Boarding forever.
            //
            // The AGL < 5 ft + |VS| < 50 fpm guard ensures we're
            // genuinely at surface level (not low-level approach,
            // which is past Boarding anyway, but defensive).
            let on_surface = snap.on_ground
                || (snap.altitude_agl_ft < 5.0
                    && snap.vertical_speed_fpm.abs() < 50.0);
            if on_surface && snap.groundspeed_kt > 0.5 {
                stats.block_off_at = Some(now);
                // Note: block_fuel_kg is tracked as a running peak
                // (see top of step_flight), not captured here. APU
                // burn during boarding would otherwise eat into the
                // captured value before block-off ever fires.
                // v0.3.0: Loadsheet-Snapshot ins Activity-Log. Einmalig
                // beim Block-off — gibt dem PIREP-Audit-Trail einen
                // klaren "was war geladen, als wir losrollten" Eintrag.
                if !stats.loadsheet_logged_at_blockoff {
                    let block = stats.block_fuel_kg.unwrap_or(snap.fuel_total_kg);
                    let zfw = snap.zfw_kg;
                    let tow = snap.total_weight_kg;
                    let plan_block = stats.planned_block_fuel_kg;
                    // v0.3.0: Loadsheet als kompakte Zeile in den
                    // pending_acars_logs Buffer pushen. Streamer drainet
                    // den nächsten Tick und mirrort in Activity-Log
                    // (Cockpit-UI) UND ACARS-Log (phpVMS-PIREP). Bewusst
                    // einzeilig — der Drain-Pfad unterstützt keine
                    // 2-zeiligen Display-Logs.
                    // v0.3.0: ohne Emoji-Icon — wird in phpVMS-PIREP-
                    // Detailseite als verzerrter Glyph gerendert (Font-
                    // Stack hat dort keinen Emoji-Font).
                    let mut line = String::from("Loadsheet @ Block-off — Block ");
                    line.push_str(&format!("{:.0} kg", block));
                    if let Some(p) = plan_block {
                        let delta = block - p;
                        line.push_str(&format!(" (Plan {:.0} kg, Δ {:+.0})", p, delta));
                    }
                    if let Some(z) = zfw {
                        line.push_str(&format!(" · ZFW {:.0} kg", z));
                    }
                    if let Some(t) = tow {
                        line.push_str(&format!(" · TOW {:.0} kg", t));
                    }
                    stats.pending_acars_logs.push(line);
                    // v0.3.0 Bonus 1: "Über-Tankt"-Hinweis als zweiter
                    // Eintrag, falls IST-Block > Plan-Block + Reserve +
                    // 500 kg Toleranz. Sanft, nicht blockierend.
                    if let (Some(p_block), Some(p_reserve)) =
                        (stats.planned_block_fuel_kg, stats.planned_reserve_kg)
                    {
                        let threshold = p_block + p_reserve + 500.0;
                        if block > threshold {
                            let extra = block - p_block;
                            stats.pending_acars_logs.push(format!(
                                "💡 Über-tankt — Sehr viel Sprit an Bord ({:+.0} kg über Plan + Reserve), höherer Burn unterwegs zu erwarten.",
                                extra
                            ));
                        }
                    }
                    stats.loadsheet_logged_at_blockoff = true;
                }
                next_phase = if snap.engines_running == 0 {
                    FlightPhase::Pushback
                } else {
                    FlightPhase::TaxiOut
                };
            }
            // v0.5.10/11: pure-hover / glider direct-launch escape hatch.
            // Aircraft that lift off straight from the gate without
            // ever entering TaxiOut would otherwise stay in Boarding
            // forever. Detect the on_ground edge and skip straight to
            // Takeoff. Covers:
            //   * Helicopters that take off vertically from the gate
            //     (GS = 0, never triggers Boarding → TaxiOut on
            //     groundspeed alone)
            //   * Gliders that get winched from the boarding spot
            //     (engines = 0 throughout)
            //
            // v0.5.11 widening: dropped engines > 0 requirement so
            // glider winch launches fire correctly. AGL > 3 ft +
            // VS > 100 fpm remain as the safety net against sim-
            // glitch on_ground-flicker during boarding.
            else if was_on_ground
                && !snap.on_ground
                && snap.altitude_agl_ft > 3.0
                && snap.vertical_speed_fpm > 100.0
            {
                next_phase = FlightPhase::Takeoff;
                stats.takeoff_at = Some(now);
                stats.takeoff_fuel_kg = Some(snap.fuel_total_kg);
                // v0.5.16: capture pitch + bank for tail-strike /
                // wing-strike maintenance detection (DisposableSpecial
                // dmaintenance reads these as numeric custom fields).
                // v0.5.24: prefer the 50Hz-sampler value (Wheels-Up-
                // frame-genau) ueber den Streamer-Tick-Snapshot.
                stats.takeoff_pitch_deg = stats
                    .sampler_takeoff_pitch_deg
                    .or(Some(snap.pitch_deg));
                stats.takeoff_bank_deg = stats
                    .sampler_takeoff_bank_deg
                    .or(Some(snap.bank_deg));
                if let Some(tw) = snap.total_weight_kg {
                    stats.takeoff_weight_kg = Some(tw as f64);
                } else {
                    let zfw = snap.zfw_kg.unwrap_or(0.0);
                    let weight = zfw as f64 + snap.fuel_total_kg as f64;
                    if weight > 0.0 {
                        stats.takeoff_weight_kg = Some(weight);
                    }
                }
                tracing::info!(
                    pirep_id = %flight.pirep_id,
                    "vertical takeoff from boarding (helicopter pure-hover) — Boarding → Takeoff"
                );
            }
        }
        FlightPhase::Pushback => {
            // Pushback → TaxiOut Übergang. Echter Ablauf am Gate:
            //   1. Tug schiebt zurück
            //   2. Tug-Crew koppelt ab (PUSHBACK STATE = 3)
            //   3. Pilot steht ~10 s mit gesetzter Bremse, hat Funk,
            //      checkt Freigabe
            //   4. Pilot löst Bremse + Schub → Flugzeug rollt an
            // Erst dann ist es "Taxi". Vorher würde der Tug-Schub bei
            // > 3 kt fälschlicherweise schon TaxiOut triggern.
            //
            // Implementierung als 3-Stufen-Gate:
            //   * `tug_done` = MSFS meldet PUSHBACK STATE == 3
            //   * `stopped`  = Flugzeug ist nach tug_done zum Stillstand
            //                  gekommen (gs < 0.5 kt). Wir merken uns
            //                  den Zeitpunkt in `pushback_stopped_at`.
            //   * `dwell_ok` = Stillstand mind. 10 s gehalten.
            //   * `powered_taxi` = Triebwerke + gs > 3 kt
            // Nur wenn alles stimmt, springt die Phase auf TaxiOut.
            //
            // Sims ohne PUSHBACK STATE (X-Plane / manche Add-ons) fallen
            // auf die alte Heuristik zurück — sonst würden sie für immer
            // in Pushback stecken bleiben.
            const PUSHBACK_DWELL_SECS: i64 = 10;
            let tug_done = snap.pushback_state == Some(3);
            let powered_taxi = snap.on_ground
                && snap.engines_running > 0
                && snap.groundspeed_kt > 3.0;

            if tug_done {
                // Stillstand nach Tug-Ab: Timestamp setzen (einmal).
                if snap.groundspeed_kt < 0.5 && stats.pushback_stopped_at.is_none() {
                    stats.pushback_stopped_at = Some(now);
                }
                let dwell_ok = stats
                    .pushback_stopped_at
                    .map(|t| (now - t).num_seconds() >= PUSHBACK_DWELL_SECS)
                    .unwrap_or(false);
                if dwell_ok && powered_taxi {
                    stats.pushback_stopped_at = None;
                    next_phase = FlightPhase::TaxiOut;
                }
            } else if snap.pushback_state.is_none() && powered_taxi {
                // Kein PUSHBACK STATE vom Sim verfügbar — alte
                // Fallback-Logik: bewegt sich + Triebwerke an = TaxiOut.
                next_phase = FlightPhase::TaxiOut;
            }
        }
        FlightPhase::TaxiOut => {
            // Threshold lowered from 40 → 30 kt so GA aircraft (Cessna,
            // Diamond) which rotate around 50 kt enter TakeoffRoll well
            // before liftoff. Plus the engine-running guard means a
            // pilot dragging the parking brake at high taxi speed
            // doesn't accidentally trigger TakeoffRoll without throttle.
            if snap.on_ground
                && snap.groundspeed_kt > 30.0
                && snap.engines_running > 0
            {
                next_phase = FlightPhase::TakeoffRoll;
            }
            // v0.5.10/11: helicopter / VTOL / glider / seaplane
            // escape hatch.
            //
            // Conventional aircraft trigger TakeoffRoll via GS > 30 kt,
            // then Takeoff via the on_ground edge. Several classes of
            // aircraft skip TakeoffRoll entirely:
            //   * Helicopters / VTOL: take off vertically (GS stays
            //     below 30 kt the whole time on the ground)
            //   * Gliders on aerotow: GS > 30 kt but engines = 0
            //     (the glider's own engine is off, the towplane pulls)
            //   * Gliders on winch: very fast vertical climb, engines = 0
            //   * Seaplanes that report on_ground=true on water:
            //     same path as conventional but engines might briefly
            //     show 0 in some sims
            //
            // For all of these we want: when in TaxiOut and the
            // on_ground edge fires (ground → air), promote to Takeoff
            // directly. The AGL > 5 ft + VS > 100 fpm guards confirm
            // a REAL liftoff (not sim-glitch on_ground-flicker on
            // bumpy runways / PMDG texture quirks).
            //
            // v0.5.11 widening: dropped the engines > 0 requirement
            // so glider tow / winch launches also fire correctly.
            // The AGL/VS guards remain as the false-positive safety
            // net (a single-frame on_ground-flicker can't satisfy
            // both AGL > 5 ft AND VS > 100 fpm).
            else if (was_on_ground
                && !snap.on_ground
                && snap.altitude_agl_ft > 5.0
                && snap.vertical_speed_fpm > 100.0)
                // v0.5.11: seaplane catchall — sims that report
                // on_ground=false on water never produce the ground
                // edge above. Once the seaplane lifts off the water
                // (AGL > 50 ft + VS > 100 fpm + actively airborne),
                // fire Takeoff regardless of edge. AGL > 50 is a
                // strict guard against sim-spawn-at-altitude
                // false-fires; a real water takeoff easily exceeds
                // 50 ft AGL within seconds of breaking the surface.
                || (!snap.on_ground
                    && snap.altitude_agl_ft > 50.0
                    && snap.vertical_speed_fpm > 100.0
                    && !snap.slew_mode
                    && !snap.paused)
            {
                next_phase = FlightPhase::Takeoff;
                stats.takeoff_at = Some(now);
                stats.takeoff_fuel_kg = Some(snap.fuel_total_kg);
                // v0.5.16: capture pitch + bank for tail-strike /
                // wing-strike maintenance detection (DisposableSpecial
                // dmaintenance reads these as numeric custom fields).
                // v0.5.24: prefer the 50Hz-sampler value (Wheels-Up-
                // frame-genau) ueber den Streamer-Tick-Snapshot.
                stats.takeoff_pitch_deg = stats
                    .sampler_takeoff_pitch_deg
                    .or(Some(snap.pitch_deg));
                stats.takeoff_bank_deg = stats
                    .sampler_takeoff_bank_deg
                    .or(Some(snap.bank_deg));
                if let Some(tw) = snap.total_weight_kg {
                    stats.takeoff_weight_kg = Some(tw as f64);
                } else {
                    let zfw = snap.zfw_kg.unwrap_or(0.0);
                    let weight = zfw as f64 + snap.fuel_total_kg as f64;
                    if weight > 0.0 {
                        stats.takeoff_weight_kg = Some(weight);
                    }
                }
                tracing::info!(
                    pirep_id = %flight.pirep_id,
                    "non-conventional takeoff detected (heli / glider / seaplane) — TaxiOut → Takeoff"
                );
            }
        }
        FlightPhase::TakeoffRoll => {
            // PMDG capture (Phase 5.6): TakeoffRoll is the right
            // moment to record V-speeds + planned TO-flaps + the
            // takeoff-config-warning-was-active flag, because the
            // pilot completed PERF-INIT before this point. We
            // capture continuously through the roll (rather than
            // once at entry) because V-speeds may not have been
            // entered until the very last moment.
            if let Some(p) = &snap.pmdg {
                if stats.pmdg_takeoff_flaps_planned.is_none() {
                    stats.pmdg_takeoff_flaps_planned = p.fmc_takeoff_flaps_deg;
                }
                if let (Some(v1), Some(vr), Some(v2)) =
                    (p.fmc_v1_kt, p.fmc_vr_kt, p.fmc_v2_kt)
                {
                    if stats.pmdg_v_speeds_takeoff.is_none() {
                        stats.pmdg_v_speeds_takeoff = Some((v1, vr, v2));
                    }
                }
                if p.takeoff_config_warning {
                    stats.pmdg_takeoff_config_warning_seen = true;
                }
                if stats.pmdg_fmc_flight_number.is_none()
                    && !p.fmc_flight_number.is_empty()
                {
                    stats.pmdg_fmc_flight_number =
                        Some(p.fmc_flight_number.clone());
                }
            }

            if was_on_ground && !snap.on_ground {
                next_phase = FlightPhase::Takeoff;
                stats.takeoff_at = Some(now);
                stats.takeoff_fuel_kg = Some(snap.fuel_total_kg);
                // v0.5.16: capture pitch + bank for tail-strike /
                // wing-strike maintenance detection (DisposableSpecial
                // dmaintenance reads these as numeric custom fields).
                // v0.5.24: prefer the 50Hz-sampler value (Wheels-Up-
                // frame-genau) ueber den Streamer-Tick-Snapshot. Bei
                // tail-strike-empfindlichen Aircraft wie A321 spart das
                // 2-3° Pitch-Drift gegen den 3-30s-spaeter-Tick.
                stats.takeoff_pitch_deg = stats
                    .sampler_takeoff_pitch_deg
                    .or(Some(snap.pitch_deg));
                stats.takeoff_bank_deg = stats
                    .sampler_takeoff_bank_deg
                    .or(Some(snap.bank_deg));
                // Prefer TOTAL WEIGHT (fuel + payload + empty); fall
                // back to ZFW + fuel only when the SimVar isn't wired.
                if let Some(tw) = snap.total_weight_kg {
                    stats.takeoff_weight_kg = Some(tw as f64);
                } else {
                    let zfw = snap.zfw_kg.unwrap_or(0.0);
                    let weight = zfw as f64 + snap.fuel_total_kg as f64;
                    if weight > 0.0 {
                        stats.takeoff_weight_kg = Some(weight);
                    }
                }
            }
        }
        FlightPhase::Takeoff => {
            if snap.altitude_agl_ft > 500.0 {
                next_phase = FlightPhase::Climb;
                // v0.5.9: reset climb peak so the new climb segment
                // starts fresh (handles re-takeoffs after a divert).
                stats.climb_peak_msl = None;
            }
        }
        FlightPhase::Climb => {
            // Track the highest altitude we've seen while climbing —
            // used as the reference point for "did we really start
            // descending, or is this just a single VS spike?".
            let climb_peak = stats.climb_peak_msl.unwrap_or(0.0);
            if (snap.altitude_msl_ft as f32) > climb_peak {
                stats.climb_peak_msl = Some(snap.altitude_msl_ft as f32);
            }

            // v0.5.9: Climb → Descent now requires BOTH:
            //   * Sustained sink rate (< −500 fpm)
            //   * Altitude actually lost from climb peak (> 200 ft)
            //
            // Pilot Michael 2026-05-07 EGPH→HEGN: a single -742 fpm
            // sample (level-off blip during climb to FL340) flipped
            // the FSM to Descent at FL050. Aircraft kept climbing to
            // FL340 and cruised, but FSM was stuck in Descent for the
            // next 50+ minutes because there's no Descent → Climb
            // transition. Without the altitude-lost guard a single
            // turbulence spike or autopilot trim correction wrecks
            // the rest of the timeline.
            //
            // 200 ft threshold filters typical level-off oscillation
            // (pitch-corrections + turbulence rarely exceed this) but
            // is well below any real top-of-descent (which loses
            // thousands of feet quickly).
            let lost_from_peak =
                stats.climb_peak_msl.unwrap_or(0.0) - snap.altitude_msl_ft as f32;

            // Climb → Descent triggers in any of three scenarios:
            //   (a) Standard TOD: sustained sink (-500 fpm) AND
            //       altitude actually lost (>200 ft) — Airliners.
            //   (b) v0.5.10: GA / Heli low-altitude approach —
            //       gentle descent (-100 fpm) AND we're below
            //       3000 ft AGL AND lost >500 ft from climb peak.
            //       Catches GA flights that cruise at 2-3000 ft and
            //       descend gently at 300-500 fpm to a pattern
            //       altitude landing.
            //   (c) v0.5.10: anything significantly below climb peak
            //       AND below 2000 ft AGL — robust catchall for
            //       low-altitude operations where neither (a) nor
            //       (b) clearly fires (helicopter ops, bush flying).
            let standard_tod = snap.vertical_speed_fpm < -500.0
                && lost_from_peak > 200.0;
            let low_altitude_descent = snap.vertical_speed_fpm < -100.0
                && snap.altitude_agl_ft < 3000.0
                && lost_from_peak > 500.0;
            let near_ground_descent = snap.altitude_agl_ft < 2000.0
                && lost_from_peak > 800.0
                && snap.vertical_speed_fpm < 0.0;
            if standard_tod || low_altitude_descent || near_ground_descent {
                next_phase = FlightPhase::Descent;
            } else if snap.vertical_speed_fpm.abs() < 200.0
                && snap.altitude_agl_ft > 5000.0
            {
                next_phase = FlightPhase::Cruise;
            }
            // v0.5.11: removed low-altitude Cruise alternative path.
            // The pre-release v0.5.10 attempt to detect Pattern /
            // GA cruise via "vs.abs() < 100 + lost_from_peak < 100"
            // was unsafe — during ACTIVE climb, lost_from_peak is
            // always ~0 (peak updates each tick). A single tick with
            // vs ≈ 50 fpm (autopilot mode switch, trim, turbulence)
            // would falsely trigger Cruise. GA aircraft just stay in
            // Climb until they descend; our enhanced Climb → Descent
            // triggers above handle the transition robustly. Cruise
            // is reserved for high-altitude flights that genuinely
            // need the 30 s streamer cadence.
        }
        FlightPhase::Cruise => {
            // Track the highest altitude we've seen at this cruise
            // — used as the reference point for "did we really
            // descend, or is this just an ATC step-down?".
            let peak = stats.cruise_peak_msl.unwrap_or(0.0);
            if (snap.altitude_msl_ft as f32) > peak {
                stats.cruise_peak_msl = Some(snap.altitude_msl_ft as f32);
            }

            // Cruise → Descent triggers when sustained sink (-500 fpm,
            // filters trim/turbulence noise) is observed AND one of:
            //
            //   (a) Lost > 5000 ft from the cruise peak — typical
            //       airline TOD. ATC step-downs (FL380 → FL360 = 2000 ft)
            //       stay in Cruise and don't ping-pong the timeline.
            //
            //   (b) AGL < 3000 ft — low-altitude pattern / GA flight.
            //       v0.5.4 hotfix: pilot reported a short MWCR → MWCR
            //       test at 5000-ft pattern altitude that never left
            //       Cruise. The FSM expected a 5000-ft drop from the
            //       cruise peak, but the cruise peak was barely above
            //       5000 ft AGL, so the drop maxed out at 4973 ft —
            //       just below threshold. Universal Arrived fallback
            //       then jumped straight Cruise → Arrived, skipping
            //       Final → Landing entirely (= no touchdown captured).
            //       AGL < 3000 ft + sinking is unambiguous approach
            //       territory regardless of cruise altitude; allow
            //       the transition.
            let lost_alt =
                stats.cruise_peak_msl.unwrap_or(0.0) - snap.altitude_msl_ft as f32;
            let sinking = snap.vertical_speed_fpm < -500.0;
            let big_drop = lost_alt > 5000.0;
            let close_to_ground = snap.altitude_agl_ft < 3000.0;
            if sinking && (big_drop || close_to_ground) {
                next_phase = FlightPhase::Descent;
            } else if check_holding_entry(&mut stats, snap, now) {
                stats.previous_phase_before_holding = Some(FlightPhase::Cruise);
                next_phase = FlightPhase::Holding;
            }
        }
        FlightPhase::Holding => {
            // v0.5.11: detect holding pattern exit. Three exit modes:
            //
            //   1. Active descent (vs < -300 fpm + AGL < 5000 ft) —
            //      ATC has cleared us out of the hold for the
            //      approach. Jump directly to Approach.
            //   2. Low-altitude descent at any rate — going down
            //      toward the runway, definitely not holding anymore.
            //   3. Sustained straight-and-level (bank < 5° for
            //      HOLDING_EXIT_DWELL_SECS) — resumed normal flight.
            //      Restore the phase we came from (Cruise or Approach).
            let descent_resumed = snap.vertical_speed_fpm < -300.0
                && snap.altitude_agl_ft < 5000.0;
            if descent_resumed {
                next_phase = FlightPhase::Approach;
                stats.holding_pending_since = None;
                stats.holding_exit_pending_since = None;
                stats.previous_phase_before_holding = None;
                // Reset approach VS-min tracker (v0.5.5) for fresh
                // approach measurement post-hold.
                stats.low_agl_vs_min_fpm = None;
            } else if snap.bank_deg.abs() < 5.0 {
                let exit = stats.holding_exit_pending_since.get_or_insert(now);
                if (now - *exit).num_seconds() >= HOLDING_EXIT_DWELL_SECS {
                    let prev = stats
                        .previous_phase_before_holding
                        .unwrap_or(FlightPhase::Cruise);
                    next_phase = prev;
                    stats.holding_pending_since = None;
                    stats.holding_exit_pending_since = None;
                    stats.previous_phase_before_holding = None;
                }
            } else {
                stats.holding_exit_pending_since = None;
            }
        }
        FlightPhase::Descent => {
            // Approach starts when we're both low *and* still
            // descending. Without the V/S guard a brief AGL dip
            // during a mountain overflight (Alps, Rockies) wrongly
            // triggered Approach mid-cruise. The aircraft is on
            // approach when it's actually heading down toward the
            // destination, not when it happened to skim a peak.
            if snap.altitude_agl_ft < 5000.0 && snap.vertical_speed_fpm < 0.0 {
                next_phase = FlightPhase::Approach;
                // v0.5.5: reset running peak-descent tracker on Approach
                // entry. Stays alive across Approach → Final transitions
                // (we keep tracking through final). Only resets here so a
                // go-around (which routes Final → Climb → Approach again)
                // gets a clean per-attempt picture.
                stats.low_agl_vs_min_fpm = None;
            }
        }
        FlightPhase::Approach => {
            // Collect approach-stability samples on every tick. V/S
            // and bank stddev over this window become the "Approach
            // Stability" sub-score in the Landing Analyzer.
            push_approach_sample(&mut stats, snap);
            // Track lowest AGL during approach for go-around detection.
            update_lowest_approach_agl(&mut stats, snap);

            // PMDG capture (Phase 5.6): on Approach the pilot has
            // typically dialed in landing flaps + VREF in the FMC,
            // and chosen autobrake. Capture continuously through
            // approach so a last-minute change still gets recorded.
            if let Some(p) = &snap.pmdg {
                if stats.pmdg_landing_flaps_planned.is_none() {
                    stats.pmdg_landing_flaps_planned = p.fmc_landing_flaps_deg;
                }
                if stats.pmdg_vref_at_landing.is_none() {
                    stats.pmdg_vref_at_landing = p.fmc_vref_kt;
                }
                // Autobrake: keep capturing until touchdown so we
                // record the LAST setting before landing.
                if !p.autobrake_label.is_empty() && p.autobrake_label != "?" {
                    stats.pmdg_autobrake_at_landing =
                        Some(p.autobrake_label.clone());
                }
            }

            if let Some(ga_phase) = check_go_around(&mut stats, snap, now) {
                next_phase = ga_phase;
            } else if snap.altitude_agl_ft < 700.0 {
                // 1500 ft AGL was too eager — pilots reported a 3 min
                // "Final" segment because most aircraft intercept the
                // ILS at that altitude and still have several miles to
                // run. Real-world Final starts ~700 ft AGL (FAF crossed
                // for non-precision, decision height area for ILS).
                next_phase = FlightPhase::Final;
            } else if check_holding_entry(&mut stats, snap, now) {
                // v0.5.11: Approach-level holding (low-altitude hold).
                // VATSIM ATC frequently issues "hold over WAYPOINT"
                // during inbound sequencing — aircraft circles at
                // ~5000 ft for 5-15 minutes. Without this we'd just
                // see endless Approach phase with no signal.
                stats.previous_phase_before_holding = Some(FlightPhase::Approach);
                next_phase = FlightPhase::Holding;
            }
        }
        FlightPhase::Final => {
            // Continue collecting approach-stability samples through
            // the final segment too. Lots of pilots stabilise late;
            // measuring only Approach would underweight the truly
            // important last 700 ft AGL.
            push_approach_sample(&mut stats, snap);
            update_lowest_approach_agl(&mut stats, snap);
            if let Some(ga_phase) = check_go_around(&mut stats, snap, now) {
                next_phase = ga_phase;
            }
            // Approach runway: snapshot whatever ATC currently has us
            // cleared for. May still change before touchdown, so we
            // refresh until wheels are down.
            if let Some(rw) = snap.selected_runway.as_ref().filter(|s| !s.is_empty()) {
                stats.approach_runway = Some(rw.clone());
            }
            if !was_on_ground && snap.on_ground {
                next_phase = FlightPhase::Landing;

                // CRITICAL: anchor the window around the ACTUAL
                // touchdown moment, not `now`. The position-streamer
                // ticks every 5 s during Landing phase, so the on-
                // ground edge is detected up to 5 s after the wheels
                // actually touched. The 30 Hz touchdown-sampler has
                // been faithfully recording samples in the meantime,
                // so the first on-ground sample in the buffer IS the
                // touchdown moment. Using `now` instead would give a
                // window over the rollout, where V/S is near zero —
                // exactly the "-48 fpm" bug.
                let actual_td_at = stats
                    .snapshot_buffer
                    .iter()
                    .find(|s| s.on_ground)
                    .map(|s| s.at)
                    .unwrap_or(now);
                stats.landing_at = Some(actual_td_at);

                // Snapshot the buffer entry AT the actual touchdown
                // moment. Used below for IAS / pitch / heading
                // capture so they reflect the real TD-instant values,
                // not the post-rollout numbers that `snap` carries
                // (current snap is up to 5 s late on the streamer
                // tick, time enough for a typical airliner to bleed
                // ~17 kt IAS and derotate the nose to negative pitch).
                // Live bug 2026-05-03: pilot at 135 kt touchdown saw
                // "IAS bei TD: 118 kt" + "pitch -4.9°" because we
                // were reading post-derotation values.
                let td_buf_sample = stats
                    .snapshot_buffer
                    .iter()
                    .find(|s| s.on_ground)
                    .cloned();

                // v0.5.12: SIM-AWARE touchdown capture — different
                // priority chains for MSFS vs X-Plane.
                //
                // The earlier v0.5.11 unified chain (AGL-Δ primary
                // for both sims) was a scope error — MSFS already
                // had a perfect source via the `PLANE TOUCHDOWN
                // NORMAL VELOCITY` SimVar (frame-accurate, latched
                // at the exact contact moment by the sim itself).
                // Routing MSFS through the AGL estimator with strict
                // AGL ≤ 5 guards rejected it (MSFS reports AGL ≈ 14 ft
                // even when on_ground=true — sim quirk), the chain
                // fell through to the X-Plane-style sampler which
                // captured a transient -1173 fpm spike from MSFS's
                // gear-contact rebound oscillation. Pilot's
                // 2026-05-07 LH595 DNAA→EDDF report: actual rate
                // ~-419/-560 fpm (Volanta + LHA + MSFS-latched),
                // we reported -1173. Bug class: cross-contamination
                // from a refactor that should've been X-Plane only.
                //
                // Clean architecture v0.5.12:
                //
                //   MSFS:
                //     1. snap.touchdown_vs_fpm (PRIMARY — latched
                //        SimVar, frame-accurate, gold standard)
                //     2. AGL-Δ estimator (sanity fallback if SimVar
                //        wasn't populated for some reason)
                //     3. Buffer scan
                //
                //   X-Plane:
                //     1. AGL-Δ estimator (PRIMARY — LandingRate-1
                //        algorithm, Volanta uses the same)
                //     2. sampler_touchdown_vs_fpm (FALLBACK — uses
                //        fnrml_gear edge from v0.4.4 sampler)
                //     3. Buffer scan
                //     4. low_agl_vs_min_fpm
                //
                // Both paths apply negative_only filtering.
                if let Some(sampler_at) = stats.sampler_touchdown_at {
                    stats.landing_at = Some(sampler_at);
                }
                let actual_td_at = stats.landing_at.unwrap_or(actual_td_at);

                let is_msfs = matches!(
                    snap.simulator,
                    Simulator::Msfs2020 | Simulator::Msfs2024
                );
                let is_xplane = matches!(
                    snap.simulator,
                    Simulator::XPlane11 | Simulator::XPlane12
                );

                // v0.5.13: AGL-derivative estimators run once.
                //
                //   * `agl_estimate_xp` — Lua-style 30-sample window
                //     (LandingRate-1 algorithm, Volanta-aligned).
                //     Used as PRIMARY for X-Plane.
                //   * `agl_estimate_msfs` — original time-tier window
                //     (multi-tier 750ms/1s/1.5s/2s/3s/12s with
                //     min-sample guards). Used as FALLBACK for MSFS
                //     when the latched SimVar is null. v0.5.12-validated
                //     against real pilot flights — leave alone.
                //
                // Both have identical AGL guards (TD ≤ 5 ft / on_ground
                // = true, window-start ≤ 250 ft) — they differ only
                // in WINDOW SIZING strategy. The Lua method adapts
                // to sample density automatically; the time-tier
                // method walks fixed time windows.
                let agl_estimate_xp =
                    estimate_xplane_touchdown_vs_lua_style(
                        &stats.snapshot_buffer,
                        actual_td_at,
                    );
                let agl_estimate_msfs = estimate_xplane_touchdown_vs_from_agl(
                    &stats.snapshot_buffer,
                    actual_td_at,
                );

                // Tight buffer-window scan (used by both paths as a
                // fallback — AGL ≤ 250 ft filter prevents pre-flare
                // contamination).
                let half_window =
                    chrono::Duration::milliseconds(TOUCHDOWN_VS_WINDOW_MS / 2);
                let vs_window_start = actual_td_at - half_window;
                let vs_window_end = actual_td_at + half_window;
                let buffered_vs_min_raw: f32 = stats
                    .snapshot_buffer
                    .iter()
                    .filter(|s| s.at >= vs_window_start && s.at <= vs_window_end)
                    .filter(|s| s.agl_ft <= TD_AGL_MAX_AT_WINDOW_START_FT)
                    .map(|s| s.vs_fpm)
                    .fold(f32::INFINITY, f32::min);
                let buffered_vs_min = if buffered_vs_min_raw.is_finite() {
                    Some(buffered_vs_min_raw)
                } else {
                    None
                };

                let touchdown_vs = if is_msfs {
                    // MSFS: priority chain (v0.5.12 — validated against
                    // 11 real pilot flights, Pete + Michael):
                    //   1. Latched SimVar (PLANE TOUCHDOWN NORMAL VELOCITY)
                    //   2. Time-tier AGL-Δ (UNCHANGED, GEES-aligned)
                    //   3. Buffered VS-min — last resort
                    //
                    // EXPLICITLY NOT used for MSFS:
                    //   * sampler_touchdown_vs_fpm — gear-contact
                    //     rebound spike contamination
                    //   * low_agl_vs_min_fpm — same risk
                    //   * Lua-style 30-sample estimator — that's
                    //     X-Plane only by design
                    let result = negative_only(snap.touchdown_vs_fpm)
                        .or_else(|| agl_estimate_msfs.map(|e| e.fpm))
                        .or_else(|| negative_only(buffered_vs_min))
                        .unwrap_or(0.0);
                    tracing::info!(
                        sim = "msfs",
                        latched = ?snap.touchdown_vs_fpm,
                        agl_estimate = ?agl_estimate_msfs.map(|e| e.fpm),
                        buffer_min = ?buffered_vs_min,
                        chosen = result,
                        "touchdown VS captured (MSFS path — time-tier estimator + sampler bypassed)"
                    );
                    result
                } else if is_xplane {
                    // X-Plane (v0.5.13): Lua-style 30-sample AGL-Δ is
                    // PRIMARY (LandingRate-1 algorithm, Volanta-aligned).
                    // Adapts to sim framerate naturally — high-fps gets
                    // tight 0.5 s windows, low-fps gets wider 2-3 s
                    // windows. No fixed time-tier brittleness.
                    let result = agl_estimate_xp
                        .map(|e| e.fpm)
                        .or_else(|| negative_only(stats.sampler_touchdown_vs_fpm))
                        .or_else(|| negative_only(buffered_vs_min))
                        .or_else(|| negative_only(stats.low_agl_vs_min_fpm))
                        .unwrap_or(0.0);
                    if let Some(ref est) = agl_estimate_xp {
                        tracing::info!(
                            sim = "xplane",
                            agl_fpm = est.fpm,
                            source = est.source,
                            window_ms = est.window_ms,
                            sample_count = est.sample_count,
                            chosen = result,
                            "touchdown VS captured (X-Plane Lua-style 30-sample primary)"
                        );
                    } else {
                        tracing::info!(
                            sim = "xplane",
                            sampler_vs = ?stats.sampler_touchdown_vs_fpm,
                            buffer_min = ?buffered_vs_min,
                            low_agl_min = ?stats.low_agl_vs_min_fpm,
                            chosen = result,
                            "touchdown VS captured (X-Plane path, AGL-Δ unavailable)"
                        );
                    }
                    result
                } else {
                    // Unknown sim — generic fallback, preserves prior
                    // behaviour for edge cases (Other/sim_set_kind=Off).
                    // Uses time-tier MSFS-side estimator since it's
                    // the more conservative of the two.
                    negative_only(snap.touchdown_vs_fpm)
                        .or_else(|| agl_estimate_msfs.map(|e| e.fpm))
                        .or_else(|| negative_only(stats.sampler_touchdown_vs_fpm))
                        .or_else(|| negative_only(buffered_vs_min))
                        .unwrap_or(0.0)
                };

                if touchdown_vs == 0.0 {
                    tracing::warn!(
                        sim = ?snap.simulator,
                        sampler_vs = ?stats.sampler_touchdown_vs_fpm,
                        latched = ?snap.touchdown_vs_fpm,
                        buffer_min = ?buffered_vs_min,
                        low_agl_min = ?stats.low_agl_vs_min_fpm,
                        "touchdown VS — all sources failed or were positive, defaulting to 0"
                    );
                }

                // ─── v0.5.23 Touchdown-Forensik-Capture ──────────────
                //
                // Spiegel der Priority-Chain oben — bestimmt welcher Pfad
                // den finalen `touchdown_vs` lieferte und merkt sich die
                // Werte ALLER Pfade zum Vergleich. Der Streamer baut
                // damit den TouchdownPayload und schickt es an aeroacars-
                // live damit der VA-Owner Algorithmen-Disagreements
                // forensisch auswerten kann.
                let vs_source: &'static str = if is_msfs {
                    if negative_only(snap.touchdown_vs_fpm).is_some() {
                        "msfs_simvar_latched"
                    } else if agl_estimate_msfs.is_some() {
                        "agl_estimate_msfs"
                    } else if negative_only(buffered_vs_min).is_some() {
                        "buffer_min"
                    } else {
                        "fallback_zero"
                    }
                } else if is_xplane {
                    if agl_estimate_xp.is_some() {
                        "agl_estimate_xp"
                    } else if negative_only(stats.sampler_touchdown_vs_fpm).is_some() {
                        "sampler_gear_force"
                    } else if negative_only(buffered_vs_min).is_some() {
                        "buffer_min"
                    } else if negative_only(stats.low_agl_vs_min_fpm).is_some() {
                        "low_agl_vs_min"
                    } else {
                        "fallback_zero"
                    }
                } else {
                    "other_fallback"
                };
                stats.landing_simulator = Some(if is_msfs {
                    "msfs"
                } else if is_xplane {
                    "xplane"
                } else {
                    "other"
                });
                stats.landing_vs_estimate_xp_fpm =
                    agl_estimate_xp.map(|e| e.fpm.round() as i32);
                stats.landing_vs_estimate_msfs_fpm =
                    agl_estimate_msfs.map(|e| e.fpm.round() as i32);
                stats.landing_vs_source = Some(vs_source);
                // gear_force_peak_n: nur X-Plane liefert den Wert ueber
                // den Sampler-Pfad. snap.gear_normal_force_n im Touchdown-
                // Frame ist die direkteste Quelle.
                stats.landing_gear_force_peak_n = if is_xplane {
                    snap.gear_normal_force_n
                } else {
                    None
                };
                // window_ms + sample_count: nur fuer den Pfad der wirklich
                // gewann (= Lua bei XP, Time-Tier bei MSFS). Fallback-Pfade
                // haben keine Window-Metrik.
                if vs_source == "agl_estimate_xp" {
                    if let Some(est) = agl_estimate_xp {
                        stats.landing_estimate_window_ms = Some(est.window_ms as i32);
                        stats.landing_estimate_sample_count = Some(est.sample_count as u32);
                    }
                } else if vs_source == "agl_estimate_msfs" {
                    if let Some(est) = agl_estimate_msfs {
                        stats.landing_estimate_window_ms = Some(est.window_ms as i32);
                        stats.landing_estimate_sample_count = Some(est.sample_count as u32);
                    }
                }

                stats.landing_rate_fpm = Some(touchdown_vs);
                stats.landing_peak_vs_fpm = Some(touchdown_vs);

                // G capture: prefer sampler-side (v0.4.4) wenn verfügbar.
                // Sonst peak G from full buffer (5 s of pre-touchdown
                // history). Subsequent ticks in the Landing arm refine
                // this further until TOUCHDOWN_G_WINDOW_MS elapses.
                let touchdown_g = if let Some(sampler_g) = stats.sampler_touchdown_g_force {
                    sampler_g
                } else {
                    let buffered_g_peak: f32 = stats
                        .snapshot_buffer
                        .iter()
                        .map(|s| s.g_force)
                        .fold(0.0, f32::max);
                    buffered_g_peak.max(snap.g_force)
                };
                stats.landing_g_force = Some(touchdown_g);
                stats.landing_peak_g_force = Some(touchdown_g);

                // IAS / pitch / heading: prefer the buffered sample
                // at the actual TD moment (see `td_buf_sample` above).
                // Fall back chain — buffered sample → MSFS-latched
                // touchdown SimVars → live snap. The `snap` fallback
                // is the worst case (post-rollout values), kept only
                // for resumed flights where the buffer is empty.
                stats.landing_pitch_deg = Some(
                    snap.touchdown_pitch_deg
                        .or_else(|| td_buf_sample.as_ref().map(|s| s.pitch_deg))
                        .unwrap_or(snap.pitch_deg),
                );
                // v0.5.16: bank at touchdown for wing-strike maintenance
                // detection. MSFS latches bank too via the touchdown
                // SimVar set; X-Plane via the buffered sample. Live
                // snap as last-resort fallback (resumed flights).
                stats.landing_bank_deg = Some(
                    snap.touchdown_bank_deg
                        .or_else(|| td_buf_sample.as_ref().map(|s| s.bank_deg))
                        .unwrap_or(snap.bank_deg),
                );
                // Heading capture: MSFS gives us a magnetic-heading
                // touchdown latch; the buffer carries true-heading
                // only. When we fall back to the buffer we approximate
                // magnetic from true via the live magvar delta
                // (true−magnetic at the streamer tick) — small error
                // since magvar doesn't change in 5 s of rollout.
                stats.landing_heading_deg = Some(
                    snap.touchdown_heading_mag_deg
                        .or_else(|| {
                            td_buf_sample.as_ref().map(|s| {
                                let magvar =
                                    snap.heading_deg_true - snap.heading_deg_magnetic;
                                s.heading_true_deg - magvar
                            })
                        })
                        .unwrap_or(snap.heading_deg_magnetic),
                );
                stats.landing_speed_kt = Some(
                    td_buf_sample
                        .as_ref()
                        .map(|s| s.indicated_airspeed_kt)
                        .unwrap_or(snap.indicated_airspeed_kt),
                );
                // v0.5.17: groundspeed at touchdown (parallel to IAS).
                // Buffered sample preferred — live snap is the post-
                // rollout fallback for resumed flights only.
                stats.landing_groundspeed_kt = Some(
                    td_buf_sample
                        .as_ref()
                        .map(|s| s.groundspeed_kt)
                        .unwrap_or(snap.groundspeed_kt),
                );
                stats.landing_fuel_kg = Some(snap.fuel_total_kg);

                // ---- Tier 2/3 BeatMyLanding-aligned extras ----

                // Position + true heading at the touchdown edge — used
                // for runway lookup and disambiguation between parallel
                // runway pairs (08L/08R) via heading match.
                //
                // Pulls from the buffered TD-moment sample (same
                // reasoning as the IAS / pitch / heading capture
                // above): `snap` is up to 5 s late on the streamer
                // tick, time enough for a typical airliner to roll
                // 50–100 m down the runway and drift left/right via
                // nose-wheel correction — the centerline offset
                // sign would invert if the pilot touched down 3 m
                // left and then drifted through center to right of
                // centerline during the rollout. Live bug 2026-05-03
                // (MKJS RWY 07: pilot reported 3 m left, app showed
                // 2.7 m right). Buffer fallback to `snap` only when
                // the buffer is empty (resumed flight).
                stats.landing_lat = Some(
                    td_buf_sample
                        .as_ref()
                        .map(|s| s.lat)
                        .unwrap_or(snap.lat),
                );
                stats.landing_lon = Some(
                    td_buf_sample
                        .as_ref()
                        .map(|s| s.lon)
                        .unwrap_or(snap.lon),
                );
                stats.landing_heading_true_deg = Some(
                    td_buf_sample
                        .as_ref()
                        .map(|s| s.heading_true_deg)
                        .unwrap_or(snap.heading_deg_true),
                );

                // Touchdown profile — full buffer reframed to ms-relative
                // timestamps. PIREP renders this as a tiny V/S curve so
                // the pilot can see the flare shape after the fact.
                stats.touchdown_profile = stats
                    .snapshot_buffer
                    .iter()
                    .map(|s| TouchdownProfilePoint {
                        t_ms: (s.at - actual_td_at).num_milliseconds() as i32,
                        vs_fpm: s.vs_fpm,
                        g_force: s.g_force,
                        agl_ft: s.agl_ft,
                        on_ground: s.on_ground,
                        heading_true_deg: s.heading_true_deg,
                        groundspeed_kt: s.groundspeed_kt,
                        indicated_airspeed_kt: s.indicated_airspeed_kt,
                        pitch_deg: s.pitch_deg,
                        bank_deg: s.bank_deg,
                    })
                    .collect();

                // Sideslip / crab angle at touchdown — GEES-aligned:
                //   sideslip = atan2(VEL_BODY_X, VEL_BODY_Z) × 180/π
                // VEL_BODY_X is the right component of velocity in the
                // aircraft's body frame; VEL_BODY_Z is the forward
                // component. The ratio's arctangent IS the angle
                // between the velocity vector and the nose. Native
                // and exact — much better than reconstructing track
                // from successive lat/lon. None when the SimVar isn't
                // wired or the aircraft is essentially stopped (Z<3 fps
                // ≈ 1.8 kt, vector noise dominates below that).
                stats.touchdown_sideslip_deg = match (
                    snap.velocity_body_x_fps,
                    snap.velocity_body_z_fps,
                ) {
                    (Some(x), Some(z)) if z.abs() > 3.0 => {
                        Some(x.atan2(z).to_degrees())
                    }
                    _ => None,
                };

                // Headwind / crosswind components at the touchdown
                // frame, native from `AIRCRAFT WIND X/Z`. Z is signed
                // such that positive = tailwind (wind blowing into the
                // aircraft from behind), so headwind = -Z. X positive
                // = wind from the right side.
                stats.landing_headwind_kt = snap.aircraft_wind_z_kt.map(|z| -z);
                stats.landing_crosswind_kt = snap.aircraft_wind_x_kt;

                // Runway correlation. None when no runway lies within
                // ~3 km of the touchdown coordinate. Sits next to
                // `approach_runway` (from `ATC RUNWAY SELECTED`) — the
                // runway_match value is the authoritative one because
                // it's derived from where the wheels actually touched,
                // not the ATC clearance. Uses the buffered TD-moment
                // lat/lon/heading (same reasoning as `landing_lat`
                // capture above) so the centerline-offset sign and
                // along-track distance reflect the real touchdown
                // point, not where the aircraft has rolled to during
                // the streamer's 5 s edge-detection lag.
                let (rw_lat, rw_lon, rw_hdg_true) = td_buf_sample
                    .as_ref()
                    .map(|s| (s.lat, s.lon, s.heading_true_deg))
                    .unwrap_or((snap.lat, snap.lon, snap.heading_deg_true));
                stats.runway_match =
                    runway::lookup_runway(rw_lat, rw_lon, rw_hdg_true);
                if let Some(rw) = &stats.runway_match {
                    tracing::info!(
                        airport = %rw.airport_ident,
                        runway = %rw.runway_ident,
                        centerline_m = rw.centerline_distance_m,
                        from_threshold_ft = rw.touchdown_distance_from_threshold_ft,
                        side = %rw.side,
                        "touchdown correlated to runway"
                    );
                }

                // Reset bounce state for the new analyzer window.
                stats.bounce_armed_above_threshold = false;
                stats.bounce_count = 0;

                // ---- Landing Analyzer (Stage 1) ----

                // Compute approach-stability stddev from the buffer
                // accumulated during Approach + Final. None when we
                // have fewer than 3 samples (resumed flight,
                // ultra-fast approach).
                //
                // v0.5.25: PLUS Stable-Approach-Gate-konforme v2-
                // Auswertung (1000-ft-AGL-Window, Glide-Slope-Deviation,
                // Vector-Window-Filter, Late-RWY-Change-Detection).
                // Beide Werte werden mit-publiziert — pre-v0.5.25-
                // Konsumenten weiter mit den σ-Werten, neue UI nutzt
                // approach_vs_deviation_fpm fuer das echte Gate-Maß.
                let (vs_sd, bank_sd) = compute_approach_stddev(&stats.approach_buffer);
                stats.approach_vs_stddev_fpm = vs_sd;
                stats.approach_bank_stddev_deg = bank_sd;
                let stab_v2 = compute_approach_stability_v2(
                    &stats.approach_buffer,
                    stats.arr_airport_elevation_ft,
                );
                stats.approach_vs_deviation_fpm = stab_v2.vs_deviation_fpm;
                stats.approach_max_vs_deviation_below_500_fpm = stab_v2.max_vs_deviation_below_500_fpm;
                stats.approach_bank_stddev_filtered_deg = stab_v2.bank_stddev_filtered_deg;
                stats.approach_runway_changed_late = stab_v2.runway_changed_late;
                stats.approach_stable_at_gate = stab_v2.stable_at_gate;
                stats.approach_vs_jerk_fpm = stab_v2.vs_jerk_fpm;
                stats.approach_ias_stddev_kt = stab_v2.ias_stddev_kt;
                stats.approach_excessive_sink = stab_v2.excessive_sink;
                stats.approach_stable_config = stab_v2.stable_config;
                stats.approach_used_hat = stab_v2.used_hat;
                stats.approach_window_sample_count = Some(stab_v2.window_sample_count);
                tracing::info!(
                    pirep_id = %flight.pirep_id,
                    vs_dev_fpm = ?stab_v2.vs_deviation_fpm,
                    max_vs_dev_below_500 = ?stab_v2.max_vs_deviation_below_500_fpm,
                    bank_sd_filtered = ?stab_v2.bank_stddev_filtered_deg,
                    rwy_changed_late = stab_v2.runway_changed_late,
                    stable_at_gate = ?stab_v2.stable_at_gate,
                    samples = stab_v2.window_sample_count,
                    "approach stability v2 computed"
                );

                // Reset rollout tracking. The Landing-phase arm below
                // accumulates haversine distance from `(landing_lat,
                // landing_lon)` until groundspeed drops below
                // ROLLOUT_STOP_GS_KT, then sets `rollout_finalized`.
                stats.rollout_distance_m = Some(0.0);
                stats.rollout_finalized = false;
                stats.rollout_last_lat = Some(snap.lat);
                stats.rollout_last_lon = Some(snap.lon);
                // Prefer TOTAL WEIGHT (fuel + payload + empty); only
                // fall back to ZFW + fuel if the SimVar is missing.
                // Fenix returns 0 for both → snapshot mapping converted
                // it to None, so the field stays unset and the PIREP
                // filter drops it.
                if let Some(tw) = snap.total_weight_kg {
                    stats.landing_weight_kg = Some(tw as f64);
                } else {
                    let zfw = snap.zfw_kg.unwrap_or(0.0);
                    let weight = zfw as f64 + snap.fuel_total_kg as f64;
                    if weight > 0.0 {
                        stats.landing_weight_kg = Some(weight);
                    }
                }
            }
        }
        FlightPhase::Landing => {
            // Rollout-distance accumulator: from `landing_lat/lon`
            // until groundspeed drops below ROLLOUT_STOP_GS_KT, sum
            // haversine deltas tick-by-tick. Survives a Tauri restart
            // because we persist (last_lat, last_lon, distance,
            // finalized) in PersistedFlightStats.
            if !stats.rollout_finalized {
                if let (Some(prev_lat), Some(prev_lon)) =
                    (stats.rollout_last_lat, stats.rollout_last_lon)
                {
                    let delta = ::geo::distance_m(prev_lat, prev_lon, snap.lat, snap.lon);
                    stats.rollout_distance_m =
                        Some(stats.rollout_distance_m.unwrap_or(0.0) + delta);
                }
                stats.rollout_last_lat = Some(snap.lat);
                stats.rollout_last_lon = Some(snap.lon);

                // Three independent finalisation triggers — whichever
                // fires first wins. See the constants near the top of
                // this file for the rationale on each threshold.
                let exit_speed_reached =
                    snap.groundspeed_kt < ROLLOUT_EXIT_GS_KT && snap.on_ground;
                let full_stop = snap.groundspeed_kt < ROLLOUT_STOP_GS_KT && snap.on_ground;
                let turned_off_runway = match stats.landing_heading_true_deg {
                    Some(td_heading) => {
                        // Wrap-around-safe signed delta in (-180, 180].
                        let mut diff = snap.heading_deg_true - td_heading;
                        while diff > 180.0 {
                            diff -= 360.0;
                        }
                        while diff <= -180.0 {
                            diff += 360.0;
                        }
                        diff.abs() > ROLLOUT_HEADING_DEVIATION_DEG
                    }
                    None => false,
                };

                if exit_speed_reached || full_stop || turned_off_runway {
                    stats.rollout_finalized = true;
                    let reason = if turned_off_runway {
                        "turned off centerline"
                    } else if full_stop {
                        "full stop"
                    } else {
                        "exit speed reached"
                    };
                    tracing::info!(
                        meters = stats.rollout_distance_m.unwrap_or(0.0),
                        gs_kt = snap.groundspeed_kt,
                        reason,
                        "rollout finalised"
                    );
                }
            }

            // Touchdown analyzer windows (BeatMyLanding-aligned):
            //   * Peak G refined for TOUCHDOWN_G_WINDOW_MS (1500 ms)
            //   * Peak |V/S| refined for TOUCHDOWN_WINDOW_SECS (5 s) —
            //     wider to catch a hard secondary contact after a bounce.
            //   * Bounces tracked via AGL-edge for BOUNCE_WINDOW_SECS (8 s).
            //   * Score finalised once the bounce window closes.
            if let Some(touchdown) = stats.landing_at {
                let elapsed_ms = (now - touchdown).num_milliseconds();
                let elapsed_secs = elapsed_ms / 1_000;
                let in_vs_window = elapsed_secs <= TOUCHDOWN_WINDOW_SECS;
                let in_g_window = elapsed_ms <= TOUCHDOWN_G_WINDOW_MS;
                let in_bounce_window = elapsed_secs <= BOUNCE_WINDOW_SECS;

                if in_vs_window {
                    let peak_vs = stats.landing_peak_vs_fpm.unwrap_or(0.0);
                    if snap.vertical_speed_fpm < peak_vs {
                        stats.landing_peak_vs_fpm = Some(snap.vertical_speed_fpm);
                    }
                }
                if in_g_window {
                    let peak_g = stats.landing_peak_g_force.unwrap_or(0.0);
                    if snap.g_force > peak_g {
                        stats.landing_peak_g_force = Some(snap.g_force);
                    }
                }

                // Bounce detection — AGL-edge state machine. Replaces
                // the noisy on-ground flicker we used pre-Tier-1, which
                // tripped on gear-strut oscillation and over-counted
                // bounces on a clean landing.
                //
                // Arm: AGL crosses up through BOUNCE_AGL_THRESHOLD_FT
                //      (35 ft, BeatMyLanding's `BounceRadioAltThresholdFeet`).
                // Fire: AGL drops back below BOUNCE_AGL_RETURN_FT
                //      (5 ft, `BounceRadioAltReturnFeet`).
                // Both must happen inside BOUNCE_WINDOW_SECS for a bounce
                // to count — past that we assume the pilot did a touch-
                // and-go or got airborne again deliberately.
                if in_bounce_window {
                    if !stats.bounce_armed_above_threshold
                        && snap.altitude_agl_ft > BOUNCE_AGL_THRESHOLD_FT
                    {
                        stats.bounce_armed_above_threshold = true;
                    } else if stats.bounce_armed_above_threshold
                        && snap.altitude_agl_ft < BOUNCE_AGL_RETURN_FT
                    {
                        stats.bounce_count = stats.bounce_count.saturating_add(1);
                        stats.bounce_armed_above_threshold = false;
                        tracing::info!(
                            count = stats.bounce_count,
                            agl_ft = snap.altitude_agl_ft,
                            elapsed_ms,
                            "bounce counted (AGL re-entered ground band)"
                        );
                    }
                }

                if !in_bounce_window && stats.landing_score.is_none() {
                    // All windows have closed — finalise the score once.
                    let peak_vs = stats.landing_peak_vs_fpm.unwrap_or(0.0);
                    let peak_g = stats.landing_peak_g_force.unwrap_or(0.0);
                    let score = LandingScore::classify(peak_vs, peak_g, stats.bounce_count);
                    stats.landing_score = Some(score);
                }

                // Touch-and-Go classifier — runs in parallel with the
                // bounce/score windows. Looks for a sustained climb
                // back above TOUCH_AND_GO_AGL_THRESHOLD_FT within
                // TOUCH_AND_GO_WATCH_SECS of the touchdown. If the
                // pilot really lifted off and is climbing out (= T&G),
                // we record the touchdown as kind=TouchAndGo, reset
                // the landing window for the next touchdown, and bump
                // the FSM back to Climb so subsequent phase
                // transitions work normally on the next descent.
                let tg_in_window = elapsed_secs <= TOUCH_AND_GO_WATCH_SECS;
                if tg_in_window {
                    let agl = snap.altitude_agl_ft as f32;
                    let conds_met = agl > TOUCH_AND_GO_AGL_THRESHOLD_FT
                        && !snap.on_ground
                        && snap.engines_running > 0;
                    if conds_met {
                        let pending = stats.touch_and_go_pending_since.get_or_insert(now);
                        if (now - *pending).num_seconds() >= TOUCH_AND_GO_DWELL_SECS {
                            // CONFIRMED: this was a touch-and-go.
                            let event = TouchdownEvent {
                                timestamp: touchdown,
                                kind: TouchdownKind::TouchAndGo,
                                peak_vs_fpm: stats.landing_peak_vs_fpm.unwrap_or(0.0),
                                peak_g: stats.landing_peak_g_force.unwrap_or(0.0),
                                lat: stats.landing_lat.unwrap_or(snap.lat),
                                lon: stats.landing_lon.unwrap_or(snap.lon),
                                sub_bounces: stats.bounce_count,
                            };
                            stats.touchdown_events.push(event.clone());
                            let tg_count = stats
                                .touchdown_events
                                .iter()
                                .filter(|e| matches!(e.kind, TouchdownKind::TouchAndGo))
                                .count();
                            tracing::info!(
                                count = tg_count,
                                peak_vs = event.peak_vs_fpm,
                                peak_g = event.peak_g,
                                sub_bounces = event.sub_bounces,
                                "touch-and-go classified — resetting landing window"
                            );
                            stats.pending_acars_logs.push(format!(
                                "Touch-and-go #{} — V/S {:.0} fpm, G {:.2}",
                                tg_count, event.peak_vs_fpm, event.peak_g
                            ));
                            // Reset landing window so the NEXT touchdown
                            // gets a fresh score window. Bounce count
                            // resets too — T&Gs don't drag down the
                            // final-landing's score.
                            stats.landing_at = None;
                            stats.landing_peak_vs_fpm = None;
                            stats.landing_peak_g_force = None;
                            stats.landing_score = None;
                            stats.landing_score_announced = false;
                            stats.landing_lat = None;
                            stats.landing_lon = None;
                            stats.bounce_count = 0;
                            stats.bounce_armed_above_threshold = false;
                            stats.touch_and_go_pending_since = None;
                            // CRITICAL: also clear the GA tracker so the
                            // NEXT approach starts with a fresh AGL
                            // minimum. Without this, the just-recorded
                            // T&G's ~50 ft touchdown AGL becomes the
                            // floor against which the next GA detector
                            // compares — which means EVERY future
                            // approach would trivially exceed
                            // "lowest + 200 ft" the moment the pilot
                            // climbs above 250 ft AGL, hiding any real
                            // missed-approach behind an immediate
                            // false-positive trigger.
                            stats.lowest_agl_during_approach_ft = None;
                            stats.go_around_climb_pending_since = None;
                            // v0.5.11: also reset climb_peak_msl so
                            // the new Climb segment after T&G starts
                            // fresh. Without this, the prior pattern
                            // altitude (e.g. 2000 ft) stays as the
                            // peak — and the moment the aircraft
                            // descends 500+ ft below it for the next
                            // approach, the v0.5.10 low-altitude
                            // Climb→Descent trigger would fire
                            // immediately (lost_from_peak > 500
                            // already satisfied via stale data).
                            // Pattern flying with multiple T&Gs
                            // would oscillate between Climb and
                            // Descent on every traffic-pattern leg.
                            stats.climb_peak_msl = None;
                            // FSM: revert to Climb so the streamer's
                            // phase-change handler emits "Phase: Climb"
                            // and subsequent descent re-detection works.
                            next_phase = FlightPhase::Climb;
                        }
                    } else {
                        // Conditions broke (engines off, AGL dropped,
                        // back on ground) — restart the dwell timer.
                        stats.touch_and_go_pending_since = None;
                    }
                } else if stats.landing_score.is_some()
                    && stats
                        .touchdown_events
                        .last()
                        .map(|e| e.timestamp != touchdown)
                        .unwrap_or(true)
                {
                    // T&G watch window expired without a climb-back →
                    // this was a real (final) landing. Push the
                    // TouchdownEvent for the audit trail, exactly once.
                    let event = TouchdownEvent {
                        timestamp: touchdown,
                        kind: TouchdownKind::FinalLanding,
                        peak_vs_fpm: stats.landing_peak_vs_fpm.unwrap_or(0.0),
                        peak_g: stats.landing_peak_g_force.unwrap_or(0.0),
                        lat: stats.landing_lat.unwrap_or(snap.lat),
                        lon: stats.landing_lon.unwrap_or(snap.lon),
                        sub_bounces: stats.bounce_count,
                    };
                    stats.touchdown_events.push(event);
                    tracing::info!(
                        total_touchdowns = stats.touchdown_events.len(),
                        "final landing recorded as TouchdownEvent"
                    );
                }
            }
            if snap.groundspeed_kt < 30.0 && snap.on_ground {
                next_phase = FlightPhase::TaxiIn;
            }
        }
        FlightPhase::TaxiIn => {
            if snap.parking_brake && snap.groundspeed_kt < 1.0 && snap.on_ground {
                next_phase = FlightPhase::BlocksOn;
                stats.block_on_at = Some(now);
                // Capture the arrival stand the moment we settle at
                // the gate. MSFS only fills `parking_name` while the
                // aircraft is on a named stand, so this is the right
                // moment.
                if let Some(name) =
                    snap.parking_name.as_ref().filter(|s| !s.is_empty())
                {
                    let label = match snap.parking_number.as_ref() {
                        Some(num) if !num.is_empty() => format!("{name} {num}"),
                        _ => name.clone(),
                    };
                    stats.arr_gate = Some(label);
                }
            }
        }
        FlightPhase::BlocksOn => {
            // BlocksOn → Arrived once the pilot has actually shut
            // down: engines off + parking brake set + at least 30 s
            // since the wheels stopped. Real pilots routinely leave
            // engines running a minute or two after blocks-on for
            // cool-down / APU transition, so we don't flip to Arrived
            // the instant they hit the brake.
            if let Some(block_on) = stats.block_on_at {
                let settled_secs = (now - block_on).num_seconds();
                if settled_secs >= 30
                    && snap.engines_running == 0
                    && snap.parking_brake
                    && snap.on_ground
                {
                    next_phase = FlightPhase::Arrived;
                }
            }
        }
        FlightPhase::Arrived
        | FlightPhase::PirepSubmitted
        | FlightPhase::Preflight => {}
    }

    // Universal "we're done" fallback. The normal FSM chain assumes a
    // fixed-wing flight with a Pushback → Cruise → BlocksOn arc, and
    // a parking brake / engines-off shutdown sequence. None of that
    // applies cleanly to:
    //   * helicopters (no taxi-out / no parking brake / vertical
    //     takeoff and landing — the FSM gets stuck around TaxiOut)
    //   * very-short hops with no real cruise phase
    //   * engine-failure / emergency landings near the destination
    //   * aircraft profiles that don't wire `parking_brake` at all
    //
    // The escape hatch: once the aircraft has actually started
    // moving (block_off recorded), if it's now sitting on-ground
    // with engines off within 2 nmi of the arrival airport for at
    // least 30 s, we force `Arrived` regardless of prior phase.
    // Block_on_at gets back-filled here so the PIREP file logic
    // (which reads it for flight-time / chocks-on) still works.
    let already_done = matches!(
        next_phase,
        FlightPhase::Arrived | FlightPhase::PirepSubmitted
    );
    let pre_block_off = matches!(
        next_phase,
        FlightPhase::Preflight | FlightPhase::Boarding
    );
    // GATE: only run the universal Arrived-fallback (and the divert
    // detection inside it) if the aircraft has actually been airborne
    // at some point. Without this, GSX/sim ground-handling jitter at
    // the gate can stamp `block_off_at` (any motion >0.5 kt counts),
    // and 30 s later the fallback fires with phase=Arrived plus a
    // bogus divert hint pointing at the departure airport — exactly
    // the bug pilots reported with NKS 833 KFLL→MKJS where GSX wackeln
    // produced "you landed at KFLL, planned was MKJS, file as divert?".
    if !already_done && !pre_block_off && stats.block_off_at.is_some() && stats.was_airborne {
        let arr_pos = runway::airport_position(&flight.arr_airport);
        let dist_to_planned_nmi = arr_pos
            .map(|(la, lo)| runway::distance_m(snap.lat, snap.lon, la, lo) / 1852.0)
            .unwrap_or(f64::INFINITY);
        let near_planned = dist_to_planned_nmi <= ARRIVED_FALLBACK_RADIUS_NM;
        // We treat "no airport in local DB" as near_planned=true so we
        // don't block the file path for obscure ICAO codes — same as
        // the original fallback behaviour.
        let near_planned = near_planned || arr_pos.is_none();

        let conditions_basic = snap.on_ground && snap.engines_running == 0;
        if conditions_basic {
            let pending_at = stats.arrived_fallback_pending_since.get_or_insert(now);
            let elapsed = (now - *pending_at).num_seconds();
            if elapsed >= ARRIVED_FALLBACK_DWELL_SECS {
                // Two paths to Arrived from here:
                //
                //   1. near_planned    → normal arrival, original fallback behaviour
                //   2. far from planned → DIVERT — find nearest airport, populate
                //                          divert_hint so the cockpit can ask the
                //                          pilot to confirm the actual destination
                //                          and file with the correct arr_airport_id
                let mut detected_hint: Option<DivertHint> = None;
                if !near_planned && dist_to_planned_nmi >= DIVERT_DETECT_RADIUS_NM {
                    let nearby = runway::find_nearest_airports(
                        snap.lat,
                        snap.lon,
                        DIVERT_NEAREST_SEARCH_RADIUS_NM * 1852.0,
                        1,
                    );
                    // Only count as "at airport X" when the nearest
                    // runway threshold is within the same on-airport
                    // tolerance we use for the planned-arrival check.
                    let nearest_icao = nearby
                        .into_iter()
                        .next()
                        .filter(|na| {
                            na.distance_m / 1852.0 <= ARRIVED_FALLBACK_RADIUS_NM
                        })
                        .map(|na| na.icao);
                    let alt_match = nearest_icao
                        .as_deref()
                        .zip(stats.planned_alternate.as_deref())
                        .map(|(a, b)| a.eq_ignore_ascii_case(b))
                        .unwrap_or(false);
                    let kind = if alt_match {
                        "alternate"
                    } else if nearest_icao.is_some() {
                        "nearest"
                    } else {
                        "unknown"
                    };
                    detected_hint = Some(DivertHint {
                        actual_icao: nearest_icao,
                        planned_arr_icao: flight.arr_airport.clone(),
                        planned_alt_icao: stats.planned_alternate.clone(),
                        distance_to_planned_nmi: dist_to_planned_nmi,
                        kind,
                    });
                    tracing::info!(
                        planned = %flight.arr_airport,
                        actual = ?detected_hint.as_ref().and_then(|h| h.actual_icao.as_deref()),
                        dist_nmi = dist_to_planned_nmi,
                        kind,
                        "divert detected"
                    );
                }

                if near_planned || detected_hint.is_some() {
                    tracing::info!(
                        prev_phase = ?prev_phase,
                        elapsed,
                        near_planned,
                        diverted = detected_hint.is_some(),
                        "Arrived fallback fired — forcing FSM to Arrived"
                    );
                    next_phase = FlightPhase::Arrived;
                    if stats.block_on_at.is_none() {
                        stats.block_on_at = Some(now);
                    }
                    // Helicopters often don't fire a touchdown event the
                    // way the analyzer expects. If we never recorded
                    // landing_at, stamp it now so the file body has a
                    // sensible flight_time.
                    //
                    // v0.5.4 enhancement: prefer the sampler's actual
                    // touchdown timestamp + VS / G if it captured the
                    // edge. The sampler runs at 50 Hz independently of
                    // the FSM, so even when we skipped Final → Landing
                    // (e.g. the v0.5.3 low-altitude-cruise FSM bug),
                    // the touchdown moment is still recorded in
                    // `stats.sampler_touchdown_*`. Without this
                    // rescue path the file_body would have null VS
                    // and a `now`-anchored landing_at, losing the
                    // landing-rate data entirely.
                    if stats.landing_at.is_none() && stats.takeoff_at.is_some() {
                        if let Some(sampler_at) = stats.sampler_touchdown_at {
                            stats.landing_at = Some(sampler_at);
                            // Also copy VS / G / rate so the LandingRecord
                            // has real touchdown values, not nulls. Without
                            // these, the dashboard shows "0 fpm / no rating"
                            // for any flight that bypassed Final → Landing.
                            if let Some(vs) = stats.sampler_touchdown_vs_fpm {
                                if stats.landing_rate_fpm.is_none() {
                                    stats.landing_rate_fpm = Some(vs);
                                }
                                if stats.landing_peak_vs_fpm.is_none() {
                                    stats.landing_peak_vs_fpm = Some(vs);
                                }
                            }
                            if let Some(g) = stats.sampler_touchdown_g_force {
                                if stats.landing_g_force.is_none() {
                                    stats.landing_g_force = Some(g);
                                }
                                if stats.landing_peak_g_force.is_none() {
                                    stats.landing_peak_g_force = Some(g);
                                }
                            }
                            tracing::info!(
                                "Arrived fallback rescued touchdown from sampler: \
                                 vs={:?} g={:?}",
                                stats.sampler_touchdown_vs_fpm,
                                stats.sampler_touchdown_g_force
                            );
                        } else {
                            stats.landing_at = Some(now);
                        }
                    }
                    if let Some(hint) = detected_hint {
                        stats.divert_hint = Some(hint);
                    }
                }
            }
        } else {
            stats.arrived_fallback_pending_since = None;
        }
    } else {
        stats.arrived_fallback_pending_since = None;
    }

    stats.was_on_ground = Some(snap.on_ground);
    stats.was_parking_brake = Some(snap.parking_brake);

    if next_phase != prev_phase {
        stats.phase = next_phase;
        stats.transitions.push((now, next_phase));
        Some(next_phase)
    } else {
        None
    }
}

/// Build the body for an in-flight `POST /api/pireps/{id}/update` call.
/// Used both by the periodic heartbeat (every `HEARTBEAT_INTERVAL`) and by
/// phase-change callbacks. Sends monotonically growing `flight_time` and
/// `distance` so Eloquent always sees the model as dirty and bumps
/// `pireps.updated_at`, which is what the cleanup cron checks.
fn build_heartbeat_body(
    snap: &SimSnapshot,
    stats: &FlightStats,
    current_phase: FlightPhase,
) -> UpdateBody {
    let now = Utc::now();
    let flight_time_secs = match (stats.takeoff_at, stats.landing_at) {
        (Some(t), Some(l)) if l > t => (l - t).num_seconds().max(0) as i32,
        (Some(t), None) => (now - t).num_seconds().max(0) as i32,
        // Pre-takeoff: fall back to block-off so cancellation cron sees
        // movement during the boarding/taxi period too.
        _ => stats
            .block_off_at
            .map(|b| (now - b).num_seconds().max(0) as i32)
            .unwrap_or(0),
    };
    // Same fuel arithmetic as the file body: block - remaining, in pounds.
    // Round to whole kg before lb conversion so the live-map / dashboard
    // shows clean integer kg values (no `5890.29 kg` artefacts from the
    // round-trip).
    let fuel_used = match (stats.block_fuel_kg, stats.last_fuel_kg) {
        (Some(b), Some(c)) if b > c => Some(((b - c) as f64).round() * KG_TO_LB),
        _ => None,
    };
    // Prefer the peak altitude observed (matches the FILE-time `level`),
    // but fall back to the current MSL altitude during early climb so the
    // live map shows something sensible before peak gets captured.
    let level = stats
        .peak_altitude_ft
        .map(|ft| ((ft / 100.0).round() * 100.0) as i32)
        .or_else(|| {
            if snap.altitude_msl_ft > 100.0 {
                Some(((snap.altitude_msl_ft / 100.0).round() * 100.0) as i32)
            } else {
                None
            }
        })
        .map(|v| v.max(0));
    // phpVMS' `Pirep::progress_percent` uses PHP's `empty()` to gate
    // the distance/upper_bound division. `empty(0)` AND `empty(null)`
    // are BOTH true in PHP — so the v0.3.0 fix (send `None` when
    // distance < 0.5 nm) never worked: phpVMS still saw null, fell
    // back to upper_bound=1, computed 1/1 = 100 % progress, and
    // showed "Geflogene Route: 100%" during Boarding. Live bug
    // confirmed 2026-05-05 (pilot Michel D. on YBBN→NWWW).
    //
    // Fix: send a tiny non-zero floor (0.001 nm) until real distance
    // accumulates. `empty(0.001)` is false → PHP runs the real
    // division → 0.001 / planned_distance ≈ 0.00..% → displayed as
    // 0 % during boarding, ramps up correctly once we move.
    let distance = Some(stats.distance_nm.max(0.001));
    let flight_time_min = if flight_time_secs >= 60 {
        Some(flight_time_secs / 60)
    } else {
        None
    };
    // v0.3.0: send block_fuel on every heartbeat so phpVMS' live page
    // can compute "Verbleibender Treibstoff = block_fuel − fuel_used"
    // correctly. Without this the missing column defaults to 0 and the
    // remaining-fuel display reads as "−<fuel_used>" (bug reported by
    // pilot on 2026-05-04: "−17008 kg" mid-cruise). Same kg→lb round-
    // trip as fuel_used so the dashboard shows clean integer values.
    let block_fuel = stats
        .block_fuel_kg
        .map(|b| (b as f64).round() * KG_TO_LB);
    UpdateBody {
        state: None,
        source: None,
        status: phase_to_status(current_phase).map(|s| s.to_string()),
        flight_time: flight_time_min,
        distance,
        fuel_used,
        block_fuel,
        level,
        source_name: Some(format!("AeroACARS/{}", env!("CARGO_PKG_VERSION"))),
        notes: None,
        // Always-fresh timestamp so Eloquent sees the row as dirty
        // even on heartbeats where no other field changed (boarding
        // / long stable cruise). See `UpdateBody::updated_at` doc.
        updated_at: Some(now.to_rfc3339()),
        // Heartbeat-only path — divert-finalize fields stay None.
        arr_airport_id: None,
        landing_rate: None,
        score: None,
        submitted_at: None,
        block_on_time: None,
    }
}

/// Handle a 404 from the live PIREP endpoints (positions / update). phpVMS
/// soft-deletes in-flight PIREPs after `acars.live_time` hours of no
/// `updated_at` bump (cron `RemoveExpiredLiveFlights`); when that happens,
/// every subsequent post returns 404 and the streamer would otherwise
/// retry-spam forever. This stops the streamer cleanly and surfaces the
/// situation to the user so they can re-file as a manual PIREP.
fn handle_remote_cancellation(app: &AppHandle, flight: &Arc<ActiveFlight>, source: &str) {
    if flight.cancelled_remotely.swap(true, Ordering::SeqCst) {
        // Already handled by an earlier 404 in the same cycle.
        return;
    }
    flight.stop.store(true, Ordering::Relaxed);
    log_activity_handle(
        app,
        ActivityLevel::Error,
        "PIREP wurde vom Server gecancelt".to_string(),
        Some(format!(
            "{source} antwortet mit 404 — phpVMS hat den laufenden PIREP entfernt (vermutlich Inaktivitäts-Timeout). Bitte als Manual-PIREP einreichen."
        )),
    );
    // Best-effort UI nudge — the dashboard listens for this and pops the
    // manual-PIREP banner. Failure to emit isn't fatal; the activity-log
    // entry above is the durable signal.
    let _ = tauri::Emitter::emit(
        app,
        "pirep_cancelled_remotely",
        serde_json::json!({
            "pirep_id": flight.pirep_id,
            "source": source,
        }),
    );
}

/// Build the custom-fields map sent in `POST /api/pireps/{id}/file`. Field
/// names follow the de-facto vmsACARS convention so VAs that already configured
/// fields for vmsACARS see them populate without any work.
fn build_pirep_fields(
    flight: &ActiveFlight,
    stats: &FlightStats,
) -> HashMap<String, String> {
    // Single-form custom-fields set: human-readable Title Case with
    // units. Earlier versions emitted EVERY field twice (Title Case +
    // snake_case "for SQL-sortable leaderboards") which made the
    // PIREP-detail list balloon to 60+ entries with obvious
    // duplicates ("Block Fuel: 9733 kg" right next to "block_fuel_kg:
    // 9733"). VA admins who want raw numbers can hit the database
    // directly — no point doubling everything in the UI.
    //
    // Notes block (separate PIREP field) still carries the prose
    // summary; THIS map carries the structured key-value pairs.
    let mut f: HashMap<String, String> = HashMap::new();

    f.insert(
        "Source".into(),
        format!("AeroACARS/{}", env!("CARGO_PKG_VERSION")),
    );

    // v0.3.0: Aircraft-Type / Name / Reg im PIREP-Custom-Field damit
    // VA-Admins auf der phpVMS-Detail-Seite ohne Lookup wissen was
    // geflogen wurde (vorher nur PMDG-Variant; jetzt für jeden
    // Aircraft-Typ aus dem Bid-Aircraft-Lookup).
    if !flight.aircraft_icao.is_empty() {
        let label = if !flight.aircraft_name.is_empty() {
            format!("{} ({})", flight.aircraft_icao, flight.aircraft_name)
        } else {
            flight.aircraft_icao.clone()
        };
        f.insert("Aircraft Type".into(), label);
    }
    if !flight.planned_registration.is_empty() {
        f.insert("Aircraft Reg".into(), flight.planned_registration.clone());
    }

    // ---- Times (HH:MM:SS UTC, glanceable) + durations ----
    fn fmt_time(t: &DateTime<Utc>) -> String {
        t.format("%H:%M:%S UTC").to_string()
    }
    if let Some(t) = stats.block_off_at {
        f.insert("Blocks Off Time".into(), fmt_time(&t));
    }
    if let Some(t) = stats.takeoff_at {
        f.insert("Takeoff Time".into(), fmt_time(&t));
    }
    if let Some(t) = stats.landing_at {
        f.insert("Landing Time".into(), fmt_time(&t));
    }
    if let Some(t) = stats.block_on_at {
        f.insert("Blocks On Time".into(), fmt_time(&t));
    }
    if let (Some(off), Some(on)) = (stats.block_off_at, stats.block_on_at) {
        f.insert(
            "Total Block Time".into(),
            humanize_duration_minutes((on - off).num_minutes()),
        );
    }
    if let (Some(takeoff), Some(landing)) = (stats.takeoff_at, stats.landing_at) {
        f.insert(
            "Total Flight Time".into(),
            humanize_duration_minutes((landing - takeoff).num_minutes()),
        );
    }
    if let (Some(off), Some(takeoff)) = (stats.block_off_at, stats.takeoff_at) {
        f.insert(
            "Taxi Out Time".into(),
            humanize_duration_minutes((takeoff - off).num_minutes()),
        );
    }
    if let (Some(land), Some(blocks_on)) = (stats.landing_at, stats.block_on_at) {
        f.insert(
            "Taxi In Time".into(),
            humanize_duration_minutes((blocks_on - land).num_minutes()),
        );
    }

    // ---- Weights & fuel (skip 0-value to avoid Fenix-bogus garbage) ----
    if let Some(w) = stats.takeoff_weight_kg.filter(|v| *v > 0.0) {
        f.insert("Takeoff Weight".into(), format!("{:.0} kg", w));
    }
    if let Some(w) = stats.landing_weight_kg.filter(|v| *v > 0.0) {
        f.insert("Landing Weight".into(), format!("{:.0} kg", w));
    }
    if let Some(b) = stats.block_fuel_kg.filter(|v| *v > 0.0) {
        f.insert("Block Fuel".into(), format!("{:.0} kg", b));
    }
    if let Some(fuel) = stats.landing_fuel_kg.filter(|v| *v > 0.0) {
        f.insert("Landing Fuel".into(), format!("{:.0} kg", fuel));
    }
    if let (Some(b), Some(c)) = (stats.block_fuel_kg, stats.last_fuel_kg) {
        if b > 0.0 && b > c {
            f.insert("Fuel Used".into(), format!("{:.0} kg", b - c));
        }
    }

    // ---- Touchdown analysis ----
    if let Some(score) = stats.landing_score {
        let grade = letter_grade(score.numeric());
        // "A+ (smooth) — 100/100" — combines all three views into one
        // glanceable line instead of three duplicate fields.
        f.insert(
            "Landing Score".into(),
            format!("{} ({}) — {}/100", grade, score.label(), score.numeric()),
        );
    }
    if let Some(rate) = stats.landing_rate_fpm {
        // SIGNED — negative on descent. Matches both LandingToast and
        // the latched SimVar reading. Pilots want to see the minus.
        // The peak (post-touchdown refinement) overrides this when
        // available since it's the worse of the two.
        //
        // v0.5.16: NUMERIC value only (no " fpm" suffix). The
        // FatihKoz/DisposableSpecial dmaintenance plugin reads this
        // field and runs `is_numeric()` against it; "-1641 fpm"
        // returns false there → hard-landing penalty silently
        // skipped. Pure numeric makes the maintenance check work.
        let value = stats.landing_peak_vs_fpm.unwrap_or(rate);
        f.insert("Landing Rate".into(), format!("{:.0}", value));
    }
    if let Some(g) = stats.landing_peak_g_force.or(stats.landing_g_force) {
        // v0.5.16: pure numeric (no " G" suffix). Some maintenance
        // plugins also read this; same is_numeric() reasoning.
        f.insert("Landing G-Force".into(), format!("{:.2}", g));
    }
    if let Some(p) = stats.landing_pitch_deg {
        // v0.5.16: pure numeric — `landing-pitch` slug is the tail-
        // strike-on-landing trigger in dmaintenance.
        f.insert("Landing Pitch".into(), format!("{:+.1}", p));
    }
    if let Some(b) = stats.landing_bank_deg {
        // v0.5.16: bank at touchdown — `landing-roll` slug is the
        // wing-strike-on-landing trigger in dmaintenance (default
        // limit 15°). Pure numeric.
        f.insert("Landing Roll".into(), format!("{:+.1}", b));
    }
    if let Some(p) = stats.takeoff_pitch_deg {
        // v0.5.16: `takeoff-pitch` slug — tail-strike-on-takeoff
        // trigger in dmaintenance. Pure numeric.
        f.insert("Takeoff Pitch".into(), format!("{:+.1}", p));
    }
    if let Some(b) = stats.takeoff_bank_deg {
        // v0.5.16: `takeoff-roll` slug — wing-strike-on-takeoff
        // trigger in dmaintenance. Pure numeric.
        f.insert("Takeoff Roll".into(), format!("{:+.1}", b));
    }
    if let Some(s) = stats.landing_speed_kt.filter(|v| *v > 0.0) {
        f.insert("Landing Speed".into(), format!("{:.0} kt", s));
    }
    if let Some(h) = stats.landing_heading_deg {
        f.insert("Landing Heading".into(), format!("{:03.0}°", h));
    }
    if let Some(slip) = stats.touchdown_sideslip_deg {
        f.insert("Touchdown Sideslip".into(), format!("{:+.1}°", slip));
    }
    if let Some(hw) = stats.landing_headwind_kt {
        if hw >= 0.0 {
            f.insert("Touchdown Headwind".into(), format!("{:.0} kt", hw));
        } else {
            f.insert("Touchdown Tailwind".into(), format!("{:.0} kt", -hw));
        }
    }
    if let Some(xw) = stats.landing_crosswind_kt {
        let side = if xw >= 0.0 { "from right" } else { "from left" };
        f.insert(
            "Touchdown Crosswind".into(),
            format!("{:.0} kt {}", xw.abs(), side),
        );
    }
    if stats.bounce_count > 0 {
        // Suppress the "Bounces: 0" row — every clean landing had it
        // and it added noise without information.
        f.insert("Bounces".into(), stats.bounce_count.to_string());
    }

    // ---- Touch-and-Go + Go-Around counters (v0.1.26) ----
    // Only surface when nonzero — every routine PIREP has zero of
    // both, no point adding empty rows. The detailed touchdown list
    // (peak V/S + G per event) lives in the Notes prose so it doesn't
    // bloat the structured fields with N rows on a training flight.
    let tg_count = stats
        .touchdown_events
        .iter()
        .filter(|e| matches!(e.kind, TouchdownKind::TouchAndGo))
        .count();
    if tg_count > 0 || stats.touchdown_events.len() > 1 {
        // "Touchdowns: 4 (3 T&G + final)" — gives the VA admin one
        // glanceable line that matches the Notes story.
        let total = stats.touchdown_events.len();
        let suffix = if tg_count == 0 {
            String::new()
        } else if total == tg_count {
            format!(" ({} T&G)", tg_count)
        } else {
            format!(" ({} T&G + final)", tg_count)
        };
        f.insert("Touchdowns".into(), format!("{}{}", total, suffix));
    }
    if stats.go_around_count > 0 {
        f.insert("Go-Arounds".into(), stats.go_around_count.to_string());
    }

    // ---- Approach Stability + Rollout (Landing Analyzer Stage 1) ----
    if let Some(sd) = stats.approach_vs_stddev_fpm {
        f.insert("Approach V/S Stddev".into(), format!("{:.0} fpm", sd));
    }
    if let Some(sd) = stats.approach_bank_stddev_deg {
        f.insert("Approach Bank Stddev".into(), format!("{:.1}°", sd));
    }
    if let Some(meters) = stats.rollout_distance_m {
        // Single field with both units — "935 m (3068 ft)" instead of
        // three separate Rollout Distance / rollout_distance_m /
        // rollout_distance_ft entries.
        f.insert(
            "Rollout Distance".into(),
            format!("{:.0} m ({:.0} ft)", meters, meters * 3.28084),
        );
    }

    // ---- PMDG Premium Telemetry (Phase H.4 / v0.2.0) ----
    // Only emitted when the pilot flew a PMDG aircraft AND had the
    // SDK enabled (so we got real cockpit data). Field names use
    // "PMDG …" prefix so VA admins can filter PIREPs that have
    // premium-telemetry coverage.
    if let Some(label) = stats.pmdg_variant_label.as_deref() {
        f.insert("PMDG Aircraft".into(), label.to_string());
    }
    if let Some(fnum) = stats.pmdg_fmc_flight_number.as_deref().filter(|s| !s.is_empty()) {
        f.insert("PMDG FMC Flight #".into(), fnum.to_string());
    }
    if let Some((v1, vr, v2)) = stats.pmdg_v_speeds_takeoff {
        f.insert(
            "PMDG V-Speeds (Takeoff)".into(),
            format!("V1 {v1} · VR {vr} · V2 {v2} kt"),
        );
    }
    if let Some(vref) = stats.pmdg_vref_at_landing {
        f.insert("PMDG VREF (Landing)".into(), format!("{vref} kt"));
    }
    if let Some(deg) = stats.pmdg_takeoff_flaps_planned {
        f.insert("PMDG Plan TO Flaps".into(), format!("{deg}°"));
    }
    if let Some(deg) = stats.pmdg_landing_flaps_planned {
        f.insert("PMDG Plan LDG Flaps".into(), format!("{deg}°"));
    }
    if let Some(ab) = stats.pmdg_autobrake_at_landing.as_deref() {
        f.insert("PMDG Autobrake (Landing)".into(), ab.to_string());
    }
    if stats.pmdg_takeoff_config_warning_seen {
        f.insert(
            "PMDG TO Config Warning".into(),
            "JA — Warnung war während TakeoffRoll aktiv".to_string(),
        );
    }
    // 777 ECL phase summary — only emit when at least ONE phase
    // was completed (avoids "PMDG ECL: 0/10" on aircraft without
    // ECL like NG3).
    if let Some(ecl) = stats.pmdg_ecl_phases_complete {
        let labels = [
            "Preflight", "BeforeStart", "BeforeTaxi", "BeforeTakeoff",
            "AfterTakeoff", "Descent", "Approach", "Landing",
            "Shutdown", "Secure",
        ];
        let done: Vec<&str> = ecl
            .iter()
            .zip(labels.iter())
            .filter_map(|(d, l)| if *d { Some(*l) } else { None })
            .collect();
        if !done.is_empty() {
            f.insert(
                "PMDG ECL Complete".into(),
                format!("{} / 10 ({})", done.len(), done.join(", ")),
            );
        }
    }

    // ---- SimBrief OFP plan + fuel-efficiency (Landing Analyzer Stage 2) ----
    // Surface the dispatcher's plan alongside the actuals, and compute
    // a fuel-efficiency delta so the VA can grade burn discipline.
    if let Some(b) = stats.planned_block_fuel_kg {
        f.insert("Plan Block Fuel".into(), format!("{:.0} kg", b));
    }
    if let Some(b) = stats.planned_burn_kg {
        f.insert("Plan Trip Burn".into(), format!("{:.0} kg", b));
    }
    if let Some(r) = stats.planned_reserve_kg {
        f.insert("Plan Reserve Fuel".into(), format!("{:.0} kg", r));
    }
    if let Some(z) = stats.planned_zfw_kg {
        f.insert("Plan ZFW".into(), format!("{:.0} kg", z));
    }
    if let Some(t) = stats.planned_tow_kg {
        f.insert("Plan TOW".into(), format!("{:.0} kg", t));
    }
    if let Some(l) = stats.planned_ldw_kg {
        f.insert("Plan LDW".into(), format!("{:.0} kg", l));
    }
    if let Some(route) = stats.planned_route.as_deref().filter(|s| !s.is_empty()) {
        f.insert("Plan Route".into(), route.to_string());
    }
    if let Some(alt) = stats.planned_alternate.as_deref().filter(|s| !s.is_empty()) {
        f.insert("Plan Alternate".into(), alt.to_string());
    }
    // Fuel efficiency: compare actual trip burn to the planned trip
    // burn. Use takeoff_fuel_kg − landing_fuel_kg as the actual trip
    // burn (excludes taxi out/in, matching SimBrief's est_burn).
    if let (Some(plan_burn), Some(toff), Some(land)) = (
        stats.planned_burn_kg,
        stats.takeoff_fuel_kg,
        stats.landing_fuel_kg,
    ) {
        if plan_burn > 0.0 && toff > 0.0 && land >= 0.0 && toff > land {
            let actual_burn = toff - land;
            let diff = actual_burn - plan_burn; // +ve = burned more than planned
            let pct = (diff / plan_burn) * 100.0;
            let sign = if diff >= 0.0 { "+" } else { "" };
            f.insert(
                "Fuel Efficiency".into(),
                format!("{sign}{:.0} kg ({sign}{:.1}%)", diff, pct),
            );
        }
    }

    // ---- Runway correlation (Tier 2) ----
    if let Some(rw) = &stats.runway_match {
        // Composite line covers airport+ident+length+surface in one
        // glanceable cell instead of five separate snake_case rows.
        f.insert(
            "Touchdown Runway".into(),
            format!(
                "{}/{} ({:.0} m, {})",
                rw.airport_ident,
                rw.runway_ident,
                rw.length_ft as f64 * 0.3048,
                rw.surface,
            ),
        );
        f.insert(
            "Centerline Offset".into(),
            format!("{:.1} m {}", rw.centerline_distance_m.abs(), rw.side),
        );
        f.insert(
            "Touchdown Past Threshold".into(),
            format!(
                "{:.0} m (runway {:.0} m long)",
                rw.touchdown_distance_from_threshold_ft as f64 * 0.3048,
                rw.length_ft as f64 * 0.3048,
            ),
        );
    }

    // ---- ATC-derived gates and approach runway (from MSFS SimVars) ----
    if let Some(g) = stats.dep_gate.as_ref().filter(|s| !s.is_empty()) {
        f.insert("Departure Gate".into(), g.clone());
    }
    if let Some(g) = stats.arr_gate.as_ref().filter(|s| !s.is_empty()) {
        f.insert("Arrival Gate".into(), g.clone());
    }
    if let Some(rw) = stats.approach_runway.as_ref().filter(|s| !s.is_empty()) {
        f.insert("Approach Runway (ATC)".into(), rw.clone());
    }

    // ---- Distance / cruise level ----
    if stats.distance_nm > 0.0 {
        f.insert(
            "Flown Distance".into(),
            format!("{:.1} nm", stats.distance_nm),
        );
    }
    if let Some(fl) = stats.peak_altitude_ft {
        f.insert("Cruise Level".into(), format!("FL{:.0}", fl / 100.0));
    }

    // ---- METAR snapshots ----
    if let Some(raw) = stats.dep_metar_raw.as_ref().filter(|s| !s.is_empty()) {
        f.insert("Departure METAR".into(), raw.clone());
    }
    if let Some(raw) = stats.arr_metar_raw.as_ref().filter(|s| !s.is_empty()) {
        f.insert("Arrival METAR".into(), raw.clone());
    }

    f
}

fn humanize_duration_minutes(minutes: i64) -> String {
    let m = minutes.max(0);
    let h = m / 60;
    let r = m % 60;
    if h == 0 {
        format!("{}m", r)
    } else {
        format!("{}h {:02}m", h, r)
    }
}

/// Build the human-readable summary that goes into the PIREP `notes` field —
/// a concise multi-line text that's always visible regardless of how the VA
/// configured custom fields.
fn build_pirep_notes(flight: &ActiveFlight, stats: &FlightStats) -> String {
    use std::fmt::Write;
    let mut s = String::new();

    // Header — flight number + route + (when known) callsign airline.
    let _ = write!(
        s,
        "{} · {} → {}",
        flight.flight_number, flight.dpt_airport, flight.arr_airport
    );

    // ---- Times ----
    let mut wrote_section = false;
    let start_section = |s: &mut String, title: &str| {
        s.push_str("\n\n");
        s.push_str(title);
        s.push('\n');
    };
    let any_time = stats.block_off_at.is_some()
        || stats.takeoff_at.is_some()
        || stats.landing_at.is_some()
        || stats.block_on_at.is_some();
    if any_time {
        start_section(&mut s, "TIMES (UTC)");
        wrote_section = true;
        if let Some(t) = stats.block_off_at {
            let _ = writeln!(s, "  Blocks off    {}", t.format("%H:%M:%S"));
        }
        if let Some(t) = stats.takeoff_at {
            let _ = writeln!(s, "  Takeoff       {}", t.format("%H:%M:%S"));
        }
        if let Some(t) = stats.landing_at {
            let _ = writeln!(s, "  Landing       {}", t.format("%H:%M:%S"));
        }
        if let Some(t) = stats.block_on_at {
            let _ = writeln!(s, "  Blocks on     {}", t.format("%H:%M:%S"));
        }
        if let (Some(off), Some(on)) = (stats.block_off_at, stats.block_on_at) {
            let _ = writeln!(
                s,
                "  Block time    {}",
                humanize_duration_minutes((on - off).num_minutes())
            );
        }
        if let (Some(t1), Some(t2)) = (stats.takeoff_at, stats.landing_at) {
            let _ = writeln!(
                s,
                "  Flight time   {}",
                humanize_duration_minutes((t2 - t1).num_minutes())
            );
        }
        if let (Some(off), Some(takeoff)) = (stats.block_off_at, stats.takeoff_at) {
            let _ = writeln!(
                s,
                "  Taxi out      {}",
                humanize_duration_minutes((takeoff - off).num_minutes())
            );
        }
        if let (Some(land), Some(on)) = (stats.landing_at, stats.block_on_at) {
            let _ = writeln!(
                s,
                "  Taxi in       {}",
                humanize_duration_minutes((on - land).num_minutes())
            );
        }
    }

    // ---- Touchdown ----
    if stats.landing_at.is_some() {
        start_section(&mut s, "TOUCHDOWN");
        wrote_section = true;
        if let Some(rate) = stats.landing_rate_fpm {
            let _ = writeln!(s, "  Landing rate  {:.0} fpm", rate);
        }
        if let Some(g) = stats.landing_g_force {
            let _ = writeln!(s, "  G-force       {:.2} G", g);
        }
        if let Some(p) = stats.landing_pitch_deg {
            let _ = writeln!(s, "  Pitch         {:+.1}°", p);
        }
        if let Some(kt) = stats.landing_speed_kt.filter(|v| *v > 0.0) {
            let _ = writeln!(s, "  Speed         {:.0} kt", kt);
        }
        if let Some(slip) = stats.touchdown_sideslip_deg {
            let _ = writeln!(s, "  Sideslip      {:+.1}°", slip);
        }
        if let Some(hw) = stats.landing_headwind_kt {
            // Positive = headwind, negative = tailwind. We swap the
            // label rather than printing a negative number.
            if hw >= 0.0 {
                let _ = writeln!(s, "  Headwind      {:.0} kt", hw);
            } else {
                let _ = writeln!(s, "  Tailwind      {:.0} kt", -hw);
            }
        }
        if let Some(xw) = stats.landing_crosswind_kt {
            // Positive = from right, negative = from left.
            let side = if xw >= 0.0 { "from right" } else { "from left" };
            let _ = writeln!(s, "  Crosswind     {:.0} kt {}", xw.abs(), side);
        }
        let _ = writeln!(s, "  Bounces       {}", stats.bounce_count);
        if let Some(sd) = stats.approach_vs_stddev_fpm {
            let _ = writeln!(s, "  Apr V/S σ     {:.0} fpm", sd);
        }
        if let Some(sd) = stats.approach_bank_stddev_deg {
            let _ = writeln!(s, "  Apr Bank σ    {:.1}°", sd);
        }
        if let Some(m) = stats.rollout_distance_m {
            let _ = writeln!(s, "  Rollout       {:.0} m", m);
        }
        if let Some(score) = stats.landing_score {
            let grade = letter_grade(score.numeric());
            let _ = writeln!(
                s,
                "  Grade         {} ({}, {}/100)",
                grade,
                score.label().to_uppercase(),
                score.numeric()
            );
        }
    }

    // ---- Touchdown history (T&G + final) ----
    // Only render when something noteworthy happened: at least one
    // T&G, OR more than one touchdown event total. A routine A→B with
    // a single final landing skips this section to keep notes short.
    let tg_count = stats
        .touchdown_events
        .iter()
        .filter(|e| matches!(e.kind, TouchdownKind::TouchAndGo))
        .count();
    if tg_count > 0 || stats.touchdown_events.len() > 1 {
        start_section(&mut s, "TOUCHDOWNS");
        wrote_section = true;
        for (i, ev) in stats.touchdown_events.iter().enumerate() {
            let label = match ev.kind {
                TouchdownKind::TouchAndGo => "T&G",
                TouchdownKind::FinalLanding => "Final",
            };
            let _ = writeln!(
                s,
                "  #{:<2} {:<5} {} V/S {:+.0} fpm · G {:.2} · bounces {}",
                i + 1,
                label,
                ev.timestamp.format("%H:%M:%S"),
                ev.peak_vs_fpm,
                ev.peak_g,
                ev.sub_bounces,
            );
        }
    }
    if stats.go_around_count > 0 {
        start_section(&mut s, "GO-AROUNDS");
        wrote_section = true;
        let _ = writeln!(s, "  Count         {}", stats.go_around_count);
    }

    // ---- Runway ----
    if let Some(rw) = &stats.runway_match {
        start_section(&mut s, "RUNWAY");
        wrote_section = true;
        let _ = writeln!(
            s,
            "  Touchdown     {}/{}  ({}, {:.0} m long)",
            rw.airport_ident,
            rw.runway_ident,
            rw.surface,
            rw.length_ft as f64 * 0.3048,
        );
        let _ = writeln!(
            s,
            "  Centerline    {:.1} m  ({})",
            rw.centerline_distance_m.abs(),
            rw.side
        );
        let _ = writeln!(
            s,
            "  Past thresh   {:.0} m",
            rw.touchdown_distance_from_threshold_ft as f64 * 0.3048,
        );
    }

    // ---- Fuel & Weight ----
    let any_fuel = stats.block_fuel_kg.filter(|v| *v > 0.0).is_some()
        || stats.takeoff_weight_kg.filter(|v| *v > 0.0).is_some()
        || stats.landing_weight_kg.filter(|v| *v > 0.0).is_some()
        || stats.landing_fuel_kg.filter(|v| *v > 0.0).is_some();
    if any_fuel {
        start_section(&mut s, "FUEL & WEIGHT");
        wrote_section = true;
        if let Some(b) = stats.block_fuel_kg.filter(|v| *v > 0.0) {
            let _ = writeln!(s, "  Block fuel    {:.0} kg", b);
        }
        if let (Some(b), Some(c)) = (stats.block_fuel_kg, stats.last_fuel_kg) {
            if b > 0.0 && b > c {
                let _ = writeln!(s, "  Fuel used     {:.0} kg", b - c);
            }
        }
        if let Some(f) = stats.landing_fuel_kg.filter(|v| *v > 0.0) {
            let _ = writeln!(s, "  Landing fuel  {:.0} kg", f);
        }
        if let Some(w) = stats.takeoff_weight_kg.filter(|v| *v > 0.0) {
            let _ = writeln!(s, "  TOW           {:.0} kg", w);
        }
        if let Some(w) = stats.landing_weight_kg.filter(|v| *v > 0.0) {
            let _ = writeln!(s, "  LDW           {:.0} kg", w);
        }
        // Stage 2: SimBrief plan + efficiency delta.
        if let Some(b) = stats.planned_burn_kg {
            let _ = writeln!(s, "  Plan burn     {:.0} kg", b);
        }
        if let Some(b) = stats.planned_block_fuel_kg {
            let _ = writeln!(s, "  Plan block    {:.0} kg", b);
        }
        if let Some(t) = stats.planned_tow_kg {
            let _ = writeln!(s, "  Plan TOW      {:.0} kg", t);
        }
        if let Some(l) = stats.planned_ldw_kg {
            let _ = writeln!(s, "  Plan LDW      {:.0} kg", l);
        }
        if let (Some(plan_burn), Some(toff), Some(land)) = (
            stats.planned_burn_kg,
            stats.takeoff_fuel_kg,
            stats.landing_fuel_kg,
        ) {
            if plan_burn > 0.0 && toff > land && toff > 0.0 && land >= 0.0 {
                let actual = toff - land;
                let diff = actual - plan_burn;
                let pct = (diff / plan_burn) * 100.0;
                let sign = if diff >= 0.0 { "+" } else { "" };
                let _ = writeln!(
                    s,
                    "  Efficiency    {sign}{:.0} kg ({sign}{:.1}%)",
                    diff, pct
                );
            }
        }
    }

    // ---- Stand / Runway (ATC-cleared) ----
    let any_atc = stats.dep_gate.as_ref().is_some_and(|v| !v.is_empty())
        || stats.arr_gate.as_ref().is_some_and(|v| !v.is_empty())
        || stats.approach_runway.as_ref().is_some_and(|v| !v.is_empty());
    if any_atc {
        start_section(&mut s, "GATES & ATC");
        wrote_section = true;
        if let Some(g) = stats.dep_gate.as_ref().filter(|v| !v.is_empty()) {
            let _ = writeln!(s, "  Departure gate  {}", g);
        }
        if let Some(g) = stats.arr_gate.as_ref().filter(|v| !v.is_empty()) {
            let _ = writeln!(s, "  Arrival gate    {}", g);
        }
        if let Some(rw) = stats.approach_runway.as_ref().filter(|v| !v.is_empty()) {
            let _ = writeln!(s, "  Approach rwy    {}", rw);
        }
    }

    // ---- Distance ----
    if stats.distance_nm > 0.0 || stats.peak_altitude_ft.is_some() {
        start_section(&mut s, "DISTANCE & LEVEL");
        wrote_section = true;
        let _ = writeln!(s, "  Flown         {:.1} nm", stats.distance_nm);
        if let Some(fl) = stats.peak_altitude_ft {
            let _ = writeln!(s, "  Cruise level  FL{:.0}", fl / 100.0);
        }
        let _ = writeln!(s, "  Positions     {}", stats.position_count);
    }

    // ---- METAR ----
    let dep_metar = stats.dep_metar_raw.as_ref().filter(|v| !v.is_empty());
    let arr_metar = stats.arr_metar_raw.as_ref().filter(|v| !v.is_empty());
    if dep_metar.is_some() || arr_metar.is_some() {
        start_section(&mut s, "METAR");
        wrote_section = true;
        if let Some(m) = dep_metar {
            let _ = writeln!(s, "  Departure  {}", m);
        }
        if let Some(m) = arr_metar {
            let _ = writeln!(s, "  Arrival    {}", m);
        }
    }

    // Footer
    if wrote_section {
        s.push('\n');
    } else {
        s.push_str("\n\n");
    }
    let _ = write!(s, "AeroACARS {}", env!("CARGO_PKG_VERSION"));

    s
}

/// Map our internal `FlightPhase` to the phpVMS PirepStatus code we POST
/// in `update_pirep`. Codes verified against the canonical phpVMS Core
/// enum at `phpvms/phpvms` → `app/Models/Enums/PirepStatus.php`:
///
///   INI Initiated   SCH Scheduled    BST Boarding         RDT Ready
///   PBT Pushback    OFB Off block    DIR Ready de-ice     DIC De-icing
///   GRT Ground rtn  TXI Taxi         TOF Takeoff (roll)   TKO Airborne
///   ICL Init climb  ENR Enroute      DV  Diverted
///   TEN Approach    APR Approach     FIN On final         LDG Landing
///   LAN Landed      ONB On block     ARR Arrived          DX  Cancelled
///   EMG Emerg desc  PSD Paused
///
/// Notes / pitfalls discovered while wiring this up (real bugs):
///   * Pushback must be PBT, not OFB. phpVMS labels `OFB` as
///     "departed" (German: "Abgeflogen") — sending OFB during pushback
///     made the live tracker show "Abgeflogen" before the aircraft had
///     even moved off the gate.
///   * There is no "TOD" or "FAP" or "TXG" in phpVMS — early versions
///     of this client invented those, which would have been silently
///     dropped (or echoed as the literal letters in some VA themes).
///   * `TOF` is the on-runway-takeoff-roll phase, `TKO` is the
///     airborne / climb-out phase. Their human-readable labels
///     collapse to "takeoff" and "enroute" respectively.
///   * Phases the API offers but we don't drive: SCH, DIR/DIC (de-ice),
///     GRT (ground return), DV (divert), EMG, PSD. We may emit them
///     later from manual-end paths.
fn phase_to_status(phase: FlightPhase) -> Option<&'static str> {
    match phase {
        FlightPhase::Preflight | FlightPhase::Boarding => Some("BST"),
        FlightPhase::Pushback => Some("PBT"),
        FlightPhase::TaxiOut => Some("TXI"),
        FlightPhase::TakeoffRoll => Some("TOF"),
        FlightPhase::Takeoff => Some("TKO"),
        FlightPhase::Climb => Some("ICL"),
        // No dedicated "descent" code in phpVMS — the canonical
        // taxonomy stays ENROUTE until the aircraft enters the
        // approach phase (which we cover with APR / FIN below).
        FlightPhase::Cruise | FlightPhase::Descent => Some("ENR"),
        // v0.5.11: phpVMS has no "Holding" status code. ENR is the
        // closest match (we're enroute, just circling). The Cockpit
        // tab UI has its own Holding badge that does NOT depend on
        // this status code mapping.
        FlightPhase::Holding => Some("ENR"),
        FlightPhase::Approach => Some("APR"),
        FlightPhase::Final => Some("FIN"),
        FlightPhase::Landing => Some("LDG"),
        // No separate "taxi to gate" code; phpVMS reuses TXI.
        FlightPhase::TaxiIn => Some("TXI"),
        FlightPhase::BlocksOn => Some("ONB"),
        FlightPhase::Arrived => Some("ARR"),
        FlightPhase::PirepSubmitted => None,
    }
}

/// Trigger one-shot METAR fetches for departure (at takeoff) and arrival
/// (at touchdown). Each side fires only once per flight — the
/// `dep_metar_requested` / `arr_metar_requested` flags on FlightStats
/// gate it. Fetch runs on the Tauri async runtime so the streamer
/// keeps ticking even if aviationweather.gov is slow.
fn maybe_spawn_metar_fetch(app: &AppHandle, flight: &Arc<ActiveFlight>, new_phase: FlightPhase) {
    let kind = match new_phase {
        FlightPhase::Takeoff => MetarKind::Departure,
        // Trigger at Final so the arrival weather is in the activity
        // log *before* the touchdown line, not after — pilots want the
        // wind for the approach, not for the rollout.
        FlightPhase::Final => MetarKind::Arrival,
        _ => return,
    };
    let icao = match kind {
        MetarKind::Departure => flight.dpt_airport.clone(),
        MetarKind::Arrival => flight.arr_airport.clone(),
    };
    {
        let mut stats = flight.stats.lock().expect("flight stats");
        let already = match kind {
            MetarKind::Departure => stats.dep_metar_requested,
            MetarKind::Arrival => stats.arr_metar_requested,
        };
        if already {
            return;
        }
        match kind {
            MetarKind::Departure => stats.dep_metar_requested = true,
            MetarKind::Arrival => stats.arr_metar_requested = true,
        }
    }
    let app = app.clone();
    let flight = Arc::clone(flight);
    tauri::async_runtime::spawn(async move {
        match metar::fetch_metar(&icao).await {
            Ok(snap) => {
                let raw = snap.raw.clone();
                {
                    let mut stats = flight.stats.lock().expect("flight stats");
                    match kind {
                        MetarKind::Departure => stats.dep_metar_raw = Some(raw.clone()),
                        MetarKind::Arrival => stats.arr_metar_raw = Some(raw.clone()),
                    }
                }
                let label = match kind {
                    MetarKind::Departure => "Departure METAR",
                    MetarKind::Arrival => "Arrival METAR",
                };
                log_activity_handle(
                    &app,
                    ActivityLevel::Info,
                    format!("{label}: {icao}"),
                    Some(if raw.is_empty() {
                        "(empty)".to_string()
                    } else {
                        raw
                    }),
                );
            }
            Err(e) => {
                tracing::warn!(error = %e, %icao, "metar fetch failed");
            }
        }
    });
}

/// Internal discriminator for the two METAR fetch directions.
#[derive(Debug, Clone, Copy)]
enum MetarKind {
    Departure,
    Arrival,
}

/// Emit the landing analyzer's verdict to the activity log exactly once,
/// the first time `step_flight` finalises a score. Called every tick;
/// idempotent by design — uses an extra "already announced" flag in
/// `FlightStats` so resumed sessions that already filed don't re-emit.
/// Returns `Some(<acars-log-message>)` exactly once (the first tick after
/// the touchdown analyzer locked in a score) so the streamer can mirror
/// the touchdown summary into `POST /pireps/{id}/acars/logs` for the
/// PIREP detail page. Subsequent ticks return None.
fn announce_landing_score(app: &AppHandle, flight: &ActiveFlight) -> Option<String> {
    let stats = flight.stats.lock().expect("flight stats");
    if stats.landing_score_announced {
        return None;
    }
    let score = stats.landing_score?;
    let peak_vs = stats.landing_peak_vs_fpm.unwrap_or(0.0);
    let peak_g = stats.landing_peak_g_force.unwrap_or(0.0);
    let bounces = stats.bounce_count;
    let level = match score {
        LandingScore::Smooth | LandingScore::Acceptable => ActivityLevel::Info,
        LandingScore::Firm => ActivityLevel::Info,
        LandingScore::Hard | LandingScore::Severe => ActivityLevel::Warn,
    };
    let bounce_part = if bounces > 0 {
        format!(", {} bounce{}", bounces, if bounces == 1 { "" } else { "s" })
    } else {
        String::new()
    };
    // Drop the lock before logging so log_activity_handle can
    // grab the activity_log mutex without deadlocking via
    // re-entrant borrow.
    drop(stats);
    let grade = letter_grade(score.numeric());
    log_activity_handle(
        app,
        level,
        format!("Touchdown: {} ({})", grade, score.label()),
        Some(format!(
            "V/S {:.0} fpm, G {:.2}{} — Score {}/100",
            peak_vs, // signed: negative = descent, matches the PIREP
            peak_g,
            bounce_part,
            score.numeric(),
        )),
    );
    record_event(
        app,
        &flight.pirep_id,
        &FlightLogEvent::LandingScored {
            timestamp: Utc::now(),
            score: score.label().to_string(),
            peak_vs_fpm: peak_vs,
            peak_g_force: peak_g,
            bounce_count: bounces,
        },
    );
    // Re-acquire to flag it as announced.
    let mut stats = flight.stats.lock().expect("flight stats");
    stats.landing_score_announced = true;
    Some(format!(
        "Touchdown {} ({}) — V/S {:.0} fpm, G {:.2}{}, score {}/100",
        grade,
        score.label(),
        peak_vs,
        peak_g,
        bounce_part,
        score.numeric(),
    ))
}

/// Diff cockpit knob state against the last-logged values and emit one
/// activity-feed entry per real change. Called from the streamer loop on
/// every tick. The first call (after a fresh `flight_start` / `flight_adopt`)
/// also logs the aircraft + simulator identity, so the log opens with a
/// "Aircraft: …" line like smartcars does.
fn detect_telemetry_changes(app: &AppHandle, flight: &ActiveFlight, snap: &SimSnapshot) {
    let mut stats = flight.stats.lock().expect("flight stats");

    // ---- Aircraft + sim banner — first tick only.
    // Aircraft banner: gated on a dedicated per-flight flag rather
    // than the heuristic "all three diff fields are still None"
    // because the heuristic is too fragile (a stale resumed flight
    // could already have those fields set, suppressing the banner
    // when it should fire). The flag is persisted, so a Tauri
    // restart mid-flight doesn't re-fire — the banner belongs to
    // the flight, not the session.
    let first_tick = !stats.aircraft_banner_logged;
    if first_tick {
        stats.aircraft_banner_logged = true;
        let title = snap.aircraft_title.as_deref().unwrap_or("(unknown)");
        // Some aircraft (Fenix, FBW) return a localization key like
        // "ATCCOM.AC_MODEL A320.0.text" in `ATC MODEL` rather than the plain
        // ICAO. Filter that out — a stray dot in an ICAO never happens.
        let icao_raw = snap.aircraft_icao.as_deref().unwrap_or("");
        // v0.2.4: when the standard ICAO is unusable (empty, or a
        // localisation key like "ATCCOM.AC_MODEL_B738.0.text" that
        // MSFS 2024 hands out for some PMDG liveries), fall back to
        // the PMDG-variant family ICAO. That's accurate to the family
        // (B738/B77W) even if not to the sub-variant.
        let pmdg_fallback_icao: Option<&'static str> = snap
            .pmdg
            .as_ref()
            .and_then(|_| {
                // Aircraft path tells us which PMDG family is active.
                // We don't know it from snap.pmdg.variant_label alone,
                // but we do know "PMDG SDK is active" — so we fall
                // back via the variant_label string content.
                let label = &snap.pmdg.as_ref()?.variant_label;
                if label.contains("737") {
                    Some("B738")
                } else if label.contains("777") {
                    // Pick the most-common 777 variant as default;
                    // exact -200LR / -300ER / 777F can't be inferred
                    // from the variant label alone (that's a richer
                    // mapping we may add later).
                    Some("B77W")
                } else {
                    None
                }
            });
        let icao = if icao_raw.contains('.') || icao_raw.is_empty() {
            pmdg_fallback_icao.unwrap_or("?")
        } else {
            icao_raw
        };
        let reg = snap
            .aircraft_registration
            .as_deref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .unwrap_or("?");
        // NOTE: planned_registration vs sim-reg comparison removed for now
        // — MSFS 2024's pilot-profile tail number overrides the livery's
        // atc_id, so the comparison fired false-positive warnings. We keep
        // `flight.planned_registration` populated server-side (it's still
        // useful for PIREP audit), but the live banner just shows what
        // MSFS reports. Re-enable later via a WASM-based livery reader.
        let _ = &flight.planned_registration;
        // v0.2.4: when PMDG SDK is feeding us premium telemetry, surface
        // that in the profile string so the pilot doesn't see the
        // misleading "Default (standard SimVars)" right before the
        // "PMDG SDK aktiv" entry one tick later.
        let profile_label = if let Some(pmdg) = snap.pmdg.as_ref() {
            format!(
                "{} + {}",
                snap.aircraft_profile.label(),
                pmdg.variant_label
            )
        } else {
            snap.aircraft_profile.label().to_string()
        };
        log_activity_handle(
            app,
            ActivityLevel::Info,
            format!("Aircraft: {title}"),
            Some(format!(
                "Type {icao} · Reg {reg} · Sim {:?} · Profile: {}",
                snap.simulator, profile_label
            )),
        );
    }

    // ---- Profile change (after first tick) — pilot swapped airframes
    if stats.last_logged_profile != Some(snap.aircraft_profile) {
        if !first_tick {
            // v0.2.4: same Premium-First label suffix as the banner —
            // PMDG SDK active means the *premium* layer is doing the
            // heavy lifting even if the underlying SimVar profile
            // is still the generic "Default" one.
            let label = if let Some(pmdg) = snap.pmdg.as_ref() {
                format!("{} + {}", snap.aircraft_profile.label(), pmdg.variant_label)
            } else {
                snap.aircraft_profile.label().to_string()
            };
            log_activity_handle(
                app,
                ActivityLevel::Info,
                format!("Aircraft profile changed → {}", label),
                None,
            );
        }
        stats.last_logged_profile = Some(snap.aircraft_profile);
    }

    // ---- Squawk
    // Real-world squawks are always 4 octal digits ≥ 1000 — Fenix and a
    // few other study-level airbuses pump the active-edit-digit into
    // TRANSPONDER CODE:1 while the pilot keypad-types, which the FlyByWire
    // / Asobo aircraft don't do. Filter anything < 1000 so the log isn't
    // flooded with "Squawk set to 0004 / 0003 / 0002" while a value like
    // 2523 is being entered. The proper fix lives in the per-aircraft
    // LVar profile (Phase H.4 stage 2).
    if let Some(sq) = snap.transponder_code {
        let is_plausible = sq >= 1000;
        if is_plausible && stats.last_logged_squawk != Some(sq) {
            // Don't spam on the first tick if the value is the boring 1200/7000.
            if !first_tick || (sq != 1200 && sq != 7000) {
                log_activity_handle(
                    app,
                    ActivityLevel::Info,
                    format!("Squawk set to {:04}", sq),
                    None,
                );
            }
            stats.last_logged_squawk = Some(sq);
        }
    }

    // ---- XPDR mode (v0.2.4 PMDG + v0.3.0 X-Plane).
    // Cockpit exposes the actual transponder MODE selector
    // (STBY / ALT-OFF / XPNDR / TA / TA-RA) which the standard MSFS
    // SimVar `TRANSPONDER STATE` only exposes as a binary on/off.
    // Now sourced from `snap.xpdr_mode_label` which the MSFS adapter
    // fills from PMDG and the X-Plane adapter fills from the standard
    // `transponder_mode` DataRef. Logged as a separate entry so the
    // squawk-code log keeps its original semantics.
    if let Some(mode) = snap.xpdr_mode_label.as_ref() {
        if !mode.is_empty()
            && stats.last_logged_pmdg_xpdr_mode.as_deref() != Some(mode.as_str())
        {
            // Skip first tick on a "boring" STBY/OFF value to avoid noise
            // when the pilot loads cold-and-dark.
            if !first_tick || (mode != "STBY" && mode != "OFF") {
                log_activity_handle(
                    app,
                    ActivityLevel::Info,
                    format!("XPDR mode → {}", mode),
                    None,
                );
            }
            stats.last_logged_pmdg_xpdr_mode = Some(mode.clone());
        }
    }

    // ---- COM / NAV frequency logging removed (2026-05).
    // In practice frequencies change every sector handoff (Departure
    // → Center → Center → Approach → Tower → Ground), which fills
    // the log without telling the VA admin anything useful. Plus
    // Fenix's RMP doesn't write to the standard SimVars at all, so
    // for that aircraft we'd be logging stale defaults from the
    // sim's COM panel rather than what the pilot actually tuned.
    // Squawk stays in the log — that genuinely changes only ~1-2x
    // per flight at meaningful moments.
    //
    // The fields on the snapshot remain populated so the debug
    // panel and inspector keep working.
    let _ = (
        snap.com1_mhz,
        snap.com2_mhz,
        snap.nav1_mhz,
        snap.nav2_mhz,
        &stats.last_logged_com1,
        &stats.last_logged_com2,
        &stats.last_logged_nav1,
        &stats.last_logged_nav2,
    );

    // ---- Exterior lights
    // Strobe is special: when the aircraft profile gives us the
    // 3-state selector (`snap.strobe_state`), we log OFF/AUTO/ON so
    // the AUTO ↔ ON transition at runway entry/exit is preserved.
    // For aircraft that only expose the binary `light_strobe`, we
    // fall through to the LightsState path below with plain ON/OFF
    // labels.
    let strobe_three_state_active = snap.strobe_state.is_some();
    if let Some(state) = snap.strobe_state {
        if stats.last_logged_strobe_state != Some(state) {
            if stats.last_logged_strobe_state.is_some() {
                let label = match state {
                    0 => "OFF",
                    1 => "AUTO",
                    2 => "ON",
                    _ => return,
                };
                log_activity_handle(
                    app,
                    ActivityLevel::Info,
                    format!("Strobe lights {label}"),
                    None,
                );
            }
            stats.last_logged_strobe_state = Some(state);
        }
    }

    // Premium-First (v0.2.4 + v0.3.0): wing + wheel-well are
    // NG3/777-only via PMDG (MSFS) or laminar/B738-only via the
    // X-Plane 737 family (Zibo / LevelUp / default-738). Both
    // simulators now write into the top-level SimSnapshot fields,
    // so we read directly from snap.light_wing / snap.light_wheel_well.
    // Field is `Some(...)` when the source platform actually reports
    // the switch (kept None on aircraft that don't have it — generic
    // Airbus, Cessna, etc.).
    let has_wing = snap.light_wing.is_some();
    let has_wheel_well = snap.light_wheel_well.is_some();
    let lights = LightsState {
        landing: snap.light_landing.unwrap_or(false),
        beacon: snap.light_beacon.unwrap_or(false),
        // Keep the binary state in the struct so the pill UI still
        // works; the activity-log path below skips Strobe when the
        // 3-state path is active to avoid double entries.
        strobe: snap.light_strobe.unwrap_or(false),
        taxi: snap.light_taxi.unwrap_or(false),
        nav: snap.light_nav.unwrap_or(false),
        logo: snap.light_logo.unwrap_or(false),
        wing: snap.light_wing.unwrap_or(false),
        wheel_well: snap.light_wheel_well.unwrap_or(false),
    };
    if stats.last_logged_lights != Some(lights) {
        if let Some(prev) = stats.last_logged_lights {
            // Log per-light transitions so the pilot sees exactly what changed.
            let mut changes: Vec<(&str, bool, bool)> = vec![
                ("Landing", prev.landing, lights.landing),
                ("Beacon", prev.beacon, lights.beacon),
                ("Taxi", prev.taxi, lights.taxi),
                ("Nav", prev.nav, lights.nav),
                ("Logo", prev.logo, lights.logo),
            ];
            // Only include Strobe in the binary path when the 3-state
            // selector isn't available; otherwise the dedicated
            // OFF/AUTO/ON log entry above already covers it.
            if !strobe_three_state_active {
                changes.push(("Strobe", prev.strobe, lights.strobe));
            }
            // Wing + wheel-well only fire when the source platform
            // actually reports the switch (PMDG / laminar/B738) —
            // prevents bogus "Wing OFF" entries on generic aircraft.
            if has_wing {
                changes.push(("Wing", prev.wing, lights.wing));
            }
            if has_wheel_well {
                changes.push(("Wheel-well", prev.wheel_well, lights.wheel_well));
            }
            for (name, old, new) in changes {
                if old != new {
                    log_activity_handle(
                        app,
                        ActivityLevel::Info,
                        format!("{name} lights {}", if new { "ON" } else { "OFF" }),
                        None,
                    );
                }
            }
        }
        stats.last_logged_lights = Some(lights);
    }

    // ---- Autopilot
    let ap = ApState {
        master: snap.autopilot_master.unwrap_or(false),
        heading: snap.autopilot_heading.unwrap_or(false),
        altitude: snap.autopilot_altitude.unwrap_or(false),
        nav: snap.autopilot_nav.unwrap_or(false),
        approach: snap.autopilot_approach.unwrap_or(false),
    };
    if stats.last_logged_ap != Some(ap) {
        if let Some(prev) = stats.last_logged_ap {
            // Master toggle is debounced — see AP_DEBOUNCE_SECS comment.
            if prev.master != ap.master {
                let now = Utc::now();
                if stats.pending_ap_master != Some(ap.master) {
                    // First time we see this new state — start the timer.
                    stats.pending_ap_master = Some(ap.master);
                    stats.pending_ap_master_since = Some(now);
                } else if let Some(since) = stats.pending_ap_master_since {
                    // Same state held for AP_DEBOUNCE_SECS → publish.
                    if (now - since).num_seconds() >= AP_DEBOUNCE_SECS {
                        log_activity_handle(
                            app,
                            ActivityLevel::Info,
                            format!(
                                "Autopilot {}",
                                if ap.master { "ENGAGED" } else { "OFF" }
                            ),
                            None,
                        );
                        stats.last_logged_ap = Some(ap);
                        stats.pending_ap_master = None;
                        stats.pending_ap_master_since = None;
                        return;
                    }
                }
                // Don't update last_logged_ap yet — we're waiting for the
                // debounce. Mode flips below still get to flow through.
            } else {
                // Master agrees with prev → drop any pending debounce.
                stats.pending_ap_master = None;
                stats.pending_ap_master_since = None;
            }
            let modes = [
                ("HDG", prev.heading, ap.heading),
                ("ALT", prev.altitude, ap.altitude),
                ("NAV", prev.nav, ap.nav),
                ("APR", prev.approach, ap.approach),
            ];
            for (name, old, new) in modes {
                if old != new {
                    log_activity_handle(
                        app,
                        ActivityLevel::Info,
                        format!("AP {name} {}", if new { "ARMED" } else { "OFF" }),
                        None,
                    );
                }
            }
        }
        // Only update if master is stable (otherwise the debounce branch
        // above already returned). Mode-only changes still want to land.
        if stats
            .last_logged_ap
            .is_none_or(|prev| prev.master == ap.master)
        {
            stats.last_logged_ap = Some(ap);
        }
    }

    // ---- Parking brake
    if stats.last_logged_parking_brake != Some(snap.parking_brake) {
        if stats.last_logged_parking_brake.is_some() {
            log_activity_handle(
                app,
                ActivityLevel::Info,
                format!(
                    "Parking brake {}",
                    if snap.parking_brake { "SET" } else { "RELEASED" }
                ),
                None,
            );
        }
        stats.last_logged_parking_brake = Some(snap.parking_brake);
    }

    // ---- Engines (count only — per-engine N1/N2 is a future extension)
    // Debounce 5 s: Fenix's GENERAL ENG COMBUSTION SimVar pulses
    // 0/1/0/1 during engine start and at idle, which produced phantom
    // "Engine shutdown / started" pairs in the activity log. We only
    // commit the change once the new count has held steady.
    if stats.last_logged_engines_running != Some(snap.engines_running) {
        let now = Utc::now();
        if stats.pending_engines_running != Some(snap.engines_running) {
            stats.pending_engines_running = Some(snap.engines_running);
            stats.pending_engines_running_since = Some(now);
        } else if let Some(since) = stats.pending_engines_running_since {
            if (now - since).num_seconds() >= 5 {
                if let Some(prev) = stats.last_logged_engines_running {
                    if prev < snap.engines_running {
                        log_activity_handle(
                            app,
                            ActivityLevel::Info,
                            format!(
                                "Engine started — {} running",
                                snap.engines_running
                            ),
                            None,
                        );
                    } else if prev > snap.engines_running {
                        log_activity_handle(
                            app,
                            ActivityLevel::Info,
                            format!(
                                "Engine shutdown — {} running",
                                snap.engines_running
                            ),
                            None,
                        );
                    }
                }
                stats.last_logged_engines_running = Some(snap.engines_running);
                stats.pending_engines_running = None;
                stats.pending_engines_running_since = None;
            }
        }
    } else {
        // Snap matches last logged again — drop any pending change.
        stats.pending_engines_running = None;
        stats.pending_engines_running_since = None;
    }

    // ---- Flaps detent
    // Premium-First (v0.2.4): if PMDG is loaded, use the cockpit-exact
    // Boeing handle label directly ("UP"/"1"/"5"/"15"/"30"/"40"). For
    // generic SimVars we map the normalised 0.0..1.0 position to a 0..5
    // detent — that's accurate for Airbus families (6 positions). For
    // Boeing without PMDG it's a known approximation; pilots flying
    // PMDG (the typical Boeing-study-level case) get the real labels.
    //
    // Detection: pmdg.flap_handle_label is non-empty when PMDG is the
    // source of truth. The pmdg-detent string itself is the activity-
    // log label (no detent_label() lookup needed).
    let pmdg_flap = snap
        .pmdg
        .as_ref()
        .map(|p| p.flap_handle_label.clone())
        .filter(|s| !s.is_empty());
    if let Some(label) = pmdg_flap {
        if stats.last_logged_flap_label.as_deref() != Some(label.as_str()) {
            if let Some(prev) = stats.last_logged_flap_label.as_ref() {
                // Direction: nominal numeric ordering of Boeing detents
                // ("UP" < "1" < "2" < "5" < "10" < "15" < "25" < "30" <
                // "40"). Mapping via boeing_detent_rank keeps the arrow
                // direction sensible for Boeing labels.
                let dir = if boeing_detent_rank(&label) > boeing_detent_rank(prev) {
                    "↓"
                } else {
                    "↑"
                };
                log_activity_handle(
                    app,
                    ActivityLevel::Info,
                    format!("Flaps {dir} {}", label),
                    Some(format!(
                        "IAS {:.0} kt, AGL {:.0} ft",
                        snap.indicated_airspeed_kt, snap.altitude_agl_ft
                    )),
                );
            }
            stats.last_logged_flap_label = Some(label);
            // Keep numeric path in sync to avoid double-firing if the
            // pilot swaps to a non-PMDG aircraft mid-session.
            stats.last_logged_flaps_detent = None;
        }
    } else {
        let flaps_detent = (snap.flaps_position.clamp(0.0, 1.0) * 5.0).round() as u8;
        if stats.last_logged_flaps_detent != Some(flaps_detent) {
            if let Some(prev) = stats.last_logged_flaps_detent {
                let dir = if flaps_detent > prev { "↓" } else { "↑" };
                log_activity_handle(
                    app,
                    ActivityLevel::Info,
                    format!("Flaps {dir} {}", detent_label(flaps_detent)),
                    Some(format!(
                        "IAS {:.0} kt, AGL {:.0} ft",
                        snap.indicated_airspeed_kt, snap.altitude_agl_ft
                    )),
                );
            }
            stats.last_logged_flaps_detent = Some(flaps_detent);
            stats.last_logged_flap_label = None;
        }
    }

    // ---- Stall / overspeed warnings (always loud)
    if stats.last_logged_stall != Some(snap.stall_warning) {
        if snap.stall_warning {
            log_activity_handle(
                app,
                ActivityLevel::Warn,
                "Stall warning".to_string(),
                Some(format!("IAS {:.0} kt, AGL {:.0} ft", snap.indicated_airspeed_kt, snap.altitude_agl_ft)),
            );
        }
        stats.last_logged_stall = Some(snap.stall_warning);
    }
    if stats.last_logged_overspeed != Some(snap.overspeed_warning) {
        if snap.overspeed_warning {
            log_activity_handle(
                app,
                ActivityLevel::Warn,
                "Overspeed warning".to_string(),
                Some(format!("IAS {:.0} kt", snap.indicated_airspeed_kt)),
            );
        }
        stats.last_logged_overspeed = Some(snap.overspeed_warning);
    }

    // ---- Gear handle (UP / DOWN — discretised so a moving handle
    //      doesn't fire mid-cycle entries).
    let gear_down = snap.gear_position >= 0.5;
    if stats.last_logged_gear_down != Some(gear_down) {
        if let Some(prev) = stats.last_logged_gear_down {
            if prev != gear_down {
                log_activity_handle(
                    app,
                    ActivityLevel::Info,
                    format!("Gear {}", if gear_down { "DOWN" } else { "UP" }),
                    Some(format!(
                        "AGL {:.0} ft, IAS {:.0} kt",
                        snap.altitude_agl_ft, snap.indicated_airspeed_kt
                    )),
                );
            }
        }
        stats.last_logged_gear_down = Some(gear_down);
    }

    // ---- Spoilers / speed brake.
    // We split into two events: armed (auto-deploy on touchdown) and
    // deployed (handle physically pulled back ≥5% — pilot using it
    // as speed brake or after touchdown).
    if let Some(armed) = snap.spoilers_armed {
        if stats.last_logged_spoilers_armed != Some(armed) {
            if let Some(prev) = stats.last_logged_spoilers_armed {
                if prev != armed {
                    log_activity_handle(
                        app,
                        ActivityLevel::Info,
                        format!("Speed brake {}", if armed { "ARMED" } else { "DISARMED" }),
                        None,
                    );
                }
            }
            stats.last_logged_spoilers_armed = Some(armed);
        }
    }
    if let Some(pos) = snap.spoilers_handle_position {
        let deployed = pos > 0.05;
        if stats.last_logged_spoilers_deployed != Some(deployed) {
            if let Some(prev) = stats.last_logged_spoilers_deployed {
                if prev != deployed {
                    log_activity_handle(
                        app,
                        ActivityLevel::Info,
                        format!(
                            "Spoilers {}",
                            if deployed { "DEPLOYED" } else { "RETRACTED" }
                        ),
                        if deployed {
                            Some(format!("Handle {:.0}%", pos * 100.0))
                        } else {
                            None
                        },
                    );
                }
            }
            stats.last_logged_spoilers_deployed = Some(deployed);
        }
    }

    // ---- APU. Bool-only "running" — combine the master-switch state
    //      with the RPM threshold so we log "APU started" only once
    //      the unit is actually up, not the moment the switch flips.
    if let Some(switch) = snap.apu_switch {
        let running = switch && snap.apu_pct_rpm.unwrap_or(0.0) >= 95.0;
        if stats.last_logged_apu_running != Some(running) {
            if let Some(prev) = stats.last_logged_apu_running {
                if prev != running {
                    log_activity_handle(
                        app,
                        ActivityLevel::Info,
                        format!("APU {}", if running { "started" } else { "shutdown" }),
                        None,
                    );
                }
            }
            stats.last_logged_apu_running = Some(running);
        }
    }

    // ---- Electrical / pneumatic systems.
    log_bool_change(
        app,
        snap.battery_master,
        &mut stats.last_logged_battery_master,
        "Battery master",
    );
    log_bool_change(
        app,
        snap.avionics_master,
        &mut stats.last_logged_avionics_master,
        "Avionics master",
    );
    log_bool_change(
        app,
        snap.pitot_heat,
        &mut stats.last_logged_pitot_heat,
        "Pitot heat",
    );
    log_bool_change(
        app,
        snap.engine_anti_ice,
        &mut stats.last_logged_engine_anti_ice,
        "Engine anti-ice",
    );
    log_bool_change(
        app,
        snap.wing_anti_ice,
        &mut stats.last_logged_wing_anti_ice,
        "Wing anti-ice",
    );

    // ---- Seat-belts (binary) and no-smoking (3-state).
    // Different value spaces: Fenix's `L:S_OH_SIGNS` is 0/1 (the
    // toggle uses logical-NOT) while `L:S_OH_SIGNS_SMOKING` is
    // 0/1/2 (the toggle branches between 0 and 2 explicitly).
    if let Some(v) = snap.seatbelts_sign {
        if stats.last_logged_seatbelts_sign != Some(v) {
            if stats.last_logged_seatbelts_sign.is_some() {
                let label = if v == 0 { "OFF" } else { "ON" };
                log_activity_handle(
                    app,
                    ActivityLevel::Info,
                    format!("Seat belts {label}"),
                    None,
                );
            }
            stats.last_logged_seatbelts_sign = Some(v);
        }
    }
    log_three_state_change(
        app,
        snap.no_smoking_sign,
        &mut stats.last_logged_no_smoking_sign,
        "No smoking",
    );

    // ---- Autobrake (string label).
    if let Some(ab) = snap.autobrake.as_ref() {
        if stats.last_logged_autobrake.as_ref() != Some(ab) {
            if let Some(prev) = stats.last_logged_autobrake.as_ref() {
                if prev != ab {
                    log_activity_handle(
                        app,
                        ActivityLevel::Info,
                        format!("Autobrake {ab}"),
                        None,
                    );
                }
            }
            stats.last_logged_autobrake = Some(ab.clone());
        }
    }

    // FCU encoder values (Selected ALT/HDG/SPD/V/S) intentionally
    // NOT logged. The Fenix LVars (`L:E_FCU_*`) are encoder click
    // counters, not real engineering values — Selected ALT 23 means
    // 23 clicks, not FL230 — so the log entries were misleading.
    // The standard `AUTOPILOT * VAR` SimVars give us the real values
    // already, but those don't update via LVar anyway. User feedback
    // (2026-05-02): "die brauchen wir doch nicht bekommen wir doch
    // eh schon aus dem standart" → drop entirely.

    // ---- PMDG SDK premium telemetry (Phase H.4 / v0.2.0) ----
    // Activity-log entries derived from snap.pmdg, the cockpit-
    // exact state we read via PMDG's SimConnect ClientData
    // channel. No-op if no PMDG aircraft is loaded or the SDK
    // isn't enabled (snap.pmdg is None in both cases).
    if let Some(p) = &snap.pmdg {
        // Capture variant label once for the PIREP custom fields.
        if stats.pmdg_variant_label.is_none() && !p.variant_label.is_empty() {
            stats.pmdg_variant_label = Some(p.variant_label.clone());
        }

        // Once-per-flight: aircraft identification banner.
        if !stats.pmdg_detected_logged {
            stats.pmdg_detected_logged = true;
            log_activity_handle(
                app,
                ActivityLevel::Info,
                format!("{} — PMDG SDK aktiv", p.variant_label),
                Some("Premium-Cockpit-Daten verfügbar (FMA, MCP, V-Speeds, FMC)".to_string()),
            );
        }

        // Once-per-flight: V-speeds banner when all four are set.
        // Pilots typically enter these via FMC PERF-INIT page; the
        // banner timestamps when the perf-init was completed.
        if !stats.pmdg_v_speeds_logged {
            if let (Some(v1), Some(vr), Some(v2), Some(vref)) = (
                p.fmc_v1_kt,
                p.fmc_vr_kt,
                p.fmc_v2_kt,
                p.fmc_vref_kt,
            ) {
                stats.pmdg_v_speeds_logged = true;
                log_activity_handle(
                    app,
                    ActivityLevel::Info,
                    format!("V-Speeds gesetzt: V1 {v1} · VR {vr} · V2 {v2} · VREF {vref}"),
                    None,
                );
            }
        }

        // FMA mode changes — single combined string so a switch from
        // "N1 / VNAV / LNAV" to "SPD / VNAV / LNAV" produces ONE log
        // entry, not three. Empty modes filtered out.
        let fma_combined = format_fma(&p.fma_speed_mode, &p.fma_pitch_mode, &p.fma_roll_mode);
        if stats.last_logged_pmdg_fma.as_deref() != Some(fma_combined.as_str()) {
            if let Some(prev) = stats.last_logged_pmdg_fma.as_deref() {
                if prev != fma_combined {
                    log_activity_handle(
                        app,
                        ActivityLevel::Info,
                        format!("FMA: {fma_combined}"),
                        None,
                    );
                }
            }
            stats.last_logged_pmdg_fma = Some(fma_combined);
        }

        // MCP selected speed — round to nearest knot (or .01 Mach).
        // We track as i16 so we get cheap equality. Value > 10 = knots,
        // ≤ 10 = Mach × 100. Both are valid pilot intents.
        if let Some(spd) = p.mcp_speed_raw {
            let spd_int = if spd > 10.0 {
                spd.round() as i16
            } else {
                (spd * 100.0).round() as i16
            };
            if stats.last_logged_pmdg_mcp_speed != Some(spd_int) {
                if stats.last_logged_pmdg_mcp_speed.is_some() {
                    let display = if spd > 10.0 {
                        format!("{spd:.0} kt")
                    } else {
                        format!("M {spd:.2}")
                    };
                    log_activity_handle(
                        app,
                        ActivityLevel::Info,
                        format!("MCP IAS → {display}"),
                        None,
                    );
                }
                stats.last_logged_pmdg_mcp_speed = Some(spd_int);
            }
        }

        // MCP heading — round to nearest degree.
        if let Some(hdg) = p.mcp_heading_deg {
            if stats.last_logged_pmdg_mcp_heading != Some(hdg) {
                if stats.last_logged_pmdg_mcp_heading.is_some() {
                    log_activity_handle(
                        app,
                        ActivityLevel::Info,
                        format!("MCP HDG → {hdg:03}°"),
                        None,
                    );
                }
                stats.last_logged_pmdg_mcp_heading = Some(hdg);
            }
        }

        // MCP altitude — log only on significant changes (≥ 100 ft)
        // so a moving knob doesn't spam.
        if let Some(alt) = p.mcp_altitude_ft {
            let last = stats.last_logged_pmdg_mcp_altitude;
            let significant = match last {
                Some(prev) => alt.abs_diff(prev) >= 100,
                None => true,
            };
            if significant && last != Some(alt) {
                if last.is_some() {
                    log_activity_handle(
                        app,
                        ActivityLevel::Info,
                        format!("MCP ALT → {alt} ft"),
                        None,
                    );
                }
                stats.last_logged_pmdg_mcp_altitude = Some(alt);
            }
        }

        // MCP V/S — log on changes ≥ 100 fpm.
        if let Some(vs) = p.mcp_vs_fpm {
            let last = stats.last_logged_pmdg_mcp_vs;
            let significant = match last {
                Some(prev) => (vs - prev).abs() >= 100,
                None => true,
            };
            if significant && last != Some(vs) {
                if last.is_some() {
                    log_activity_handle(
                        app,
                        ActivityLevel::Info,
                        format!("MCP V/S → {vs:+} fpm"),
                        None,
                    );
                }
                stats.last_logged_pmdg_mcp_vs = Some(vs);
            }
        }

        // A/T armed.
        if stats.last_logged_pmdg_at_armed != Some(p.at_armed) {
            if stats.last_logged_pmdg_at_armed.is_some() {
                log_activity_handle(
                    app,
                    ActivityLevel::Info,
                    format!("A/T {}", if p.at_armed { "armed" } else { "disarmed" }),
                    None,
                );
            }
            stats.last_logged_pmdg_at_armed = Some(p.at_armed);
        }

        // A/P engaged (CMD A or B).
        if stats.last_logged_pmdg_ap_engaged != Some(p.ap_engaged) {
            if stats.last_logged_pmdg_ap_engaged.is_some() {
                log_activity_handle(
                    app,
                    ActivityLevel::Info,
                    format!("A/P {}", if p.ap_engaged { "engaged" } else { "disengaged" }),
                    None,
                );
            }
            stats.last_logged_pmdg_ap_engaged = Some(p.ap_engaged);
        }

        // Takeoff config warning was here pre-v0.3.0 — moved out to a
        // simulator-agnostic block (after the PMDG `if let Some(p)`)
        // because both PMDG (MSFS) and X-Plane Zibo/LevelUp 737 fill
        // the universal `snap.takeoff_config_warning` field.

        // ---- v0.2.2 wider integration ----

        // FMC thrust-limit mode (777). Empty string = NG3 (no
        // thrust-limit field) — skip there, NG3 derives MCP_annunN1
        // for similar purpose elsewhere.
        if !p.thrust_limit_mode.is_empty()
            && stats.last_logged_pmdg_thrust_mode.as_deref()
                != Some(p.thrust_limit_mode.as_str())
        {
            if stats.last_logged_pmdg_thrust_mode.is_some() {
                log_activity_handle(
                    app,
                    ActivityLevel::Info,
                    format!("Thrust mode → {}", p.thrust_limit_mode),
                    None,
                );
            }
            stats.last_logged_pmdg_thrust_mode = Some(p.thrust_limit_mode.clone());
        }

        // 777 Electronic Checklist — log once per phase on rising
        // edge + capture for the PIREP custom field. Skips
        // silently for NG3 (ecl_complete = None).
        if let Some(ecl) = p.ecl_complete {
            // Capture for final PIREP — sticky union of all phases
            // ever ticked.
            let cap = stats.pmdg_ecl_phases_complete.get_or_insert([false; 10]);
            for (i, &done) in ecl.iter().enumerate() {
                if done {
                    cap[i] = true;
                }
            }
            for (idx, &done) in ecl.iter().enumerate() {
                if done && !stats.last_logged_pmdg_ecl[idx] {
                    let label = match idx {
                        0 => "Preflight",
                        1 => "Before Start",
                        2 => "Before Taxi",
                        3 => "Before Takeoff",
                        4 => "After Takeoff",
                        5 => "Descent",
                        6 => "Approach",
                        7 => "Landing",
                        8 => "Shutdown",
                        9 => "Secure",
                        _ => continue,
                    };
                    log_activity_handle(
                        app,
                        ActivityLevel::Info,
                        format!("ECL: {label} ✓ complete"),
                        None,
                    );
                    stats.last_logged_pmdg_ecl[idx] = true;
                }
            }
        }

        // PMDG-authoritative APU bit (777). More accurate than
        // the standard-SimVar APU_PCT_RPM heuristic. NG3 leaves
        // this None and falls back to the standard APU detection.
        if let Some(apu) = p.apu_running {
            if stats.last_logged_pmdg_apu != Some(apu) {
                if stats.last_logged_pmdg_apu.is_some() {
                    log_activity_handle(
                        app,
                        ActivityLevel::Info,
                        format!("APU {} (PMDG)", if apu { "started" } else { "shutdown" }),
                        None,
                    );
                }
                stats.last_logged_pmdg_apu = Some(apu);
            }
        }

        // Wheel chocks (777). Sets/removes around pushback time.
        if let Some(chocks) = p.wheel_chocks_set {
            if stats.last_logged_pmdg_chocks != Some(chocks) {
                if stats.last_logged_pmdg_chocks.is_some() {
                    log_activity_handle(
                        app,
                        ActivityLevel::Info,
                        format!("Chocks {}", if chocks { "ON" } else { "OFF" }),
                        None,
                    );
                }
                stats.last_logged_pmdg_chocks = Some(chocks);
            }
        }
    }

    // ---- Takeoff config warning (universal — PMDG + X-Plane 737)
    // Only on rising edge (warning turning ON). Falling edge
    // (pilot fixed it) doesn't warrant a log line. Source field
    // is filled by both adapters: MSFS via the PMDG snapshot()
    // merge, X-Plane via the `laminar/B738/annunciator/takeoff_config`
    // DataRef. None on aircraft that don't have an EICAS check.
    if let Some(to_warn) = snap.takeoff_config_warning {
        if stats.last_logged_pmdg_to_warning != Some(to_warn) {
            if to_warn {
                log_activity_handle(
                    app,
                    ActivityLevel::Warn,
                    "TAKEOFF CONFIG warning".to_string(),
                    Some("Cockpit zeigt rote Warnung — TO-Setup unvollständig".to_string()),
                );
            }
            stats.last_logged_pmdg_to_warning = Some(to_warn);
        }
    }
}

/// Combine FMA sub-modes into a single human-readable string.
/// Empty sub-modes are filtered out so a partially-engaged FMA
/// reads "VNAV / LNAV" rather than " / VNAV / LNAV".
fn format_fma(speed: &str, pitch: &str, roll: &str) -> String {
    let parts: Vec<&str> = [speed, pitch, roll]
        .iter()
        .copied()
        .filter(|s| !s.is_empty())
        .collect();
    if parts.is_empty() {
        "—".to_string()
    } else {
        parts.join(" / ")
    }
}

/// Activity-log helper for 3-state cabin signs (OFF / AUTO / ON).
fn log_three_state_change(
    app: &AppHandle,
    snap_value: Option<u8>,
    last_logged: &mut Option<u8>,
    label: &str,
) {
    let Some(v) = snap_value else { return };
    if *last_logged != Some(v) {
        if last_logged.is_some() {
            let state = match v {
                0 => "OFF",
                1 => "AUTO",
                2 => "ON",
                _ => return,
            };
            log_activity_handle(
                app,
                ActivityLevel::Info,
                format!("{label} {state}"),
                None,
            );
        }
        *last_logged = Some(v);
    }
}

/// Debounced logger for FCU encoder displays. Each tick, the pilot
/// might be turning the knob — we don't want a "Selected ALT 36000"
///
/// Currently unused — the call sites were removed when we dropped
/// the Fenix `L:E_FCU_*` LVar logging (those returned encoder
/// click counts, not engineering values). Kept because we plan to
/// revive it for the standard `AUTOPILOT * VAR` SimVars. Once the
/// new wiring lands, drop the `#[allow(dead_code)]` below.
/// for every click on the way from 12000 to 36000. We hold the new
/// value for FCU_DEBOUNCE_SECS, and only emit the log entry once it
/// has held steady for that long.
#[allow(dead_code)]
fn fcu_debounce(
    app: &AppHandle,
    snap_value: Option<i32>,
    last_logged: &mut Option<i32>,
    pending: &mut Option<(i32, DateTime<Utc>)>,
    now: DateTime<Utc>,
    label: &str,
    unit: &str,
) {
    const FCU_DEBOUNCE_SECS: i64 = 2;
    let Some(v) = snap_value else { return };
    if *last_logged == Some(v) {
        // No change since last log — drop any pending entry.
        *pending = None;
        return;
    }
    match *pending {
        Some((pv, since)) if pv == v => {
            // Same new value held — has it been steady long enough?
            if (now - since).num_seconds() >= FCU_DEBOUNCE_SECS {
                if last_logged.is_some() {
                    log_activity_handle(
                        app,
                        ActivityLevel::Info,
                        format!("{label} {v} {unit}"),
                        None,
                    );
                }
                *last_logged = Some(v);
                *pending = None;
            }
        }
        _ => {
            // New value (or first change) — start the debounce.
            *pending = Some((v, now));
        }
    }
}

/// Helper for bool-only telemetry change logging — checks Option,
/// initialises silently on first read, and emits an activity-log line
/// on every subsequent transition.
fn log_bool_change(
    app: &AppHandle,
    snap_value: Option<bool>,
    last_logged: &mut Option<bool>,
    label: &str,
) {
    let Some(v) = snap_value else { return };
    if *last_logged != Some(v) {
        if let Some(prev) = *last_logged {
            if prev != v {
                log_activity_handle(
                    app,
                    ActivityLevel::Info,
                    format!("{label} {}", if v { "ON" } else { "OFF" }),
                    None,
                );
            }
        }
        *last_logged = Some(v);
    }
}

fn snapshot_to_position(snap: &SimSnapshot) -> PositionEntry {
    PositionEntry {
        lat: snap.lat,
        lon: snap.lon,
        altitude: snap.altitude_msl_ft,
        altitude_agl: Some(snap.altitude_agl_ft),
        altitude_msl: Some(snap.altitude_msl_ft),
        heading: Some(snap.heading_deg_magnetic),
        gs: Some(snap.groundspeed_kt),
        vs: Some(snap.vertical_speed_fpm),
        ias: Some(snap.indicated_airspeed_kt),
        // phpVMS expects pounds for both the on-board total and the flow.
        fuel: Some((snap.fuel_total_kg as f64 * KG_TO_LB) as f32),
        fuel_flow: snap
            .fuel_flow_kg_per_h
            .map(|kgph| (kgph as f64 * KG_TO_LB) as f32),
        transponder: snap.transponder_code,
        autopilot: snap.autopilot_master,
        // distance-to-destination needs arrival airport coords; filled in
        // by `step_flight` once the airports cache has them. Leave None
        // for now so we don't send a misleading 0.
        distance: None,
        log: build_position_log(snap),
        sim_time: snap.timestamp.to_rfc3339(),
    }
}

/// Pack the telemetry that phpVMS doesn't have first-class columns for
/// (exterior lights, COM/NAV frequencies, autopilot modes, parking brake,
/// stall/overspeed warnings) into a compact JSON blob written to the
/// position's `log` field. The PIREP detail page renders this verbatim,
/// and Rules-Lua scripts on the server can parse it.
fn build_position_log(snap: &SimSnapshot) -> Option<String> {
    let payload = serde_json::json!({
        "lights": {
            "landing": snap.light_landing,
            "beacon": snap.light_beacon,
            "strobe": snap.light_strobe,
            "taxi": snap.light_taxi,
            "nav": snap.light_nav,
            "logo": snap.light_logo,
        },
        "com": {
            "com1": snap.com1_mhz,
            "com2": snap.com2_mhz,
        },
        "nav": {
            "nav1": snap.nav1_mhz,
            "nav2": snap.nav2_mhz,
        },
        "ap": {
            "master": snap.autopilot_master,
            "hdg": snap.autopilot_heading,
            "alt": snap.autopilot_altitude,
            "nav": snap.autopilot_nav,
            "apr": snap.autopilot_approach,
        },
        "state": {
            "parking_brake": snap.parking_brake,
            "stall": snap.stall_warning,
            "overspeed": snap.overspeed_warning,
            "gear": snap.gear_position,
            "flaps": snap.flaps_position,
            "engines_running": snap.engines_running,
        },
        "env": {
            "wind_dir_deg": snap.wind_direction_deg,
            "wind_kt": snap.wind_speed_kt,
            "qnh_hpa": snap.qnh_hpa,
            "oat_c": snap.outside_air_temp_c,
        },
        // ATC ground-handling info — current parking stand the aircraft
        // is on (only filled while still on a named stand, blank during
        // taxi / cruise / approach) and the ATC-cleared runway. Gives
        // the live-map "where on the ramp / approach" for free.
        "atc": {
            "parking_name": snap.parking_name,
            "parking_number": snap.parking_number,
            "runway": snap.selected_runway,
        },
        // Aircraft profile that produced this row. Lets the VA admin filter
        // PIREPs by add-on (FBW vs Fenix vs PMDG) when reviewing data and
        // know whether the cockpit-state snapshot used standard SimVars or
        // an LVar mapping (Phase H.4 Stage 2).
        "profile": snap.aircraft_profile,
    });
    serde_json::to_string(&payload).ok()
}

// ---- Simulator selection + status ----

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct SimConfig {
    #[serde(default)]
    kind: SimKind,
}

fn sim_config_path(app: &AppHandle) -> Result<PathBuf, UiError> {
    app.path()
        .app_config_dir()
        .map(|dir| dir.join(SIM_CONFIG_FILE))
        .map_err(|e| UiError::new("config_path", e.to_string()))
}

fn read_sim_config(app: &AppHandle) -> SimConfig {
    let Ok(path) = sim_config_path(app) else {
        return SimConfig::default();
    };
    if !path.exists() {
        return SimConfig::default();
    }
    match std::fs::read(&path).map(|b| serde_json::from_slice::<SimConfig>(&b)) {
        Ok(Ok(cfg)) => cfg,
        _ => SimConfig::default(),
    }
}

fn write_sim_config(app: &AppHandle, cfg: &SimConfig) -> Result<(), UiError> {
    let path = sim_config_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| UiError::new("config_write", e.to_string()))?;
    }
    let json = serde_json::to_vec_pretty(cfg)
        .map_err(|e| UiError::new("config_serialize", e.to_string()))?;
    std::fs::write(&path, json).map_err(|e| UiError::new("config_write", e.to_string()))
}

/// Apply the selected kind to whichever adapter handles it. Always
/// stops the inactive adapter so we never have both listening
/// simultaneously (= duplicate snapshots in `current_snapshot`).
fn apply_sim_kind(state: &tauri::State<'_, AppState>, kind: SimKind) {
    // Stop both adapters first; we'll start exactly one (or none) below.
    #[cfg(target_os = "windows")]
    {
        let mut msfs = state.msfs.lock().expect("msfs lock");
        msfs.stop();
    }
    {
        let mut xp = state.xplane.lock().expect("xplane lock");
        xp.stop();
    }

    if kind.is_msfs() {
        #[cfg(target_os = "windows")]
        {
            let mut msfs = state.msfs.lock().expect("msfs lock");
            msfs.start(kind);
        }
    } else if kind.is_xplane() {
        let mut xp = state.xplane.lock().expect("xplane lock");
        xp.start(kind);
    }
    // SimKind::Off → both stay stopped.
}

#[derive(Serialize, Default)]
pub struct SimStatus {
    /// "disconnected" | "connecting" | "connected"
    state: String,
    /// User-selected sim ("off" | "msfs2020" | "msfs2024" | "xplane11" | "xplane12").
    kind: String,
    snapshot: Option<SimSnapshot>,
    last_error: Option<String>,
    /// Whether the selected kind is actually implemented in this build.
    available: bool,
}

fn kind_str(kind: SimKind) -> &'static str {
    match kind {
        SimKind::Off => "off",
        SimKind::Msfs2020 => "msfs2020",
        SimKind::Msfs2024 => "msfs2024",
        SimKind::XPlane11 => "xplane11",
        SimKind::XPlane12 => "xplane12",
    }
}

/// `GET` the persisted sim selection.
#[tauri::command]
fn sim_get_kind(app: AppHandle) -> String {
    kind_str(read_sim_config(&app).kind).to_string()
}

/// Persist a new sim selection AND apply it to the running adapter.
/// Accepts: "off" | "msfs2020" | "msfs2024" | "xplane11" | "xplane12".
#[tauri::command]
fn sim_set_kind(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    kind: String,
) -> Result<(), UiError> {
    let parsed = match kind.as_str() {
        "off" => SimKind::Off,
        "msfs2020" => SimKind::Msfs2020,
        "msfs2024" => SimKind::Msfs2024,
        "xplane11" => SimKind::XPlane11,
        "xplane12" => SimKind::XPlane12,
        _ => return Err(UiError::new("invalid_sim_kind", format!("unknown kind: {kind}"))),
    };
    write_sim_config(&app, &SimConfig { kind: parsed })?;
    apply_sim_kind(&state, parsed);
    tracing::info!(?parsed, "sim kind selected");
    Ok(())
}

#[tauri::command]
fn sim_status(app: AppHandle, state: tauri::State<'_, AppState>) -> SimStatus {
    let kind = read_sim_config(&app).kind;

    // X-Plane path is available on every platform (UDP, no platform-
    // specific deps).
    if kind.is_xplane() {
        let adapter = state.xplane.lock().expect("xplane lock");
        let s = match adapter.state() {
            sim_xplane::ConnectionState::Disconnected => "disconnected",
            sim_xplane::ConnectionState::Connecting => "connecting",
            sim_xplane::ConnectionState::Connected => "connected",
        };
        return SimStatus {
            state: s.into(),
            kind: kind_str(kind).into(),
            snapshot: adapter.snapshot(),
            last_error: adapter.last_error(),
            available: true,
        };
    }

    // MSFS path is Windows-only.
    #[cfg(target_os = "windows")]
    {
        let adapter = state.msfs.lock().expect("msfs lock");
        if kind.is_msfs() {
            let s = match adapter.state() {
                sim_msfs::ConnectionState::Disconnected => "disconnected",
                sim_msfs::ConnectionState::Connecting => "connecting",
                sim_msfs::ConnectionState::Connected => "connected",
            };
            return SimStatus {
                state: s.into(),
                kind: kind_str(kind).into(),
                snapshot: adapter.snapshot(),
                last_error: adapter.last_error(),
                available: true,
            };
        }
        // SimKind::Off
        return SimStatus {
            state: "disconnected".into(),
            kind: kind_str(kind).into(),
            snapshot: None,
            last_error: None,
            available: kind == SimKind::Off,
        };
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = state;
        let last_error = if kind.is_msfs() {
            Some("MSFS adapter is Windows-only".into())
        } else {
            None
        };
        SimStatus {
            state: "disconnected".into(),
            kind: kind_str(kind).into(),
            snapshot: None,
            last_error,
            available: kind == SimKind::Off,
        }
    }
}

/// Pilot-initiated re-sync of the active sim adapter's cached
/// snapshot. Clears the lat/lon/etc. cache so `sim_status` returns
/// `state: "connecting"` + `snapshot: None` until the next genuine
/// frame arrives. Lets the briefing tab recover from edge cases the
/// automatic 5 s stale-timeout doesn't cover (sim paused while
/// changing flight, SimConnect trickles stale data, etc.) without
/// any "skip the gate" override semantics.
#[tauri::command]
fn sim_force_resync(app: AppHandle, state: tauri::State<'_, AppState>) {
    let kind = read_sim_config(&app).kind;
    if kind.is_xplane() {
        state.xplane.lock().expect("xplane lock").clear_snapshot();
        return;
    }
    #[cfg(target_os = "windows")]
    {
        if kind.is_msfs() {
            state.msfs.lock().expect("msfs lock").clear_snapshot();
        }
    }
}

// ---- PMDG SDK status (Phase H.4 / v0.2.0) ----
//
// The Settings tab needs to know:
//   * Is a PMDG aircraft loaded? (variant detected)
//   * Are we subscribed?
//   * Are we actually receiving data? (= SDK enabled in pilot's
//     options ini)
// to show the "SDK enabled?" hint when needed and to display the
// premium-telemetry status badge in the cockpit tab.

/// Minimal serializable PMDG status for the UI. Mirrors the
/// adapter's `PmdgStatus` but with the variant flattened to a
/// string so JSON parsing is trivial on the frontend side.
#[derive(serde::Serialize)]
struct PmdgStatusDto {
    /// `"ng3"`, `"x777"`, or `null` when no PMDG aircraft is loaded.
    variant: Option<&'static str>,
    /// True once SimConnect ClientData subscription has been
    /// successfully registered.
    subscribed: bool,
    /// True once at least one ClientData packet has actually arrived.
    /// `subscribed && !ever_received` (after a few seconds) is the
    /// signal that SDK isn't enabled in the pilot's options ini.
    ever_received: bool,
    /// Seconds since the last ClientData packet. `null` when never.
    stale_secs: Option<u64>,
    /// True when (variant detected, subscribed, but no data flowing
    /// for >5 s) — the heuristic for "SDK probably not enabled".
    /// UI shows the enable-hint modal when this is true.
    looks_like_sdk_disabled: bool,
}

#[tauri::command]
fn pmdg_status(state: tauri::State<'_, AppState>) -> PmdgStatusDto {
    #[cfg(target_os = "windows")]
    {
        let adapter = state.msfs.lock().expect("msfs lock");
        let s = adapter.pmdg_status();
        return PmdgStatusDto {
            variant: s.variant.map(|v| match v {
                sim_msfs::pmdg::PmdgVariant::Ng3 => "ng3",
                sim_msfs::pmdg::PmdgVariant::X777 => "x777",
            }),
            subscribed: s.subscribed,
            ever_received: s.ever_received,
            stale_secs: if s.stale_secs == u64::MAX {
                None
            } else {
                Some(s.stale_secs)
            },
            looks_like_sdk_disabled: s.looks_like_sdk_disabled(),
        };
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = state;
        PmdgStatusDto {
            variant: None,
            subscribed: false,
            ever_received: false,
            stale_secs: None,
            looks_like_sdk_disabled: false,
        }
    }
}

// ---- Live SimVar / LVar inspector (Settings → Debug) ----
//
// These commands forward to the MsfsAdapter's inspector watchlist
// (Phase B). Non-Windows targets just return errors / empty lists
// since the adapter is Windows-only anyway.

#[derive(serde::Deserialize)]
struct InspectorAddArgs {
    name: String,
    unit: String,
    /// "number" | "bool" | "string" — matches sim_msfs::WatchKind.
    kind: String,
}

#[tauri::command]
fn inspector_add(
    _state: tauri::State<'_, AppState>,
    args: InspectorAddArgs,
) -> Result<u32, UiError> {
    #[cfg(target_os = "windows")]
    {
        let kind = match args.kind.as_str() {
            "number" => sim_msfs::WatchKind::Number,
            "bool" => sim_msfs::WatchKind::Bool,
            "string" => sim_msfs::WatchKind::String,
            _ => {
                return Err(UiError::new(
                    "invalid_watch_kind",
                    format!("unknown kind: {}", args.kind),
                ))
            }
        };
        let trimmed_name = args.name.trim().to_string();
        let trimmed_unit = args.unit.trim().to_string();
        if trimmed_name.is_empty() {
            return Err(UiError::new(
                "empty_name",
                "SimVar / LVar name cannot be empty",
            ));
        }
        let adapter = _state.msfs.lock().expect("msfs lock");
        let id = adapter.add_watch(trimmed_name, trimmed_unit, kind);
        Ok(id)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = args;
        Err(UiError::new("unsupported", "inspector is Windows-only"))
    }
}

#[tauri::command]
fn inspector_remove(
    _state: tauri::State<'_, AppState>,
    id: u32,
) -> Result<(), UiError> {
    #[cfg(target_os = "windows")]
    {
        let adapter = _state.msfs.lock().expect("msfs lock");
        adapter.remove_watch(id);
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = id;
        Err(UiError::new("unsupported", "inspector is Windows-only"))
    }
}

#[tauri::command]
fn inspector_list(_state: tauri::State<'_, AppState>) -> Vec<serde_json::Value> {
    #[cfg(target_os = "windows")]
    {
        let adapter = _state.msfs.lock().expect("msfs lock");
        adapter
            .watches()
            .into_iter()
            .map(|w| serde_json::to_value(w).unwrap_or(serde_json::Value::Null))
            .collect()
    }
    #[cfg(not(target_os = "windows"))]
    {
        Vec::new()
    }
}

/// X-Plane DataRef inspector: returns the static catalog with the
/// most recent received value per entry. Unlike the MSFS Inspector
/// (where the user adds names manually) the X-Plane catalog is
/// fixed at compile time — every DataRef we subscribe is shown.
/// Pilots use this to verify the UDP feed is alive and the Sim is
/// responding to RREF subscriptions.
#[tauri::command]
fn xplane_inspector_list(state: tauri::State<'_, AppState>) -> Vec<serde_json::Value> {
    let adapter = state.xplane.lock().expect("xplane lock");
    adapter
        .subscribed_datarefs()
        .into_iter()
        .map(|s| serde_json::to_value(s).unwrap_or(serde_json::Value::Null))
        .collect()
}

/// Best-effort detection of the X-Plane install path. Returns the
/// absolute path as a string when found, or `null` when nothing
/// plausible exists — UI then falls back to a folder picker.
///
/// MUST be async + `spawn_blocking`: the Windows path runs
/// `reg.exe query` (sub-process spawn ~200-800ms cold-start) plus
/// up to 4 filesystem `is_dir` probes. Doing that on a sync
/// `#[tauri::command]` blocks the Tauri IPC bandwidth and freezes
/// the Settings panel during its first paint (v0.5.0 release-day
/// regression — pilot reported jerky scrolling + language-switcher
/// hang while the registry probe was running).
#[tauri::command]
async fn xplane_detect_install_path() -> Option<String> {
    tauri::async_runtime::spawn_blocking(|| {
        xplane_plugin_install::detect_install_path()
            .map(|p| p.to_string_lossy().into_owned())
    })
    .await
    .unwrap_or(None)
}

/// Download the matching plugin zip from this AeroACARS version's
/// GitHub release and extract it into `<install_dir>/Resources/
/// plugins/AeroACARS/`. Idempotent — overwrites in place. Returns
/// the install summary (path, bytes, files) on success, or an
/// error message string on failure.
#[tauri::command]
async fn xplane_install_plugin(
    install_dir: String,
) -> Result<xplane_plugin_install::PluginInstallResult, String> {
    let path = std::path::PathBuf::from(install_dir);
    xplane_plugin_install::install_plugin(&path).await
}

/// Remove the plugin folder from the given X-Plane install. No-op
/// when no plugin folder exists. Async + `spawn_blocking` so a slow
/// `remove_dir_all` (Windows Defender scanning, network drives, etc.)
/// can't stall IPC.
#[tauri::command]
async fn xplane_uninstall_plugin(install_dir: String) -> Result<(), String> {
    let path = std::path::PathBuf::from(install_dir);
    tauri::async_runtime::spawn_blocking(move || {
        xplane_plugin_install::uninstall_plugin(&path)
    })
    .await
    .map_err(|e| format!("worker thread panicked: {e}"))?
}

/// Status of the optional AeroACARS X-Plane Plugin (v0.5.0+
/// "Premium Mode"). The Cockpit tab uses this to show a green
/// "X-PLANE PREMIUM" badge when frame-perfect telemetry is flowing.
///
/// Return shape (JSON):
///   {
///     "active":      bool,    // packets in last 3 s
///     "ever_seen":   bool,    // any packet this session
///     "packet_count": u64,    // total since adapter start
///     "last_error":  string?  // bind-failure reason, if any
///   }
///
/// Inert when the active sim isn't X-Plane (returns all-false).
#[tauri::command]
fn xplane_premium_status(state: tauri::State<'_, AppState>) -> serde_json::Value {
    let adapter = state.xplane.lock().expect("xplane lock");
    let s = adapter.premium_status();
    let err = adapter.premium_last_error();
    serde_json::json!({
        "active": s.active,
        "ever_seen": s.ever_seen,
        "packet_count": s.packet_count,
        "last_error": err,
    })
}

/// Probe both potential simulators and return a suggested SimKind.
/// Used on first launch when no sim is configured, OR via a
/// "Detect Sim" button in Settings.
///
/// Order:
///   1. UDP probe to 127.0.0.1:49000 — if X-Plane responds, return
///      X-Plane (12 by default; we have no protocol-level way to
///      tell 11 vs 12 apart).
///   2. SimConnect_Open probe (Windows only) — if MSFS is running,
///      return MSFS 2024.
///   3. Otherwise: SimKind::Off.
///
/// Returns the kind as a string matching the `sim_set_kind` API:
///   "off" | "msfs2020" | "msfs2024" | "xplane11" | "xplane12"
#[tauri::command]
async fn detect_running_sim() -> String {
    // Run the UDP probe on a blocking-tasks thread so the 500 ms
    // worst-case latency doesn't tie up the Tauri event loop.
    let xp = tauri::async_runtime::spawn_blocking(sim_xplane::is_xplane_running)
        .await
        .unwrap_or(false);
    if xp {
        tracing::info!("auto-detect: X-Plane responding on UDP 49000");
        return "xplane12".to_string();
    }
    // MSFS probe: simplest is to try opening SimConnect briefly. We
    // don't have a non-invasive variant yet (Phase 3); for now skip
    // the MSFS probe and let the user pick manually if X-Plane isn't
    // up. The active MsfsAdapter will report Connected if MSFS is
    // there anyway, so detection can be inferred from that signal.
    "off".to_string()
}

/// On login or session restore, check the on-disk active-flight file. If it's
/// recent enough, recreate the in-memory ActiveFlight and restart position
/// streaming — picks up exactly where the previous run left off.
async fn try_resume_flight(
    app: &AppHandle,
    state: &tauri::State<'_, AppState>,
    client: &Client,
) {
    let Some(persisted) = read_persisted_flight(app) else {
        return;
    };
    // Drop sessions that are clearly stale (e.g. a flight from days ago) so
    // we don't keep flogging a long-dead PIREP forever.
    let age = Utc::now() - persisted.started_at;
    if age > chrono::Duration::hours(RESUME_MAX_AGE_HOURS) {
        tracing::info!(
            pirep_id = %persisted.pirep_id,
            age_hours = age.num_hours(),
            "discarding stale persisted flight"
        );
        clear_persisted_flight(app);
        return;
    }

    // Verify the PIREP still exists server-side BEFORE rebuilding the
    // streamer. phpVMS's RemoveExpiredLiveFlights cron may have soft-deleted
    // it while the client was offline (heartbeat couldn't fire), in which
    // case adopting locally would just produce a wave of 404s. A 404 here
    // means the PIREP is gone — clear the disk record and let the user
    // re-prefile cleanly. Other errors (network, auth) are non-fatal —
    // we proceed and let the streamer's own error handling kick in.
    match client.get_pirep(&persisted.pirep_id).await {
        Ok(p) => {
            // State 0 = IN_PROGRESS. Anything else means the PIREP was
            // filed / cancelled / accepted elsewhere — there's nothing for
            // us to resume into.
            if p.state.is_some() && p.state != Some(0) {
                tracing::info!(
                    pirep_id = %persisted.pirep_id,
                    state = ?p.state,
                    status = ?p.status,
                    "persisted PIREP no longer in progress, discarding resume"
                );
                clear_persisted_flight(app);
                log_activity_handle(
                    app,
                    ActivityLevel::Warn,
                    "Gespeicherter Flug nicht mehr aktiv".to_string(),
                    Some(format!(
                        "PIREP {} hat serverseitig Status {:?} — Resume verworfen.",
                        persisted.pirep_id, p.status
                    )),
                );
                return;
            }
        }
        Err(ApiError::NotFound) => {
            tracing::info!(
                pirep_id = %persisted.pirep_id,
                "persisted PIREP no longer on server (likely soft-deleted by cron), discarding resume"
            );
            clear_persisted_flight(app);
            log_activity_handle(
                app,
                ActivityLevel::Warn,
                "Gespeicherter Flug existiert nicht mehr".to_string(),
                Some(
                    "phpVMS hat den PIREP entfernt (vermutlich Inaktivitäts-Timeout). \
                     Bitte den Flug neu starten oder als Manual-PIREP einreichen."
                        .to_string(),
                ),
            );
            return;
        }
        Err(e) => {
            tracing::warn!(
                pirep_id = %persisted.pirep_id,
                error = %e,
                "could not verify persisted PIREP — proceeding with resume anyway"
            );
        }
    }

    // Same guard as flight_start / flight_adopt: if another resume is
    // already running (StrictMode double-mount in dev fires
    // phpvms_load_session twice in close succession), bail silently.
    // Without this, the second resume would do a duplicate get_bids /
    // get_aircraft round-trip even though the first had already won.
    let _setup_guard = match FlightSetupGuard::try_acquire(&state.flight_setup_in_progress) {
        Ok(g) => g,
        Err(_) => {
            tracing::debug!("resume already in progress, skipping duplicate call");
            return;
        }
    };

    {
        let guard = state.active_flight.lock().expect("active_flight lock");
        if guard.is_some() {
            // Expected on second mount under React StrictMode (dev) — the
            // first mount already restored the flight. Idempotent, no WARN.
            tracing::debug!("active flight already in memory, skipping resume");
            return;
        }
    }

    tracing::info!(
        pirep_id = %persisted.pirep_id,
        age_minutes = age.num_minutes(),
        "resuming in-progress flight"
    );
    record_event(
        app,
        &persisted.pirep_id,
        &FlightLogEvent::FlightResumed {
            timestamp: Utc::now(),
            pirep_id: persisted.pirep_id.clone(),
            age_minutes: age.num_minutes(),
        },
    );

    // Backfill missing airline ICAO + planned registration. PersistedFlight
    // defaults to "" when the JSON pre-dates these fields, or when the flight
    // was originally adopted from a discovered PIREP that didn't have a
    // matching bid. Try get_bids() once on resume; if nothing matches we
    // leave them empty and the UI renders without them — same as before.
    let mut airline_icao = persisted.airline_icao.clone();
    let mut planned_registration = persisted.planned_registration.clone();
    if airline_icao.is_empty() || planned_registration.is_empty() {
        if let Ok(bids) = client.get_bids().await {
            if let Some(matching) = bids
                .iter()
                .find(|b| b.flight.flight_number == persisted.flight_number)
            {
                if airline_icao.is_empty() {
                    if let Some(a) = matching.flight.airline.as_ref() {
                        airline_icao = a.icao.clone();
                    }
                }
                if planned_registration.is_empty() {
                    if let Some(id) = matching
                        .flight
                        .simbrief
                        .as_ref()
                        .and_then(|sb| sb.aircraft_id)
                    {
                        if let Ok(details) = client.get_aircraft(id).await {
                            planned_registration = details
                                .registration
                                .unwrap_or_default()
                                .trim()
                                .to_string();
                        }
                    }
                }
            }
        }
    }

    // Restore stats so the resumed flight continues with the distance,
    // fuel-burn and phase the streamer had right before the crash.
    // Without this, every resume produces a "0 distance / 0 fuel" PIREP
    // because we'd start the stats from zero again.
    let mut restored_stats = FlightStats::new();
    persisted.stats.clone().apply_to(&mut restored_stats);
    tracing::info!(
        distance_nm = restored_stats.distance_nm,
        position_count = restored_stats.position_count,
        ?restored_stats.phase,
        "restored flight stats from disk"
    );

    let flight = Arc::new(ActiveFlight {
        pirep_id: persisted.pirep_id.clone(),
        bid_id: persisted.bid_id,
        started_at: persisted.started_at,
        airline_icao,
        // Resume-Pfad: Airline-Logo-URL aus der persisted Bid wieder
        // herholen. Wenn die Persisted-Snapshot-Version das Feld nicht
        // hatte (alter Build vor v0.4.0), bleibt None.
        airline_logo_url: persisted.airline_logo_url.clone(),
        planned_registration,
        // v0.3.0: Aircraft-Type bei Resume aus dem persisted Flight ist
        // nicht direkt verfügbar — der frühere Stand hatte das nicht.
        // Beim Resume bleibt der Custom-Field "Aircraft Type" daher
        // leer. Akzeptabel, weil Resume eh nur passiert wenn der
        // Pilot mitten im Flug die App neu startet — der ursprüngliche
        // PIREP hat die Aircraft-Daten phpVMS-seitig schon.
        aircraft_icao: String::new(),
        aircraft_name: String::new(),
        flight_number: persisted.flight_number.clone(),
        dpt_airport: persisted.dpt_airport.clone(),
        arr_airport: persisted.arr_airport.clone(),
        fares: persisted.fares.clone(),
        stats: Mutex::new(restored_stats),
        stop: AtomicBool::new(false),
        // Flag the UI to surface a 10-second confirmation banner before any
        // position posts go out. The streamer is intentionally NOT spawned
        // here — `flight_resume_confirm` does that once the user accepts (or
        // the timer expires), or `flight_cancel` aborts the PIREP.
        was_just_resumed: AtomicBool::new(true),
        streamer_spawned: AtomicBool::new(false),
        cancelled_remotely: AtomicBool::new(false),
    });

    {
        let mut guard = state.active_flight.lock().expect("active_flight lock");
        *guard = Some(flight);
    }
    // NOTE: do not spawn the streamer here. The frontend-driven
    // `flight_resume_confirm` does it after the resume banner is dismissed.
}

// ---- Bootstrap ----

// ---- Auto-start watcher ----
//
// Polls every AUTO_START_INTERVAL_SECS while enabled. When the
// aircraft is parked at the departure airport of one of the user's
// bids AND the loaded aircraft type/registration matches the bid,
// it fires `flight_start` for that bid. Stops re-firing for the
// same bid until the resulting flight ends (tracked via
// `AppState::auto_start_last_bid_id`).
const AUTO_START_INTERVAL_SECS: u64 = 3;
/// How close to the departure airport (in meters) the aircraft must
/// be to trigger auto-start. 5 km is generous enough for any major
/// airport's stand area; tighter and we'd miss bids where the pilot
/// spawns at a distant gate.
const AUTO_START_PROXIMITY_M: f64 = 5_000.0;

#[tauri::command]
fn auto_start_get_enabled(state: tauri::State<'_, AppState>) -> bool {
    state.auto_start_enabled.load(Ordering::Relaxed)
}

/// v0.3.0: Persistenz-Pfad für Auto-Start-Setting. localStorage geht im
/// Tauri-Dev-Mode bzw. nach Force-Kill der App regelmäßig verloren —
/// Backend-File überlebt jeden Restart und ist die Source-of-Truth.
fn auto_start_settings_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|p| p.join("auto_start.json"))
}

fn read_auto_start_persisted(app: &AppHandle) -> bool {
    let path = match auto_start_settings_path(app) {
        Some(p) => p,
        None => return false,
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return false,
    };
    // Datei-Format ist trivial: `{"enabled": true}` oder `{"enabled": false}`.
    // Kein serde-Overkill, plain regex-style match.
    text.contains("\"enabled\":true") || text.contains("\"enabled\": true")
}

fn write_auto_start_persisted(app: &AppHandle, enabled: bool) {
    let Some(path) = auto_start_settings_path(app) else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let body = format!("{{\"enabled\":{}}}", enabled);
    let _ = std::fs::write(&path, body);
}

/// v0.3.0: Frontend-API für den Auto-Start-Skip-Grund. Liefert den
/// aktuellen Reason-Code (engines_on / moving / airborne) plus das
/// Alter in Sekunden seit der Watcher den letzten Skip festgestellt
/// hat. Frontend rendert daraus einen Banner im Briefing-Tab — der
/// Pilot wartet ja nicht im Settings-Log auf Hinweise.
///
/// Liefert `None` wenn alles passt (kein Skip kürzlich) oder Auto-
/// Start aus ist.
#[tauri::command]
fn auto_start_skip_status(
    state: tauri::State<'_, AppState>,
) -> Option<AutoStartSkipDto> {
    if !state.auto_start_enabled.load(Ordering::Relaxed) {
        return None;
    }
    let g = state.auto_start_skip_reason.lock().unwrap();
    let (at, code) = g.as_ref()?;
    let age_secs = (Utc::now() - *at).num_seconds();
    // Älter als 10 s: vermutlich nicht mehr aktuell — Frontend soll
    // den Banner nicht zeigen.
    if age_secs > 10 {
        return None;
    }
    Some(AutoStartSkipDto {
        reason: code.clone(),
        age_secs,
    })
}

#[derive(Debug, Clone, Serialize)]
struct AutoStartSkipDto {
    reason: String,
    age_secs: i64,
}

#[tauri::command]
fn auto_start_set_enabled(
    enabled: bool,
    app: AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), UiError> {
    let was = state.auto_start_enabled.swap(enabled, Ordering::Relaxed);
    // v0.3.0: Persistieren in JSON-File damit der Toggle-Stand auch
    // nach App-Restart / Force-Kill / Dev-Build erhalten bleibt.
    write_auto_start_persisted(&app, enabled);
    // The watcher task itself runs for the entire app lifetime (spawned
    // once in `.setup()`); flipping this flag is enough — no need to
    // spawn or kill anything here. Earlier versions did spawn-on-toggle,
    // which had a first-launch failure mode on Mac where the watcher
    // never came up at all and toggling off→on didn't recover (because
    // the gate `!was` blocked re-spawn after an IPC race).
    if enabled && !was {
        log_activity_handle(
            &app,
            ActivityLevel::Info,
            "Auto-Start aktiviert".to_string(),
            Some("Watcher prüft alle 3 s ob ein Bid + Departure-Airport zueinander passen".to_string()),
        );
    } else if !enabled && was {
        log_activity_handle(
            &app,
            ActivityLevel::Info,
            "Auto-Start deaktiviert".to_string(),
            None,
        );
    }
    Ok(())
}

/// Spawn the auto-start watcher task. Idempotent in practice:
/// the body checks `auto_start_enabled` on every tick and returns
/// when false, so spawning twice just means one drops out quickly.
fn spawn_auto_start_watcher(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tracing::info!("auto-start watcher started");
        loop {
            tokio::time::sleep(Duration::from_secs(AUTO_START_INTERVAL_SECS)).await;
            let state = app.state::<AppState>();
            // No-op when disabled — the watcher itself runs forever now
            // (spawned once at app start), the toggle just gates the
            // body. Used to `break` and rely on a re-spawn on next
            // toggle, which raced with first-launch IPC on Mac.
            if !state.auto_start_enabled.load(Ordering::Relaxed) {
                continue;
            }
            // Skip if a flight is already active.
            {
                let guard = state.active_flight.lock().expect("active_flight lock");
                if guard.is_some() {
                    continue;
                }
            }
            // Skip if no sim snapshot yet, or the aircraft is moving /
            // engines on (= already taxiing or rolling, not "ready to
            // fire" anymore). v0.3.0: bei jedem Skip einen kurzen
            // Hint im Activity-Log, damit der Pilot weiß WARUM Auto-
            // Start nicht greift. Throttled auf 1× / 60 s pro reason
            // damit der Log nicht spamt.
            let Some(snap) = current_snapshot(&app) else {
                continue;
            };
            // v0.3.0: Race-Condition-Fix nach Sim-Reconnect. Wenn der
            // Sim gerade frisch connected ist (Pilot hat X-Plane neu
            // gestartet), hat der Snapshot kurz Default-0-Werte
            // (engines=0, on_ground=true, gs=0) — die ALLE die
            // Auto-Start-Bedingungen "scheinbar erfüllen" und der
            // Watcher fired BEVOR die echten Daten reinkommen
            // (Triebwerke an, Aircraft-Title gesetzt etc.). Pilot
            // hat dann ungewollt einen Auto-Start mit laufenden
            // Triebwerken bekommen.
            //
            // Lösung: Wir prüfen ob die Snapshot-Daten "warm" sind
            // (Aircraft-Title vorhanden + Fuel > 100 kg = echtes
            // Aircraft, nicht Default). Wenn nicht warm, skip ohne
            // Hint-Banner — die Daten settled binnen 1-3 s und der
            // Pilot will dafür keinen Banner sehen.
            let sim_data_warm = snap
                .aircraft_title
                .as_deref()
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false)
                && snap.fuel_total_kg > 100.0;
            if !sim_data_warm {
                tracing::debug!("auto-start: sim data not warm yet — skipping");
                continue;
            }
            let skip_reason: Option<(&'static str, &'static str)> = if !snap.on_ground {
                Some(("airborne", "Du bist in der Luft — Auto-Start funktioniert nur am Boden"))
            } else if snap.groundspeed_kt > 5.0 {
                Some(("moving", "Flugzeug rollt schon — Auto-Start greift nur am Stand"))
            } else if snap.engines_running > 0 {
                Some(("engines_on", "Triebwerke sind an — Auto-Start greift nur bei kalt/stehendem Flugzeug. Manuell 'Flug starten' nutzen."))
            } else {
                None
            };
            if let Some((reason_code, reason_msg)) = skip_reason {
                // Throttle: nur loggen wenn wir den Grund 60s+ nicht
                // gemeldet haben oder der Grund neu ist.
                let now = Utc::now();
                let should_log = {
                    let mut g = state.auto_start_skip_reason.lock().unwrap();
                    let log_it = match g.as_ref() {
                        None => true,
                        Some((_, last_code)) if last_code != reason_code => true,
                        Some((last_at, _))
                            if (now - *last_at).num_seconds() >= 60 =>
                        {
                            true
                        }
                        _ => false,
                    };
                    if log_it {
                        *g = Some((now, reason_code.to_string()));
                    }
                    log_it
                };
                if should_log {
                    log_activity_handle(
                        &app,
                        ActivityLevel::Info,
                        "Auto-Start: nicht möglich".to_string(),
                        Some(reason_msg.to_string()),
                    );
                }
                continue;
            }
            // Voraussetzungen erfüllt — Reason-State löschen damit beim
            // nächsten skip wieder ein frischer Hint kommt.
            {
                let mut g = state.auto_start_skip_reason.lock().unwrap();
                *g = None;
            }
            // Fetch the user's bids to find a match.
            let client = match {
                let g = state.client.lock().expect("client lock");
                g.clone()
            } {
                Some(c) => c,
                None => continue, // not logged in
            };
            let bids = match client.get_bids().await {
                Ok(v) => v,
                Err(_) => continue,
            };
            // Don't re-fire for the same bid within the same parked
            // session. Cleared in `flight_end` via the active_flight
            // guard above resetting on the next tick.
            let last_bid = {
                let g = state.auto_start_last_bid_id.lock().unwrap();
                *g
            };
            for bid in &bids {
                if Some(bid.id) == last_bid {
                    continue;
                }
                if !bid_matches_current_state(bid, &snap) {
                    continue;
                }
                // Match — fire flight_start.
                tracing::info!(
                    bid_id = bid.id,
                    flight = %bid.flight.flight_number,
                    "auto-start triggering flight_start"
                );
                log_activity_handle(
                    &app,
                    ActivityLevel::Info,
                    format!(
                        "Auto-Start: {} {} → {}",
                        bid.flight.flight_number,
                        bid.flight.dpt_airport_id,
                        bid.flight.arr_airport_id
                    ),
                    Some(format!(
                        "Aircraft {} matched bid {}",
                        snap.aircraft_title.as_deref().unwrap_or("(unknown)"),
                        bid.id
                    )),
                );
                {
                    let mut g = state.auto_start_last_bid_id.lock().unwrap();
                    *g = Some(bid.id);
                }
                let app_for_call = app.clone();
                let bid_id = bid.id;
                tauri::async_runtime::spawn(async move {
                    let state_ref = app_for_call.state::<AppState>();
                    if let Err(e) =
                        flight_start(app_for_call.clone(), state_ref, bid_id).await
                    {
                        tracing::warn!(
                            ?e,
                            bid_id,
                            "auto-start: flight_start command failed"
                        );
                        // Clear the last_bid_id so the user can retry
                        // by toggling auto-start off/on or fixing the
                        // condition (e.g. wrong aircraft).
                        let s = app_for_call.state::<AppState>();
                        let mut g = s.auto_start_last_bid_id.lock().unwrap();
                        *g = None;
                    }
                });
                break;
            }
        }
    });
}

/// Does the loaded aircraft + position match this bid's expectations?
///
/// MVP version: airport-proximity match only — pilot must be parked
/// within `AUTO_START_PROXIMITY_M` of the bid's departure airport.
/// Aircraft-type matching is not required because resolving the
/// bid's planned aircraft requires an extra `get_aircraft` API call,
/// and shipping that on every 3 s tick is too much. If multiple bids
/// match the same airport, the first one wins; pilot can cancel
/// the auto-started flight to fall through to the next.
fn bid_matches_current_state(bid: &Bid, snap: &SimSnapshot) -> bool {
    let Some((apt_lat, apt_lon)) = runway::airport_position(&bid.flight.dpt_airport_id) else {
        return false;
    };
    let dist = runway::distance_m(snap.lat, snap.lon, apt_lat, apt_lon);
    dist <= AUTO_START_PROXIMITY_M
}

/// What state the tray icon is currently expressing — drives the
/// icon color, tooltip text, and the "status" header item in the
/// right-click menu. Computed from the active flight's stats every
/// `TRAY_UPDATE_INTERVAL_SECS` so the pilot can glance at the
/// taskbar/menubar and immediately tell what AeroACARS is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrayState {
    /// No active flight. Default app icon (template-friendly).
    Idle,
    /// Active flight, heartbeat fresh, no offline backlog.
    Active,
    /// Active flight but heartbeat is stale OR position queue is
    /// piling up. Visible warning so pilot can investigate.
    Warning,
    /// Critical: PIREP cancelled remotely (404 from heartbeat) or
    /// streamer hasn't posted in a long time. Plus a one-shot
    /// system notification when first transitioning into this state.
    Error,
}

const TRAY_UPDATE_INTERVAL_SECS: u64 = 5;
/// Heartbeat older than this → tray flips to Warning.
const TRAY_HEARTBEAT_STALE_SECS: i64 = 60;
/// Position queue with more than this many pending → Warning.
const TRAY_QUEUE_WARN_THRESHOLD: u32 = 5;

/// Snapshot of what the tray should currently show. Held by
/// `AppState` so the periodic updater can diff against the previous
/// state to (a) avoid pointless icon swaps and (b) detect
/// transitions that warrant a system notification.
#[derive(Debug, Clone)]
struct TrayDisplay {
    state: TrayState,
    tooltip: String,
    /// Human-readable header to show as the first (disabled) menu
    /// entry, or `None` to omit the header (idle case).
    menu_header: Option<String>,
}

fn compute_tray_display(app: &AppHandle) -> TrayDisplay {
    let state = app.state::<AppState>();
    let guard = state.active_flight.lock().expect("active_flight lock");
    let Some(flight) = guard.as_ref() else {
        return TrayDisplay {
            state: TrayState::Idle,
            tooltip: "AeroACARS".to_string(),
            menu_header: None,
        };
    };
    let stats = flight.stats.lock().expect("flight stats");

    // Detect critical conditions first.
    let cancelled = flight.cancelled_remotely.load(Ordering::Relaxed);
    let now = Utc::now();
    let heartbeat_age_s = stats
        .last_heartbeat_at
        .map(|t| (now - t).num_seconds())
        .unwrap_or(i64::MAX);
    let queue = stats.queued_position_count;
    let phase_label = phase_human_label(stats.phase);
    let callsign = format_callsign(&flight.airline_icao, &flight.flight_number);
    let route = format!("{} → {}", flight.dpt_airport, flight.arr_airport);

    let s = if cancelled {
        TrayState::Error
    } else if heartbeat_age_s > TRAY_HEARTBEAT_STALE_SECS
        || queue >= TRAY_QUEUE_WARN_THRESHOLD
    {
        TrayState::Warning
    } else {
        TrayState::Active
    };

    let tooltip = match s {
        TrayState::Error => format!(
            "AeroACARS — {} {}\nPIREP serverseitig gecancelt",
            callsign, route
        ),
        TrayState::Warning => {
            let mut parts = vec![format!("AeroACARS — {} {}", callsign, route)];
            if heartbeat_age_s > TRAY_HEARTBEAT_STALE_SECS && stats.last_heartbeat_at.is_some() {
                parts.push(format!("Heartbeat stale ({}s)", heartbeat_age_s));
            } else if stats.last_heartbeat_at.is_none() {
                parts.push("Warte auf ersten Heartbeat".to_string());
            }
            if queue >= TRAY_QUEUE_WARN_THRESHOLD {
                parts.push(format!("{} Positionen offline gestaut", queue));
            }
            parts.join("\n")
        }
        TrayState::Active => {
            let mut t = format!("AeroACARS — {} {} · {}", callsign, route, phase_label);
            if let Some(t0) = stats.last_heartbeat_at {
                let age = (now - t0).num_seconds();
                t.push_str(&format!("\nHeartbeat vor {}s", age));
            }
            t
        }
        TrayState::Idle => "AeroACARS".to_string(),
    };

    let menu_header = Some(format!("✈ {} · {}", callsign, phase_label));

    TrayDisplay {
        state: s,
        tooltip,
        menu_header,
    }
}

/// Build the system-tray icon with a Show / Quit context menu and
/// click-to-toggle behaviour. Called once during Tauri setup; the
/// resulting tray lives for the app lifetime — when the window is
/// hidden via the minimize-to-tray feature this icon is the only
/// way back to the app.
///
/// On Windows the icon shows in the system tray (bottom-right);
/// on macOS in the menubar (top-right) — same code, the OS routes
/// it to the platform-native location.
/// Build the tray context menu with the given status-header label.
/// Extracted so the periodic tray updater can rebuild and swap it
/// when the active flight's state changes (Tauri 2's TrayIcon has
/// `set_menu` but no menu getter, so we build fresh each time).
fn build_tray_menu(
    app: &AppHandle,
    status_label: &str,
) -> Result<tauri::menu::Menu<tauri::Wry>, Box<dyn std::error::Error>> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
    let status_item = MenuItem::with_id(app, "status", status_label, false, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let show_item = MenuItem::with_id(app, "show", "Anzeigen / Show", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Beenden / Quit", true, None::<&str>)?;
    Ok(Menu::with_items(
        app,
        &[&status_item, &separator, &show_item, &quit_item],
    )?)
}

fn build_tray_icon(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
    use tauri::Manager;

    let menu = build_tray_menu(app, "AeroACARS — kein aktiver Flug")?;

    // Idle icon = the default app icon, untouched. The periodic
    // tray-updater swaps to a status-badged variant when there's an
    // active flight (see `make_status_icon`).
    let icon = app
        .default_window_icon()
        .ok_or("no default window icon")?
        .clone();

    TrayIconBuilder::with_id("aeroacars-tray")
        .tooltip("AeroACARS")
        .icon(icon)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.unminimize();
                    let _ = w.set_focus();
                }
            }
            "quit" => {
                // Bypass the CloseRequested → hide path by calling
                // app.exit() directly. Same effect as toggling
                // minimize-to-tray off and clicking X, but explicit.
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            // Left-click on the tray icon toggles the main window's
            // visibility. Right-click opens the context menu (handled
            // by Tauri itself because show_menu_on_left_click=false).
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(w) = app.get_webview_window("main") {
                    if w.is_visible().unwrap_or(false) {
                        let _ = w.hide();
                    } else {
                        let _ = w.show();
                        let _ = w.unminimize();
                        let _ = w.set_focus();
                    }
                }
            }
        })
        .build(app)?;

    Ok(())
}

/// Color used for the status badge in the bottom-right corner of the
/// tray icon. RGBA. Idle has no badge — falls through to default
/// app icon.
fn tray_state_color(state: TrayState) -> Option<[u8; 4]> {
    match state {
        TrayState::Idle => None,
        // Calm Apple-system green for "all good".
        TrayState::Active => Some([0x30, 0xD1, 0x58, 0xFF]),
        // Warm amber for "look at me".
        TrayState::Warning => Some([0xFF, 0x9F, 0x0A, 0xFF]),
        // Strong Apple-system red for "PIREP gone".
        TrayState::Error => Some([0xFF, 0x45, 0x3A, 0xFF]),
    }
}

/// Produce a tray-icon variant of `base` with a colored status circle
/// painted in the bottom-right corner. The badge is sized to the
/// lower-right ~35 % of the icon and has a 1-pixel transparent ring
/// around it so it visually pops off the underlying logo.
///
/// Returns `None` for `TrayState::Idle` — caller uses the unmodified
/// base icon in that case.
fn make_status_icon<'a>(
    base: &tauri::image::Image<'a>,
    state: TrayState,
) -> Option<tauri::image::Image<'static>> {
    let color = tray_state_color(state)?;
    let w = base.width();
    let h = base.height();
    let mut rgba: Vec<u8> = base.rgba().to_vec();

    // Badge geometry — bottom-right circle covering ~35 % of the icon.
    let badge_d = (w.min(h) as f32 * 0.55) as i32;
    let r = badge_d / 2;
    let cx = w as i32 - r - 1;
    let cy = h as i32 - r - 1;
    // 1-pixel transparent halo for visual separation.
    let r_outer = r + 1;

    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let dx = x - cx;
            let dy = y - cy;
            let d2 = dx * dx + dy * dy;
            let idx = ((y as u32 * w + x as u32) * 4) as usize;
            if d2 <= r * r {
                // Solid badge color.
                rgba[idx] = color[0];
                rgba[idx + 1] = color[1];
                rgba[idx + 2] = color[2];
                rgba[idx + 3] = color[3];
            } else if d2 <= r_outer * r_outer {
                // Halo — clear to transparent.
                rgba[idx + 3] = 0;
            }
        }
    }
    Some(tauri::image::Image::new_owned(rgba, w, h))
}

/// Periodic background task: every `TRAY_UPDATE_INTERVAL_SECS`,
/// recompute the desired tray display from the active flight's
/// stats and push it to the live `TrayIcon`. Diff against the
/// previous state to avoid pointless icon-repaints AND to detect
/// the transition into `Error` (which triggers a one-shot system
/// notification so a tray-only pilot can't miss it).
fn spawn_tray_updater(app: AppHandle) {
    use std::sync::Mutex;
    use tauri::tray::TrayIconId;
    let tray_id = TrayIconId::new("aeroacars-tray");
    let last_state: std::sync::Arc<Mutex<Option<TrayState>>> =
        std::sync::Arc::new(Mutex::new(None));
    tauri::async_runtime::spawn(async move {
        // Cache the colored icon variants once at startup so we
        // don't recompute the pixel buffer on every 5 s tick.
        let base_icon = match app.default_window_icon() {
            Some(i) => i.clone(),
            None => {
                tracing::warn!("no default window icon — tray-updater giving up");
                return;
            }
        };
        let active_icon = make_status_icon(&base_icon, TrayState::Active);
        let warning_icon = make_status_icon(&base_icon, TrayState::Warning);
        let error_icon = make_status_icon(&base_icon, TrayState::Error);

        loop {
            tokio::time::sleep(Duration::from_secs(TRAY_UPDATE_INTERVAL_SECS)).await;

            let display = compute_tray_display(&app);
            let prev = {
                let g = last_state.lock().unwrap();
                *g
            };
            let mut g = last_state.lock().unwrap();
            *g = Some(display.state);
            drop(g);

            // Push tooltip + (rebuilt) menu so the status header
            // reflects the current flight info. Tauri 2's TrayIcon
            // exposes set_menu() but not a getter, so we rebuild the
            // menu structure each tick — cheap (3 native menu items)
            // and avoids storing MenuItem<Wry> lifetimes in AppState.
            if let Some(tray) = app.tray_by_id(&tray_id) {
                let _ = tray.set_tooltip(Some(&display.tooltip));

                let header_label = display
                    .menu_header
                    .clone()
                    .unwrap_or_else(|| "AeroACARS — kein aktiver Flug".to_string());
                if let Ok(new_menu) = build_tray_menu(&app, &header_label) {
                    let _ = tray.set_menu(Some(new_menu));
                }

                // Icon swap only on state change (it's a relatively
                // expensive call on Win, and Mac sometimes flickers).
                if prev != Some(display.state) {
                    let new_icon = match display.state {
                        TrayState::Idle => Some(base_icon.clone()),
                        TrayState::Active => active_icon.clone(),
                        TrayState::Warning => warning_icon.clone(),
                        TrayState::Error => error_icon.clone(),
                    };
                    if let Some(icon) = new_icon {
                        let _ = tray.set_icon(Some(icon));
                    }
                }
            }

            // Fire a one-shot system notification when we just
            // entered Error. Pilots running in tray-only mode would
            // otherwise miss the "PIREP cancelled" event entirely
            // (the activity-log entry is invisible until they
            // re-open the window).
            if prev != Some(TrayState::Error) && display.state == TrayState::Error {
                use tauri_plugin_notification::NotificationExt;
                let _ = app
                    .notification()
                    .builder()
                    .title("AeroACARS")
                    .body(&display.tooltip)
                    .show();
            }
        }
    });
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,aeroacars=debug"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_tracing();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "AeroACARS starting");

    // Restore the activity log from disk before anything else uses
    // AppState. Pre-populates the in-memory VecDeque so the pilot
    // sees the entire flight history (phase changes, lights, METAR
    // fetches, etc.) when they re-open Tauri mid-flight, instead of
    // an empty log every time.
    let app_state = AppState::default();
    {
        let restored = load_activity_log_at_boot();
        let mut log = app_state.activity_log.lock().expect("activity_log lock");
        for entry in restored {
            log.push_back(entry);
            while log.len() > ACTIVITY_LOG_CAPACITY {
                log.pop_front();
            }
        }
        // Then append the new "App started" banner on top of the
        // restored history, so the pilot sees both the past flight
        // events AND a clear marker for "this is a fresh boot".
        let banner = ActivityEntry {
            timestamp: Utc::now(),
            level: ActivityLevel::Info,
            message: format!("AeroACARS v{} gestartet", env!("CARGO_PKG_VERSION")),
            // Tiny credit line in the boot banner. The pilot sees it
            // every time they open the app — keeps the tool feeling
            // human-made rather than "generated by AI".
            detail: Some("Made with ❤️ in Gifhorn — by Thomas Kant".into()),
        };
        tracing::info!(message = %banner.message, "activity");
        log.push_back(banner);
        while log.len() > ACTIVITY_LOG_CAPACITY {
            log.pop_front();
        }
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_notification::init())
        .manage(app_state)
        .on_window_event(|window, event| {
            // CloseRequested fires when the user clicks the red X
            // (Mac) or the title-bar X (Win). When the
            // minimize-to-tray toggle is on, we suppress the close
            // and just hide the window — the pilot keeps using it
            // via the tray icon. When the toggle is off, the close
            // proceeds normally and the app exits.
            //
            // Critically: we ONLY override the main "AeroACARS"
            // window, not any future child windows. Child windows
            // close as expected.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() != "main" {
                    return;
                }
                let state = window.app_handle().state::<AppState>();
                let minimize = state
                    .minimize_to_tray_enabled
                    .load(std::sync::atomic::Ordering::Relaxed);
                if minimize {
                    api.prevent_close();
                    if let Err(e) = window.hide() {
                        tracing::warn!(error = %e, "failed to hide main window");
                    } else {
                        tracing::info!("main window hidden to tray");
                    }
                }
            }
        })
        .setup(|app| {
            // v0.5.15: initialize file-based secrets storage BEFORE
            // anything else touches the secrets API. We resolve the
            // app data dir from the Tauri PathResolver. If this fails
            // (extremely unlikely — only if the OS denies the user's
            // own appdata dir), secret reads/writes will return
            // SecretError::NotInitialized and the app degrades to
            // "no persistent login" but keeps running.
            match app.path().app_data_dir() {
                Ok(dir) => {
                    if let Err(e) = secrets::init(&dir) {
                        tracing::error!(error = %e, "secrets init failed");
                    } else {
                        // One-shot migration of any pre-v0.5.15
                        // credentials from the OS keyring (Apple
                        // Keychain / Windows Credential Manager) to
                        // our JSON file. Pilots see one final batch
                        // of Keychain prompts on the upgrade run;
                        // every subsequent launch is silent.
                        let accounts = [
                            "primary",          // phpVMS API key
                            "mqtt-username",
                            "mqtt-password",
                            "mqtt-va",
                            "mqtt-pilot-id",
                            "mqtt-broker-url",
                        ];
                        let n = secrets::migrate_from_keyring(&accounts);
                        if n > 0 {
                            tracing::info!(
                                migrated = n,
                                "v0.5.15 keyring → file migration finished"
                            );
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, "could not resolve app_data_dir for secrets");
                }
            }
            // Resolve the activity-log persistence path now that we
            // have an AppHandle. After this, save_activity_log() can
            // run without a handle (uses the OnceLock cache).
            init_activity_log_path(&app.handle());
            // Persist the boot-time state (restored log + banner)
            // immediately so a crash before any activity event still
            // keeps the banner visible on next launch.
            let state = app.state::<AppState>();
            let log = state.activity_log.lock().expect("activity_log lock");
            save_activity_log(&log);
            drop(log);
            // Spawn the auto-start watcher exactly once, here, for the
            // lifetime of the app. The watcher itself short-circuits when
            // the toggle flag is off — flipping the flag at runtime is
            // enough, no need to spawn-on-toggle. Earlier versions did
            // spawn-on-toggle; that had a Mac-first-launch failure mode
            // where the very first toggle's IPC call lost the spawn race
            // and the user had to restart the app to recover.
            // v0.3.0: Persistierten Auto-Start-Stand laden bevor der
            // Watcher startet. Ohne diesen Read würde der AppState
            // mit Default-`false` initialisiert und der Frontend-Hook
            // `auto_start_get_enabled` würde immer `false` zurückgeben,
            // egal ob der Pilot ihn vorher aktiviert hatte.
            {
                let persisted = read_auto_start_persisted(&app.handle());
                let state = app.state::<AppState>();
                state
                    .auto_start_enabled
                    .store(persisted, Ordering::Relaxed);
                if persisted {
                    tracing::info!("auto-start restored from persisted settings");
                }
            }
            spawn_auto_start_watcher(app.handle().clone());
            // Build the system-tray icon + menu. On Windows this lands
            // in the system tray (bottom-right); on Mac in the menubar
            // (top-right). The icon click toggles window visibility,
            // and the right-click context menu has Show / Quit. See
            // `minimize_to_tray_enabled` for the behaviour gate.
            if let Err(e) = build_tray_icon(&app.handle()) {
                tracing::warn!(error = %e, "tray icon setup failed — minimize-to-tray will not work");
            } else {
                // Periodic updater for the tray's tooltip / status
                // header / colored badge. Reads active flight stats
                // every TRAY_UPDATE_INTERVAL_SECS, fires a system
                // notification on critical-state transitions.
                spawn_tray_updater(app.handle().clone());
            }
            // v0.5.11: try to start MQTT live-tracking publisher in
            // the background. Non-fatal — if no API key is present
            // yet (fresh install, user hasn't logged in) it just
            // returns and we'll retry after login.
            {
                let app_for_mqtt = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    init_mqtt_publisher_via_provisioning(app_for_mqtt).await;
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            phpvms_login,
            phpvms_logout,
            phpvms_load_session,
            phpvms_get_bids,
            fetch_simbrief_preview,
            flight_refresh_simbrief,
            flight_resume_after_disconnect,
            phpvms_get_aircraft,
            auto_start_skip_status,
            phpvms_refresh_profile,
            divert_nearest_airports,
            fetch_release_notes,
            get_minimize_to_tray,
            set_minimize_to_tray,
            sim_get_kind,
            sim_set_kind,
            sim_status,
            sim_force_resync,
            pmdg_status,
            airport_get,
            flight_status,
            flight_start,
            flight_end,
            flight_end_manual,
            flight_cancel,
            activity_log_get,
            activity_log_clear,
            landing_list,
            landing_get,
            landing_get_current,
            landing_delete,
            metar_get,
            flight_forget,
            flight_discover_resumable,
            flight_adopt,
            flight_resume_confirm,
            inspector_add,
            inspector_remove,
            inspector_list,
            xplane_inspector_list,
            xplane_premium_status,
            xplane_detect_install_path,
            xplane_install_plugin,
            xplane_uninstall_plugin,
            detect_running_sim,
            auto_start_set_enabled,
            auto_start_get_enabled,
            flight_logs_stats,
            flight_logs_delete_all,
            flight_logs_purge_older_than,
        ])
        .build(tauri::generate_context!())
        .expect("error while building AeroACARS")
        .run(|app_handle, event| {
            // v0.5.11: clean MQTT publisher shutdown on app exit.
            // ExitRequested fires once before the process tears down;
            // we send the LWT-replacement OFFLINE status, give the
            // network thread a brief moment to flush, then return so
            // Tauri continues the exit. If we don't do this, the
            // last-will-and-testament eventually publishes OFFLINE
            // anyway when the broker times the connection out (~60 s),
            // but the explicit shutdown is faster and cleaner.
            if matches!(event, tauri::RunEvent::ExitRequested { .. }) {
                let app_for_mqtt = app_handle.clone();
                tauri::async_runtime::block_on(async move {
                    let state = app_for_mqtt.state::<AppState>();
                    let handle = state.mqtt.lock().await.take();
                    if let Some(handle) = handle {
                        handle.shutdown();
                        // Brief flush window so the publisher task can
                        // post the OFFLINE status before tokio shuts
                        // down. 200 ms is the spec recommendation.
                        tokio::time::sleep(std::time::Duration::from_millis(200))
                            .await;
                    }
                });
            }
        });
}

// ----------------------------------------------------------------------
// Unit tests for Touch-and-Go + Go-Around helpers (v0.1.26).
//
// These cover the two pure-ish helpers that drive the FSM-side
// detectors. Tests construct minimal SimSnapshot values via Default
// (provided by sim-core) and walk the helpers through synthetic
// telemetry sequences. They DON'T exercise the FSM end-to-end —
// `step_flight` has many side-channels that are hard to fake without
// a full `ActiveFlight` setup. The integration coverage comes from
// real in-sim flights against a dev-server PIREP.
// ----------------------------------------------------------------------
#[cfg(test)]
mod touch_and_go_go_around_tests {
    use super::*;
    use chrono::TimeZone;
    use sim_core::SimSnapshot;

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 3, 12, 0, 0).unwrap()
    }

    fn snap_at(agl_ft: f64, vs_fpm: f32, on_ground: bool) -> SimSnapshot {
        let mut s = SimSnapshot::default();
        s.altitude_agl_ft = agl_ft;
        s.vertical_speed_fpm = vs_fpm;
        s.on_ground = on_ground;
        s.engines_running = 2;
        s
    }

    // ---- update_lowest_approach_agl ----

    #[test]
    fn lowest_agl_starts_unset_and_takes_first_sample() {
        let mut stats = FlightStats::default();
        assert_eq!(stats.lowest_agl_during_approach_ft, None);
        update_lowest_approach_agl(&mut stats, &snap_at(2500.0, -800.0, false));
        assert_eq!(stats.lowest_agl_during_approach_ft, Some(2500.0));
    }

    #[test]
    fn lowest_agl_only_decreases_never_increases() {
        let mut stats = FlightStats::default();
        update_lowest_approach_agl(&mut stats, &snap_at(2500.0, -800.0, false));
        update_lowest_approach_agl(&mut stats, &snap_at(1200.0, -700.0, false));
        update_lowest_approach_agl(&mut stats, &snap_at(800.0, -600.0, false));
        // A climb-correction during approach must NOT erase the minimum.
        update_lowest_approach_agl(&mut stats, &snap_at(900.0, 200.0, false));
        assert_eq!(stats.lowest_agl_during_approach_ft, Some(800.0));
    }

    #[test]
    fn lowest_agl_ignores_negative_glitch_samples() {
        let mut stats = FlightStats::default();
        update_lowest_approach_agl(&mut stats, &snap_at(1200.0, -700.0, false));
        // Brief sim/terrain-mesh glitch reporting -50 ft AGL — must not
        // poison the minimum, otherwise every later sample looks like a
        // 200 ft go-around climb.
        update_lowest_approach_agl(&mut stats, &snap_at(-50.0, -700.0, false));
        assert_eq!(stats.lowest_agl_during_approach_ft, Some(1200.0));
    }

    // ---- check_go_around ----

    #[test]
    fn go_around_does_nothing_without_a_prior_minimum() {
        let mut stats = FlightStats::default();
        // No update_lowest_approach_agl called → no minimum tracked.
        let out = check_go_around(&mut stats, &snap_at(2000.0, 1500.0, false), t0());
        assert!(out.is_none());
        assert_eq!(stats.go_around_count, 0);
    }

    #[test]
    fn go_around_fires_after_dwell_seconds() {
        let mut stats = FlightStats::default();
        // Establish approach minimum at 400 ft AGL.
        update_lowest_approach_agl(&mut stats, &snap_at(400.0, -600.0, false));
        let now = t0();
        // Climb-back: 700 ft AGL with V/S +1200 fpm — meets all
        // conditions (700 > 400+200, 1200 > 500, airborne, engines on).
        let climb = snap_at(700.0, 1200.0, false);
        // First tick: arms the dwell, doesn't classify yet.
        let out = check_go_around(&mut stats, &climb, now);
        assert!(out.is_none());
        assert!(stats.go_around_climb_pending_since.is_some());
        // After GO_AROUND_DWELL_SECS the classification fires.
        let out2 = check_go_around(
            &mut stats,
            &climb,
            now + chrono::Duration::seconds(GO_AROUND_DWELL_SECS + 1),
        );
        assert!(matches!(out2, Some(FlightPhase::Climb)));
        assert_eq!(stats.go_around_count, 1);
        // Lowest tracker reset so the next descent starts a fresh window.
        assert_eq!(stats.lowest_agl_during_approach_ft, None);
        assert_eq!(stats.go_around_climb_pending_since, None);
        // ACARS log line was queued for the streamer.
        assert_eq!(stats.pending_acars_logs.len(), 1);
        assert!(stats.pending_acars_logs[0].contains("Go-around"));
    }

    #[test]
    fn go_around_does_not_fire_below_recovery_threshold() {
        let mut stats = FlightStats::default();
        update_lowest_approach_agl(&mut stats, &snap_at(400.0, -600.0, false));
        let now = t0();
        // Only +150 ft above lowest — below GO_AROUND_AGL_RECOVERY_FT.
        // Even with V/S +1500 we should NOT classify.
        let weak = snap_at(550.0, 1500.0, false);
        check_go_around(&mut stats, &weak, now);
        let out = check_go_around(
            &mut stats,
            &weak,
            now + chrono::Duration::seconds(GO_AROUND_DWELL_SECS + 1),
        );
        assert!(out.is_none());
        assert_eq!(stats.go_around_count, 0);
    }

    #[test]
    fn go_around_dwell_resets_when_conditions_break() {
        let mut stats = FlightStats::default();
        update_lowest_approach_agl(&mut stats, &snap_at(400.0, -600.0, false));
        let now = t0();
        // Brief climb arms the dwell.
        check_go_around(&mut stats, &snap_at(700.0, 1200.0, false), now);
        assert!(stats.go_around_climb_pending_since.is_some());
        // V/S drops back below threshold mid-dwell — reset expected.
        check_go_around(
            &mut stats,
            &snap_at(700.0, 200.0, false),
            now + chrono::Duration::seconds(3),
        );
        assert!(stats.go_around_climb_pending_since.is_none());
        assert_eq!(stats.go_around_count, 0);
    }

    #[test]
    fn go_around_does_not_fire_when_aircraft_caught_glideslope_high() {
        let mut stats = FlightStats::default();
        // Pilot intercepted the GS from above — minimum is still very
        // high (3000 ft AGL during approach intercept). A brief +800
        // fpm during the level-off MUST NOT trigger GA.
        update_lowest_approach_agl(&mut stats, &snap_at(3000.0, -800.0, false));
        let now = t0();
        let blip = snap_at(3300.0, 800.0, false);
        check_go_around(&mut stats, &blip, now);
        let out = check_go_around(
            &mut stats,
            &blip,
            now + chrono::Duration::seconds(GO_AROUND_DWELL_SECS + 1),
        );
        // lowest > 1500 ft → detector is gated off entirely.
        assert!(out.is_none());
        assert_eq!(stats.go_around_count, 0);
    }

    #[test]
    fn go_around_requires_engines_running() {
        let mut stats = FlightStats::default();
        update_lowest_approach_agl(&mut stats, &snap_at(400.0, -600.0, false));
        let now = t0();
        let mut climb = snap_at(700.0, 1200.0, false);
        climb.engines_running = 0; // engines off → not really going around
        check_go_around(&mut stats, &climb, now);
        let out = check_go_around(
            &mut stats,
            &climb,
            now + chrono::Duration::seconds(GO_AROUND_DWELL_SECS + 1),
        );
        assert!(out.is_none());
        assert_eq!(stats.go_around_count, 0);
    }
}

// ---- v0.5.11: AGL-derivative touchdown VS estimator regression tests ----
//
// Per pilot's deep analysis 2026-05-07. These tests guard against
// the entire bug class identified there.
#[cfg(test)]
mod touchdown_vs_estimator_tests {
    use super::*;
    use chrono::TimeZone;
    use std::collections::VecDeque;

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 7, 12, 0, 0).unwrap()
    }

    fn make_sample(at: DateTime<Utc>, agl_ft: f32, vs_fpm: f32) -> TelemetrySample {
        TelemetrySample {
            at,
            vs_fpm,
            g_force: 1.0,
            on_ground: false,
            agl_ft,
            heading_true_deg: 0.0,
            groundspeed_kt: 130.0,
            indicated_airspeed_kt: 130.0,
            lat: 0.0,
            lon: 0.0,
            pitch_deg: 0.0,
            bank_deg: 0.0,
        }
    }

    /// Test 1: Rebound-VSI is positive but AGL is geometrically
    /// dropping → result must be NEGATIVE.
    /// Reproduces the original pilot bug (V/S +57 fpm at G 1.52).
    #[test]
    fn agl_estimator_overrides_positive_rebound_vsi() {
        let td = t0();
        let mut buffer = VecDeque::new();
        // Last 1 second of approach: AGL drops 25 → 0 ft.
        // VS readings are positive (rebound) but geometry says
        // descent at -25 ft/sec = -1500 fpm avg.
        for ms in 0..=1000 {
            if ms % 100 != 0 {
                continue;
            }
            let agl = 25.0 - (ms as f32 / 1000.0) * 25.0;
            let vs = 50.0; // positive rebound — VSI lies
            buffer.push_back(make_sample(
                td - chrono::Duration::milliseconds(1000 - ms),
                agl.max(0.0),
                vs,
            ));
        }
        let est = estimate_xplane_touchdown_vs_from_agl(&buffer, td)
            .expect("should estimate from AGL despite positive VSI");
        assert!(
            est.fpm < 0.0,
            "expected negative descent rate, got {} fpm",
            est.fpm
        );
        assert!(
            est.fpm < -500.0,
            "expected steep descent (~-1500 fpm), got {} fpm",
            est.fpm
        );
    }

    /// Test 2: Pre-flare steep descent (-1300 fpm @ 900 ft AGL)
    /// followed by gentle touchdown (-200 fpm @ 0 ft).
    /// Estimator must IGNORE the high-altitude steep descent.
    #[test]
    fn agl_estimator_ignores_pre_flare_high_altitude_descent() {
        let td = t0();
        let mut buffer = VecDeque::new();
        // 9 seconds ago: AGL 900 ft, VS -1300 fpm (steep descent)
        buffer.push_back(make_sample(
            td - chrono::Duration::seconds(9),
            900.0,
            -1300.0,
        ));
        // Last 1 second: smooth flare from 30 → 0 ft AGL = -1800 fpm
        // Wait, let me make this gentler: 3 → 0 ft over 1s = -180 fpm
        for ms in 0..=1000 {
            if ms % 100 != 0 {
                continue;
            }
            let agl = 3.0 - (ms as f32 / 1000.0) * 3.0;
            buffer.push_back(make_sample(
                td - chrono::Duration::milliseconds(1000 - ms),
                agl.max(0.0),
                -200.0,
            ));
        }
        let est = estimate_xplane_touchdown_vs_from_agl(&buffer, td)
            .expect("should pick the close-to-ground tier");
        // Should be gentle (~-180 fpm), NOT the -1300 from 900 ft.
        assert!(
            est.fpm > -500.0,
            "expected gentle descent (NOT pre-flare -1300 contamination), got {} fpm",
            est.fpm
        );
    }

    /// Test 3: Butter landing — slow flare, gentle touchdown.
    /// Estimator should give ~-100 fpm, not the original approach descent.
    #[test]
    fn agl_estimator_butter_landing() {
        let td = t0();
        let mut buffer = VecDeque::new();
        // Last 2 seconds: AGL 4 → 1 ft (= -90 fpm avg, butter)
        for ms in 0..=2000 {
            if ms % 200 != 0 {
                continue;
            }
            let agl = 4.0 - (ms as f32 / 2000.0) * 3.0;
            buffer.push_back(make_sample(
                td - chrono::Duration::milliseconds(2000 - ms),
                agl,
                -100.0,
            ));
        }
        let est = estimate_xplane_touchdown_vs_from_agl(&buffer, td)
            .expect("should give a butter result");
        assert!(est.fpm < 0.0, "expected negative descent, got {}", est.fpm);
        assert!(
            est.fpm > -300.0,
            "expected gentle butter (~-100 to -200 fpm), got {} fpm",
            est.fpm
        );
    }

    /// Test 4: All VS readings are positive (extreme rebound) but
    /// AGL drops geometrically. AGL wins, result is negative.
    #[test]
    fn agl_estimator_wins_when_all_vs_positive() {
        let td = t0();
        let mut buffer = VecDeque::new();
        // Last 1 second: AGL 10 → 0 ft, VS readings all positive (impossible
        // physically but happens in sim post-touchdown rebound)
        for ms in 0..=1000 {
            if ms % 100 != 0 {
                continue;
            }
            let agl = 10.0 - (ms as f32 / 1000.0) * 10.0;
            buffer.push_back(make_sample(
                td - chrono::Duration::milliseconds(1000 - ms),
                agl.max(0.0),
                100.0, // all positive!
            ));
        }
        let est = estimate_xplane_touchdown_vs_from_agl(&buffer, td)
            .expect("AGL should give answer despite all-positive VS");
        assert!(
            est.fpm < 0.0,
            "expected negative descent from AGL geometry, got {} fpm (VS readings would have given positive)",
            est.fpm
        );
    }

    /// negative_only filter test: positives are dropped.
    #[test]
    fn negative_only_drops_positive_values() {
        assert_eq!(negative_only(Some(57.0)), None);
        assert_eq!(negative_only(Some(0.0)), None);
        assert_eq!(negative_only(Some(-200.0)), Some(-200.0));
        assert_eq!(negative_only(None), None);
        assert_eq!(negative_only(Some(f32::NAN)), None);
    }
}

#[cfg(test)]
mod aircraft_alias_tests {
    use super::aircraft_types_match;

    /// Live bug 2026-05-04: Emirates UAE770 EK770 (A359 bid)
    /// blocked because the sim loaded "A350-900 (No Cabin)" — both
    /// names refer to the same airframe.
    #[test]
    fn a359_matches_a350_900_long_form() {
        assert!(aircraft_types_match("A359", "A350-900"));
        assert!(aircraft_types_match("A350-900", "A359"));
    }

    #[test]
    fn b738_matches_737_800() {
        assert!(aircraft_types_match("B738", "737-800"));
        assert!(aircraft_types_match("737-800", "B738"));
    }

    #[test]
    fn b77w_matches_777_300er() {
        assert!(aircraft_types_match("B77W", "777-300ER"));
        assert!(aircraft_types_match("777-300ER", "B77W"));
        assert!(aircraft_types_match("B77W", "777-300 ER"));
    }

    #[test]
    fn a20n_matches_a320neo() {
        assert!(aircraft_types_match("A20N", "A320NEO"));
        assert!(aircraft_types_match("A20N", "A320-NEO"));
        assert!(aircraft_types_match("A20N", "A320 NEO"));
    }

    #[test]
    fn b789_matches_787_9() {
        assert!(aircraft_types_match("B789", "787-9"));
    }

    #[test]
    fn unrelated_types_dont_match() {
        // Real mismatch should still be blocked.
        assert!(!aircraft_types_match("B738", "A320"));
        assert!(!aircraft_types_match("A359", "B77W"));
    }

    #[test]
    fn case_insensitive() {
        assert!(aircraft_types_match("a359", "a350-900"));
        assert!(aircraft_types_match("A359", "a350-900"));
    }

    #[test]
    fn strict_equality_still_works_for_unaliased() {
        assert!(aircraft_types_match("DH8D", "DH8D"));
        assert!(!aircraft_types_match("DH8D", "DH8C"));
    }
}

