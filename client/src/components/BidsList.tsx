import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useTranslation } from "react-i18next";
import type {
  ActiveFlightInfo,
  AirportInfo,
  Bid,
  Flight,
  SimConnectionState,
  SimSnapshot,
  UiError,
} from "../types";

/**
 * Maximum distance (in nautical miles) between the aircraft and the bid's
 * departure airport before "Start flight" is enabled. Mirrors the server-side
 * threshold in `cloudeacars-app/src/lib.rs::MAX_START_DISTANCE_NM`.
 */
const MAX_START_DISTANCE_NM = 5.0;

/** Great-circle distance in nautical miles. */
function distanceNm(lat1: number, lon1: number, lat2: number, lon2: number): number {
  const R = 6_371_008.8; // metres
  const toRad = (d: number) => (d * Math.PI) / 180;
  const phi1 = toRad(lat1);
  const phi2 = toRad(lat2);
  const dphi = toRad(lat2 - lat1);
  const dlam = toRad(lon2 - lon1);
  const a =
    Math.sin(dphi / 2) ** 2 +
    Math.cos(phi1) * Math.cos(phi2) * Math.sin(dlam / 2) ** 2;
  const meters = 2 * R * Math.asin(Math.sqrt(a));
  return meters / 1852;
}

type State =
  | { kind: "loading" }
  | { kind: "error"; error: UiError }
  | { kind: "empty" }
  | { kind: "ready"; bids: Bid[] };

interface Props {
  baseUrl: string;
  /** Sim connection state to gate the Start Flight button. */
  simState: SimConnectionState;
  /** Latest sim snapshot — used to compute distance to each bid's dpt airport. */
  simSnapshot: SimSnapshot | null;
  /** Whether a flight is already active (disables Start on every bid). */
  hasActiveFlight: boolean;
  /** Notify the parent when a bid is selected so it can drive the next step. */
  onSelect?: (bid: Bid | null) => void;
  /** Notify the parent that a flight just started. */
  onFlightStarted?: (flight: ActiveFlightInfo) => void;
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

export function BidsList({
  baseUrl,
  simState,
  simSnapshot,
  hasActiveFlight,
  onSelect,
  onFlightStarted,
}: Props) {
  const { t, i18n } = useTranslation();
  const [state, setState] = useState<State>({ kind: "loading" });
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [startingId, setStartingId] = useState<number | null>(null);
  const [startError, setStartError] = useState<{
    bidId: number;
    message: string;
  } | null>(null);
  /** Cached airport coords keyed by uppercase ICAO. */
  const [airports, setAirports] = useState<Record<string, AirportInfo>>({});
  /** Tracks ICAOs we've already requested so we don't fetch the same one twice. */
  const requestedIcaosRef = useRef<Set<string>>(new Set());

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

  // Whenever the bids change, fetch the coordinates of every unique departure
  // airport in the background. Results are cached server-side too, so this is
  // cheap on subsequent calls.
  useEffect(() => {
    if (state.kind !== "ready") return;
    const uniqueIcaos = new Set(
      state.bids.map((b) => b.flight.dpt_airport_id.trim().toUpperCase()),
    );
    for (const icao of uniqueIcaos) {
      if (!icao || requestedIcaosRef.current.has(icao)) continue;
      if (airports[icao]) continue;
      requestedIcaosRef.current.add(icao);
      void (async () => {
        try {
          const info = await invoke<AirportInfo>("airport_get", { icao });
          setAirports((prev) => ({ ...prev, [icao]: info }));
        } catch {
          // Leave the icao un-cached; the user will still get a clear error
          // if they try to start the flight (server-side check kicks in).
          requestedIcaosRef.current.delete(icao);
        }
      })();
    }
  }, [state, airports]);

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

  async function startFlight(bid: Bid) {
    if (startingId !== null || hasActiveFlight) return;
    setStartingId(bid.id);
    setStartError(null);
    try {
      const result = await invoke<ActiveFlightInfo>("flight_start", {
        bidId: bid.id,
      });
      onFlightStarted?.(result);
    } catch (err: unknown) {
      const ui = asUiError(err);
      // Map known backend error codes to localized messages; fall back to the
      // raw server-supplied message if the code is unfamiliar.
      const knownCodes = [
        "no_sim_snapshot",
        "not_on_ground",
        "not_at_departure",
        "missing_airline",
        "missing_aircraft",
        "flight_already_active",
        "bid_not_found",
        "aircraft_not_available",
        "phpvms_error",
      ];
      const message = knownCodes.includes(ui.code)
        ? `${t(`flight.error.${ui.code}`)} (${ui.message})`
        : ui.message;
      setStartError({ bidId: bid.id, message });
    } finally {
      setStartingId(null);
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

            // Compute distance from the aircraft to this bid's dpt airport (if
            // we have both pieces of info). Drives the proactive gating: the
            // Start button is enabled only when the aircraft is on the ground
            // and within MAX_START_DISTANCE_NM of the departure airport.
            const dptIcao = f.dpt_airport_id.trim().toUpperCase();
            const dptCoords = airports[dptIcao];
            let distanceToDptNm: number | null = null;
            if (
              simSnapshot &&
              dptCoords &&
              dptCoords.lat !== null &&
              dptCoords.lon !== null
            ) {
              distanceToDptNm = distanceNm(
                simSnapshot.lat,
                simSnapshot.lon,
                dptCoords.lat,
                dptCoords.lon,
              );
            }
            const onGround = simSnapshot?.on_ground ?? false;
            const tooFar =
              distanceToDptNm !== null &&
              distanceToDptNm > MAX_START_DISTANCE_NM;

            const startDisabled =
              startingId !== null ||
              hasActiveFlight ||
              simState !== "connected" ||
              !onGround ||
              tooFar;

            let startTitle = "";
            if (simState !== "connected") {
              startTitle = t("bids.start_disabled_no_sim");
            } else if (hasActiveFlight) {
              startTitle = t("bids.start_disabled_active_flight");
            } else if (!onGround) {
              startTitle = t("bids.start_disabled_not_on_ground");
            } else if (tooFar && distanceToDptNm !== null) {
              startTitle = t("bids.start_disabled_too_far", {
                distance: distanceToDptNm.toFixed(1),
                airport: dptIcao,
              });
            }

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

                  {/* Always-visible action row so the user can see how to start
                      a flight without first clicking to expand the card. */}
                  <div className="bid-card__actions">
                    <button
                      type="button"
                      className="button button--primary bid-card__start"
                      onClick={() => void startFlight(bid)}
                      disabled={startDisabled}
                      title={startTitle}
                    >
                      {startingId === bid.id
                        ? t("bids.starting")
                        : t("bids.start_flight")}
                    </button>
                    {distanceToDptNm !== null && (
                      <span
                        className={`bid-card__distance ${
                          tooFar ? "bid-card__distance--far" : "bid-card__distance--near"
                        }`}
                        title={t("bids.distance_to_departure", {
                          airport: dptIcao,
                        })}
                      >
                        {tooFar ? "✕ " : "✓ "}
                        {distanceToDptNm < 1
                          ? `< 1 nm ${t("bids.from_airport", { airport: dptIcao })}`
                          : `${distanceToDptNm.toFixed(1)} nm ${t("bids.from_airport", { airport: dptIcao })}`}
                      </span>
                    )}
                    {f.simbrief?.id && (
                      <button
                        type="button"
                        className="button"
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

                  {startError?.bidId === bid.id && (
                    <p className="bid-card__start-error" role="alert">
                      {startError.message}
                    </p>
                  )}

                  {isSelected && f.route && (
                    <div className="bid-card__details">
                      <div className="bid-card__route-text">
                        <span className="bid-card__route-label">
                          {t("bids.route")}:
                        </span>{" "}
                        <code>{f.route}</code>
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
