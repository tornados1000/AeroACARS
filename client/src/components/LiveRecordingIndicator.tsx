import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

interface Props {
  /** ISO-8601 UTC timestamp of the most recent successful post. */
  lastPositionAt: string | null;
  /** How many positions are sitting in the offline queue. */
  queuedCount: number;
  /** Total number of positions sent across this flight. */
  positionCount: number;
}

/**
 * Visual "this flight is being recorded" indicator for the cockpit
 * panel — like the REC dot on a video camera. Three states:
 *
 *   * Live (green pulse): last post within ~30 s, queue empty.
 *   * Queued (amber, no pulse): network hiccup, but we're capturing
 *     locally and will replay.
 *   * Stale (red, no pulse): no successful post in over a minute —
 *     pilot should suspect a SimConnect or network issue.
 *
 * The "X seconds ago" line ticks every second so the pilot has live
 * feedback that the streamer hasn't frozen.
 */
export function LiveRecordingIndicator({
  lastPositionAt,
  queuedCount,
  positionCount,
}: Props) {
  const { t } = useTranslation();
  const [, setTick] = useState(0);

  // Local 1 Hz tick so the "X seconds ago" line stays current between
  // 2 s flight_status polls. Pure cosmetic — drives no logic.
  useEffect(() => {
    const id = setInterval(() => setTick((n) => n + 1), 1000);
    return () => clearInterval(id);
  }, []);

  const ageSecs = lastPositionAt
    ? Math.max(0, Math.floor((Date.now() - new Date(lastPositionAt).getTime()) / 1000))
    : null;

  const status: "live" | "queued" | "stale" | "idle" =
    ageSecs == null
      ? "idle"
      : queuedCount > 0
        ? "queued"
        : ageSecs > 60
          ? "stale"
          : "live";

  const label = t(`recording.status.${status}`);
  const detail =
    ageSecs == null
      ? t("recording.no_post_yet")
      : queuedCount > 0
        ? t("recording.queued_pending", { count: queuedCount })
        : t("recording.last_send_secs", { secs: ageSecs });

  return (
    <div
      className={`live-rec live-rec--${status}`}
      role="status"
      aria-live="polite"
      title={`${label} — ${detail}`}
    >
      <span className="live-rec__dot" aria-hidden="true" />
      <span className="live-rec__label">{label}</span>
      <span className="live-rec__detail">{detail}</span>
      <span className="live-rec__count" title={t("recording.total_sent")}>
        {positionCount}
      </span>
    </div>
  );
}
