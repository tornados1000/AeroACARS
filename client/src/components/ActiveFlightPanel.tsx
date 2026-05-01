import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import type { ActiveFlightInfo } from "../types";
import { ManualFileDialog } from "./ManualFileDialog";
import { WeatherBriefing } from "./WeatherBriefing";

interface Props {
  /** Active-flight info, owned by Dashboard. Pure display. */
  info: ActiveFlightInfo | null;
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

export function ActiveFlightPanel({ info, onEnded }: Props) {
  const { t, i18n } = useTranslation();
  const [busy, setBusy] = useState<"end" | "cancel" | "forget" | null>(null);
  const [error, setError] = useState<string | null>(null);
  /**
   * When `flight_end` fails with `flight_validation_failed`, the backend
   * sends back a list of i18n-keyed missing-field codes. We surface the
   * ManualFileDialog so the pilot can either cancel the flight or file it
   * as a manual PIREP (with optional divert + reason). Null = no dialog.
   */
  const [validationMissing, setValidationMissing] = useState<string[] | null>(
    null,
  );
  // Tick once a second so the elapsed-time display refreshes between polls.
  const [, setTick] = useState(0);
  useEffect(() => {
    const id = setInterval(() => setTick((t) => t + 1), 1000);
    return () => clearInterval(id);
  }, []);

  if (!info) return null;

  async function handleEnd() {
    if (busy) return;
    setBusy("end");
    setError(null);
    try {
      await invoke("flight_end");
      onEnded?.();
    } catch (err: unknown) {
      // Backend's UiError shape: { code, message, details? }. The validation
      // path puts `{ missing: ["distance", ...] }` into details so we can
      // render the dialog with the exact reasons the file was rejected.
      const e = err as {
        code?: string;
        message?: string;
        details?: { missing?: string[] };
      };
      if (e?.code === "flight_validation_failed") {
        setValidationMissing(e.details?.missing ?? []);
      } else {
        const msg =
          typeof err === "object" && err !== null && "message" in err
            ? String((err as { message: string }).message)
            : String(err);
        setError(msg);
      }
    } finally {
      setBusy(null);
    }
  }

  /** User accepted the cancel option from the validation dialog. */
  async function handleCancelFromDialog() {
    setValidationMissing(null);
    setBusy("cancel");
    setError(null);
    try {
      await invoke("flight_cancel");
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
    if (!confirm(t("active_flight.confirm_cancel"))) return;
    setBusy("cancel");
    setError(null);
    try {
      await invoke("flight_cancel");
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

  /**
   * Force-discard local active-flight state without touching phpVMS. Useful
   * when the cancel call fails because the PIREP is already gone server-side
   * but our local state still thinks a flight is active.
   */
  async function handleForget() {
    if (busy) return;
    if (!confirm(t("active_flight.confirm_forget"))) return;
    setBusy("forget");
    setError(null);
    try {
      await invoke("flight_forget");
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

  return (
    <section className="active-flight">
      <header className="active-flight__header">
        <div className="active-flight__title-block">
          <span className="active-flight__label">
            {t("active_flight.title")}
          </span>
          <div className="active-flight__heading">
            <h2 className="active-flight__callsign">
              {info.airline_icao
                ? `${info.airline_icao} ${info.flight_number}`
                : info.flight_number}
            </h2>
            <span
              className={`active-flight__phase active-flight__phase--${info.phase}`}
            >
              {t(`active_flight.phase.${info.phase}`, {
                defaultValue: info.phase,
              })}
            </span>
          </div>
        </div>
        <div className="active-flight__route">
          <span className="active-flight__icao">{info.dpt_airport}</span>
          <span className="active-flight__route-arrow">
            <span className="active-flight__arrow">→</span>
            <span className="active-flight__route-distance">
              {fmtDistance(info.distance_nm, i18n.language)}
            </span>
          </span>
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
          <button
            type="button"
            className="active-flight__forget"
            onClick={handleForget}
            disabled={busy !== null}
            title={t("active_flight.forget_hint")}
          >
            {busy === "forget"
              ? t("active_flight.forgetting")
              : t("active_flight.forget")}
          </button>
        </div>
      </header>

      <dl className="active-flight__stats">
        <div className="active-flight__stat">
          <dt>{t("active_flight.elapsed")}</dt>
          <dd>{fmtDuration(info.started_at, i18n.language)}</dd>
        </div>
        <div className="active-flight__stat">
          <dt>{t("active_flight.distance")}</dt>
          <dd>{fmtDistance(info.distance_nm, i18n.language)}</dd>
        </div>
        <div className="active-flight__stat">
          <dt>{t("active_flight.positions")}</dt>
          <dd>{info.position_count}</dd>
        </div>
      </dl>

      <WeatherBriefing dptIcao={info.dpt_airport} arrIcao={info.arr_airport} />

      {error && (
        <p className="active-flight__error" role="alert">
          {error}
        </p>
      )}

      {validationMissing !== null && (
        <ManualFileDialog
          plannedArrival={info.arr_airport}
          missing={validationMissing}
          onFiled={() => {
            setValidationMissing(null);
            onEnded?.();
          }}
          onCancelFlight={() => void handleCancelFromDialog()}
          onClose={() => setValidationMissing(null)}
        />
      )}
    </section>
  );
}
