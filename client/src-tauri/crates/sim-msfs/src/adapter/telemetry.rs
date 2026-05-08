//! Static SimVar list + byte-level parser for the data block
//! SimConnect sends back per `SIMCONNECT_RECV_SIMOBJECT_DATA`.
//!
//! Each entry in [`TELEMETRY_FIELDS`] is added in order to the data
//! definition; the parser reads the same order at fixed offsets. The
//! whole point of this module is that **a single rejected SimVar can
//! never shift another field's position** — every field knows its
//! width and we walk the buffer step by step. If a SimVar is rejected
//! by SimConnect, the data block is shorter than expected and `parse`
//! either returns the value or [`f64::NAN`] / `0` for the missing
//! tail; nothing prior shifts.

use chrono::Utc;
use sim_core::{AircraftProfile, SimSnapshot, Simulator};

const KG_PER_LB: f64 = 0.453_592_37;

#[derive(Debug, Clone, Copy)]
pub enum FieldKind {
    /// 8-byte IEEE 754.
    Float64,
    /// 4-byte signed integer (SimConnect bool is INT32).
    Int32,
    /// 256-byte fixed buffer, NUL-terminated.
    String256,
}

impl FieldKind {
    pub fn size(self) -> usize {
        match self {
            FieldKind::Float64 => 8,
            FieldKind::Int32 => 4,
            FieldKind::String256 => 256,
        }
    }
}

/// Static description of one telemetry field.
#[derive(Debug, Clone, Copy)]
pub struct TelemetryField {
    pub name: &'static str,
    pub unit: &'static str,
    pub kind: FieldKind,
}

/// Order matters: this is exactly the order in which SimConnect will
/// pack the bytes for us.
pub const TELEMETRY_FIELDS: &[TelemetryField] = &[
    // ---- Identity ----
    F::str("TITLE", ""),
    F::str("ATC MODEL", ""),
    F::str("ATC ID", ""),
    // ---- Position ----
    F::f64("PLANE LATITUDE", "degrees"),
    F::f64("PLANE LONGITUDE", "degrees"),
    F::f64("PLANE ALTITUDE", "feet"),
    F::f64("PLANE ALT ABOVE GROUND", "feet"),
    // ---- Attitude / motion ----
    F::f64("PLANE HEADING DEGREES TRUE", "degrees"),
    F::f64("PLANE HEADING DEGREES MAGNETIC", "degrees"),
    F::f64("PLANE PITCH DEGREES", "degrees"),
    F::f64("PLANE BANK DEGREES", "degrees"),
    F::f64("VERTICAL SPEED", "feet per minute"),
    // Body-frame velocity. Used at touchdown to derive sideslip /
    // crab natively (atan2(VEL_BODY_X, VEL_BODY_Z) × 180/π) which is
    // what GEES does. Way more accurate than computing track from
    // successive lat/lon.
    F::f64("VELOCITY BODY X", "feet per second"),
    F::f64("VELOCITY BODY Z", "feet per second"),
    // ---- Speeds ----
    F::f64("GROUND VELOCITY", "knots"),
    F::f64("AIRSPEED INDICATED", "knots"),
    F::f64("AIRSPEED TRUE", "knots"),
    F::f64("G FORCE", "GForce"),
    // Body-frame wind components. Positive AIRCRAFT WIND X = wind
    // from the aircraft's right (= crosswind from the right side).
    // Positive AIRCRAFT WIND Z = tailwind. Sign convention per MSFS
    // SDK; we surface absolute headwind/crosswind in the PIREP.
    F::f64("AIRCRAFT WIND X", "knots"),
    F::f64("AIRCRAFT WIND Z", "knots"),
    // ---- Aircraft state ----
    F::bool("SIM ON GROUND"),
    F::bool("BRAKE PARKING POSITION"),
    F::bool("STALL WARNING"),
    F::bool("OVERSPEED WARNING"),
    F::f64("GEAR POSITION", "percent over 100"),
    F::f64("FLAPS HANDLE PERCENT", "percent over 100"),
    F::bool("GENERAL ENG COMBUSTION:1"),
    F::bool("GENERAL ENG COMBUSTION:2"),
    F::bool("GENERAL ENG COMBUSTION:3"),
    F::bool("GENERAL ENG COMBUSTION:4"),
    // ---- Fuel & weight (SU2 EX1 + legacy fallback) ----
    F::f64("FUEL TOTAL QUANTITY WEIGHT EX1", "pounds"),
    F::f64("FUEL TOTAL QUANTITY WEIGHT", "pounds"),
    F::f64("TOTAL WEIGHT", "pounds"),
    F::f64("EMPTY WEIGHT", "pounds"),
    // ---- Environment ----
    F::f64("AMBIENT WIND DIRECTION", "degrees"),
    F::f64("AMBIENT WIND VELOCITY", "knots"),
    F::f64("KOHLSMAN SETTING MB", "millibars"),
    F::f64("AMBIENT TEMPERATURE", "celsius"),
    // Total Air Temperature — what an aircraft thermometer measures
    // (TAT > OAT in flight due to compression heating).
    F::f64("TOTAL AIR TEMPERATURE", "celsius"),
    // Mach number — current aircraft Mach. 0..1 transonic, >1 supersonic.
    F::f64("AIRSPEED MACH", "mach"),
    // ---- Avionics (Phase 5 / SU2-safe standard SimVars) ----
    // All wired by Asobo's simulation core regardless of aircraft;
    // Fenix is the documented exception — it bypasses the standard
    // COM/NAV SimVars and uses internal LVars. We surface the raw
    // values here and the snapshot mapping suppresses them for
    // Fenix to avoid the "1024 MHz" QNH-bleed garbage we saw with
    // the old crate.
    F::f64("TRANSPONDER CODE:1", "BCO16"),
    F::f64("COM ACTIVE FREQUENCY:1", "MHz"),
    F::f64("COM ACTIVE FREQUENCY:2", "MHz"),
    F::f64("NAV ACTIVE FREQUENCY:1", "MHz"),
    F::f64("NAV ACTIVE FREQUENCY:2", "MHz"),
    // ---- Exterior lights ----
    F::bool("LIGHT LANDING"),
    F::bool("LIGHT BEACON"),
    F::bool("LIGHT STROBE"),
    F::bool("LIGHT TAXI"),
    F::bool("LIGHT NAV"),
    F::bool("LIGHT LOGO"),
    // ---- Autopilot ----
    F::bool("AUTOPILOT MASTER"),
    F::bool("AUTOPILOT HEADING LOCK"),
    F::bool("AUTOPILOT ALTITUDE LOCK"),
    F::bool("AUTOPILOT NAV1 LOCK"),
    F::bool("AUTOPILOT APPROACH HOLD"),
    // ---- Powerplant (per-engine fuel flow, summed in mapping) ----
    F::f64("ENG FUEL FLOW PPH:1", "pounds per hour"),
    F::f64("ENG FUEL FLOW PPH:2", "pounds per hour"),
    F::f64("ENG FUEL FLOW PPH:3", "pounds per hour"),
    F::f64("ENG FUEL FLOW PPH:4", "pounds per hour"),

    // ---- Surfaces ----
    // 0..1, position of the spoiler / speed-brake handle.
    F::f64("SPOILERS HANDLE POSITION", "percent over 100"),
    // Auto-spoilers armed for landing (separate from physical handle).
    F::bool("SPOILERS ARMED"),

    // ---- Pushback ----
    // Enum: 0 = Straight, 1 = Left, 2 = Right, 3 = No Pushback.
    // MSFS itself drives this — we use it as the authoritative
    // "pushback finished" signal in the FSM, since the simple
    // "moving + engines on = TaxiOut" trigger fires while the tug
    // is still pushing the aircraft. Value 3 means the tug has
    // disconnected (or the pilot used Ctrl+P to stop), which is
    // when we should advance to TaxiOut.
    F::f64("PUSHBACK STATE", "Enum"),

    // ---- Systems ----
    // APU master switch (0 = off, 1 = on).
    F::bool("APU SWITCH"),
    // APU N (RPM) percentage 0..100. Useful to distinguish "starting"
    // from "running" — the switch is on long before the APU is up.
    F::f64("APU PCT RPM", "percent"),
    // Battery #1 master. Most aircraft only have one battery exposed.
    F::bool("ELECTRICAL MASTER BATTERY:1"),
    F::bool("AVIONICS MASTER SWITCH"),
    F::bool("PITOT HEAT"),
    // Engine anti-ice — sampled per engine, combined to "any-on" in
    // the snapshot mapping so the UI just shows one indicator.
    F::bool("ENG ANTI ICE:1"),
    F::bool("ENG ANTI ICE:2"),
    F::bool("ENG ANTI ICE:3"),
    F::bool("ENG ANTI ICE:4"),
    // Wing / structural deice (Airbus calls this WING ANTI ICE).
    F::bool("STRUCTURAL DEICE SWITCH"),

    // ---- FBW A32NX LVars ----
    // LVars don't get rejected by SimConnect — non-FBW aircraft just
    // read 0 from them, so adding them universally is safe. The
    // snapshot mapping only consults these when AircraftProfile
    // detects FBW. Reference:
    // https://github.com/flybywiresim/aircraft/blob/master/fbw-a32nx/docs/a320-simvars.md
    F::f64("L:A32NX_TRANSPONDER_CODE", "Number"),
    F::f64("L:A32NX_AUTOPILOT_ACTIVE", "Bool"),
    F::f64("L:A32NX_AUTOPILOT_HEADING_HOLD_MODE", "Bool"),
    F::f64("L:A32NX_AUTOPILOT_ALTITUDE_HOLD_MODE", "Bool"),
    F::f64("L:A32NX_AUTOPILOT_LOC_MODE_ACTIVE", "Bool"),
    F::f64("L:A32NX_AUTOPILOT_APPR_MODE_ACTIVE", "Bool"),
    // FBW total fuel quantity, kg — the documented "live" total.
    F::f64("L:A32NX_TOTAL_FUEL_QUANTITY", "Number"),

    // ---- Fenix A320 LVars ----
    // Names verified against the Axis-and-Ohs Fenix script bundle
    // shipped at docs/vendor/FENIX_A3XX_AxisAndOhs_Scripts.xml — each
    // LVar below appears in that file as either a read or a write
    // target, so the names are stable for Fenix Block 2.
    //
    // Naming convention (from Fenix's `Cockpit_Behavior.xml`):
    //   * `L:S_OH_*` — overhead switch *state* (instantaneous position)
    //   * `L:S_FCU_*` — FCU button *state* (push state)
    //   * `L:E_FCU_*` — FCU encoder *display value* (selected ALT/HDG/…)
    //   * `L:I_MIP_*` — MIP indicator *lamp* (Autobrake LO/MED/MAX)
    //   * `L:S_MIP_*` — MIP switch *state*
    //
    // LVars never get rejected by SimConnect; a non-Fenix aircraft
    // just reads 0 from them, so the byte-level parser stays
    // healthy. The snapshot mapping consults each LVar only when
    // AircraftProfile::FenixA320 is detected.

    // Lights overhead (already wired before this batch).
    // Beacon switch: 0 = OFF, 1 = ON.
    F::f64("L:S_OH_EXT_LT_BEACON", "Number"),
    // Strobe selector: 0 = OFF, 1 = AUTO, 2 = ON.
    F::f64("L:S_OH_EXT_LT_STROBE", "Number"),
    // Combined nav + logo: 0 = OFF, 1 = NAV only, 2 = NAV + LOGO.
    F::f64("L:S_OH_EXT_LT_NAV_LOGO", "Number"),
    // Parking brake on Fenix MIP: 0 = released, 1 = set.
    F::f64("L:S_MIP_PARKING_BRAKE", "Number"),

    // Cabin signs: real A320 has 3-pos toggles (OFF/AUTO/ON);
    // Fenix exposes them under the SIGNS namespace, NOT under
    // INT_LT as my first guess assumed.
    F::f64("L:S_OH_SIGNS", "Number"),
    F::f64("L:S_OH_SIGNS_SMOKING", "Number"),

    // APU electrical pushbuttons.
    F::f64("L:S_OH_ELEC_APU_MASTER", "Number"),
    F::f64("L:S_OH_ELEC_APU_START", "Number"),

    // Anti-ice (engine + wing). The PROBE/WINDOW HEAT switch lives
    // outside the PNEUMATIC namespace by Fenix's convention.
    F::f64("L:S_OH_PNEUMATIC_ENG1_ANTI_ICE", "Number"),
    F::f64("L:S_OH_PNEUMATIC_ENG2_ANTI_ICE", "Number"),
    F::f64("L:S_OH_PNEUMATIC_WING_ANTI_ICE", "Number"),
    F::f64("L:S_OH_PROBE_HEAT", "Number"),

    // Electric panel.
    F::f64("L:S_OH_ELEC_BAT1", "Number"),
    F::f64("L:S_OH_ELEC_BAT2", "Number"),
    F::f64("L:S_OH_ELEC_EXT_PWR", "Number"),

    // FCU button states — replace the unreliable `L:I_FCU_*` lamp
    // LVars from earlier sessions. The S_ prefix is the button
    // press state, which actually toggles cleanly.
    F::f64("L:S_FCU_AP1", "Number"),
    F::f64("L:S_FCU_AP2", "Number"),
    F::f64("L:S_FCU_APPR", "Number"),
    F::f64("L:S_FCU_ATHR", "Number"),

    // FCU encoder displays — what the pilot has selected on the
    // glareshield. Used to log "Selected ALT 36000" / "Selected
    // HDG 280" / etc. as the pilot tunes them.
    F::f64("L:E_FCU_ALTITUDE", "Number"),
    F::f64("L:E_FCU_HEADING", "Number"),
    F::f64("L:E_FCU_SPEED", "Number"),
    F::f64("L:E_FCU_VS", "Number"),

    // Autobrake setting indicators (lamp LVars on the MIP).
    F::f64("L:I_MIP_AUTOBRAKE_LO_L", "Number"),
    F::f64("L:I_MIP_AUTOBRAKE_MED_L", "Number"),
    F::f64("L:I_MIP_AUTOBRAKE_MAX_L", "Number"),
];

// Helper builders so the table above stays compact.
struct F;
impl F {
    const fn str(name: &'static str, unit: &'static str) -> TelemetryField {
        TelemetryField {
            name,
            unit,
            kind: FieldKind::String256,
        }
    }
    const fn f64(name: &'static str, unit: &'static str) -> TelemetryField {
        TelemetryField {
            name,
            unit,
            kind: FieldKind::Float64,
        }
    }
    const fn bool(name: &'static str) -> TelemetryField {
        TelemetryField {
            name,
            unit: "bool",
            kind: FieldKind::Int32,
        }
    }
}

/// Decoded telemetry — one snapshot's worth of values, before the
/// final mapping into [`SimSnapshot`].
#[derive(Debug, Default)]
pub struct Telemetry {
    pub title: String,
    pub atc_model: String,
    pub atc_id: String,

    pub lat: f64,
    pub lon: f64,
    pub altitude_msl_ft: f64,
    pub altitude_agl_ft: f64,

    pub heading_true_deg: f64,
    pub heading_magnetic_deg: f64,
    pub pitch_deg: f64,
    pub bank_deg: f64,
    pub vertical_speed_fpm: f64,
    /// Body-frame velocity components in feet per second. Used to
    /// compute sideslip / crab angle natively at touchdown.
    pub velocity_body_x_fps: f64,
    pub velocity_body_z_fps: f64,

    pub groundspeed_kt: f64,
    pub indicated_airspeed_kt: f64,
    pub true_airspeed_kt: f64,
    pub g_force: f64,
    /// Body-frame wind components in knots. Positive aircraft_wind_x
    /// = crosswind from the right; positive aircraft_wind_z = tailwind.
    pub aircraft_wind_x_kt: f64,
    pub aircraft_wind_z_kt: f64,

    pub on_ground: bool,
    pub parking_brake: bool,
    pub stall_warning: bool,
    pub overspeed_warning: bool,
    pub gear_position: f64,
    pub flaps_position: f64,
    pub eng1_firing: bool,
    pub eng2_firing: bool,
    pub eng3_firing: bool,
    pub eng4_firing: bool,

    pub fuel_total_lb_ex1: f64,
    pub fuel_total_lb_legacy: f64,
    pub total_weight_lb: f64,
    pub empty_weight_lb: f64,

    pub wind_direction_deg: f64,
    pub wind_speed_kt: f64,
    pub qnh_hpa: f64,
    pub oat_c: f64,
    pub tat_c: f64,
    pub mach: f64,

    pub transponder_bcd: f64,
    pub com1_mhz: f64,
    pub com2_mhz: f64,
    pub nav1_mhz: f64,
    pub nav2_mhz: f64,

    pub light_landing: bool,
    pub light_beacon: bool,
    pub light_strobe: bool,
    pub light_taxi: bool,
    pub light_nav: bool,
    pub light_logo: bool,

    pub ap_master: bool,
    pub ap_heading: bool,
    pub ap_altitude: bool,
    pub ap_nav: bool,
    pub ap_approach: bool,

    pub eng1_ff_pph: f64,
    pub eng2_ff_pph: f64,
    pub eng3_ff_pph: f64,
    pub eng4_ff_pph: f64,

    pub spoilers_handle_position: f64,
    pub spoilers_armed: bool,

    pub pushback_state: f64,

    pub apu_switch: bool,
    pub apu_pct_rpm: f64,
    pub battery_master: bool,
    pub avionics_master: bool,
    pub pitot_heat: bool,
    pub eng1_anti_ice: bool,
    pub eng2_anti_ice: bool,
    pub eng3_anti_ice: bool,
    pub eng4_anti_ice: bool,
    pub structural_deice: bool,

    // FBW A32NX LVars
    pub fbw_xpdr: f64,
    pub fbw_ap_active: f64,
    pub fbw_ap_hdg: f64,
    pub fbw_ap_alt: f64,
    pub fbw_ap_nav: f64,
    pub fbw_ap_appr: f64,
    pub fbw_total_fuel_kg: f64,

    // Fenix A320 LVars
    pub fnx_beacon: f64,
    pub fnx_strobe: f64,
    pub fnx_nav_logo: f64,
    pub fnx_park_brake: f64,
    pub fnx_signs_seatbelts: f64,
    pub fnx_signs_smoking: f64,
    pub fnx_apu_master: f64,
    pub fnx_apu_start: f64,
    pub fnx_eng1_anti_ice: f64,
    pub fnx_eng2_anti_ice: f64,
    pub fnx_wing_anti_ice: f64,
    pub fnx_probe_heat: f64,
    pub fnx_bat1: f64,
    pub fnx_bat2: f64,
    pub fnx_ext_pwr: f64,
    pub fnx_fcu_ap1: f64,
    pub fnx_fcu_ap2: f64,
    pub fnx_fcu_appr: f64,
    pub fnx_fcu_athr: f64,
    pub fnx_fcu_alt: f64,
    pub fnx_fcu_hdg: f64,
    pub fnx_fcu_spd: f64,
    pub fnx_fcu_vs: f64,
    pub fnx_autobrake_lo: f64,
    pub fnx_autobrake_med: f64,
    pub fnx_autobrake_max: f64,
}

// ---- Touchdown sample (separate data definition #2) ----
//
// MSFS itself latches these the moment the gear contacts the ground;
// values stay frozen until the next takeoff. Lives in its own data
// definition so a rejection (e.g. on aircraft / sim builds that
// don't expose all of these yet) can't shift the per-tick telemetry
// layout. Verified field names + units against the MSFS 2024 SDK
// docs:
// https://docs.flightsimulator.com/msfs2024/html/6_Programming_APIs/SimVars/Aircraft_SimVars/Aircraft_Misc_Variables.htm

pub const TOUCHDOWN_FIELDS: &[TelemetryField] = &[
    F::f64("PLANE TOUCHDOWN NORMAL VELOCITY", "feet per second"),
    F::f64("PLANE TOUCHDOWN PITCH DEGREES", "degrees"),
    F::f64("PLANE TOUCHDOWN BANK DEGREES", "degrees"),
    F::f64("PLANE TOUCHDOWN HEADING DEGREES MAGNETIC", "degrees"),
    F::f64("PLANE TOUCHDOWN LATITUDE", "radians"),
    F::f64("PLANE TOUCHDOWN LONGITUDE", "radians"),
];

#[derive(Debug, Default, Clone, Copy)]
pub struct Touchdown {
    pub vs_fps: f64,
    pub pitch_deg: f64,
    pub bank_deg: f64,
    pub heading_mag_deg: f64,
    pub lat_rad: f64,
    pub lon_rad: f64,
}

impl Touchdown {
    pub fn from_block(bytes: &[u8]) -> Self {
        let mut t = Touchdown::default();
        let mut off = 0usize;
        if let Some(v) = read_f64(bytes, off) { t.vs_fps = v; }
        off += 8;
        if let Some(v) = read_f64(bytes, off) { t.pitch_deg = v; }
        off += 8;
        if let Some(v) = read_f64(bytes, off) { t.bank_deg = v; }
        off += 8;
        if let Some(v) = read_f64(bytes, off) { t.heading_mag_deg = v; }
        off += 8;
        if let Some(v) = read_f64(bytes, off) { t.lat_rad = v; }
        off += 8;
        if let Some(v) = read_f64(bytes, off) { t.lon_rad = v; }
        let _ = off;
        t
    }

    pub fn _dummy() {} // keep impl block aligned
}

// ---- Live SimVar/LVar Inspector (debug feature) ----
//
// A user-driven watchlist that registers arbitrary SimVar / LVar
// names against SimConnect at runtime. Lives behind a separate data
// definition (#3) so the user can add a name with a typo without
// breaking real telemetry.

/// Type discriminator for watched values. Matches the SimConnect
/// data type we use for the corresponding `AddToDataDefinition` call.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WatchKind {
    /// FLOAT64. Use for raw numeric SimVars and LVars (most cases).
    Number,
    /// INT32. Use for SimConnect bool SimVars (e.g. SIM ON GROUND).
    Bool,
    /// STRING256. Use for TITLE / ATC MODEL etc.
    String,
}

impl WatchKind {
    pub fn size(self) -> usize {
        match self {
            WatchKind::Number => 8,
            WatchKind::Bool => 4,
            WatchKind::String => 256,
        }
    }
}

/// Latest value for one watch entry.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum WatchValue {
    Number(f64),
    Bool(bool),
    String(String),
}

/// One entry in the inspector's watchlist. `value` is None until the
/// next dispatch tick after registration.
#[derive(Debug, Clone, serde::Serialize)]
pub struct InspectorWatch {
    pub id: u32,
    pub name: String,
    pub unit: String,
    pub kind: WatchKind,
    /// Set whenever a SIMCONNECT_RECV_EXCEPTION fires for this entry
    /// during registration, so the UI can render an error indicator
    /// instead of a stale value.
    pub error: Option<String>,
    pub value: Option<WatchValue>,
}

/// Mutable inspector state, owned by the adapter's `Shared`.
#[derive(Debug, Default)]
pub struct InspectorState {
    pub watches: Vec<InspectorWatch>,
    pub next_id: u32,
    /// Set when the watchlist has changed and the worker needs to
    /// re-register data definition #3.
    pub dirty: bool,
}

impl InspectorState {
    pub fn add(&mut self, name: String, unit: String, kind: WatchKind) -> u32 {
        self.next_id += 1;
        let id = self.next_id;
        self.watches.push(InspectorWatch {
            id,
            name,
            unit,
            kind,
            error: None,
            value: None,
        });
        self.dirty = true;
        id
    }

    pub fn remove(&mut self, id: u32) {
        let before = self.watches.len();
        self.watches.retain(|w| w.id != id);
        if self.watches.len() != before {
            self.dirty = true;
        }
    }

    /// Parse the data block returned by SimConnect for the inspector
    /// definition — fields are at fixed offsets in watchlist order,
    /// same parsing model as the main telemetry block.
    pub fn ingest(&mut self, bytes: &[u8]) {
        let mut off = 0usize;
        for w in &mut self.watches {
            match w.kind {
                WatchKind::Number => {
                    if let Some(v) = read_f64(bytes, off) {
                        w.value = Some(WatchValue::Number(v));
                    }
                    off += 8;
                }
                WatchKind::Bool => {
                    if let Some(v) = read_i32(bytes, off) {
                        w.value = Some(WatchValue::Bool(v != 0));
                    }
                    off += 4;
                }
                WatchKind::String => {
                    if let Some(v) = read_str256(bytes, off) {
                        w.value = Some(WatchValue::String(v));
                    }
                    off += 256;
                }
            }
        }
    }
}

impl Touchdown {
    /// `true` while no *real* touchdown has happened yet.
    ///
    /// MSFS populates the PLANE TOUCHDOWN * SimVars with the
    /// aircraft's current state when it's spawned on the ground —
    /// matching live position, heading, pitch — but with `vs_fps`
    /// at exactly 0. That's not a useful "touchdown" for an ACARS
    /// landing analyzer, only an actual descent ends with a
    /// non-zero touchdown rate. Filtering on `vs_fps == 0` cleanly
    /// rejects both the all-zero pre-spawn state and the
    /// spawn-on-ground state, leaving real landings to come
    /// through.
    pub fn is_uninitialised(&self) -> bool {
        self.vs_fps == 0.0
    }
}

impl Telemetry {
    fn from_block(bytes: &[u8]) -> Self {
        // Walk the buffer in TELEMETRY_FIELDS order. If the buffer is
        // shorter than expected (some SimVar got rejected and the
        // tail is missing), every later field stays at its default.
        let mut t = Telemetry::default();
        let mut off = 0usize;

        // Macro-equivalent: pull next field into `dst` if the buffer
        // is long enough. Strings copy the NUL-terminated content.
        macro_rules! pull_f64 {
            ($dst:expr) => {
                if let Some(v) = read_f64(bytes, off) {
                    $dst = v;
                }
                off += 8;
            };
        }
        macro_rules! pull_i32 {
            ($dst:expr) => {
                if let Some(v) = read_i32(bytes, off) {
                    $dst = v != 0;
                }
                off += 4;
            };
        }
        macro_rules! pull_str {
            ($dst:expr) => {
                if let Some(v) = read_str256(bytes, off) {
                    $dst = v;
                }
                off += 256;
            };
        }

        // Same order as TELEMETRY_FIELDS — keep these in lock-step.
        pull_str!(t.title);
        pull_str!(t.atc_model);
        pull_str!(t.atc_id);

        pull_f64!(t.lat);
        pull_f64!(t.lon);
        pull_f64!(t.altitude_msl_ft);
        pull_f64!(t.altitude_agl_ft);

        pull_f64!(t.heading_true_deg);
        pull_f64!(t.heading_magnetic_deg);
        pull_f64!(t.pitch_deg);
        pull_f64!(t.bank_deg);
        pull_f64!(t.vertical_speed_fpm);
        pull_f64!(t.velocity_body_x_fps);
        pull_f64!(t.velocity_body_z_fps);

        pull_f64!(t.groundspeed_kt);
        pull_f64!(t.indicated_airspeed_kt);
        pull_f64!(t.true_airspeed_kt);
        pull_f64!(t.g_force);
        pull_f64!(t.aircraft_wind_x_kt);
        pull_f64!(t.aircraft_wind_z_kt);

        pull_i32!(t.on_ground);
        pull_i32!(t.parking_brake);
        pull_i32!(t.stall_warning);
        pull_i32!(t.overspeed_warning);
        pull_f64!(t.gear_position);
        pull_f64!(t.flaps_position);
        pull_i32!(t.eng1_firing);
        pull_i32!(t.eng2_firing);
        pull_i32!(t.eng3_firing);
        pull_i32!(t.eng4_firing);

        pull_f64!(t.fuel_total_lb_ex1);
        pull_f64!(t.fuel_total_lb_legacy);
        pull_f64!(t.total_weight_lb);
        pull_f64!(t.empty_weight_lb);

        pull_f64!(t.wind_direction_deg);
        pull_f64!(t.wind_speed_kt);
        pull_f64!(t.qnh_hpa);
        pull_f64!(t.oat_c);
        pull_f64!(t.tat_c);
        pull_f64!(t.mach);

        pull_f64!(t.transponder_bcd);
        pull_f64!(t.com1_mhz);
        pull_f64!(t.com2_mhz);
        pull_f64!(t.nav1_mhz);
        pull_f64!(t.nav2_mhz);

        pull_i32!(t.light_landing);
        pull_i32!(t.light_beacon);
        pull_i32!(t.light_strobe);
        pull_i32!(t.light_taxi);
        pull_i32!(t.light_nav);
        pull_i32!(t.light_logo);

        pull_i32!(t.ap_master);
        pull_i32!(t.ap_heading);
        pull_i32!(t.ap_altitude);
        pull_i32!(t.ap_nav);
        pull_i32!(t.ap_approach);

        pull_f64!(t.eng1_ff_pph);
        pull_f64!(t.eng2_ff_pph);
        pull_f64!(t.eng3_ff_pph);
        pull_f64!(t.eng4_ff_pph);

        pull_f64!(t.spoilers_handle_position);
        pull_i32!(t.spoilers_armed);

        pull_f64!(t.pushback_state);

        pull_i32!(t.apu_switch);
        pull_f64!(t.apu_pct_rpm);
        pull_i32!(t.battery_master);
        pull_i32!(t.avionics_master);
        pull_i32!(t.pitot_heat);
        pull_i32!(t.eng1_anti_ice);
        pull_i32!(t.eng2_anti_ice);
        pull_i32!(t.eng3_anti_ice);
        pull_i32!(t.eng4_anti_ice);
        pull_i32!(t.structural_deice);

        pull_f64!(t.fbw_xpdr);
        pull_f64!(t.fbw_ap_active);
        pull_f64!(t.fbw_ap_hdg);
        pull_f64!(t.fbw_ap_alt);
        pull_f64!(t.fbw_ap_nav);
        pull_f64!(t.fbw_ap_appr);
        pull_f64!(t.fbw_total_fuel_kg);

        pull_f64!(t.fnx_beacon);
        pull_f64!(t.fnx_strobe);
        pull_f64!(t.fnx_nav_logo);
        pull_f64!(t.fnx_park_brake);
        pull_f64!(t.fnx_signs_seatbelts);
        pull_f64!(t.fnx_signs_smoking);
        pull_f64!(t.fnx_apu_master);
        pull_f64!(t.fnx_apu_start);
        pull_f64!(t.fnx_eng1_anti_ice);
        pull_f64!(t.fnx_eng2_anti_ice);
        pull_f64!(t.fnx_wing_anti_ice);
        pull_f64!(t.fnx_probe_heat);
        pull_f64!(t.fnx_bat1);
        pull_f64!(t.fnx_bat2);
        pull_f64!(t.fnx_ext_pwr);
        pull_f64!(t.fnx_fcu_ap1);
        pull_f64!(t.fnx_fcu_ap2);
        pull_f64!(t.fnx_fcu_appr);
        pull_f64!(t.fnx_fcu_athr);
        pull_f64!(t.fnx_fcu_alt);
        pull_f64!(t.fnx_fcu_hdg);
        pull_f64!(t.fnx_fcu_spd);
        pull_f64!(t.fnx_fcu_vs);
        pull_f64!(t.fnx_autobrake_lo);
        pull_f64!(t.fnx_autobrake_med);
        pull_f64!(t.fnx_autobrake_max);

        // Silence the unused-assignment warning the last `pull_*!`
        // emits (the macro always advances `off`, but the very last
        // call doesn't read it again).
        let _ = off;

        t
    }
}

/// Convenience used by the worker: parse + remap to `SimSnapshot`.
pub fn parse(bytes: &[u8], simulator: Simulator) -> SimSnapshot {
    let t = Telemetry::from_block(bytes);
    telemetry_to_snapshot(t, simulator)
}

/// Map 0.0 → None, anything > 0 → Some. Used for SimVars where a
/// genuine zero is meaningless (frequencies, percentages) so we can
/// tell "this addon doesn't wire it" from "it's actually zero".
fn positive_or_none(v: f32) -> Option<f32> {
    if v > 0.0 { Some(v) } else { None }
}

fn read_f64(bytes: &[u8], off: usize) -> Option<f64> {
    bytes.get(off..off + 8).map(|s| {
        let mut buf = [0u8; 8];
        buf.copy_from_slice(s);
        f64::from_le_bytes(buf)
    })
}

fn read_i32(bytes: &[u8], off: usize) -> Option<i32> {
    bytes.get(off..off + 4).map(|s| {
        let mut buf = [0u8; 4];
        buf.copy_from_slice(s);
        i32::from_le_bytes(buf)
    })
}

fn read_str256(bytes: &[u8], off: usize) -> Option<String> {
    bytes.get(off..off + 256).map(|s| {
        let end = s.iter().position(|b| *b == 0).unwrap_or(s.len());
        String::from_utf8_lossy(&s[..end]).into_owned()
    })
}

fn telemetry_to_snapshot(t: Telemetry, simulator: Simulator) -> SimSnapshot {
    let profile = AircraftProfile::detect(&t.title, &t.atc_model);
    let is_fenix = matches!(profile, AircraftProfile::FenixA320);
    let is_fbw = matches!(profile, AircraftProfile::FbwA32nx);

    let engines_running = (t.eng1_firing as u8)
        + (t.eng2_firing as u8)
        + (t.eng3_firing as u8)
        + (t.eng4_firing as u8);

    // Fuel pick order: FBW LVar (already in kg) > EX1 SimVar (SU2+,
    // works for modern fuel system) > legacy WEIGHT SimVar.
    let fuel_total_kg = if is_fbw && t.fbw_total_fuel_kg > 0.0 {
        t.fbw_total_fuel_kg as f32
    } else if t.fuel_total_lb_ex1 > 0.0 {
        (t.fuel_total_lb_ex1 * KG_PER_LB) as f32
    } else {
        (t.fuel_total_lb_legacy * KG_PER_LB) as f32
    };

    // Gross weight: TOTAL WEIGHT is documented as authoritative.
    let total_weight_kg = if t.total_weight_lb > 0.0 {
        Some((t.total_weight_lb * KG_PER_LB) as f32)
    } else {
        None
    };

    // ZFW = Zero Fuel Weight = gross weight minus current fuel.
    // Matches the value Airbus EFBs / FMCs display under "ZFW".
    // Only meaningful when both inputs are positive — otherwise the
    // arithmetic produces nonsense (e.g. GW=0 - fuel=4700 → -4700).
    let zfw_kg = match total_weight_kg {
        Some(gw) if gw > 0.0 && fuel_total_kg >= 0.0 && gw > fuel_total_kg => {
            Some(gw - fuel_total_kg)
        }
        _ => None,
    };

    // OEW (operating empty weight). Reject implausibly small values —
    // the Asobo A320neo default reports ~1422 kg which is clearly bogus
    // (real OEW is ~42 t). Smallest realistic transport-cat empty
    // weight is a King Air at ~3.5 t / 7700 lb, so we'd ideally clamp
    // there, but for now we just drop literal-zero readings and trust
    // the value otherwise (lets GA addons through).
    let empty_weight_kg: Option<f32> = {
        let kg = (t.empty_weight_lb * KG_PER_LB) as f32;
        if kg > 0.0 { Some(kg) } else { None }
    };

    // Payload = ZFW − OEW. No MSFS SimVar exposes payload directly
    // (Fenix and most addons leave `PAYLOAD WEIGHT` unwired) but the
    // arithmetic is exact: ZFW = OEW + Payload by definition. Skip
    // when either input is missing or the result would be negative
    // (= bogus OEW > ZFW combination).
    let payload_kg: Option<f32> = match (zfw_kg, empty_weight_kg) {
        (Some(z), Some(o)) if z > o => Some(z - o),
        _ => None,
    };

    // Total fuel flow across all running engines, kg/h. Sum the
    // per-engine PPH SimVars and convert.
    let total_ff_pph = t.eng1_ff_pph + t.eng2_ff_pph + t.eng3_ff_pph + t.eng4_ff_pph;
    let fuel_flow_kg_per_h = if total_ff_pph > 0.0 {
        Some((total_ff_pph * KG_PER_LB) as f32)
    } else {
        None
    };

    // Transponder code: FBW writes a plain decimal LVar (e.g.
    // L:A32NX_TRANSPONDER_CODE = 2523 means squawk 2523), the
    // standard SimVar is BCD-encoded (0x1234 = squawk 1234).
    let transponder_code = if is_fbw && t.fbw_xpdr > 0.0 {
        Some(t.fbw_xpdr.round().clamp(0.0, 7777.0) as u16)
    } else if t.transponder_bcd > 0.0 {
        let raw = t.transponder_bcd.round() as u32;
        let d1 = (raw >> 12) & 0xF;
        let d2 = (raw >> 8) & 0xF;
        let d3 = (raw >> 4) & 0xF;
        let d4 = raw & 0xF;
        Some((d1 * 1000 + d2 * 100 + d3 * 10 + d4) as u16)
    } else {
        None
    };

    // Autopilot:
    //   * FBW: dedicated LVars (live mode state).
    //   * Fenix: the `L:S_FCU_*` button-state LVars from the AAO
    //     script. We treat AP1 OR AP2 active as "Master engaged".
    //     Heading / altitude / NAV button-state isn't directly the
    //     same as "mode is armed", but it's a closer signal than
    //     the I_FCU_* lamp LVars from the legacy session (those
    //     flickered with unrelated cockpit switches).
    //   * Default + others: standard MSFS SimVars.
    let (ap_master, ap_hdg, ap_alt, ap_nav, ap_appr) = if is_fbw {
        (
            t.fbw_ap_active != 0.0,
            t.fbw_ap_hdg != 0.0,
            t.fbw_ap_alt != 0.0,
            t.fbw_ap_nav != 0.0,
            t.fbw_ap_appr != 0.0,
        )
    } else if is_fenix {
        let master = t.fnx_fcu_ap1 as i32 != 0 || t.fnx_fcu_ap2 as i32 != 0;
        // We don't have HDG/ALT/NAV-mode LVars yet; fall back to the
        // standard SimVars for those if Fenix wires them, otherwise
        // they stay false. AP master is the most important value.
        (
            master,
            t.ap_heading,
            t.ap_altitude,
            t.ap_nav,
            t.fnx_fcu_appr as i32 != 0 || t.ap_approach,
        )
    } else {
        (
            t.ap_master,
            t.ap_heading,
            t.ap_altitude,
            t.ap_nav,
            t.ap_approach,
        )
    };

    // Lights: Fenix uses overhead-LVars instead of the standard
    // SimVars, with selector positions (off / auto / on; nav-only /
    // nav+logo). Translate to bools for the binary pills, plus a
    // separate `strobe_state` carrying the full 0/1/2 enum so the
    // activity log can distinguish AUTO from ON (real pilots flip
    // between those at runway entry/exit, and we'd lose the event
    // if we collapsed everything to "Strobe lights ON").
    let (light_beacon, light_strobe, light_nav, light_logo) = if is_fenix {
        (
            t.fnx_beacon as i32 != 0,
            t.fnx_strobe as i32 != 0,
            t.fnx_nav_logo as i32 >= 1,
            t.fnx_nav_logo as i32 >= 2,
        )
    } else {
        (t.light_beacon, t.light_strobe, t.light_nav, t.light_logo)
    };
    let strobe_state = if is_fenix {
        Some(t.fnx_strobe.round().clamp(0.0, 2.0) as u8)
    } else {
        None
    };

    // Parking brake: Fenix routes through L:S_MIP_PARKING_BRAKE
    // (the MIP switch state) which is more reliable than the
    // standard SimVar on that aircraft.
    let parking_brake = if is_fenix {
        t.fnx_park_brake as i32 != 0
    } else {
        t.parking_brake
    };

    // System switch overrides for Fenix (LVar names verified against
    // the Axis-and-Ohs script bundle). Each one falls back to the
    // standard SimVar if the LVar reads exactly 0 — that way the
    // override only takes over when Fenix is actually feeding values.
    let apu_switch = if is_fenix {
        t.fnx_apu_master as i32 != 0
    } else {
        t.apu_switch
    };
    let pitot_heat = if is_fenix {
        // L:S_OH_PROBE_HEAT: 0=AUTO, 1=ON. AUTO means heating is
        // automatically active when engines are running, so we
        // treat both states as "heat available".
        t.fnx_probe_heat >= 0.0 // always considered "active" on Airbus
    } else {
        t.pitot_heat
    };
    let battery_master = if is_fenix {
        // BAT 1 OR BAT 2 in AUTO/ON position counts as "battery on".
        // 0=OFF, 1=AUTO on real Airbus.
        t.fnx_bat1 as i32 != 0 || t.fnx_bat2 as i32 != 0
    } else {
        t.battery_master
    };
    let engine_anti_ice = if is_fenix {
        t.fnx_eng1_anti_ice as i32 != 0 || t.fnx_eng2_anti_ice as i32 != 0
    } else {
        t.eng1_anti_ice || t.eng2_anti_ice || t.eng3_anti_ice || t.eng4_anti_ice
    };
    let wing_anti_ice = if is_fenix {
        t.fnx_wing_anti_ice as i32 != 0
    } else {
        t.structural_deice
    };

    // Cabin signs (Fenix only — no standard SimVar covers these).
    //
    // The AAO script reveals the value spaces are different between
    // the two signs:
    //   * `L:S_OH_SIGNS` (seat belts) is BINARY — its toggle uses
    //     the logical-NOT operator `! (>L:S_OH_SIGNS)`, which only
    //     makes sense for a 0/1 LVar. We clamp accordingly.
    //   * `L:S_OH_SIGNS_SMOKING` (no smoking) is 3-state — the toggle
    //     branches `0 == if{ 2 } els{ 0 }` and other scripts touch
    //     value 1, confirming OFF/AUTO/ON semantics.
    //
    // Keep both as `Option<u8>`; the activity-log helper picks the
    // right label set per field below.
    let seatbelts_sign = if is_fenix {
        Some(t.fnx_signs_seatbelts.round().clamp(0.0, 1.0) as u8)
    } else {
        None
    };
    let no_smoking_sign = if is_fenix {
        Some(t.fnx_signs_smoking.round().clamp(0.0, 2.0) as u8)
    } else {
        None
    };

    // FCU selected values — currently only Fenix exposes them via
    // dedicated LVars. Default-aircraft AP target SimVars (e.g.
    // AUTOPILOT ALTITUDE LOCK VAR) exist but aren't subscribed yet,
    // so for now FCU values stay None outside Fenix.
    let (fcu_alt, fcu_hdg, fcu_spd, fcu_vs) = if is_fenix {
        (
            Some(t.fnx_fcu_alt.round() as i32),
            Some(t.fnx_fcu_hdg.round() as i32),
            Some(t.fnx_fcu_spd.round() as i32),
            Some(t.fnx_fcu_vs.round() as i32),
        )
    } else {
        (None, None, None, None)
    };

    // Autobrake setting — derived from the three indicator-lamp
    // LVars (LO/MED/MAX). Only one of them is illuminated at a
    // time. Fenix exposes these as `L:I_MIP_AUTOBRAKE_*_L`.
    let autobrake = if is_fenix {
        if t.fnx_autobrake_max as i32 != 0 {
            Some("MAX".to_string())
        } else if t.fnx_autobrake_med as i32 != 0 {
            Some("MED".to_string())
        } else if t.fnx_autobrake_lo as i32 != 0 {
            Some("LO".to_string())
        } else {
            Some("OFF".to_string())
        }
    } else {
        None
    };

    // Pushback state — value 3 means MSFS reports the tug has
    // disconnected (or there was never a tug). Anything else is
    // an active push. Stored as Option<u8> so consumers can tell
    // "not wired" from "no pushback (=3)".
    let pushback_state = {
        let raw = t.pushback_state.round() as i32;
        if (0..=3).contains(&raw) {
            Some(raw as u8)
        } else {
            None
        }
    };

    SimSnapshot {
        timestamp: Utc::now(),
        lat: t.lat,
        lon: t.lon,
        altitude_msl_ft: t.altitude_msl_ft,
        altitude_agl_ft: t.altitude_agl_ft,
        heading_deg_true: t.heading_true_deg as f32,
        heading_deg_magnetic: t.heading_magnetic_deg as f32,
        // v0.5.24: MSFS-SimConnect convention is INVERTED — `PLANE PITCH
        // DEGREES` reports positive values when the nose is BELOW horizon.
        // We negate here so downstream code (FSM phase transitions,
        // tail-strike check, sampler capture, PIREP custom fields,
        // analytics) sees the universal aviation convention: positive
        // pitch = nose UP, like X-Plane reports natively.
        // Without this, every MSFS PIREP had inverted pitch (e.g. an
        // A321 rotation showed as -11.2° instead of +11.2°), which made
        // tail-strike checks rely on abs() to mask the bug, but
        // confused pilots reading the raw value in their PIREP detail.
        pitch_deg: -(t.pitch_deg as f32),
        bank_deg: t.bank_deg as f32,
        vertical_speed_fpm: t.vertical_speed_fpm as f32,
        velocity_body_x_fps: Some(t.velocity_body_x_fps as f32),
        velocity_body_z_fps: Some(t.velocity_body_z_fps as f32),
        groundspeed_kt: t.groundspeed_kt as f32,
        // Clamp small negative readings to zero — MSFS pitot simulation
        // (especially with study-level addons) sometimes reports a few
        // negative knots while parked. Mirrors the X-Plane adapter's
        // identical clamp; pilots reasonably treat "−10 kt" as a bug.
        indicated_airspeed_kt: (t.indicated_airspeed_kt as f32).max(0.0),
        true_airspeed_kt: (t.true_airspeed_kt as f32).max(0.0),
        aircraft_wind_x_kt: Some(t.aircraft_wind_x_kt as f32),
        aircraft_wind_z_kt: Some(t.aircraft_wind_z_kt as f32),
        g_force: t.g_force as f32,
        on_ground: t.on_ground,
        // MSFS-Adapter liefert keinen Gear-Normal-Force-Wert; das
        // X-Plane-Pendant (sampler-side touchdown edge) ist hier
        // nicht aktiv — MSFS hat eh den separaten
        // PLANE TOUCHDOWN NORMAL VELOCITY-SimVar als Primary-Quelle.
        gear_normal_force_n: None,
        parking_brake,
        stall_warning: t.stall_warning,
        overspeed_warning: t.overspeed_warning,
        paused: false,
        slew_mode: false,
        simulation_rate: 1.0,
        gear_position: t.gear_position as f32,
        flaps_position: t.flaps_position as f32,
        engines_running,
        fuel_total_kg,
        fuel_used_kg: 0.0,
        zfw_kg,
        payload_kg,
        total_weight_kg,
        // Touchdown sample: not yet wired in raw mode; stays None
        // until we add a second data definition for them. The legacy
        // adapter also kept these None.
        touchdown_vs_fpm: None,
        touchdown_pitch_deg: None,
        touchdown_bank_deg: None,
        touchdown_heading_mag_deg: None,
        touchdown_lat: None,
        touchdown_lon: None,
        wind_direction_deg: Some(t.wind_direction_deg as f32),
        wind_speed_kt: Some(t.wind_speed_kt as f32),
        qnh_hpa: Some(t.qnh_hpa as f32),
        outside_air_temp_c: Some(t.oat_c as f32),
        total_air_temp_c: Some(t.tat_c as f32),
        mach: Some(t.mach as f32),
        empty_weight_kg,
        aircraft_title: Some(t.title).filter(|s| !s.is_empty()),
        aircraft_icao: Some(t.atc_model).filter(|s| !s.is_empty()),
        aircraft_registration: Some(t.atc_id).filter(|s| !s.is_empty()),
        simulator,
        sim_version: None,
        // Avionics: standard SimVars. Under the legacy Rust crate we
        // had to force None for Fenix because the memory layout shifted
        // and we'd read QNH-bleed garbage (e.g. "COM1 1024 MHz"). Raw
        // FFI parses each field at a fixed offset so the noise is gone
        // — emit whatever the SimVar reports. The activity-log change
        // detector skips entries that don't actually change, so an
        // aircraft that genuinely doesn't wire these just leaves them
        // at their default (0 → no log entries) without spamming.
        transponder_code,
        com1_mhz: positive_or_none(t.com1_mhz as f32),
        com2_mhz: positive_or_none(t.com2_mhz as f32),
        nav1_mhz: positive_or_none(t.nav1_mhz as f32),
        nav2_mhz: positive_or_none(t.nav2_mhz as f32),
        light_landing: Some(t.light_landing),
        light_beacon: Some(light_beacon),
        light_strobe: Some(light_strobe),
        light_taxi: Some(t.light_taxi),
        light_nav: Some(light_nav),
        light_logo: Some(light_logo),
        strobe_state,
        autopilot_master: Some(ap_master),
        autopilot_heading: Some(ap_hdg),
        autopilot_altitude: Some(ap_alt),
        autopilot_nav: Some(ap_nav),
        autopilot_approach: Some(ap_appr),
        fuel_flow_kg_per_h,
        spoilers_handle_position: Some(t.spoilers_handle_position as f32),
        spoilers_armed: Some(t.spoilers_armed),
        pushback_state,
        apu_switch: Some(apu_switch),
        apu_pct_rpm: Some(t.apu_pct_rpm as f32),
        battery_master: Some(battery_master),
        avionics_master: Some(t.avionics_master),
        pitot_heat: Some(pitot_heat),
        engine_anti_ice: Some(engine_anti_ice),
        wing_anti_ice: Some(wing_anti_ice),
        // v0.3.0: filled by the PMDG snapshot()-merge layer when a
        // PMDG aircraft is loaded. Standard MSFS SimVars don't expose
        // these as separate fields, so they stay None for non-PMDG.
        light_wing: None,
        light_wheel_well: None,
        xpdr_mode_label: None,
        takeoff_config_warning: None,
        seatbelts_sign,
        no_smoking_sign,
        fcu_selected_altitude_ft: fcu_alt,
        fcu_selected_heading_deg: fcu_hdg,
        fcu_selected_speed_kt: fcu_spd,
        fcu_selected_vs_fpm: fcu_vs,
        autobrake,
        parking_name: None,
        parking_number: None,
        selected_runway: None,
        aircraft_profile: profile,
        // PMDG SDK data is filled in MsfsAdapter::snapshot() by
        // merging the latest ClientData block — not here in the
        // standard SimVar parse path.
        pmdg: None,
    }
}
