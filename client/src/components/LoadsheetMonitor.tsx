import { useTranslation } from "react-i18next";
import type { ActiveFlightInfo } from "../types";

interface Props {
  info: ActiveFlightInfo;
}

/**
 * Live-Loadsheet (v0.3.0).
 *
 * Wird im Cockpit-Tab während der Boarding-/Preflight-Phase angezeigt
 * und zeigt eine kompakte IST/SOLL/Δ-Tabelle für Block-Fuel, ZFW und
 * TOW. Updates live mit jedem Sim-Snapshot über den activeFlight-Poll.
 *
 * Verschwindet ab Phase = TaxiOut/Pushback — ab dann zählt nur noch
 * der Cruise/Approach/Landing-Pfad, das Loadsheet ist "abgeschlossen".
 *
 * Werte:
 * - **Block-Fuel:** sim_fuel_kg vs. planned_block_fuel_kg
 * - **ZFW:** sim_zfw_kg vs. planned_zfw_kg
 * - **TOW:** sim_tow_kg vs. planned_tow_kg
 *
 * Plus Overweight-Detection: wenn IST > MAX-Wert (planned_max_*_kg),
 * fires eine rote "OVERWEIGHT"-Warnung. Nicht blockierend, nur Info.
 */
export function LoadsheetMonitor({ info }: Props) {
  const { t } = useTranslation();

  // Sichtbar nur in den Boarding-Phasen — sobald TaxiOut beginnt, ist
  // das Loadsheet "abgeschlossen" und wir blenden's aus.
  const visible =
    info.phase === "preflight" || info.phase === "boarding";
  if (!visible) return null;

  // Wenn weder ein Plan noch Live-Werte da sind, ist die Komponente
  // wertlos — gar nicht rendern.
  const hasAnyPlan =
    info.planned_block_fuel_kg != null ||
    info.planned_zfw_kg != null ||
    info.planned_tow_kg != null;
  const hasAnyLive =
    info.sim_fuel_kg != null ||
    info.sim_zfw_kg != null ||
    info.sim_tow_kg != null;
  if (!hasAnyPlan && !hasAnyLive) return null;

  const rows: LoadsheetRow[] = [
    {
      label: t("cockpit.loadsheet.block"),
      ist: info.sim_fuel_kg,
      soll: info.planned_block_fuel_kg,
      max: null, // kein MAX-Block-Fuel-Konzept (das wäre Tank-Kapazität)
    },
    {
      label: "ZFW",
      ist: info.sim_zfw_kg,
      soll: info.planned_zfw_kg,
      max: info.planned_max_zfw_kg,
    },
    {
      label: "TOW",
      ist: info.sim_tow_kg,
      soll: info.planned_tow_kg,
      max: info.planned_max_tow_kg,
    },
  ];

  // Status-Hint unten — was läuft gerade?
  const fuelDelta =
    info.sim_fuel_kg != null && info.planned_block_fuel_kg != null
      ? info.sim_fuel_kg - info.planned_block_fuel_kg
      : null;
  const zfwDelta =
    info.sim_zfw_kg != null && info.planned_zfw_kg != null
      ? info.sim_zfw_kg - info.planned_zfw_kg
      : null;
  const hint = computeHint(fuelDelta, zfwDelta, t);

  return (
    <section className="loadsheet">
      <div className="loadsheet__header">
        <span className="loadsheet__title">📋 {t("cockpit.loadsheet.title")}</span>
        <span className="loadsheet__phase">
          {info.phase === "preflight"
            ? t("cockpit.loadsheet.preflight")
            : t("cockpit.loadsheet.boarding")}
        </span>
      </div>
      <table className="loadsheet__table">
        <thead>
          <tr>
            <th />
            <th>{t("cockpit.loadsheet.ist")}</th>
            <th>{t("cockpit.loadsheet.soll")}</th>
            <th>Δ</th>
            <th>MAX</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => (
            <LoadsheetRow key={r.label} row={r} />
          ))}
        </tbody>
      </table>
      {hint && <div className="loadsheet__hint">{hint}</div>}
    </section>
  );
}

interface LoadsheetRow {
  label: string;
  ist: number | null;
  soll: number | null;
  max: number | null;
}

function LoadsheetRow({ row }: { row: LoadsheetRow }) {
  const delta =
    row.ist != null && row.soll != null ? row.ist - row.soll : null;
  const deltaPct =
    delta != null && row.soll != null && row.soll !== 0
      ? Math.abs(delta / row.soll) * 100
      : null;

  // Δ-Farbcode (gleich wie Landung-Tab): <5% grün, 5-10% gelb, >10% rot
  let deltaClass = "";
  if (deltaPct != null) {
    if (deltaPct < 5) deltaClass = "loadsheet__delta--ok";
    else if (deltaPct < 10) deltaClass = "loadsheet__delta--warn";
    else deltaClass = "loadsheet__delta--alert";
  }

  // Overweight-Detection: wenn IST > MAX, fires "OVERWEIGHT"-Stil.
  const overweight = row.ist != null && row.max != null && row.ist > row.max;

  return (
    <tr className={overweight ? "loadsheet__row--overweight" : ""}>
      <td>{row.label}</td>
      <td>{row.ist != null ? `${Math.round(row.ist).toLocaleString("de-DE")} kg` : "—"}</td>
      <td>{row.soll != null ? `${Math.round(row.soll).toLocaleString("de-DE")} kg` : "—"}</td>
      <td className={deltaClass}>
        {delta != null
          ? `${delta >= 0 ? "+" : ""}${Math.round(delta).toLocaleString("de-DE")} kg`
          : "—"}
      </td>
      <td>
        {row.max != null
          ? overweight
            ? `⚠ ${Math.round(row.max).toLocaleString("de-DE")} kg`
            : `${Math.round(row.max).toLocaleString("de-DE")} kg`
          : "—"}
      </td>
    </tr>
  );
}

/**
 * Status-Hint unten unter der Tabelle. Versucht den aktuellen
 * Boarding-Sub-State zu erkennen:
 * - Tank-Vorgang läuft (Block-Fuel steigt, weit unter Plan)
 * - Boarding läuft (ZFW steigt, unter Plan)
 * - Bereit für Pushback (alle Werte nahe Plan)
 *
 * Bewusst sehr lockere Toleranzen — soll dem Piloten ein Gefühl geben,
 * keine harte Logik. Wenn keine Plan-Werte da sind, kein Hint.
 */
function computeHint(
  fuelDelta: number | null,
  zfwDelta: number | null,
  // Looser type so we don't fight i18next's overload soup. The runtime
  // shape is fine — we only ever pass simple string-keyed records.
  t: (k: string, opts?: Record<string, unknown>) => string,
): string | null {
  if (fuelDelta == null && zfwDelta == null) return null;

  // "Fast bereit" = Block + ZFW jeweils innerhalb 200 kg vom Plan.
  const fuelOk = fuelDelta == null || Math.abs(fuelDelta) < 200;
  const zfwOk = zfwDelta == null || Math.abs(zfwDelta) < 200;
  if (fuelOk && zfwOk) {
    return t("cockpit.loadsheet.hint_ready");
  }

  // Negativ + groß = Tankvorgang oder Boarding läuft noch.
  if (fuelDelta != null && fuelDelta < -300) {
    return t("cockpit.loadsheet.hint_fueling", {
      missing: Math.abs(Math.round(fuelDelta)).toLocaleString("de-DE"),
    });
  }
  if (zfwDelta != null && zfwDelta < -300) {
    return t("cockpit.loadsheet.hint_boarding", {
      missing: Math.abs(Math.round(zfwDelta)).toLocaleString("de-DE"),
    });
  }

  // Positiv + groß = überladen / mehr Sprit als Plan.
  if (fuelDelta != null && fuelDelta > 500) {
    return t("cockpit.loadsheet.hint_overfueled", {
      extra: Math.round(fuelDelta).toLocaleString("de-DE"),
    });
  }

  return null;
}
