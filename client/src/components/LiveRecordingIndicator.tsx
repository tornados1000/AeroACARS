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

  // v0.5.51/v0.6.0 — Stale-Threshold von 60 auf 180 sec. Vorher
  // triggerte „FEHLER" sofort wenn der phpVMS-POST > 60 sec her war.
  // Mit der v0.6.0-Architektur (Memory-Outbox + eigener phpVMS-Worker
  // mit 3s-Tick + 4-30s phase-aware Cadence) ist „60 sec Pause"
  // absolut normal im Cruise. 180 sec unterscheidet echte Connection-
  // Probleme (3+ failed POST-Cycles inkl. 5-sec Per-Item-Timeout) von
  // normalen Pausen zwischen Batches.
  const STALE_THRESHOLD_SEC = 180;
  const status: "live" | "queued" | "stale" | "idle" =
    ageSecs == null
      ? "idle"
      : queuedCount > 0
        ? "queued"
        : ageSecs > STALE_THRESHOLD_SEC
          ? "stale"
          : "live";

  const label = t(`recording.status.${status}`);
  const detail =
    ageSecs == null
      ? t("recording.no_post_yet")
      : queuedCount > 0
        ? t("recording.queued_pending", { count: queuedCount })
        : t("recording.last_send_secs", { secs: ageSecs });

  // v0.5.51 — UI-Klarstellung. Vorher stand einfach nur die Zahl
  // `positionCount` ohne Label rechts in der Pille. Bei status="stale"
  // las das aus wie „FEHLER 1101" → Pilot denkt 1101 wäre ein Fehler-Code.
  // Jetzt: explizites Σ-Symbol + i18n-Tooltip + visueller Separator.
  return (
    <div
      className={`live-rec live-rec--${status}`}
      role="status"
      aria-live="polite"
      title={`${label} — ${detail} · ${t("recording.total_sent")}: ${positionCount}`}
    >
      <span className="live-rec__dot" aria-hidden="true" />
      <span className="live-rec__label">{label}</span>
      <span className="live-rec__detail">{detail}</span>
      <span className="live-rec__sep" aria-hidden="true">·</span>
      <span className="live-rec__count" title={t("recording.total_sent")}>
        Σ&nbsp;{positionCount}
      </span>
    </div>
  );
}
