import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { useConfirm } from "./ConfirmDialog";

// ---- Types (mirror storage::LandingRecord on the Rust side) -------------

export interface LandingProfilePoint {
  t_ms: number;
  vs_fpm: number;
  g_force: number;
  agl_ft: number;
  on_ground: boolean;
  heading_true_deg: number;
  groundspeed_kt: number;
  indicated_airspeed_kt: number;
  pitch_deg: number;
  bank_deg: number;
}

export interface LandingRunwayMatch {
  airport_ident: string;
  runway_ident: string;
  surface: string;
  length_ft: number;
  centerline_distance_m: number;
  centerline_distance_abs_ft: number;
  side: string;
  touchdown_distance_from_threshold_ft: number;
}

export interface LandingRecord {
  pirep_id: string;
  touchdown_at: string;
  recorded_at: string;
  flight_number: string;
  airline_icao: string;
  dpt_airport: string;
  arr_airport: string;
  aircraft_registration: string | null;
  aircraft_icao: string | null;
  aircraft_title: string | null;
  sim_kind: string | null;

  score_numeric: number;
  score_label: string;
  grade_letter: string;

  landing_rate_fpm: number;
  landing_peak_vs_fpm: number | null;
  landing_g_force: number | null;
  landing_peak_g_force: number | null;
  landing_pitch_deg: number | null;
  landing_bank_deg: number | null;
  landing_speed_kt: number | null;
  landing_heading_deg: number | null;
  landing_weight_kg: number | null;
  touchdown_sideslip_deg: number | null;
  bounce_count: number;

  headwind_kt: number | null;
  crosswind_kt: number | null;

  approach_vs_stddev_fpm: number | null;
  approach_bank_stddev_deg: number | null;
  rollout_distance_m: number | null;

  planned_block_fuel_kg: number | null;
  planned_burn_kg: number | null;
  planned_tow_kg: number | null;
  planned_ldw_kg: number | null;
  planned_zfw_kg: number | null;
  actual_trip_burn_kg: number | null;
  fuel_efficiency_kg_diff: number | null;
  fuel_efficiency_pct: number | null;
  takeoff_weight_kg: number | null;
  takeoff_fuel_kg: number | null;
  landing_fuel_kg: number | null;
  block_fuel_kg: number | null;

  runway_match: LandingRunwayMatch | null;
  touchdown_profile: LandingProfilePoint[];
  approach_samples: ApproachSample[];
}

export interface ApproachSample {
  vs_fpm: number;
  bank_deg: number;
}

// ---- Score breakdown ---------------------------------------------------
//
// We split the overall touchdown score into 6 sub-categories so the pilot
// can see *which* aspect of the landing pulled the grade down. Each is a
// 0-100 score with a short rationale. Thresholds are calibrated against
// FOQA-style guidelines and the existing primary score table.

export interface SubScore {
  key: string;
  points: number;
  /** Pre-formatted value to show on the card ("379 fpm", "1.10 G", …). */
  value: string;
  /** "good" | "ok" | "bad" — drives the colour band. */
  band: "good" | "ok" | "bad";
  /** Why we awarded this score (one short sentence). */
  rationale: string;
}

function band(points: number): "good" | "ok" | "bad" {
  if (points >= 85) return "good";
  if (points >= 65) return "ok";
  return "bad";
}

function scoreLandingRate(fpm: number): { points: number; rationale: string } {
  const abs = Math.abs(fpm);
  if (abs <= 100) return { points: 100, rationale: "smooth_touchdown" };
  if (abs <= 240) return { points: 90, rationale: "firm_but_clean" };
  if (abs <= 360) return { points: 75, rationale: "above_target" };
  if (abs <= 600) return { points: 50, rationale: "hard_landing" };
  if (abs <= 1000) return { points: 25, rationale: "very_hard" };
  return { points: 0, rationale: "severe_inspection" };
}

function scoreGForce(g: number): { points: number; rationale: string } {
  if (g <= 1.2) return { points: 100, rationale: "smooth_g" };
  if (g <= 1.4) return { points: 95, rationale: "comfortable_g" };
  if (g <= 1.7) return { points: 80, rationale: "noticeable_g" };
  if (g <= 2.0) return { points: 60, rationale: "firm_g" };
  if (g <= 2.6) return { points: 30, rationale: "hard_g" };
  return { points: 0, rationale: "severe_g" };
}

function scoreBounces(n: number): { points: number; rationale: string } {
  if (n === 0) return { points: 100, rationale: "clean_set" };
  if (n === 1) return { points: 65, rationale: "one_bounce" };
  if (n === 2) return { points: 35, rationale: "two_bounces" };
  return { points: 0, rationale: "many_bounces" };
}

function scoreStability(vsStd: number): { points: number; rationale: string } {
  if (vsStd <= 100) return { points: 100, rationale: "very_stable" };
  if (vsStd <= 200) return { points: 85, rationale: "stable" };
  if (vsStd <= 300) return { points: 65, rationale: "average_stability" };
  if (vsStd <= 400) return { points: 45, rationale: "unstable_approach" };
  return { points: 25, rationale: "very_unstable" };
}

function scoreRollout(usedPct: number): { points: number; rationale: string } {
  if (usedPct <= 50) return { points: 100, rationale: "excellent_stop" };
  if (usedPct <= 70) return { points: 85, rationale: "good_stop" };
  if (usedPct <= 85) return { points: 65, rationale: "long_rollout" };
  if (usedPct <= 95) return { points: 40, rationale: "very_long_rollout" };
  return { points: 20, rationale: "marginal_runway" };
}

function scoreFuel(absPct: number): { points: number; rationale: string } {
  if (absPct <= 5) return { points: 100, rationale: "on_plan" };
  if (absPct <= 10) return { points: 85, rationale: "near_plan" };
  if (absPct <= 20) return { points: 65, rationale: "off_plan" };
  if (absPct <= 30) return { points: 45, rationale: "very_off_plan" };
  return { points: 25, rationale: "way_off_plan" };
}

function computeSubScores(r: LandingRecord): SubScore[] {
  const out: SubScore[] = [];

  const rate = scoreLandingRate(r.landing_rate_fpm);
  out.push({
    key: "landing_rate",
    points: rate.points,
    value: `${r.landing_rate_fpm.toFixed(0)} fpm`,
    band: band(rate.points),
    rationale: rate.rationale,
  });

  if (r.landing_peak_g_force != null) {
    const g = scoreGForce(r.landing_peak_g_force);
    out.push({
      key: "g_force",
      points: g.points,
      value: `${r.landing_peak_g_force.toFixed(2)} G`,
      band: band(g.points),
      rationale: g.rationale,
    });
  }

  const b = scoreBounces(r.bounce_count);
  out.push({
    key: "bounces",
    points: b.points,
    value: `${r.bounce_count}`,
    band: band(b.points),
    rationale: b.rationale,
  });

  if (r.approach_vs_stddev_fpm != null) {
    const s = scoreStability(r.approach_vs_stddev_fpm);
    out.push({
      key: "stability",
      points: s.points,
      value: `${r.approach_vs_stddev_fpm.toFixed(0)} fpm σ`,
      band: band(s.points),
      rationale: s.rationale,
    });
  }

  if (
    r.runway_match &&
    r.runway_match.length_ft > 0 &&
    r.rollout_distance_m != null
  ) {
    const lengthM = r.runway_match.length_ft * 0.3048;
    const usedPct = (r.rollout_distance_m / lengthM) * 100;
    const ro = scoreRollout(usedPct);
    out.push({
      key: "rollout",
      points: ro.points,
      value: `${usedPct.toFixed(0)}% (${r.rollout_distance_m.toFixed(0)} m)`,
      band: band(ro.points),
      rationale: ro.rationale,
    });
  }

  if (r.fuel_efficiency_pct != null) {
    const f = scoreFuel(Math.abs(r.fuel_efficiency_pct));
    const sign = r.fuel_efficiency_pct >= 0 ? "+" : "";
    out.push({
      key: "fuel",
      points: f.points,
      value: `${sign}${r.fuel_efficiency_pct.toFixed(1)}%`,
      band: band(f.points),
      rationale: f.rationale,
    });
  }

  return out;
}

/** Rationale → i18n key for the coach tip. We point straight at the
 *  fully-qualified `landing.tip.*` path so a missing translation
 *  shows up as the key (easier to spot in QA) rather than as a
 *  silent fallback. */
function coachTipKey(rationale: string): string {
  return `landing.tip.${rationale}`;
}

// ---- Helpers ------------------------------------------------------------

function gradeColor(grade: string): string {
  if (grade === "A+" || grade === "A") return "#22c55e"; // green
  if (grade === "B+" || grade === "B") return "#84cc16"; // lime
  if (grade === "C") return "#eab308"; // amber
  if (grade === "D") return "#f97316"; // orange
  return "#ef4444"; // red — F
}

function fmtNumber(
  v: number | null | undefined,
  digits = 0,
  unit = "",
): string {
  if (v == null || !Number.isFinite(v)) return "—";
  return `${v.toFixed(digits)}${unit ? ` ${unit}` : ""}`;
}

function fmtSigned(v: number | null | undefined, digits = 0, unit = ""): string {
  if (v == null || !Number.isFinite(v)) return "—";
  const sign = v >= 0 ? "+" : "";
  return `${sign}${v.toFixed(digits)}${unit ? ` ${unit}` : ""}`;
}

function fmtDateTime(iso: string): string {
  try {
    const d = new Date(iso);
    return d.toLocaleString();
  } catch {
    return iso;
  }
}

/** Translate the runway-side enum from the backend ("RIGHT"/"LEFT"/
 *  "CENTER") into the i18n key under `landing.side.*` so the UI
 *  reads "rechts" / "links" / "Mitte" in German. */
function sideKey(side: string): string {
  const upper = side.toUpperCase();
  if (upper === "RIGHT") return "landing.side.right";
  if (upper === "LEFT") return "landing.side.left";
  return "landing.side.center";
}

// ---- VS Curve chart -----------------------------------------------------

function VsCurveChart({ profile }: { profile: LandingProfilePoint[] }) {
  const { t } = useTranslation();
  if (profile.length < 2) {
    return (
      <div className="landing-chart landing-chart--empty">
        {t("landing.no_profile")}
      </div>
    );
  }
  const w = 480;
  const h = 160;
  const pad = { top: 12, right: 12, bottom: 24, left: 38 };
  const innerW = w - pad.left - pad.right;
  const innerH = h - pad.top - pad.bottom;

  const ts = profile.map((p) => p.t_ms);
  const vss = profile.map((p) => p.vs_fpm);
  const tMin = Math.min(...ts);
  const tMax = Math.max(...ts);
  const vMin = Math.min(0, ...vss); // include 0-line
  const vMax = Math.max(0, ...vss);
  const tRange = Math.max(1, tMax - tMin);
  const vRange = Math.max(1, vMax - vMin);

  function x(tms: number) {
    return pad.left + ((tms - tMin) / tRange) * innerW;
  }
  function y(vs: number) {
    return pad.top + innerH - ((vs - vMin) / vRange) * innerH;
  }

  const path = profile
    .map((p, i) => `${i === 0 ? "M" : "L"} ${x(p.t_ms).toFixed(1)} ${y(p.vs_fpm).toFixed(1)}`)
    .join(" ");

  // Touchdown marker = sample with smallest |t_ms|
  const tdIdx = profile.reduce(
    (best, p, i) => (Math.abs(p.t_ms) < Math.abs(profile[best].t_ms) ? i : best),
    0,
  );
  const td = profile[tdIdx];

  return (
    <svg
      className="landing-chart"
      viewBox={`0 0 ${w} ${h}`}
      preserveAspectRatio="xMidYMid meet"
      role="img"
      aria-label={t("landing.vs_curve")}
    >
      {/* Frame */}
      <rect
        x={pad.left}
        y={pad.top}
        width={innerW}
        height={innerH}
        fill="rgba(255,255,255,0.02)"
        stroke="rgba(255,255,255,0.15)"
      />
      {/* Zero line */}
      <line
        x1={pad.left}
        x2={pad.left + innerW}
        y1={y(0)}
        y2={y(0)}
        stroke="rgba(255,255,255,0.3)"
        strokeDasharray="2,3"
      />
      {/* Touchdown vertical */}
      <line
        x1={x(td.t_ms)}
        x2={x(td.t_ms)}
        y1={pad.top}
        y2={pad.top + innerH}
        stroke="#facc15"
        strokeDasharray="3,3"
      />
      {/* Curve */}
      <path
        d={path}
        fill="none"
        stroke="#38bdf8"
        strokeWidth="2"
        strokeLinejoin="round"
      />
      {/* Touchdown dot */}
      <circle cx={x(td.t_ms)} cy={y(td.vs_fpm)} r="4" fill="#facc15" />
      {/* Y axis labels */}
      <text
        x={pad.left - 4}
        y={y(vMax) + 4}
        textAnchor="end"
        fontSize="10"
        fill="currentColor"
      >
        {vMax.toFixed(0)}
      </text>
      <text
        x={pad.left - 4}
        y={y(vMin) + 4}
        textAnchor="end"
        fontSize="10"
        fill="currentColor"
      >
        {vMin.toFixed(0)}
      </text>
      <text
        x={pad.left - 4}
        y={y(0) + 4}
        textAnchor="end"
        fontSize="10"
        fill="rgba(255,255,255,0.6)"
      >
        0
      </text>
      {/* X axis labels — we hide the right-edge tMax label when TD
          sits at (or near) the right edge, otherwise the "TD" yellow
          label visually merges with "0.0s" into "TDs" (real bug
          observed). Same for tMin/start-edge.                       */}
      {Math.abs(x(td.t_ms) - pad.left) > 22 && (
        <text x={pad.left} y={h - 8} fontSize="10" fill="currentColor">
          {(tMin / 1000).toFixed(1)}s
        </text>
      )}
      {Math.abs(x(td.t_ms) - (pad.left + innerW)) > 22 && (
        <text
          x={pad.left + innerW}
          y={h - 8}
          textAnchor="end"
          fontSize="10"
          fill="currentColor"
        >
          {(tMax / 1000).toFixed(1)}s
        </text>
      )}
      <text
        x={x(td.t_ms)}
        y={h - 8}
        textAnchor="middle"
        fontSize="10"
        fontWeight="600"
        fill="#facc15"
      >
        TD
      </text>
    </svg>
  );
}

// ---- Runway diagram ----------------------------------------------------

function RunwayDiagram({
  rw,
  rolloutDistanceM,
}: {
  rw: LandingRunwayMatch;
  rolloutDistanceM: number | null;
}) {
  const { t } = useTranslation();
  const w = 480;
  const h = 130;
  // Runway band
  const rwLeft = 30;
  const rwRight = w - 30;
  const rwTop = h / 2 - 16;
  const rwBottom = h / 2 + 16;
  const lengthFt = rw.length_ft;
  const lengthM = lengthFt * 0.3048;
  const tdFromThresh = rw.touchdown_distance_from_threshold_ft;
  const tdFromThreshM = tdFromThresh * 0.3048;
  // Map threshold→far-end onto the rect.
  const tdFrac = Math.min(1, Math.max(0, tdFromThresh / Math.max(1, lengthFt)));
  const tdX = rwLeft + tdFrac * (rwRight - rwLeft);

  // Centerline offset → vertical Y inside the strip
  const offsetM = rw.centerline_distance_m;
  const widthM = 45; // assume ~45 m runway width if not exposed
  const offFrac = Math.max(-1, Math.min(1, offsetM / widthM));
  const tdY = (rwTop + rwBottom) / 2 + offFrac * 12;

  // Rollout band: from TD point along the runway centerline for the
  // accumulated rollout distance (in m). End-X clamps to the far-end
  // marker if the rollout extended past the runway end (rare —
  // means the pilot exited beyond the threshold).
  const rolloutEndFrac =
    rolloutDistanceM != null && lengthM > 0
      ? Math.min(1, (tdFromThreshM + rolloutDistanceM) / lengthM)
      : null;
  const rolloutEndX =
    rolloutEndFrac != null
      ? rwLeft + rolloutEndFrac * (rwRight - rwLeft)
      : null;

  return (
    <svg
      className="landing-runway"
      viewBox={`0 0 ${w} ${h}`}
      preserveAspectRatio="xMidYMid meet"
      role="img"
      aria-label={t("landing.runway_diagram")}
    >
      {/* Runway tarmac */}
      <rect
        x={rwLeft}
        y={rwTop}
        width={rwRight - rwLeft}
        height={rwBottom - rwTop}
        fill="#1f2937"
        stroke="rgba(255,255,255,0.3)"
      />
      {/* Centerline dashes */}
      <line
        x1={rwLeft + 8}
        x2={rwRight - 8}
        y1={(rwTop + rwBottom) / 2}
        y2={(rwTop + rwBottom) / 2}
        stroke="#fbbf24"
        strokeWidth="1.4"
        strokeDasharray="10,8"
      />
      {/* Rollout band — semi-transparent green strip from TD to exit */}
      {rolloutEndX != null && (
        <>
          <rect
            x={Math.min(tdX, rolloutEndX)}
            y={rwTop + 4}
            width={Math.abs(rolloutEndX - tdX)}
            height={rwBottom - rwTop - 8}
            fill="rgba(34,197,94,0.28)"
            stroke="rgba(34,197,94,0.7)"
            strokeWidth="1"
          />
          {/* Exit marker line */}
          <line
            x1={rolloutEndX}
            x2={rolloutEndX}
            y1={rwTop - 2}
            y2={rwBottom + 2}
            stroke="#22c55e"
            strokeWidth="2"
          />
        </>
      )}
      {/* Threshold marker */}
      <line
        x1={rwLeft + 4}
        x2={rwLeft + 4}
        y1={rwTop}
        y2={rwBottom}
        stroke="#ffffff"
        strokeWidth="3"
      />
      {/* Far end */}
      <line
        x1={rwRight - 4}
        x2={rwRight - 4}
        y1={rwTop}
        y2={rwBottom}
        stroke="#ffffff"
        strokeWidth="3"
      />
      {/* Touchdown dot */}
      <circle cx={tdX} cy={tdY} r="6" fill="#22d3ee" stroke="#000" strokeWidth="1" />
      {/* Labels above the runway */}
      <text x={rwLeft} y={rwTop - 6} fontSize="11" fill="currentColor">
        {rw.runway_ident} · {rw.airport_ident}
      </text>
      <text
        x={rwRight}
        y={rwTop - 6}
        textAnchor="end"
        fontSize="11"
        fill="currentColor"
      >
        {lengthM.toFixed(0)} m
      </text>
      {/* TD label below the dot */}
      <text
        x={tdX}
        y={rwBottom + 14}
        textAnchor="middle"
        fontSize="10"
        fill="#22d3ee"
      >
        TD · {tdFromThreshM.toFixed(0)} m {t("landing.past_threshold_short")}
      </text>
      {/* Centerline-offset annotation, suppressed for CENTER (=0 m) */}
      {Math.abs(rw.centerline_distance_m) >= 1 && (
        <text
          x={tdX}
          y={rwBottom + 26}
          textAnchor="middle"
          fontSize="10"
          fill="currentColor"
        >
          {Math.abs(rw.centerline_distance_m).toFixed(1)} m {t(sideKey(rw.side))}
        </text>
      )}
      {/* Exit label — only when there's enough horizontal room */}
      {rolloutEndX != null && Math.abs(rolloutEndX - tdX) > 60 && (
        <text
          x={rolloutEndX}
          y={rwTop - 6}
          textAnchor="middle"
          fontSize="10"
          fill="#22c55e"
        >
          {t("landing.exit")} · {(rolloutDistanceM ?? 0).toFixed(0)} m
        </text>
      )}
    </svg>
  );
}

// ---- Wind compass -------------------------------------------------------

function WindCompass({
  headwindKt,
  crosswindKt,
}: {
  headwindKt: number | null;
  crosswindKt: number | null;
}) {
  const { t } = useTranslation();
  if (headwindKt == null && crosswindKt == null) return null;
  const hw = headwindKt ?? 0;
  const xw = crosswindKt ?? 0;

  const totalKt = Math.sqrt(hw * hw + xw * xw);
  // atan2(xw, hw) — xw > 0 = from right, hw > 0 = from front. This is
  // the direction the wind COMES FROM relative to the aircraft nose.
  const angleRad = Math.atan2(xw, hw);
  const w = 200;
  const h = 220;
  const cx = w / 2;
  const cy = 90;
  const r = 60;

  // Pilot convention: wind is described by the direction it comes FROM.
  // Render as a wind-vane needle pointing AT that source. Tail starts
  // near the centre (just outside the aircraft silhouette), head sits
  // on the rim in the direction the wind is coming from.
  const tailX = cx + Math.sin(angleRad) * 16;
  const tailY = cy - Math.cos(angleRad) * 16;
  const headX = cx + Math.sin(angleRad) * (r - 4);
  const headY = cy - Math.cos(angleRad) * (r - 4);

  // Pulled from the labels' "from-quadrant" so the user reads it as
  // "wind aus 5 Uhr" / "from front".
  const cardinalLabel = (() => {
    const deg = ((angleRad * 180) / Math.PI + 360) % 360;
    if (deg < 22.5 || deg >= 337.5) return t("landing.wind_from_front");
    if (deg < 67.5) return t("landing.wind_from_front_right");
    if (deg < 112.5) return t("landing.wind_from_right");
    if (deg < 157.5) return t("landing.wind_from_rear_right");
    if (deg < 202.5) return t("landing.wind_from_rear");
    if (deg < 247.5) return t("landing.wind_from_rear_left");
    if (deg < 292.5) return t("landing.wind_from_left");
    return t("landing.wind_from_front_left");
  })();

  return (
    <svg
      className="landing-wind"
      viewBox={`0 0 ${w} ${h}`}
      preserveAspectRatio="xMidYMid meet"
      role="img"
      aria-label={t("landing.wind")}
    >
      <defs>
        <marker
          id="wind-arrow"
          markerWidth="10"
          markerHeight="10"
          refX="5"
          refY="5"
          orient="auto"
        >
          <path d="M0,0 L10,5 L0,10 z" fill="#38bdf8" />
        </marker>
      </defs>
      {/* Compass face */}
      <circle
        cx={cx}
        cy={cy}
        r={r + 8}
        fill="rgba(255,255,255,0.04)"
        stroke="rgba(255,255,255,0.25)"
      />
      {/* Cardinal ticks */}
      {[0, 90, 180, 270].map((deg) => {
        const a = (deg * Math.PI) / 180;
        const x1 = cx + Math.sin(a) * (r + 8);
        const y1 = cy - Math.cos(a) * (r + 8);
        const x2 = cx + Math.sin(a) * (r + 2);
        const y2 = cy - Math.cos(a) * (r + 2);
        return (
          <line
            key={deg}
            x1={x1}
            y1={y1}
            x2={x2}
            y2={y2}
            stroke="rgba(255,255,255,0.35)"
            strokeWidth="1.5"
          />
        );
      })}
      {/* Aircraft silhouette pointing up (north on dial = nose) */}
      <polygon
        points={`${cx},${cy - 14} ${cx - 9},${cy + 12} ${cx + 9},${cy + 12}`}
        fill="#a3a3a3"
      />
      {/* Wind needle — points OUTWARD toward the source (windvane convention). */}
      <line
        x1={tailX}
        y1={tailY}
        x2={headX}
        y2={headY}
        stroke="#38bdf8"
        strokeWidth="3"
        markerEnd="url(#wind-arrow)"
        strokeLinecap="round"
      />
      {/* Total speed */}
      <text
        x={cx}
        y={cy + r + 28}
        textAnchor="middle"
        fontSize="18"
        fontWeight="600"
        fill="currentColor"
      >
        {totalKt.toFixed(0)} kt
      </text>
      {/* "Wind aus vorn", "Wind aus links", … */}
      <text
        x={cx}
        y={cy + r + 46}
        textAnchor="middle"
        fontSize="11"
        fill="var(--text-muted, #888)"
      >
        {cardinalLabel}
      </text>
      {/* Component breakdown */}
      <text
        x={cx}
        y={cy + r + 60}
        textAnchor="middle"
        fontSize="10"
        fill="var(--text-muted, #888)"
      >
        H {hw >= 0 ? "+" : ""}
        {hw.toFixed(0)} · X {xw >= 0 ? "+" : ""}
        {xw.toFixed(0)} kt
      </text>
    </svg>
  );
}

// ---- Approach stability time-series chart ------------------------------

function ApproachChart({ samples }: { samples: ApproachSample[] }) {
  const { t } = useTranslation();
  if (samples.length < 3) return null;
  const w = 600;
  const h = 140;
  const pad = { top: 12, right: 12, bottom: 22, left: 40 };
  const innerW = w - pad.left - pad.right;
  const innerH = h - pad.top - pad.bottom;
  const vss = samples.map((s) => s.vs_fpm);
  const vMin = Math.min(0, ...vss, -1500);
  const vMax = Math.max(0, ...vss, 100);
  const vRange = Math.max(1, vMax - vMin);
  const xStep = innerW / Math.max(1, samples.length - 1);
  const y = (vs: number) => pad.top + innerH - ((vs - vMin) / vRange) * innerH;
  const path = samples
    .map((s, i) => `${i === 0 ? "M" : "L"} ${(pad.left + i * xStep).toFixed(1)} ${y(s.vs_fpm).toFixed(1)}`)
    .join(" ");
  // Stable-target band: -1000 to -500 fpm is the typical glide-slope V/S range
  const bandTop = y(-500);
  const bandBottom = y(-1000);
  return (
    <svg
      className="landing-chart"
      viewBox={`0 0 ${w} ${h}`}
      preserveAspectRatio="xMidYMid meet"
      role="img"
      aria-label={t("landing.approach_chart")}
    >
      <rect
        x={pad.left}
        y={pad.top}
        width={innerW}
        height={innerH}
        fill="rgba(255,255,255,0.02)"
        stroke="rgba(255,255,255,0.15)"
      />
      {/* Stable target band */}
      <rect
        x={pad.left}
        y={bandTop}
        width={innerW}
        height={Math.max(0, bandBottom - bandTop)}
        fill="rgba(34,197,94,0.08)"
      />
      <path d={path} fill="none" stroke="#38bdf8" strokeWidth="1.6" />
      <text x={pad.left - 4} y={y(vMax) + 4} textAnchor="end" fontSize="10" fill="currentColor">
        {vMax.toFixed(0)}
      </text>
      <text x={pad.left - 4} y={y(vMin) + 4} textAnchor="end" fontSize="10" fill="currentColor">
        {vMin.toFixed(0)}
      </text>
      <text x={pad.left} y={h - 6} fontSize="10" fill="currentColor">
        {t("landing.approach_start")}
      </text>
      <text x={pad.left + innerW} y={h - 6} textAnchor="end" fontSize="10" fill="currentColor">
        {t("landing.touchdown")}
      </text>
    </svg>
  );
}

// ---- (i) info badge — small click-to-toggle popover --------------------
//
// Pilots may not know what "V/S σ" or "Bahn-Auslastung" means precisely.
// Each sub-score card carries a small (i) icon that, when clicked,
// reveals an explanation popover above/below the card. We use a click
// instead of hover so it's tappable on touch devices and stays open
// while the pilot reads it.

function InfoBadge({ explanation }: { explanation: string }) {
  const [open, setOpen] = useState(false);
  // Whether to flip the popover to the LEFT side of the badge to keep
  // it inside the viewport. Decided on click using getBoundingClientRect
  // — popover is ~300 px wide; if the badge's right edge sits within
  // 320 px of the viewport's right edge we flip.
  const [flipLeft, setFlipLeft] = useState(false);
  return (
    <span className="info-badge-wrap">
      <button
        type="button"
        className={`info-badge ${open ? "info-badge--open" : ""}`}
        onClick={(e) => {
          e.stopPropagation();
          if (!open) {
            const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
            setFlipLeft(window.innerWidth - rect.right < 320);
          }
          setOpen((v) => !v);
        }}
        aria-label="info"
      >
        i
      </button>
      {open && (
        <span
          className={`info-badge__popover ${
            flipLeft ? "info-badge__popover--flip-left" : ""
          }`}
          role="tooltip"
        >
          {explanation}
          <button
            type="button"
            className="info-badge__close"
            onClick={(e) => {
              e.stopPropagation();
              setOpen(false);
            }}
            aria-label="close"
          >
            ×
          </button>
        </span>
      )}
    </span>
  );
}

// ---- Score breakdown card grid -----------------------------------------

function ScoreBreakdown({ subs }: { subs: SubScore[] }) {
  const { t } = useTranslation();
  if (subs.length === 0) return null;
  return (
    <div className="landing-subscores">
      {subs.map((s) => (
        <div
          key={s.key}
          className={`landing-subscore landing-subscore--${s.band}`}
        >
          <div className="landing-subscore__head">
            <span className="landing-subscore__label">
              {t(`landing.sub.${s.key}`)}
              <InfoBadge explanation={t(`landing.info.${s.key}`)} />
            </span>
            <span className="landing-subscore__points">{s.points} PTS</span>
          </div>
          <div className="landing-subscore__value">{s.value}</div>
          <div className="landing-subscore__bar">
            <div
              className="landing-subscore__fill"
              style={{ width: `${s.points}%` }}
            />
          </div>
          <div className="landing-subscore__rationale">
            {t(`landing.rat.${s.rationale}`)}
          </div>
        </div>
      ))}
    </div>
  );
}

// ---- Coach tip — focuses on the worst sub-score ------------------------

function CoachTip({ subs }: { subs: SubScore[] }) {
  const { t } = useTranslation();
  if (subs.length === 0) return null;
  // Sort ascending, the lowest sub-score is the area to improve. If
  // everything is ≥ 90, surface the genuine "good landing" message.
  const sorted = [...subs].sort((a, b) => a.points - b.points);
  const worst = sorted[0];
  const tipKey = coachTipKey(worst.rationale);
  return (
    <div
      className={`landing-coach landing-coach--${
        worst.points >= 85 ? "good" : worst.points >= 65 ? "ok" : "bad"
      }`}
    >
      <div className="landing-coach__head">
        {t("landing.coach_title")} ·{" "}
        <strong>{t(`landing.sub.${worst.key}`)}</strong>
      </div>
      <p className="landing-coach__body">{t(tipKey)}</p>
    </div>
  );
}

// ---- Trend sparkline (last N landings) ---------------------------------

function TrendSparkline({ records }: { records: LandingRecord[] }) {
  const { t } = useTranslation();
  if (records.length < 2) return null;
  // Use newest-first list; chart wants oldest→newest left→right.
  const latest = records.slice(0, 12).reverse();
  const w = 360;
  const h = 60;
  const pad = 6;
  const innerW = w - pad * 2;
  const innerH = h - pad * 2;
  const scores = latest.map((r) => r.score_numeric);
  const sMin = Math.min(...scores, 50);
  const sMax = Math.max(...scores, 100);
  const range = Math.max(1, sMax - sMin);
  const xStep = innerW / Math.max(1, latest.length - 1);
  const y = (s: number) => pad + innerH - ((s - sMin) / range) * innerH;
  const path = latest
    .map((r, i) =>
      `${i === 0 ? "M" : "L"} ${(pad + i * xStep).toFixed(1)} ${y(r.score_numeric).toFixed(1)}`,
    )
    .join(" ");
  const last = latest[latest.length - 1];
  return (
    <div className="landing-trend">
      <span className="landing-trend__label">{t("landing.trend")}</span>
      <svg
        className="landing-trend__svg"
        viewBox={`0 0 ${w} ${h}`}
        preserveAspectRatio="none"
        role="img"
        aria-label={t("landing.trend")}
      >
        <path d={path} fill="none" stroke="#38bdf8" strokeWidth="2" />
        {latest.map((r, i) => (
          <circle
            key={r.pirep_id}
            cx={pad + i * xStep}
            cy={y(r.score_numeric)}
            r={i === latest.length - 1 ? 4 : 2}
            fill={i === latest.length - 1 ? gradeColor(r.grade_letter) : "#38bdf8"}
          />
        ))}
      </svg>
      <span className="landing-trend__last">
        {last.score_numeric}/100 · {last.grade_letter}
      </span>
    </div>
  );
}

// ---- Fuel comparison bar ------------------------------------------------

function FuelComparisonBar({
  plan,
  actual,
}: {
  plan: number;
  actual: number;
}) {
  const { t } = useTranslation();
  const max = Math.max(plan, actual, 1);
  const planPct = (plan / max) * 100;
  const actualPct = (actual / max) * 100;
  const diff = actual - plan;
  const sign = diff >= 0 ? "+" : "";
  const pct = (diff / Math.max(1, plan)) * 100;

  return (
    <div className="landing-fuelbar">
      <div className="landing-fuelbar__row">
        <span className="landing-fuelbar__label">{t("landing.plan_burn")}</span>
        <div className="landing-fuelbar__track">
          <div
            className="landing-fuelbar__fill landing-fuelbar__fill--plan"
            style={{ width: `${planPct}%` }}
          />
        </div>
        <span className="landing-fuelbar__value">{plan.toFixed(0)} kg</span>
      </div>
      <div className="landing-fuelbar__row">
        <span className="landing-fuelbar__label">{t("landing.actual_burn")}</span>
        <div className="landing-fuelbar__track">
          <div
            className={`landing-fuelbar__fill ${
              diff > 0
                ? "landing-fuelbar__fill--over"
                : "landing-fuelbar__fill--under"
            }`}
            style={{ width: `${actualPct}%` }}
          />
        </div>
        <span className="landing-fuelbar__value">{actual.toFixed(0)} kg</span>
      </div>
      <div
        className={`landing-fuelbar__delta ${diff > 0 ? "landing-fuelbar__delta--over" : ""}`}
      >
        {sign}
        {diff.toFixed(0)} kg ({sign}
        {pct.toFixed(1)}%)
      </div>
    </div>
  );
}

// ---- Stability gauge ----------------------------------------------------

function StabilityIndicator({
  vsStd,
  bankStd,
}: {
  vsStd: number | null;
  bankStd: number | null;
}) {
  const { t } = useTranslation();
  if (vsStd == null && bankStd == null) return null;

  function band(v: number, good: number, ok: number): string {
    if (v <= good) return "good";
    if (v <= ok) return "ok";
    return "bad";
  }
  const vsBand = vsStd != null ? band(vsStd, 100, 200) : "n/a";
  const bankBand = bankStd != null ? band(bankStd, 3, 6) : "n/a";

  return (
    <div className="landing-stability">
      <div className={`landing-stability__row landing-stability__row--${vsBand}`}>
        <span>{t("landing.vs_stddev")}</span>
        <strong>{vsStd != null ? `${vsStd.toFixed(0)} fpm` : "—"}</strong>
      </div>
      <div className={`landing-stability__row landing-stability__row--${bankBand}`}>
        <span>{t("landing.bank_stddev")}</span>
        <strong>{bankStd != null ? `${bankStd.toFixed(1)}°` : "—"}</strong>
      </div>
    </div>
  );
}

// ---- Detail view --------------------------------------------------------

function LandingDetail({
  record,
  allRecords,
  onBack,
  onDelete,
  isPreview,
}: {
  record: LandingRecord;
  /** Full history — used to compute personal-best comparisons. */
  allRecords: LandingRecord[];
  onBack: () => void;
  onDelete?: () => void;
  isPreview: boolean;
}) {
  const { t } = useTranslation();

  const callsign = record.airline_icao
    ? `${record.airline_icao}${record.flight_number}`
    : record.flight_number;

  const subs = useMemo(() => computeSubScores(record), [record]);

  // Personal-best comparison — best (closest to zero) landing rate
  // across ALL filed PIREPs. None when this is the only record yet.
  const personalBest = useMemo(() => {
    const others = allRecords.filter((r) => r.pirep_id !== record.pirep_id);
    if (others.length === 0) return null;
    return others.reduce(
      (best, r) =>
        Math.abs(r.landing_rate_fpm) < Math.abs(best.landing_rate_fpm) ? r : best,
      others[0],
    );
  }, [allRecords, record.pirep_id]);

  const isNewBest =
    personalBest != null &&
    Math.abs(record.landing_rate_fpm) < Math.abs(personalBest.landing_rate_fpm);

  return (
    <div className="landing-detail">
      <div className="landing-detail__top">
        <button type="button" className="landing-back" onClick={onBack}>
          ← {t("landing.back_to_list")}
        </button>
        {!isPreview && onDelete && (
          <button
            type="button"
            className="landing-delete"
            onClick={onDelete}
            title={t("landing.delete")}
          >
            🗑 {t("landing.delete")}
          </button>
        )}
      </div>

      <div className="landing-headline">
        <div
          className="landing-grade-big"
          style={{ background: gradeColor(record.grade_letter) }}
        >
          {record.grade_letter}
        </div>
        <div className="landing-headline__text">
          <h2>
            {callsign} · {record.dpt_airport} → {record.arr_airport}
          </h2>
          <div className="landing-headline__sub">
            {record.score_label.toUpperCase()} · {record.score_numeric}/100 ·{" "}
            {fmtDateTime(record.touchdown_at)}
            {isPreview && (
              <span className="landing-preview-badge">{t("landing.preview")}</span>
            )}
            {isNewBest && (
              <span className="landing-best-badge">★ {t("landing.new_best")}</span>
            )}
          </div>
          {record.aircraft_title && (
            <div className="landing-headline__aircraft">
              {record.aircraft_title}
              {record.aircraft_registration ? ` · ${record.aircraft_registration}` : ""}
              {record.aircraft_icao ? ` · ${record.aircraft_icao}` : ""}
              {record.sim_kind ? ` · ${record.sim_kind}` : ""}
            </div>
          )}
          {personalBest && !isNewBest && (
            <div className="landing-headline__pb">
              {t("landing.this_landing")}: {record.landing_rate_fpm.toFixed(0)} fpm ·{" "}
              {t("landing.personal_best")}: {personalBest.landing_rate_fpm.toFixed(0)}{" "}
              fpm ({personalBest.dpt_airport} → {personalBest.arr_airport})
            </div>
          )}
        </div>
      </div>

      {/* Score breakdown — most important new section */}
      <section className="landing-section">
        <h3>
          {t("landing.score_breakdown")}
          <InfoBadge explanation={t("landing.info.score_section")} />
        </h3>
        <ScoreBreakdown subs={subs} />
        <CoachTip subs={subs} />
      </section>

      {/* Touchdown: V/S curve + vitals + Wind compass (consolidated) */}
      <section className="landing-section">
        <h3>{t("landing.touchdown")}</h3>
        <div className="landing-grid landing-grid--td">
          <VsCurveChart profile={record.touchdown_profile} />
          <dl className="landing-keyvals">
            <div>
              <dt>{t("landing.landing_rate")}</dt>
              <dd>{fmtNumber(record.landing_rate_fpm, 0, "fpm")}</dd>
            </div>
            <div>
              <dt>{t("landing.peak_vs")}</dt>
              <dd>{fmtNumber(record.landing_peak_vs_fpm, 0, "fpm")}</dd>
            </div>
            <div>
              <dt>{t("landing.g_force")}</dt>
              <dd>{fmtNumber(record.landing_g_force, 2, "G")}</dd>
            </div>
            <div>
              <dt>{t("landing.peak_g")}</dt>
              <dd>{fmtNumber(record.landing_peak_g_force, 2, "G")}</dd>
            </div>
            <div>
              <dt>{t("landing.pitch")}</dt>
              <dd>{fmtSigned(record.landing_pitch_deg, 1, "°")}</dd>
            </div>
            <div>
              <dt>{t("landing.bank")}</dt>
              <dd>{fmtSigned(record.landing_bank_deg, 1, "°")}</dd>
            </div>
            <div>
              <dt>{t("landing.speed")}</dt>
              <dd>{fmtNumber(record.landing_speed_kt, 0, "kt")}</dd>
            </div>
            <div>
              <dt>{t("landing.sideslip")}</dt>
              <dd>{fmtSigned(record.touchdown_sideslip_deg, 1, "°")}</dd>
            </div>
            <div>
              <dt>{t("landing.bounces")}</dt>
              <dd>{record.bounce_count}</dd>
            </div>
            <div>
              <dt>{t("landing.heading")}</dt>
              <dd>{fmtNumber(record.landing_heading_deg, 0, "°")}</dd>
            </div>
          </dl>
          <WindCompass
            headwindKt={record.headwind_kt}
            crosswindKt={record.crosswind_kt}
          />
        </div>
      </section>

      {/* Approach stability — full-width, with chart underneath the bands */}
      <section className="landing-section">
        <h3>{t("landing.approach_stability")}</h3>
        <div className="landing-stability-row">
          <StabilityIndicator
            vsStd={record.approach_vs_stddev_fpm}
            bankStd={record.approach_bank_stddev_deg}
          />
        </div>
        {record.approach_samples.length >= 3 && (
          <div className="landing-stability-chart">
            <ApproachChart samples={record.approach_samples} />
          </div>
        )}
      </section>

      {/* Runway */}
      {record.runway_match && (
        <section className="landing-section">
          <h3>
            {t("landing.runway")}
            <InfoBadge explanation={t("landing.info.runway_section")} />
          </h3>
          <RunwayDiagram
            rw={record.runway_match}
            rolloutDistanceM={record.rollout_distance_m}
          />
          <dl className="landing-keyvals landing-keyvals--inline">
            <div>
              <dt>{t("landing.runway_id")}</dt>
              <dd>
                {record.runway_match.airport_ident}/{record.runway_match.runway_ident}{" "}
                ({record.runway_match.surface})
              </dd>
            </div>
            <div>
              <dt>{t("landing.runway_length")}</dt>
              <dd>
                {(record.runway_match.length_ft * 0.3048).toFixed(0)} m
              </dd>
            </div>
            <div>
              <dt>{t("landing.centerline_offset")}</dt>
              <dd>
                {Math.abs(record.runway_match.centerline_distance_m).toFixed(1)} m{" "}
                {t(sideKey(record.runway_match.side))}
              </dd>
            </div>
            <div>
              <dt>{t("landing.past_threshold")}</dt>
              <dd>
                {(record.runway_match.touchdown_distance_from_threshold_ft * 0.3048).toFixed(0)} m
              </dd>
            </div>
            {record.rollout_distance_m != null && (
              <div>
                <dt>{t("landing.rollout")}</dt>
                <dd>{record.rollout_distance_m.toFixed(0)} m</dd>
              </div>
            )}
            {record.runway_match.length_ft > 0 &&
              record.rollout_distance_m != null && (
                <div>
                  <dt>{t("landing.runway_used_pct")}</dt>
                  <dd>
                    {(
                      ((record.rollout_distance_m * 3.28084) /
                        record.runway_match.length_ft) *
                      100
                    ).toFixed(0)}
                    %
                  </dd>
                </div>
              )}
          </dl>
        </section>
      )}

      {/* Fuel + Weight — Soll/Ist-Vergleich (v0.3.0).
          Render whenever ANY fuel/weight value is present. */}
      {(record.planned_burn_kg != null ||
        record.actual_trip_burn_kg != null ||
        record.block_fuel_kg != null ||
        record.takeoff_fuel_kg != null ||
        record.landing_fuel_kg != null ||
        record.takeoff_weight_kg != null ||
        record.landing_weight_kg != null) && (
        <section className="landing-section">
          <h3>
            {t("landing.fuel")}
            <InfoBadge explanation={t("landing.info.fuel_section")} />
          </h3>
          {/* v0.4.2: Hinweis wenn der Flug keinen SimBrief-OFP hatte —
              dann sind alle SOLL-Spalten in den Tabellen unten leer.
              Banner erklärt klar warum, statt nur ratlose Striche. */}
          {record.planned_block_fuel_kg == null &&
            record.planned_burn_kg == null &&
            record.planned_tow_kg == null &&
            record.planned_ldw_kg == null &&
            record.planned_zfw_kg == null && (
              <div className="landing-no-plan-hint" role="note">
                ℹ️ {t("landing.no_plan_hint")}
              </div>
            )}
          {record.planned_burn_kg != null && record.actual_trip_burn_kg != null && (
            <FuelComparisonBar
              plan={record.planned_burn_kg}
              actual={record.actual_trip_burn_kg}
            />
          )}
          <ComparisonTable
            title={t("landing.fuel_table")}
            rows={[
              {
                label: t("landing.block_fuel"),
                ist: record.block_fuel_kg,
                soll: record.planned_block_fuel_kg,
              },
              {
                label: t("landing.takeoff_fuel"),
                ist: record.takeoff_fuel_kg,
                soll: null, // SimBrief OFP hat nur Block + Burn, kein TO-Fuel separat
              },
              {
                label: t("landing.landing_fuel"),
                ist: record.landing_fuel_kg,
                soll: record.planned_block_fuel_kg != null && record.planned_burn_kg != null
                  ? record.planned_block_fuel_kg - record.planned_burn_kg
                  : null,
              },
              {
                label: t("landing.trip_burn"),
                ist: record.actual_trip_burn_kg,
                soll: record.planned_burn_kg,
              },
            ]}
          />
          <ComparisonTable
            title={t("landing.weight_table")}
            rows={[
              {
                label: t("landing.tow"),
                ist: record.takeoff_weight_kg,
                soll: record.planned_tow_kg,
              },
              {
                label: t("landing.ldw"),
                ist: record.landing_weight_kg,
                soll: record.planned_ldw_kg,
              },
              {
                label: t("landing.zfw"),
                ist: record.takeoff_weight_kg != null && record.takeoff_fuel_kg != null
                  ? record.takeoff_weight_kg - record.takeoff_fuel_kg
                  : null,
                soll: record.planned_zfw_kg,
              },
            ]}
          />
          <LoadsheetScore record={record} />
        </section>
      )}
    </div>
  );
}

// ---- Loadsheet-Bewertung (v0.3.0) ----------------------------------------
//
// Numerischer Score 0-100 basierend auf Abweichungen Plan vs. IST. Nicht
// blockierend — nur Information für den Piloten ("nächstes Mal weniger
// Reserve-Sprit"). Score wird auch im PIREP-Custom-Field gepostet.
//
// Algorithmus:
// - Start bei 100
// - Pro Wert (Block-Fuel, ZFW, TOW, LDW): Δ > 5 % → -5 Punkte
// - Pro Wert: Δ > 10 % → -15 Punkte (additiv: also bei 12% sind's -20)
// - Niemals < 0
// - Wenn keine Plan-Werte vorhanden, kein Score (komplette Sektion blendet aus)

interface LoadsheetScoreInput {
  block_fuel_kg: number | null;
  takeoff_weight_kg: number | null;
  landing_weight_kg: number | null;
  takeoff_fuel_kg: number | null;
  planned_block_fuel_kg: number | null;
  planned_tow_kg: number | null;
  planned_ldw_kg: number | null;
  planned_zfw_kg: number | null;
}

function LoadsheetScore({ record }: { record: LoadsheetScoreInput }) {
  const { t } = useTranslation();

  // Berechne Δ% für jeden vergleichbaren Wert.
  const items = [
    {
      label: "Block-Fuel",
      ist: record.block_fuel_kg,
      soll: record.planned_block_fuel_kg,
    },
    {
      label: "TOW",
      ist: record.takeoff_weight_kg,
      soll: record.planned_tow_kg,
    },
    {
      label: "LDW",
      ist: record.landing_weight_kg,
      soll: record.planned_ldw_kg,
    },
    {
      label: "ZFW",
      ist:
        record.takeoff_weight_kg != null && record.takeoff_fuel_kg != null
          ? record.takeoff_weight_kg - record.takeoff_fuel_kg
          : null,
      soll: record.planned_zfw_kg,
    },
  ];

  // Nur Items mit beidem vergleichbar.
  const comparable = items.filter(
    (i) => i.ist != null && i.soll != null && i.soll > 0,
  );

  if (comparable.length === 0) return null; // Kein Plan → keine Bewertung

  // Score berechnen + Penalty-Liste sammeln für Anzeige.
  let score = 100;
  const breakdown: Array<{ label: string; pct: number; penalty: number }> = [];
  for (const item of comparable) {
    const ist = item.ist!;
    const soll = item.soll!;
    const pct = Math.abs((ist - soll) / soll) * 100;
    let penalty = 0;
    if (pct > 10) penalty += 15;
    else if (pct > 5) penalty += 5;
    score -= penalty;
    breakdown.push({ label: item.label, pct, penalty });
  }
  score = Math.max(0, score);

  // Score-Farbe.
  let scoreClass = "loadsheet-score__value--ok";
  if (score < 70) scoreClass = "loadsheet-score__value--alert";
  else if (score < 90) scoreClass = "loadsheet-score__value--warn";

  return (
    <div className="loadsheet-score">
      <div className="loadsheet-score__header">
        <span className="loadsheet-score__title">
          {t("landing.loadsheet_score")}
        </span>
        <span className={`loadsheet-score__value ${scoreClass}`}>
          {score}/100
        </span>
      </div>
      <ul className="loadsheet-score__breakdown">
        {breakdown.map((b) => (
          <li key={b.label}>
            <span>{b.label}</span>
            <span
              className={
                b.pct < 5
                  ? "loadsheet-score__pct--ok"
                  : b.pct < 10
                  ? "loadsheet-score__pct--warn"
                  : "loadsheet-score__pct--alert"
              }
            >
              {b.pct < 5 ? "✓" : b.pct < 10 ? "⚠" : "✕"}{" "}
              {b.pct >= 0.05 ? `${b.pct.toFixed(1)}%` : "0%"}
              {b.penalty > 0 && ` (-${b.penalty})`}
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}

// ---- Soll/Ist-Vergleichstabelle (v0.3.0) ---------------------------------
//
// Drei Spalten: IST | SOLL | Δ. Zeilen werden nur gerendert wenn IST oder
// SOLL vorhanden ist — leere Zeilen kommen NICHT in die Tabelle. Δ wird mit
// Farbcode versehen: grün <1%, gelb 1-3%, rot >3% Abweichung. Bei Weight
// gilt "drüber Plan = warnung", bei Fuel gilt "stark unter Plan = warnung
// (zu wenig getankt)".

interface ComparisonRow {
  label: string;
  ist: number | null;
  soll: number | null;
}

function ComparisonTable({ title, rows }: { title: string; rows: ComparisonRow[] }) {
  // Filter Zeilen die weder IST noch SOLL haben.
  const visible = rows.filter((r) => r.ist != null || r.soll != null);
  if (visible.length === 0) return null;
  return (
    <div className="landing-comparison">
      <div className="landing-comparison__title">{title}</div>
      <table className="landing-comparison__table">
        <thead>
          <tr>
            <th />
            <th>IST</th>
            <th>SOLL</th>
            <th>Δ</th>
          </tr>
        </thead>
        <tbody>
          {visible.map((r) => {
            const delta = r.ist != null && r.soll != null ? r.ist - r.soll : null;
            const deltaPct =
              delta != null && r.soll != null && r.soll !== 0
                ? Math.abs(delta / r.soll) * 100
                : null;
            // Farbcode (v0.3.0 Schwellen, praxisnah für Flugbetrieb):
            //   < 5 %   → grün  (im Rahmen normaler Operations)
            //   5-10 %  → gelb  (erkennbare Abweichung, normal)
            //   > 10 %  → rot   (substantiell, sollte begründet werden)
            let deltaClass = "";
            if (deltaPct != null) {
              if (deltaPct < 5) deltaClass = "landing-comparison__delta--ok";
              else if (deltaPct < 10) deltaClass = "landing-comparison__delta--warn";
              else deltaClass = "landing-comparison__delta--alert";
            }
            return (
              <tr key={r.label}>
                <td>{r.label}</td>
                <td>{r.ist != null ? fmtNumber(r.ist, 0, "kg") : "—"}</td>
                <td>{r.soll != null ? fmtNumber(r.soll, 0, "kg") : "—"}</td>
                <td className={deltaClass}>
                  {delta != null
                    ? `${delta >= 0 ? "+" : ""}${delta.toFixed(0)} kg`
                    : "—"}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

// ---- Stats summary across all landings ----------------------------------

function HistoryStats({ records }: { records: LandingRecord[] }) {
  const { t } = useTranslation();
  const stats = useMemo(() => {
    if (records.length === 0) return null;
    const total = records.length;
    const avgScore =
      records.reduce((s, r) => s + r.score_numeric, 0) / total;
    const bestRate = records.reduce(
      (best, r) =>
        Math.abs(r.landing_rate_fpm) < Math.abs(best.landing_rate_fpm) ? r : best,
      records[0],
    );
    const aGrades = records.filter(
      (r) => r.grade_letter === "A+" || r.grade_letter === "A",
    ).length;
    const totalBounces = records.reduce((s, r) => s + r.bounce_count, 0);
    return { total, avgScore, bestRate, aGrades, totalBounces };
  }, [records]);

  if (!stats) return null;

  return (
    <div className="landing-stats">
      <div className="landing-stat">
        <div className="landing-stat__label">{t("landing.total")}</div>
        <div className="landing-stat__value">{stats.total}</div>
      </div>
      <div className="landing-stat">
        <div className="landing-stat__label">{t("landing.avg_score")}</div>
        <div className="landing-stat__value">{stats.avgScore.toFixed(1)}</div>
      </div>
      <div className="landing-stat">
        <div className="landing-stat__label">{t("landing.a_grades")}</div>
        <div className="landing-stat__value">{stats.aGrades}</div>
      </div>
      <div className="landing-stat">
        <div className="landing-stat__label">{t("landing.best_rate")}</div>
        <div className="landing-stat__value">
          {stats.bestRate.landing_rate_fpm.toFixed(0)} fpm
        </div>
      </div>
      <div className="landing-stat">
        <div className="landing-stat__label">{t("landing.bounces")}</div>
        <div className="landing-stat__value">{stats.totalBounces}</div>
      </div>
    </div>
  );
}

// ---- Main panel ---------------------------------------------------------

export function LandingPanel() {
  const { t } = useTranslation();
  const { confirm, dialog: confirmDialog } = useConfirm();
  const [records, setRecords] = useState<LandingRecord[]>([]);
  const [preview, setPreview] = useState<LandingRecord | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  async function refresh() {
    setLoading(true);
    try {
      const [list, current] = await Promise.all([
        invoke<LandingRecord[]>("landing_list"),
        invoke<LandingRecord | null>("landing_get_current"),
      ]);
      setRecords(list);
      setPreview(current ?? null);
    } catch (e) {
      console.warn("landing_list failed", e);
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void refresh();
    // Refresh the preview every 5 s while we're on this tab so the
    // pilot sees their landing scores updating live during rollout.
    const t = setInterval(refresh, 5000);
    return () => clearInterval(t);
  }, []);

  async function handleDelete(id: string) {
    if (
      !(await confirm({
        message: t("landing.confirm_delete"),
        destructive: true,
      }))
    )
      return;
    try {
      await invoke("landing_delete", { pirepId: id });
      setSelectedId(null);
      await refresh();
    } catch (e) {
      console.warn("landing_delete failed", e);
    }
  }

  // Detail view
  if (selectedId) {
    const rec = records.find((r) => r.pirep_id === selectedId);
    if (rec) {
      return (
        <section className="phase landing-panel">
          {confirmDialog}
          <LandingDetail
            record={rec}
            allRecords={records}
            onBack={() => setSelectedId(null)}
            onDelete={() => handleDelete(rec.pirep_id)}
            isPreview={false}
          />
        </section>
      );
    }
  }

  // Preview-only state (active flight has touched down but record not yet filed)
  return (
    <section className="phase landing-panel">
      {confirmDialog}
      {preview && (
        <div className="landing-preview-card">
          <h3>{t("landing.live_preview")}</h3>
          <LandingDetail
            record={preview}
            allRecords={records}
            onBack={() => setPreview(null)}
            isPreview={true}
          />
        </div>
      )}

      <h2 className="landing-history-title">{t("landing.history")}</h2>
      <HistoryStats records={records} />
      <TrendSparkline records={records} />

      {loading && records.length === 0 && (
        <p className="landing-empty">{t("landing.loading")}</p>
      )}
      {!loading && records.length === 0 && !preview && (
        <p className="landing-empty">{t("landing.no_landings")}</p>
      )}

      {records.length > 0 && (
        <table className="landing-table">
          <thead>
            <tr>
              <th>{t("landing.col_grade")}</th>
              <th>{t("landing.col_when")}</th>
              <th>{t("landing.col_callsign")}</th>
              <th>{t("landing.col_route")}</th>
              <th>{t("landing.col_aircraft")}</th>
              <th>{t("landing.col_rate")}</th>
              <th>{t("landing.col_score")}</th>
            </tr>
          </thead>
          <tbody>
            {records.map((r) => (
              <tr
                key={r.pirep_id}
                className="landing-row"
                onClick={() => setSelectedId(r.pirep_id)}
                tabIndex={0}
              >
                <td>
                  <span
                    className="landing-grade-pill"
                    style={{ background: gradeColor(r.grade_letter) }}
                  >
                    {r.grade_letter}
                  </span>
                </td>
                <td>{fmtDateTime(r.touchdown_at)}</td>
                <td>
                  {r.airline_icao}
                  {r.flight_number}
                </td>
                <td>
                  {r.dpt_airport} → {r.arr_airport}
                </td>
                <td>
                  {r.aircraft_icao || r.aircraft_registration || r.aircraft_title || "—"}
                </td>
                <td>{r.landing_rate_fpm.toFixed(0)} fpm</td>
                <td>{r.score_numeric}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  );
}
