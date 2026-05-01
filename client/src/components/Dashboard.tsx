import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import type {
  ActiveFlightInfo,
  Bid,
  LoginResult,
  SimConnectionState,
  SimSnapshot,
} from "../types";
import { ActiveFlightPanel } from "./ActiveFlightPanel";
import { BidsList } from "./BidsList";
import { SimPanel } from "./SimPanel";

interface Props {
  session: LoginResult;
  onLogout: () => void;
  onSimStateChange?: (state: SimConnectionState) => void;
  debugMode: boolean;
}

export function Dashboard({
  session,
  onLogout,
  onSimStateChange,
  debugMode,
}: Props) {
  const { t } = useTranslation();
  const { profile } = session;
  const airlineLabel = profile.airline
    ? `${profile.airline.icao} · ${profile.airline.name}`
    : null;
  const [, setSelectedBid] = useState<Bid | null>(null);
  const [activeFlight, setActiveFlight] = useState<ActiveFlightInfo | null>(
    null,
  );
  const [simState, setSimState] = useState<SimConnectionState>("disconnected");
  const [simSnapshot, setSimSnapshot] = useState<SimSnapshot | null>(null);

  // On dashboard mount, check whether a flight is already active (e.g. after
  // the app was restarted while a flight was running).
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const flight = await invoke<ActiveFlightInfo | null>("flight_status");
        if (!cancelled) setActiveFlight(flight);
      } catch {
        // ignore
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  function handleSimStateChange(next: SimConnectionState) {
    setSimState(next);
    onSimStateChange?.(next);
  }

  return (
    <>
      <section className="dashboard">
        <div className="dashboard__person">
          <h2 className="dashboard__name">{profile.name}</h2>
          <div className="dashboard__chips">
            {profile.ident && (
              <span className="dashboard__chip">{profile.ident}</span>
            )}
            {profile.rank?.name && (
              <span className="dashboard__chip dashboard__chip--muted">
                {profile.rank.name}
              </span>
            )}
            {airlineLabel && (
              <span className="dashboard__chip dashboard__chip--muted">
                {airlineLabel}
              </span>
            )}
          </div>
        </div>

        <div className="dashboard__locations">
          <div className="dashboard__loc">
            <span className="dashboard__loc-label">
              {t("dashboard.current_short")}
            </span>
            <span className="dashboard__loc-value">
              {profile.curr_airport ?? "—"}
            </span>
          </div>
          <div className="dashboard__loc">
            <span className="dashboard__loc-label">
              {t("dashboard.home_short")}
            </span>
            <span className="dashboard__loc-value">
              {profile.home_airport ?? "—"}
            </span>
          </div>
        </div>

        <button
          type="button"
          className="dashboard__logout"
          onClick={onLogout}
        >
          {t("actions.logout")}
        </button>
      </section>

      <SimPanel
        onStateChange={handleSimStateChange}
        onSnapshotChange={setSimSnapshot}
        debugMode={debugMode}
      />

      <ActiveFlightPanel onEnded={() => setActiveFlight(null)} />

      <BidsList
        baseUrl={session.base_url}
        simState={simState}
        simSnapshot={simSnapshot}
        hasActiveFlight={activeFlight !== null}
        onSelect={setSelectedBid}
        onFlightStarted={setActiveFlight}
      />
    </>
  );
}
