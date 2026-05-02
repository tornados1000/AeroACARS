//! CloudeAcars — Tauri application root.
//!
//! Holds the active `api_client::Client` in shared state, exposes auth commands
//! to the UI (login, logout, session restore), and persists the site URL to a
//! per-user config dir. The API key itself is stored via `secrets` (OS keyring),
//! never on disk in plaintext.

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
use storage::{PositionQueue, QueuedPosition};
use sim_core::{FlightPhase, SimKind, SimSnapshot};
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
fn title_mentions_icao(title: &str, icao: &str) -> bool {
    let title_upper = title.to_uppercase();
    let icao_upper = icao.to_uppercase();
    title_upper.contains(&icao_upper)
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
    /// Registration phpVMS assigned to this flight (e.g. "D-AIUV").
    /// Looked up via `get_aircraft(bid.flight.aircraft_id)` at start
    /// time. Compared against the live `ATC ID` SimVar in the activity
    /// log so the pilot sees immediately if they loaded the wrong tail
    /// number in MSFS. Empty string when unknown (fresh-PIREP / disk-
    /// resume edge cases where we couldn't match a bid).
    planned_registration: String,
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
    #[serde(default)]
    landing_rate_fpm: Option<f32>,
    #[serde(default)]
    landing_g_force: Option<f32>,
    #[serde(default)]
    landing_pitch_deg: Option<f32>,
    #[serde(default)]
    landing_speed_kt: Option<f32>,
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
    peak_altitude_ft: Option<f32>,
    #[serde(default)]
    aircraft_banner_logged: bool,
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
            landing_rate_fpm: stats.landing_rate_fpm,
            landing_g_force: stats.landing_g_force,
            landing_pitch_deg: stats.landing_pitch_deg,
            landing_speed_kt: stats.landing_speed_kt,
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
            peak_altitude_ft: stats.peak_altitude_ft,
            aircraft_banner_logged: stats.aircraft_banner_logged,
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
        stats.landing_rate_fpm = self.landing_rate_fpm;
        stats.landing_g_force = self.landing_g_force;
        stats.landing_pitch_deg = self.landing_pitch_deg;
        stats.landing_speed_kt = self.landing_speed_kt;
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
        stats.peak_altitude_ft = self.peak_altitude_ft;
        stats.aircraft_banner_logged = self.aircraft_banner_logged;
    }
}

/// One entry in the touchdown ring buffer (see `FlightStats::snapshot_buffer`).
#[derive(Debug, Clone, Copy)]
struct TelemetrySample {
    at: DateTime<Utc>,
    vs_fpm: f32,
    g_force: f32,
    on_ground: bool,
}

/// Maximum age of any entry in the touchdown ring buffer. 5 s gives
/// us roughly the last final-approach segment plus the touchdown
/// itself. Longer than ~6 s and we'd start picking up Cruise data;
/// shorter than ~3 s and we'd miss the descent rate moments before
/// flare.
const TOUCHDOWN_BUFFER_SECS: i64 = 5;

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

    // ---- Capture at takeoff ----
    takeoff_weight_kg: Option<f64>,
    takeoff_fuel_kg: Option<f32>,

    // ---- Capture at touchdown ----
    landing_rate_fpm: Option<f32>,
    landing_g_force: Option<f32>,
    landing_pitch_deg: Option<f32>,
    landing_speed_kt: Option<f32>,
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

    /// Highest MSL altitude we've seen while in Cruise (or any step
    /// climb during Cruise). Drives the Cruise → Descent guard so
    /// short ATC step-downs (FL380 → FL360) don't flip the phase to
    /// Descent — only a real TOD drop of >5000 ft from this peak
    /// counts.
    cruise_peak_msl: Option<f32>,
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
    /// FCU selected values — debounced so a knob-spin doesn't fire a
    /// log entry per click.
    last_logged_fcu_alt: Option<i32>,
    last_logged_fcu_hdg: Option<i32>,
    last_logged_fcu_spd: Option<i32>,
    last_logged_fcu_vs: Option<i32>,
    pending_fcu_alt: Option<(i32, DateTime<Utc>)>,
    pending_fcu_hdg: Option<(i32, DateTime<Utc>)>,
    pending_fcu_spd: Option<(i32, DateTime<Utc>)>,
    pending_fcu_vs: Option<(i32, DateTime<Utc>)>,
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
        let by_g = if peak_g >= 2.5 {
            Self::Severe
        } else if peak_g >= TOUCHDOWN_G_HARD {
            Self::Hard
        } else if peak_g >= TOUCHDOWN_G_FIRM {
            Self::Firm
        } else if peak_g >= 1.2 {
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

/// Snapshot of the six exterior lights we track. Compared as a whole so we
/// can emit one log entry per "lights change" rather than six on a config
/// transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct LightsState {
    landing: bool,
    beacon: bool,
    strobe: bool,
    taxi: bool,
    nav: bool,
    logo: bool,
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
    /// Number of positions sitting in the offline queue. Non-zero
    /// means the streamer hit a network failure recently and the
    /// dashboard should warn the pilot.
    queued_position_count: u32,
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
/// virtual airlines (and the activity-log readers) recognise. Boeing
/// pilots will read "Flaps 3" the same way; the FULL label maps to
/// detent 5 on Airbus, but Boeings don't reach 5 anyway.
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

/// Render the full callsign as "DLH 155" (airline + space + number) when
/// the airline ICAO is known, falling back to bare "155" otherwise.
fn format_callsign(airline_icao: &str, flight_number: &str) -> String {
    if airline_icao.is_empty() {
        flight_number.to_string()
    } else {
        format!("{} {}", airline_icao, flight_number)
    }
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
/// into the rollout.
const TOUCHDOWN_WINDOW_SECS: i64 = 5;

/// Hard-landing thresholds, ordered worst-first. The first row that the
/// peak |V/S| or G-force breaches wins; combined with `bounce_count`
/// this maps to a `LandingScore`. Numbers are typical VA-rule defaults
/// — VAs that want their own thresholds can override later via
/// `phpvms_get_settings` (Phase L).
const TOUCHDOWN_VS_SEVERE_FPM: f32 = 1000.0;
const TOUCHDOWN_VS_HARD_FPM: f32 = 600.0;
const TOUCHDOWN_VS_FIRM_FPM: f32 = 240.0;
const TOUCHDOWN_VS_SMOOTH_FPM: f32 = 60.0;
const TOUCHDOWN_G_HARD: f32 = 1.8;
const TOUCHDOWN_G_FIRM: f32 = 1.4;

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
}

#[tauri::command]
fn app_info() -> AppInfo {
    AppInfo {
        name: "CloudeAcars",
        version: env!("CARGO_PKG_VERSION"),
        commit: option_env!("CLOUDEACARS_GIT_SHA"),
    }
}

/// Authenticate against a phpVMS site. On success: stores key in OS keyring,
/// writes URL to site config, and caches the live `Client` in `AppState`.
#[tauri::command]
async fn phpvms_login(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    url: String,
    api_key: String,
) -> Result<LoginResult, UiError> {
    let conn = Connection::new(&url, api_key.trim())?;
    let client = Client::new(conn)?;
    let profile = client.get_profile().await?;

    secrets::store_api_key(KEYRING_ACCOUNT, api_key.trim())
        .map_err(|e| UiError::new("keyring", e.to_string()))?;
    write_site_config(&app, &SiteConfig { url: url.clone() })?;

    let base_url = client.connection().base_url().to_string();
    *state.client.lock().expect("client mutex") = Some(client.clone());

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
    let Some(cfg) = read_site_config(&app)? else {
        return Ok(None);
    };
    let Some(api_key) = secrets::load_api_key(KEYRING_ACCOUNT)
        .map_err(|e| UiError::new("keyring", e.to_string()))?
    else {
        return Ok(None);
    };

    let conn = Connection::new(&cfg.url, &api_key)?;
    let client = Client::new(conn)?;
    match client.get_profile().await {
        Ok(profile) => {
            let base_url = client.connection().base_url().to_string();
            *state.client.lock().expect("client mutex") = Some(client.clone());
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

/// `GET /api/user/bids` — the pilot's open bids.
#[tauri::command]
async fn phpvms_get_bids(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Bid>, UiError> {
    let client = current_client(&state)?;
    Ok(client.get_bids().await?)
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
    // Identifier is set in `tauri.conf.json` as `com.cloudeacars.app`.
    let path = std::path::Path::new(&appdata)
        .join("com.cloudeacars.app")
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
        queued_position_count: stats.queued_position_count,
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

    // Look up planned registration so the activity log can compare it
    // against the live `ATC ID` SimVar — pilot sees instantly if the
    // wrong tail number is loaded in MSFS.
    // Bid.flight has no direct aircraft_id; the chosen aircraft lives
    // on the SimBrief OFP. If the pilot hasn't generated an OFP, we
    // simply leave planned_registration empty.
    let planned_registration = match matching_bid
        .and_then(|b| b.flight.simbrief.as_ref())
        .and_then(|sb| sb.aircraft_id)
    {
        Some(id) => client
            .get_aircraft(id)
            .await
            .ok()
            .and_then(|a| a.registration)
            .unwrap_or_default()
            .trim()
            .to_string(),
        None => String::new(),
    };

    let flight = Arc::new(ActiveFlight {
        pirep_id: pirep.id.clone(),
        bid_id,
        // We don't know the original prefile time; treat "now" as the start
        // for our counters. The PIREP's actual times are intact server-side.
        started_at: Utc::now(),
        airline_icao,
        planned_registration,
        flight_number,
        dpt_airport,
        arr_airport,
        fares,
        stats: Mutex::new(FlightStats::new()),
        stop: AtomicBool::new(false),
        // Surfaced via flight_status to trigger the resume banner.
        was_just_resumed: AtomicBool::new(true),
        streamer_spawned: AtomicBool::new(false),
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
        if expected != actual && !title_supports_expected {
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
        source_name: format!("CloudeAcars/{}", env!("CARGO_PKG_VERSION")),
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
        notes: None,
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
        planned_registration,
        flight_number: bid.flight.flight_number.clone(),
        dpt_airport: bid.flight.dpt_airport_id.clone(),
        arr_airport: bid.flight.arr_airport_id.clone(),
        fares,
        stats: Mutex::new(FlightStats::new()),
        stop: AtomicBool::new(false),
        // Fresh start triggered by user — no banner needed.
        was_just_resumed: AtomicBool::new(false),
        streamer_spawned: AtomicBool::new(false),
    });

    save_active_flight(&app, &flight);

    {
        let mut guard = state.active_flight.lock().expect("active_flight lock");
        *guard = Some(Arc::clone(&flight));
    }
    // ActiveFlight is committed; release the setup-in-progress flag.
    setup_guard.disarm();

    spawn_position_streamer(app.clone(), Arc::clone(&flight), client);
    spawn_touchdown_sampler(app.clone(), Arc::clone(&flight));

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
#[tauri::command]
async fn flight_end(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), UiError> {
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
    let distance_to_arr_nm = compute_distance_to_airport(&app, &state, &arr_icao).await;

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
    let body = {
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
        let fuel_used = match (stats.block_fuel_kg, stats.last_fuel_kg) {
            (Some(b), Some(c)) if b > c => Some((b - c) as f64 * KG_TO_LB),
            _ => None,
        };
        // Block fuel sent natively so phpVMS computes "Verbleibender
        // Treibstoff" correctly (= block_fuel - fuel_used). Without
        // this the dashboard shows "-fuel_used kg" because the
        // missing block_fuel defaults to 0. Same unit as fuel_used.
        let block_fuel = stats
            .block_fuel_kg
            .filter(|kg| *kg > 0.0)
            .map(|kg| (kg as f64) * KG_TO_LB);
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
        let notes = build_pirep_notes(&flight, &stats);

        FileBody {
            flight_time,
            fuel_used,
            block_fuel,
            distance: Some(distance_nm),
            level,
            landing_rate,
            score,
            source_name: Some(format!("CloudeAcars/{}", env!("CARGO_PKG_VERSION"))),
            notes: Some(notes),
            fares,
            fields: Some(fields),
        }
    };
    tracing::info!(
        pirep_id = %flight.pirep_id,
        flight_time = body.flight_time.unwrap_or(0),
        distance = body.distance.unwrap_or(0.0),
        fuel_used = body.fuel_used.unwrap_or(0.0),
        fare_classes = flight.fares.len(),
        custom_fields = body.fields.as_ref().map(|f| f.len()).unwrap_or(0),
        "filing PIREP"
    );
    match client.file_pirep(&flight.pirep_id, &body).await {
        Ok(()) => {
            clear_persisted_flight(&app);
            log_activity(
                &state,
                ActivityLevel::Info,
                format!(
                    "PIREP filed: {} {} → {}",
                    format_callsign(&flight.airline_icao, &flight.flight_number),
                    flight.dpt_airport,
                    flight.arr_airport
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
///
/// All optional fields end up in the PIREP notes with clear `[MANUAL]` /
/// `[DIVERT]` tags so the admin reviewing the flight can act on them.
#[tauri::command]
async fn flight_end_manual(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    notes_override: Option<String>,
    divert_to: Option<String>,
    reason: Option<String>,
) -> Result<(), UiError> {
    let flight = {
        let mut guard = state.active_flight.lock().expect("active_flight lock");
        guard
            .take()
            .ok_or_else(|| UiError::new("no_active_flight", "no flight is active"))?
    };
    flight.stop.store(true, Ordering::Relaxed);
    let client = current_client(&state)?;

    let body = {
        let stats = flight.stats.lock().expect("flight stats");
        // Same flight-time / block-fuel / level / landing-rate / score
        // mapping as the regular file path in `flight_end`. Manual
        // filing intentionally still ships these even if some are
        // None — phpVMS skips missing values cleanly thanks to
        // `skip_serializing_if`.
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
        let fuel_used = match (stats.block_fuel_kg, stats.last_fuel_kg) {
            (Some(b), Some(c)) if b > c => Some((b - c) as f64 * KG_TO_LB),
            _ => None,
        };
        let block_fuel = stats
            .block_fuel_kg
            .filter(|kg| *kg > 0.0)
            .map(|kg| (kg as f64) * KG_TO_LB);
        let level = stats.peak_altitude_ft.map(|ft| {
            let rounded = ((ft / 100.0).round() * 100.0) as i32;
            rounded.max(0)
        });
        let landing_rate = stats.landing_rate_fpm.map(|v| v as f64);
        let score = stats.landing_score.map(|s| s.numeric());
        let distance_nm = stats.distance_nm;
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
            distance: Some(distance_nm),
            level,
            landing_rate,
            score,
            source_name: Some(format!(
                "CloudeAcars/{} (manual)",
                env!("CARGO_PKG_VERSION")
            )),
            notes: Some(notes),
            fares,
            fields: Some(fields),
        }
    };
    tracing::info!(pirep_id = %flight.pirep_id, "filing PIREP (manual)");
    match client.file_pirep(&flight.pirep_id, &body).await {
        Ok(()) => {
            clear_persisted_flight(&app);
            log_activity(
                &state,
                ActivityLevel::Warn,
                format!(
                    "Manual PIREP filed: {} {} → {}",
                    format_callsign(&flight.airline_icao, &flight.flight_number),
                    flight.dpt_airport,
                    flight.arr_airport
                ),
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
fn spawn_touchdown_sampler(app: AppHandle, flight: Arc<ActiveFlight>) {
    tauri::async_runtime::spawn(async move {
        tracing::info!(pirep_id = %flight.pirep_id, "touchdown sampler started");
        loop {
            // 33 ms ≈ 30 Hz target. The actual upper bound comes
            // from how fast `current_snapshot` returns fresh data.
            tokio::time::sleep(Duration::from_millis(33)).await;
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
            });
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
        // Phase-adaptive cadence: re-read the current phase on every tick so
        // the sleep matches what the aircraft is actually doing right now
        // (5 s during takeoff/landing, 8 s on approach/final, 10 s on the
        // ground / climb / descent, 30 s in cruise — capped at 30 s so the
        // live map never goes more than half a minute stale).
        loop {
            let current_phase = {
                let stats = flight.stats.lock().expect("flight stats");
                stats.phase
            };
            tokio::time::sleep(position_interval(current_phase)).await;
            if flight.stop.load(Ordering::Relaxed) {
                break;
            }

            let snapshot = current_snapshot(&app);
            let Some(snap) = snapshot else {
                tracing::warn!(
                    pirep_id = %flight.pirep_id,
                    "no sim snapshot yet — skipping position post"
                );
                continue;
            };

            // Snapshot the current phase BEFORE stepping so we can pass
            // the from→to pair to the recorder when it changes.
            let prev_phase = {
                let stats = flight.stats.lock().expect("flight stats");
                stats.phase
            };
            // Update running stats AND step the flight-phase FSM.
            let phase_change = step_flight(&flight, &snap);
            // Detect when the touchdown-analyzer window has just locked
            // in a final score so we can emit the activity-log entry
            // exactly once. `landing_score` flips from None to Some
            // inside `step_flight` after TOUCHDOWN_WINDOW_SECS.
            announce_landing_score(&app, &flight);
            // Diff cockpit knobs against last-seen values and log changes
            // to the activity feed. One entry per change, not per tick.
            detect_telemetry_changes(&app, &flight, &snap);
            let position = snapshot_to_position(&snap);

            // Try to drain any positions we couldn't ship in earlier
            // ticks before sending the new one — keeps phpVMS's row
            // ordering chronological even after a network gap.
            let queue = open_position_queue(&app);
            if let Some(q) = &queue {
                drain_position_queue(q, &client, &flight.pirep_id).await;
            }

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
                if let Some(status) = phase_to_status(new_phase) {
                    tracing::info!(
                        pirep_id = %flight.pirep_id,
                        ?new_phase,
                        status,
                        "flight phase transition"
                    );
                    let body = UpdateBody {
                        state: None,
                        status: Some(status.to_string()),
                        notes: None,
                    };
                    if let Err(e) = client.update_pirep(&flight.pirep_id, &body).await {
                        tracing::warn!(
                            pirep_id = %flight.pirep_id,
                            ?new_phase,
                            error = %e,
                            "could not push phase status update"
                        );
                    }
                }
            }
        }
        tracing::info!(pirep_id = %flight.pirep_id, "position streamer stopped");
    });
}

#[cfg(target_os = "windows")]
fn current_snapshot(app: &AppHandle) -> Option<SimSnapshot> {
    let state = app.state::<AppState>();
    let adapter = state.msfs.lock().expect("msfs lock");
    adapter.snapshot()
}

#[cfg(not(target_os = "windows"))]
fn current_snapshot(_app: &AppHandle) -> Option<SimSnapshot> {
    None
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
    stats.last_fuel_kg = Some(snap.fuel_total_kg);

    // Block fuel is captured at the Pushback / TaxiOut transition
    // (block-off moment) below in `step_flight`. Capturing it on
    // the very first snapshot was unreliable: Fenix doesn't load
    // EFB-managed fuel until ~5 s after flight_start, so the first
    // tick saw a stale default (typically 3000 kg / 6600 lb) and
    // we ended up reporting Used Fuel = 0 because the diff against
    // the higher real value at landing went negative. The field
    // stays None until block-off — that's both technically and
    // operationally correct.

    // Peak altitude tracker — every tick, take the max with the live
    // MSL altitude. Reported as the PIREP `level` field.
    let alt = snap.altitude_msl_ft as f32;
    stats.peak_altitude_ft = Some(stats.peak_altitude_ft.map_or(alt, |p| p.max(alt)));

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
            if snap.on_ground && snap.groundspeed_kt > 0.5 {
                stats.block_off_at = Some(now);
                // Block fuel = fuel on board at the block-off moment.
                // Capturing here (instead of on the very first
                // snapshot) survives the typical Fenix-EFB load
                // sequence: pilot opens EFB during Boarding, sets
                // fuel via SimBrief import, the LVar settles a few
                // seconds later. By the time the aircraft actually
                // moves we're guaranteed to read the final value —
                // matches what airline ops calls "block fuel".
                stats.block_fuel_kg = Some(snap.fuel_total_kg);
                next_phase = if snap.engines_running == 0 {
                    FlightPhase::Pushback
                } else {
                    FlightPhase::TaxiOut
                };
            }
        }
        FlightPhase::Pushback => {
            // Pushback → TaxiOut handoff. Three signals, in priority
            // order, all checked together:
            //
            // 1. `PUSHBACK STATE == 3` from MSFS — the sim itself
            //    reporting "no pushback" (tug disconnected, or the
            //    pilot used Ctrl+P to stop). This is the
            //    authoritative signal regardless of whether the
            //    push was straight, left, right, or even a pull-
            //    forward maneuver.
            // 2. Engines running AND ground speed >3 kt — fallback
            //    for situations where PUSHBACK STATE never reports
            //    (e.g. pilot pushed by hand without using the tug
            //    feature). Same trigger as before.
            //
            // The PUSHBACK STATE field on the snapshot is Option<u8>;
            // a value of Some(3) means "tug done", Some(0..=2) means
            // "still pushing", None means "MSFS hasn't told us".
            let tug_done = snap.pushback_state == Some(3);
            let powered_taxi = snap.on_ground
                && snap.engines_running > 0
                && snap.groundspeed_kt > 3.0;
            if tug_done && powered_taxi {
                next_phase = FlightPhase::TaxiOut;
            } else if snap.pushback_state.is_none() && powered_taxi {
                // No pushback-state info from the sim — fall back
                // to the legacy "moving + engines on" trigger so
                // we don't get stuck in Pushback indefinitely on
                // sims / aircraft that don't expose the field.
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
        }
        FlightPhase::TakeoffRoll => {
            if was_on_ground && !snap.on_ground {
                next_phase = FlightPhase::Takeoff;
                stats.takeoff_at = Some(now);
                stats.takeoff_fuel_kg = Some(snap.fuel_total_kg);
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
            }
        }
        FlightPhase::Climb => {
            // Descent threshold raised to −500 fpm (was −300). Real
            // top-of-descent rates are −1500..−2500 fpm; −300 was
            // tripping on level-off corrections, autopilot trims and
            // light turbulence and pushing the dashboard into
            // Descent prematurely.
            if snap.vertical_speed_fpm < -500.0 {
                next_phase = FlightPhase::Descent;
            } else if snap.vertical_speed_fpm.abs() < 200.0
                && snap.altitude_agl_ft > 5000.0
            {
                next_phase = FlightPhase::Cruise;
            }
        }
        FlightPhase::Cruise => {
            // Track the highest altitude we've seen at this cruise
            // — used as the reference point for "did we really
            // descend, or is this just an ATC step-down?".
            let peak = stats.cruise_peak_msl.unwrap_or(0.0);
            if (snap.altitude_msl_ft as f32) > peak {
                stats.cruise_peak_msl = Some(snap.altitude_msl_ft as f32);
            }

            // Cruise → Descent only when BOTH conditions hold:
            //   * Sustained sink rate (< −500 fpm — filters
            //     turbulence and autopilot trim noise)
            //   * Lost > 5000 ft from the cruise peak (a real TOD
            //     drops you many thousand feet; an ATC step-down
            //     FL380 → FL360 is only 2000 ft and stays in Cruise)
            //
            // Step-climbs and short step-downs are *not* phase
            // changes — they're routine cruise activity. Flipping
            // back to Climb / forward to Descent for every new
            // assigned level would ping-pong the timeline.
            let lost_alt =
                stats.cruise_peak_msl.unwrap_or(0.0) - snap.altitude_msl_ft as f32;
            if snap.vertical_speed_fpm < -500.0 && lost_alt > 5000.0 {
                next_phase = FlightPhase::Descent;
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
            }
        }
        FlightPhase::Approach => {
            // 1500 ft AGL was too eager — pilots reported a 3 min
            // "Final" segment because most aircraft intercept the
            // ILS at that altitude and still have several miles to
            // run. Real-world Final starts ~700 ft AGL (FAF crossed
            // for non-precision, decision height area for ILS).
            if snap.altitude_agl_ft < 700.0 {
                next_phase = FlightPhase::Final;
            }
        }
        FlightPhase::Final => {
            // Approach runway: snapshot whatever ATC currently has us
            // cleared for. May still change before touchdown, so we
            // refresh until wheels are down.
            if let Some(rw) = snap.selected_runway.as_ref().filter(|s| !s.is_empty()) {
                stats.approach_runway = Some(rw.clone());
            }
            if !was_on_ground && snap.on_ground {
                next_phase = FlightPhase::Landing;
                stats.landing_at = Some(now);

                // V/S capture priority:
                //   1. SimConnect's PLANE TOUCHDOWN NORMAL VELOCITY,
                //      latched by the sim itself at the actual frame
                //      of contact. This is the cleanest signal *if*
                //      Fenix / FBW / etc. update it (some don't).
                //   2. Most-negative VS in the ring buffer's airborne
                //      samples — handles the common case where the
                //      single on-ground tick already shows a positive
                //      bounce-up VS while the buffer still remembers
                //      the actual descent rate.
                //   3. The current snapshot's live VS (last resort).
                //
                // We ALWAYS take the worst of (1) and (2) to be safe
                // against either source returning a partial value.
                let buffered_vs_min: f32 = stats
                    .snapshot_buffer
                    .iter()
                    .filter(|s| !s.on_ground)
                    .map(|s| s.vs_fpm)
                    .fold(f32::INFINITY, f32::min);
                let candidates = [
                    snap.touchdown_vs_fpm.unwrap_or(f32::INFINITY),
                    if buffered_vs_min.is_finite() {
                        buffered_vs_min
                    } else {
                        f32::INFINITY
                    },
                    snap.vertical_speed_fpm,
                ];
                let touchdown_vs = candidates
                    .iter()
                    .copied()
                    .filter(|v| v.is_finite())
                    .fold(f32::INFINITY, f32::min);
                let touchdown_vs = if touchdown_vs.is_finite() {
                    touchdown_vs
                } else {
                    snap.vertical_speed_fpm
                };
                stats.landing_rate_fpm = Some(touchdown_vs);
                stats.landing_peak_vs_fpm = Some(touchdown_vs);

                // G capture priority:
                //   1. Highest G in the ring buffer (catches the impact
                //      spike even when the on-ground tick already shows
                //      G≈1.0 because the bounce is already in the air).
                //   2. Current snapshot G (in case the spike happens
                //      exactly on this tick).
                let buffered_g_peak: f32 = stats
                    .snapshot_buffer
                    .iter()
                    .map(|s| s.g_force)
                    .fold(0.0, f32::max);
                let touchdown_g = buffered_g_peak.max(snap.g_force);
                stats.landing_g_force = Some(touchdown_g);
                stats.landing_peak_g_force = Some(touchdown_g);

                stats.landing_pitch_deg =
                    Some(snap.touchdown_pitch_deg.unwrap_or(snap.pitch_deg));
                stats.landing_heading_deg =
                    Some(snap.touchdown_heading_mag_deg.unwrap_or(snap.heading_deg_magnetic));
                stats.landing_speed_kt = Some(snap.indicated_airspeed_kt);
                stats.landing_fuel_kg = Some(snap.fuel_total_kg);
                stats.bounce_count = 0;
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
            // Touchdown analyzer window: refine peak descent rate, peak G,
            // and count bounces while the wheels are still settling.
            if let Some(touchdown) = stats.landing_at {
                let in_window = (now - touchdown).num_seconds() <= TOUCHDOWN_WINDOW_SECS;
                if in_window {
                    // Peak |V/S| — keep the most negative number we see.
                    let peak_vs = stats.landing_peak_vs_fpm.unwrap_or(0.0);
                    if snap.vertical_speed_fpm < peak_vs {
                        stats.landing_peak_vs_fpm = Some(snap.vertical_speed_fpm);
                    }
                    // Peak G — highest reading wins.
                    let peak_g = stats.landing_peak_g_force.unwrap_or(0.0);
                    if snap.g_force > peak_g {
                        stats.landing_peak_g_force = Some(snap.g_force);
                    }
                    // Bounce: lifted off the ground again before the
                    // window closed — count it and (loosely) treat the
                    // next contact as a fresh touchdown for peak
                    // tracking. Phase stays Landing so we don't loop
                    // through Final again.
                    if was_on_ground && !snap.on_ground {
                        stats.bounce_count = stats.bounce_count.saturating_add(1);
                    }
                } else if stats.landing_score.is_none() {
                    // Window just closed — finalise the score once.
                    let peak_vs = stats.landing_peak_vs_fpm.unwrap_or(0.0);
                    let peak_g = stats.landing_peak_g_force.unwrap_or(0.0);
                    let score = LandingScore::classify(peak_vs, peak_g, stats.bounce_count);
                    stats.landing_score = Some(score);
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

/// Build the custom-fields map sent in `POST /api/pireps/{id}/file`. Field
/// names follow the de-facto vmsACARS convention so VAs that already configured
/// fields for vmsACARS see them populate without any work.
fn build_pirep_fields(
    flight: &ActiveFlight,
    stats: &FlightStats,
) -> HashMap<String, String> {
    let mut f: HashMap<String, String> = HashMap::new();

    f.insert(
        "Source".into(),
        format!("CloudeAcars/{}", env!("CARGO_PKG_VERSION")),
    );
    f.insert("Departure Airport".into(), flight.dpt_airport.clone());
    f.insert("Arrival Airport".into(), flight.arr_airport.clone());

    // Times: render as readable UTC `HH:MM:SS UTC` instead of full
    // ISO 8601 with microseconds. The PIREP custom-field column on
    // phpVMS is narrow and pilots reading it want a glanceable
    // timestamp, not a 30-character mil-spec timestamp. The full
    // ISO string is still available in the FlightStarted / Landing
    // events written to the JSONL flight log on disk.
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

    // Skip 0-value weight/fuel fields — Fenix and some addons don't wire the
    // SimVars reliably and we'd otherwise spam phpVMS custom fields with
    // "0 kg" placeholders that look broken to dispatchers.
    if let Some(w) = stats.takeoff_weight_kg.filter(|v| *v > 0.0) {
        f.insert("Takeoff Weight".into(), format!("{:.0} kg", w));
    }
    if let Some(rate) = stats.landing_rate_fpm {
        // Negative on touchdown — preserve sign so VAs see e.g. -221 fpm.
        f.insert("Landing Rate".into(), format!("{:.0} fpm", rate));
    }
    if let Some(g) = stats.landing_g_force {
        f.insert("Landing G-Force".into(), format!("{:.2} G", g));
    }
    if let Some(p) = stats.landing_pitch_deg {
        f.insert("Landing Pitch".into(), format!("{:.1}°", p));
    }
    if let Some(s) = stats.landing_speed_kt.filter(|v| *v > 0.0) {
        f.insert("Landing Speed".into(), format!("{:.0} kt", s));
    }
    if let Some(h) = stats.landing_heading_deg {
        f.insert("Landing Heading".into(), format!("{:03.0}°", h));
    }
    if let Some(w) = stats.landing_weight_kg.filter(|v| *v > 0.0) {
        f.insert("Landing Weight".into(), format!("{:.0} kg", w));
    }
    if let Some(fuel) = stats.landing_fuel_kg.filter(|v| *v > 0.0) {
        f.insert("Landing Fuel".into(), format!("{:.0} kg", fuel));
    }
    if let Some(b) = stats.block_fuel_kg.filter(|v| *v > 0.0) {
        f.insert("Block Fuel".into(), format!("{:.0} kg", b));
    }
    if let (Some(b), Some(c)) = (stats.block_fuel_kg, stats.last_fuel_kg) {
        if b > 0.0 && b > c {
            f.insert("Fuel Used".into(), format!("{:.0} kg", b - c));
        }
    }

    // Landing analyzer (Phase I) — peak values are usually worse than the
    // initial-touchdown sample, so include both for VA review.
    if let Some(score) = stats.landing_score {
        f.insert("Landing Score".into(), score.label().to_string());
    }
    if let Some(vs) = stats.landing_peak_vs_fpm {
        f.insert("Peak Landing Rate".into(), format!("{:.0} fpm", vs));
    }
    if let Some(g) = stats.landing_peak_g_force {
        f.insert("Peak Landing G".into(), format!("{:.2} G", g));
    }
    if stats.bounce_count > 0 {
        f.insert("Bounces".into(), stats.bounce_count.to_string());
    }

    // METAR snapshots (Phase J.2) — captured at takeoff / touchdown.
    if let Some(raw) = stats.dep_metar_raw.as_ref().filter(|s| !s.is_empty()) {
        f.insert("Departure METAR".into(), raw.clone());
    }
    if let Some(raw) = stats.arr_metar_raw.as_ref().filter(|s| !s.is_empty()) {
        f.insert("Arrival METAR".into(), raw.clone());
    }

    // ATC-derived gates and approach runway (from MSFS SimVars).
    if let Some(g) = stats.dep_gate.as_ref().filter(|s| !s.is_empty()) {
        f.insert("Departure Gate".into(), g.clone());
    }
    if let Some(g) = stats.arr_gate.as_ref().filter(|s| !s.is_empty()) {
        f.insert("Arrival Gate".into(), g.clone());
    }
    if let Some(rw) = stats.approach_runway.as_ref().filter(|s| !s.is_empty()) {
        f.insert("Approach Runway".into(), rw.clone());
    }

    // Computed durations.
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
    if let (Some(land), Some(blocks_on)) = (stats.landing_at, stats.block_on_at) {
        f.insert(
            "Taxi In Time".into(),
            humanize_duration_minutes((blocks_on - land).num_minutes()),
        );
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
    let mut lines = Vec::new();
    lines.push(format!(
        "{} {} → {}",
        flight.flight_number, flight.dpt_airport, flight.arr_airport
    ));
    if let Some(t) = stats.block_off_at {
        lines.push(format!("Blocks off: {}", t.to_rfc3339()));
    }
    if let Some(t) = stats.takeoff_at {
        lines.push(format!("Takeoff: {}", t.to_rfc3339()));
    }
    if let Some(t) = stats.landing_at {
        lines.push(format!("Landing: {}", t.to_rfc3339()));
    }
    if let Some(t) = stats.block_on_at {
        lines.push(format!("Blocks on: {}", t.to_rfc3339()));
    }
    if let Some(rate) = stats.landing_rate_fpm {
        lines.push(format!(
            "Touchdown: {:.0} fpm, {:.2} G, {:.1}° pitch, {:.0} kt",
            rate,
            stats.landing_g_force.unwrap_or(0.0),
            stats.landing_pitch_deg.unwrap_or(0.0),
            stats.landing_speed_kt.unwrap_or(0.0),
        ));
    }
    if let (Some(b), Some(c)) = (stats.block_fuel_kg, stats.last_fuel_kg) {
        if b > c {
            lines.push(format!("Fuel: {:.0} kg block / {:.0} kg used", b, b - c));
        }
    }
    lines.push(format!(
        "CloudeAcars {} ({} positions, {:.1} nm)",
        env!("CARGO_PKG_VERSION"),
        stats.position_count,
        stats.distance_nm
    ));
    lines.join("\n")
}

/// Map our internal `FlightPhase` to the phpVMS PirepStatus code we POST in
/// `update_pirep`. Some phases collapse to the same code (e.g. Climb and
/// Cruise both report ENR).
fn phase_to_status(phase: FlightPhase) -> Option<&'static str> {
    match phase {
        FlightPhase::Preflight | FlightPhase::Boarding => Some("BST"),
        FlightPhase::Pushback => Some("OFB"),
        FlightPhase::TaxiOut => Some("TXI"),
        FlightPhase::TakeoffRoll => Some("TKO"),
        FlightPhase::Takeoff => Some("TOF"),
        FlightPhase::Climb | FlightPhase::Cruise => Some("ENR"),
        FlightPhase::Descent => Some("TEN"),
        FlightPhase::Approach | FlightPhase::Final => Some("APP"),
        FlightPhase::Landing | FlightPhase::TaxiIn => Some("LAN"),
        FlightPhase::BlocksOn | FlightPhase::Arrived => Some("ARR"),
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
fn announce_landing_score(app: &AppHandle, flight: &ActiveFlight) {
    let stats = flight.stats.lock().expect("flight stats");
    if !stats.landing_score_announced {
        if let Some(score) = stats.landing_score {
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
            log_activity_handle(
                app,
                level,
                format!("Touchdown: {}", score.label()),
                Some(format!(
                    "V/S {:.0} fpm, G {:.2}{}",
                    peak_vs.abs(),
                    peak_g,
                    bounce_part,
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
        }
    }
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
        let icao = if icao_raw.contains('.') || icao_raw.is_empty() {
            "?"
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
        log_activity_handle(
            app,
            ActivityLevel::Info,
            format!("Aircraft: {title}"),
            Some(format!(
                "Type {icao} · Reg {reg} · Sim {:?} · Profile: {}",
                snap.simulator,
                snap.aircraft_profile.label()
            )),
        );
    }

    // ---- Profile change (after first tick) — pilot swapped airframes
    if stats.last_logged_profile != Some(snap.aircraft_profile) {
        if !first_tick {
            log_activity_handle(
                app,
                ActivityLevel::Info,
                format!(
                    "Aircraft profile changed → {}",
                    snap.aircraft_profile.label()
                ),
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
    // Map normalised 0.0..1.0 to a detent 0..5. Airbus has six positions
    // (UP / 1 / 1+F / 2 / 3 / FULL); Boeing tops out at 4. Either way,
    // rounding keeps every transition discrete and stops floating-point
    // jitter on a moving lever from spamming the log.
    let flaps_detent = (snap.flaps_position.clamp(0.0, 1.0) * 5.0).round() as u8;
    if stats.last_logged_flaps_detent != Some(flaps_detent) {
        if let Some(prev) = stats.last_logged_flaps_detent {
            // Direction matters: pilots care whether they're configuring
            // for departure ("Flaps 1") or for landing ("Flaps FULL").
            let dir = if flaps_detent > prev { "↓" } else { "↑" };
            log_activity_handle(
                app,
                ActivityLevel::Info,
                format!("Flaps {dir} {}", detent_label(flaps_detent)),
                Some(format!("IAS {:.0} kt, AGL {:.0} ft", snap.indicated_airspeed_kt, snap.altitude_agl_ft)),
            );
        }
        stats.last_logged_flaps_detent = Some(flaps_detent);
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

    // ---- FCU selected values (debounced 2 s).
    // Reborrow `stats` as an explicit `&mut FlightStats` so the
    // borrow checker can split-borrow disjoint fields when we
    // pass two `&mut self.field` arguments to the helper. Without
    // this it sees the MutexGuard's deref and refuses.
    let now_ts = Utc::now();
    let stats: &mut FlightStats = &mut stats;
    fcu_debounce(
        app,
        snap.fcu_selected_altitude_ft,
        &mut stats.last_logged_fcu_alt,
        &mut stats.pending_fcu_alt,
        now_ts,
        "Selected ALT",
        "ft",
    );
    fcu_debounce(
        app,
        snap.fcu_selected_heading_deg,
        &mut stats.last_logged_fcu_hdg,
        &mut stats.pending_fcu_hdg,
        now_ts,
        "Selected HDG",
        "°",
    );
    fcu_debounce(
        app,
        snap.fcu_selected_speed_kt,
        &mut stats.last_logged_fcu_spd,
        &mut stats.pending_fcu_spd,
        now_ts,
        "Selected SPD",
        "kt",
    );
    fcu_debounce(
        app,
        snap.fcu_selected_vs_fpm,
        &mut stats.last_logged_fcu_vs,
        &mut stats.pending_fcu_vs,
        now_ts,
        "Selected V/S",
        "fpm",
    );
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
/// for every click on the way from 12000 to 36000. We hold the new
/// value for FCU_DEBOUNCE_SECS, and only emit the log entry once it
/// has held steady for that long.
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

/// Apply the selected kind to the MSFS adapter (start / stop / no-op).
/// X-Plane kinds are accepted as a setting but the X-Plane adapter is Phase 2;
/// for now we just stop the MSFS adapter and let the UI display the "coming
/// soon" state.
fn apply_sim_kind(_state: &tauri::State<'_, AppState>, _kind: SimKind) {
    #[cfg(target_os = "windows")]
    {
        let mut adapter = _state.msfs.lock().expect("msfs lock");
        if _kind.is_msfs() {
            adapter.start(_kind);
        } else {
            adapter.stop();
        }
    }
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
fn sim_status(app: AppHandle, _state: tauri::State<'_, AppState>) -> SimStatus {
    let kind = read_sim_config(&app).kind;
    #[cfg(target_os = "windows")]
    {
        let adapter = _state.msfs.lock().expect("msfs lock");
        let (state_str, last_error) = if kind.is_msfs() {
            let s = match adapter.state() {
                sim_msfs::ConnectionState::Disconnected => "disconnected",
                sim_msfs::ConnectionState::Connecting => "connecting",
                sim_msfs::ConnectionState::Connected => "connected",
            };
            (s, adapter.last_error())
        } else if kind.is_xplane() {
            ("disconnected", Some("X-Plane support arrives in Phase 2".into()))
        } else {
            ("disconnected", None)
        };
        let snapshot = if kind.is_msfs() {
            adapter.snapshot()
        } else {
            None
        };
        SimStatus {
            state: state_str.into(),
            kind: kind_str(kind).into(),
            snapshot,
            last_error,
            available: kind.is_msfs() || kind == SimKind::Off,
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let last_error = if kind.is_msfs() {
            Some("MSFS adapter is Windows-only".into())
        } else if kind.is_xplane() {
            Some("X-Plane support arrives in Phase 2".into())
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
        planned_registration,
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
    });

    {
        let mut guard = state.active_flight.lock().expect("active_flight lock");
        *guard = Some(flight);
    }
    // NOTE: do not spawn the streamer here. The frontend-driven
    // `flight_resume_confirm` does it after the resume banner is dismissed.
}

// ---- Bootstrap ----

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,cloudeacars=debug"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_tracing();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "CloudeAcars starting");

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
            message: format!("CloudeAcars v{} gestartet", env!("CARGO_PKG_VERSION")),
            detail: None,
        };
        tracing::info!(message = %banner.message, "activity");
        log.push_back(banner);
        while log.len() > ACTIVITY_LOG_CAPACITY {
            log.pop_front();
        }
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(app_state)
        .setup(|app| {
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
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            phpvms_login,
            phpvms_logout,
            phpvms_load_session,
            phpvms_get_bids,
            sim_get_kind,
            sim_set_kind,
            sim_status,
            airport_get,
            flight_status,
            flight_start,
            flight_end,
            flight_end_manual,
            flight_cancel,
            activity_log_get,
            activity_log_clear,
            metar_get,
            flight_forget,
            flight_discover_resumable,
            flight_adopt,
            flight_resume_confirm,
            inspector_add,
            inspector_remove,
            inspector_list,
        ])
        .run(tauri::generate_context!())
        .expect("error while running CloudeAcars");
}
