// v0.7.8 Phase 1 — Component-Tests fuer SinkrateForensik
// Spec: docs/spec/v0.7.8-landing-rate-explainability.md §8.1
//
// Fokus: pure functions (Bucket-Math, Trend-Detection, Coaching-Tipp-
// Selektor, hasForensics, Score-Basis-Cascade). Render-Tests fuer
// Missing-Data-Cases.

import { describe, it, expect } from "vitest";
import {
  hasForensics,
  computeBuckets,
  isMonotonAccelerating,
  pickCoachingTip,
  scoreBasisVs,
  selectTraceSamples,
  vsTone,
} from "./SinkrateForensik";
import type { LandingProfilePoint } from "./LandingPanel";

// ───────────────────────────────────────────────────────────────────────────
// GSG-218-Fixture (Live-DB-Werte, Spec §8.3)
// ───────────────────────────────────────────────────────────────────────────
const GSG_218 = {
  landing_rate_fpm: -358,
  landing_peak_vs_fpm: -343,
  landing_source: "vs_at_impact",
  vs_at_edge_fpm: -343,
  vs_smoothed_250ms_fpm: -344,
  vs_smoothed_500ms_fpm: -314,
  vs_smoothed_1000ms_fpm: -266,
  vs_smoothed_1500ms_fpm: -229,
  peak_vs_pre_flare_fpm: -347,
  peak_g_post_500ms: 1.82,
  peak_g_post_1000ms: 1.82,
  flare_reduction_fpm: 0,
  forensic_sample_count: 469,
};

const EMPTY_LEGACY = {
  forensic_sample_count: null,
  vs_smoothed_250ms_fpm: null,
  vs_smoothed_500ms_fpm: null,
  vs_smoothed_1000ms_fpm: null,
  vs_smoothed_1500ms_fpm: null,
  vs_at_edge_fpm: null,
};

// ───────────────────────────────────────────────────────────────────────────
describe("hasForensics — Render-Gate", () => {
  it("true wenn forensic_sample_count gesetzt", () => {
    expect(hasForensics({ ...EMPTY_LEGACY, forensic_sample_count: 469 })).toBe(true);
  });
  it("true wenn nur vs_smoothed_500ms gesetzt", () => {
    expect(hasForensics({ ...EMPTY_LEGACY, vs_smoothed_500ms_fpm: -314 })).toBe(true);
  });
  it("true wenn nur vs_at_edge_fpm gesetzt", () => {
    expect(hasForensics({ ...EMPTY_LEGACY, vs_at_edge_fpm: -343 })).toBe(true);
  });
  it("false wenn ALLE Felder null (Legacy)", () => {
    expect(hasForensics(EMPTY_LEGACY)).toBe(false);
  });
});

// ───────────────────────────────────────────────────────────────────────────
describe("computeBuckets — Disjoint-Bucket-Differenz", () => {
  it("GSG-218: korrekte Bucket-Werte aus Live-DB-Fixture", () => {
    const buckets = computeBuckets(-229, -266, -314, -344, -343);
    expect(buckets).not.toBeNull();
    // (1500*-229 - 1000*-266) / 500 = -155
    expect(Math.round(buckets![0]!.vs)).toBe(-155);
    // (1000*-266 - 500*-314) / 500 = -218
    expect(Math.round(buckets![1]!.vs)).toBe(-218);
    // (500*-314 - 250*-344) / 250 = -284
    expect(Math.round(buckets![2]!.vs)).toBe(-284);
    // Edge direkt
    expect(buckets![3]!.vs).toBe(-343);
  });

  it("null wenn auch nur ein Mittelwert fehlt (Bucket-Math braucht alle 4)", () => {
    expect(computeBuckets(null, -266, -314, -344, -343)).toBeNull();
    expect(computeBuckets(-229, null, -314, -344, -343)).toBeNull();
    expect(computeBuckets(-229, -266, -314, null, -343)).toBeNull();
  });

  it("Edge-Fallback: wenn vs_at_edge fehlt, nimm vs_smoothed_250ms", () => {
    const buckets = computeBuckets(-229, -266, -314, -344, null);
    expect(buckets![3]!.vs).toBe(-344);
  });
});

// ───────────────────────────────────────────────────────────────────────────
describe("isMonotonAccelerating — Trend ueber BETRAG", () => {
  it("GSG-218: |155|→|218|→|284|→|343| alle Deltas >20 = true", () => {
    const buckets = computeBuckets(-229, -266, -314, -344, -343)!;
    expect(isMonotonAccelerating(buckets)).toBe(true);
  });

  it("Stabile Landung (gleichmaessig, kein Drop): false", () => {
    // Konstruiert: 4 Buckets mit |VS|-Werten 100, 105, 110, 115 fpm
    // Deltas: 5, 5, 5 — alle <20 → KEIN monotones Beschleunigen
    const buckets = [
      { label: "a", vs: -100 },
      { label: "b", vs: -105 },
      { label: "c", vs: -110 },
      { label: "d", vs: -115 },
    ];
    expect(isMonotonAccelerating(buckets)).toBe(false);
  });

  it("Flare wirkt (Betrag faellt am Ende): false", () => {
    // 4 Buckets: -200, -250, -300, -150 — letzter Bucket weicher
    const buckets = [
      { label: "a", vs: -200 },
      { label: "b", vs: -250 },
      { label: "c", vs: -300 },
      { label: "d", vs: -150 },
    ];
    expect(isMonotonAccelerating(buckets)).toBe(false);
  });
});

// ───────────────────────────────────────────────────────────────────────────
describe("pickCoachingTip — Prioritaet (erster Match gewinnt)", () => {
  it("GSG-218: flare_lost wegen monoton beschleunigendem Trend", () => {
    const buckets = computeBuckets(-229, -266, -314, -344, -343)!;
    const tip = pickCoachingTip({
      buckets,
      peakGPost500ms: 1.82,
      flareReductionFpm: 0,
      vsAtEdgeFpm: -343,
      vsSmoothed1500ms: -229,
    });
    expect(tip).toBe("flare_lost");
  });

  it("Hard-G ohne Bucket-Trend: hard_g triggert", () => {
    const buckets = [
      { label: "a", vs: -100 },
      { label: "b", vs: -100 },
      { label: "c", vs: -100 },
      { label: "d", vs: -100 },
    ];
    const tip = pickCoachingTip({
      buckets,
      peakGPost500ms: 1.85,
      flareReductionFpm: 100,
      vsAtEdgeFpm: -100,
      vsSmoothed1500ms: -100,
    });
    expect(tip).toBe("hard_g");
  });

  it("Sanfte Landung ohne Drop: clean", () => {
    const buckets = [
      { label: "a", vs: -90 },
      { label: "b", vs: -100 },
      { label: "c", vs: -110 },
      { label: "d", vs: -120 },
    ];
    const tip = pickCoachingTip({
      buckets,
      peakGPost500ms: 1.2,
      flareReductionFpm: 150,
      vsAtEdgeFpm: -120,
      vsSmoothed1500ms: -90,
    });
    expect(tip).toBe("clean");
  });

  it("Late-Drop ueber Math.abs (v1.3-Fix): triggert bei |343|-|229|=114 > 100", () => {
    // Kein Bucket-Trend, kein Hard-G, kein No-Flare — nur Late-Drop
    const buckets = [
      { label: "a", vs: -100 },
      { label: "b", vs: -100 },
      { label: "c", vs: -100 },
      { label: "d", vs: -343 },
    ];
    const tip = pickCoachingTip({
      buckets: null, // explicit kein Trend-Trigger
      peakGPost500ms: 1.3,
      flareReductionFpm: 100,
      vsAtEdgeFpm: -343,
      vsSmoothed1500ms: -229,
    });
    expect(tip).toBe("late_drop");
    // Buckets ungenutzt — Test demonstriert dass Late-Drop unabhaengig
    // vom Trend triggern kann
    expect(buckets).toBeTruthy();
  });

  it("No-Flare: flare_reduction < 50", () => {
    const tip = pickCoachingTip({
      buckets: null,
      peakGPost500ms: 1.3,
      flareReductionFpm: 30,
      vsAtEdgeFpm: -200,
      vsSmoothed1500ms: -180,
    });
    expect(tip).toBe("no_flare");
  });
});

// ───────────────────────────────────────────────────────────────────────────
describe("scoreBasisVs — Cascade-Chain", () => {
  it("nimmt landing_peak_vs_fpm wenn gesetzt", () => {
    expect(scoreBasisVs({
      landing_peak_vs_fpm: -343,
      landing_rate_fpm: -358,
    })).toBe(-343);
  });

  it("Fallback auf landing_rate_fpm wenn landing_peak_vs_fpm null", () => {
    expect(scoreBasisVs({
      landing_peak_vs_fpm: null,
      landing_rate_fpm: -358,
    })).toBe(-358);
  });
});

// ───────────────────────────────────────────────────────────────────────────
describe("vsTone — Bands nach landingScoring.ts:128-131", () => {
  it("|150| → good (< 200)", () => expect(vsTone(-150)).toBe("good"));
  it("|343| → neutral (< 400, GSG-218-Fall)", () => expect(vsTone(-343)).toBe("neutral"));
  it("|500| → warn (< 600)", () => expect(vsTone(-500)).toBe("warn"));
  it("|800| → err (< 1000)", () => expect(vsTone(-800)).toBe("err"));
  it("|1200| → err-severe (>= 1000)", () => expect(vsTone(-1200)).toBe("err-severe"));
  it("null → null", () => expect(vsTone(null)).toBeNull());
});

// ───────────────────────────────────────────────────────────────────────────
describe("selectTraceSamples — touchdown_profile Filter [-3500, 0]", () => {
  it("waehlt nearest-neighbor bei -3000/-2000/-1000/-500/-100", () => {
    const profile: LandingProfilePoint[] = [
      { t_ms: -3100, vs_fpm: -110, agl_ft: 18, on_ground: false,
        g_force: 1, heading_true_deg: 0, groundspeed_kt: 130, indicated_airspeed_kt: 130, pitch_deg: 2, bank_deg: 0 },
      { t_ms: -2100, vs_fpm: -144, agl_ft: 12, on_ground: false,
        g_force: 1, heading_true_deg: 0, groundspeed_kt: 130, indicated_airspeed_kt: 130, pitch_deg: 2, bank_deg: 0 },
      { t_ms: -1100, vs_fpm: -171, agl_ft: 8, on_ground: false,
        g_force: 1, heading_true_deg: 0, groundspeed_kt: 130, indicated_airspeed_kt: 130, pitch_deg: 2, bank_deg: 0 },
      { t_ms: -600, vs_fpm: -249, agl_ft: 3, on_ground: false,
        g_force: 1, heading_true_deg: 0, groundspeed_kt: 130, indicated_airspeed_kt: 130, pitch_deg: 2, bank_deg: 0 },
      { t_ms: -100, vs_fpm: -348, agl_ft: 1, on_ground: false,
        g_force: 1, heading_true_deg: 0, groundspeed_kt: 130, indicated_airspeed_kt: 130, pitch_deg: 2, bank_deg: 0 },
    ];
    const trace = selectTraceSamples(profile);
    expect(trace.length).toBe(5);
    expect(trace[0]!.t_ms).toBe(-3100);
    expect(trace[4]!.t_ms).toBe(-100);
  });

  it("leeres Array wenn profile < 3 Samples", () => {
    expect(selectTraceSamples([])).toEqual([]);
    expect(selectTraceSamples([
      { t_ms: -100, vs_fpm: -200, agl_ft: 5, on_ground: false,
        g_force: 1, heading_true_deg: 0, groundspeed_kt: 130, indicated_airspeed_kt: 130, pitch_deg: 2, bank_deg: 0 },
    ])).toEqual([]);
  });

  it("leeres Array wenn alle Samples ausserhalb [-3500, 0]", () => {
    const profile: LandingProfilePoint[] = [
      { t_ms: -5000, vs_fpm: -110, agl_ft: 200, on_ground: false,
        g_force: 1, heading_true_deg: 0, groundspeed_kt: 130, indicated_airspeed_kt: 130, pitch_deg: 2, bank_deg: 0 },
      { t_ms: -4000, vs_fpm: -120, agl_ft: 100, on_ground: false,
        g_force: 1, heading_true_deg: 0, groundspeed_kt: 130, indicated_airspeed_kt: 130, pitch_deg: 2, bank_deg: 0 },
      { t_ms: -3700, vs_fpm: -130, agl_ft: 50, on_ground: false,
        g_force: 1, heading_true_deg: 0, groundspeed_kt: 130, indicated_airspeed_kt: 130, pitch_deg: 2, bank_deg: 0 },
    ];
    expect(selectTraceSamples(profile)).toEqual([]);
  });

  it("null/undefined profile → leeres Array", () => {
    expect(selectTraceSamples(null)).toEqual([]);
    expect(selectTraceSamples(undefined)).toEqual([]);
  });
});

// ───────────────────────────────────────────────────────────────────────────
describe("GSG-218 End-to-End — kompletter Fixture-Trockenlauf (Spec §8.3)", () => {
  it("Score-Basis ist -343 (Cascade), nicht -358 (Rate)", () => {
    expect(scoreBasisVs(GSG_218)).toBe(-343);
  });
  it("Score-Basis-Tone ist neutral (|343| in [200, 400))", () => {
    expect(vsTone(scoreBasisVs(GSG_218))).toBe("neutral");
  });
  it("Buckets matchen die Live-DB-Werte 1:1", () => {
    const buckets = computeBuckets(
      GSG_218.vs_smoothed_1500ms_fpm,
      GSG_218.vs_smoothed_1000ms_fpm,
      GSG_218.vs_smoothed_500ms_fpm,
      GSG_218.vs_smoothed_250ms_fpm,
      GSG_218.vs_at_edge_fpm,
    )!;
    expect(buckets.map((b) => Math.round(b.vs))).toEqual([-155, -218, -284, -343]);
  });
  it("Coaching-Tipp ist flare_lost (Trend dominiert)", () => {
    const buckets = computeBuckets(
      GSG_218.vs_smoothed_1500ms_fpm,
      GSG_218.vs_smoothed_1000ms_fpm,
      GSG_218.vs_smoothed_500ms_fpm,
      GSG_218.vs_smoothed_250ms_fpm,
      GSG_218.vs_at_edge_fpm,
    );
    expect(pickCoachingTip({
      buckets,
      peakGPost500ms: GSG_218.peak_g_post_500ms,
      flareReductionFpm: GSG_218.flare_reduction_fpm,
      vsAtEdgeFpm: GSG_218.vs_at_edge_fpm,
      vsSmoothed1500ms: GSG_218.vs_smoothed_1500ms_fpm,
    })).toBe("flare_lost");
  });
});
