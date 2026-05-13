//! Touchdown-quality assessment against authoritative runway data.
//!
//! Pure functions — no I/O, no globals. Inputs are the matched runway
//! plus a single touchdown sample; outputs are classification enums and
//! signed delta-meters/feet. The streamer-tick wires these into the
//! `LandingRecord` and the LandingPanel renders them as compliance pills.
//!
//! Spec: `docs/spec/v0.8.0-vps-navdata-runway-awareness.md`
//!  - F3 Touchdown-Zone (TDZ): ICAO Annex 14 / FAA 150/5340-1.
//!    TDZ length = min(900 m, runway_length / 3). Below 1200 m runway
//!    length the marking is not defined and we skip the feature.
//!  - F4 Aim-Point: FAA AIM 8-9-1. 400 m past threshold for runways
//!    ≥ 2400 m (= 7874 ft), 300 m for shorter runways.
//!  - F5 TCH-Compliance: comparison against `nav_runway.tch_ft`.
//!  - F6 Displaced-Threshold-Warning: touchdown in the painted-arrow
//!    pre-threshold zone is illegal in real ops.
//!  - F7 Wind-vs-Runway: classic vector decomposition against the
//!    runway's true course, NOT the magnetic course (the wind we get
//!    from the sim is also referenced to true north).
//!
//! All distances internal to this module are **meters from the
//! landing threshold along the centerline** — same sign convention as
//! `runway::lookup_runway` (positive = past threshold).
//!
//! Slice A only stages the pure functions + tests. Slice B wires them
//! into the streamer-tick + `record_landing_for_filed_flight`. Until
//! then the items here have no production caller — `#[allow(dead_code)]`
//! is intentional and **must be removed when Slice B lands** so any
//! later regression (unwired feature) shows up as a warning again.
#![allow(dead_code)]

/// FAA AIM aim-point switchover: at or above this length, the standard
/// aim-point shifts from 300 m to 400 m past the threshold.
const LONG_RUNWAY_FT: f64 = 7874.0; // = 2400 m

/// ICAO Annex 14: TDZ markings are only painted on runways ≥ 1200 m.
/// Below that there's no defined "touchdown zone" and we surface "n/a"
/// in the UI instead of inventing a number.
const TDZ_MIN_RUNWAY_M: f64 = 1200.0;

/// ICAO Annex 14: TDZ ends at min(900 m, length/3).
const TDZ_MAX_LENGTH_M: f64 = 900.0;

/// Aim-Point distances (FAA AIM 8-9-1).
const AIM_SHORT_M: f64 = 300.0;
const AIM_LONG_M: f64 = 400.0;

const FT_PER_M: f64 = 3.280_839_895;

// ─── F3: TDZ ─────────────────────────────────────────────────────────

/// Whether the touchdown landed inside the marked TDZ and which third
/// of the runway it sits in. `None` when the runway is too short to
/// have a defined TDZ.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TdzResult {
    /// True when 0 < td_distance_m ≤ tdz_length_m. Negative
    /// `td_distance_m` (undershoot) is reported as `false` — the pilot
    /// is on the approach side of the threshold, not in the TDZ.
    pub in_tdz: bool,
    /// 1-indexed third of the runway the touchdown lies in. 1 = first
    /// third (the good zone), 2 = middle, 3 = last. Undershoots = 1
    /// for simplicity (they're below the threshold but conceptually in
    /// the "near the start" bucket).
    pub third: u8,
    /// Length of the TDZ marker in meters (≤ 900 m, ≤ length/3).
    pub tdz_length_m: f64,
}

/// Classify a touchdown distance against the runway's TDZ markings.
///
/// `td_distance_m` is signed along-track from the landing threshold —
/// same convention as `runway::lookup_runway`. Returns `None` when the
/// runway is too short for ICAO TDZ markings to apply.
pub fn classify_tdz(td_distance_m: f64, runway_length_m: f64) -> Option<TdzResult> {
    if runway_length_m < TDZ_MIN_RUNWAY_M {
        return None;
    }
    let tdz_length_m = (runway_length_m / 3.0).min(TDZ_MAX_LENGTH_M);
    let in_tdz = td_distance_m > 0.0 && td_distance_m <= tdz_length_m;
    let third = if td_distance_m <= runway_length_m / 3.0 {
        1
    } else if td_distance_m <= (2.0 * runway_length_m) / 3.0 {
        2
    } else {
        3
    };
    Some(TdzResult {
        in_tdz,
        third,
        tdz_length_m,
    })
}

// ─── F4: Aim-Point ───────────────────────────────────────────────────

/// Aim-point classification buckets. Stable strings — the i18n keys
/// and the wire-payload field both consume these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AimClass {
    /// |delta| < 60 m — on or very close to the aim point.
    Perfect,
    /// delta in [-150, -60] m — touched down a bit early.
    ShortOfAim,
    /// delta in [60, 200] m — touched a bit past, still acceptable.
    PastAim,
    /// delta in [200, 500] m — long landing, rollout-distance concern.
    LongLanding,
    /// |delta| > 500 m (past) or delta < -150 m (short) — severe.
    Severe,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AimResult {
    /// Expected aim-point distance from threshold in meters
    /// (300 m short / 400 m long, per FAA AIM 8-9-1).
    pub aim_point_m: f64,
    /// Signed delta = `td_distance_m - aim_point_m`. Positive = past
    /// aim point, negative = short of aim point.
    pub delta_m: f64,
    pub class: AimClass,
}

/// Classify the touchdown distance against the standard aim point.
pub fn classify_aim(td_distance_m: f64, runway_length_m: f64) -> AimResult {
    let aim_point_m = if runway_length_m * FT_PER_M >= LONG_RUNWAY_FT {
        AIM_LONG_M
    } else {
        AIM_SHORT_M
    };
    let delta_m = td_distance_m - aim_point_m;
    let class = if delta_m.abs() < 60.0 {
        AimClass::Perfect
    } else if delta_m >= 60.0 && delta_m < 200.0 {
        AimClass::PastAim
    } else if delta_m <= -60.0 && delta_m >= -150.0 {
        AimClass::ShortOfAim
    } else if delta_m >= 200.0 && delta_m <= 500.0 {
        AimClass::LongLanding
    } else {
        AimClass::Severe
    };
    AimResult {
        aim_point_m,
        delta_m,
        class,
    }
}

// ─── F5: TCH-Compliance ──────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TchClass {
    /// |delta| ≤ 5 ft — on profile.
    OnProfile,
    /// delta in [-15, -5] ft — a hair low, fine for ILS Cat I.
    SlightlyLow,
    /// delta in [5, 20] ft — slightly high, expect longer float.
    SlightlyHigh,
    /// delta > 20 ft — long-landing risk material.
    High,
    /// delta < -15 ft — below profile, tail-strike / obstacle hazard.
    BelowProfile,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TchResult {
    pub actual_ft: f64,
    pub expected_ft: f64,
    pub delta_ft: f64,
    pub class: TchClass,
}

/// Classify the actual TCH measured at threshold-crossing against the
/// runway's published TCH. The actual_ft is provided by the caller —
/// this function does no sample-buffer arithmetic, just classification.
pub fn classify_tch(actual_ft: f64, expected_ft: f64) -> TchResult {
    let delta_ft = actual_ft - expected_ft;
    let class = if delta_ft.abs() <= 5.0 {
        TchClass::OnProfile
    } else if delta_ft > 5.0 && delta_ft <= 20.0 {
        TchClass::SlightlyHigh
    } else if delta_ft < -5.0 && delta_ft >= -15.0 {
        TchClass::SlightlyLow
    } else if delta_ft > 20.0 {
        TchClass::High
    } else {
        TchClass::BelowProfile
    };
    TchResult {
        actual_ft,
        expected_ft,
        delta_ft,
        class,
    }
}

// ─── F6: Displaced-Threshold-Warning ─────────────────────────────────

/// Did the pilot touch down inside the painted pre-threshold zone
/// (= before the displaced threshold)? `nav_runway.threshold` is
/// already the *displaced* threshold (= landing threshold) — touching
/// down at `td_distance_m < 0` AND in the DDS-painted-zone is the
/// illegal case.
///
/// Convention: `displaced_threshold_ft` is the distance from the
/// painted runway start to the landing threshold. A touchdown at
/// `td_distance_m = -X` means the pilot is X meters *before* the
/// landing threshold; if X < displaced_threshold_m, the touchdown
/// lands on the displaced-threshold paint = illegal.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DisplacedResult {
    /// True when the touchdown sits between the runway start and the
    /// landing threshold (= on the displaced-threshold paint).
    pub in_pre_threshold_zone: bool,
    pub displaced_threshold_m: f64,
}

pub fn classify_displaced(td_distance_m: f64, displaced_threshold_ft: f64) -> DisplacedResult {
    let displaced_threshold_m = displaced_threshold_ft / FT_PER_M;
    // Pilot below threshold (td_distance_m < 0) AND not so far below
    // that they undershot completely off the airport.
    let in_pre_threshold_zone =
        displaced_threshold_m > 0.0 && td_distance_m < 0.0 && td_distance_m > -displaced_threshold_m;
    DisplacedResult {
        in_pre_threshold_zone,
        displaced_threshold_m,
    }
}

// ─── F7: Wind-vs-Runway (exact) ──────────────────────────────────────

/// Wind decomposition relative to the runway centerline (using the
/// runway's *true* course, not magnetic).
///
/// Sign conventions:
///   * `headwind_kt > 0` → wind blowing into the aircraft's face
///     (= classic „good" headwind on approach).
///   * `crosswind_kt > 0` → wind from the right (= aircraft would
///     drift left without correction).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WindResult {
    pub headwind_kt: f64,
    pub crosswind_kt: f64,
}

/// Compute headwind/crosswind for a runway from a meteorological wind
/// vector. `wind_dir_true_deg` is the *from* direction (METAR
/// convention: 270° = wind blowing from the west toward the east).
/// `runway_true_course_deg` is the direction the aircraft is rolling
/// in (= threshold-to-end bearing).
pub fn classify_wind(
    wind_speed_kt: f64,
    wind_dir_true_deg: f64,
    runway_true_course_deg: f64,
) -> WindResult {
    let mut diff = (wind_dir_true_deg - runway_true_course_deg).to_radians();
    // Normalise to (-π, π] so the trig comes out signed.
    while diff > std::f64::consts::PI {
        diff -= 2.0 * std::f64::consts::PI;
    }
    while diff <= -std::f64::consts::PI {
        diff += 2.0 * std::f64::consts::PI;
    }
    // METAR wind is the from-direction. A wind FROM the runway heading
    // is straight in your face → max headwind. cos(0) = 1.
    let headwind_kt = wind_speed_kt * diff.cos();
    // Wind FROM the right of the landing direction → crosswind +.
    // RWY 360° + wind FROM 090° (east, = right) → diff = +90° (after
    // normalisation), sin(+90°) = +1 → crosswind = +10. That matches
    // the pilot convention "crosswind from the right".
    let crosswind_kt = wind_speed_kt * diff.sin();
    WindResult {
        headwind_kt,
        crosswind_kt,
    }
}

// ─── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// MS713 anchor: OLBA RWY 17, 3250 m runway. Touchdown 320 m past
    /// threshold (the example used throughout the spec).
    const MS713_RUNWAY_M: f64 = 3250.0;
    const MS713_TD_M: f64 = 320.0;

    #[test]
    fn tdz_ms713_anchor() {
        let r = classify_tdz(MS713_TD_M, MS713_RUNWAY_M).expect("MS713 RWY > 1200 m");
        assert!(r.in_tdz, "320 m past threshold on 3250 m RWY → in TDZ");
        assert_eq!(r.third, 1);
        // TDZ = min(900, 3250/3) = min(900, 1083) = 900.
        assert!((r.tdz_length_m - 900.0).abs() < 0.01);
    }

    #[test]
    fn tdz_long_landing_third_two() {
        // 1430 m past threshold on 3250 m → outside 900 m TDZ, second third.
        let r = classify_tdz(1430.0, 3250.0).unwrap();
        assert!(!r.in_tdz);
        assert_eq!(r.third, 2);
    }

    #[test]
    fn tdz_skipped_short_runway() {
        // 1000 m runway — too short for TDZ markings.
        assert!(classify_tdz(200.0, 1000.0).is_none());
    }

    #[test]
    fn tdz_undershoot_not_in_tdz() {
        let r = classify_tdz(-12.0, 3250.0).unwrap();
        assert!(!r.in_tdz, "undershoot is on the approach side, not TDZ");
        assert_eq!(r.third, 1);
    }

    #[test]
    fn tdz_length_caps_at_900m_for_short_runway() {
        // 2400 m → length/3 = 800, cap doesn't bind, expect 800.
        let r = classify_tdz(500.0, 2400.0).unwrap();
        assert!((r.tdz_length_m - 800.0).abs() < 0.01);
        // 5000 m → length/3 = 1666, cap binds at 900.
        let r = classify_tdz(500.0, 5000.0).unwrap();
        assert!((r.tdz_length_m - 900.0).abs() < 0.01);
    }

    #[test]
    fn aim_ms713_perfect_within_60m() {
        // 320 m TD vs 400 m aim (3250 m > 2400 m threshold) → delta -80 m.
        // Actually wait — 3250 m × 3.28 = 10663 ft > 7874 ft → long runway,
        // aim = 400 m. Delta = 320 - 400 = -80 → outside ±60 → ShortOfAim.
        let r = classify_aim(MS713_TD_M, MS713_RUNWAY_M);
        assert!((r.aim_point_m - 400.0).abs() < 0.01, "long RWY → 400 m aim");
        assert!((r.delta_m - (-80.0)).abs() < 0.01);
        assert_eq!(r.class, AimClass::ShortOfAim);
    }

    #[test]
    fn aim_short_runway_uses_300m() {
        // 1500 m × 3.28 = 4920 ft < 7874 ft → short, aim = 300 m.
        // TD 320 m → delta +20 m → Perfect.
        let r = classify_aim(320.0, 1500.0);
        assert!((r.aim_point_m - 300.0).abs() < 0.01);
        assert!((r.delta_m - 20.0).abs() < 0.01);
        assert_eq!(r.class, AimClass::Perfect);
    }

    #[test]
    fn aim_long_landing_warn() {
        // TD 800 m, RWY 3250 m → aim 400 m, delta +400 m → LongLanding.
        let r = classify_aim(800.0, 3250.0);
        assert_eq!(r.class, AimClass::LongLanding);
    }

    #[test]
    fn aim_severe_far_past() {
        let r = classify_aim(1200.0, 3250.0);
        assert_eq!(r.class, AimClass::Severe);
    }

    #[test]
    fn aim_severe_far_short() {
        // delta = -250 → Severe (below -150 m threshold).
        let r = classify_aim(150.0, 3250.0);
        assert_eq!(r.class, AimClass::Severe);
    }

    #[test]
    fn tch_on_profile() {
        let r = classify_tch(47.0, 49.0);
        assert_eq!(r.class, TchClass::OnProfile);
        assert!((r.delta_ft - (-2.0)).abs() < 0.01);
    }

    #[test]
    fn tch_slightly_low() {
        let r = classify_tch(40.0, 50.0);
        assert_eq!(r.class, TchClass::SlightlyLow);
    }

    #[test]
    fn tch_slightly_high() {
        let r = classify_tch(62.0, 50.0);
        assert_eq!(r.class, TchClass::SlightlyHigh);
    }

    #[test]
    fn tch_high_warn() {
        let r = classify_tch(75.0, 50.0);
        assert_eq!(r.class, TchClass::High);
    }

    #[test]
    fn tch_below_profile_dangerous() {
        let r = classify_tch(28.0, 50.0);
        assert_eq!(r.class, TchClass::BelowProfile);
    }

    #[test]
    fn displaced_olba_rwy35_anchor() {
        // OLBA RWY 35 has DDS = 2690 ft (820 m). Pilot touched down
        // 200 m past *landing* threshold → td_distance_m = +200 →
        // NOT in pre-threshold zone (well past it).
        let r = classify_displaced(200.0, 2690.0);
        assert!(!r.in_pre_threshold_zone);
        assert!((r.displaced_threshold_m - 819.91).abs() < 0.5);
    }

    #[test]
    fn displaced_touchdown_on_dds_paint_illegal() {
        // Pilot touched down 80 m BEFORE the landing threshold on a
        // runway with 820 m DDS — illegal real-world landing on the
        // painted displaced-threshold arrows.
        let r = classify_displaced(-80.0, 2690.0);
        assert!(r.in_pre_threshold_zone);
    }

    #[test]
    fn displaced_no_dds_never_triggers() {
        // OLBA RWY 17 has DDS = 0 — undershoot is undershoot, no DDS warning.
        let r = classify_displaced(-50.0, 0.0);
        assert!(!r.in_pre_threshold_zone);
    }

    #[test]
    fn displaced_undershoot_past_dds_is_off_field() {
        // Pilot 900 m before threshold, DDS only 820 m → undershot
        // past even the painted runway start = off-airport. Don't
        // flag DDS for that — that's a different (worse) issue.
        let r = classify_displaced(-900.0, 2690.0);
        assert!(!r.in_pre_threshold_zone);
    }

    #[test]
    fn wind_straight_headwind() {
        // RWY 17 (true 176.94°), wind FROM 177° at 12 kt → ~12 kt
        // headwind, ~0 kt crosswind.
        let r = classify_wind(12.0, 177.0, 176.94);
        assert!((r.headwind_kt - 12.0).abs() < 0.01);
        assert!(r.crosswind_kt.abs() < 0.05);
    }

    #[test]
    fn wind_pure_tailwind() {
        // Wind from opposite direction (357° from RWY 177°) → -12 kt
        // headwind (tailwind), 0 kt crosswind.
        let r = classify_wind(12.0, 357.0, 177.0);
        assert!((r.headwind_kt - (-12.0)).abs() < 0.05);
        assert!(r.crosswind_kt.abs() < 0.05);
    }

    #[test]
    fn wind_pure_crosswind_from_right() {
        // RWY 360° (= north), wind from 90° (east) at 10 kt → wind comes
        // from the right of the landing direction → crosswind +10 kt
        // (right-cross), headwind 0.
        let r = classify_wind(10.0, 90.0, 360.0);
        assert!(r.headwind_kt.abs() < 0.05);
        assert!(
            (r.crosswind_kt - 10.0).abs() < 0.05,
            "got cx={}",
            r.crosswind_kt
        );
    }

    #[test]
    fn wind_pure_crosswind_from_left() {
        // RWY 360°, wind from 270° (west) → from the left → crosswind -10 kt.
        let r = classify_wind(10.0, 270.0, 360.0);
        assert!(r.headwind_kt.abs() < 0.05);
        assert!(
            (r.crosswind_kt - (-10.0)).abs() < 0.05,
            "got cx={}",
            r.crosswind_kt
        );
    }

    #[test]
    fn wind_45deg_split() {
        // RWY 360°, wind FROM 045° at 14.14 kt → headwind 10, crosswind +10.
        let r = classify_wind(14.142_135_6, 45.0, 360.0);
        assert!((r.headwind_kt - 10.0).abs() < 0.05);
        assert!((r.crosswind_kt - 10.0).abs() < 0.05);
    }
}
