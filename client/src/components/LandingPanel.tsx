import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";

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
      {/* X axis labels */}
      <text
        x={pad.left}
        y={h - 8}
        fontSize="10"
        fill="currentColor"
      >
        {(tMin / 1000).toFixed(1)}s
      </text>
      <text
        x={pad.left + innerW}
        y={h - 8}
        textAnchor="end"
        fontSize="10"
        fill="currentColor"
      >
        {(tMax / 1000).toFixed(1)}s
      </text>
      <text
        x={x(td.t_ms)}
        y={h - 8}
        textAnchor="middle"
        fontSize="10"
        fill="#facc15"
      >
        TD
      </text>
    </svg>
  );
}

// ---- Runway diagram ----------------------------------------------------

function RunwayDiagram({ rw }: { rw: LandingRunwayMatch }) {
  const { t } = useTranslation();
  const w = 480;
  const h = 120;
  // Runway band
  const rwLeft = 30;
  const rwRight = w - 30;
  const rwTop = h / 2 - 18;
  const rwBottom = h / 2 + 18;
  const lengthFt = rw.length_ft;
  const tdFromThresh = rw.touchdown_distance_from_threshold_ft;
  // Map threshold→far-end onto the rect.
  const tdFrac = Math.min(1, Math.max(0, tdFromThresh / Math.max(1, lengthFt)));
  const tdX = rwLeft + tdFrac * (rwRight - rwLeft);

  // Centerline offset → vertical Y inside the strip (max ±1.5 widths)
  const offsetM = rw.centerline_distance_m;
  const widthM = 45; // assume ~45 m runway width if not exposed
  const offFrac = Math.max(-1, Math.min(1, offsetM / widthM));
  const tdY = (rwTop + rwBottom) / 2 + offFrac * 14;

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
      {/* Labels */}
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
        {rw.length_ft.toFixed(0)} ft
      </text>
      <text
        x={tdX}
        y={rwBottom + 14}
        textAnchor="middle"
        fontSize="10"
        fill="#22d3ee"
      >
        TD · {tdFromThresh.toFixed(0)} ft past thresh
      </text>
      <text
        x={tdX}
        y={rwBottom + 26}
        textAnchor="middle"
        fontSize="10"
        fill="currentColor"
      >
        {Math.abs(rw.centerline_distance_m).toFixed(1)} m {rw.side.toLowerCase()}
      </text>
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

  // Direction relative to runway: angle from nose
  const totalKt = Math.sqrt(hw * hw + xw * xw);
  // atan2(xw, hw) — xw > 0 = from right, hw > 0 = from front
  const angleRad = Math.atan2(xw, hw);
  const r = 38;
  const cx = 50;
  const cy = 50;
  const ax = cx + Math.sin(angleRad) * r;
  const ay = cy - Math.cos(angleRad) * r;

  return (
    <svg
      className="landing-wind"
      viewBox="0 0 100 100"
      preserveAspectRatio="xMidYMid meet"
      role="img"
      aria-label={t("landing.wind")}
    >
      <circle
        cx={cx}
        cy={cy}
        r={r + 4}
        fill="rgba(255,255,255,0.04)"
        stroke="rgba(255,255,255,0.2)"
      />
      {/* Aircraft nose pointing up */}
      <polygon points={`${cx},${cy - 6} ${cx - 4},${cy + 6} ${cx + 4},${cy + 6}`} fill="#a3a3a3" />
      {/* Wind arrow */}
      <line
        x1={ax}
        y1={ay}
        x2={cx}
        y2={cy}
        stroke="#38bdf8"
        strokeWidth="2"
        markerEnd="url(#wind-arrow)"
      />
      <defs>
        <marker
          id="wind-arrow"
          markerWidth="6"
          markerHeight="6"
          refX="3"
          refY="3"
          orient="auto"
        >
          <path d="M0,0 L6,3 L0,6 z" fill="#38bdf8" />
        </marker>
      </defs>
      <text x={cx} y={cy + r + 14} textAnchor="middle" fontSize="9" fill="currentColor">
        {totalKt.toFixed(0)} kt
      </text>
      <text x={cx} y={cy + r + 24} textAnchor="middle" fontSize="8" fill="currentColor">
        H {hw.toFixed(0)} · X {xw.toFixed(0)}
      </text>
    </svg>
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
  onBack,
  onDelete,
  isPreview,
}: {
  record: LandingRecord;
  onBack: () => void;
  onDelete?: () => void;
  isPreview: boolean;
}) {
  const { t } = useTranslation();

  const callsign = record.airline_icao
    ? `${record.airline_icao}${record.flight_number}`
    : record.flight_number;

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
          </div>
          {record.aircraft_title && (
            <div className="landing-headline__aircraft">
              {record.aircraft_title}
              {record.aircraft_registration ? ` · ${record.aircraft_registration}` : ""}
              {record.aircraft_icao ? ` · ${record.aircraft_icao}` : ""}
              {record.sim_kind ? ` · ${record.sim_kind}` : ""}
            </div>
          )}
        </div>
      </div>

      {/* Top row: V/S curve + key metrics */}
      <section className="landing-section">
        <h3>{t("landing.touchdown")}</h3>
        <div className="landing-grid landing-grid--2">
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
        </div>
      </section>

      {/* Approach + Wind row */}
      <section className="landing-section">
        <h3>{t("landing.approach_stability")}</h3>
        <div className="landing-grid landing-grid--2">
          <StabilityIndicator
            vsStd={record.approach_vs_stddev_fpm}
            bankStd={record.approach_bank_stddev_deg}
          />
          <WindCompass
            headwindKt={record.headwind_kt}
            crosswindKt={record.crosswind_kt}
          />
        </div>
      </section>

      {/* Runway */}
      {record.runway_match && (
        <section className="landing-section">
          <h3>{t("landing.runway")}</h3>
          <RunwayDiagram rw={record.runway_match} />
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
              <dd>{fmtNumber(record.runway_match.length_ft, 0, "ft")}</dd>
            </div>
            <div>
              <dt>{t("landing.centerline_offset")}</dt>
              <dd>
                {Math.abs(record.runway_match.centerline_distance_m).toFixed(1)} m{" "}
                {record.runway_match.side.toLowerCase()}
              </dd>
            </div>
            <div>
              <dt>{t("landing.past_threshold")}</dt>
              <dd>
                {fmtNumber(
                  record.runway_match.touchdown_distance_from_threshold_ft,
                  0,
                  "ft",
                )}
              </dd>
            </div>
            {record.rollout_distance_m != null && (
              <div>
                <dt>{t("landing.rollout")}</dt>
                <dd>{record.rollout_distance_m.toFixed(0)} m</dd>
              </div>
            )}
          </dl>
        </section>
      )}

      {/* Fuel + weights */}
      {(record.planned_burn_kg != null || record.actual_trip_burn_kg != null) && (
        <section className="landing-section">
          <h3>{t("landing.fuel")}</h3>
          {record.planned_burn_kg != null && record.actual_trip_burn_kg != null && (
            <FuelComparisonBar
              plan={record.planned_burn_kg}
              actual={record.actual_trip_burn_kg}
            />
          )}
          <dl className="landing-keyvals landing-keyvals--inline">
            {record.block_fuel_kg != null && (
              <div>
                <dt>{t("landing.block_fuel")}</dt>
                <dd>{fmtNumber(record.block_fuel_kg, 0, "kg")}</dd>
              </div>
            )}
            {record.takeoff_fuel_kg != null && (
              <div>
                <dt>{t("landing.takeoff_fuel")}</dt>
                <dd>{fmtNumber(record.takeoff_fuel_kg, 0, "kg")}</dd>
              </div>
            )}
            {record.landing_fuel_kg != null && (
              <div>
                <dt>{t("landing.landing_fuel")}</dt>
                <dd>{fmtNumber(record.landing_fuel_kg, 0, "kg")}</dd>
              </div>
            )}
            {record.takeoff_weight_kg != null && (
              <div>
                <dt>{t("landing.tow")}</dt>
                <dd>{fmtNumber(record.takeoff_weight_kg, 0, "kg")}</dd>
              </div>
            )}
            {record.landing_weight_kg != null && (
              <div>
                <dt>{t("landing.ldw")}</dt>
                <dd>{fmtNumber(record.landing_weight_kg, 0, "kg")}</dd>
              </div>
            )}
            {record.planned_tow_kg != null && (
              <div>
                <dt>{t("landing.plan_tow")}</dt>
                <dd>{fmtNumber(record.planned_tow_kg, 0, "kg")}</dd>
              </div>
            )}
            {record.planned_ldw_kg != null && (
              <div>
                <dt>{t("landing.plan_ldw")}</dt>
                <dd>{fmtNumber(record.planned_ldw_kg, 0, "kg")}</dd>
              </div>
            )}
          </dl>
        </section>
      )}
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
    if (!window.confirm(t("landing.confirm_delete"))) return;
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
          <LandingDetail
            record={rec}
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
      {preview && (
        <div className="landing-preview-card">
          <h3>{t("landing.live_preview")}</h3>
          <LandingDetail
            record={preview}
            onBack={() => setPreview(null)}
            isPreview={true}
          />
        </div>
      )}

      <h2 className="landing-history-title">{t("landing.history")}</h2>
      <HistoryStats records={records} />

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
