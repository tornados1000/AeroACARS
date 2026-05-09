import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { UseUpdateCheckerResult } from "../hooks/useUpdateChecker";

/**
 * Inline „Update verfügbar"-Button im App-Header.
 *
 * v0.5.48: Source-of-Truth für Update-State ist jetzt der zentrale
 * `useUpdateChecker`-Hook (in App.tsx aufgerufen). Diese Komponente
 * konsumiert das Ergebnis. Visuelle Eskalation:
 *
 * - `fresh` (< 24 h gesehen): dezenter Button wie bisher
 * - `pulse` (≥ 24 h ignoriert): Button bekommt sanfte Pulse-Animation
 *   damit der Pilot ihn nicht weiter übersieht
 * - `banner` (≥ 72 h ignoriert): Button glüht zusätzlich + dauerhafte
 *   Pulsation. Parallel macht UpdateBanner das große Banner — Button
 *   bleibt aber sichtbar damit der Pilot direkt installieren kann
 *
 * Renders nichts wenn Hook `stage === "none"` meldet.
 */
export function UpdateButton({ checker }: { checker: UseUpdateCheckerResult }) {
  const { t } = useTranslation();
  const { update, stage, installing, progress, installAndRelaunch } = checker;
  const [open, setOpen] = useState(false);

  if (!update || stage === "none") return null;

  const cls = [
    "update-button",
    stage === "pulse" ? "update-button--pulse" : "",
    stage === "banner" ? "update-button--escalated" : "",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <>
      <button
        type="button"
        className={cls}
        onClick={() => setOpen(true)}
        title={t("update.button_title", { version: update.version })}
      >
        <span className="update-button__icon" aria-hidden="true">
          ⬇
        </span>
        <span>{t("update.button_label")}</span>
      </button>

      {open && (
        <div
          className="update-modal__backdrop"
          onClick={() => !installing && setOpen(false)}
        >
          <div
            className="update-modal"
            role="dialog"
            aria-labelledby="update-modal-title"
            onClick={(e) => e.stopPropagation()}
          >
            <h3 id="update-modal-title" className="update-modal__title">
              {t("update.modal_title", { version: update.version })}
            </h3>
            {update.body && (
              <p className="update-modal__notes">{update.body}</p>
            )}
            {progress && (
              <p className="update-modal__progress">{progress}</p>
            )}
            <div className="update-modal__actions">
              <button
                type="button"
                className="button button--primary"
                onClick={() => void installAndRelaunch()}
                disabled={installing}
              >
                {installing ? "…" : t("update.install_now")}
              </button>
              <button
                type="button"
                className="button"
                onClick={() => setOpen(false)}
                disabled={installing}
              >
                {t("update.later")}
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
