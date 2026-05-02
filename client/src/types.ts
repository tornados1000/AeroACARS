// Mirrors the Rust types in `client/src-tauri/crates/api-client/src/lib.rs`
// and the command return types in `client/src-tauri/src/lib.rs`.
//
// Keep these in sync manually for now. Phase 4: codegen via JSON Schema.

export interface Airline {
  id: number;
  icao: string;
  iata: string | null;
  name: string;
  logo: string | null;
}

export interface Rank {
  name: string | null;
}

export interface Profile {
  id: number;
  pilot_id: number;
  ident: string | null;
  name: string;
  email: string | null;
  airline_id: number | null;
  curr_airport: string | null;
  home_airport: string | null;
  airline: Airline | null;
  rank: Rank | null;
}

export interface LoginResult {
  profile: Profile;
  base_url: string;
}

/** Stable error code from the Rust side; maps to an i18n key. */
export interface UiError {
  code: string;
  message: string;
}

export interface AppInfo {
  name: string;
  version: string;
  commit: string | null;
  /** Small credit line for the Settings footer. */
  credit: string;
}

export interface Airport {
  id: string;
  icao: string | null;
  iata: string | null;
  name: string | null;
}

export interface Distance {
  mi: number | null;
  km: number | null;
  nmi: number | null;
}

export interface SimBrief {
  id: string;
  url: string | null;
  aircraft_id: number | null;
}

export interface Flight {
  id: string;
  flight_number: string;
  route_code: string | null;
  route_leg: string | null;
  callsign: string | null;
  dpt_airport_id: string;
  arr_airport_id: string;
  alt_airport_id: string | null;
  flight_time: number | null;
  level: number | null;
  route: string | null;
  flight_type: string | null;
  distance: Distance | null;
  airline: Airline | null;
  dpt_airport: Airport | null;
  arr_airport: Airport | null;
  simbrief: SimBrief | null;
}

export interface Bid {
  id: number;
  user_id: number;
  flight_id: string;
  flight: Flight;
}

// ---- Simulator telemetry (mirrors sim-core::SimSnapshot) ----

export type Simulator =
  | "Msfs2020"
  | "Msfs2024"
  | "XPlane11"
  | "XPlane12"
  | "Other";

export interface SimSnapshot {
  timestamp: string; // ISO-8601 UTC
  lat: number;
  lon: number;
  altitude_msl_ft: number;
  altitude_agl_ft: number;
  heading_deg_true: number;
  heading_deg_magnetic: number;
  pitch_deg: number;
  bank_deg: number;
  vertical_speed_fpm: number;
  /** Body-frame velocity components (fps). Used for native sideslip
   *  at touchdown. None when the SimVar isn't wired. */
  velocity_body_x_fps: number | null;
  velocity_body_z_fps: number | null;
  groundspeed_kt: number;
  indicated_airspeed_kt: number;
  true_airspeed_kt: number;
  /** Body-frame wind components (knots). Positive aircraft_wind_x_kt
   *  = crosswind from the right; positive aircraft_wind_z_kt = tailwind. */
  aircraft_wind_x_kt: number | null;
  aircraft_wind_z_kt: number | null;
  g_force: number;
  on_ground: boolean;
  parking_brake: boolean;
  stall_warning: boolean;
  overspeed_warning: boolean;
  paused: boolean;
  slew_mode: boolean;
  simulation_rate: number;
  gear_position: number;
  flaps_position: number;
  engines_running: number;
  fuel_total_kg: number;
  fuel_used_kg: number;
  zfw_kg: number | null;
  payload_kg: number | null;
  /** Gross weight (`TOTAL WEIGHT` SimVar). null when the aircraft addon
   *  doesn't wire it (e.g. Fenix). Already in kg. */
  total_weight_kg: number | null;
  /** Avionics — null when the active aircraft profile doesn't wire
   *  the underlying SimVar/LVar (notably Fenix for COM/NAV). */
  transponder_code: number | null;
  com1_mhz: number | null;
  com2_mhz: number | null;
  nav1_mhz: number | null;
  nav2_mhz: number | null;
  /** Exterior lights — null when the active aircraft profile doesn't
   *  wire the underlying SimVar/LVar. */
  light_landing: boolean | null;
  light_beacon: boolean | null;
  light_strobe: boolean | null;
  light_taxi: boolean | null;
  light_nav: boolean | null;
  light_logo: boolean | null;
  /** 3-state strobe selector — 0=OFF, 1=AUTO, 2=ON. null when only
   *  the binary `light_strobe` is available. */
  strobe_state: number | null;
  /** Autopilot state — same null semantics as lights. */
  autopilot_master: boolean | null;
  autopilot_heading: boolean | null;
  autopilot_altitude: boolean | null;
  autopilot_nav: boolean | null;
  autopilot_approach: boolean | null;
  /** Surfaces */
  spoilers_handle_position: number | null;
  spoilers_armed: boolean | null;
  /** MSFS PUSHBACK STATE: 0/1/2 = pushing, 3 = no pushback. */
  pushback_state: number | null;
  /** Systems */
  apu_switch: boolean | null;
  apu_pct_rpm: number | null;
  battery_master: boolean | null;
  avionics_master: boolean | null;
  pitot_heat: boolean | null;
  /** Combined "any engine has anti-ice on". */
  engine_anti_ice: boolean | null;
  /** Wing / structural deice. */
  wing_anti_ice: boolean | null;
  /** Cabin SEAT BELTS sign — 0=OFF, 1=AUTO, 2=ON (Fenix LVar based). */
  seatbelts_sign: number | null;
  /** Cabin NO SMOKING sign — same enum semantics. */
  no_smoking_sign: number | null;
  /** FCU encoder displays — selected ALT/HDG/SPD/VS. */
  fcu_selected_altitude_ft: number | null;
  fcu_selected_heading_deg: number | null;
  fcu_selected_speed_kt: number | null;
  fcu_selected_vs_fpm: number | null;
  /** Autobrake setting label ("OFF"/"LO"/"MED"/"MAX") or null. */
  autobrake: string | null;
  /** Latched touchdown sample — populated by SimConnect at the moment
   *  the gear hits the ground. null until the first touchdown. */
  touchdown_vs_fpm: number | null;
  touchdown_pitch_deg: number | null;
  touchdown_bank_deg: number | null;
  touchdown_heading_mag_deg: number | null;
  touchdown_lat: number | null;
  touchdown_lon: number | null;
  wind_direction_deg: number | null;
  wind_speed_kt: number | null;
  qnh_hpa: number | null;
  outside_air_temp_c: number | null;
  total_air_temp_c: number | null;
  mach: number | null;
  /** Aircraft empty weight in kg. `null` on Asobo default airliners
   *  (their OEW is bogus — verified ~1422 kg for the A320neo). */
  empty_weight_kg: number | null;
  aircraft_title: string | null;
  aircraft_icao: string | null;
  aircraft_registration: string | null;
  simulator: Simulator;
  sim_version: string | null;
}

export type SimConnectionState = "disconnected" | "connecting" | "connected";

export type SimKind =
  | "off"
  | "msfs2020"
  | "msfs2024"
  | "xplane11"
  | "xplane12";

export interface SimStatus {
  state: SimConnectionState;
  kind: SimKind;
  snapshot: SimSnapshot | null;
  last_error: string | null;
  available: boolean;
}

export type FlightPhase =
  | "preflight"
  | "boarding"
  | "pushback"
  | "taxi_out"
  | "takeoff_roll"
  | "takeoff"
  | "climb"
  | "cruise"
  | "descent"
  | "approach"
  | "final"
  | "landing"
  | "taxi_in"
  | "blocks_on"
  | "arrived"
  | "pirep_submitted";

export interface ResumableFlight {
  pirep_id: string;
  flight_number: string;
  dpt_airport: string;
  arr_airport: string;
  status: string | null;
}

export interface ActiveFlightInfo {
  pirep_id: string;
  bid_id: number;
  started_at: string;
  /** ICAO of the operating airline, e.g. "DLH". Combined with
   *  `flight_number` to render the full callsign ("DLH155"). */
  airline_icao: string;
  /** Planned aircraft registration from phpVMS (e.g. "D-AIUV"). Empty
   *  when no matching bid / aircraft details could be looked up. */
  planned_registration: string;
  flight_number: string;
  dpt_airport: string;
  arr_airport: string;
  distance_nm: number;
  position_count: number;
  phase: FlightPhase;
  block_off_at: string | null;
  takeoff_at: string | null;
  landing_at: string | null;
  block_on_at: string | null;
  landing_rate_fpm: number | null;
  landing_g_force: number | null;
  was_just_resumed: boolean;
  /** Stand the aircraft pushed back from (MSFS ATC PARKING NAME). */
  dep_gate: string | null;
  /** Stand the pilot parked at after arrival. */
  arr_gate: string | null;
  /** Approach runway from `ATC RUNWAY SELECTED` at Final. */
  approach_runway: string | null;
  /** ISO-8601 UTC timestamp of the most recent successful
   *  position-post, or null if none has succeeded yet. */
  last_position_at: string | null;
  /** Positions sitting in the offline queue waiting to replay.
   *  0 means we're online and current. */
  queued_position_count: number;
}

export interface AirportInfo {
  icao: string;
  name: string | null;
  lat: number | null;
  lon: number | null;
}
