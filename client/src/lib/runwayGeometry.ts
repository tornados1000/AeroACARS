// Bahn-Geometrie — die EINE Stelle, an der aus einem Runway-Match Laengen
// werden.
//
// v0.20.0: ausgelagert aus LandingPanel.tsx. Der RunwayDiagramV2-Mapper
// braucht dieselbe LDA-Formel wie die rollout-Kachel; importierte er sie aus
// LandingPanel, entstuende ein zirkulaerer Laufzeit-Import (LandingPanel
// importiert den Mapper). Das laeuft in dev und in vitest, kann aber im
// Rollup-Produktions-Build als TDZ-Fehler hochgehen. Beide holen die Formel
// jetzt von hier.

/** Die Felder eines Runway-Matches, aus denen sich die Laengen ergeben.
 *  Strukturell getypt, damit dieses Modul nichts aus LandingPanel importieren
 *  muss — auch nicht als Typ. */
export interface RunwayLengths {
  length_ft: number;
  displaced_threshold_ft?: number | null;
}

/**
 * LDA (Landing Distance Available) in Metern.
 * Spec LE5: `LDA_m = (length_ft − displaced_threshold_ft) × 0.3048`.
 * Liefert null wenn die Geometrie unbrauchbar ist (length ≤ 0, LDA ≤ 0).
 */
export function rolloutLdaMeters(rm: RunwayLengths): number | null {
  if (!Number.isFinite(rm.length_ft) || rm.length_ft <= 0) return null;
  const displacedFt = rm.displaced_threshold_ft ?? 0;
  const lda = (rm.length_ft - displacedFt) * 0.3048;
  return lda > 0 ? lda : null;
}
