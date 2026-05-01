//! Simulator-agnostic abstractions.
//!
//! All simulator adapters (`sim-msfs`, `sim-xplane`, future `sim-prepar3d`, …) implement
//! the `SimAdapter` trait and emit `SimSnapshot`s at a configurable rate. The flight phase
//! FSM and recorder consume these snapshots without knowing which simulator they came from.

#![allow(dead_code)] // Phase 1 stub.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// One sample of simulator telemetry.
///
/// Field set tracks the requirements spec §8 ("Simulator-Daten und Telemetrie").
/// Adapters fill what's available; downstream consumers tolerate `None`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimSnapshot {
    pub timestamp: DateTime<Utc>,

    // Position
    pub lat: f64,
    pub lon: f64,
    pub altitude_msl_ft: f64,
    pub altitude_agl_ft: f64,

    // Attitude / motion
    pub heading_deg_true: f32,
    pub heading_deg_magnetic: f32,
    pub pitch_deg: f32,
    pub bank_deg: f32,
    pub vertical_speed_fpm: f32,

    // Speeds
    pub groundspeed_kt: f32,
    pub indicated_airspeed_kt: f32,
    pub true_airspeed_kt: f32,

    // Forces & flags
    pub g_force: f32,
    pub on_ground: bool,
    pub parking_brake: bool,
    pub stall_warning: bool,
    pub overspeed_warning: bool,
    pub paused: bool,
    pub slew_mode: bool,
    pub simulation_rate: f32,

    // Configuration
    pub gear_position: f32, // 0.0 = up, 1.0 = down
    pub flaps_position: f32,
    pub engines_running: u8,

    // Fuel & weight
    pub fuel_total_kg: f32,
    pub fuel_used_kg: f32,
    pub zfw_kg: Option<f32>,
    pub payload_kg: Option<f32>,
    /// Gross weight in kg (`TOTAL WEIGHT` SimVar). Includes fuel + payload.
    /// `None` for aircraft addons that don't wire it (notably Fenix —
    /// then we fall back to addon LVars or leave the PIREP field blank).
    pub total_weight_kg: Option<f32>,

    // ---- Touchdown sample (latched by MSFS at the moment the gear
    // touches the ground; values stay valid until the next takeoff).
    // Reading these is more reliable than sampling V/S continuously
    // and trying to guess which frame was the actual touchdown — the
    // sim itself takes the snapshot for us. ----
    /// Vertical velocity at touchdown, fpm. Negative on a real landing
    /// (we mirror MSFS' sign convention).
    pub touchdown_vs_fpm: Option<f32>,
    pub touchdown_pitch_deg: Option<f32>,
    pub touchdown_bank_deg: Option<f32>,
    pub touchdown_heading_mag_deg: Option<f32>,
    pub touchdown_lat: Option<f64>,
    pub touchdown_lon: Option<f64>,

    // Environment
    pub wind_direction_deg: Option<f32>,
    pub wind_speed_kt: Option<f32>,
    pub qnh_hpa: Option<f32>,
    pub outside_air_temp_c: Option<f32>,

    // Identity
    pub aircraft_title: Option<String>,
    pub aircraft_icao: Option<String>,
    /// Aircraft registration / tail number as set in the sim (e.g. "D-AILU").
    pub aircraft_registration: Option<String>,
    pub simulator: Simulator,
    pub sim_version: Option<String>,

    // ---- Avionics (Phase H.1) ----
    /// 4-digit transponder / squawk code, e.g. 7000.
    pub transponder_code: Option<u16>,
    /// Active COM1 frequency in MHz (e.g. 121.500).
    pub com1_mhz: Option<f32>,
    pub com2_mhz: Option<f32>,
    pub nav1_mhz: Option<f32>,
    pub nav2_mhz: Option<f32>,

    // ---- Exterior lights ----
    pub light_landing: Option<bool>,
    pub light_beacon: Option<bool>,
    pub light_strobe: Option<bool>,
    pub light_taxi: Option<bool>,
    pub light_nav: Option<bool>,
    pub light_logo: Option<bool>,

    // ---- Autopilot ----
    pub autopilot_master: Option<bool>,
    pub autopilot_heading: Option<bool>,
    pub autopilot_altitude: Option<bool>,
    pub autopilot_nav: Option<bool>,
    pub autopilot_approach: Option<bool>,

    // ---- Powerplant (totals — per-engine arrays land later) ----
    /// Total fuel-flow across all running engines, kg/h.
    pub fuel_flow_kg_per_h: Option<f32>,

    // ---- Surfaces ----
    /// 0.0..1.0, current position of the spoiler / speed-brake handle.
    /// Drives both ground-spoiler and in-flight speed-brake feedback.
    pub spoilers_handle_position: Option<f32>,
    /// Auto-spoilers armed for landing.
    pub spoilers_armed: Option<bool>,

    // ---- Systems ----
    pub apu_switch: Option<bool>,
    /// 0..100. Useful to tell "starting" (rising) from "running" (~95).
    pub apu_pct_rpm: Option<f32>,
    pub battery_master: Option<bool>,
    pub avionics_master: Option<bool>,
    pub pitot_heat: Option<bool>,
    /// "any engine has anti-ice on" — combined per-engine readings.
    pub engine_anti_ice: Option<bool>,
    /// Wing / structural deice (Airbus "WING ANTI ICE").
    pub wing_anti_ice: Option<bool>,

    // ---- ATC / Gate info (from MSFS ATC system) ----
    /// Stand identifier from `ATC PARKING NAME` (e.g. "GATE_HEAVY").
    /// Only filled while the aircraft sits on a named stand; goes
    /// empty after pushback. The recorder snapshots it at flight
    /// start (departure gate) and on BlocksOn (arrival gate).
    pub parking_name: Option<String>,
    /// Stand number suffix (e.g. "12", "A 8") to combine with the name.
    pub parking_number: Option<String>,
    /// `ATC RUNWAY SELECTED` — the runway the MSFS ATC system
    /// currently has the aircraft cleared for. Useful to record the
    /// approach runway at touchdown.
    pub selected_runway: Option<String>,

    // ---- Aircraft profile (Phase H.4) ----
    /// Detected aircraft profile. Drives which set of variables (default
    /// MSFS SimVars vs add-on-specific LVars) the adapter reads from.
    /// `Default` covers Asobo + most payware that pipes state into the
    /// standard SimVars (e.g. PMDG 737/777 mostly do); the named variants
    /// cover study-level add-ons that pull state from their own LVars.
    pub aircraft_profile: AircraftProfile,
}

/// Identifies the active aircraft add-on so the adapter can read the right
/// LVars (and the activity log can show the pilot which mapping is in use).
/// Detection runs on every snapshot but the answer is cached on the adapter
/// — the title only changes when the pilot loads a different airframe.
///
/// Mapping status (Phase H.4 backlog):
///   * `FbwA32nx`   — LVars wired (untested live, FBW not loaded yet).
///   * `FenixA320`  — Lights / parking brake / flaps wired and verified
///                    in MSFS 2024. AP indicator LVars (`I_FCU_AP*`) were
///                    observed flickering and are intentionally disabled
///                    until a stable source is identified.
///   * `Pmdg737`    — detection only; LVars TBD (PMDG ships its own
///                    SimConnect ClientData SDK, not plain LVars — needs
///                    a separate subscribe path).
///   * `Pmdg777`    — same as 737.
///   * `IniA340`    — detection only; LVar list TBD.
///   * `IniA350`    — detection only; LVar list TBD.
///   * `IniA346Pro` — detection only; LVar list TBD.
///
/// Cross-cutting issue: ~50 LVar fields in one SimConnectObject macro
/// appears to overflow the read path on Fenix (lights/flaps return 0.0
/// despite live cockpit changes); future work is a runtime-defined LVar
/// reader (likely via `msfs-rs` or MobiFlight WASM client data) so each
/// profile only subscribes to the LVars it actually uses.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AircraftProfile {
    /// Standard MSFS SimVars only — works for Asobo and most payware.
    #[default]
    Default,
    /// FlyByWire A32NX (community Airbus). Reads `L:A32NX_*` LVars.
    /// LVar reference: github.com/flybywiresim/aircraft/blob/master/
    /// fbw-a32nx/docs/a320-simvars.md
    FbwA32nx,
    /// Fenix Simulations A320 v2. LVars in `Cockpit_Behavior.xml` plus
    /// the Fenix knowledge base.
    FenixA320,
    /// PMDG 737 (700/800/900). LVars are in PMDG's SDK header.
    Pmdg737,
    /// PMDG 777 (200/300). Same SDK family as the 737.
    Pmdg777,
    /// INIBuilds A340 (Standard Edition).
    IniA340,
    /// INIBuilds A350.
    IniA350,
    /// INIBuilds A340-600 Pro.
    IniA346Pro,
}

impl AircraftProfile {
    /// Best-effort identification from the MSFS `TITLE` and `ATC MODEL`
    /// strings. Falls back to `Default` when nothing matches — that's the
    /// safe path: we keep using the standard SimVar set we already wire
    /// up, so undetected aircraft still produce a working PIREP.
    pub fn detect(title: &str, icao: &str) -> Self {
        let t = title.to_lowercase();
        let i = icao.to_lowercase();
        // FlyByWire — distinguish from real Airbus by FBW's marker text.
        if t.contains("flybywire") || t.contains("fbw a32nx") || t.contains("a32nx") {
            return Self::FbwA32nx;
        }
        // Fenix — title typically begins with "FenixA320" / "FenixA319".
        if t.contains("fenix") {
            return Self::FenixA320;
        }
        // PMDG 737 — covers 736/737/738/739 NG and MAX variants.
        if t.contains("pmdg") && (t.contains("737") || i.contains("b73")) {
            return Self::Pmdg737;
        }
        // PMDG 777 — 772/773 ER/Freighter.
        if t.contains("pmdg") && (t.contains("777") || i.contains("b77")) {
            return Self::Pmdg777;
        }
        // INIBuilds A350.
        if t.contains("inibuilds") && t.contains("a350") {
            return Self::IniA350;
        }
        // INIBuilds A340-600 Pro — pro suffix is the discriminator vs the
        // standard A340 build.
        if t.contains("inibuilds") && (t.contains("a346") || t.contains("a340-600"))
            && t.contains("pro")
        {
            return Self::IniA346Pro;
        }
        // INIBuilds A340 base.
        if t.contains("inibuilds") && t.contains("a340") {
            return Self::IniA340;
        }
        Self::Default
    }

    /// Short human-readable label for the activity log.
    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "Default (standard SimVars)",
            Self::FbwA32nx => "FlyByWire A32NX",
            Self::FenixA320 => "Fenix A320",
            Self::Pmdg737 => "PMDG 737",
            Self::Pmdg777 => "PMDG 777",
            Self::IniA340 => "INIBuilds A340",
            Self::IniA350 => "INIBuilds A350",
            Self::IniA346Pro => "INIBuilds A340-600 Pro",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Simulator {
    Msfs2020,
    Msfs2024,
    XPlane11,
    XPlane12,
    Other,
}

/// User-selected simulator kind, persisted across app restarts.
/// Drives which adapter the app boots after login.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SimKind {
    Off,
    Msfs2020,
    Msfs2024,
    XPlane11,
    XPlane12,
}

impl Default for SimKind {
    fn default() -> Self {
        SimKind::Msfs2024
    }
}

impl SimKind {
    pub fn is_msfs(self) -> bool {
        matches!(self, SimKind::Msfs2020 | SimKind::Msfs2024)
    }

    pub fn is_xplane(self) -> bool {
        matches!(self, SimKind::XPlane11 | SimKind::XPlane12)
    }

    /// Map to the `Simulator` enum used inside `SimSnapshot`.
    pub fn as_simulator(self) -> Simulator {
        match self {
            SimKind::Off => Simulator::Other,
            SimKind::Msfs2020 => Simulator::Msfs2020,
            SimKind::Msfs2024 => Simulator::Msfs2024,
            SimKind::XPlane11 => Simulator::XPlane11,
            SimKind::XPlane12 => Simulator::XPlane12,
        }
    }
}

/// Flight phases as required by spec §9.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum FlightPhase {
    #[default]
    Preflight,
    Boarding,
    Pushback,
    TaxiOut,
    TakeoffRoll,
    Takeoff,
    Climb,
    Cruise,
    Descent,
    Approach,
    Final,
    Landing,
    TaxiIn,
    BlocksOn,
    Arrived,
    PirepSubmitted,
}

/// What every simulator adapter must provide.
///
/// Phase 1: implemented by `sim-msfs` (SimConnect). Phase 2: `sim-xplane` (UDP from XPLM plugin).
pub trait SimAdapter: Send + 'static {
    /// Display name of the adapter, e.g. "MSFS 2024 (SimConnect)".
    fn name(&self) -> &str;

    /// Try to connect to the simulator. Returns `Ok(())` once a snapshot stream is established.
    fn connect(&mut self) -> Result<(), SimError>;

    /// Disconnect cleanly.
    fn disconnect(&mut self);

    /// Whether the adapter is currently connected and producing snapshots.
    fn is_connected(&self) -> bool;

    /// Pull the most recent snapshot, if any. Non-blocking.
    fn latest_snapshot(&self) -> Option<SimSnapshot>;
}

#[derive(Debug, thiserror::Error)]
pub enum SimError {
    #[error("simulator not running")]
    NotRunning,
    #[error("connection refused: {0}")]
    Refused(String),
    #[error("transport error: {0}")]
    Transport(String),
    #[error("not implemented yet")]
    NotImplemented,
}

// TODO(phase-1):
//   * Implement the SimAdapter trait in `sim-msfs`.
//   * Add a Phase FSM driven by SimSnapshot streams.
//   * Add a snapshot ring buffer for downstream consumers.
