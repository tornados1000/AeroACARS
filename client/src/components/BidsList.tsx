import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useTranslation } from "react-i18next";
import type { Bid, Flight, UiError } from "../types";

type State =
  | { kind: "loading" }
  | { kind: "error"; error: UiError }
  | { kind: "empty" }
  | { kind: "ready"; bids: Bid[] };

interface Props {
  baseUrl: string;
  /** Notify the parent when a bid is selected so it can drive the next step. */
  onSelect?: (bid: Bid | null) => void;
}

const KNOWN_ERROR_CODES = new Set([
  "not_logged_in",
  "network",
  "unauthenticated",
  "forbidden",
  "not_found",
  "rate_limited",
  "server",
  "bad_response",
]);

function errorKey(code: string): string {
  return KNOWN_ERROR_CODES.has(code)
    ? `bids.error.${code}`
    : "bids.error.unknown";
}

function formatFlightTime(minutes: number | null, locale: string): string {
  if (minutes == null) return "—";
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  if (h === 0) return `${m}m`;
  if (m === 0) return `${h}h`;
  return locale.startsWith("de") ? `${h}h ${m.toString().padStart(2, "0")}m` : `${h}h ${m}m`;
}

function formatDistanceNm(nmi: number | null, locale: string): string {
  if (nmi == null) return "—";
  return `${new Intl.NumberFormat(locale).format(Math.round(nmi))} nmi`;
}

function formatLevel(level: number | null): string | null {
  if (level == null || level <= 0) return null;
  return `FL${level.toString().padStart(3, "0")}`;
}

function buildCallsigns(flight: Flight): string {
  const icao = flight.airline?.icao?.trim();
  const iata = flight.airline?.iata?.trim();
  const fnum = flight.flight_number;
  const icaoCs = icao ? `${icao}${fnum}` : null;
  const iataCs = iata ? `${iata}${fnum}` : null;
  if (icaoCs && iataCs) return `${icaoCs} · ${iataCs}`;
  return icaoCs ?? iataCs ?? fnum;
}

function asUiError(err: unknown): UiError {
  return typeof err === "object" && err !== null && "code" in err
    ? (err as UiError)
    : { code: "unknown", message: String(err) };
}

/** Two-letter monogram fallback when no logo is available. */
function airlineMonogram(flight: Flight): string {
  return (
    flight.airline?.iata?.toUpperCase() ??
    flight.airline?.icao?.slice(0, 2).toUpperCase() ??
    "✈"
  );
}

export function BidsList({ baseUrl, onSelect }: Props) {
  const { t, i18n } = useTranslation();
  const [state, setState] = useState<State>({ kind: "loading" });
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [refreshing, setRefreshing] = useState(false);

  const fetchBids = useCallback(async () => {
    try {
      const bids = await invoke<Bid[]>("phpvms_get_bids");
      setState(bids.length === 0 ? { kind: "empty" } : { kind: "ready", bids });
    } catch (err: unknown) {
      setState({ kind: "error", error: asUiError(err) });
    }
  }, []);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const bids = await invoke<Bid[]>("phpvms_get_bids");
        if (cancelled) return;
        setState(bids.length === 0 ? { kind: "empty" } : { kind: "ready", bids });
      } catch (err: unknown) {
        if (cancelled) return;
        setState({ kind: "error", error: asUiError(err) });
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  async function handleRefresh() {
    if (refreshing) return;
    setRefreshing(true);
    await fetchBids();
    setRefreshing(false);
  }

  function handleSelect(bid: Bid) {
    const next = bid.id === selectedId ? null : bid.id;
    setSelectedId(next);
    onSelect?.(next === null ? null : bid);
  }

  async function openFlightPage(flight: Flight) {
    const url = `${baseUrl.replace(/\/$/, "")}/flights/${flight.id}`;
    try {
      await openUrl(url);
    } catch {
      // ignore — the user will see the URL didn't open and can retry
    }
  }

  async function openOfp(flight: Flight) {
    if (!flight.simbrief?.id) return;
    // paxstudio-theme convention; works on GSG and most paxstudio-based VAs.
    // TODO: per-VA configurable URL pattern in Phase 4.
    const url = `${baseUrl.replace(/\/$/, "")}/paxstudio/ofp/${flight.simbrief.id}`;
    try {
      await openUrl(url);
    } catch {
      // ignore
    }
  }

  return (
    <section className="bids">
      <header className="bids__header">
        <h2>{t("bids.title")}</h2>
        <button
          type="button"
          className="bids__refresh"
          onClick={handleRefresh}
          disabled={refreshing || state.kind === "loading"}
          aria-label={t("bids.refresh")}
          title={t("bids.refresh")}
        >
          {refreshing ? "…" : "⟳"} <span>{t("bids.refresh")}</span>
        </button>
      </header>

      {state.kind === "loading" && <p className="bids__hint">{t("bids.loading")}</p>}

      {state.kind === "empty" && <p className="bids__hint">{t("bids.empty")}</p>}

      {state.kind === "error" && (
        <p className="bids__error" role="alert">
          {t(errorKey(state.error.code))}
        </p>
      )}

      {state.kind === "ready" && (
        <ul className="bids__list">
          {state.bids.map((bid) => {
            const f = bid.flight;
            const dpt = f.dpt_airport?.icao ?? f.dpt_airport_id;
            const arr = f.arr_airport?.icao ?? f.arr_airport_id;
            const dptName = f.dpt_airport?.name ?? null;
            const arrName = f.arr_airport?.name ?? null;
            const callsign = buildCallsigns(f);
            const airlineName = f.airline?.name ?? null;
            const airlineLogo = f.airline?.logo?.trim() || null;
            const monogram = airlineMonogram(f);
            const level = formatLevel(f.level);
            const isSelected = bid.id === selectedId;

            return (
              <li key={bid.id}>
                <article
                  className={`bid-card ${isSelected ? "bid-card--selected" : ""}`}
                >
                  <button
                    type="button"
                    className="bid-card__body"
                    onClick={() => handleSelect(bid)}
                    aria-pressed={isSelected}
                    aria-expanded={isSelected}
                  >
                    <div className="bid-card__top">
                      <div className="bid-card__brand">
                        <div
                          className={`bid-card__logo ${
                            airlineLogo ? "" : "bid-card__logo--placeholder"
                          }`}
                        >
                          {airlineLogo ? (
                            <img src={airlineLogo} alt={airlineName ?? callsign} />
                          ) : (
                            <span>{monogram}</span>
                          )}
                        </div>
                        <div className="bid-card__title">
                          <span className="bid-card__callsign">{callsign}</span>
                          {airlineName && (
                            <span className="bid-card__airline">{airlineName}</span>
                          )}
                        </div>
                      </div>
                      <div className="bid-card__meta">
                        <span title={t("bids.flight_time")}>
                          ⏱ {formatFlightTime(f.flight_time, i18n.language)}
                        </span>
                        <span title={t("bids.distance")}>
                          📏 {formatDistanceNm(f.distance?.nmi ?? null, i18n.language)}
                        </span>
                        {level && (
                          <span title={t("bids.cruise_level")}>✈ {level}</span>
                        )}
                      </div>
                    </div>

                    <div className="bid-card__route">
                      <div className="bid-card__leg">
                        <span className="bid-card__icao">{dpt}</span>
                        {dptName && (
                          <span className="bid-card__airport-name">{dptName}</span>
                        )}
                      </div>
                      <div className="bid-card__path" aria-hidden="true">
                        <span className="bid-card__plane">✈</span>
                      </div>
                      <div className="bid-card__leg bid-card__leg--arrival">
                        <span className="bid-card__icao">{arr}</span>
                        {arrName && (
                          <span className="bid-card__airport-name">{arrName}</span>
                        )}
                      </div>
                    </div>
                  </button>

                  {isSelected && (
                    <div className="bid-card__details">
                      {f.route && (
                        <div className="bid-card__route-text">
                          <span className="bid-card__route-label">
                            {t("bids.route")}:
                          </span>{" "}
                          <code>{f.route}</code>
                        </div>
                      )}
                      <div className="bid-card__actions">
                        {f.simbrief?.id && (
                          <button
                            type="button"
                            className="button button--primary"
                            onClick={() => void openOfp(f)}
                          >
                            {t("bids.open_ofp")} ↗
                          </button>
                        )}
                        <button
                          type="button"
                          className="button"
                          onClick={() => void openFlightPage(f)}
                        >
                          {t("bids.open_flight_page")} ↗
                        </button>
                      </div>
                    </div>
                  )}
                </article>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
