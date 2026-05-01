import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { Bid, LoginResult } from "../types";
import { BidsList } from "./BidsList";

interface Props {
  session: LoginResult;
  onLogout: () => void;
}

export function Dashboard({ session, onLogout }: Props) {
  const { t } = useTranslation();
  const { profile, base_url } = session;
  const airlineLabel = profile.airline
    ? `${profile.airline.icao} — ${profile.airline.name}`
    : "—";
  const identAndRank = [profile.ident, profile.rank?.name]
    .filter(Boolean)
    .join(" · ");
  const [, setSelectedBid] = useState<Bid | null>(null);

  return (
    <>
      <section className="dashboard">
        <header className="dashboard__header">
          <div>
            <h2>{t("dashboard.welcome", { name: profile.name })}</h2>
            {identAndRank && (
              <p className="dashboard__ident">{identAndRank}</p>
            )}
            <p className="dashboard__site">
              {t("dashboard.site")}: <code>{base_url}</code>
            </p>
          </div>
          <button type="button" onClick={onLogout}>
            {t("actions.logout")}
          </button>
        </header>

        <dl className="dashboard__pilot">
          <dt>{t("dashboard.pilot_id")}</dt>
          <dd>{profile.pilot_id}</dd>

          <dt>{t("dashboard.airline")}</dt>
          <dd>{airlineLabel}</dd>

          <dt>{t("dashboard.current_airport")}</dt>
          <dd>{profile.curr_airport ?? "—"}</dd>

          <dt>{t("dashboard.home_airport")}</dt>
          <dd>{profile.home_airport ?? "—"}</dd>
        </dl>
      </section>

      <BidsList baseUrl={base_url} onSelect={setSelectedBid} />
    </>
  );
}
