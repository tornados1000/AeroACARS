import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import type { ActiveFlightInfo } from "../types";

const POLL_MS = 2000;

interface Props {
  /** Notify parent when the flight ends so it can refresh bids etc. */
  onEnded?: () => void;
}

function fmtDuration(startedIso: string, locale: string): string {
  const started = new Date(startedIso).getTime();
  const ms = Date.now() - started;
  const minutes = Math.max(0, Math.floor(ms / 60000));
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  if (h === 0) return `${m}m`;
  return locale.startsWith("de")
    ? `${h}h ${m.toString().padStart(2, "0")}m`
    : `${h}h ${m}m`;
}

function fmtDistance(nm: number, locale: string): string {
  return `${new Intl.NumberFormat(locale, { maximumFractionDigits: 1 }).format(
    nm,
  )} nmi`;
}

export function ActiveFlightPanel({ onEnded }: Props) {
  const { t, i18n } = useTranslation();
  const [info, setInfo] = useState<ActiveFlightInfo | null>(null);
  const [busy, setBusy] = useState<"end" | "cancel" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [tick, setTick] = useState(0);

  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setInterval> | null = null;

    async function poll() {
      try {
        const next = await invoke<ActiveFlightInfo | null>("flight_status");
        if (cancelled) return;
        setInfo(next);
      } catch {
        // ignore — IPC errors are transient on dev rebuilds
      }
    }

    void poll();
    timer = setInterval(() => {
      void poll();
      setTick((t) => t + 1); // also refreshes the elapsed-time display
    }, POLL_MS);
    return () => {
      cancelled = true;
      if (timer) clearInterval(timer);
    };
  }, []);

  if (!info) return null;

  async function handleEnd() {
    if (busy) return;
    setBusy("end");
    setError(null);
    try {
      await invoke("flight_end");
      setInfo(null);
      onEnded?.();
    } catch (err: unknown) {
      const msg =
        typeof err === "object" && err !== null && "message" in err
          ? String((err as { message: string }).message)
          : String(err);
      setError(msg);
    } finally {
      setBusy(null);
    }
  }

  async function handleCancel() {
    if (busy) return;
    setBusy("cancel");
    setError(null);
    try {
      await invoke("flight_cancel");
      setInfo(null);
      onEnded?.();
    } catch (err: unknown) {
      const msg =
        typeof err === "object" && err !== null && "message" in err
          ? String((err as { message: string }).message)
          : String(err);
      setError(msg);
    } finally {
      setBusy(null);
    }
  }

  // tick is intentionally referenced so React re-renders elapsed time.
  void tick;

  return (
    <section className="active-flight">
      <header className="active-flight__header">
        <div>
          <span className="active-flight__label">
            {t("active_flight.title")}
          </span>
          <h2 className="active-flight__callsign">{info.flight_number}</h2>
        </div>
        <div className="active-flight__route">
          <span className="active-flight__icao">{info.dpt_airport}</span>
          <span className="active-flight__arrow">→</span>
          <span className="active-flight__icao">{info.arr_airport}</span>
        </div>
        <div className="active-flight__actions">
          <button
            type="button"
            className="button button--primary"
            onClick={handleEnd}
            disabled={busy !== null}
          >
            {busy === "end" ? t("active_flight.filing") : t("active_flight.end")}
          </button>
          <button type="button" onClick={handleCancel} disabled={busy !== null}>
            {busy === "cancel"
              ? t("active_flight.cancelling")
              : t("active_flight.cancel")}
          </button>
        </div>
      </header>

      <dl className="active-flight__stats">
        <div>
          <dt>{t("active_flight.elapsed")}</dt>
          <dd>{fmtDuration(info.started_at, i18n.language)}</dd>
        </div>
        <div>
          <dt>{t("active_flight.distance")}</dt>
          <dd>{fmtDistance(info.distance_nm, i18n.language)}</dd>
        </div>
        <div>
          <dt>{t("active_flight.positions")}</dt>
          <dd>{info.position_count}</dd>
        </div>
      </dl>

      {error && (
        <p className="active-flight__error" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}
