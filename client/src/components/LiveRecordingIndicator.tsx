import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

interface Props {
  /** ISO-8601 UTC timestamp of the most recent successful post. */
  lastPositionAt: string | null;
  /** How many positions are sitting in the in-memory outbox awaiting POST. */
  queuedCount: number;
  /** Total number of positions sent across this flight. */
  positionCount: number;
  /**
   * v0.6.2 — Connection-Health from phpVMS-Worker.
   *   - "live"    → letzter POST war Erfolg
   *   - "failing" → letzter POST scheiterte (echter Network-Loss)
   *   - "blocked" → v0.7.17 (B-007) phpVMS lehnt mit 401/403 ab (Account
   *     gesperrt / inaktiv / API-Key revoked). Worker hat sich beendet,
   *     keine weiteren Retries — Pilot muss VA-Admin kontaktieren.
   *
   * Wird zusammen mit `queuedCount` für die Status-Anzeige verwendet.
   *
   * Vor v0.6.2 zeigte der Indikator „queued offline" für jeden Backlog,
   * was zwischen normalen Sync-Pausen und echten Connection-Loss nicht
   * unterscheiden konnte → Pilot dachte er sei offline obwohl alles ok.
   */
  connectionState?: "live" | "failing" | "blocked";
}

/**
 * Visual "this flight is being recorded" indicator for the cockpit
 * panel — like the REC dot on a video camera. Four states:
 *
 *   * Live   (grün, pulse): connection live, no backlog → alles ok.
 *   * Sync   (blau, soft pulse): connection live, backlog in der
 *     Outbox wartet auf nächsten POST-Cycle (= normal in Cruise mit
 *     30s-Cadence). Pilot muss nichts tun, wird automatisch raus.
 *   * Offline (rot, no pulse): letzter POST scheiterte. Echte
 *     Verbindungs-Probleme. Backlog wächst wenn nicht behoben.
 *   * Stale  (grau, no pulse): kein POST seit > 3 min. App vermutlich
 *     hängt oder Sim-Disconnect.
 *
 * Die "X seconds ago" Zeile tickt jede Sekunde damit der Pilot Live-
 * Feedback hat dass der Streamer nicht eingefroren ist.
 */
export function LiveRecordingIndicator({
  lastPositionAt,
  queuedCount,
  positionCount,
  connectionState,
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
  // mit phase-aware Cadence 4-30s) ist „60 sec Pause" absolut normal
  // im Cruise. 180 sec unterscheidet echte Connection-Probleme von
  // normalen Pausen zwischen Batches.
  const STALE_THRESHOLD_SEC = 180;

  // v0.6.2 — 3 Status statt 2 (Live / Sync / Offline / Stale).
  // Priority: Stale > Offline (failing) > Sync (queued+live) > Live.
  // - Stale wenn lange nichts: vermutlich App tot ODER Sim-Disconnect
  // - Offline wenn letzter POST gescheitert: echte Verbindungs-Probleme
  // - Sync wenn Backlog UND letzter POST Erfolg: nur Cadence-Pause
  // - Live wenn Backlog leer UND letzter POST Erfolg
  // v0.7.17 (B-007): "blocked" hat hoechste Prioritaet — wir wollen
  // dass Pilot SOFORT sieht dass sein Account gesperrt ist, nicht
  // versteckt unter "stale" oder "offline".
  const status: "live" | "sync" | "offline" | "stale" | "blocked" | "idle" =
    connectionState === "blocked"
      ? "blocked"
      : ageSecs == null
        ? "idle"
        : ageSecs > STALE_THRESHOLD_SEC
          ? "stale"
          : connectionState === "failing"
            ? "offline"
            : queuedCount > 0
              ? "sync"
              : "live";

  const label = t(`recording.status.${status}`);
  const detail =
    status === "blocked"
      ? t("recording.blocked_detail")
      : ageSecs == null
        ? t("recording.no_post_yet")
        : status === "offline"
          ? t("recording.offline_pending", { count: queuedCount })
          : status === "sync"
            ? t("recording.sync_pending", { count: queuedCount })
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
