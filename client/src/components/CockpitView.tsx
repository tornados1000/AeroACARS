import { useTranslation } from "react-i18next";
import type { ActiveFlightInfo, LoginResult, SimSnapshot } from "../types";
import { ResumeFlightBanner } from "./ResumeFlightBanner";
import { ActiveFlightPanel } from "./ActiveFlightPanel";

interface Props {
  session: LoginResult;
  activeFlight: ActiveFlightInfo | null;
  setActiveFlight: (info: ActiveFlightInfo | null) => void;
  simSnapshot: SimSnapshot | null;
  /** Called when there's no active flight and the user wants to pick
   *  one — UI nudges them to the briefing tab. */
  onSwitchToBriefing: () => void;
}

/**
 * Cockpit tab — the in-flight pilot view. Shows the resume banner
 * (when a stale flight is detected on startup), the active-flight
 * panel with weather briefing and PIREP actions, and a friendly empty
 * state when no flight is running.
 *
 * Deliberately no SimPanel here — sim telemetry lives in Settings →
 * Debug. The pilot during a flight cares about phase, route and
 * weather, not floating-point lat/lon.
 */
export function CockpitView({
  activeFlight,
  setActiveFlight,
  onSwitchToBriefing,
}: Props) {
  const { t } = useTranslation();

  if (!activeFlight) {
    return (
      <section className="cockpit-empty">
        <div className="cockpit-empty__icon" aria-hidden="true">
          ✈
        </div>
        <h2 className="cockpit-empty__title">{t("cockpit.empty_title")}</h2>
        <p className="cockpit-empty__hint">{t("cockpit.empty_hint")}</p>
        <button
          type="button"
          className="button button--primary"
          onClick={onSwitchToBriefing}
        >
          {t("cockpit.go_briefing")}
        </button>
      </section>
    );
  }

  return (
    <>
      <ResumeFlightBanner
        activeFlight={activeFlight}
        onAdopted={setActiveFlight}
        onCancelled={() => setActiveFlight(null)}
      />

      {!activeFlight.was_just_resumed && (
        <ActiveFlightPanel
          info={activeFlight}
          onEnded={() => setActiveFlight(null)}
        />
      )}
    </>
  );
}
