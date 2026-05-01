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
        // On the ground (taxi, pushback, takeoff roll) — 10 s is plenty;
        // movements are slow and the live map just needs a clean trail.
        FlightPhase::Boarding
        | FlightPhase::Pushback
        | FlightPhase::TaxiOut
        | FlightPhase::TakeoffRoll
        | FlightPhase::TaxiIn => 10,
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
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
enum ActivityLevel {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize)]
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
        }
    }

    fn apply_to(self, stats: &mut FlightStats) {
        stats.distance_nm = self.distance_nm;
        stats.position_count = self.position_count;
        stats.phase = self.phase;
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
    }
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

    // ---- Fuel tracking ----
    block_fuel_kg: Option<f32>,
    last_fuel_kg: Option<f32>,

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
    last_logged_ap: Option<ApState>,
    /// Debounce: when did we first observe the *current* AP master state?
    /// We only emit a "Autopilot ENGAGED/OFF" log entry if the new state
    /// has held for `AP_DEBOUNCE_SECS`. Stops a misbehaving LVar (pulsed
    /// momentary buttons, sim-engine restarts) from flooding the log.
    pending_ap_master: Option<bool>,
    pending_ap_master_since: Option<DateTime<Utc>>,
    last_logged_parking_brake: Option<bool>,
    last_logged_engines_running: Option<u8>,
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
            log_activity(
                &state,
                ActivityLevel::Info,
                format!("Session restored — {}", profile.name),
                Some(format!("Sim: {:?}", saved_kind)),
            );
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
/// Skipped (returns empty) if a flight is already attached locally.
#[tauri::command]
async fn flight_discover_resumable(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ResumableFlight>, UiError> {
    {
        let guard = state.active_flight.lock().expect("active_flight lock");
        if guard.is_some() {
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

/// File the active PIREP with computed final stats. Refuses to file if any
/// of the minimum-quality checks in `validate_for_filing` fail — the UI then
/// surfaces a dialog letting the user cancel the flight or file as a manual
/// PIREP via [`flight_end_manual`].
#[tauri::command]
async fn flight_end(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), UiError> {
    // Validate WITHOUT removing the flight from state, so a failed validation
    // leaves the user able to retry, edit-and-file-manual, or cancel.
    {
        let guard = state.active_flight.lock().expect("active_flight lock");
        let flight = guard
            .as_ref()
            .ok_or_else(|| UiError::new("no_active_flight", "no flight is active"))?;
        let stats = flight.stats.lock().expect("flight stats");
        let elapsed_minutes = (Utc::now() - flight.started_at).num_minutes() as i32;
        let missing = validate_for_filing(flight, &stats, elapsed_minutes);
        if !missing.is_empty() {
            tracing::warn!(
                pirep_id = %flight.pirep_id,
                missing = ?missing,
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
        let elapsed_minutes = (Utc::now() - flight.started_at).num_minutes() as i32;

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
        let distance_nm = stats.distance_nm;
        let fields = build_pirep_fields(&flight, &stats);
        let notes = build_pirep_notes(&flight, &stats);

        FileBody {
            flight_time: Some(elapsed_minutes.max(0)),
            fuel_used,
            distance: Some(distance_nm),
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
                Some(format!(
                    "Distance {:.1} nm, fuel {:.0} lb",
                    body.distance.unwrap_or(0.0),
                    body.fuel_used.unwrap_or(0.0)
                )),
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
        let elapsed_minutes = (Utc::now() - flight.started_at).num_minutes() as i32;
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
            flight_time: Some(elapsed_minutes.max(0)),
            fuel_used,
            distance: Some(distance_nm),
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
    spawn_position_streamer(app, flight, client);
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

            match client
                .post_positions(&flight.pirep_id, &[position])
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
                }
                Err(e) => {
                    tracing::warn!(
                        pirep_id = %flight.pirep_id,
                        error = %e,
                        "position post failed; will retry on next tick"
                    );
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
            if should_persist {
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

    // Capture block fuel on the very first snapshot.
    if stats.block_fuel_kg.is_none() {
        stats.block_fuel_kg = Some(snap.fuel_total_kg);
    }

    let now = Utc::now();
    let prev_phase = stats.phase;
    let mut next_phase = prev_phase;
    let was_on_ground = stats.was_on_ground.unwrap_or(snap.on_ground);
    let was_brake = stats.was_parking_brake.unwrap_or(snap.parking_brake);

    // Match on a local Copy so the rest of the body is free to mutate `stats`.
    match prev_phase {
        FlightPhase::Boarding => {
            if was_brake && !snap.parking_brake && snap.on_ground {
                next_phase = FlightPhase::Pushback;
                stats.block_off_at = Some(now);
            }
        }
        FlightPhase::Pushback => {
            if snap.groundspeed_kt > 5.0 && snap.on_ground {
                next_phase = FlightPhase::TaxiOut;
            }
        }
        FlightPhase::TaxiOut => {
            if snap.groundspeed_kt > 40.0 && snap.on_ground {
                next_phase = FlightPhase::TakeoffRoll;
            }
        }
        FlightPhase::TakeoffRoll => {
            if was_on_ground && !snap.on_ground {
                next_phase = FlightPhase::Takeoff;
                stats.takeoff_at = Some(now);
                stats.takeoff_fuel_kg = Some(snap.fuel_total_kg);
                let zfw = snap.zfw_kg.unwrap_or(0.0);
                let weight = zfw as f64 + snap.fuel_total_kg as f64;
                if weight > 0.0 {
                    stats.takeoff_weight_kg = Some(weight);
                }
            }
        }
        FlightPhase::Takeoff => {
            if snap.altitude_agl_ft > 500.0 {
                next_phase = FlightPhase::Climb;
            }
        }
        FlightPhase::Climb | FlightPhase::Cruise => {
            if snap.vertical_speed_fpm < -300.0 {
                next_phase = FlightPhase::Descent;
            }
        }
        FlightPhase::Descent => {
            if snap.altitude_agl_ft < 5000.0 {
                next_phase = FlightPhase::Approach;
            }
        }
        FlightPhase::Approach => {
            if snap.altitude_agl_ft < 1500.0 {
                next_phase = FlightPhase::Final;
            }
        }
        FlightPhase::Final => {
            if !was_on_ground && snap.on_ground {
                next_phase = FlightPhase::Landing;
                stats.landing_at = Some(now);
                stats.landing_rate_fpm = Some(snap.vertical_speed_fpm);
                stats.landing_g_force = Some(snap.g_force);
                stats.landing_pitch_deg = Some(snap.pitch_deg);
                stats.landing_speed_kt = Some(snap.indicated_airspeed_kt);
                stats.landing_heading_deg = Some(snap.heading_deg_magnetic);
                stats.landing_fuel_kg = Some(snap.fuel_total_kg);
                // Seed the landing-analyzer peaks with the initial
                // touchdown sample. Subsequent ticks within the window
                // (TOUCHDOWN_WINDOW_SECS) refine these to the worst
                // observed value.
                stats.landing_peak_vs_fpm = Some(snap.vertical_speed_fpm);
                stats.landing_peak_g_force = Some(snap.g_force);
                stats.bounce_count = 0;
                let zfw = snap.zfw_kg.unwrap_or(0.0);
                let weight = zfw as f64 + snap.fuel_total_kg as f64;
                if weight > 0.0 {
                    stats.landing_weight_kg = Some(weight);
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
            }
        }
        FlightPhase::BlocksOn
        | FlightPhase::Arrived
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

    if let Some(t) = stats.block_off_at {
        f.insert("Blocks Off Time".into(), t.to_rfc3339());
    }
    if let Some(t) = stats.takeoff_at {
        f.insert("Takeoff Time".into(), t.to_rfc3339());
    }
    if let Some(t) = stats.landing_at {
        f.insert("Landing Time".into(), t.to_rfc3339());
    }
    if let Some(t) = stats.block_on_at {
        f.insert("Blocks On Time".into(), t.to_rfc3339());
    }

    if let Some(w) = stats.takeoff_weight_kg {
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
    if let Some(s) = stats.landing_speed_kt {
        f.insert("Landing Speed".into(), format!("{:.0} kt", s));
    }
    if let Some(h) = stats.landing_heading_deg {
        f.insert("Landing Heading".into(), format!("{:03.0}°", h));
    }
    if let Some(w) = stats.landing_weight_kg {
        f.insert("Landing Weight".into(), format!("{:.0} kg", w));
    }
    if let Some(fuel) = stats.landing_fuel_kg {
        f.insert("Landing Fuel".into(), format!("{:.0} kg", fuel));
    }
    if let Some(b) = stats.block_fuel_kg {
        f.insert("Block Fuel".into(), format!("{:.0} kg", b));
    }
    if let (Some(b), Some(c)) = (stats.block_fuel_kg, stats.last_fuel_kg) {
        if b > c {
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
    let first_tick = stats.last_logged_squawk.is_none()
        && stats.last_logged_com1.is_none()
        && stats.last_logged_lights.is_none();
    if first_tick {
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

    // ---- COM frequencies
    if let Some(f) = snap.com1_mhz {
        if stats.last_logged_com1.map(|v| (v - f).abs() > 0.001) != Some(false) {
            if !first_tick {
                log_activity_handle(
                    app,
                    ActivityLevel::Info,
                    format!("COM1 → {:.3} MHz", f),
                    None,
                );
            }
            stats.last_logged_com1 = Some(f);
        }
    }
    if let Some(f) = snap.com2_mhz {
        if stats.last_logged_com2.map(|v| (v - f).abs() > 0.001) != Some(false) {
            if !first_tick {
                log_activity_handle(
                    app,
                    ActivityLevel::Info,
                    format!("COM2 → {:.3} MHz", f),
                    None,
                );
            }
            stats.last_logged_com2 = Some(f);
        }
    }

    // ---- NAV frequencies
    if let Some(f) = snap.nav1_mhz {
        if stats.last_logged_nav1.map(|v| (v - f).abs() > 0.001) != Some(false) {
            if !first_tick {
                log_activity_handle(
                    app,
                    ActivityLevel::Info,
                    format!("NAV1 → {:.3} MHz", f),
                    None,
                );
            }
            stats.last_logged_nav1 = Some(f);
        }
    }
    if let Some(f) = snap.nav2_mhz {
        if stats.last_logged_nav2.map(|v| (v - f).abs() > 0.001) != Some(false) {
            if !first_tick {
                log_activity_handle(
                    app,
                    ActivityLevel::Info,
                    format!("NAV2 → {:.3} MHz", f),
                    None,
                );
            }
            stats.last_logged_nav2 = Some(f);
        }
    }

    // ---- Exterior lights
    let lights = LightsState {
        landing: snap.light_landing.unwrap_or(false),
        beacon: snap.light_beacon.unwrap_or(false),
        strobe: snap.light_strobe.unwrap_or(false),
        taxi: snap.light_taxi.unwrap_or(false),
        nav: snap.light_nav.unwrap_or(false),
        logo: snap.light_logo.unwrap_or(false),
    };
    if stats.last_logged_lights != Some(lights) {
        if let Some(prev) = stats.last_logged_lights {
            // Log per-light transitions so the pilot sees exactly what changed.
            let changes = [
                ("Landing", prev.landing, lights.landing),
                ("Beacon", prev.beacon, lights.beacon),
                ("Strobe", prev.strobe, lights.strobe),
                ("Taxi", prev.taxi, lights.taxi),
                ("Nav", prev.nav, lights.nav),
                ("Logo", prev.logo, lights.logo),
            ];
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
    if stats.last_logged_engines_running != Some(snap.engines_running) {
        if let Some(prev) = stats.last_logged_engines_running {
            if prev < snap.engines_running {
                log_activity_handle(
                    app,
                    ActivityLevel::Info,
                    format!(
                        "Engine started — {} of {} running",
                        snap.engines_running, snap.engines_running
                    ),
                    None,
                );
            } else if prev > snap.engines_running {
                log_activity_handle(
                    app,
                    ActivityLevel::Info,
                    format!("Engine shutdown — {} running", snap.engines_running),
                    None,
                );
            }
        }
        stats.last_logged_engines_running = Some(snap.engines_running);
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

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::default())
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
            flight_forget,
            flight_discover_resumable,
            flight_adopt,
            flight_resume_confirm,
        ])
        .run(tauri::generate_context!())
        .expect("error while running CloudeAcars");
}
