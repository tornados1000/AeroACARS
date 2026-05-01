import { useTranslation } from "react-i18next";
import type { Profile } from "../types";

interface Props {
  profile: Profile;
  onLogout: () => void;
}

/**
 * Pilot identity card for the Briefing tab. Big airline logo on the
 * left (key visual the user asked for), pilot name + chips on the
 * right, and current/home airports as a compact pair so the pilot can
 * tell at a glance whether the right aircraft is parked nearby.
 */
export function PilotHeader({ profile, onLogout }: Props) {
  const { t } = useTranslation();
  const airline = profile.airline;
  const initials = (profile.name || "?")
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((s) => s[0]?.toUpperCase() ?? "")
    .join("");

  return (
    <section className="pilot-header">
      <div className="pilot-header__logo">
        {airline?.logo ? (
          <img src={airline.logo} alt={airline.name} />
        ) : (
          <div className="pilot-header__logo-fallback" aria-hidden="true">
            {airline?.icao ?? "✈"}
          </div>
        )}
      </div>

      <div className="pilot-header__identity">
        <h2 className="pilot-header__name">{profile.name}</h2>
        <div className="pilot-header__chips">
          {profile.ident && (
            <span className="pilot-header__chip">{profile.ident}</span>
          )}
          {profile.rank?.name && (
            <span className="pilot-header__chip pilot-header__chip--muted">
              {profile.rank.name}
            </span>
          )}
          {airline && (
            <span className="pilot-header__chip pilot-header__chip--muted">
              {airline.icao} · {airline.name}
            </span>
          )}
        </div>
      </div>

      <div className="pilot-header__locations">
        <div className="pilot-header__loc">
          <span className="pilot-header__loc-label">
            {t("dashboard.current_short")}
          </span>
          <span className="pilot-header__loc-value">
            {profile.curr_airport ?? "—"}
          </span>
        </div>
        <div className="pilot-header__loc">
          <span className="pilot-header__loc-label">
            {t("dashboard.home_short")}
          </span>
          <span className="pilot-header__loc-value">
            {profile.home_airport ?? "—"}
          </span>
        </div>
      </div>

      <button
        type="button"
        className="pilot-header__logout"
        onClick={onLogout}
        title={initials}
      >
        {t("actions.logout")}
      </button>
    </section>
  );
}
