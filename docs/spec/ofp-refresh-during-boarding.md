# OFP-Refresh waehrend Boarding — Stand-Aufnahme + Spec

**Status:** Draft v1.0 for Thomas-Review
**Stand:** 2026-05-11
**Trigger:** Real-Pilot-Frust beim laufenden Flug (Tab "Meine Fluege" → "Aktualisieren" tut nicht was Pilot erwartet)

> **Problem in einem Satz:** Pilot regeneriert SimBrief-OFP waehrend Boarding, klickt "Aktualisieren" im Bid-Tab — die `planned_*`-Werte im aktiven Flug bleiben aber alt, weil dieser Refresh-Pfad den aktiven Flug nicht anpackt.

---

## 1. Datenfluss heute (verifiziert im Code v0.7.6)

```
SimBrief.com                phpVMS (PAX Studio)        AeroACARS Client
─────────────               ──────────────────         ──────────────────
Pilot regeneriert  ──┐                                          
OFP                  │                                          
                     │                                          
                     └─→ User klickt "Laden von SB"             
                         in PAX Studio                          
                              │                                 
                              ▼                                 
                         Bid.simbrief.id wird auf neue          
                         OFP-ID gesetzt  (phpVMS-Bid-DB)        
                                                                
                                          ┌──── /api/user/bids ──┘
                                          │                     
                                          ▼                     
                                   Bid-Liste mit neuer
                                   simbrief.id im Client cache
                                                                
                                          │                     
                                          ▼                     
SimBrief direkt   ←─────  GET  https://www.simbrief.com/
(public-by-ID)             ofp/flightplans/xml/{id}.xml
                                          │                     
                                          ▼                     
                                   SimBriefOfp parsed →
                                   planned_block_fuel_kg
                                   planned_burn_kg
                                   planned_zfw_kg
                                   planned_tow_kg
                                   planned_ldw_kg
                                   etc.
```

**Wichtig:** `simbrief.com/ofp/flightplans/xml/{id}.xml` ist die einzige Quelle fuer die `planned_*`-Werte im Client. phpVMS speichert NICHT die OFP-Werte selbst — phpVMS speichert nur die `simbrief.id` (= Pointer zum OFP auf SimBrief-Seite). Wenn `simbrief.id` neu ist, kommt der frische Plan; wenn alt, der alte.

---

## 2. Drei Refresh-Pfade im Client (Stand v0.7.6)

| Wo | Funktion | Was wird gemacht | Sichtbar wann |
|---|---|---|---|
| **Tab "Meine Fluege"** Header-Button "⟳ Aktualisieren" | `BidsList.handleRefresh` | `phpvms_get_bids` + `sim_force_resync` + `phpvms_refresh_profile` | immer |
| **Cockpit-Tab** "OFP refreshen"-Button (kleines Action-Row) | `ActiveFlightPanel.handleRefreshOfp` → `flight_refresh_simbrief` | re-fetch bids + neuer OFP vom Bid + UEBERSCHREIBT `planned_*` im aktiven Flug | nur `preflight\|boarding\|taxi_out` |
| **Loadsheet-Card** Inline-Refresh-Button (v0.5.46 Adrian-Fix) | `LoadsheetMonitor.handleRefreshOfp` → `flight_refresh_simbrief` | identisch zu (2) | nur `preflight\|boarding` UND wenn OFP-Outdated-Heuristik triggert (fuel-delta gross + zfw-delta klein) |

**Kern-Erkenntnis:** Nur (2) und (3) aktualisieren wirklich die `planned_*`-Werte im aktiven Flug. (1) — der prominente Button im Bid-Tab — tut das NICHT.

---

## 3. Real-Pilot-Workflow vs Tool-Reaktion

| Schritt | Pilot tut | AeroACARS-Reaktion | Erwartung |
|---|---|---|---|
| 1 | bookt Bid in phpVMS | — | — |
| 2 | regeneriert OFP auf simbrief.com | — | — |
| 3 | startet AeroACARS, klickt "Flug starten" | `flight_start` → `fetch_simbrief_ofp(sb.id)` → schreibt `planned_*` in FlightStats | ✓ |
| 4 | belaedt im Sim Pax/Cargo/Fuel | — | — |
| 5 | merkt: OFP-Werte passen nicht (Reserve falsch, Pax-Anzahl falsch, ...) | — | — |
| 6 | aendert auf simbrief.com → neuer OFP | — | — |
| 7 | klickt **PAX Studio "Laden von SB"** auf phpVMS-Site | phpVMS-Bid bekommt neue `simbrief.id` (server-side) | ✓ |
| 8 | klickt **AeroACARS "⟳ Aktualisieren"** im Bid-Tab | `phpvms_get_bids` zieht neue Bid-Liste (mit neuer `simbrief.id`), aber **`planned_*` im aktiven Flug bleiben alt** | ❌ Pilot erwartet aktualisierten OFP |
| 9 | sieht: Loadsheet-Werte sind weiter falsch | — | (frustrierter Pilot) |
| 10 | **wenn Glueck**: findet Cockpit-Refresh-Button oder Loadsheet-Inline-Refresh-Button | `flight_refresh_simbrief` zieht neue OFP → `planned_*` ueberschrieben | ✓ |

**Ergebnis:** Der prominente Button im Tab "Meine Fluege" (= dort wo der Pilot zuerst schaut) macht NICHT was er erwartet, und der wirksame Button ist in einem anderen Tab versteckt.

---

## 4. Code-Anchors (Stand v0.7.6)

| Datei | Zeile | Was |
|---|---|---|
| `client/src/components/BidsList.tsx` | 240-258 | `handleRefresh()` — der "falsche" Button |
| `client/src/components/ActiveFlightPanel.tsx` | 138-155 | `handleRefreshOfp()` — wirksam, aber versteckt |
| `client/src/components/ActiveFlightPanel.tsx` | 249-266 | Phase-Gate `preflight\|boarding\|taxi_out` |
| `client/src/components/LoadsheetMonitor.tsx` | 102-122 | Inline-Refresh + OFP-Outdated-Heuristik |
| `client/src/components/LoadsheetMonitor.tsx` | 76-93 | Heuristik fuel-delta >= 400 kg OR >= 5% AND zfw-delta < 200 kg |
| `client/src-tauri/src/lib.rs` | 4327-4427 | `flight_refresh_simbrief` Command — die wirksame Backend-Logik |
| `client/src-tauri/crates/api-client/src/lib.rs` | 1146-1177 | `fetch_simbrief_ofp` — fetcht `simbrief.com/ofp/flightplans/xml/{id}.xml` direkt |

---

## 5. Mutmassliche Wurzeln (priorisiert)

### W1 — UI-Discoverability (sicher die Haupt-Wurzel)
Pilot drueckt im Tab "Meine Fluege" auf "Aktualisieren" und erwartet "alles wird neu gezogen", inklusive aktivem Flug. Der Button macht aber nur Bid-Liste + Sim-Resync + Profile. Der Pilot-spezifische OFP-Refresh fuer den aktiven Flug ist nur via Cockpit-Tab oder Loadsheet-Inline-Hint erreichbar.

### W2 — Phase-Limit zu strikt? (zu pruefen)
`flight_refresh_simbrief` ist im Cockpit nur in `preflight | boarding | taxi_out` sichtbar (= bis kurz vor Takeoff). Das Limit ist sachlich begruendet — nach Takeoff sollte der Plan nicht mehr aendern — aber wenn der Pilot zwischen `taxi_out` und Pushback gerade nicht hinkommt: kein Refresh mehr moeglich. Im LoadsheetMonitor sogar nur bis `boarding` Ende.

### W3 — Cache-Layer? (unwahrscheinlich, aber zu pruefen)
Theoretisch koennte reqwest oder phpVMS einen HTTP-Cache haben. Praktisch:
- SimBrief antwortet auf jede ID frisch (URL-basiert, kein Cache-Header).
- phpVMS-Bid-Endpoint `/api/user/bids` koennte server-side cachen, aber das ist VA-spezifisch (paxstudio-Config).

### W4 — PAX Studio "Laden von SB" updated nicht die OFP-ID am Bid? (zu pruefen mit User)
Wenn das PAX-Studio-Modul nur Pax/Cargo aktualisiert aber NICHT die `simbrief.id` am Bid austauscht, dann holt auch `flight_refresh_simbrief` weiterhin den alten OFP. Diese Wurzel sitzt server-side, nicht im AeroACARS-Code.

**Quick-Check fuer User:** Nach "Laden von SB" einmal `https://german-sky-group.eu/api/user/bids` aufrufen (Browser, eingeloggt) und schauen ob `simbrief.id` wirklich die neue ist. Wenn ja → W1+W2 reichen. Wenn nein → W4 ist die Wurzel, PAX-Studio-Issue.

---

## 6. Soll-Verhalten (Spec)

### Was wir wollen
1. **"Aktualisieren" im Bid-Tab macht das was es heisst** — inkl. aktiver Flug-OFP, wenn vorhanden und in refreshbarer Phase.
2. **Discoverability:** Pilot soll nicht zwischen Tabs wechseln muessen.
3. **Klarer Feedback-Loop:** wenn der OFP-Refresh KEINE Aenderung gebracht hat (alte OFP-ID immer noch verlinkt), soll der Pilot das wissen — damit er erkennt dass die phpVMS-Seite das Problem ist.

### Was wir NICHT tun
- Kein Phase-Limit-Aufweichen (nach Takeoff bleibt OFP-Refresh gesperrt — das ist sachlich richtig)
- Kein neuer Score-Logik-Pfad
- Kein Pax-Studio-Reverse-Engineering (das ist ein anderes Repo)

---

## 7. Loesungs-Optionen

### Option A (klein, additiv): `BidsList.handleRefresh` ruft `flight_refresh_simbrief` mit auf

```ts
async function handleRefresh() {
  if (refreshing) return;
  setRefreshing(true);
  // hasActiveFlight ist schon im Component-State verfuegbar
  const tasks: Promise<unknown>[] = [
    fetchBids(),
    invoke("sim_force_resync").catch(() => null),
    invoke<Profile | null>("phpvms_refresh_profile").catch(() => null),
  ];
  if (hasActiveFlight) {
    // v0.7.7: auch den OFP des aktiven Flugs refreshen damit der
    // prominente "Aktualisieren"-Button im Bid-Tab das tut was der
    // Pilot erwartet. Phase-Gate liegt server-side
    // (flight_refresh_simbrief returnt Error wenn Phase falsch).
    tasks.push(
      invoke<SimBriefOfpDto>("flight_refresh_simbrief").catch((err) => {
        // Nicht fatal — z.B. Phase = Cruise → Backend lehnt ab.
        // Aktuell hat flight_refresh_simbrief allerdings KEIN explizites
        // Phase-Gate; das muesste fuer diese Option ergaenzt werden.
        return null;
      }),
    );
  }
  const [, , freshProfile] = await Promise.all(tasks);
  if (freshProfile && onProfileRefreshed) onProfileRefreshed(freshProfile);
  setTimeout(() => setRefreshing(false), 400);
}
```

**Vorteile:**
- Minimaler Eingriff, additive Aenderung
- User-Erwartung wird erfuellt
- Bestehende Cockpit/Loadsheet-Buttons bleiben unveraendert (mehrfache Wege)

**Nachteile:**
- Brauchen wir ein Phase-Gate auf `flight_refresh_simbrief` damit wir nach Takeoff nicht versehentlich den Plan ueberschreiben? Aktuell hat das Command kein Phase-Gate — der Cockpit-Button uebernimmt das frontseitig.

**Risiko:** Wenn Pilot im Cruise versehentlich "Aktualisieren" druckt, wuerde mit Option A der OFP neu geholt und ueberschrieben. Das ist seman tisch fragwuerdig (nach Takeoff soll Plan stehen).

**→ Loesung:** Phase-Gate in den Backend-Command einbauen (siehe Option A1).

### Option A1: Option A + Phase-Gate in `flight_refresh_simbrief`

```rust
async fn flight_refresh_simbrief(...) -> Result<SimBriefOfpDto, UiError> {
    let (bid_id, current_phase) = {
        let guard = state.active_flight.lock()?;
        let flight = guard.as_ref().ok_or(...)?;
        let stats = flight.stats.lock()?;
        (flight.bid_id, stats.phase)
    };
    // v0.7.7: nach Takeoff darf der Plan nicht mehr ueberschrieben werden
    if !matches!(current_phase,
        FlightPhase::Preflight | FlightPhase::Boarding | FlightPhase::TaxiOut)
    {
        return Err(UiError::new(
            "phase_locked",
            "OFP-Refresh ist nur bis vor Takeoff moeglich (Preflight/Boarding/Taxi-Out)",
        ));
    }
    // ... rest unveraendert
}
```

**Damit:** Bid-Tab-Refresh ruft das Command, kriegt `phase_locked`-Error in spaeteren Phasen, ignoriert das still. Im Cockpit-Tab kriegt der Pilot weiter den dedizierten Button (mit Phase-bedingter Sichtbarkeit).

### Option B (UX-Pille): Toast wenn OFP-Refresh ohne Wert-Aenderung blieb

Wenn `flight_refresh_simbrief` aufgerufen wird und die neue `simbrief.id` == alte `simbrief.id` (= phpVMS hatte schon nichts neues), zeige im Activity-Log + als kurzer Toast:

```
ℹ️ OFP unveraendert — phpVMS-Bid trug bereits diese OFP-ID. Pruefe ob
   PAX Studio "Laden von SB" wirklich gelaufen ist.
```

Damit weiss der Pilot dass das Problem server-side liegt.

### Option C (groesser, langfristig): Auto-Refresh-Heuristik im aktiven Flug

Beim 15s-Bid-Poll (heute pausiert bei aktivem Flug) parallel pruefen: ist die `simbrief.id` der aktuellen Bid != der `simbrief.id` zum Zeitpunkt `flight_start`? Wenn ja → still automatisch `flight_refresh_simbrief` aufrufen, danach Toast "OFP aktualisiert".

**Vorteile:** Pilot braucht gar nicht zu klicken.

**Nachteile:**
- Aenderung am Polling-Verhalten (heute pausiert bei aktivem Flug, mit Begruendung)
- Auto-Update kann Pilot ueberraschen wenn er es nicht erwartet
- Race-Conditions zwischen Sim-Loadsheet-Vergleich und Plan-Update

**Verschoben auf spaeter** — Option A1 + B reichen fuer das berichtete Symptom.

---

## 8. Empfehlung: v0.7.7 minimal-Schnitt

**P1 — Option A1:** Bid-Tab-Refresh ruft `flight_refresh_simbrief` mit, plus Phase-Gate im Backend-Command.

**P2 — Option B:** Toast wenn OFP-ID identisch blieb (= phpVMS/PAX-Studio-Hinweis).

**P3 — Doku:** im Activity-Log einen einmaligen Hinweis loggen wenn Pilot nach phpVMS-Bid-Aenderung den frischen OFP geholt hat — damit man im JSONL-Replay sieht "ja, der OFP wurde refresht zur Zeit X" als Audit-Trail.

**NICHT in v0.7.7:**
- Auto-Refresh-Polling (Option C) — separater Schnitt
- Phase-Limit-Aufweichen — das Limit ist sachlich richtig
- PAX-Studio-Reverse-Engineering — separates Repo

### Tests (Vorschlag)

- `flight_refresh_simbrief_returns_phase_locked_after_takeoff`
- `flight_refresh_simbrief_returns_same_ofp_id_when_phpvms_unchanged` (Toast-Trigger)
- Bid-Tab-Refresh-Integration: aktiver Flug in Boarding + neue OFP-ID am phpVMS-Bid → `planned_*` wird ueberschrieben

### Akzeptanz an Real-Pilot-Workflow

Nach v0.7.7 muss gelten:
- Pilot im Boarding, neue OFP-ID via PAX Studio "Laden von SB" verfuegbar
- Pilot klickt "⟳ Aktualisieren" im Bid-Tab
- → Loadsheet-Werte sind aktualisiert ohne Tab-Wechsel
- Pilot kann weiterhin den dedizierten Cockpit-Refresh-Button nutzen
- Nach Takeoff: "Aktualisieren" macht nur Bid-Liste, `planned_*` bleiben festgenagelt

---

## 9. Offene Punkte fuer Thomas

- [ ] **W4 verifizieren:** updated PAX Studio "Laden von SB" wirklich die `simbrief.id` am Bid? Schneller Test: `https://german-sky-group.eu/api/user/bids` vor und nach dem Klick anschauen.
- [ ] **Phase-Gate-Werte bestaetigen:** `preflight | boarding | taxi_out` wie im Cockpit-Button — okay so, oder soll der Bid-Tab auch in `taxi_out` greifen?
- [ ] **Toast-Wording fuer Option B:** "OFP unveraendert" reicht, oder ausfuehrlicher?
- [ ] **Aufwandsschaetzung:** ~30 Zeilen Frontend + ~10 Zeilen Backend + 2-3 Tests. Halbe Stunde Code, kein Tag-Bedarf ohne dein Go.

---

## 10. Versionierung dieser Spec

- **v1.0 (2026-05-11):** Initial Stand-Aufnahme + Loesungs-Optionen
