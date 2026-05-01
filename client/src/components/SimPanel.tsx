import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import type { SimSnapshot, SimStatus } from "../types";

const POLL_MS = 500;

interface Props {
  /** Lift the connection state so other parts of the dashboard can react. */
  onStateChange?: (state: SimStatus["state"]) => void;
  /** Lift the latest snapshot so siblings (e.g. BidsList) can use lat/lon. */
  onSnapshotChange?: (snapshot: SimSnapshot | null) => void;
  /** When true, render the full telemetry grid below the compact summary. */
  debugMode: boolean;
}

function fmtNumber(value: number, fractionDigits: number, locale: string): string {
  return new Intl.NumberFormat(locale, {
    minimumFractionDigits: fractionDigits,
    maximumFractionDigits: fractionDigits,
  }).format(value);
}

function fmtCoord(deg: number): string {
  const sign = deg >= 0 ? "+" : "−";
  return `${sign}${Math.abs(deg).toFixed(6)}°`;
}

function fmtFt(ft: number, locale: string): string {
  return `${new Intl.NumberFormat(locale).format(Math.round(ft))} ft`;
}

function fmtKt(kt: number, locale: string): string {
  return `${new Intl.NumberFormat(locale).format(Math.round(kt))} kt`;
}

function fmtFpm(fpm: number, locale: string): string {
  const rounded = Math.round(fpm);
  const sign = rounded > 0 ? "+" : "";
  return `${sign}${new Intl.NumberFormat(locale).format(rounded)} fpm`;
}

function fmtHeading(deg: number): string {
  const norm = ((deg % 360) + 360) % 360;
  return `${Math.round(norm).toString().padStart(3, "0")}°`;
}

/**
 * MSFS often returns localization keys instead of plain text for ATC MODEL,
 * e.g. "TT:ATCCOM.AC_MODEL_A320.0.text" or "ATCCOM.AC_MODEL A320.0.text".
 * Try to pull the model code out; if we can't, return null so the field is
 * hidden rather than showing a useless string to the user.
 */
function cleanAtcModel(raw: string | null | undefined): string | null {
  if (!raw) return null;
  const s = raw.trim();
  if (!s) return null;
  const match = s.match(/AC_MODEL[_ ]([^.\s]+)\.\d+\.text$/i);
  if (match) return match[1];
  // Looks like an unresolved localization key — hide it.
  if (s.toUpperCase().startsWith("TT:")) return null;
  if (s.endsWith(".text") || s.includes("ATCCOM.")) return null;
  return s;
}

export function SimPanel({
  onStateChange,
  onSnapshotChange,
  debugMode,
}: Props) {
  const { t, i18n } = useTranslation();
  const [status, setStatus] = useState<SimStatus | null>(null);

  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setInterval> | null = null;

    async function poll() {
      try {
        const next = await invoke<SimStatus>("sim_status");
        if (cancelled) return;
        setStatus((prev) => {
          if (prev?.state !== next.state) onStateChange?.(next.state);
          return next;
        });
        onSnapshotChange?.(next.snapshot);
      } catch {
        if (!cancelled) {
          setStatus({
            state: "disconnected",
            kind: "off",
            snapshot: null,
            last_error: "ipc",
            available: false,
          });
          onSnapshotChange?.(null);
        }
      }
    }

    void poll();
    timer = setInterval(poll, POLL_MS);
    return () => {
      cancelled = true;
      if (timer) clearInterval(timer);
    };
  }, [onStateChange, onSnapshotChange]);

  if (!status) {
    return (
      <section className="sim-panel">
        <header className="sim-panel__header">
          <h2>{t("sim.title")}</h2>
        </header>
        <p className="sim-panel__hint">{t("sim.loading")}</p>
      </section>
    );
  }

  const { state, kind, snapshot, last_error } = status;
  const stateLabel = t(`sim.state.${state}`);
  const isOff = kind === "off";
  const isXplane = kind === "xplane11" || kind === "xplane12";
  const kindLabel = t(`sim.kinds.${kind}`);

  return (
    <section className={`sim-panel sim-panel--${state}`}>
      <header className="sim-panel__header">
        <div className="sim-panel__header-left">
          <h2>{t("sim.title")}</h2>
          <span className="sim-panel__kind">{kindLabel}</span>
        </div>
        <span
          className={`sim-panel__state sim-panel__state--${state}`}
          aria-live="polite"
        >
          <span className="sim-panel__dot" /> {stateLabel}
        </span>
      </header>

      {isOff && <p className="sim-panel__hint">{t("sim.off_hint")}</p>}

      {isXplane && (
        <p className="sim-panel__hint">{t("sim.xplane_phase2")}</p>
      )}

      {!isOff && !isXplane && state !== "connected" && (
        <p className="sim-panel__hint">
          {state === "connecting"
            ? t("sim.connecting_hint")
            : t("sim.idle_hint")}
        </p>
      )}

      {last_error && state !== "connected" && !isOff && (
        <p className="sim-panel__error" role="alert">
          {t("sim.last_error_prefix")}: <code>{last_error}</code>
        </p>
      )}

      {snapshot && state === "connected" && (
        <CompactSummary snap={snapshot} locale={i18n.language} />
      )}

      {snapshot && debugMode && (
        <details className="sim-panel__debug" open>
          <summary>{t("sim.debug_section")}</summary>
          <SnapshotGrid snap={snapshot} locale={i18n.language} />
        </details>
      )}
    </section>
  );
}

function CompactSummary({
  snap,
  locale,
}: {
  snap: SimSnapshot;
  locale: string;
}) {
  const { t } = useTranslation();
  const aircraft = snap.aircraft_title?.trim() || t("sim.aircraft_unknown");
  const registration = snap.aircraft_registration?.trim();
  const icaoModel = cleanAtcModel(snap.aircraft_icao);
  const extras = [icaoModel, registration].filter(Boolean);
  return (
    <dl className="sim-panel__compact">
      <Row label={t("sim.fields.aircraft")}>
        <span>{aircraft}</span>
        {extras.length > 0 && (
          <span className="sim-panel__compact-muted">
            {" — "}
            {extras.join(" · ")}
          </span>
        )}
      </Row>
      <Row label={t("sim.fields.position")}>
        {fmtCoord(snap.lat)} · {fmtCoord(snap.lon)}
      </Row>
      <Row label={t("sim.fields.altitude_msl")}>
        {fmtFt(snap.altitude_msl_ft, locale)}
        <span className="sim-panel__compact-muted">
          {" "}
          ({t("sim.fields.altitude_agl")} {fmtFt(snap.altitude_agl_ft, locale)})
        </span>
      </Row>
      <Row label={t("sim.fields.on_ground")}>
        {snap.on_ground ? t("sim.yes") : t("sim.no")}
      </Row>
    </dl>
  );
}

function SnapshotGrid({ snap, locale }: { snap: SimSnapshot; locale: string }) {
  const { t } = useTranslation();
  return (
    <dl className="sim-panel__grid">
      <Row label={t("sim.fields.position")}>
        {fmtCoord(snap.lat)} · {fmtCoord(snap.lon)}
      </Row>
      <Row label={t("sim.fields.altitude_msl")}>
        {fmtFt(snap.altitude_msl_ft, locale)}
      </Row>
      <Row label={t("sim.fields.altitude_agl")}>
        {fmtFt(snap.altitude_agl_ft, locale)}
      </Row>
      <Row label={t("sim.fields.heading")}>
        {fmtHeading(snap.heading_deg_magnetic)} ({t("sim.fields.heading_mag")}) ·{" "}
        {fmtHeading(snap.heading_deg_true)} ({t("sim.fields.heading_true")})
      </Row>
      <Row label={t("sim.fields.groundspeed")}>
        {fmtKt(snap.groundspeed_kt, locale)}
      </Row>
      <Row label={t("sim.fields.airspeed")}>
        IAS {fmtKt(snap.indicated_airspeed_kt, locale)} · TAS{" "}
        {fmtKt(snap.true_airspeed_kt, locale)}
      </Row>
      <Row label={t("sim.fields.vertical_speed")}>
        {fmtFpm(snap.vertical_speed_fpm, locale)}
      </Row>
      <Row label={t("sim.fields.attitude")}>
        {t("sim.fields.pitch")} {fmtNumber(snap.pitch_deg, 1, locale)}° ·{" "}
        {t("sim.fields.bank")} {fmtNumber(snap.bank_deg, 1, locale)}°
      </Row>
      <Row label={t("sim.fields.g_force")}>
        {fmtNumber(snap.g_force, 2, locale)} G
      </Row>
      <Row label={t("sim.fields.on_ground")}>
        {snap.on_ground ? t("sim.yes") : t("sim.no")}
      </Row>
    </dl>
  );
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <>
      <dt>{label}</dt>
      <dd>{children}</dd>
    </>
  );
}
