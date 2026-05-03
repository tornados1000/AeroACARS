import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import type { PmdgStatus } from "../types";

/**
 * Settings → Debug: PMDG SDK status card.
 *
 * Three states:
 *
 *  1. **Inactive** (no PMDG variant detected) — grey badge, hint
 *     that the panel becomes useful once a PMDG aircraft is loaded.
 *  2. **Active** (variant detected, data flowing) — green badge,
 *     shows variant + last-packet age. This is the happy path.
 *  3. **SDK disabled** (variant detected, subscribed, no packets
 *     for >5s) — amber warning with the exact instructions to
 *     enable the SDK in the pilot's `737NG3_Options.ini` /
 *     `777X_Options.ini`.
 *
 * Polls `pmdg_status` every 2s — cheap, the Tauri command just
 * reads a mutex on the adapter side.
 *
 * Phase H.4 / v0.2.0 — Boeing Premium Telemetry.
 */
export function PmdgPremiumPanel() {
  const { t } = useTranslation();
  const [status, setStatus] = useState<PmdgStatus | null>(null);

  useEffect(() => {
    let cancelled = false;
    async function poll() {
      try {
        const next = await invoke<PmdgStatus>("pmdg_status");
        if (!cancelled) setStatus(next);
      } catch {
        // IPC errors are transient on dev rebuilds; ignore.
      }
    }
    void poll();
    const id = window.setInterval(poll, 2000);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, []);

  if (!status) return null;

  // Decide the visual state.
  const variantLabel =
    status.variant === "ng3"
      ? "PMDG 737 NG3"
      : status.variant === "x777"
        ? "PMDG 777X"
        : null;

  const stateClass = status.looks_like_sdk_disabled
    ? "pmdg-panel--warn"
    : status.ever_received
      ? "pmdg-panel--active"
      : "pmdg-panel--inactive";

  const ageLabel =
    status.stale_secs === null
      ? "—"
      : status.stale_secs < 60
        ? `vor ${status.stale_secs}s`
        : `vor ${Math.floor(status.stale_secs / 60)} min`;

  return (
    <section className={`pmdg-panel ${stateClass}`}>
      <header className="pmdg-panel__header">
        <span className="pmdg-panel__title">
          {t("pmdg_panel.title")}
        </span>
        {variantLabel && (
          <span className="pmdg-panel__variant">{variantLabel}</span>
        )}
      </header>

      {/* No PMDG aircraft loaded */}
      {!variantLabel && (
        <p className="pmdg-panel__hint">
          {t("pmdg_panel.inactive_hint")}
        </p>
      )}

      {/* PMDG loaded but SDK probably not enabled */}
      {variantLabel && status.looks_like_sdk_disabled && (
        <div className="pmdg-panel__warning">
          <p className="pmdg-panel__warning-title">
            ⚠️ {t("pmdg_panel.sdk_disabled_title")}
          </p>
          <p>{t("pmdg_panel.sdk_disabled_explanation")}</p>
          <ol className="pmdg-panel__steps">
            <li>{t("pmdg_panel.step_close_msfs")}</li>
            <li>
              {t("pmdg_panel.step_open_options_ini")}{" "}
              <code className="pmdg-panel__code">
                {status.variant === "ng3"
                  ? "pmdg-aircraft-738\\work\\737NG3_Options.ini"
                  : "pmdg-aircraft-77er\\work\\777X_Options.ini"}
              </code>
            </li>
            <li>
              {t("pmdg_panel.step_add_lines")}
              <pre className="pmdg-panel__code-block">
{`[SDK]
EnableDataBroadcast=1`}
              </pre>
            </li>
            <li>{t("pmdg_panel.step_save_restart")}</li>
          </ol>
        </div>
      )}

      {/* Active — data flowing */}
      {variantLabel && status.ever_received && !status.looks_like_sdk_disabled && (
        <div className="pmdg-panel__active">
          <div className="pmdg-panel__metrics">
            <div className="pmdg-panel__metric">
              <span className="pmdg-panel__metric-label">
                {t("pmdg_panel.last_packet")}
              </span>
              <span className="pmdg-panel__metric-value">{ageLabel}</span>
            </div>
            <div className="pmdg-panel__metric">
              <span className="pmdg-panel__metric-label">
                {t("pmdg_panel.subscription")}
              </span>
              <span className="pmdg-panel__metric-value">
                {status.subscribed ? "✅" : "—"}
              </span>
            </div>
          </div>
          <p className="pmdg-panel__hint">
            {t("pmdg_panel.active_hint")}
          </p>
        </div>
      )}

      {/* Subscribed but no data yet (recent — give it a few seconds) */}
      {variantLabel && !status.ever_received && !status.looks_like_sdk_disabled && (
        <p className="pmdg-panel__hint">
          {t("pmdg_panel.waiting")}
        </p>
      )}
    </section>
  );
}
