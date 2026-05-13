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
  svgHeight: 400,
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

          {/* TDZ-Box — gelbe Schraffur als Bereichs-Indikator + dünner
              Rahmen + Label. Die diagonale Schraffur soll visuell
              vermitteln "hier soll der Touchdown rein". */}
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
                opacity="0.55"
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
                fill={TOKENS.tdzStroke}
                fontWeight="700"
                fontFamily="monospace"
                textAnchor="middle"
              >
                AUFSETZZONE (TDZ) {props.td_tdz_length_m?.toFixed(0)} m
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

          {/* Aim-Point — ICAO Annex 14 §5.2.6: GENAU ZWEI breite
              Streifen, symmetrisch zur Centerline. Ein Streifen liegt
              direkt OBERHALB der CL, einer direkt UNTERHALB. (Frühere
              v2-Version hatte 4 kleine Quadrate in 2×2 = falsche
              "Stufen"-Optik — User-Befund 2026-05-13.) Streifen-Breite
              hier 24 px (entspricht ~50 m Real-Länge, ICAO gibt 30–60 m
              je nach Bahn). */}
          {aimX != null && (
            <g>
              <rect
                x={aimX - 12}
                y={rwyCl - 22}
                width={24}
                height={18}
                fill={TOKENS.aimMarker}
                opacity="0.95"
              />
              <rect
                x={aimX - 12}
                y={rwyCl + 4}
                width={24}
                height={18}
                fill={TOKENS.aimMarker}
                opacity="0.95"
              />
              {/* Pfeilspitze + Label oberhalb der Bahn — zeigt explizit
                  dass die zwei großen gelben Streifen die Aim-Point-
                  Markierungen sind (wie auf echten Runways gemalt). */}
              <polygon
                points={`${aimX - 7},${rwyTop - 14} ${aimX + 7},${rwyTop - 14} ${aimX},${rwyTop - 4}`}
                fill={TOKENS.aimMarker}
              />
              <text
                x={aimX}
                y={rwyTop - 32}
                textAnchor="middle"
                fontSize="13"
                fill={TOKENS.aimMarker}
                fontWeight="700"
                fontFamily="monospace"
              >
                AIM-POINT {props.aim_point_m?.toFixed(0)} m
              </text>
              <text
                x={aimX}
                y={rwyTop - 19}
                textAnchor="middle"
                fontSize="10"
                fill={TOKENS.aimMarker}
                fontFamily="monospace"
                opacity="0.85"
              >
                ↓ Soll-Aufsetz-Stelle
              </text>
              <title>
                Aim-Point — die zwei großen weißen Quadrate auf der echten
                Bahn (hier gelb gezeichnet). Pilot zielt im Anflug auf
                diese Markierung; durch den Flare setzt er typisch
                50–150 m DAHINTER auf (= Anfang der TDZ).
                Position: {props.aim_point_m?.toFixed(0)} m hinter Schwelle.
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

          {/* "Bahn verbleibend X m" — Annotation im leeren Bereich
              hinter dem Exit-Punkt. Übernommen aus dem Pilot-Client-
              Legacy-Layout, weil Piloten dort sofort sehen wie viel
              Bahn sie verschenkt haben. Nur wenn nach Exit noch
              sinnvoller Platz auf der Bahn ist (≥ 200 m). */}
          {exitX != null &&
            props.rollout_m != null &&
            lengthM - (props.td_distance_from_threshold_m + props.rollout_m) >= 200 && (
              <g>
                {/* Vertikale Linie an der "Bahn-Ende"-Position */}
                <line
                  x1={padX + innerW - 4}
                  y1={rwyTop + 6}
                  x2={padX + innerW - 4}
                  y2={rwyBot - 6}
                  stroke="rgba(255,255,255,0.55)"
                  strokeWidth="2"
                />
                {/* Doppelpfeil "von Exit bis Bahn-Ende" */}
                <line
                  x1={exitX + 14}
                  y1={rwyCl - 30}
                  x2={padX + innerW - 8}
                  y2={rwyCl - 30}
                  stroke="rgba(148,163,184,0.75)"
                  strokeWidth="1.5"
                  strokeDasharray="4,3"
                />
                <polygon
                  points={`${padX + innerW - 8},${rwyCl - 30} ${padX + innerW - 14},${rwyCl - 34} ${padX + innerW - 14},${rwyCl - 26}`}
                  fill="rgba(148,163,184,0.85)"
                />
                <polygon
                  points={`${exitX + 14},${rwyCl - 30} ${exitX + 20},${rwyCl - 34} ${exitX + 20},${rwyCl - 26}`}
                  fill="rgba(148,163,184,0.85)"
                />
                {/* "Bahn verbleibend X m" Label */}
                <text
                  x={(exitX + padX + innerW) / 2}
                  y={rwyCl - 38}
                  textAnchor="middle"
                  fontSize="13"
                  fill="rgba(226,232,240,0.85)"
                  fontWeight="600"
                  fontFamily="monospace"
                >
                  Bahn verbleibend {(lengthM - (props.td_distance_from_threshold_m + props.rollout_m)).toFixed(0)} m
                </text>
                <text
                  x={(exitX + padX + innerW) / 2}
                  y={rwyCl + 8}
                  textAnchor="middle"
                  fontSize="11"
                  fill="rgba(148,163,184,0.7)"
                  fontFamily="monospace"
                >
                  ({(((lengthM - (props.td_distance_from_threshold_m + props.rollout_m)) / lengthM) * 100).toFixed(0)} % unbenutzt)
                </text>
              </g>
            )}

          {/* Offset-Indikator: großer Pfeil + Label UNTER der Bahn, mit
              dünner Anker-Linie zum TD-Dot. Bewusst außerhalb der
              Bahn-Fläche, damit es nicht hinter den AIM-Quadraten
              verschwindet wenn TD und Aim-Position fast übereinander
              liegen. Nur wenn |offset| > 0.5 m. */}
          {Math.abs(props.td_centerline_offset_m) > 0.5 && (() => {
            const isLeft = props.td_centerline_offset_m < 0;
            // Pfeil-Group direkt unter der Bahn — über dem TD-Distanz-Label.
            const arrowY = rwyBot + 22;
            const arrowLen = 56;
            const ax1 = isLeft ? tdX + arrowLen / 2 : tdX - arrowLen / 2;
            const ax2 = isLeft ? tdX - arrowLen / 2 : tdX + arrowLen / 2;
            return (
              <g>
                {/* Dünne Anker-Linie vom TD-Dot zum Pfeil */}
                <line
                  x1={tdX}
                  y1={tdY}
                  x2={tdX}
                  y2={arrowY - 8}
                  stroke={dotColor}
                  strokeWidth="1"
                  strokeDasharray="2,3"
                  opacity="0.5"
                />
                {/* Großer Pfeil-Schaft */}
                <line
                  x1={ax1}
                  y1={arrowY}
                  x2={ax2}
                  y2={arrowY}
                  stroke={dotColor}
                  strokeWidth="3.5"
                />
                {/* Große Pfeilspitze */}
                <polygon
                  points={
                    isLeft
                      ? `${ax2 - 10},${arrowY - 8} ${ax2},${arrowY} ${ax2 - 10},${arrowY + 8}`
                      : `${ax2 + 10},${arrowY - 8} ${ax2},${arrowY} ${ax2 + 10},${arrowY + 8}`
                  }
                  fill={dotColor}
                />
                {/* Großes Label neben dem Pfeil */}
                <text
                  x={isLeft ? ax2 - 14 : ax2 + 14}
                  y={arrowY + 5}
                  fontSize="15"
                  fill={dotColor}
                  fontWeight="800"
                  fontFamily="monospace"
                  textAnchor={isLeft ? "end" : "start"}
                >
                  {Math.abs(props.td_centerline_offset_m).toFixed(1)} m {isLeft ? "LINKS" : "RECHTS"}
                </text>
              </g>
            );
          })()}

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

          {/* Bremspunkt — Punkt an dem die Groundspeed unter 40 kt fiel.
              Heißt NICHT "hier verlässt der Pilot die Bahn" (das passiert
              an einem der nächsten Taxiway-Abzweige). Heißt "ab hier
              kannst du normal abbiegen". Früher als "EXIT" gelabelt,
              das war missverständlich (User-Befund 2026-05-13). */}
          {exitX != null && (
            <g>
              <circle cx={exitX} cy={tdY} r="11" fill={TOKENS.exitDot} opacity="0.25" />
              <circle cx={exitX} cy={tdY} r="6" fill={TOKENS.exitDot} stroke="#0c1628" strokeWidth="1.5" />
              {/* Label-Pair mit dunklem Outline-Stroke für Lesbarkeit
                  über dem Cyan-Rollout-Strich + Tarmac-Hintergrund. */}
              <text
                x={exitX}
                y={tdY - 26}
                textAnchor="middle"
                fontSize="14"
                fill={TOKENS.exitDot}
                fontWeight="800"
                fontFamily="monospace"
                stroke="#0c1628"
                strokeWidth="3"
                paintOrder="stroke"
              >
                Bremspunkt
              </text>
              <text
                x={exitX}
                y={tdY - 12}
                textAnchor="middle"
                fontSize="13"
                fill={TOKENS.exitDot}
                fontWeight="800"
                fontFamily="monospace"
                stroke="#0c1628"
                strokeWidth="3"
                paintOrder="stroke"
              >
                40 kt
              </text>
              <title>
                Bremspunkt — Ab hier hast du auf ~40 kt abgebremst. Das ist
                die typische High-Speed-Exit-Geschwindigkeit; am nächsten
                Rollwege-Abzweig kannst du die Bahn jetzt normal verlassen.
                NICHT die Stelle wo du tatsächlich abbiegst — das passiert
                später, an einem konkreten Taxiway.
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

          {/* TD-Label unter dem Dot — nur Distanz. L/R wird durch den
              großen L/R-Pfeil oben dargestellt. Bei Offset < 0.5 m
              steht hier zusätzlich "auf CL". */}
          <g>
            <text
              x={tdX}
              y={rwyBot + 46}
              textAnchor="middle"
              fontSize="13"
              fill={dotColor}
              fontWeight="700"
              fontFamily="monospace"
            >
              TD {props.td_distance_from_threshold_m.toFixed(0)} m
              {Math.abs(props.td_centerline_offset_m) < 0.5 ? " · auf CL" : ""}
            </text>
          </g>

          {/* Distanz-Skala unter der Bahn. */}
          <g>
            <line
              x1={padX}
              y1={rwyBot + 62}
              x2={padX + innerW}
              y2={rwyBot + 62}
              stroke="rgba(255,255,255,0.25)"
              strokeWidth="1"
            />
            {scaleTicks.map((d) => {
              const x = mToX(d);
              return (
                <g key={d}>
                  <line
                    x1={x}
                    y1={rwyBot + 57}
                    x2={x}
                    y2={rwyBot + 67}
                    stroke="rgba(255,255,255,0.5)"
                    strokeWidth="1.2"
                  />
                  <text
                    x={x}
                    y={rwyBot + 80}
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
        {exitX && <LegendDot color={TOKENS.exitDot} label="Bremspunkt (40 kt)" />}
        {ddsActive && <LegendItem swatch={TOKENS.ddsBorder} label="Pre-Threshold — Landung verboten" />}
      </div>

      {/* ─── 4. DETAIL-PILLS ─────────────────────────────────────────
          v2.2 Layout-Switch: vom 3-Box-Layout (jeweils mehrere Rows
          drin) auf atomare Pill-Cards (1 Stat pro Pill) wie im
          Pilot-Client-Legacy-Layout. Macht den Block kompakter und
          info-dichter — Piloten finden den gewünschten Wert schneller. */}
      <div
        style={{
          display: "flex",
          flexWrap: "wrap",
          gap: 8,
        }}
      >
        <Pill label="Bahn" value={`${props.airport_ident}/${props.runway_ident}${props.surface ? ` (${surfaceLabel(props.surface)})` : ""}`} />
        <Pill label="Länge" value={`${props.length_m.toFixed(0)} m`} />
        <Pill
          label="Hinter Schwelle"
          value={`${props.td_distance_from_threshold_m.toFixed(0)} m`}
          tone={
            props.pre_displaced_threshold === true
              ? "bad"
              : props.td_distance_from_threshold_m < 0
              ? "bad"
              : props.td_distance_from_threshold_m > 1000
              ? "warn"
              : "good"
          }
        />
        <Pill
          label="Mittellinie"
          value={
            Math.abs(props.td_centerline_offset_m) < 0.5
              ? "auf CL"
              : `${Math.abs(props.td_centerline_offset_m).toFixed(1)} m ${
                  props.td_centerline_offset_m > 0 ? "RECHTS" : "LINKS"
                }`
          }
          tone={
            Math.abs(props.td_centerline_offset_m) < 5
              ? "good"
              : Math.abs(props.td_centerline_offset_m) < 15
              ? "warn"
              : "bad"
          }
        />
        {props.rollout_m != null && (
          <Pill label="Ausrollstrecke" value={`${props.rollout_m.toFixed(0)} m`} />
        )}
        {bahnUsedPct != null && (
          <Pill
            label="Bahn-Auslastung"
            value={`${bahnUsedPct.toFixed(0)} %`}
            tone={bahnUsedPct > 85 ? "warn" : "good"}
          />
        )}
        {props.td_in_tdz != null && (
          <Pill
            label="Touchdown-Zone"
            value={
              props.td_in_tdz
                ? `✓ ${props.td_third ? thirdLabel(props.td_third) : "im Marker"}`
                : `✗ ${props.td_third ? thirdLabel(props.td_third) : "verfehlt"}`
            }
            tone={props.td_in_tdz ? "good" : "warn"}
          />
        )}
        {props.aim_class && props.aim_delta_m != null && props.aim_point_m != null && (
          <Pill
            label="Aim-Point"
            value={`${props.aim_point_m.toFixed(0)} m · Δ ${props.aim_delta_m >= 0 ? "+" : ""}${props.aim_delta_m.toFixed(0)} m · ${aimClassLabel(props.aim_class)}`}
            tone={aimTone(props.aim_class)}
          />
        )}
        {props.tch_actual_ft != null && props.tch_class && (
          <Pill
            label="Anflug-Profil (TCH)"
            value={`${props.tch_actual_ft.toFixed(0)} ft${props.tch_delta_ft != null ? ` · Δ ${props.tch_delta_ft >= 0 ? "+" : ""}${props.tch_delta_ft.toFixed(0)} ft` : ""} · ${tchClassLabel(props.tch_class)}`}
            tone={tchTone(props.tch_class)}
          />
        )}
        {props.pre_displaced_threshold === true && (
          <Pill
            label="⚠ Pre-Threshold"
            value="Aufsetzen VOR der Landeschwelle (illegal IRL)"
            tone="bad"
          />
        )}
        <Pill
          label="Navdata-Quelle"
          value={
            props.source === "navigraph"
              ? `VPS Navdata · AIRAC ${props.nav_cycle ?? "?"}`
              : props.source === "ourairports_fallback"
              ? "OurAirports (Fallback)"
              : "OurAirports"
          }
          tone={
            props.source === "navigraph"
              ? "good"
              : props.source === "ourairports_fallback"
              ? "warn"
              : "neutral"
          }
        />
      </div>

      {glossaryOpen && (
        <GlossaryModal onClose={() => setGlossaryOpen(false)} />
      )}
    </section>
  );
}

// ─── Kleine UI-Helpers ──────────────────────────────────────────────

// Atomare Stat-Pille — 1 Label + 1 Value, optionale Tone-Farbe am Wert.
// Ersetzt das alte 3-Box-DetailCard-Layout (v2.2).
function Pill({
  label,
  value,
  tone = "neutral",
}: {
  label: string;
  value: string;
  tone?: "good" | "warn" | "bad" | "neutral";
}) {
  const valueColor =
    tone === "good"
      ? "#22c55e"
      : tone === "warn"
      ? "#fbbf24"
      : tone === "bad"
      ? "#ef4444"
      : "#e2e8f0";
  return (
    <div
      style={{
        padding: "8px 12px",
        background: "rgba(255,255,255,0.04)",
        border: "1px solid rgba(255,255,255,0.10)",
        borderRadius: 8,
        display: "flex",
        flexDirection: "column",
        gap: 2,
        minWidth: 110,
        maxWidth: 320,
      }}
    >
      <div
        style={{
          fontSize: "0.68rem",
          fontWeight: 700,
          letterSpacing: 1.1,
          textTransform: "uppercase",
          opacity: 0.65,
        }}
      >
        {label}
      </div>
      <div style={{ fontSize: "0.95rem", fontWeight: 700, color: valueColor }}>
        {value}
      </div>
    </div>
  );
}

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
