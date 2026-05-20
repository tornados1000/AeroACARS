// v0.12.3 (LE8/LE9) — regression coverage for the frontend scored-G
// wiring. Spec: docs/spec/v0.12.3-landing-g-foqa-measurement.md.
//
//   * scoreG()         — the EMA-scored G with raw-peak fallback, used by
//                        QuickFlags + RunwayDiagram.
//   * computeSubScores — the TS scoring mirror; sub_g_force must score
//                        scored_g_load when present, else peak_g_load.

import { describe, it, expect } from "vitest";
import { scoreG } from "./LandingPanel";
import { computeSubScores } from "../lib/landingScoring";

describe("scoreG() — frontend score-G helper (LE9)", () => {
  it("prefers the EMA scored G over the raw 50 Hz peak", () => {
    expect(
      scoreG({ landing_scored_g_force: 1.78, landing_peak_g_force: 1.95 }),
    ).toBe(1.78);
  });

  it("falls back to the raw peak when scored G is null/undefined", () => {
    expect(
      scoreG({ landing_scored_g_force: null, landing_peak_g_force: 1.95 }),
    ).toBe(1.95);
    expect(
      scoreG({ landing_scored_g_force: undefined, landing_peak_g_force: 1.95 }),
    ).toBe(1.95);
  });

  it("returns null when neither value is present", () => {
    expect(
      scoreG({ landing_scored_g_force: null, landing_peak_g_force: null }),
    ).toBeNull();
  });
});

describe("computeSubScores g_force — TS mirror (LE8)", () => {
  const gValue = (input: Parameters<typeof computeSubScores>[0]) =>
    computeSubScores(input).find((s) => s.key === "g_force")?.value;

  it("scores the EMA scored_g_load when present", () => {
    expect(gValue({ scored_g_load: 1.78, peak_g_load: 1.95 })).toBe("1.78 G");
  });

  it("falls back to peak_g_load when scored_g_load is absent", () => {
    expect(gValue({ peak_g_load: 1.95 })).toBe("1.95 G");
  });

  it("emits no g_force sub-score when neither G value is present", () => {
    expect(gValue({ vs_fpm: -150 })).toBeUndefined();
  });
});
