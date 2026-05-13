// Runway Diagram v2 — Display-Only Polish nach v0.8.0.
//
// Spec: docs/spec/runway-diagram-v2.contract.md
//
// Pure-Display-Component, KEIN neues Scoring, KEINE neuen Wire-Felder.
// Nutzt ausschließlich existing v0.8.0-Felder aus LandingRecord oder
// TouchdownDto.payload (Wire-symmetrisch). Pilot-Client liest aus
// `landings.json` (lokal persistiert, kein VPS-Fetch nötig). Webapp
// kann später dieselbe Component importieren.
//
// Layout (4 Bereiche full-width):
//   1. Header — Airport/RWY/Length/Source + Hilfe-Button
//   2. SVG-Diagramm (viewBox 1200x320, responsive)
//   3. Legende
//   4. 4 Detail-Karten (Aufsetz-Bewertung / Position / Anflug-Profil / Datenquelle)

import { useMemo, useState } from "react";
import { GlossaryModal } from "./RunwayGlossaryModal";

// ─── Public types ───────────────────────────────────────────────────

export type AimClass =
  | "perfect"
  | "short_of_aim"
  | "past_aim"
  | "long_landing"
  | "severe";

export type TchClass =
  | "on_profile"
  | "slightly_low"
  | "slightly_high"
  | "high"
  | "below_profile";

export interface RunwayDiagramV2Props {
  airport_ident: string;
  airport_name?: string | null;
  runway_ident: string;
  length_m: number;
  surface?: string | null;
  source: "navigraph" | "ourairports_fallback" | null;
  nav_cycle?: string | null;
  displaced_threshold_m?: number;
  td_distance_from_threshold_m: number;
  td_centerline_offset_m: number;
  td_in_tdz?: boolean | null;
  td_third?: 1 | 2 | 3 | null;
  td_tdz_length_m?: number | null;
  aim_point_m?: number | null;
  aim_delta_m?: number | null;
  aim_class?: AimClass | null;
  tch_actual_ft?: number | null;
  tch_expected_ft?: number | null;
  tch_delta_ft?: number | null;
  tch_class?: TchClass | null;
  pre_displaced_threshold?: boolean | null;
  rollout_m?: number | null;
  locale?: "de" | "en" | "it";
}

// ─── Visual tokens ───────────────────────────────────────────────────

const TOKENS = {
  svgWidth: 1200,
  // v2.1: H 320→380, padY 70→95 — frühere Versionen schnitten die
  // Skala-Labels am unteren SVG-Rand ab (rwyBot+70 fiel exakt auf
  // viewBox-Bottom). Jetzt 40 px Margin unten + 25 px mehr oben für
  // die ZIEL-Annotation.
  svgHeight: 380,
  rwyPaddingX: 70,
  rwyPaddingY: 95,
  tarmac: "#1a2030",
  tarmacBorder: "rgba(255,255,255,0.18)",
  threshold: "rgba(255,255,255,0.85)",
  centerline: "rgba(255,255,255,0.5)",
  centerlineDashArray: "14,10",
  tdzFill: "rgba(253,224,138,0.18)",
  tdzStroke: "rgba(253,224,138,0.55)",
  aimMarker: "#fbbf24",
  rollout: "#22d3ee",
  rolloutGlow: "rgba(34,211,238,0.18)",
  exitDot: "#f59e0b",
  ddsZone: "rgba(124,45,18,0.45)",
  ddsBorder: "rgba(220,38,38,0.65)",
  tdPerfect: "#22c55e",
  tdAcceptable: "#22d3ee",
  tdWarn: "#fbbf24",
  tdSevere: "#ef4444",
} as const;

function tdColor(p: RunwayDiagramV2Props): string {
  if (p.pre_displaced_threshold === true) return TOKENS.tdSevere;
  switch (p.aim_class) {
    case "perfect":
    case "past_aim":
    case "short_of_aim":
      return TOKENS.tdPerfect;
    case "long_landing":
      return TOKENS.tdWarn;
    case "severe":
      return TOKENS.tdSevere;
    default:
      return TOKENS.tdAcceptable;
  }
}

// ─── Component ───────────────────────────────────────────────────────

export function RunwayDiagramV2(props: RunwayDiagramV2Props) {
  const [glossaryOpen, setGlossaryOpen] = useState(false);

  const W = TOKENS.svgWidth;
  const H = TOKENS.svgHeight;
  const padX = TOKENS.rwyPaddingX;
  const padY = TOKENS.rwyPaddingY;
  const innerW = W - 2 * padX;
  const innerH = H - 2 * padY;
  const rwyTop = padY;
  const rwyBot = padY + innerH;
  const rwyCl = (rwyTop + rwyBot) / 2;

  // Meter → X-Pixel auf der Bahn.
  const lengthM = Math.max(500, props.length_m);
  const mToX = (m: number) =>
    padX + (Math.max(-200, Math.min(lengthM, m)) / lengthM) * innerW;

  // Centerline-Offset → Y. ±widthM/2 → ±(innerH/2 - safetyMargin).
  // widthM = 45 m typisch, aber wir stretchen für Sichtbarkeit (sonst
  // wäre ±2 m visuell unsichtbar).
  const widthM = 45;
  const yMaxOffset = innerH / 2 - 20;
  const clampedOffset = Math.max(
    -widthM / 2,
    Math.min(widthM / 2, props.td_centerline_offset_m),
  );
  const tdY = rwyCl + (clampedOffset / (widthM / 2)) * yMaxOffset;
  const tdX = mToX(props.td_distance_from_threshold_m);
  const dotColor = tdColor(props);

  // Skala-Ticks anhand Bahn-Länge: 0/300/600/900/1200/1500/1800/2400 etc.
  const scaleTicks = useMemo(() => {
    const candidates = [0, 300, 600, 900, 1200, 1500, 1800, 2100, 2400, 3000, 3600, 4200];
    return candidates.filter((d) => d <= lengthM);
  }, [lengthM]);

  // Aim-Marker only when known + within bahn.
  const aimX =
    props.aim_point_m != null && props.aim_point_m > 0
      ? mToX(props.aim_point_m)
      : null;

  // TDZ-Box only when length covers the marker.
  const tdzEndX =
    props.td_tdz_length_m != null && props.td_tdz_length_m > 0
      ? mToX(props.td_tdz_length_m)
      : null;

  // DDS-Zone (Pre-Threshold-Markierung) — Renderpolicy: nur rendern
  // wenn displaced_threshold_m > 0. Die Zone wird LINKS der Threshold-
  // Linie gezeichnet (negative Distanz vom Threshold).
  const ddsM = props.displaced_threshold_m ?? 0;
  const ddsActive = ddsM > 0;

  // Rollout-Endpunkt
  const exitDistM =
    props.rollout_m != null
      ? Math.min(lengthM, props.td_distance_from_threshold_m + props.rollout_m)
      : null;
  const exitX = exitDistM != null ? mToX(exitDistM) : null;

  // Bahn-Auslastung
  const bahnUsedPct =
    props.rollout_m != null && lengthM > 0
      ? Math.min(100, ((props.td_distance_from_threshold_m + props.rollout_m) / lengthM) * 100)
      : null;

  // Source-Label — neutral Wording per Spec §Akzeptanz (Lizenz-Vorsicht):
  // UI sagt "VPS Navdata (AIRAC X)" statt direkt "Navigraph".
  const sourceLabel = (() => {
    if (props.source === "navigraph") {
      return `VPS Navdata (AIRAC ${props.nav_cycle ?? "?"}) ✓`;
    }
    if (props.source === "ourairports_fallback") {
      return "OurAirports (Fallback) — Schwellen-Position kann abweichen";
    }
    return "OurAirports";
  })();

  return (
    <section
      className="rwy-v2"
      aria-label="Landebahn-Analyse"
      style={{ width: "100%", display: "flex", flexDirection: "column", gap: 12 }}
    >
      {/* ─── 1. HEADER ─────────────────────────────────────────────── */}
      <header
        style={{
          display: "flex",
          alignItems: "flex-start",
          justifyContent: "space-between",
          gap: 16,
          padding: "12px 16px",
          background: "rgba(255,255,255,0.04)",
          borderRadius: 8,
          borderTop: "2px solid rgba(34,197,94,0.5)",
        }}
      >
        <div>
          <h3
            style={{
              fontSize: "1.15rem",
              margin: 0,
              marginBottom: 6,
              display: "flex",
              alignItems: "center",
              gap: 8,
              letterSpacing: 0.2,
            }}
          >
            🛬 Landebahn-Analyse
          </h3>
          <div
            style={{
              fontSize: "0.92rem",
              lineHeight: 1.55,
              opacity: 0.92,
            }}
          >
            <strong>{props.airport_ident}</strong>
            {props.airport_name ? ` (${props.airport_name})` : ""}
            {" · "}
            <strong>Bahn {props.runway_ident}</strong>
            {" · "}
            {props.length_m.toFixed(0)} m
            {props.surface ? ` · ${surfaceLabel(props.surface)}` : ""}
          </div>
          <div
            style={{
              fontSize: "0.82rem",
              opacity: props.source === "ourairports_fallback" ? 0.95 : 0.7,
              marginTop: 4,
              color:
                props.source === "ourairports_fallback" ? "#fbbf24" : undefined,
            }}
          >
            Datenquelle: {sourceLabel}
          </div>
        </div>
        <button
          type="button"
          onClick={() => setGlossaryOpen(true)}
          aria-label="Begriffe erklärt — Glossar öffnen"
          style={{
            padding: "6px 12px",
            background: "rgba(255,255,255,0.06)",
            border: "1px solid rgba(255,255,255,0.18)",
            borderRadius: 6,
            color: "inherit",
            cursor: "pointer",
            fontSize: "0.85rem",
            whiteSpace: "nowrap",
          }}
        >
          ⓘ Begriffe erklärt
        </button>
      </header>

      {/* ─── 2. SVG-DIAGRAMM ───────────────────────────────────────── */}
      <div
        style={{
          width: "100%",
          background: "rgba(0,0,0,0.25)",
          borderRadius: 8,
          padding: "12px 8px 4px 8px",
        }}
      >
        <svg
          viewBox={`0 0 ${W} ${H}`}
          preserveAspectRatio="xMidYMid meet"
          style={{ width: "100%", height: "auto", display: "block" }}
          role="img"
          aria-label="Bahn-Geometrie mit Aufsetzpunkt"
        >
          {/* Tarmac */}
          <rect
            x={padX}
            y={rwyTop}
            width={innerW}
            height={innerH}
            fill={TOKENS.tarmac}
            stroke={TOKENS.tarmacBorder}
            strokeWidth="1"
          />

          {/* DDS Pre-Threshold-Zone (vor dem padX wird visuell hinzugefügt
              durch Strich-Anhang links — bei displaced_threshold_m > 0). */}
          {ddsActive && (
            <g>
              <rect
                x={padX}
                y={rwyTop + 4}
                width={Math.max(0, mToX(ddsM) - padX)}
                height={innerH - 8}
                fill={TOKENS.ddsZone}
                stroke={TOKENS.ddsBorder}
                strokeDasharray="4,4"
                strokeWidth="1.2"
              >
                <title>
                  Pre-Threshold-Zone (Displaced Threshold, DDS): {ddsM.toFixed(0)} m
                  vor der Landeschwelle — Aufsetzen hier ist in der echten Welt
                  illegal (Hindernis-Clearance).
                </title>
              </rect>
              <text
                x={padX + 8}
                y={rwyBot - 8}
                fontSize="11"
                fill="#fca5a5"
                fontWeight="700"
                fontFamily="monospace"
              >
                DDS {ddsM.toFixed(0)} m
              </text>
            </g>
          )}

          {/* Threshold-Streifen (links der Bahn, Block aus 8 weißen
              Vertikal-Strichen). */}
          <g>
            {Array.from({ length: 8 }, (_, i) => (
              <rect
                key={i}
                x={padX + 4}
                y={rwyTop + 4 + (i * (innerH - 8)) / 8}
                width={20}
                height={(innerH - 8) / 8 - 2}
                fill={TOKENS.threshold}
              />
            ))}
            <title>
              Schwelle (Threshold) — Beginn des landbaren Bahn-Teils.
            </title>
          </g>

          {/* TDZ-Box (Schraffur-Pattern). */}
          {tdzEndX != null && tdzEndX > padX + 24 && (
            <g>
              <defs>
                <pattern
                  id="tdz-hatch"
                  patternUnits="userSpaceOnUse"
                  width="10"
                  height="10"
                  patternTransform="rotate(45)"
                >
                  <line
                    x1="0"
                    y1="0"
                    x2="0"
                    y2="10"
                    stroke={TOKENS.tdzStroke}
                    strokeWidth="2"
                  />
                </pattern>
              </defs>
              <rect
                x={padX + 24}
                y={rwyTop + 30}
                width={tdzEndX - padX - 24}
                height={innerH - 60}
                fill="url(#tdz-hatch)"
              />
              <rect
                x={padX + 24}
                y={rwyTop + 30}
                width={tdzEndX - padX - 24}
                height={innerH - 60}
                fill={TOKENS.tdzFill}
                stroke={TOKENS.tdzStroke}
                strokeDasharray="6,5"
                strokeWidth="1"
              >
                <title>
                  Aufsetzzone (Touchdown Zone, TDZ): erste {props.td_tdz_length_m?.toFixed(0)} m
                  der Bahn. Soll-Aufsetz-Bereich nach ICAO Annex 14.
                </title>
              </rect>
              <text
                x={padX + 24 + (tdzEndX - padX - 24) / 2}
                y={rwyTop + 18}
                fontSize="12"
                fill="#fde68a"
                fontWeight="700"
                fontFamily="monospace"
                textAnchor="middle"
              >
                AUFSETZZONE {props.td_tdz_length_m?.toFixed(0)} m
              </text>
            </g>
          )}

          {/* Centerline (gestrichelt). */}
          <line
            x1={padX + 28}
            y1={rwyCl}
            x2={padX + innerW - 6}
            y2={rwyCl}
            stroke={TOKENS.centerline}
            strokeWidth="1.6"
            strokeDasharray={TOKENS.centerlineDashArray}
          />

          {/* Aim-Marker (zwei Quadrate ober/unter CL + Pfeil). */}
          {aimX != null && (
            <g>
              <rect
                x={aimX - 14}
                y={rwyCl - 30}
                width={10}
                height={20}
                fill={TOKENS.aimMarker}
                opacity="0.95"
              />
              <rect
                x={aimX + 4}
                y={rwyCl - 30}
                width={10}
                height={20}
                fill={TOKENS.aimMarker}
                opacity="0.95"
              />
              <rect
                x={aimX - 14}
                y={rwyCl + 10}
                width={10}
                height={20}
                fill={TOKENS.aimMarker}
                opacity="0.95"
              />
              <rect
                x={aimX + 4}
                y={rwyCl + 10}
                width={10}
                height={20}
                fill={TOKENS.aimMarker}
                opacity="0.95"
              />
              <polygon
                points={`${aimX - 7},${rwyTop - 14} ${aimX + 7},${rwyTop - 14} ${aimX},${rwyTop - 4}`}
                fill={TOKENS.aimMarker}
              />
              <text
                x={aimX}
                y={rwyTop - 18}
                textAnchor="middle"
                fontSize="13"
                fill={TOKENS.aimMarker}
                fontWeight="700"
                fontFamily="monospace"
              >
                ZIEL {props.aim_point_m?.toFixed(0)} m
              </text>
              <title>
                Ziel-Markierung (Aim Point) — FAA AIM 8-9-1. Soll-Aufsetzpunkt
                bei {props.aim_point_m?.toFixed(0)} m hinter der Schwelle.
              </title>
            </g>
          )}

          {/* Rollout-Linie (Glow + Solid). */}
          {exitX != null && (
            <g>
              <line
                x1={tdX}
                y1={tdY}
                x2={exitX}
                y2={tdY}
                stroke={TOKENS.rolloutGlow}
                strokeWidth="14"
              />
              <line
                x1={tdX}
                y1={tdY}
                x2={exitX}
                y2={tdY}
                stroke={TOKENS.rollout}
                strokeWidth="3"
                opacity="0.75"
              />
            </g>
          )}

          {/* Offset-Indikator: senkrechter Pfeil von der Mittellinie zum
              TD-Dot. Macht das L/R-Offset visuell (statt nur über die
              y-Position) erkennbar. Nur wenn der Offset > 0.5 m ist —
              sonst überlappt der Pfeil mit dem Dot selbst. */}
          {Math.abs(props.td_centerline_offset_m) > 0.5 && (
            <g>
              {/* Senkrechte Linie CL → TD */}
              <line
                x1={tdX}
                y1={rwyCl}
                x2={tdX}
                y2={tdY > rwyCl ? tdY - 11 : tdY + 11}
                stroke={dotColor}
                strokeWidth="1.5"
                strokeDasharray="3,3"
                opacity="0.8"
              />
              {/* Pfeilspitze am TD-Ende */}
              <polygon
                points={
                  tdY > rwyCl
                    ? `${tdX},${tdY - 9} ${tdX - 4},${tdY - 14} ${tdX + 4},${tdY - 14}`
                    : `${tdX},${tdY + 9} ${tdX - 4},${tdY + 14} ${tdX + 4},${tdY + 14}`
                }
                fill={dotColor}
              />
              {/* Kleine L/R-Label neben dem Dot */}
              <text
                x={tdX + 14}
                y={tdY + 4}
                fontSize="11"
                fill={dotColor}
                fontWeight="800"
                fontFamily="monospace"
              >
                {props.td_centerline_offset_m > 0 ? "→ R" : "← L"}
                {" "}
                {Math.abs(props.td_centerline_offset_m).toFixed(1)} m
              </text>
            </g>
          )}

          {/* Touchdown-Punkt — Doppel-Glow + Solid Dot. */}
          <g>
            <circle cx={tdX} cy={tdY} r="22" fill={dotColor} opacity="0.10" />
            <circle cx={tdX} cy={tdY} r="14" fill={dotColor} opacity="0.22" />
            <circle cx={tdX} cy={tdY} r="9" fill={dotColor} stroke="#0c1628" strokeWidth="2" />
            <title>
              Aufsetzpunkt (Touchdown): {props.td_distance_from_threshold_m.toFixed(0)} m
              {props.td_distance_from_threshold_m < 0
                ? " vor"
                : " hinter"}{" "}
              Schwelle,{" "}
              {Math.abs(props.td_centerline_offset_m).toFixed(1)} m{" "}
              {props.td_centerline_offset_m > 0.5
                ? "rechts"
                : props.td_centerline_offset_m < -0.5
                ? "links"
                : "auf"}
              {" "}der Mittellinie.
            </title>
          </g>

          {/* Exit-Punkt (orange). */}
          {exitX != null && (
            <g>
              <circle cx={exitX} cy={tdY} r="11" fill={TOKENS.exitDot} opacity="0.25" />
              <circle cx={exitX} cy={tdY} r="6" fill={TOKENS.exitDot} stroke="#0c1628" strokeWidth="1.5" />
              <text
                x={exitX}
                y={tdY - 18}
                textAnchor="middle"
                fontSize="11"
                fill={TOKENS.exitDot}
                fontWeight="700"
                fontFamily="monospace"
              >
                EXIT
              </text>
              <title>
                Exit-Punkt — Ende der Ausrollstrecke (Rollout). Geschwindigkeit
                {" "}~40 kt, Pilot kann abbiegen.
              </title>
            </g>
          )}

          {/* RWY-Designator (groß links). */}
          <text
            x={padX / 2 - 4}
            y={rwyCl + 10}
            textAnchor="middle"
            fontSize="28"
            fill="#f1f5f9"
            fontWeight="800"
            fontFamily="monospace"
          >
            {props.runway_ident}
          </text>

          {/* Landerichtungs-Pfeil rechts — Doppel-Chevron für ICAO-look. */}
          <g>
            <polygon
              points={`${W - padX + 14},${rwyCl} ${W - padX - 8},${rwyCl - 22} ${W - padX - 8},${rwyCl + 22}`}
              fill="rgba(255,255,255,0.55)"
            />
            <polygon
              points={`${W - padX - 6},${rwyCl} ${W - padX - 28},${rwyCl - 22} ${W - padX - 28},${rwyCl + 22}`}
              fill="rgba(255,255,255,0.30)"
            />
            <title>Landerichtung</title>
          </g>

          {/* TD-Label unter dem Dot. */}
          <g>
            <text
              x={tdX}
              y={rwyBot + 18}
              textAnchor="middle"
              fontSize="12"
              fill={dotColor}
              fontWeight="700"
              fontFamily="monospace"
            >
              TD {props.td_distance_from_threshold_m.toFixed(0)} m
            </text>
            <text
              x={tdX}
              y={rwyBot + 32}
              textAnchor="middle"
              fontSize="11"
              fill={dotColor}
              fontFamily="monospace"
            >
              {Math.abs(props.td_centerline_offset_m) < 0.5
                ? "auf CL"
                : `${Math.abs(props.td_centerline_offset_m).toFixed(1)} m ${
                    props.td_centerline_offset_m > 0 ? "RECHTS" : "LINKS"
                  }`}
            </text>
          </g>

          {/* Distanz-Skala unter der Bahn. */}
          <g>
            <line
              x1={padX}
              y1={rwyBot + 52}
              x2={padX + innerW}
              y2={rwyBot + 52}
              stroke="rgba(255,255,255,0.25)"
              strokeWidth="1"
            />
            {scaleTicks.map((d) => {
              const x = mToX(d);
              return (
                <g key={d}>
                  <line
                    x1={x}
                    y1={rwyBot + 47}
                    x2={x}
                    y2={rwyBot + 57}
                    stroke="rgba(255,255,255,0.5)"
                    strokeWidth="1.2"
                  />
                  <text
                    x={x}
                    y={rwyBot + 70}
                    textAnchor="middle"
                    fontSize="10"
                    fill="rgba(255,255,255,0.55)"
                    fontFamily="monospace"
                  >
                    {d} m
                  </text>
                </g>
              );
            })}
          </g>
        </svg>
      </div>

      {/* ─── 3. LEGENDE ─────────────────────────────────────────────── */}
      <div
        style={{
          display: "flex",
          gap: 18,
          flexWrap: "wrap",
          fontSize: "0.78rem",
          opacity: 0.85,
          padding: "0 4px",
        }}
      >
        <LegendItem swatch={TOKENS.threshold} label="Schwelle" />
        {tdzEndX && <LegendItem swatch={TOKENS.tdzStroke} label="Aufsetzzone (TDZ)" />}
        {aimX && <LegendItem swatch={TOKENS.aimMarker} label="Ziel-Markierung (AIM)" />}
        <LegendDot color={dotColor} label="Aufsetzpunkt (TD)" />
        {exitX && <LegendDot color={TOKENS.exitDot} label="Exit" />}
        {ddsActive && <LegendItem swatch={TOKENS.ddsBorder} label="Pre-Threshold — Landung verboten" />}
      </div>

      {/* ─── 4. DETAIL-KARTEN ──────────────────────────────────────── */}
      <div
        style={{
          display: "grid",
          gridTemplateColumns: "repeat(auto-fit, minmax(220px, 1fr))",
          gap: 12,
        }}
      >
        <DetailCard title="Aufsetz-Bewertung" accent="#22c55e">
          {props.td_in_tdz != null && (
            <DetailRow
              tone={props.td_in_tdz ? "good" : "warn"}
              label="Aufsetzzone"
              value={
                props.td_in_tdz
                  ? `✓ ${props.td_third ? thirdLabel(props.td_third) : "im Marker"}`
                  : `✗ ${props.td_third ? thirdLabel(props.td_third) : "verfehlt"}`
              }
            />
          )}
          {props.aim_class && props.aim_delta_m != null && (
            <DetailRow
              tone={aimTone(props.aim_class)}
              label="Ziel-Markierung"
              value={`${aimClassLabel(props.aim_class)} · Δ ${props.aim_delta_m >= 0 ? "+" : ""}${props.aim_delta_m.toFixed(0)} m`}
            />
          )}
          {props.pre_displaced_threshold === true && (
            <DetailRow
              tone="bad"
              label="⚠ Pre-Threshold"
              value="Aufsetzen vor der Landeschwelle (illegal IRL)"
            />
          )}
          {props.td_in_tdz == null && props.aim_class == null && (
            <DetailRow tone="neutral" label="Bewertung" value="Pre-v0.8.0 — keine Daten" />
          )}
        </DetailCard>

        <DetailCard title="Position">
          <DetailRow
            label="Hinter Schwelle"
            value={`${props.td_distance_from_threshold_m.toFixed(0)} m`}
          />
          <DetailRow
            label="Mittellinie"
            value={
              Math.abs(props.td_centerline_offset_m) < 0.5
                ? "auf Mittellinie"
                : `${Math.abs(props.td_centerline_offset_m).toFixed(1)} m ${
                    props.td_centerline_offset_m > 0 ? "RECHTS" : "LINKS"
                  }`
            }
          />
          {props.rollout_m != null && (
            <DetailRow
              label="Ausrollen"
              value={`${props.rollout_m.toFixed(0)} m${bahnUsedPct != null ? ` (${bahnUsedPct.toFixed(0)} %)` : ""}`}
            />
          )}
        </DetailCard>

        {/* TCH-Card NUR rendern wenn actual vorhanden. Spec § Display-Polish. */}
        {props.tch_actual_ft != null && props.tch_class && (
          <DetailCard title="Anflug-Profil">
            <DetailRow
              label="Schwellen-Höhe (TCH)"
              value={`${props.tch_actual_ft.toFixed(0)} ft${props.tch_expected_ft != null ? ` (Soll ${props.tch_expected_ft})` : ""}`}
            />
            {props.tch_delta_ft != null && (
              <DetailRow
                tone={tchTone(props.tch_class)}
                label="Abweichung"
                value={`Δ ${props.tch_delta_ft >= 0 ? "+" : ""}${props.tch_delta_ft.toFixed(0)} ft · ${tchClassLabel(props.tch_class)}`}
              />
            )}
          </DetailCard>
        )}

        <DetailCard title="Datenquelle">
          {props.source === "navigraph" ? (
            <>
              <DetailRow
                tone="good"
                label="VPS Navdata ✓"
                value={`AIRAC ${props.nav_cycle ?? "?"}`}
              />
              <DetailRow
                label=""
                value="Zentrale AIRAC-Daten, vom VA-Admin gepflegt (±0.5 m)"
              />
            </>
          ) : props.source === "ourairports_fallback" ? (
            <>
              <DetailRow tone="warn" label="⚠ Fallback" value="OurAirports-Community-Daten" />
              <DetailRow
                label=""
                value="Schwellen-Position kann abweichen — Navigraph beim Flugstart nicht erreichbar"
              />
            </>
          ) : (
            <>
              <DetailRow tone="neutral" label="OurAirports" value="Community-Daten" />
              <DetailRow
                label=""
                value="Pre-v0.8.0 Flug — keine zentrale Navdata-Quelle"
              />
            </>
          )}
        </DetailCard>
      </div>

      {glossaryOpen && (
        <GlossaryModal onClose={() => setGlossaryOpen(false)} />
      )}
    </section>
  );
}

// ─── Kleine UI-Helpers ──────────────────────────────────────────────

function LegendItem({ swatch, label }: { swatch: string; label: string }) {
  return (
    <span style={{ display: "inline-flex", alignItems: "center", gap: 6 }}>
      <span
        style={{
          width: 14,
          height: 8,
          background: swatch,
          display: "inline-block",
          borderRadius: 2,
        }}
      />
      {label}
    </span>
  );
}

function LegendDot({ color, label }: { color: string; label: string }) {
  return (
    <span style={{ display: "inline-flex", alignItems: "center", gap: 6 }}>
      <span
        style={{
          width: 10,
          height: 10,
          background: color,
          borderRadius: 999,
          display: "inline-block",
        }}
      />
      {label}
    </span>
  );
}

function DetailCard({
  title,
  accent,
  children,
}: {
  title: string;
  accent?: string;
  children: React.ReactNode;
}) {
  return (
    <div
      style={{
        padding: "10px 12px",
        background: "rgba(255,255,255,0.04)",
        border: "1px solid rgba(255,255,255,0.10)",
        borderRadius: 8,
        borderLeft: accent ? `3px solid ${accent}` : undefined,
        display: "flex",
        flexDirection: "column",
        gap: 6,
      }}
    >
      <div
        style={{
          fontSize: "0.72rem",
          fontWeight: 700,
          letterSpacing: 1.2,
          textTransform: "uppercase",
          opacity: 0.75,
        }}
      >
        {title}
      </div>
      {children}
    </div>
  );
}

function DetailRow({
  label,
  value,
  tone = "neutral",
}: {
  label: string;
  value: string;
  tone?: "good" | "warn" | "bad" | "neutral";
}) {
  const toneColor =
    tone === "good"
      ? "#22c55e"
      : tone === "warn"
      ? "#fbbf24"
      : tone === "bad"
      ? "#ef4444"
      : undefined;
  return (
    <div style={{ display: "flex", flexDirection: "column", lineHeight: 1.35 }}>
      {label && (
        <div style={{ fontSize: "0.72rem", opacity: 0.65 }}>{label}</div>
      )}
      <div style={{ fontSize: "0.92rem", fontWeight: 600, color: toneColor }}>
        {value}
      </div>
    </div>
  );
}

// ─── Pure label helpers ─────────────────────────────────────────────

function surfaceLabel(s: string): string {
  const map: Record<string, string> = {
    ASP: "Asphalt",
    CON: "Beton",
    GRV: "Schotter",
    GRS: "Gras",
    DIRT: "Erde",
    TURF: "Rasen",
  };
  return map[s.toUpperCase()] ?? s;
}

function thirdLabel(t: 1 | 2 | 3): string {
  return t === 1 ? "erstes Drittel" : t === 2 ? "zweites Drittel" : "drittes Drittel";
}

function aimClassLabel(c: AimClass): string {
  switch (c) {
    case "perfect":
      return "perfekt";
    case "short_of_aim":
      return "zu früh";
    case "past_aim":
      return "etwas spät";
    case "long_landing":
      return "long landing";
    case "severe":
      return "kritisch";
  }
}

function aimTone(c: AimClass): "good" | "warn" | "bad" {
  if (c === "perfect" || c === "past_aim" || c === "short_of_aim") return "good";
  if (c === "long_landing") return "warn";
  return "bad";
}

function tchClassLabel(c: TchClass): string {
  switch (c) {
    case "on_profile":
      return "auf Profil";
    case "slightly_low":
      return "leicht niedrig";
    case "slightly_high":
      return "leicht hoch";
    case "high":
      return "zu hoch";
    case "below_profile":
      return "unter Profil";
  }
}

function tchTone(c: TchClass): "good" | "warn" | "bad" {
  if (c === "on_profile") return "good";
  if (c === "slightly_low" || c === "slightly_high") return "warn";
  return "bad";
}
