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
