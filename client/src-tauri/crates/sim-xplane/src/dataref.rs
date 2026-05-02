//! DataRef catalog — the SimVar-equivalent SimSnapshot fields.
//!
//! Every entry maps an X-Plane DataRef name to a slot in the parsed
//! `XPlaneState` (which then converts to `SimSnapshot`). The order
//! determines our subscription index; we send one RREF request per
//! row and store the (index → field) mapping.
//!
//! All DataRef names are confirmed to exist in BOTH X-Plane 11 and
//! 12, sourced from the official DataRef list at
//! <https://developer.x-plane.com/datarefs/>. When 11 and 12 diverged
//! on a given name, we use the canonical XP12 name and accept that
//! XP11 may report a default value (so the snapshot field stays at
//! its default rather than crashing).
//!
//! ## Units
//!
//! X-Plane DataRefs document their units explicitly. We pull values
//! in their native unit and convert to `SimSnapshot` units in
//! `XPlaneState::to_snapshot`. Notable conversions:
//!
//!   * `local_vx/vz`     m/s → ft/s for body velocity
//!   * `acf_m_*`         kg → kg (no-op, but the field is float)
//!   * `latitude/longitude` deg → deg (no-op, but x-plane uses
//!     `flightmodel/position/{latitude,longitude}` not `lat_/lon_`)

use sim_core::{SimSnapshot, Simulator};

/// Field on the parsed `XPlaneState` to assign each DataRef value to.
/// Each entry corresponds to one RREF subscription.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldId {
    Latitude,
    Longitude,
    AltitudeMslFt,
    AltitudeAglFt,
    HeadingDegTrue,
    HeadingDegMagnetic,
    PitchDeg,
    BankDeg,
    VerticalSpeedFpm,
    GroundspeedKt,
    IndicatedAirspeedKt,
    TrueAirspeedKt,
    GForce,
    OnGround,
    ParkingBrake,
    GearDeploy,
    FlapsHandle,
    Eng1Running,
    Eng2Running,
    Eng3Running,
    Eng4Running,
    FuelTotalKg,
    EmptyWeightKg,
    TotalWeightKg,
    /// Body-frame X velocity (m/s), positive right. Used for sideslip.
    LocalVxMs,
    /// Body-frame Z velocity (m/s), positive forward. Used for sideslip.
    LocalVzMs,
    /// Wind X relative to airframe (m/s). Phase 2.
    WindXMs,
    /// Wind Z relative to airframe (m/s). Phase 2.
    WindZMs,
}

/// One row in the catalog: a DataRef name + which snapshot field it
/// fills. The index used over the wire equals the position in
/// `CATALOG`.
pub struct DatarefEntry {
    pub name: &'static str,
    pub field: FieldId,
}

/// The subscription catalog. Order is the wire index.
pub const CATALOG: &[DatarefEntry] = &[
    // --- Position ---
    DatarefEntry {
        name: "sim/flightmodel/position/latitude",
        field: FieldId::Latitude,
    },
    DatarefEntry {
        name: "sim/flightmodel/position/longitude",
        field: FieldId::Longitude,
    },
    DatarefEntry {
        name: "sim/flightmodel/position/elevation",
        field: FieldId::AltitudeMslFt,
    },
    DatarefEntry {
        // X-Plane reports AGL in METERS via `y_agl`. We convert at
        // the snapshot boundary.
        name: "sim/flightmodel/position/y_agl",
        field: FieldId::AltitudeAglFt,
    },
    // --- Attitude / motion ---
    DatarefEntry {
        name: "sim/flightmodel/position/psi",
        field: FieldId::HeadingDegTrue,
    },
    DatarefEntry {
        name: "sim/flightmodel/position/magpsi",
        field: FieldId::HeadingDegMagnetic,
    },
    DatarefEntry {
        name: "sim/flightmodel/position/theta",
        field: FieldId::PitchDeg,
    },
    DatarefEntry {
        name: "sim/flightmodel/position/phi",
        field: FieldId::BankDeg,
    },
    DatarefEntry {
        name: "sim/flightmodel/position/vh_ind_fpm",
        field: FieldId::VerticalSpeedFpm,
    },
    // --- Speeds ---
    DatarefEntry {
        // groundspeed is in m/s native; we convert to knots.
        name: "sim/flightmodel/position/groundspeed",
        field: FieldId::GroundspeedKt,
    },
    DatarefEntry {
        name: "sim/flightmodel/position/indicated_airspeed",
        field: FieldId::IndicatedAirspeedKt,
    },
    DatarefEntry {
        name: "sim/flightmodel/position/true_airspeed",
        field: FieldId::TrueAirspeedKt,
    },
    // --- Forces ---
    DatarefEntry {
        name: "sim/flightmodel2/misc/gforce_normal",
        field: FieldId::GForce,
    },
    DatarefEntry {
        // bool 0/1
        name: "sim/flightmodel/failures/onground_any",
        field: FieldId::OnGround,
    },
    DatarefEntry {
        // 0..1 ratio
        name: "sim/cockpit2/controls/parking_brake_ratio",
        field: FieldId::ParkingBrake,
    },
    // --- Gear / flaps (just gear[0] for "is the nose gear deployed";
    //     gives a 0..1 in DEPLOY_RATIO array; index 0 is the first
    //     gear leg). Phase 1 takes a single value; Phase 2 will
    //     subscribe array indices for per-leg readings. ---
    DatarefEntry {
        name: "sim/flightmodel2/gear/deploy_ratio",
        field: FieldId::GearDeploy,
    },
    DatarefEntry {
        name: "sim/flightmodel2/controls/flap_handle_deploy_ratio",
        field: FieldId::FlapsHandle,
    },
    // --- Engines (running flag per engine, array index syntax
    //     unsupported in raw RREF — we'd subscribe `[0]`, `[1]` etc
    //     but X-Plane doesn't index into arrays via name string; we
    //     therefore subscribe each engine via the per-index DataRef
    //     name `sim/flightmodel/engine/ENGN_running`, which when
    //     subscribed without bracket returns the FIRST element only.
    //     For Phase 1 we accept "first engine running" as a proxy
    //     for "any engine running" and will fix this in Phase 2 by
    //     using the indexable DataRef variant). ---
    DatarefEntry {
        name: "sim/flightmodel/engine/ENGN_running",
        field: FieldId::Eng1Running,
    },
    // --- Weight & fuel (kg native) ---
    DatarefEntry {
        name: "sim/aircraft/weight/acf_m_fuel_total",
        field: FieldId::FuelTotalKg,
    },
    DatarefEntry {
        name: "sim/aircraft/weight/acf_m_empty",
        field: FieldId::EmptyWeightKg,
    },
    DatarefEntry {
        name: "sim/flightmodel/weight/m_total",
        field: FieldId::TotalWeightKg,
    },
    // --- Body velocity (m/s) — for native sideslip in Phase 2 ---
    DatarefEntry {
        name: "sim/flightmodel/forces/local_vx",
        field: FieldId::LocalVxMs,
    },
    DatarefEntry {
        name: "sim/flightmodel/forces/local_vz",
        field: FieldId::LocalVzMs,
    },
];

/// Mutable parsed state — populated as RREF responses arrive. Held
/// behind a Mutex by the adapter; on every snapshot request we copy
/// it out and convert to `SimSnapshot`.
#[derive(Debug, Clone, Default)]
pub struct XPlaneState {
    pub lat: f64,
    pub lon: f64,
    pub altitude_msl_ft: f64,
    /// Stored in METERS (X-Plane native). Convert at snapshot time.
    pub altitude_agl_m: f64,
    pub heading_true_deg: f32,
    pub heading_magnetic_deg: f32,
    pub pitch_deg: f32,
    pub bank_deg: f32,
    pub vertical_speed_fpm: f32,
    /// Stored in M/S (X-Plane native). Convert at snapshot time.
    pub groundspeed_ms: f32,
    pub indicated_airspeed_kt: f32,
    pub true_airspeed_kt: f32,
    pub g_force: f32,
    pub on_ground: bool,
    pub parking_brake_ratio: f32,
    pub gear_deploy: f32,
    pub flaps_handle: f32,
    pub eng1_running: bool,
    pub fuel_total_kg: f32,
    pub empty_weight_kg: f32,
    pub total_weight_kg: f32,
    pub local_vx_ms: f32,
    pub local_vz_ms: f32,
    pub wind_x_ms: f32,
    pub wind_z_ms: f32,
    /// True once we've received at least one RREF packet — drives
    /// the connection state machine's transition into `Connected`.
    pub got_first_packet: bool,
}

impl XPlaneState {
    /// Apply one (index, value) pair from an RREF response.
    pub fn apply(&mut self, index: i32, value: f32) {
        let Some(entry) = CATALOG.get(index as usize) else {
            tracing::trace!(index, value, "RREF index out of range");
            return;
        };
        self.got_first_packet = true;
        match entry.field {
            FieldId::Latitude => self.lat = value as f64,
            FieldId::Longitude => self.lon = value as f64,
            FieldId::AltitudeMslFt => self.altitude_msl_ft = value as f64,
            FieldId::AltitudeAglFt => self.altitude_agl_m = value as f64, // misnamed: stored in m
            FieldId::HeadingDegTrue => self.heading_true_deg = value,
            FieldId::HeadingDegMagnetic => self.heading_magnetic_deg = value,
            FieldId::PitchDeg => self.pitch_deg = value,
            FieldId::BankDeg => self.bank_deg = value,
            FieldId::VerticalSpeedFpm => self.vertical_speed_fpm = value,
            FieldId::GroundspeedKt => self.groundspeed_ms = value, // m/s native
            FieldId::IndicatedAirspeedKt => self.indicated_airspeed_kt = value,
            FieldId::TrueAirspeedKt => self.true_airspeed_kt = value,
            FieldId::GForce => self.g_force = value,
            FieldId::OnGround => self.on_ground = value > 0.5,
            FieldId::ParkingBrake => self.parking_brake_ratio = value,
            FieldId::GearDeploy => self.gear_deploy = value,
            FieldId::FlapsHandle => self.flaps_handle = value,
            FieldId::Eng1Running => self.eng1_running = value > 0.5,
            FieldId::Eng2Running => {} // Phase 2
            FieldId::Eng3Running => {} // Phase 2
            FieldId::Eng4Running => {} // Phase 2
            FieldId::FuelTotalKg => self.fuel_total_kg = value,
            FieldId::EmptyWeightKg => self.empty_weight_kg = value,
            FieldId::TotalWeightKg => self.total_weight_kg = value,
            FieldId::LocalVxMs => self.local_vx_ms = value,
            FieldId::LocalVzMs => self.local_vz_ms = value,
            FieldId::WindXMs => self.wind_x_ms = value,
            FieldId::WindZMs => self.wind_z_ms = value,
        }
    }

    /// Convert the accumulated state to a fresh `SimSnapshot`. The
    /// timestamp is stamped at conversion time (UTC now). Fields
    /// without an X-Plane equivalent stay at SimSnapshot's `Default`
    /// (None for Options, sensible zeros for required fields).
    pub fn to_snapshot(&self, simulator: Simulator) -> SimSnapshot {
        const M_PER_FT: f64 = 0.3048;
        const KT_PER_MS: f32 = 1.9438445; // 1 m/s = 1.9438 knots

        // Derive payload from ZFW-OEW like we do on the MSFS side.
        // ZFW = total weight - fuel.
        let zfw_kg = if self.total_weight_kg > 0.0 && self.total_weight_kg > self.fuel_total_kg {
            Some(self.total_weight_kg - self.fuel_total_kg)
        } else {
            None
        };
        let oew = if self.empty_weight_kg > 0.0 {
            Some(self.empty_weight_kg)
        } else {
            None
        };
        let payload_kg = match (zfw_kg, oew) {
            (Some(z), Some(o)) if z > o => Some(z - o),
            _ => None,
        };

        SimSnapshot {
            timestamp: chrono::Utc::now(),
            lat: self.lat,
            lon: self.lon,
            altitude_msl_ft: self.altitude_msl_ft,
            altitude_agl_ft: self.altitude_agl_m / M_PER_FT,
            heading_deg_true: self.heading_true_deg,
            heading_deg_magnetic: self.heading_magnetic_deg,
            pitch_deg: self.pitch_deg,
            bank_deg: self.bank_deg,
            vertical_speed_fpm: self.vertical_speed_fpm,
            velocity_body_x_fps: Some((self.local_vx_ms / 0.3048) as f32),
            velocity_body_z_fps: Some((self.local_vz_ms / 0.3048) as f32),
            groundspeed_kt: self.groundspeed_ms * KT_PER_MS,
            indicated_airspeed_kt: self.indicated_airspeed_kt,
            true_airspeed_kt: self.true_airspeed_kt,
            aircraft_wind_x_kt: Some(self.wind_x_ms * KT_PER_MS),
            aircraft_wind_z_kt: Some(self.wind_z_ms * KT_PER_MS),
            g_force: self.g_force,
            on_ground: self.on_ground,
            parking_brake: self.parking_brake_ratio > 0.5,
            stall_warning: false, // not subscribed yet (Phase 2)
            overspeed_warning: false,
            paused: false,
            slew_mode: false,
            simulation_rate: 1.0,
            gear_position: self.gear_deploy,
            flaps_position: self.flaps_handle,
            engines_running: if self.eng1_running { 1 } else { 0 },
            fuel_total_kg: self.fuel_total_kg,
            fuel_used_kg: 0.0,
            zfw_kg,
            payload_kg,
            total_weight_kg: if self.total_weight_kg > 0.0 {
                Some(self.total_weight_kg)
            } else {
                None
            },
            // No latched-touchdown DataRef in X-Plane; the buffer-
            // based fallback in src/lib.rs takes over.
            touchdown_vs_fpm: None,
            touchdown_pitch_deg: None,
            touchdown_bank_deg: None,
            touchdown_heading_mag_deg: None,
            touchdown_lat: None,
            touchdown_lon: None,
            wind_direction_deg: None,
            wind_speed_kt: None,
            qnh_hpa: None,
            outside_air_temp_c: None,
            total_air_temp_c: None,
            mach: None,
            empty_weight_kg: oew,
            aircraft_title: None,
            aircraft_icao: None,
            aircraft_registration: None,
            simulator,
            sim_version: None,
            // Avionics / lights / AP / systems — all Phase 2.
            transponder_code: None,
            com1_mhz: None,
            com2_mhz: None,
            nav1_mhz: None,
            nav2_mhz: None,
            light_landing: None,
            light_beacon: None,
            light_strobe: None,
            light_taxi: None,
            light_nav: None,
            light_logo: None,
            strobe_state: None,
            autopilot_master: None,
            autopilot_heading: None,
            autopilot_altitude: None,
            autopilot_nav: None,
            autopilot_approach: None,
            fuel_flow_kg_per_h: None,
            spoilers_handle_position: None,
            spoilers_armed: None,
            pushback_state: None,
            apu_switch: None,
            apu_pct_rpm: None,
            battery_master: None,
            avionics_master: None,
            pitot_heat: None,
            engine_anti_ice: None,
            wing_anti_ice: None,
            seatbelts_sign: None,
            no_smoking_sign: None,
            fcu_selected_altitude_ft: None,
            fcu_selected_heading_deg: None,
            fcu_selected_speed_kt: None,
            fcu_selected_vs_fpm: None,
            autobrake: None,
            parking_name: None,
            parking_number: None,
            selected_runway: None,
            aircraft_profile: sim_core::AircraftProfile::default(),
        }
    }
}
