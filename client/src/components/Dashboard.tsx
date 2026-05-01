import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { Bid, LoginResult, SimConnectionState } from "../types";
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

      <SimPanel onStateChange={onSimStateChange} debugMode={debugMode} />

      <BidsList baseUrl={session.base_url} onSelect={setSelectedBid} />
    </>
  );
}
