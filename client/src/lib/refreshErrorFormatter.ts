/**
 * v0.7.8 v1.5.2: Shared Formatter fuer `flight_refresh_simbrief`-Fehler.
 *
 * Drei Call-Sites rufen heute `flight_refresh_simbrief`:
 *   1. BidsList "Aktualisieren"-Button (Bid-Tab)
 *   2. ActiveFlightPanel "OFP refreshen"-Button (Cockpit-Tab)
 *   3. LoadsheetMonitor inline-Refresh-Button (v0.5.46 Adrian-Fix)
 *
 * Ohne diesen gemeinsamen Helper wuerde Cockpit + Loadsheet bei einem
 * Mismatch-Fehler einfach `err.message` anzeigen — was bei v0.7.8
 * structured-JSON-Details (siehe `lib.rs` flight_refresh_simbrief
 * Mismatch-Handler) bedeuten wuerde dass der Pilot rohes JSON sieht
 * wie `{"active_callsigns":"CFG1504","active_dpt":"EDDF",...}`.
 *
 * Dieser Helper macht aus der JSON-Encoded UiError eine lesbare
 * lokalisierte Pilot-Notice. Spec docs/spec/ofp-refresh-simbrief-direct-
 * v0.7.8.md §8 + Thomas-QS v1.5.1.
 */

import type { TFunction } from "i18next";

/** Tauri-UiError-Shape — `code` + `message` Tupel. */
export interface TauriRefreshError {
  code?: string;
  message?: string;
}

export interface FormattedNotice {
  /** Lokalisierter Pilot-lesbarer Text. */
  text: string;
  /** Notice-Tonalitaet — Frontend rendert das passend (rot/gelb/blau). */
  tone: "info" | "warn" | "err";
}

/**
 * Formatiert einen `flight_refresh_simbrief`-Error in eine lokalisierte
 * Pilot-Notice. Bekannte error.codes werden ueber i18n-Templates mit
 * Parametern gerendert; unbekannte Codes fallen auf den `err.message`
 * (= raw) zurueck.
 *
 * **Wichtig:** Diese Funktion ist die EINZIGE Stelle die JSON-Details
 * aus `err.message` parsed. Wenn das JSON-Encoding-Format im Backend
 * mal aendert, ist hier der zentrale Anpassungspunkt — nicht ueber
 * 3 Komponenten verteilt.
 */
export function formatRefreshError(
  err: TauriRefreshError | null | undefined,
  t: TFunction,
): FormattedNotice | null {
  if (!err) return null;

  // ─── ofp_does_not_match_active_flight ──────────────────────────────
  // Backend kodiert active_callsigns/dpt/arr + sb_callsign/origin/dest
  // als JSON im message-Feld.
  if (err.code === "ofp_does_not_match_active_flight") {
    // v1.5.2: active_callsigns ist eine "/" -getrennte Liste aller
    // validen Kandidaten (z.B. "CFG1504 / CFG4TK / 4TK"), gebaut vom
    // Backend via build_candidate_callsigns. Pilot sieht damit ALLE
    // valid-Formen statt nur airline+flight_number.
    const defaults: Record<string, string> = {
      active_callsigns: "—",
      active_dpt: "—",
      active_arr: "—",
      sb_callsign: "—",
      sb_origin: "—",
      sb_dest: "—",
    };
    const params: Record<string, string> = { ...defaults };
    if (err.message) {
      try {
        const parsed = JSON.parse(err.message) as Record<string, unknown>;
        for (const k of Object.keys(defaults)) {
          const v = parsed[k];
          if (typeof v === "string" && v.length > 0) params[k] = v;
        }
      } catch {
        // JSON-Parse-Fehler → Defaults ("—") bleiben. Pilot sieht
        // unspezifischen Notice, kein Crash, kein rohes JSON.
      }
    }
    return {
      text: t("bids.ofp_does_not_match_active_flight", params),
      tone: "warn",
    };
  }

  // ─── Bekannte SimBrief-direct-Error-Codes ──────────────────────────
  // Alle haben i18n-Keys ohne Parameter (= einfache Strings).
  const KNOWN_CODES: Record<string, { key: string; tone: FormattedNotice["tone"] }> = {
    simbrief_user_not_found: { key: "bids.simbrief_user_not_found", tone: "warn" },
    simbrief_unavailable_and_bid_gone: {
      key: "bids.simbrief_unavailable_and_bid_gone",
      tone: "warn",
    },
    simbrief_direct_failed: { key: "bids.simbrief_direct_failed", tone: "warn" },
    bid_not_found: { key: "bids.ofp_bid_gone", tone: "warn" },
  };
  if (err.code && KNOWN_CODES[err.code]) {
    const { key, tone } = KNOWN_CODES[err.code];
    return { text: t(key), tone };
  }

  // ─── phase_locked + no_simbrief_link → silent ──────────────────────
  // BidsList-Refresh in einer spaeteren Phase soll keinen Notice
  // produzieren (die Bid-Liste wird trotzdem aktualisiert). Cockpit-/
  // Loadsheet-Button waere in diesen Phasen auch gar nicht sichtbar.
  if (err.code === "phase_locked" || err.code === "no_simbrief_link") {
    return null;
  }

  // ─── Unbekannte Codes: rohe message als err-Tone ───────────────────
  // Letzter Fallback — sollte nur fuer interne Bugs / unerwartete
  // Backend-Errors greifen.
  if (err.message && err.message.length > 0) {
    return { text: err.message, tone: "err" };
  }
  return null;
}
