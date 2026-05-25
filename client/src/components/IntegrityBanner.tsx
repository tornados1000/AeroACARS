// v0.13.0 Slice 6 — Mid-Session-Integrity-Banner
//
// Zeigt eine Warnung wenn der Recorder ein integrity-flag-Event
// gepublisht hat. Drei Severities:
//   info     — kein Banner (zu noisy für UI)
//   anomaly  — orange info banner, dismissable
//   critical — rotes blockierendes Banner, prominent
//
// Hinweis: Stream C macht den finalen Score-Trust auf PIREP-Submission;
// dieser Banner ist nur die LIVE-Warnung während des Flugs damit der
// Pilot weiß "hier stimmt was nicht, der PIREP wird vermutlich
// untrusted werden".

import { useTranslation } from "react-i18next";
import { useIntegrityFlags } from "../hooks/useIntegrityFlags";

export function IntegrityBanner() {
  const { t } = useTranslation();
  const { state, dismiss } = useIntegrityFlags();

  if (state.sessionSeverity === "info" || state.recentFlags.length === 0) return null;
  if (state.dismissed && state.sessionSeverity !== "critical") return null;

  const isCritical = state.sessionSeverity === "critical";
  const bgClass = isCritical
    ? "bg-red-900/95 border-red-500 text-red-50"
    : "bg-amber-800/95 border-amber-500 text-amber-50";

  const latestFlag = state.recentFlags[0];
  const flagType = latestFlag?.flag.type ?? "UNKNOWN";
  const flagPhase = latestFlag?.flag.phase ?? "—";

  return (
    <div
      className={`fixed top-12 left-1/2 -translate-x-1/2 z-50 px-4 py-3 rounded-lg border-2 shadow-xl max-w-2xl ${bgClass}`}
      role="alert"
      aria-live="assertive"
    >
      <div className="flex items-start gap-3">
        <span className="text-2xl" aria-hidden>
          {isCritical ? "⚠" : "ⓘ"}
        </span>
        <div className="flex-1">
          <p className="font-bold text-sm uppercase tracking-wide">
            {isCritical
              ? t("integrity.title_critical", "Data-Integrity-Problem entdeckt")
              : t("integrity.title_anomaly", "Datenanomalie")}
          </p>
          <p className="text-sm mt-1">
            {t("integrity.flag_description", {
              defaultValue: "{{type}} in Phase {{phase}}",
              type: flagType,
              phase: flagPhase,
            })}
          </p>
          {isCritical && (
            <p className="text-xs mt-2 opacity-80">
              {t("integrity.critical_warning",
                "Der PIREP wird wahrscheinlich als 'untrusted' eingestuft und für VA-Admin-Review markiert.")}
            </p>
          )}
          {state.recentFlags.length > 1 && (
            <p className="text-xs mt-1 opacity-70">
              {t("integrity.flag_count", {
                defaultValue: "{{count}} flags in dieser Session",
                count: state.recentFlags.length,
              })}
            </p>
          )}
        </div>
        {!isCritical && (
          <button
            type="button"
            onClick={dismiss}
            className="text-xs px-2 py-1 rounded bg-amber-700/50 hover:bg-amber-700/70"
            aria-label={t("integrity.dismiss", "Schließen")}
          >
            ✕
          </button>
        )}
      </div>
    </div>
  );
}
