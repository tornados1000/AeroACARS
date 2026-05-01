import { useTranslation } from "react-i18next";
import type { FlightPhase } from "../types";

/**
 * Visual timeline of the major flight phases with a small SVG plane
 * that glides between checkpoints as the FSM advances. We don't render
 * every FSM phase as its own checkpoint — that's eleven and would be
 * cluttered — instead we collapse them into seven major checkpoints
 * the pilot recognises from the flight plan.
 *
 * The plane's `transform: translateX(...)` is CSS-transitioned so the
 * jump between phases looks like the aircraft taxiing / accelerating
 * forward instead of teleporting.
 */
interface Props {
  phase: FlightPhase;
}

interface Checkpoint {
  key: string;
  fsm: FlightPhase[];
}

// Ten checkpoints — every FSM phase has a visually distinct slot
// except for the very short ones (TakeoffRoll merges into Takeoff,
// Final merges into Approach, BlocksOn merges into Gate). Keeps
// Climb / Cruise / Descent as separate steps so the pilot sees a
// proper "rising arc" through the flight.
const CHECKPOINTS: Checkpoint[] = [
  { key: "boarding", fsm: ["preflight", "boarding"] },
  { key: "pushback", fsm: ["pushback"] },
  { key: "taxi", fsm: ["taxi_out"] },
  { key: "takeoff", fsm: ["takeoff_roll", "takeoff"] },
  { key: "climb", fsm: ["climb"] },
  { key: "cruise", fsm: ["cruise"] },
  { key: "descent", fsm: ["descent"] },
  { key: "approach", fsm: ["approach", "final"] },
  { key: "landing", fsm: ["landing", "taxi_in"] },
  { key: "gate", fsm: ["blocks_on", "arrived", "pirep_submitted"] },
];

function activeIndex(phase: FlightPhase): number {
  for (let i = 0; i < CHECKPOINTS.length; i++) {
    if (CHECKPOINTS[i]!.fsm.includes(phase)) return i;
  }
  return 0;
}

export function PhaseTimeline({ phase }: Props) {
  const { t } = useTranslation();
  const current = activeIndex(phase);
  const lastIndex = CHECKPOINTS.length - 1;
  // 0..1 — the plane sits exactly on the active checkpoint, smoothly
  // transitioned in CSS so the move between phases feels alive.
  const progress = lastIndex === 0 ? 0 : current / lastIndex;

  return (
    <div className="phase-timeline" aria-label={t("phase_timeline.title")}>
      <div className="phase-timeline__track">
        <div
          className="phase-timeline__track-fill"
          style={{ width: `${progress * 100}%` }}
        />
        {CHECKPOINTS.map((cp, i) => {
          const reached = i <= current;
          return (
            <div
              key={cp.key}
              className={`phase-timeline__node ${
                i < current ? "phase-timeline__node--past" : ""
              } ${i === current ? "phase-timeline__node--current" : ""} ${
                reached ? "phase-timeline__node--reached" : ""
              }`}
              style={{ left: `${(i / lastIndex) * 100}%` }}
            >
              <span className="phase-timeline__dot" />
              <span className="phase-timeline__label">
                {t(`phase_timeline.nodes.${cp.key}`)}
              </span>
            </div>
          );
        })}
      </div>
    </div>
  );
}
