// v0.9.0 (#Discord-RPC) — Hook der bei jeder Aenderung von ActiveFlightInfo /
// SimSnapshot eine neue Presence ans Rust-Backend schiebt.
//
// Spec: docs/spec/v0.9.0-discord-rich-presence.md (Trigger-Tabelle)
//
// Design:
//   - Wenn kein Flug aktiv → discord_rpc_clear_flight()
//   - Wenn Flug aktiv → discord_rpc_push_state() mit aktueller Phase + Alt
//   - Backend dedupliziert intern (set_flight prueft ob input geaendert)
//   - Wenn der Discord-Toggle aus ist, no-ops das Backend ohnehin, also
//     ist's billig diesen Hook auch bei "RPC off" laufen zu lassen.

import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { ActiveFlightInfo, SimStatus } from "../types";

interface Args {
  activeFlight: ActiveFlightInfo | null;
  simStatus: SimStatus | null;
  /** Optional: phpVMS-Profil-URL fuer den "Open Profile"-Button. */
  profileUrl?: string | null;
}

export function useDiscordRpcPush({ activeFlight, simStatus, profileUrl }: Args) {
  // Letzter gepushter Snapshot — verhindert Spam bei jedem Render.
  // Wir vergleichen serialisierte JSON-Strings (klein, billig).
  const lastPushedRef = useRef<string>("");

  useEffect(() => {
    if (!activeFlight) {
      // Sauberes Clear nur einmal (wenn vorher was anderes drin war).
      if (lastPushedRef.current !== "CLEARED") {
        lastPushedRef.current = "CLEARED";
        void invoke("discord_rpc_clear_flight").catch(() => undefined);
      }
      return;
    }

    const callsign = `${activeFlight.airline_icao}${activeFlight.flight_number}`;
    const altitudeFt = simStatus?.snapshot?.altitude_msl_ft
      ? Math.round(simStatus.snapshot.altitude_msl_ft)
      : null;
    // started_at ISO → unix seconds
    const startUnix = Math.floor(new Date(activeFlight.started_at).getTime() / 1000);

    // Aircraft-ICAO: ActiveFlightInfo hat nur planned_registration (=Tail).
    // Phase-Tag-Allowlist erlaubt aircraft, also nehmen wir den Tail wenn nichts
    // anderes da ist — Discord-Anzeige ist "Aircraft" Fallback (siehe Spec).
    const aircraft = activeFlight.planned_registration || "";

    const payload = {
      callsign,
      dep_icao: activeFlight.dpt_airport,
      arr_icao: activeFlight.arr_airport,
      aircraft,
      altitude_ft: altitudeFt,
      phase: activeFlight.phase.toUpperCase(), // Tauri-Command parsed kanonisch
      sim: simStatus?.kind ?? "",
      start_unix: startUnix,
      profile_url: profileUrl ?? null,
    };

    const key = JSON.stringify(payload);
    if (key === lastPushedRef.current) return; // nichts geaendert

    lastPushedRef.current = key;
    void invoke("discord_rpc_push_state", { args: payload }).catch(() => undefined);
  }, [activeFlight, simStatus, profileUrl]);
}
