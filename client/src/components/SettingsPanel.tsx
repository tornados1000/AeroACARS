import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import type { ActiveFlightInfo, SimKind, SimStatus } from "../types";
import type { Theme } from "../theme";
import { SimDebugPanel } from "./SimDebugPanel";
import { PmdgPremiumPanel } from "./PmdgPremiumPanel";
import { XPlanePremiumPanel } from "./XPlanePremiumPanel";
import { useConfirm } from "./ConfirmDialog";

const ALL_KINDS: SimKind[] = [
  "msfs2024",
  "msfs2020",
  "xplane11",
  "xplane12",
  "off",
];

interface Props {
  debugMode: boolean;
  onDebugModeChange: (next: boolean) => void;
  /** Auto-file the PIREP once the FSM reaches Arrived. Persisted
   *  via the App.tsx storage helpers. */
  autoFile: boolean;
  onAutoFileChange: (next: boolean) => void;
  /** Auto-start a flight when the aircraft is parked at one of the
   *  bid's departure airports. Persisted via App.tsx storage helpers. */
  autoStart: boolean;
  onAutoStartChange: (next: boolean) => void;
  /** Auto-purge per-flight JSONL log files older than 30 days on
   *  every app start. Persisted via App.tsx storage helpers; the
   *  actual sweep call is fired once at mount inside App.tsx. */
  autoDeleteFlightLogs: boolean;
  onAutoDeleteFlightLogsChange: (next: boolean) => void;
  /** When true, clicking the close button hides the window into the
   *  system tray (Win) / menubar (Mac) instead of quitting. Default
   *  off; toggle persisted in localStorage and synced to the Rust
   *  backend so the close-handler reads it directly. */
  minimizeToTray: boolean;
  onMinimizeToTrayChange: (next: boolean) => void;
  theme: Theme;
  onThemeChange: (next: Theme) => void;
  /** Latest sim telemetry — surfaced in the debug section when the
   *  user has enabled debug mode. Polled centrally by `useSimSession`. */
  simStatus: SimStatus | null;
  /** Active flight, used to surface heartbeat / position-post timing
   *  in the debug panel. Null when no flight is in progress. */
  activeFlight: ActiveFlightInfo | null;
}

export function SettingsPanel({
  debugMode,
  onDebugModeChange,
  autoFile,
  onAutoFileChange,
  autoStart,
  onAutoStartChange,
  autoDeleteFlightLogs,
  onAutoDeleteFlightLogsChange,
  minimizeToTray,
  onMinimizeToTrayChange,
  theme,
  onThemeChange,
  simStatus,
  activeFlight,
}: Props) {
  const { t, i18n } = useTranslation();
  const [kind, setKind] = useState<SimKind | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const k = await invoke<string>("sim_get_kind");
        if (!cancelled) setKind(k as SimKind);
      } catch {
        if (!cancelled) setKind("off");
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  async function handleKindChange(next: SimKind) {
    if (busy) return;
    setBusy(true);
    setKind(next);
    try {
      await invoke("sim_set_kind", { kind: next });
    } catch {
      // ignore
    } finally {
      setBusy(false);
    }
  }

  const language = i18n.resolvedLanguage ?? "en";

  return (
    <section className="settings">
      <header className="settings__header">
        <h2>{t("settings.title")}</h2>
        <p className="settings__hint">{t("settings.description")}</p>
      </header>

      <div className="settings__section">
        <h3>{t("settings.appearance_section")}</h3>

        <label className="settings__field">
          <span className="settings__field-label">
            {t("settings.language_label")}
          </span>
          <select
            value={language}
            onChange={(e) => i18n.changeLanguage(e.target.value)}
          >
            <option value="de">{t("actions.language_de")}</option>
            <option value="en">{t("actions.language_en")}</option>
          </select>
        </label>

        <label className="settings__field">
          <span className="settings__field-label">
            {t("settings.theme_label")}
          </span>
          <select
            value={theme}
            onChange={(e) => onThemeChange(e.target.value as Theme)}
          >
            <option value="dark">{t("settings.theme_dark")}</option>
            <option value="light">{t("settings.theme_light")}</option>
          </select>
        </label>
      </div>

      <div className="settings__section">
        <h3>{t("settings.simulator_section")}</h3>
        <p className="settings__row-hint">{t("settings.simulator_hint")}</p>
        <label className="settings__field">
          <span className="settings__field-label">
            {t("settings.simulator_label")}
          </span>
          <select
            value={kind ?? "off"}
            onChange={(e) => handleKindChange(e.target.value as SimKind)}
            disabled={busy || kind === null}
          >
            {ALL_KINDS.map((k) => (
              <option key={k} value={k}>
                {t(`sim.kinds.${k}`)}
              </option>
            ))}
          </select>
        </label>
      </div>

      <div className="settings__section">
        <h3>{t("settings.filing_section")}</h3>
        <label className="settings__checkbox">
          <input
            type="checkbox"
            checked={autoFile}
            onChange={(e) => onAutoFileChange(e.target.checked)}
          />
          <span>
            <strong>{t("settings.auto_file_label")}</strong>
            <span className="settings__row-hint">
              {t("settings.auto_file_hint")}
            </span>
          </span>
        </label>
        <label className="settings__checkbox">
          <input
            type="checkbox"
            checked={autoStart}
            onChange={(e) => onAutoStartChange(e.target.checked)}
          />
          <span>
            <strong>Auto-Start aufzeichnen</strong>
            <span className="settings__row-hint">
              Startet einen Flug automatisch, sobald das Flugzeug am
              Departure-Airport eines deiner Bids steht (≤ 5 km, On-Ground,
              Engines aus). Watcher tickt alle 3 s.
            </span>
          </span>
        </label>
      </div>

      <div className="settings__section">
        <h3>{t("settings.storage_section")}</h3>
        <FlightLogsManager
          autoDelete={autoDeleteFlightLogs}
          onAutoDeleteChange={onAutoDeleteFlightLogsChange}
        />
      </div>

      <div className="settings__section">
        <h3>{t("behaviour.section_title")}</h3>
        <label className="settings__checkbox">
          <input
            type="checkbox"
            checked={minimizeToTray}
            onChange={(e) => onMinimizeToTrayChange(e.target.checked)}
          />
          <span>
            <strong>{t("behaviour.minimize_to_tray_label")}</strong>
            <span
              className="settings__row-hint"
              dangerouslySetInnerHTML={{
                __html: t("behaviour.minimize_to_tray_hint"),
              }}
            />
          </span>
        </label>
      </div>

      <div className="settings__section">
        <h3>{t("settings.developer_section")}</h3>
        <label className="settings__checkbox">
          <input
            type="checkbox"
            checked={debugMode}
            onChange={(e) => onDebugModeChange(e.target.checked)}
          />
          <span>
            <strong>{t("settings.debug_mode_label")}</strong>
            <span className="settings__row-hint">
              {t("settings.debug_mode_hint")}
            </span>
          </span>
        </label>

        {debugMode && (
          <div className="settings__debug-panel">
            <SimDebugPanel status={simStatus} />
            <PhpvmsHeartbeatDebug activeFlight={activeFlight} />
            <PmdgPremiumPanel
              simState={simStatus?.state ?? "disconnected"}
              simSnapshot={simStatus?.snapshot ?? null}
            />
            <XPlanePremiumPanel
              simState={simStatus?.state ?? "disconnected"}
            />
          </div>
        )}
      </div>
    </section>
  );
}

/**
 * Live status card for the phpVMS API connection — keeps the same
 * visual language as the SimDebugPanel above so the Settings → Debug
 * area reads as a single coherent block. Re-renders once a second to
 * keep the relative ages live.
 *
 * State-color mapping mirrors the simulator card:
 *   - no active flight        → disconnected (grey)
 *   - heartbeat ≤ 45 s ago    → connected (green pulse)
 *   - heartbeat 45-90 s       → connecting (yellow pulse, "stale")
 *   - heartbeat > 90 s / never→ connecting (yellow, but with a stale label)
 * "Connecting" is reused for both warn cases because phpVMS doesn't
 * have an "error" CSS variant in the sim-panel system; a yellow pulse
 * is the right level of "this is fishy, look at me" without false-
 * alarming red on a single dropped packet.
 */
function PhpvmsHeartbeatDebug({ activeFlight }: { activeFlight: ActiveFlightInfo | null }) {
  const { t } = useTranslation();
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, []);

  const ageSec = (iso: string | null): number | null => {
    if (!iso) return null;
    return Math.max(0, Math.floor((now - new Date(iso).getTime()) / 1000));
  };
  const fmtAge = (sec: number | null): string => {
    if (sec === null) return t("phpvms_status.age_unknown");
    if (sec < 60) return t("phpvms_status.age_seconds", { seconds: sec });
    const m = Math.floor(sec / 60);
    const r = sec % 60;
    return r === 0
      ? t("phpvms_status.age_minutes", { minutes: m })
      : t("phpvms_status.age_minutes_seconds", { minutes: m, seconds: r });
  };

  if (!activeFlight) {
    return (
      <section className="sim-panel sim-panel--disconnected">
        <header className="sim-panel__header">
          <div className="sim-panel__header-left">
            <h2>{t("phpvms_status.title")}</h2>
            <span className="sim-panel__kind">{t("phpvms_status.badge_heartbeat")}</span>
          </div>
          <span className="sim-panel__state">
            <span className="sim-panel__dot" /> {t("phpvms_status.state_inactive")}
          </span>
        </header>
        <p className="sim-panel__hint">{t("phpvms_status.no_active_flight")}</p>
      </section>
    );
  }

  const heartbeatAge = ageSec(activeFlight.last_heartbeat_at);
  const positionAge = ageSec(activeFlight.last_position_at);
  // Heartbeat fires every 30s by design — anything beyond ~45s means a
  // posted call failed or the streamer hasn't gotten its first one in yet.
  const isStale = heartbeatAge === null || heartbeatAge > 45;
  const state = isStale ? "connecting" : "connected";
  const stateLabel = isStale
    ? t("phpvms_status.state_waiting")
    : t("phpvms_status.state_active");
  // Truncate the PIREP id for the badge — full id is ~16 chars, that's
  // too wide for a header pill. The first 6 chars are enough to
  // disambiguate in practice.
  const pirepBadge = activeFlight.pirep_id.length > 8
    ? `${activeFlight.pirep_id.slice(0, 6)}…`
    : activeFlight.pirep_id;

  return (
    <section className={`sim-panel sim-panel--${state}`}>
      <header className="sim-panel__header">
        <div className="sim-panel__header-left">
          <h2>{t("phpvms_status.title")}</h2>
          <span className="sim-panel__kind">
            {t("phpvms_status.badge_pirep_prefix")} {pirepBadge}
          </span>
        </div>
        <span className={`sim-panel__state sim-panel__state--${state}`}>
          <span className="sim-panel__dot" /> {stateLabel}
        </span>
      </header>
      <dl className="sim-panel__compact">
        <dt>{t("phpvms_status.row_last_position")}</dt>
        <dd>
          {fmtAge(positionAge)}
          {positionAge !== null && positionAge > 60 && (
            <span className="sim-panel__compact-muted"> · {t("phpvms_status.muted_stale")}</span>
          )}
        </dd>
        <dt>{t("phpvms_status.row_last_heartbeat")}</dt>
        <dd>
          {fmtAge(heartbeatAge)}
          {heartbeatAge === null && (
            <span className="sim-panel__compact-muted"> · {t("phpvms_status.muted_not_yet_sent")}</span>
          )}
        </dd>
        {activeFlight.queued_position_count > 0 && (
          <>
            <dt>{t("phpvms_status.row_position_queue")}</dt>
            <dd>
              {activeFlight.queued_position_count}{" "}
              <span className="sim-panel__compact-muted">{t("phpvms_status.muted_pending_offline")}</span>
            </dd>
          </>
        )}
      </dl>
    </section>
  );
}

/**
 * Settings → Speicher: lets the pilot manage on-disk per-flight JSONL
 * recorder files. Two controls:
 *  - Toggle for the auto-purge sweep that the App.tsx mount triggers
 *    (default ON, threshold 30 days).
 *  - "Alle löschen jetzt" button (with confirm) for the manual nuke.
 *
 * The backing files are at `<app_data_dir>/flight_logs/<pirep_id>.jsonl`
 * — see README → Troubleshooting for the exact OS-specific paths.
 * Stats are loaded once on mount and re-fetched after a successful
 * delete so the user sees the file count drop immediately.
 */
function FlightLogsManager({
  autoDelete,
  onAutoDeleteChange,
}: {
  autoDelete: boolean;
  onAutoDeleteChange: (next: boolean) => void;
}) {
  const { t } = useTranslation();
  const { confirm, dialog: confirmDialog } = useConfirm();
  const [stats, setStats] = useState<{ count: number; total_bytes: number } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Inline success-feedback shown right under the delete button.
  // Replaces the v0.3.1 `window.alert()` which is silently dropped on
  // macOS WKWebView (same bug class as `confirm()`). Cleared on next
  // refresh, so the user sees it briefly and then it goes away.
  const [doneMsg, setDoneMsg] = useState<string | null>(null);

  const refresh = async () => {
    try {
      const s = await invoke<{ count: number; total_bytes: number }>("flight_logs_stats");
      setStats(s);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  };

  useEffect(() => {
    void refresh();
  }, []);

  const handleDeleteAll = async () => {
    const ok = await confirm({
      message: t("settings.delete_all_logs_confirm"),
      destructive: true,
    });
    if (!ok) return;
    setBusy(true);
    setDoneMsg(null);
    try {
      const res = await invoke<{ deleted: number }>("flight_logs_delete_all");
      await refresh();
      setDoneMsg(
        t("settings.delete_all_logs_done", {
          count: res.deleted,
          defaultValue:
            res.deleted === 1
              ? t("settings.delete_all_logs_done_one", { count: res.deleted })
              : t("settings.delete_all_logs_done_other", { count: res.deleted }),
        }),
      );
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const fmtSize = (bytes: number): string => {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
    return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
  };

  const usageText = (() => {
    if (stats === null) return t("settings.storage_usage_loading");
    if (stats.count === 0) return t("settings.storage_usage_empty");
    const key = stats.count === 1
      ? "settings.storage_usage_count_one"
      : "settings.storage_usage_count_other";
    return t(key, { count: stats.count, size: fmtSize(stats.total_bytes) });
  })();

  return (
    <>
      {confirmDialog}
      <label className="settings__checkbox">
        <input
          type="checkbox"
          checked={autoDelete}
          onChange={(e) => onAutoDeleteChange(e.target.checked)}
        />
        <span>
          <strong>{t("settings.auto_delete_logs_label")}</strong>
          <span
            className="settings__row-hint"
            dangerouslySetInnerHTML={{ __html: t("settings.auto_delete_logs_hint") }}
          />
        </span>
      </label>

      <div className="storage-card">
        <div className="storage-card__row">
          <span className="storage-card__label">{t("settings.storage_usage_label")}</span>
          <span className="storage-card__value">{usageText}</span>
        </div>
        <div className="storage-card__actions">
          <button
            type="button"
            className="storage-card__btn storage-card__btn--danger"
            onClick={handleDeleteAll}
            disabled={busy || stats === null || stats.count === 0}
          >
            {busy ? t("settings.delete_all_logs_busy") : t("settings.delete_all_logs_button")}
          </button>
          <button
            type="button"
            className="storage-card__btn"
            onClick={() => void refresh()}
            disabled={busy}
            aria-label={t("settings.refresh_button_label")}
            title={t("settings.refresh_button_title")}
          >
            ↻
          </button>
        </div>
        {error && <p className="storage-card__error">{error}</p>}
        {doneMsg && <p className="storage-card__done">{doneMsg}</p>}
      </div>
    </>
  );
}
