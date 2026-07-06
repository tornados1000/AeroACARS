// #phase-v2 Cutover: Regressions-Schutz fuer die Phasen-Badge-Entscheidung.
// Das Badge zeigt jetzt die v2-Phase; bei einer ATC-Hoehen-Restriktion
// (shadow_segment === 'level') im En-Route-Band zeigt es 'Level' -- aber NIE
// am Boden (sonst ueberschriebe ein Level-Segment 'Taxi'/'Boarding').
import { describe, it, expect } from "vitest";
import { phaseBadgeDisplay } from "./ActiveFlightPanel";

describe("phaseBadgeDisplay (#phase-v2 cutover)", () => {
  it("shows the normal phase when no level segment is present", () => {
    expect(phaseBadgeDisplay("cruise", undefined)).toEqual({
      labelKey: "cruise",
      className: "cruise",
    });
    expect(phaseBadgeDisplay("climb", "climbing")).toEqual({
      labelKey: "climb",
      className: "climb",
    });
  });

  it("shows Level on a level-off restriction during climb or descent", () => {
    for (const phase of ["climb", "descent"]) {
      expect(phaseBadgeDisplay(phase, "level")).toEqual({
        labelKey: "level",
        className: "level",
      });
    }
  });

  it("keeps Cruise as Cruise even though its segment is 'level'", () => {
    // In steady cruise the kinematic segment IS "level" (rate/VS ~0). Cruise is
    // the normal state, not a restriction — the badge must stay "cruise".
    expect(phaseBadgeDisplay("cruise", "level")).toEqual({
      labelKey: "cruise",
      className: "cruise",
    });
  });

  it("never overrides ground/terminal phases with Level", () => {
    // A stray level segment outside the en-route band must not hijack the phase.
    for (const phase of [
      "taxi_out",
      "boarding",
      "takeoff",
      "approach",
      "final",
      "landing",
      "taxi_in",
      "arrived",
    ]) {
      expect(phaseBadgeDisplay(phase, "level")).toEqual({
        labelKey: phase,
        className: phase,
      });
    }
  });

  it("ignores other segment values", () => {
    expect(phaseBadgeDisplay("descent", "descending")).toEqual({
      labelKey: "descent",
      className: "descent",
    });
    expect(phaseBadgeDisplay("cruise", null)).toEqual({
      labelKey: "cruise",
      className: "cruise",
    });
  });
});
