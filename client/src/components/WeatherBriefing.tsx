import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";

interface MetarSnapshot {
  icao: string;
  raw: string;
  time: string;
  wind_direction_deg: number | null;
  wind_speed_kt: number | null;
  gust_kt: number | null;
  visibility_m: number | null;
  temperature_c: number | null;
  dewpoint_c: number | null;
  qnh_hpa: number | null;
}

interface MetarFetchState {
  kind: "loading" | "ready" | "error";
  data?: MetarSnapshot;
  error?: string;
}

interface Props {
  /** Departure airport ICAO. */
  dptIcao: string;
  /** Arrival airport ICAO. */
  arrIcao: string;
}

/** Auto-refresh window for the briefing panel. NOAA observations are
 *  reissued at most ~hourly, so polling more often is wasteful — we hit
 *  it once on mount and let the user click Refresh when they want a
 *  fresh look. */
function fmtVisibilityKm(meters: number | null): string {
  if (meters == null) return "—";
  const km = meters / 1000;
  return km >= 10 ? "≥ 10 km" : `${km.toFixed(1)} km`;
}

function fmtWind(
  direction: number | null,
  speed: number | null,
  gust: number | null,
): string {
  if (speed == null || speed === 0) return "Calm";
  const dir = direction == null ? "VRB" : `${direction.toFixed(0).padStart(3, "0")}°`;
  const gustPart = gust && gust > 0 ? `G${gust.toFixed(0)}` : "";
  return `${dir} / ${speed.toFixed(0)}${gustPart} kt`;
}

function MetarCard({
  label,
  state,
}: {
  label: string;
  state: MetarFetchState;
}) {
  const { t } = useTranslation();
  if (state.kind === "loading") {
    return (
      <div className="weather-card weather-card--loading">
        <h3 className="weather-card__label">{label}</h3>
        <p className="weather-card__hint">{t("weather.loading")}</p>
      </div>
    );
  }
  if (state.kind === "error") {
    return (
      <div className="weather-card weather-card--error">
        <h3 className="weather-card__label">{label}</h3>
        <p className="weather-card__hint">{state.error ?? t("weather.error")}</p>
      </div>
    );
  }
  const m = state.data!;
  return (
    <div className="weather-card">
      <header className="weather-card__header">
        <h3 className="weather-card__label">{label}</h3>
        <span className="weather-card__icao">{m.icao}</span>
      </header>
      <p className="weather-card__raw">{m.raw || "—"}</p>
      <dl className="weather-card__stats">
        <div>
          <dt>{t("weather.wind")}</dt>
          <dd>{fmtWind(m.wind_direction_deg, m.wind_speed_kt, m.gust_kt)}</dd>
        </div>
        <div>
          <dt>{t("weather.visibility")}</dt>
          <dd>{fmtVisibilityKm(m.visibility_m)}</dd>
        </div>
        <div>
          <dt>{t("weather.temp_dew")}</dt>
          <dd>
            {m.temperature_c != null ? `${m.temperature_c.toFixed(0)}°` : "—"}
            {" / "}
            {m.dewpoint_c != null ? `${m.dewpoint_c.toFixed(0)}°` : "—"}
          </dd>
        </div>
        <div>
          <dt>{t("weather.qnh")}</dt>
          <dd>{m.qnh_hpa != null ? `${m.qnh_hpa.toFixed(0)} hPa` : "—"}</dd>
        </div>
      </dl>
    </div>
  );
}

export function WeatherBriefing({ dptIcao, arrIcao }: Props) {
  const { t } = useTranslation();
  const [dpt, setDpt] = useState<MetarFetchState>({ kind: "loading" });
  const [arr, setArr] = useState<MetarFetchState>({ kind: "loading" });
  const [refreshing, setRefreshing] = useState(false);

  const fetchOne = useCallback(
    async (icao: string, set: (s: MetarFetchState) => void) => {
      set({ kind: "loading" });
      try {
        const data = await invoke<MetarSnapshot>("metar_get", { icao });
        set({ kind: "ready", data });
      } catch (err: unknown) {
        const msg =
          typeof err === "object" && err !== null && "message" in err
            ? String((err as { message: string }).message)
            : String(err);
        set({ kind: "error", error: msg });
      }
    },
    [],
  );

  // Initial load + reload whenever the route changes.
  useEffect(() => {
    if (!dptIcao && !arrIcao) return;
    void (async () => {
      setRefreshing(true);
      await Promise.all([
        dptIcao ? fetchOne(dptIcao, setDpt) : Promise.resolve(),
        arrIcao ? fetchOne(arrIcao, setArr) : Promise.resolve(),
      ]);
      setRefreshing(false);
    })();
  }, [dptIcao, arrIcao, fetchOne]);

  async function handleRefresh() {
    if (refreshing) return;
    setRefreshing(true);
    await Promise.all([
      dptIcao ? fetchOne(dptIcao, setDpt) : Promise.resolve(),
      arrIcao ? fetchOne(arrIcao, setArr) : Promise.resolve(),
    ]);
    setRefreshing(false);
  }

  return (
    <section className="weather-briefing">
      <header className="weather-briefing__header">
        <h2 className="weather-briefing__title">{t("weather.title")}</h2>
        <button
          type="button"
          className="weather-briefing__refresh"
          onClick={() => void handleRefresh()}
          disabled={refreshing}
          aria-label={t("weather.refresh")}
        >
          {refreshing ? "…" : "⟳"} <span>{t("weather.refresh")}</span>
        </button>
      </header>
      <div className="weather-briefing__cards">
        <MetarCard label={t("weather.departure")} state={dpt} />
        <MetarCard label={t("weather.arrival")} state={arr} />
      </div>
    </section>
  );
}
