// Mapper LandingRecord → RunwayDiagramV2Props.
// Pilot-Client-Pfad: liest direkt aus LandingRecord (entweder lokal
// persistiert in landings.json oder live aus FlightStats). Webapp hat
// einen separaten Mapper aus TouchdownDto.payload — siehe Spec
// §Mapping aus TouchdownDto.payload.

import type {
  RunwayDiagramV2Props,
  AimClass,
  TchClass,
} from "../components/RunwayDiagramV2";
import type { LandingRecord } from "../components/LandingPanel";

const FT_TO_M = 0.3048;

export function mapLandingRecordToV2Props(
  record: LandingRecord,
): RunwayDiagramV2Props | null {
  const rw = record.runway_match;
  if (!rw) return null;

  const source = ((): "navigraph" | "ourairports_fallback" | null => {
    if (rw.source === "navigraph") return "navigraph";
    if (rw.source === "ourairports_fallback") return "ourairports_fallback";
    return null;
  })();

  const td_distance_from_threshold_m =
    record.td_distance_from_threshold_m ??
    rw.touchdown_distance_from_threshold_ft * FT_TO_M;

  return {
    airport_ident: rw.airport_ident,
    airport_name: null,
    runway_ident: rw.runway_ident,
    length_m: rw.length_ft * FT_TO_M,
    surface: rw.surface ?? null,
    source,
    nav_cycle: rw.nav_cycle ?? null,
    displaced_threshold_m: (rw.displaced_threshold_ft ?? 0) * FT_TO_M,
    td_distance_from_threshold_m,
    td_centerline_offset_m: rw.centerline_distance_m,
    td_in_tdz: record.td_in_tdz ?? null,
    td_third: (record.td_third ?? null) as 1 | 2 | 3 | null,
    td_tdz_length_m: record.td_tdz_length_m ?? null,
    aim_point_m: record.aim_point_m ?? null,
    aim_delta_m: record.aim_delta_m ?? null,
    aim_class: (record.aim_class ?? null) as AimClass | null,
    tch_actual_ft: record.tch_actual_ft ?? null,
    tch_expected_ft: rw.tch_expected_ft ?? null,
    tch_delta_ft: record.tch_delta_ft ?? null,
    tch_class: (record.tch_class ?? null) as TchClass | null,
    pre_displaced_threshold: record.pre_displaced_threshold ?? null,
    rollout_m: record.rollout_distance_m ?? null,
    // Aircraft-Daten für die Landeeinschätzung
    aircraft_icao: record.aircraft_icao ?? null,
    aircraft_title: record.aircraft_title ?? null,
    aircraft_registration: record.aircraft_registration ?? null,
    landing_weight_kg: record.landing_weight_kg ?? null,
    planned_ldw_kg: record.planned_ldw_kg ?? null,
    landing_speed_kt: record.landing_speed_kt ?? null,
    landing_pitch_deg: record.landing_pitch_deg ?? null,
    landing_bank_deg: record.landing_bank_deg ?? null,
    landing_peak_g_force: record.landing_peak_g_force ?? null,
    headwind_kt: record.headwind_kt ?? null,
    crosswind_kt: record.crosswind_kt ?? null,
    locale: "de",
  };
}
