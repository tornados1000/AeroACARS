import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import type { ActiveFlightInfo, LoginResult, SimSnapshot } from "../types";
import { ResumeFlightBanner } from "./ResumeFlightBanner";
import { ActiveFlightPanel } from "./ActiveFlightPanel";
import { StableApproachBanner } from "./StableApproachBanner";
// v0.3.0: LoadsheetMonitor wird jetzt direkt im ActiveFlightPanel
// gerendert (zwischen InfoStrip und WeatherBriefing), damit das
// Loadsheet visuell zum aktiven Flug gehört statt als getrennte
// Section unter dem WeatherBriefing zu hängen.
import { DivertBanner } from "./DivertBanner";

interface Props {
  session: LoginResult;
  activeFlight: ActiveFlightInfo | null;
  setActiveFlight: (info: ActiveFlightInfo | null) => void;
  simSnapshot: SimSnapshot | null;
  /** Called when there's no active flight and the user wants to pick
   *  one — UI nudges them to the briefing tab. */
  onSwitchToBriefing: () => void;
  /** Auto-file the PIREP once the FSM reaches `Arrived`. Toggle in
   *  Settings → Filing. When false the pilot has to click
   *  "Flug beenden" themselves. */
  autoFile: boolean;
  /** v0.5.38: Stable-Approach-Banner anzeigen. Default ON. */
  approachAdvisoriesEnabled: boolean;
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
  simSnapshot,
  onSwitchToBriefing,
  autoFile,
  approachAdvisoriesEnabled,
}: Props) {
  const { t } = useTranslation();
  // v0.4.2: Snapshot der gerade gefilten Flugdaten — wird beim
  // onEnded-Callback gefüllt und nach 8 s automatisch wieder cleared.
  // Banner zeigt Pilot eine prominente „PIREP eingereicht"-Bestätigung
  // statt nur stillem Verschwinden des ActiveFlightPanel.
  const [filedFlightInfo, setFiledFlightInfo] = useState<{
    callsign: string;
    dpt: string;
    arr: string;
    at: number;
  } | null>(null);
  useEffect(() => {
    if (!filedFlightInfo) return;
    const id = window.setTimeout(() => setFiledFlightInfo(null), 8000);
    return () => window.clearTimeout(id);
  }, [filedFlightInfo]);

  // Auto-file the PIREP once the FSM marks the flight as Arrived
  // (BlocksOn + 30 s + engines off + parking brake set). Most pilots
  // wouldn't manually click "Flug beenden" if the app could just
  // submit on its own — and with all the pre-flight validation in
  // flight_end the worst case is a soft fail (the manual file dialog
  // surfaces, same as today). We attempt it exactly once per flight
  // via the ref, so a transient phase flutter back to TaxiIn doesn't
  // re-trigger.
  const autoFiledRef = useRef<string | null>(null);
  useEffect(() => {
    if (!activeFlight) {
      autoFiledRef.current = null;
      return;
    }
    if (!autoFile) return;
    if (activeFlight.phase !== "arrived") return;
    // Suppress auto-file when we've detected a divert. The pilot must
    // explicitly choose "submit as divert to X" / "submit as planned"
    // / "override" via the DivertBanner — silently filing with the
    // wrong arr_airport_id would defeat the whole point.
    if (activeFlight.divert_hint) return;
    if (autoFiledRef.current === activeFlight.pirep_id) return;
    autoFiledRef.current = activeFlight.pirep_id;
    void (async () => {
      try {
        await invoke("flight_end");
        // Clear the active flight in the React tree *immediately*
        // instead of waiting for the next 2 s status poll to notice.
        // Without this, pilots reported the cockpit panel sticking
        // around after the auto-file completed; the polling-only
        // path had a race window where a stale poll could overwrite
        // a "no flight" reading and bring it back briefly.
        setActiveFlight(null);
      } catch {
        // Validation failure (e.g. distance to airport > MAX, fuel
        // missing) — leave activeFlight alone so the manual "End"
        // button still works and surfaces the file-or-cancel dialog.
        // Don't reset the ref: we don't want a retry loop, the pilot
        // can hit the button manually.
      }
    })();
  }, [activeFlight, autoFile]);

  if (!activeFlight) {
    return (
      <>
        {/* v0.4.2: PIREP-Erfolgs-Banner. Bleibt 8 s sichtbar nach
            erfolgreichem Filing, dann auto-dismiss. Pilot kann
            sofort weiter arbeiten — Banner ist nicht-blockierend. */}
        {filedFlightInfo && (
          <div className="cockpit-pirep-success" role="status">
            <div className="cockpit-pirep-success__icon">✅</div>
            <div className="cockpit-pirep-success__text">
              <strong>{t("cockpit.pirep_filed_title")}</strong>
              <span>
                {t("cockpit.pirep_filed_detail", {
                  callsign: filedFlightInfo.callsign,
                  dpt: filedFlightInfo.dpt,
                  arr: filedFlightInfo.arr,
                })}
              </span>
            </div>
            <button
              type="button"
              className="cockpit-pirep-success__close"
              onClick={() => setFiledFlightInfo(null)}
              aria-label={t("cockpit.pirep_filed_dismiss")}
            >
              ✕
            </button>
          </div>
        )}
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
      </>
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
        <DivertBanner
          activeFlight={activeFlight}
          onResolved={() => setActiveFlight(null)}
        />
      )}

      {/* v0.5.38: Visual Stable-Approach-Advisory. Steht ÜBER dem
          ActiveFlightPanel sodass es bei jedem Flugzustand sichtbar
          ist. Banner blendet sich selbst aus wenn der Anflug stabil
          ist — null Visual-Footprint im Normal-Fall. */}
      <StableApproachBanner
        activeFlight={activeFlight}
        simSnapshot={simSnapshot}
        enabled={approachAdvisoriesEnabled}
      />

      {!activeFlight.was_just_resumed && (
        <ActiveFlightPanel
          info={activeFlight}
          simSnapshot={simSnapshot}
          onEnded={() => {
            // v0.4.2: Snapshot der gerade abgeschlossenen Flugdaten
            // an den Banner unten hochreichen — der Pilot soll eine
            // prominente Bestätigung sehen, nicht nur ein stilles
            // Verschwinden des ActiveFlightPanels.
            setFiledFlightInfo({
              callsign: activeFlight.airline_icao
                ? `${activeFlight.airline_icao} ${activeFlight.flight_number}`
                : activeFlight.flight_number,
              dpt: activeFlight.dpt_airport,
              arr: activeFlight.arr_airport,
              at: Date.now(),
            });
            setActiveFlight(null);
          }}
        />
      )}

      {/* Live-Loadsheet wird seit v0.3.0 direkt im ActiveFlightPanel
          gerendert — siehe Import-Kommentar oben. */}
    </>
  );
}
