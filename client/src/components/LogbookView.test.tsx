import { describe, it, expect } from "vitest";
import { profileSvg } from "./LogbookView";

// Live-Befund Thomas K., 05.07.2026: AGL- und Gelände-Linie im Höhenprofil
// (Logbuch-Detail) "fehlen" optisch. Verifiziert per Live-DB-Query: die
// Werte SIND korrekt vorhanden (altitude_agl 8.82..36328 ft für diesen
// Flug) — das Problem ist reine SVG-Zeichenreihenfolge: die MSL-Linie
// wurde zuletzt (= oben) gezeichnet und überdeckt AGL/Gelände überall dort
// vollständig, wo sie nahe an MSL liegen (bei flachem Terrain praktisch die
// ganze Flugzeit). Fix: MSL zuerst, dann Gelände, dann AGL zuoberst.
describe("profileSvg", () => {
  it("draws MSL first, then terrain, then AGL last (topmost) so AGL/terrain are never hidden", () => {
    const route = [
      { lat: 0, lon: 0, alt_ft: 100, agl_ft: 50 },
      { lat: 0, lon: 0.1, alt_ft: 36000, agl_ft: 35800 },
      { lat: 0, lon: 0.2, alt_ft: 100, agl_ft: 9 },
    ];
    const svg = profileSvg(route);

    const mslStrokeIdx = svg.indexOf('stroke="var(--accent)"');
    const terrStrokeIdx = svg.indexOf('stroke="var(--text-muted)"');
    const aglStrokeIdx = svg.indexOf('stroke="var(--success)"');

    expect(mslStrokeIdx).toBeGreaterThan(-1);
    expect(terrStrokeIdx).toBeGreaterThan(-1);
    expect(aglStrokeIdx).toBeGreaterThan(-1);
    // Zeichenreihenfolge in SVG = Stapelreihenfolge (später = oben).
    expect(mslStrokeIdx).toBeLessThan(terrStrokeIdx);
    expect(terrStrokeIdx).toBeLessThan(aglStrokeIdx);
  });

  it("returns a placeholder when fewer than 2 points have altitude data", () => {
    expect(profileSvg([])).toContain("Keine Höhendaten");
    expect(profileSvg([{ lat: 0, lon: 0 }])).toContain("Keine Höhendaten");
  });
});
