// Runway Diagram v2 — Skin definition.
//
// Eine "Skin" ist die komplette visuelle Konfiguration der V2-Component:
// Farben, Geometrie, Label-Strings, Tone-Schwellen, Display-Flags. Die
// Pilot-Client- und Webapp-Komponenten lesen NICHT mehr Hardcoded-Werte
// sondern beziehen alles aus dem aktuell geladenen Skin.
//
// Lade-Strategie:
//   1. Beim App-Start versucht der Pilot-Client einen frischen Skin von
//      https://live.nexusairva.org/api/v2-skin zu holen
//   2. Erfolgreicher Fetch → in AppData\com.aeroacars.app\v2-skin.json
//      cachen + verwenden
//   3. Fehlgeschlagener Fetch (offline) → letzten Cache lesen
//   4. Kein Cache (Erstinstallation offline) → DEFAULT_SKIN aus diesem
//      File verwenden
//
// Damit:
//   - Funktioniert offline (DEFAULT_SKIN als Fallback)
//   - Live-Änderungen für alle Piloten (= VPS-Skin tauschen, Piloten
//     pulken beim nächsten App-Start)
//   - Pilot-Client-Design bleibt nativ (kein iframe), Skin steuert nur
//     Tokens/Labels/Schwellen/Flags

export interface V2Skin {
  /** Schema-Version. Erhöhen wenn breaking changes an den Feldern. */
  version: number;

  // ─── Visual Tokens (Farben + strokeWidth + dashArray) ─────────────
  tokens: {
    tarmac: string;
    tarmacBorder: string;
    threshold: string;
    centerline: string;
    centerlineDashArray: string;
    tdzFill: string;
    tdzStroke: string;
    aimMarker: string;
    rollout: string;
    rolloutGlow: string;
    exitDot: string;
    ddsZone: string;
    ddsBorder: string;
    tdPerfect: string;
    tdAcceptable: string;
    tdWarn: string;
    tdSevere: string;
  };

  // ─── SVG-Geometrie ─────────────────────────────────────────────────
  geometry: {
    svgWidth: number;
    svgHeight: number;
    rwyPaddingX: number;
    rwyPaddingY: number;
  };

  // ─── Label-Strings (i18n-fähig — heute nur DE) ─────────────────────
  labels: {
    aufsetzzone_prefix: string;     // "AUFSETZZONE (TDZ)"
    aim_point_prefix: string;        // "AIM-POINT"
    aim_subtitle: string;            // "↓ Soll-Aufsetz-Stelle"
    bremspunkt_title: string;        // "Bremspunkt"
    bremspunkt_speed: string;        // "40 kt"
    td_prefix: string;               // "TD"
    auf_cl: string;                  // "auf CL"
    flugzeug_label: string;          // "Flugzeug"
    flugzeug_type: string;           // "Type"
    flugzeug_reg: string;            // "Reg"
    flugzeug_weight: string;         // "Gewicht"
    flugzeug_iast: string;           // "TD-IAS"
    flugzeug_pb: string;             // "P / B"
    flugzeug_peakg: string;          // "Peak-G"
    flugzeug_wind: string;           // "Wind"
    pill_bahn: string;               // "Bahn"
    pill_laenge: string;             // "Länge"
    pill_hinter_schwelle: string;    // "Hinter Schwelle"
    pill_mittellinie: string;        // "Mittellinie"
    pill_ausrollstrecke: string;     // "Ausrollstrecke"
    pill_bahn_auslastung: string;    // "Bahn-Auslastung"
    pill_tdz: string;                // "Touchdown-Zone"
    pill_aim_point: string;          // "Aim-Point"
    pill_tch: string;                // "Anflug-Profil (TCH)"
    pill_pre_threshold: string;      // "⚠ Pre-Threshold"
    pill_navdata: string;            // "Navdata-Quelle"
  };

  // ─── Tone-Schwellen (= Grenzwerte für grün/gelb/rot) ───────────────
  thresholds: {
    peak_g_warn: number;             // 1.5
    peak_g_bad: number;              // 1.7
    crosswind_warn: number;          // 15
    crosswind_bad: number;           // 25
    tailwind_bad: number;            // 3 (negative HW)
    pitch_bad_below: number;         // 0 (Pitch < 0 = Tail-Strike-Risk)
    bank_warn_above: number;         // 5
    bahn_auslastung_warn_above: number;   // 85
    centerline_warn_above: number;        // 5
    centerline_bad_above: number;         // 15
    hinter_schwelle_warn_above: number;   // 1000
  };

  // ─── Display-Flags (= ein/ausblenden ohne Code-Änderung) ───────────
  display: {
    show_aim_marker: boolean;
    show_aufsetzzone_box: boolean;
    show_brakepoint: boolean;
    show_opposite_runway: boolean;
    show_bahn_length: boolean;
    show_flugzeug_bar: boolean;
    show_lr_offset_arrow: boolean;
  };
}

// ─── Default-Skin (bundled, used als Offline-Fallback) ──────────────

export const DEFAULT_SKIN: V2Skin = {
  version: 1,
  tokens: {
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
  },
  geometry: {
    svgWidth: 1200,
    svgHeight: 400,
    rwyPaddingX: 70,
    rwyPaddingY: 95,
  },
  labels: {
    aufsetzzone_prefix: "AUFSETZZONE (TDZ)",
    aim_point_prefix: "AIM-POINT",
    aim_subtitle: "↓ Soll-Aufsetz-Stelle",
    bremspunkt_title: "Bremspunkt",
    bremspunkt_speed: "40 kt",
    td_prefix: "TD",
    auf_cl: "auf CL",
    flugzeug_label: "Flugzeug",
    flugzeug_type: "Type",
    flugzeug_reg: "Reg",
    flugzeug_weight: "Gewicht",
    flugzeug_iast: "TD-IAS",
    flugzeug_pb: "P / B",
    flugzeug_peakg: "Peak-G",
    flugzeug_wind: "Wind",
    pill_bahn: "Bahn",
    pill_laenge: "Länge",
    pill_hinter_schwelle: "Hinter Schwelle",
    pill_mittellinie: "Mittellinie",
    pill_ausrollstrecke: "Ausrollstrecke",
    pill_bahn_auslastung: "Bahn-Auslastung",
    pill_tdz: "Touchdown-Zone",
    pill_aim_point: "Aim-Point",
    pill_tch: "Anflug-Profil (TCH)",
    pill_pre_threshold: "⚠ Pre-Threshold",
    pill_navdata: "Navdata-Quelle",
  },
  thresholds: {
    peak_g_warn: 1.5,
    peak_g_bad: 1.7,
    crosswind_warn: 15,
    crosswind_bad: 25,
    tailwind_bad: 3,
    pitch_bad_below: 0,
    bank_warn_above: 5,
    bahn_auslastung_warn_above: 85,
    centerline_warn_above: 5,
    centerline_bad_above: 15,
    hinter_schwelle_warn_above: 1000,
  },
  display: {
    show_aim_marker: true,
    show_aufsetzzone_box: true,
    show_brakepoint: true,
    show_opposite_runway: true,
    show_bahn_length: true,
    show_flugzeug_bar: true,
    show_lr_offset_arrow: true,
  },
};

/**
 * Tiefes Mergen einer fremden (möglicherweise unvollständigen) Skin
 * mit den Defaults. Fehlende Felder werden mit Defaults ergänzt, damit
 * eine neu-deployed Pilot-Client-Version mit altem VPS-Skin nicht
 * crasht.
 */
export function mergeWithDefaults(partial: Partial<V2Skin> | null | undefined): V2Skin {
  if (!partial) return DEFAULT_SKIN;
  return {
    version: partial.version ?? DEFAULT_SKIN.version,
    tokens: { ...DEFAULT_SKIN.tokens, ...(partial.tokens ?? {}) },
    geometry: { ...DEFAULT_SKIN.geometry, ...(partial.geometry ?? {}) },
    labels: { ...DEFAULT_SKIN.labels, ...(partial.labels ?? {}) },
    thresholds: { ...DEFAULT_SKIN.thresholds, ...(partial.thresholds ?? {}) },
    display: { ...DEFAULT_SKIN.display, ...(partial.display ?? {}) },
  };
}
