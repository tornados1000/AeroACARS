import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useTranslation } from "react-i18next";
import type {
  ActiveFlightInfo,
  AirportInfo,
  Bid,
  Flight,
  SimBriefOfp,
  SimConnectionState,
  SimSnapshot,
  UiError,
} from "../types";

/**
 * Maximum distance (in nautical miles) between the aircraft and the bid's
 * departure airport before "Start flight" is enabled. Mirrors the server-side
 * threshold in `aeroacars-app/src/lib.rs::MAX_START_DISTANCE_NM`.
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

import type { Profile } from "../types";

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
  /** Called whenever the user hits the Refresh button — passes a fresh
   *  profile fetched from phpVMS so the parent can update the cached
   *  session and the PilotHeader picks up new curr_airport/etc. without
   *  requiring a logout/login cycle. v0.1.30. */
  onProfileRefreshed?: (profile: Profile) => void;
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

/// phpVMS-Flight-Type-Code → kurzes UI-Label.
/// Codes laut phpVMS-Core: J=Sched.Pax, F=Sched.Cargo, C=Charter,
/// X=Reposition, I=Special, T=Training, M=Military, R=Repositioning.
function flightTypeLabel(type: string): string {
  switch (type.toUpperCase()) {
    case "J": return "PAX";
    case "F": return "CARGO";
    case "C": return "CHARTER";
    case "X":
    case "R": return "REPO";
    case "T": return "TRAINING";
    case "M": return "MIL";
    case "I": return "SPECIAL";
    default: return type.toUpperCase();
  }
}

/// CSS-Class-Suffix abhängig vom Flight-Type — steuert die Badge-Farbe.
/// Pax = blau, Cargo = orange, Charter = lila, Repo = grau.
function flightTypeKind(type: string): string {
  switch (type.toUpperCase()) {
    case "J": return "pax";
    case "F": return "cargo";
    case "C": return "charter";
    case "X":
    case "R": return "repo";
    default: return "other";
  }
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
  onProfileRefreshed,
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

  // Background-poll bids so a freshly booked flight on phpVMS appears in
  // the list almost immediately — the pilot has the app open precisely
  // because they're about to fly, so a 15 s tick feels live without
  // burning through the phpVMS rate-limit budget (60 req/min/IP).
  // Pauses while a flight is active (no new flight can start anyway).
  useEffect(() => {
    if (hasActiveFlight) return;
    const id = setInterval(() => {
      void fetchBids();
    }, 15_000);
    return () => clearInterval(id);
  }, [hasActiveFlight, fetchBids]);

  /** Combined refresh: bid list + sim-position cache + pilot profile.
   *
   *  v0.1.30 added the profile re-fetch: pilots reported the
   *  "Standort" (curr_airport) chip in the header staying wrong even
   *  after their PIREP filed and phpVMS server-side updated the
   *  pilot's location. Cause: the cached LoginResult only ever held
   *  the login-time profile; a manual logout/login was the only way
   *  to refresh it. Now hitting "Aktualisieren" also pulls a fresh
   *  profile and bubbles it up to the parent so the header updates.
   *
   *  All three calls fire in parallel — the slowest network call
   *  bounds the spinner duration; the sim resync and any profile
   *  failure are non-fatal.
   */
  async function handleRefresh() {
    if (refreshing) return;
    setRefreshing(true);
    const [, , freshProfile] = await Promise.all([
      fetchBids(),
      invoke("sim_force_resync").catch(() => {
        // Adapter command failures only happen if the mutex is
        // poisoned (= app already broken). Don't fail the whole
        // refresh because of it.
      }),
      invoke<Profile | null>("phpvms_refresh_profile").catch(() => null),
    ]);
    if (freshProfile && onProfileRefreshed) {
      onProfileRefreshed(freshProfile);
    }
    // Tiny visible spinner tail so the pilot sees confirmation even
    // when both calls return instantly.
    setTimeout(() => setRefreshing(false), 400);
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
        "aircraft_mismatch",
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

  // Sim-position health check. v0.1.29 onwards we ONLY surface the
  // warning row when something actually looks wrong — in the happy
  // path (sim connected, position fresh and real) the row is
  // invisible so it doesn't clutter the page. Two failure modes
  // catch the bugs we've actually seen:
  //
  //   * sim says it's connected but lat/lon is exactly 0,0 →
  //     adapter's default-uninitialised state (= no real frame yet).
  //   * sim says it's connected and we have lat/lon, but the bid
  //     list shows "X nm from <airport>" with X far above any sane
  //     threshold (set by tooFar in the bid card below) AND the
  //     pilot tries to start anyway. We can't easily pre-compute
  //     this here because it depends on the airport coords cache,
  //     so the per-bid card is responsible for showing the warning
  //     inline (start-button title + start-disabled state). This
  //     row only catches the "no position at all" case.
  const hasSimPosition =
    simSnapshot !== null &&
    !(simSnapshot.lat === 0 && simSnapshot.lon === 0);
  const showPositionWarning = simState === "connected" && !hasSimPosition;

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

      {showPositionWarning && (
        <div
          className="bids__sim-position bids__sim-position--warn"
          role="status"
        >
          <span className="bids__sim-position-icon" aria-hidden="true">⚠️</span>
          <span className="bids__sim-position-message">
            {t("bids.sim_position_warning")}
          </span>
        </div>
      )}

      {state.kind === "loading" && <p className="bids__hint">{t("bids.loading")}</p>}

      {state.kind === "empty" && <p className="bids__hint">{t("bids.empty")}</p>}

      {state.kind === "error" && (
        <div className="bids__error" role="alert">
          <p>{t(errorKey(state.error.code))}</p>
          {/* For decode failures the technical message tells us *which*
              bid / field broke parsing — Ralf hit this on the Eurowings
              test instance. Surface it in a <details> so the pilot can
              copy/paste the snippet without it dominating the panel. */}
          {state.error.code === "bad_response" && state.error.message && (
            <details className="bids__error-details">
              <summary>{t("bids.error.details_summary")}</summary>
              <code>{state.error.message}</code>
            </details>
          )}
        </div>
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
            // Defensive: a snapshot reporting EXACTLY 0,0 lat/lon is
            // never a real airport — both adapters return that as
            // their default uninitialised value, and the open ocean
            // off the African coast is the only real point matching
            // (no real flight starts there). Treat as "no position
            // yet" rather than computing a 5000-nm phantom distance.
            // Belt-and-braces alongside the backend stale-snapshot
            // clear (commits in adapter.rs) so a brief race window
            // between Connected-state and first-real-position can't
            // surface a wrong "too far" gate to the pilot.
            const hasRealPosition =
              simSnapshot !== null &&
              !(simSnapshot.lat === 0 && simSnapshot.lon === 0);
            let distanceToDptNm: number | null = null;
            if (
              hasRealPosition &&
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
            const noPositionYet =
              simState === "connected" && !hasRealPosition;

            const startDisabled =
              startingId !== null ||
              hasActiveFlight ||
              simState !== "connected" ||
              noPositionYet ||
              !onGround ||
              tooFar;

            let startTitle = "";
            if (simState !== "connected") {
              startTitle = t("bids.start_disabled_no_sim");
            } else if (hasActiveFlight) {
              startTitle = t("bids.start_disabled_active_flight");
            } else if (noPositionYet) {
              startTitle = t("bids.start_disabled_no_position");
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
                        {f.flight_type && (
                          <span
                            className={`bid-card__type-badge bid-card__type-badge--${flightTypeKind(f.flight_type)}`}
                            title={t("bids.flight_type")}
                          >
                            {flightTypeLabel(f.flight_type)}
                          </span>
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

                  {isSelected && (
                    <BidDetails flight={f} />
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

/**
 * Ausgeklappte Bid-Card-Details (v0.3.0):
 * - Aircraft-Info aus SimBrief-Subfleet (B738 · Boeing 737-800)
 * - Pax/Cargo-Load aus den fares
 * - Route-String (kommt aus phpVMS-Bid)
 * - SimBrief-Plan-Vorschau (Block-Fuel, Trip-Burn, TOW, LDW, Reserve,
 *   ZFW, Alternate) wird per `fetch_simbrief_preview` Tauri-Command
 *   geholt sobald die Card ausgeklappt wird. Lädt asynchron, kein
 *   Blocking — Pilot sieht erstmal die Route, der Plan-Block flutscht
 *   in 1-2s nach.
 */
function BidDetails({ flight }: { flight: Flight }) {
  const { t } = useTranslation();
  const [plan, setPlan] = useState<SimBriefOfp | null>(null);
  const [planError, setPlanError] = useState<string | null>(null);
  const [planLoading, setPlanLoading] = useState(false);

  // OFP-Vorschau bei Bid-Selection holen. Nur einmal pro Render-Lifetime.
  useEffect(() => {
    const ofpId = flight.simbrief?.id;
    if (!ofpId) return;
    setPlanLoading(true);
    setPlanError(null);
    invoke<SimBriefOfp | null>("fetch_simbrief_preview", { ofpId })
      .then((result) => {
        setPlan(result);
        if (!result) {
          setPlanError(t("bids.simbrief_unavailable"));
        }
      })
      .catch((err: UiError) => {
        setPlanError(err.message ?? "Fehler");
      })
      .finally(() => setPlanLoading(false));
  }, [flight.simbrief?.id, t]);

  // Pax + Cargo aus den Fares zusammenrechnen.
  const fares = flight.simbrief?.subfleet?.fares ?? [];
  const paxCount = fares
    .filter((f) => (f.type ?? 0) === 0)
    .reduce((sum, f) => sum + (f.count ?? 0), 0);
  const cargoKg = fares
    .filter((f) => (f.type ?? 0) === 1)
    .reduce((sum, f) => sum + (f.count ?? 0), 0);

  const aircraftType = flight.simbrief?.subfleet?.type_;
  const aircraftName = flight.simbrief?.subfleet?.name;

  return (
    <div className="bid-card__details">
      {/* Aircraft-Info (wenn SimBrief-Subfleet bekannt) */}
      {(aircraftType || aircraftName) && (
        <div className="bid-card__aircraft">
          <span className="bid-card__detail-label">{t("bids.aircraft")}:</span>{" "}
          {aircraftType && <code>{aircraftType}</code>}
          {aircraftName && <span> · {aircraftName}</span>}
        </div>
      )}
      {/* Load-Chips: Pax + Cargo */}
      {(paxCount > 0 || cargoKg > 0) && (
        <div className="bid-card__load">
          {paxCount > 0 && (
            <span className="bid-card__load-chip bid-card__load-chip--pax">
              {paxCount} PAX
            </span>
          )}
          {cargoKg > 0 && (
            <span className="bid-card__load-chip bid-card__load-chip--cargo">
              {(cargoKg / 1000).toFixed(1)} t cargo
            </span>
          )}
        </div>
      )}
      {/* SimBrief Plan-Vorschau: Block / Burn / Reserve / TOW / LDW / ZFW + Alternate */}
      {flight.simbrief?.id && (
        <div className="bid-card__simbrief">
          <div className="bid-card__detail-label">
            {t("bids.simbrief_plan")}
            {planLoading && " …"}
          </div>
          {plan && (
            <div className="bid-card__simbrief-grid">
              <PlanRow label="Block" kg={plan.planned_block_fuel_kg} />
              <PlanRow label="Trip" kg={plan.planned_burn_kg} />
              <PlanRow label="Reserve" kg={plan.planned_reserve_kg} />
              <PlanRow label="TOW" kg={plan.planned_tow_kg} />
              <PlanRow label="LDW" kg={plan.planned_ldw_kg} />
              <PlanRow label="ZFW" kg={plan.planned_zfw_kg} />
              {plan.alternate && (
                <div className="bid-card__simbrief-row">
                  <span className="bid-card__simbrief-label">Alt</span>
                  <code>{plan.alternate}</code>
                </div>
              )}
            </div>
          )}
          {planError && !plan && (
            <div className="bid-card__simbrief-error">{planError}</div>
          )}
        </div>
      )}
      {/* Route-String aus phpVMS-Bid */}
      {flight.route && (
        <div className="bid-card__route-text">
          <span className="bid-card__route-label">{t("bids.route")}:</span>{" "}
          <code>{flight.route}</code>
        </div>
      )}
    </div>
  );
}

function PlanRow({ label, kg }: { label: string; kg: number }) {
  if (kg <= 0) return null;
  return (
    <div className="bid-card__simbrief-row">
      <span className="bid-card__simbrief-label">{label}</span>
      <span className="bid-card__simbrief-value">
        {Math.round(kg).toLocaleString("de-DE")} kg
      </span>
    </div>
  );
}
