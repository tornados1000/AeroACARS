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
    /// Wind X relative to airframe (m/s).
    WindXMs,
    /// Wind Z relative to airframe (m/s).
    WindZMs,
    // Phase 2 additions:
    LightLanding,
    LightBeacon,
    LightStrobe,
    LightTaxi,
    LightNav,
    ApMaster,
    ApHeading,
    ApAltitude,
    ApNav,
    ApApproach,
    SpoilersHandle,
    SpoilersArmed,
    StallWarning,
    BatteryMaster,
    AvionicsMaster,
    ApuSwitch,
    PitotHeat,
    QnhInHg,
    OatC,
    Mach,
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
        // X-Plane reports MSL altitude in METERS via `elevation`.
        // The misleading `_ft` in the FieldId is historic — we
        // convert to feet at the snapshot boundary, alongside AGL.
        // Live-bug 2026-05-03: pilot saw "HÖHE 5.554 ft / AGL
        // 18.113 ft" at FL180 because we treated meters as feet
        // (5554 m × 3.28084 = 18221 ft, ≈ AGL).
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
    // --- Engines: explicit array index per engine. The unbracketed
    //     `ENGN_running` was unreliable (returned 0 even when engine 1
    //     was running on a 2-engine heavy — verified via live test).
    //     Bracket-suffix syntax IS supported by RREF: X-Plane parses
    //     `[N]` and returns just that array slot.
    DatarefEntry {
        name: "sim/flightmodel/engine/ENGN_running[0]",
        field: FieldId::Eng1Running,
    },
    // --- Weight & fuel (kg native).
    //     `acf_m_fuel_total` is the MAX TANK CAPACITY, not the
    //     current onboard fuel weight (verified via live test: full
    //     tank reported as 0 kg). Use the live `flightmodel/weight`
    //     DataRef instead. ---
    DatarefEntry {
        name: "sim/flightmodel/weight/m_fuel_total",
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
    // --- Body velocity (m/s) — for native sideslip ---
    DatarefEntry {
        name: "sim/flightmodel/forces/local_vx",
        field: FieldId::LocalVxMs,
    },
    DatarefEntry {
        name: "sim/flightmodel/forces/local_vz",
        field: FieldId::LocalVzMs,
    },
    // ---- Phase 2 DataRefs ----
    // Multi-engine: array indices via `[N]` syntax (X-Plane parses
    // the bracket and returns just that element). Engine #1 is [0].
    DatarefEntry {
        name: "sim/flightmodel/engine/ENGN_running[1]",
        field: FieldId::Eng2Running,
    },
    DatarefEntry {
        name: "sim/flightmodel/engine/ENGN_running[2]",
        field: FieldId::Eng3Running,
    },
    DatarefEntry {
        name: "sim/flightmodel/engine/ENGN_running[3]",
        field: FieldId::Eng4Running,
    },
    // Lights — bool 0/1.
    DatarefEntry {
        name: "sim/cockpit2/switches/landing_lights_on",
        field: FieldId::LightLanding,
    },
    DatarefEntry {
        name: "sim/cockpit2/switches/beacon_on",
        field: FieldId::LightBeacon,
    },
    DatarefEntry {
        name: "sim/cockpit2/switches/strobe_lights_on",
        field: FieldId::LightStrobe,
    },
    DatarefEntry {
        name: "sim/cockpit2/switches/taxi_light_on",
        field: FieldId::LightTaxi,
    },
    DatarefEntry {
        name: "sim/cockpit2/switches/navigation_lights_on",
        field: FieldId::LightNav,
    },
    // Logo light: X-Plane uses the same nav-light dataref by
    // convention; some payware breaks this out separately. We
    // alias to nav for now (Phase 3 if a payware author asks).
    // Autopilot — XP exposes per-mode "engaged" status as int 0..2
    // (off / armed / engaged). We treat >0 as "on".
    DatarefEntry {
        name: "sim/cockpit2/autopilot/servos_on",
        field: FieldId::ApMaster,
    },
    DatarefEntry {
        name: "sim/cockpit2/autopilot/heading_status",
        field: FieldId::ApHeading,
    },
    DatarefEntry {
        name: "sim/cockpit2/autopilot/altitude_hold_status",
        field: FieldId::ApAltitude,
    },
    DatarefEntry {
        name: "sim/cockpit2/autopilot/nav_status",
        field: FieldId::ApNav,
    },
    DatarefEntry {
        name: "sim/cockpit2/autopilot/approach_status",
        field: FieldId::ApApproach,
    },
    // Surfaces — speedbrake is a 0..1 ratio.
    DatarefEntry {
        name: "sim/cockpit2/controls/speedbrake_ratio",
        field: FieldId::SpoilersHandle,
    },
    DatarefEntry {
        name: "sim/cockpit2/annunciators/speedbrake",
        field: FieldId::SpoilersArmed,
    },
    // Wind components in airframe-relative coords (m/s). Used for
    // headwind/crosswind reporting in the PIREP. Same DataRefs in
    // X-Plane 11 and 12.
    DatarefEntry {
        name: "sim/weather/aircraft/wind_now_x_msc",
        field: FieldId::WindXMs,
    },
    DatarefEntry {
        name: "sim/weather/aircraft/wind_now_z_msc",
        field: FieldId::WindZMs,
    },
    // Stall warning — annunciator (bool).
    DatarefEntry {
        name: "sim/cockpit2/annunciators/stall_warning",
        field: FieldId::StallWarning,
    },
    // Systems — battery / avionics / APU / pitot heat.
    DatarefEntry {
        name: "sim/cockpit2/electrical/battery_on[0]",
        field: FieldId::BatteryMaster,
    },
    DatarefEntry {
        name: "sim/cockpit2/electrical/avionics_on",
        field: FieldId::AvionicsMaster,
    },
    DatarefEntry {
        name: "sim/cockpit2/electrical/APU_running",
        field: FieldId::ApuSwitch,
    },
    DatarefEntry {
        name: "sim/cockpit2/ice/ice_pitot_heat_on_pilot",
        field: FieldId::PitotHeat,
    },
    // QNH (hPa) and ambient temp — both rewired 2026-05-03 after
    // a live pilot bug report:
    //
    // Old QNH DataRef `sim/weather/region/altimeter_temperature_effect`
    // is NOT pressure — it's a unitless temperature-correction
    // factor for the cold-weather altimeter. We were storing that
    // factor in `qnh_inhg` and multiplying by 33.86 → producing
    // garbage (typical reading: ~1.0 → "33 hPa", way off).
    //
    // Old OAT DataRef `sim/weather/region/temperatures_aloft_deg_c[0]`
    // is the SURFACE temperature (index 0 of the aloft array), not
    // the temperature at aircraft altitude. Pilot at FL180 saw
    // "+22°C" while the cockpit PFD correctly showed SAT −18°C.
    //
    // The cockpit2/temperature DataRef is the actual aircraft-level
    // ambient (= SAT in modern X-Plane). The barometer_current_inhg
    // DataRef is the standard altimeter reading the pilot would
    // dial into the Kollsman window — same value across X-Plane 11
    // and 12, no version branching needed.
    DatarefEntry {
        name: "sim/weather/barometer_current_inhg",
        field: FieldId::QnhInHg,
    },
    DatarefEntry {
        name: "sim/cockpit2/temperature/outside_air_temp_degc",
        field: FieldId::OatC,
    },
    // Mach number.
    DatarefEntry {
        name: "sim/flightmodel/misc/machno",
        field: FieldId::Mach,
    },
];

/// Mutable parsed state — populated as RREF responses arrive. Held
/// behind a Mutex by the adapter; on every snapshot request we copy
/// it out and convert to `SimSnapshot`.
#[derive(Debug, Clone, Default)]
pub struct XPlaneState {
    pub lat: f64,
    pub lon: f64,
    /// Stored in METERS (X-Plane native — `sim/flightmodel/position/elevation`
    /// reports meters MSL, not feet, despite the historic field name).
    /// Convert to feet at snapshot time alongside AGL.
    pub altitude_msl_m: f64,
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
    // Phase 2: multi-engine, lights, AP, surfaces, systems, environment.
    pub eng2_running: bool,
    pub eng3_running: bool,
    pub eng4_running: bool,
    pub light_landing: bool,
    pub light_beacon: bool,
    pub light_strobe: bool,
    pub light_taxi: bool,
    pub light_nav: bool,
    pub ap_master: bool,
    pub ap_heading: bool,
    pub ap_altitude: bool,
    pub ap_nav: bool,
    pub ap_approach: bool,
    pub spoilers_handle: f32,
    pub spoilers_armed: bool,
    pub stall_warning: bool,
    pub battery_master: bool,
    pub avionics_master: bool,
    pub apu_switch: bool,
    pub pitot_heat: bool,
    pub qnh_inhg: f32,
    pub oat_c: f32,
    pub mach: f32,
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
            FieldId::AltitudeMslFt => self.altitude_msl_m = value as f64, // misnamed: stored in m
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
            FieldId::Eng2Running => self.eng2_running = value > 0.5,
            FieldId::Eng3Running => self.eng3_running = value > 0.5,
            FieldId::Eng4Running => self.eng4_running = value > 0.5,
            FieldId::FuelTotalKg => self.fuel_total_kg = value,
            FieldId::EmptyWeightKg => self.empty_weight_kg = value,
            FieldId::TotalWeightKg => self.total_weight_kg = value,
            FieldId::LocalVxMs => self.local_vx_ms = value,
            FieldId::LocalVzMs => self.local_vz_ms = value,
            FieldId::WindXMs => self.wind_x_ms = value,
            FieldId::WindZMs => self.wind_z_ms = value,
            FieldId::LightLanding => self.light_landing = value > 0.5,
            FieldId::LightBeacon => self.light_beacon = value > 0.5,
            FieldId::LightStrobe => self.light_strobe = value > 0.5,
            FieldId::LightTaxi => self.light_taxi = value > 0.5,
            FieldId::LightNav => self.light_nav = value > 0.5,
            FieldId::ApMaster => self.ap_master = value > 0.5,
            FieldId::ApHeading => self.ap_heading = value > 0.5,
            FieldId::ApAltitude => self.ap_altitude = value > 0.5,
            FieldId::ApNav => self.ap_nav = value > 0.5,
            FieldId::ApApproach => self.ap_approach = value > 0.5,
            FieldId::SpoilersHandle => self.spoilers_handle = value,
            FieldId::SpoilersArmed => self.spoilers_armed = value > 0.5,
            FieldId::StallWarning => self.stall_warning = value > 0.5,
            FieldId::BatteryMaster => self.battery_master = value > 0.5,
            FieldId::AvionicsMaster => self.avionics_master = value > 0.5,
            FieldId::ApuSwitch => self.apu_switch = value > 0.5,
            FieldId::PitotHeat => self.pitot_heat = value > 0.5,
            FieldId::QnhInHg => self.qnh_inhg = value,
            FieldId::OatC => self.oat_c = value,
            FieldId::Mach => self.mach = value,
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
            altitude_msl_ft: self.altitude_msl_m / M_PER_FT,
            altitude_agl_ft: self.altitude_agl_m / M_PER_FT,
            heading_deg_true: self.heading_true_deg,
            heading_deg_magnetic: self.heading_magnetic_deg,
            pitch_deg: self.pitch_deg,
            bank_deg: self.bank_deg,
            vertical_speed_fpm: self.vertical_speed_fpm,
            velocity_body_x_fps: Some((self.local_vx_ms / 0.3048) as f32),
            velocity_body_z_fps: Some((self.local_vz_ms / 0.3048) as f32),
            groundspeed_kt: self.groundspeed_ms * KT_PER_MS,
            // X-Plane's pitot simulation produces small negative IAS/TAS
            // readings when the aircraft is stationary on the ground
            // (sensor noise, residual ram pressure modelling). Clamp to
            // zero at the source so neither the cockpit gauges nor
            // downstream consumers (PIREP, activity log) ever surface
            // a "−10 kt" — pilots reasonably treat that as a bug.
            indicated_airspeed_kt: self.indicated_airspeed_kt.max(0.0),
            true_airspeed_kt: self.true_airspeed_kt.max(0.0),
            aircraft_wind_x_kt: Some(self.wind_x_ms * KT_PER_MS),
            aircraft_wind_z_kt: Some(self.wind_z_ms * KT_PER_MS),
            g_force: self.g_force,
            on_ground: self.on_ground,
            parking_brake: self.parking_brake_ratio > 0.5,
            stall_warning: self.stall_warning,
            overspeed_warning: false, // X-Plane has no direct overspeed annunciator
            paused: false,
            slew_mode: false,
            simulation_rate: 1.0,
            gear_position: self.gear_deploy,
            flaps_position: self.flaps_handle,
            engines_running: [
                self.eng1_running,
                self.eng2_running,
                self.eng3_running,
                self.eng4_running,
            ]
            .iter()
            .filter(|&&r| r)
            .count() as u8,
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
            qnh_hpa: if self.qnh_inhg > 0.0 {
                // X-Plane reports altimeter setting in inHg natively;
                // convert to hPa (1 inHg = 33.8639 hPa).
                Some(self.qnh_inhg * 33.8639)
            } else {
                None
            },
            outside_air_temp_c: Some(self.oat_c),
            total_air_temp_c: None,
            mach: if self.mach > 0.0 { Some(self.mach) } else { None },
            empty_weight_kg: oew,
            aircraft_title: None,
            aircraft_icao: None,
            aircraft_registration: None,
            simulator,
            sim_version: None,
            // Avionics — X-Plane exposes COM/NAV via separate
            // DataRefs but addons disagree on conventions; keep
            // None for Phase 2, revisit if a payware author asks.
            transponder_code: None,
            com1_mhz: None,
            com2_mhz: None,
            nav1_mhz: None,
            nav2_mhz: None,
            light_landing: Some(self.light_landing),
            light_beacon: Some(self.light_beacon),
            light_strobe: Some(self.light_strobe),
            light_taxi: Some(self.light_taxi),
            light_nav: Some(self.light_nav),
            // X-Plane's nav-light DataRef covers logo on most payware.
            light_logo: Some(self.light_nav),
            strobe_state: None,
            autopilot_master: Some(self.ap_master),
            autopilot_heading: Some(self.ap_heading),
            autopilot_altitude: Some(self.ap_altitude),
            autopilot_nav: Some(self.ap_nav),
            autopilot_approach: Some(self.ap_approach),
            fuel_flow_kg_per_h: None,
            spoilers_handle_position: Some(self.spoilers_handle),
            spoilers_armed: Some(self.spoilers_armed),
            // Pushback isn't a sim-managed thing in X-Plane.
            pushback_state: None,
            apu_switch: Some(self.apu_switch),
            apu_pct_rpm: None,
            battery_master: Some(self.battery_master),
            avionics_master: Some(self.avionics_master),
            pitot_heat: Some(self.pitot_heat),
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
            // PMDG SDK is MSFS-only; X-Plane never fills this.
            pmdg: None,
        }
    }
}
