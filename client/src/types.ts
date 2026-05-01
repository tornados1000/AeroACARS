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
  groundspeed_kt: number;
  indicated_airspeed_kt: number;
  true_airspeed_kt: number;
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
  wind_direction_deg: number | null;
  wind_speed_kt: number | null;
  qnh_hpa: number | null;
  outside_air_temp_c: number | null;
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
