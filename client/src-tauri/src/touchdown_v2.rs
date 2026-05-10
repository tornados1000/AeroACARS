//! Touchdown-Forensik v2 — Layer 1 (Detection) + Layer 2 (Validation) +
//! Layer 3 (VS-Calculation am impact_frame).
//!
//! Spec: docs/spec/touchdown-forensics-v2.md (v2.3, approved)
//!
//! **Designed pure** (sim-agnostisch, ohne Side-Effects auf Tauri-State)
//! damit die Logik 1:1 gegen historische JSONL-Replays gepruft werden kann.
//! Layer 4 (LandingEpisode-Aggregation + Lifecycle) sitzt ausserhalb dieses
//! Moduls weil sie Tauri-State + Persistierung braucht.
//!
//! **Sim-Trennung** ist STRUKTURELL unvermeidbar weil X-Plane das wichtigste
//! Validation-Signal hat (gear_normal_force_n) und MSFS nicht. Siehe Spec
//! Sektion 4.

use chrono::{DateTime, Duration, Utc};
use recorder::TouchdownWindowSample;
use serde::{Deserialize, Serialize};

use crate::SimKind;

/// **forensics_version=2** Marker fuer Events + PIREP-payload.
/// Recorder/aeroacars-live identifiziert via diesem Wert welche
/// Auswertungs-Logik zu verwenden ist.
pub const FORENSICS_VERSION: u8 = 2;

// ─── Layer 1: TD-Candidate Detection ──────────────────────────────────────

/// Ein Sample-Pair fuer Edge-Detection (prev → current).
/// Liefert Some(TdCandidate) wenn ein Edge-Trigger detected wurde.
///
/// X-Plane: Edge wenn prev.in_air UND (current.on_ground ODER gear_force > epsilon)
/// MSFS:    Edge wenn prev.in_air UND current.on_ground
///
/// `prev.in_air` = !prev.on_ground UND (prev.gear_force.unwrap_or(0) <= epsilon)
pub fn detect_td_candidate(
    prev: Option<&TouchdownWindowSample>,
    current: &TouchdownWindowSample,
    current_idx: usize,
    sim: SimKind,
) -> Option<TdCandidate> {
    const GEAR_FORCE_EPSILON_N: f32 = 1.0;

    let prev = prev?;
    let prev_in_air = !prev.on_ground
        && prev
            .gear_normal_force_n
            .map(|f| f <= GEAR_FORCE_EPSILON_N)
            .unwrap_or(true);
    if !prev_in_air {
        return None;
    }

    let edge_now = if sim.is_xplane() {
        current.on_ground
            || current
                .gear_normal_force_n
                .map(|f| f > GEAR_FORCE_EPSILON_N)
                .unwrap_or(false)
    } else {
        // MSFS / Off — only on_ground edge (kein gear_normal_force_n verfuegbar)
        current.on_ground
    };

    if !edge_now {
        return None;
    }

    Some(TdCandidate {
        edge_sample_index: current_idx,
        edge_at: current.at,
        edge_agl_ft: current.agl_ft,
        edge_vs_fpm: current.vs_fpm,
        edge_gear_force_n: current.gear_normal_force_n,
        edge_g_force: current.g_force,
        edge_total_weight_kg: current.total_weight_kg,
    })
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TdCandidate {
    pub edge_sample_index: usize,
    pub edge_at: DateTime<Utc>,
    pub edge_agl_ft: f32,
    pub edge_vs_fpm: f32,
    pub edge_gear_force_n: Option<f32>,
    pub edge_g_force: f32,
    pub edge_total_weight_kg: Option<f32>,
}

// ─── Layer 2: TD-Validation (sim-spezifisch) ──────────────────────────────

/// Ergebnis der Validation. Bei VALIDATED wird die TD als „echter contact"
/// behandelt; bei FALSE_EDGE ist es ein Streifschuss/Float.
#[derive(Debug, Clone)]
pub enum ValidationResult {
    Validated {
        result: ValidationDetail,
    },
    FalseEdge {
        reason: FalseEdgeReason,
        result: ValidationDetail,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationDetail {
    pub sim: SimKind,
    pub gear_force_threshold_n: f32,
    pub gear_force_pass: bool,
    pub gear_force_peak_in_window_n: Option<f32>,
    pub gear_force_sustained_ms: Option<u64>,
    pub g_force_pass: Option<bool>,
    pub g_force_peak_in_window: f32,
    pub low_agl_persistence_pass: bool,
    pub low_agl_actual_ms: u64,
    pub sustained_ground_pass: Option<bool>,
    pub sustained_ground_actual_ms: u64,
    pub vs_negative_pass: bool,
    pub vs_at_impact_used_for_test: f32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum FalseEdgeReason {
    /// X-Plane: gear_force unter threshold (oder nicht lange genug)
    GearForceBelowThreshold,
    /// MSFS: weniger als 3 von 4 Tests passed
    InsufficientVoteScore,
}

/// Mass-aware gear-force threshold:
/// max(1000 N floor, 3% × total_weight × 9.80665).
///
/// 1000 N als hartes Minimum schuetzt vor zu strikt-strict bei Glidern/Ultralight.
/// 3% × static weight skaliert mit Aircraft-Mass — A330 (250t) ergibt ~73.5 kN,
/// Cessna 152 (757kg) ergibt ~222 N → floor wins → 1000 N.
pub fn gear_force_threshold_n(total_weight_kg: Option<f32>) -> f32 {
    const ABS_FLOOR: f32 = 1000.0;
    const MASS_RATIO: f32 = 0.03;
    const G: f32 = 9.80665;

    let dynamic = total_weight_kg
        .filter(|w| *w > 100.0)
        .map(|w| w * G * MASS_RATIO)
        .unwrap_or(ABS_FLOOR);
    dynamic.max(ABS_FLOOR)
}

/// Validate eine TdCandidate gegen die sim-spezifischen Tests.
///
/// X-Plane: gear_force ist MUST-PASS (Anchor). A1 FAIL → Validation FAIL.
/// MSFS:    weiches Voting (3 von 4 Tests muessen PASS).
///
/// Time-windows werden via `at`-Timestamps gemessen, NICHT via Sample-Count
/// (Sampler ist nicht garantiert genau 50 Hz).
pub fn validate_candidate(
    candidate: &TdCandidate,
    samples: &[TouchdownWindowSample],
    sim: SimKind,
    impact_frame_vs: f32,
) -> ValidationResult {
    let edge_at = candidate.edge_at;
    let threshold_n = gear_force_threshold_n(candidate.edge_total_weight_kg);

    // Test: gear_force-impact (X-Plane only — MUST-PASS)
    let (gear_force_pass, gear_force_peak, gear_force_sustained_ms) =
        evaluate_gear_force_test(samples, edge_at, threshold_n);

    // Test: g_force-spike (MSFS-relevant)
    let g_force_peak = evaluate_g_force_peak(samples, edge_at);
    let g_force_pass = g_force_peak > 1.05;

    // Test: low_agl_persistence (beide Sims)
    let (low_agl_pass, low_agl_ms) = evaluate_low_agl_persistence(samples, edge_at);

    // Test: sustained_ground_contact (MSFS-relevant)
    let (sustained_pass, sustained_ms) = evaluate_sustained_ground(samples, edge_at);

    // Test: vs_negative_at_impact (beide)
    let vs_negative_pass = impact_frame_vs < -10.0;

    let detail = ValidationDetail {
        sim,
        gear_force_threshold_n: threshold_n,
        gear_force_pass,
        gear_force_peak_in_window_n: gear_force_peak,
        gear_force_sustained_ms,
        g_force_pass: Some(g_force_pass),
        g_force_peak_in_window: g_force_peak,
        low_agl_persistence_pass: low_agl_pass,
        low_agl_actual_ms: low_agl_ms,
        sustained_ground_pass: Some(sustained_pass),
        sustained_ground_actual_ms: sustained_ms,
        vs_negative_pass,
        vs_at_impact_used_for_test: impact_frame_vs,
    };

    if sim.is_xplane() {
        // Sonderfall: Wenn der Sample-Buffer KEIN gear_force enthaelt
        // (= alle samples haben gear_normal_force_n = None — passiert bei
        // legacy JSONLs vor v0.7.0 ODER wenn ein X-Plane-Addon das DataRef
        // nicht setzt), fallback auf MSFS-style Voting.
        let any_gear_force_data = samples
            .iter()
            .any(|s| s.gear_normal_force_n.is_some());

        if any_gear_force_data {
            // Echte X-Plane-Validation mit gear_force als MUST-PASS
            if !gear_force_pass {
                return ValidationResult::FalseEdge {
                    reason: FalseEdgeReason::GearForceBelowThreshold,
                    result: detail,
                };
            }
            if !low_agl_pass || !vs_negative_pass {
                return ValidationResult::FalseEdge {
                    reason: FalseEdgeReason::GearForceBelowThreshold,
                    result: detail,
                };
            }
            return ValidationResult::Validated { result: detail };
        }
        // Fallback ohne gear_force: 4-of-4 Voting (= strenger als MSFS
        // damit X-Plane edge-trigger-happy-Streifschuesse nicht durch).
        // Plus: agl_persistence ist hier kritisch weil g_force-spike bei
        // X-Plane ohne gear_force evtl unzuverlaessig
        let passes = [g_force_pass, sustained_pass, low_agl_pass, vs_negative_pass]
            .iter()
            .filter(|p| **p)
            .count();
        if passes >= 4 {
            ValidationResult::Validated { result: detail }
        } else {
            ValidationResult::FalseEdge {
                reason: FalseEdgeReason::InsufficientVoteScore,
                result: detail,
            }
        }
    } else {
        // MSFS / Off: 4 Tests, mind. 3 PASS = VALIDATED.
        let passes = [g_force_pass, sustained_pass, low_agl_pass, vs_negative_pass]
            .iter()
            .filter(|p| **p)
            .count();
        if passes >= 3 {
            ValidationResult::Validated { result: detail }
        } else {
            ValidationResult::FalseEdge {
                reason: FalseEdgeReason::InsufficientVoteScore,
                result: detail,
            }
        }
    }
}

/// gear_force-impact Evaluation (X-Plane).
/// Returns (pass, peak_in_window, sustained_ms_above_threshold).
///
/// Confirmation-Window: Force ueber threshold fuer mind. 60ms anhaltend
/// (gemessen via Timestamps), mit mind. 2 distinct samples (Anti-Glitch).
fn evaluate_gear_force_test(
    samples: &[TouchdownWindowSample],
    edge_at: DateTime<Utc>,
    threshold_n: f32,
) -> (bool, Option<f32>, Option<u64>) {
    let window_end = edge_at + Duration::milliseconds(500);

    // Sammle alle Samples im Window mit gear_force >= threshold
    let above: Vec<&TouchdownWindowSample> = samples
        .iter()
        .filter(|s| s.at >= edge_at && s.at <= window_end)
        .filter(|s| {
            s.gear_normal_force_n
                .map(|f| f.is_finite() && f >= threshold_n)
                .unwrap_or(false)
        })
        .collect();

    let peak_in_window = samples
        .iter()
        .filter(|s| s.at >= edge_at && s.at <= window_end)
        .filter_map(|s| s.gear_normal_force_n)
        .filter(|f| f.is_finite())
        .fold(None::<f32>, |acc, f| {
            Some(acc.map(|a| a.max(f)).unwrap_or(f))
        });

    if above.len() < 2 {
        return (false, peak_in_window, Some(0));
    }

    // Continuous duration (Timestamps): max gap zwischen consecutive samples
    // muss klein bleiben damit "sustained" ehrlich gemeint ist.
    // Hier vereinfacht: span vom ersten bis letzten above-sample.
    let first = above.first().unwrap();
    let last = above.last().unwrap();
    let sustained_ms = (last.at - first.at).num_milliseconds().max(0) as u64;

    (sustained_ms >= 60 && above.len() >= 2, peak_in_window, Some(sustained_ms))
}

/// peak g_force im Window [edge_at, edge_at + 500ms]
fn evaluate_g_force_peak(samples: &[TouchdownWindowSample], edge_at: DateTime<Utc>) -> f32 {
    let window_end = edge_at + Duration::milliseconds(500);
    samples
        .iter()
        .filter(|s| s.at >= edge_at && s.at <= window_end)
        .map(|s| s.g_force)
        .filter(|g| g.is_finite())
        .fold(0.0_f32, f32::max)
}

/// low_agl_persistence: agl_ft < 5.0 fuer mind. 1000ms ab edge_at.
/// Returns (pass, actual_ms_below_5ft).
fn evaluate_low_agl_persistence(
    samples: &[TouchdownWindowSample],
    edge_at: DateTime<Utc>,
) -> (bool, u64) {
    let target_dur = Duration::milliseconds(1000);
    let window_end = edge_at + target_dur;

    // Suche erste violation (agl >= 5.0) im Target-Window
    let mut first_violation_at: Option<DateTime<Utc>> = None;
    for s in samples.iter().filter(|s| s.at >= edge_at && s.at <= window_end) {
        if s.agl_ft >= 5.0 {
            first_violation_at = Some(s.at);
            break;
        }
    }

    match first_violation_at {
        None => {
            // Keine violation im Window — PASS
            (true, target_dur.num_milliseconds() as u64)
        }
        Some(t) => {
            let actual_ms = (t - edge_at).num_milliseconds().max(0) as u64;
            (actual_ms >= 1000, actual_ms)
        }
    }
}

/// sustained_ground_contact: on_ground=True fuer mind. 500ms continuous.
/// Returns (pass, actual_continuous_ms).
fn evaluate_sustained_ground(
    samples: &[TouchdownWindowSample],
    edge_at: DateTime<Utc>,
) -> (bool, u64) {
    let mut last_at = edge_at;
    let mut found_break = false;

    for s in samples.iter().filter(|s| s.at >= edge_at) {
        if !s.on_ground {
            found_break = true;
            break;
        }
        last_at = s.at;
    }

    let dur_ms = (last_at - edge_at).num_milliseconds().max(0) as u64;
    let pass = !found_break || dur_ms >= 500;
    (pass, dur_ms)
}

// ─── Layer 3: VS-Calculation am IMPACT-Frame ──────────────────────────────

/// Drei Frames die wir aus dem Buffer extrahieren (siehe Spec 5.1):
///   contact_frame:      = candidate.edge_sample_index (von Layer 1)
///   impact_frame:       min vs in [contact-250ms, contact+100ms] = raw härteste Sink
///   initial_load_peak:  max gear_force/g_force in [contact, contact+500ms]
///   episode_load_peak:  max in ganzer Episode (kommt von Layer 4, nicht hier)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpactFrameResult {
    pub contact_at: DateTime<Utc>,
    pub impact_at: DateTime<Utc>,
    pub impact_vs_fpm: f32,
    pub initial_load_peak_n: Option<f32>,   // X-Plane
    pub initial_load_peak_g: f32,           // beide
}

/// Berechne impact_frame + initial_load_peak aus dem Buffer um den contact_frame.
/// NaN-safe: nur finite vs_fpm Samples + total_cmp().
pub fn compute_impact_frame(
    samples: &[TouchdownWindowSample],
    contact_at: DateTime<Utc>,
) -> Option<ImpactFrameResult> {
    // impact_frame = min vs in [contact-250ms, contact+100ms]
    let window_start = contact_at - Duration::milliseconds(250);
    let window_end = contact_at + Duration::milliseconds(100);

    let impact_sample = samples
        .iter()
        .filter(|s| s.at >= window_start && s.at <= window_end && s.vs_fpm.is_finite())
        .min_by(|a, b| a.vs_fpm.total_cmp(&b.vs_fpm))?;

    // initial_load_peak: max in [contact, contact+500ms]
    let load_window_end = contact_at + Duration::milliseconds(500);
    let load_window: Vec<&TouchdownWindowSample> = samples
        .iter()
        .filter(|s| s.at >= contact_at && s.at <= load_window_end)
        .collect();

    let initial_load_peak_n = load_window
        .iter()
        .filter_map(|s| s.gear_normal_force_n)
        .filter(|f| f.is_finite())
        .fold(None::<f32>, |acc, f| {
            Some(acc.map(|a| a.max(f)).unwrap_or(f))
        });

    let initial_load_peak_g = load_window
        .iter()
        .map(|s| s.g_force)
        .filter(|g| g.is_finite())
        .fold(0.0_f32, f32::max);

    Some(ImpactFrameResult {
        contact_at,
        impact_at: impact_sample.at,
        impact_vs_fpm: impact_sample.vs_fpm,
        initial_load_peak_n,
        initial_load_peak_g,
    })
}

// ─── Layer 3 Cont: VS-Cascade + HARD GUARDS ───────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Confidence {
    High,
    Medium,
    Low,
    VeryLow,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LandingRateResult {
    pub vs_fpm: f32,
    pub source: String,
    pub confidence: Confidence,
    pub forensics_version: u8,
    pub contact_at: DateTime<Utc>,
    pub impact_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RejectionReason {
    EmptyWindow,
    AllSourcesPositive,
    PositiveVs,
    ImplausiblyHigh,
}

/// HARD GUARD: niemals positiv, niemals < -3000 fpm
fn finalize_vs(candidate_fpm: f32) -> Result<f32, RejectionReason> {
    if !candidate_fpm.is_finite() {
        return Err(RejectionReason::EmptyWindow);
    }
    if candidate_fpm > 0.0 {
        return Err(RejectionReason::PositiveVs);
    }
    if candidate_fpm < -3000.0 {
        return Err(RejectionReason::ImplausiblyHigh);
    }
    Ok(candidate_fpm)
}

/// Compute the final landing rate using the sim-agnostic cascade.
/// Cascade priority: vs_at_impact → smoothed_500ms → smoothed_1000ms →
/// pre_flare_peak → REJECT.
pub fn compute_landing_rate(
    samples: &[TouchdownWindowSample],
    impact_result: &ImpactFrameResult,
) -> Result<LandingRateResult, RejectionReason> {
    let impact_at = impact_result.impact_at;
    let vs_at_impact = impact_result.impact_vs_fpm;

    // Smoothed averages around impact_frame
    let vs_smoothed_500 = avg_vs_in_window(samples, impact_at, -500, 0);
    let vs_smoothed_1000 = avg_vs_in_window(samples, impact_at, -1000, 0);
    let pre_flare_peak = min_vs_in_window(samples, impact_at, -3000, -500);

    let chosen = if vs_at_impact < -10.0 {
        (vs_at_impact, "vs_at_impact_frame", Confidence::High)
    } else if vs_smoothed_500.map(|v| v < -10.0).unwrap_or(false) {
        (
            vs_smoothed_500.unwrap(),
            "vs_smoothed_500ms_at_impact",
            Confidence::Medium,
        )
    } else if vs_smoothed_1000.map(|v| v < -10.0).unwrap_or(false) {
        (
            vs_smoothed_1000.unwrap(),
            "vs_smoothed_1000ms_at_impact",
            Confidence::Low,
        )
    } else if pre_flare_peak.map(|v| v < 0.0).unwrap_or(false) {
        (
            pre_flare_peak.unwrap(),
            "pre_flare_peak",
            Confidence::VeryLow,
        )
    } else {
        return Err(RejectionReason::AllSourcesPositive);
    };

    let final_vs = finalize_vs(chosen.0)?;

    Ok(LandingRateResult {
        vs_fpm: final_vs,
        source: chosen.1.to_string(),
        confidence: chosen.2,
        forensics_version: FORENSICS_VERSION,
        contact_at: impact_result.contact_at,
        impact_at,
    })
}

// ─── Layer 4: LandingEpisode + Episode-Klassifizierung ────────────────────

/// Snapshot des Aircraft-Zustands zum Zeitpunkt eines TD-Contacts.
/// Wird einmal pro Episode beim ersten validated contact festgehalten —
/// wird fuer mass-aware threshold + deterministische Replays gebraucht.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AircraftStateSnapshot {
    pub total_weight_kg: Option<f32>,
    pub sim: SimKind,
}

/// Eine TD-Candidate die Validation gefailt hat (Float/Streifschuss).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FalseEdge {
    pub edge_at: DateTime<Utc>,
    pub edge_agl_ft: f32,
    pub edge_vs_fpm: f32,
    pub reason: FalseEdgeReason,
    pub validation: ValidationDetail,
}

/// Echter erster Bodenkontakt einer Episode (validated).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContactDetail {
    pub contact_at: DateTime<Utc>,
    pub impact_at: DateTime<Utc>,
    pub vs_at_impact_fpm: f32,
    pub vs_at_contact_fpm: f32,
    pub agl_at_contact_ft: f32,
    pub validation: ValidationDetail,
    pub initial_load_peak_n: Option<f32>,
    pub initial_load_peak_g: f32,
    pub confidence: Confidence,
    pub source: String,
}

/// Ein nachfolgender low-level Touch innerhalb derselben Episode
/// (= aircraft bleibt unter 50ft AGL, kein climb-out).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LowLevelTouch {
    pub at: DateTime<Utc>,
    pub vs_at_impact_fpm: f32,
    pub agl_max_ft: f32,
    pub sustained_ms: u64,
}

/// Wann + wie eine Episode in den finalen Settle uebergeht.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettleDetail {
    pub settle_at: DateTime<Utc>,
    pub final_groundspeed_kt: f32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum HardestImpactSource {
    Contact,
    LowLevelTouch(u8),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EpisodeClass {
    /// aircraft blieb am Boden, gs sinkt — Pilot ist gelandet
    FinalLanding,
    /// aircraft hob nach Touch wieder ab, stieg auf 100-1000ft AGL,
    /// kam zurueck — Pattern-Flug
    TouchAndGo,
    /// aircraft stieg > 1000ft AGL nach Touch — Go-Around
    GoAround,
    /// noch nicht klassifiziert (Episode laeuft noch)
    Pending,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LandingEpisode {
    pub episode_index: u8,
    pub aircraft_state_at_contact: AircraftStateSnapshot,
    pub false_edges: Vec<FalseEdge>,
    pub contact: ContactDetail,
    pub low_level_touches: Vec<LowLevelTouch>,
    pub settle: Option<SettleDetail>,
    /// max gear_force / g_force innerhalb der GANZEN Episode (incl. rollout) —
    /// Forensik-only, NICHT fuer Score
    pub episode_load_peak_n: Option<f32>,
    pub episode_load_peak_g: f32,
    pub hardest_impact_vs_fpm: f32,
    pub hardest_impact_source: HardestImpactSource,
    pub classification: EpisodeClass,
}

/// Kontext fuer Episode-Klassifizierung — Werte kommen aus der laufenden
/// Sampler-State-Machine (Layer 4 caller).
#[derive(Debug, Clone, Copy)]
pub struct EpisodePostContactState {
    /// Maximum AGL nach contact (in der ganzen post-contact Periode).
    pub max_agl_ft_after_contact: f32,
    /// Hat aircraft fuer >= 30s unter 50ft AGL geblieben mit gs<30kt?
    pub settled_under_50ft_for_30s: bool,
    /// Aktuelle groundspeed (fuer Settle-Detection)
    pub current_gs_kt: f32,
}

/// Klassifiziere eine Episode basierend auf was nach dem contact passiert ist.
/// Spec Sektion 6.2 (FinalLanding) + 6.4 (DAH 3181 Beispiel).
pub fn classify_episode(state: EpisodePostContactState) -> EpisodeClass {
    if state.max_agl_ft_after_contact > 1000.0 {
        EpisodeClass::GoAround
    } else if state.max_agl_ft_after_contact > 100.0 {
        EpisodeClass::TouchAndGo
    } else if state.settled_under_50ft_for_30s {
        EpisodeClass::FinalLanding
    } else {
        EpisodeClass::Pending
    }
}

/// Bestimme den haertesten Impact innerhalb einer Episode (= Bounce-Score-Regel).
/// Spec Sektion 6.5: härtester Impact = min vs_at_impact (= numerisch kleinster
/// = haertester Sink) zwischen contact + allen low_level_touches.
pub fn compute_hardest_impact(
    contact_vs: f32,
    low_level_touches: &[LowLevelTouch],
) -> (f32, HardestImpactSource) {
    let mut hardest = contact_vs;
    let mut source = HardestImpactSource::Contact;

    for (i, touch) in low_level_touches.iter().enumerate() {
        if touch.vs_at_impact_fpm < hardest {
            hardest = touch.vs_at_impact_fpm;
            source = HardestImpactSource::LowLevelTouch(i as u8);
        }
    }

    (hardest, source)
}

// ─── Helpers ──────────────────────────────────────────────────────────────

fn avg_vs_in_window(
    samples: &[TouchdownWindowSample],
    center: DateTime<Utc>,
    delta_start_ms: i64,
    delta_end_ms: i64,
) -> Option<f32> {
    let window_start = center + Duration::milliseconds(delta_start_ms);
    let window_end = center + Duration::milliseconds(delta_end_ms);

    let values: Vec<f32> = samples
        .iter()
        .filter(|s| s.at >= window_start && s.at <= window_end && s.vs_fpm.is_finite())
        .map(|s| s.vs_fpm)
        .collect();

    if values.is_empty() {
        None
    } else {
        Some(values.iter().sum::<f32>() / values.len() as f32)
    }
}

fn min_vs_in_window(
    samples: &[TouchdownWindowSample],
    center: DateTime<Utc>,
    delta_start_ms: i64,
    delta_end_ms: i64,
) -> Option<f32> {
    let window_start = center + Duration::milliseconds(delta_start_ms);
    let window_end = center + Duration::milliseconds(delta_end_ms);

    samples
        .iter()
        .filter(|s| s.at >= window_start && s.at <= window_end && s.vs_fpm.is_finite())
        .map(|s| s.vs_fpm)
        .min_by(|a, b| a.total_cmp(b))
}

// ─── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gear_force_threshold_floor_for_glider() {
        // Cessna 152 weight 757kg → dynamic = 757*9.80665*0.03 = 222.7N → floor wins
        let t = gear_force_threshold_n(Some(757.0));
        assert_eq!(t, 1000.0);
    }

    #[test]
    fn gear_force_threshold_dynamic_for_a330() {
        // DAH 3181 A330 250t → 250000*9.80665*0.03 = ~73550N
        let t = gear_force_threshold_n(Some(250000.0));
        assert!(
            (t - 73549.875).abs() < 1.0,
            "expected ~73550, got {}",
            t
        );
    }

    #[test]
    fn gear_force_threshold_floor_for_no_weight() {
        let t = gear_force_threshold_n(None);
        assert_eq!(t, 1000.0);
    }

    #[test]
    fn gear_force_threshold_floor_for_zero_weight() {
        let t = gear_force_threshold_n(Some(0.0));
        assert_eq!(t, 1000.0);
    }

    #[test]
    fn finalize_vs_rejects_positive() {
        assert_eq!(finalize_vs(100.0), Err(RejectionReason::PositiveVs));
    }

    #[test]
    fn finalize_vs_rejects_implausibly_high() {
        assert_eq!(
            finalize_vs(-3500.0),
            Err(RejectionReason::ImplausiblyHigh)
        );
    }

    #[test]
    fn finalize_vs_rejects_nan() {
        assert_eq!(finalize_vs(f32::NAN), Err(RejectionReason::EmptyWindow));
    }

    #[test]
    fn finalize_vs_accepts_typical_landing() {
        assert_eq!(finalize_vs(-150.0), Ok(-150.0));
    }

    fn touch(vs: f32) -> LowLevelTouch {
        LowLevelTouch {
            at: Utc::now(),
            vs_at_impact_fpm: vs,
            agl_max_ft: 3.0,
            sustained_ms: 200,
        }
    }

    #[test]
    fn hardest_impact_no_bounces_returns_contact() {
        let (vs, src) = compute_hardest_impact(-300.0, &[]);
        assert_eq!(vs, -300.0);
        assert_eq!(src, HardestImpactSource::Contact);
    }

    #[test]
    fn hardest_impact_contact_harder_than_bounce() {
        // PTO 705 Pattern: contact -182, low_level -61 → hardest = -182 (contact)
        let (vs, src) = compute_hardest_impact(-182.0, &[touch(-61.0)]);
        assert_eq!(vs, -182.0);
        assert_eq!(src, HardestImpactSource::Contact);
    }

    #[test]
    fn hardest_impact_bounce_harder_than_contact() {
        // Hard-Bounce-Pattern: contact -200, bounce -600 → hardest = -600 (bounce)
        let (vs, src) = compute_hardest_impact(-200.0, &[touch(-100.0), touch(-600.0)]);
        assert_eq!(vs, -600.0);
        assert_eq!(src, HardestImpactSource::LowLevelTouch(1));
    }

    #[test]
    fn classify_final_landing() {
        let s = EpisodePostContactState {
            max_agl_ft_after_contact: 30.0,
            settled_under_50ft_for_30s: true,
            current_gs_kt: 15.0,
        };
        assert_eq!(classify_episode(s), EpisodeClass::FinalLanding);
    }

    #[test]
    fn classify_touch_and_go_pattern() {
        let s = EpisodePostContactState {
            max_agl_ft_after_contact: 800.0,
            settled_under_50ft_for_30s: false,
            current_gs_kt: 90.0,
        };
        assert_eq!(classify_episode(s), EpisodeClass::TouchAndGo);
    }

    #[test]
    fn classify_go_around() {
        let s = EpisodePostContactState {
            max_agl_ft_after_contact: 1500.0,
            settled_under_50ft_for_30s: false,
            current_gs_kt: 130.0,
        };
        assert_eq!(classify_episode(s), EpisodeClass::GoAround);
    }

    #[test]
    fn classify_pending_low_agl_not_settled_yet() {
        let s = EpisodePostContactState {
            max_agl_ft_after_contact: 20.0,
            settled_under_50ft_for_30s: false,
            current_gs_kt: 80.0,
        };
        assert_eq!(classify_episode(s), EpisodeClass::Pending);
    }
}
