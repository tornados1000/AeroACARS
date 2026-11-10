# OFP-Refresh waehrend Boarding — Stand-Aufnahme + Spec

**Status:** Draft v1.4 nach 4. Thomas-Review (Struktur-Placement + Snippet-Korrektur + Tests-Pflicht)
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
| **Cockpit-Tab** "OFP refreshen"-Button (kleines Action-Row) | `ActiveFlightPanel.handleRefreshOfp` → `flight_refresh_simbrief` | re-fetch bids + neuer OFP vom Bid + UEBERSCHREIBT `planned_*` im aktiven Flug | nur `preflight\|boarding\|taxi_out` (siehe v1.1-Korrektur in §6) |
| **Loadsheet-Card** Inline-Refresh-Button (v0.5.46 Adrian-Fix) | `LoadsheetMonitor.handleRefreshOfp` → `flight_refresh_simbrief` | identisch zu (2) | nur `preflight\|boarding` UND wenn OFP-Outdated-Heuristik triggert |

**Kern-Erkenntnis:** Nur (2) und (3) aktualisieren wirklich die `planned_*`-Werte im aktiven Flug. (1) — der prominente Button im Bid-Tab — tut das NICHT.

---

## 3. Real-Pilot-Workflow vs Tool-Reaktion

| Schritt | Pilot tut | AeroACARS-Reaktion | Erwartung |
|---|---|---|---|
| 1 | bookt Bid in phpVMS | — | — |
| 2 | regeneriert OFP auf simbrief.com | — | — |
| 3 | startet AeroACARS, klickt "Flug starten" | `flight_start` → `fetch_simbrief_ofp(sb.id)` → schreibt `planned_*` in FlightStats | ✓ |
| 4 | belaedt im Sim Pax/Cargo/Fuel | — | — |
| 5 | merkt: OFP-Werte passen nicht | — | — |
| 6 | aendert auf simbrief.com → neuer OFP | — | — |
| 7 | klickt **PAX Studio "Laden von SB"** auf phpVMS-Site | phpVMS-Bid bekommt neue `simbrief.id` (server-side) | ✓ |
| 8 | klickt **AeroACARS "⟳ Aktualisieren"** im Bid-Tab | `phpvms_get_bids` zieht neue Bid-Liste (mit neuer `simbrief.id`), aber **`planned_*` im aktiven Flug bleiben alt** | ❌ Pilot erwartet aktualisierten OFP |
| 9 | sieht: Loadsheet-Werte sind weiter falsch | — | (frustrierter Pilot) |
| 10 | **wenn Glueck**: findet Cockpit-Refresh-Button oder Loadsheet-Inline-Refresh-Button — UND der Bid existiert noch (= seltener phpVMS-Zustand vor Prefile, siehe W5) | `flight_refresh_simbrief` zieht neue OFP → `planned_*` ueberschrieben | ✓ |

**Ergebnis:** Der prominente Button im Tab "Meine Fluege" (= dort wo der Pilot zuerst schaut) macht NICHT was er erwartet, und der wirksame Button ist in einem anderen Tab versteckt.

---

## 4. Code-Anchors (Stand v0.7.6)

| Datei | Zeile | Was |
|---|---|---|
| `client/src/components/BidsList.tsx` | 240-258 | `handleRefresh()` — der "falsche" Button |
| `client/src/components/ActiveFlightPanel.tsx` | 138-155 | `handleRefreshOfp()` — wirksam, aber versteckt |
| `client/src/components/ActiveFlightPanel.tsx` | 249-266 | Phase-Gate `preflight\|boarding\|taxi_out` — **fehlt `pushback`** (siehe v1.1 §6) |
| `client/src/components/LoadsheetMonitor.tsx` | 102-122 | Inline-Refresh + OFP-Outdated-Heuristik |
| `client/src/components/LoadsheetMonitor.tsx` | 76-93 | Heuristik fuel-delta >= 400 kg OR >= 5% AND zfw-delta < 200 kg |
| `client/src-tauri/src/lib.rs` | 4327-4427 | `flight_refresh_simbrief` Command |
| `client/src-tauri/src/lib.rs` | 1549 | `FlightStats.flight_plan_source` (existiert) — **kein `simbrief_ofp_id` Feld** (siehe v1.1 §6 P2-Fix) |
| `client/src-tauri/src/lib.rs` | 4400 | `stats.flight_plan_source = Some("simbrief")` |
| `client/src-tauri/crates/api-client/src/lib.rs` | 1146-1177 | `fetch_simbrief_ofp` — gibt `Ok(None)` bei Netzwerk-/HTTP-Fehler (siehe v1.1 §6 Punkt 4) |
| `client/src-tauri/crates/sim-core/src/lib.rs` | 677-703 | FlightPhase enum — `Pushback` ist zwischen `Boarding` und `TaxiOut` |

---

## 5. Mutmassliche Wurzeln (priorisiert)

### W1 — UI-Discoverability (Haupt-Wurzel)
Pilot drueckt im Tab "Meine Fluege" auf "Aktualisieren" und erwartet "alles wird neu gezogen", inklusive aktivem Flug. Der Button macht aber nur Bid-Liste + Sim-Resync + Profile.

### W2 — Phase-Gate-Inkonsistenz (klar)
Cockpit-Button gated heute auf `preflight | boarding | taxi_out`. **`Pushback` fehlt** — dort verschwindet der Button obwohl die Phase noch pre-takeoff ist und der Plan noch nutzbar ueberschrieben werden koennte (Loadsheet sieht den Plan, der Touchdown-Score noch nicht). Bewusste Entscheidung notwendig.

### W3 — Cache-Layer? (unwahrscheinlich)
SimBrief antwortet auf jede ID frisch. phpVMS-Cache wuerde nur greifen wenn paxstudio das so konfiguriert hat (VA-spezifisch).

### W4 — PAX Studio "Laden von SB" updated nicht die OFP-ID am Bid? (v1.2: widerlegt)
**Update v1.2 nach QS des PAX-Studio-Repos:** PAX Studio arbeitet beim "Load/Sync from SB" korrekt — `DashboardController::sync()` nutzt `static_id`, ruft `downloadOfpCompat(...)` und bekommt ein aktualisiertes `SimBrief`-Model zurueck. Hat sogar Guards (`ofp_mismatch`) und schuetzt SimBrief-Daten waehrend ACARS-IN_PROGRESS-PIREP. Die Wurzel sitzt also **nicht** in PAX Studio.

### W5 — Bid verschwindet aus `/api/user/bids` sobald AeroACARS prefiled hat (v1.2 NEU — kritisch)

Vom PaxStudio-Changelog dokumentiert: **phpVMS 7 entfernt einen Bid sobald ACARS einen PIREP prefiled hat.** AeroACARS' `flight_start` ruft `prefile_pirep` an Position lib.rs:5322 — also sobald der Pilot "Flug starten" klickt. Resultat:

```
flight_start                 phpVMS server-side                AeroACARS local state
─────────────                ──────────────────                ─────────────────────
client.prefile_pirep() ───→  PIREP angelegt                    
                             Bid wird AUTO-REMOVED             
                                                               flight.bid_id bleibt
                                                               (im active_flight state)
                                                               
[Boarding, Pilot regeneriert OFP, klickt "Laden von SB"]
                                                               
                             PAX Studio updated SimBrief-      
                             record  server-side               
                                                               
[Pilot klickt "⟳ Aktualisieren" im Bid-Tab]                    
                                                               
                                                          ┌──→ flight_refresh_simbrief:
                                                          │    client.get_bids()
client.get_bids() ────────→  Bid-Liste OHNE den           │    .find(|b| b.id == bid_id)
                             gefilten Bid               ──┴─→  None → bid_not_found ERR
```

**Konsequenz:** Das ganze `flight_refresh_simbrief`-Command in seiner heutigen Form ist **unbrauchbar im realen Boarding-Fall** weil es exklusiv ueber den Bid-Pointer geht, der bereits weg ist. Die in v1.0/v1.1 spezifizierte v0.7.7-Loesung (Bid-Tab ruft `flight_refresh_simbrief`) wuerde im echten Pilot-Workflow `bid_not_found` triggern und nichts veraendern.

**Beleg im AeroACARS-Code:**
- `flight_start` ruft `client.prefile_pirep` **vor** Boarding-Beginn (lib.rs:5322)
- `flight_refresh_simbrief` ruft `client.get_bids() + .find(|b| b.id == bid_id)` ohne Fallback (lib.rs:4344-4355)
- `consume_bid_best_effort` (AeroACARS-eigene Bid-Loeschung) feuert nur AT-FILE-TIME, nicht AT-PREFILE — der serverseitige Auto-Remove ist also unabhaengig davon

**Quick-Check fuer User:** Nach "Flug starten" einmal `https://german-sky-group.eu/api/user/bids` aufrufen (Browser, eingeloggt) und schauen ob der Bid noch da ist. Wenn nein → W5 bestaetigt.

**Konsequenz fuer v0.7.7-Scope:** Siehe §7 — Spec wird ehrlich umgeschnitten.

---

## 6. v1.1/v1.2-Refinement nach Thomas-Reviews

### 6.1 P2 unterschaetzt — Persistenz-Feld noetig

`FlightStats` traegt heute kein `simbrief_ofp_id` und kein `simbrief_ofp_generated_at` — nur `flight_plan_source = Some("simbrief")` als Marker (lib.rs:1549). Fuer "OFP unveraendert"-Feedback (Toast) brauchen wir den alten Wert UND den neuen zum Vergleich.

**Erweiterung der Spec:**

```rust
// FlightStats (lib.rs ~1549) — runtime-state
flight_plan_source: Option<&'static str>,
// NEU:
simbrief_ofp_id: Option<String>,           // "1777622821_5F3E3B3842"
simbrief_ofp_generated_at: Option<String>, // raw SimBrief <params><time_generated>
```

**v1.2-Korrektur Punkt 1:** Plus in `PersistedFlightStats` an **lib.rs:806** (NICHT `storage/src/lib.rs` — `PersistedFlightStats` ist lokal definiert, `storage::LandingRecord` ist eine andere Struktur). Beide neue Felder bekommen `#[serde(default)]` damit alte Persistenz lesbar bleibt.

**v1.2/v1.4-Korrektur Punkt 2:** `ofp_generated_at` ist im Parser heute **`String`** (api-client/lib.rs:444), gelesen 1:1 aus `<params><time_generated>`. Spec v1.0/v1.1 hatte faelschlich `DateTime<Utc>` aus `<times><sched_out>` vorgeschlagen — das wuerde Parser-Refactor + Datums-Parsing erfordern.

Fuer v0.7.7 persistieren wir den **raw SimBrief `time_generated` string** als `Option<String>` (was der Parser eben liefert — typisch Unix-Timestamp-String, aber wir machen keine Annahmen ueber Format). Spec v1.3 nannte das faelschlich "ISO-String" — das ist nicht garantiert, korrigiert in v1.4. Falls v0.7.8 ein normalisiertes Display braucht, kann der Parser dann bewusst auf `DateTime<Utc>` umgestellt werden.

**v1.3-Korrektur (Punkt 2): `flight_id` als Foundation in v0.7.7 mitnehmen.**

`ActiveFlight` (lib.rs:691) traegt heute `bid_id`, `flight_number`, `dpt_airport`, `arr_airport` — aber **NICHT** den `flight_id`-String aus dem phpVMS-Bid. `Bid.flight_id: String` ist aber im API-Response sehr wohl da (api-client/lib.rs:500).

Konsequenz wenn wir das nicht in v0.7.7 mitnehmen:
- W5-Workflow: Bid weg nach Prefile, `client.get_bids()` liefert nicht mehr den `flight_id`
- v0.7.8 Variante A (PAX-Studio-Endpoint `/api/paxstudio/flights/{flight_id}/simbrief`) — der `flight_id`-Schluessel ist dann verloren
- v0.7.8 Variante B (SimBrief-direct) — koennte auch `flight_id` fuer Match-Verifikation nutzen

Daher: **AeroACArS soll `flight_id` AT FLIGHT_START aus dem Bid extrahieren und persistieren**, solange der Bid noch da ist (vor dem `prefile_pirep`-Call).

**v1.4-Korrektur (Punkt 1): Struktur-Placement.**

`flight_id` gehoert **top-level in `ActiveFlight` + `PersistedFlight`**, nicht in `PersistedFlightStats`. Begruendung:
- `PersistedFlight` (lib.rs:775) traegt schon `bid_id: i64` top-level
- `stats: PersistedFlightStats` ist Telemetrie + FSM-State (distance, fuel, phase) — `flight_id` ist Identifier, nicht Telemetrie
- Analogie zu `bid_id` macht das Mapping selbstverstaendlich

```rust
// ActiveFlight (lib.rs:691) — runtime state
struct ActiveFlight {
    bid_id: i64,
    flight_id: String,    // NEU v0.7.7 — Sibling von bid_id, NICHT in stats
    flight_number: String,
    // ...
}

// PersistedFlight (lib.rs:775) — disk-snapshot
struct PersistedFlight {
    pirep_id: String,
    bid_id: i64,
    #[serde(default)]
    flight_id: String,    // NEU v0.7.7 — Sibling von bid_id
    started_at: DateTime<Utc>,
    // ...
    stats: PersistedFlightStats,  // hier KEINE Aenderung
}

// PersistedFlightStats (lib.rs:806) — bleibt unveraendert
// (nur simbrief_ofp_id + simbrief_ofp_generated_at kommen dazu)
```

**v1.4-Korrektur (Punkt 3): Snippet nutzt `bid` direkt.**

Im `flight_start` (lib.rs:5152) ist nach dem Bid-Lookup die Variable `bid: Bid` (kein Option — der `ok_or_else` an Z. 5155 erzwingt das). Also:

```rust
// flight_start (lib.rs:5152) — NACH Bid-Lookup, VOR prefile_pirep
let bid = bids
    .into_iter()
    .find(|b| b.id == bid_id)
    .ok_or_else(|| UiError::new("bid_not_found", ...))?;

let flight_id = bid.flight_id.clone();
// Im ActiveFlight-Init weiter unten als bid.flight_id.clone() durchreichen.
```

Die `matching_bid.map(...)`-Version aus v1.3 stammte versehentlich aus dem Resume-/Adopt-Pfad (lib.rs:5005) wo `matching_bid: Option<&Bid>` ist — anderer Code-Pfad, anderes Pattern. `flight_start` selbst hat `bid` direkt.

`flight_start` setzt beide. `flight_refresh_simbrief` liest den alten Wert, vergleicht mit dem neuen aus dem frisch geholten OFP, und gibt das Ergebnis im Result-DTO mit zurueck.

**v1.2-Korrektur Punkt 3 (DTO-Split):** `SimBriefOfpDto` wird heute von ZWEI Commands genutzt:
- `flight_refresh_simbrief` (lib.rs:4327) — der Refresh-Pfad
- `fetch_simbrief_preview` (lib.rs:4437) — die Bid-Card-Vorschau **bevor** der Flight gestartet ist (also gar kein "previous_ofp_id"-Konzept moeglich)

Wenn wir `previous_ofp_id` / `current_ofp_id` / `changed` direkt in `SimBriefOfpDto` einbauen, wird die Preview unnoetig komisch — der Wert waere immer `previous = None, current = id, changed = true` (Tautologie). Sauberer:

```rust
// Bleibt unveraendert — fuer Preview-Pfad
struct SimBriefOfpDto { /* bestehende Felder */ }

// NEU — nur fuer Refresh-Pfad
struct SimBriefRefreshResult {
    pub ofp: SimBriefOfpDto,
    pub previous_ofp_id: Option<String>,
    pub current_ofp_id: String,
    pub changed: bool,
}

#[tauri::command]
async fn flight_refresh_simbrief(...) -> Result<SimBriefRefreshResult, UiError> {
    // ...
}
```

Frontend nutzt `result.changed` fuer den Toast, `result.ofp.*` fuer Plan-Werte. `fetch_simbrief_preview` bleibt bei `Option<SimBriefOfpDto>`.

### 6.2 Phase-Gate inklusive `Pushback`

Heute im Cockpit-Button: `preflight | boarding | taxi_out`. Spec v1.1 entscheidet:

**Gate fuer v0.7.7: `Preflight | Boarding | Pushback | TaxiOut`**

Begruendung:
- `Pushback` ist die Phase wo der Flieger schon Cleared-Pushback hat, aber noch nicht rollt. Plan-Werte sind weiterhin nutzbar fuer Loadsheet-Vergleich und sub_scores.
- Erst `TakeoffRoll` aufwaerts soll der Plan festgenagelt sein (Score-Aggregat).
- Der heutige Gate-Vorschlag im Spec-v1.0 (ohne `Pushback`) war inkonsistent zur Begruendung "bis vor Takeoff".

Backend (`flight_refresh_simbrief`) bekommt den expliziten Gate-Check:

```rust
if !matches!(current_phase,
    FlightPhase::Preflight
        | FlightPhase::Boarding
        | FlightPhase::Pushback
        | FlightPhase::TaxiOut)
{
    return Err(UiError::new(
        "phase_locked",
        "OFP-Refresh ist nur bis vor Takeoff moeglich (Preflight bis TaxiOut)",
    ));
}
```

**v1.2-Korrektur Punkt 5 (Loadsheet nicht blind syncen):** Backend-Gate + Cockpit-Button-Sichtbarkeit auf `Preflight|Boarding|Pushback|TaxiOut` ja. ABER `LoadsheetMonitor.tsx` ist heute bewusst auf `preflight|boarding` begrenzt (lib.tsx:52) — die Begruendung "Loadsheet abgeschlossen sobald TaxiOut beginnt" ist eine eigene UX-Entscheidung. Wir aendern in v0.7.7 **NICHT** das Loadsheet-Phase-Gate mit. Konsequenz:

- Cockpit-Button (ActiveFlightPanel) sichtbar in `preflight|boarding|pushback|taxi_out`
- Loadsheet-Inline-Button (LoadsheetMonitor) sichtbar in `preflight|boarding` (unveraendert)
- Backend `flight_refresh_simbrief` akzeptiert alle vier Phasen

Falls Pilot in `Pushback` oder `TaxiOut` einen Refresh braucht, geht das via Cockpit-Button — der Loadsheet-Card ist dort eh nicht mehr offen. Loadsheet-bis-Pushback waere ein eigener UX-Schnitt (eigene Spec).

### 6.3 P3 ist Erweiterung, nicht neu

`flight_refresh_simbrief` loggt bereits "OFP refreshed" mit Plan-Werten (lib.rs:4404-4412). v1.1 erweitert um:

```rust
log_activity(
    &state,
    ActivityLevel::Info,
    if changed { "OFP refreshed" } else { "OFP unchanged" }.to_string(),
    Some(format!(
        "{} → {} ({}). Block {:.0} kg, TOW {:.0} kg, LDW {:.0} kg",
        previous_ofp_id.as_deref().unwrap_or("—"),
        current_ofp_id,
        if changed { "neu" } else { "identisch" },
        ofp.planned_block_fuel_kg, ofp.planned_tow_kg, ofp.planned_ldw_kg
    )),
);
```

Damit der JSONL-Audit-Trail im Replay sichtbar macht "ja, der OFP wurde refresht zur Zeit X, alt → neu".

### 6.4 Fehlersemantik in `fetch_simbrief_ofp` schaerfen

Heute (api-client/lib.rs:1146-1177):
- Netzwerk-Fehler → `Ok(None)`
- Non-2xx HTTP → `Ok(None)`
- Body-Read-Fehler → `Ok(None)`
- Erfolg, aber Parse fehlgeschlagen → `Ok(None)` (via `parse_simbrief_ofp`)

Caller (`flight_refresh_simbrief`) sieht nur `Ok(None)` → wirft `ofp_unusable`. Pilot kann nicht unterscheiden ob:
- SimBrief offline
- OFP-ID existiert nicht / wurde geloescht
- OFP existiert, aber XML hat unerwartetes Format

**Erweiterung:**

```rust
pub enum SimBriefFetchError {
    Network(reqwest::Error),    // → Toast: "SimBrief nicht erreichbar"
    HttpStatus(StatusCode),     // 404 → "OFP-ID nicht gefunden", 5xx → "SimBrief-Fehler"
    BodyRead,                   // selten — Netzwerk-Abbruch nach Header
    ParseFailed,                // → "OFP-Format unbekannt"
    OfpUnusable,                // Plan-Werte 0/negativ → "OFP unvollstaendig"
}

pub async fn fetch_simbrief_ofp(
    &self, ofp_id: &str,
) -> Result<SimBriefOfp, SimBriefFetchError> { ... }
```

`flight_refresh_simbrief` mappt die Variante auf passende `UiError`-Codes/Strings → Toast hilft jetzt wirklich.

**Aufwand-Hinweis:** Diese Aenderung beruehrt auch `flight_start` und `fetch_simbrief_preview` (alle drei Caller). Migration: Caller die heute `Ok(None)` toleriert haben mappen `Err(SimBriefFetchError::*)` auf `Ok(None)` (keine Regression), neuer Caller (`flight_refresh_simbrief`) nutzt die Varianten differenziert.

### 6.5a Toast-Infrastruktur (v1.2 Punkt 4)

Im Code gibt es heute **keinen generischen `showToast`-Helper** (verifiziert via Grep). Das Pseudo-`showToast(...)` in der v1.1-Spec war eine versteckte Zusatzaufgabe.

**Entscheidung v1.2:** Fuer v0.7.7 reicht ein **lokaler State im `BidsList`-Header** — kein neuer Toast-Component-Sweep. Pattern:

```tsx
// in BidsList (oder Parent if needed)
const [refreshNotice, setRefreshNotice] = useState<{
  text: string;
  tone: "info" | "warn";
} | null>(null);

// auto-clear nach 6s
useEffect(() => {
  if (!refreshNotice) return;
  const t = setTimeout(() => setRefreshNotice(null), 6000);
  return () => clearTimeout(t);
}, [refreshNotice]);

// Render: kleiner Info-Pill direkt unter dem Header-Button
{refreshNotice && (
  <div className={`bids-refresh-notice bids-refresh-notice--${refreshNotice.tone}`}>
    {refreshNotice.text}
  </div>
)}
```

Wenn spaeter ein VA-uebergreifender Toast-Bedarf entsteht (z.B. Fehler-Toasts, Score-Toasts) kann man das in v0.8.x in eine `<Toast>`-Component oder einen `useToast()`-Hook umziehen — heute waere das overengineering fuer einen einzigen Notice-Punkt.

### 6.5b UI-Update sofort nach Refresh

`flight_status` wird vom Cockpit-Tab 2-sekuendlich gepollt → Loadsheet sieht den neuen Plan spaetestens 2s nach Refresh. Im Bid-Tab haengt das Update vom 15s-Bid-Poll (im Boarding pausiert!) ab — also potenziell ueberhaupt nicht ohne weiteres Trigger.

**Loesung:** `flight_refresh_simbrief` emit-t am Ende ein `flight-status-update`-Event (oder benutzt den bestehenden Status-Refresh-Mechanismus). Bid-Tab horcht NICHT auf flight_status — also entweder:
- (a) BidsList.handleRefresh ruft nach erfolgreichem `flight_refresh_simbrief` ein `invoke("flight_status")` und propagiert die neuen Werte (= Parent-Component aktualisiert sich)
- (b) Backend emit-t `app.emit("flight-status-changed", ...)` nach jedem `flight_refresh_simbrief` und alle Listener (Cockpit + Loadsheet + ggf. BidsList) bekommen die Aenderung.

Variante (b) ist sauberer aber groesserer Eingriff. v0.7.7 macht (a) — Bid-Tab triggert einen Status-Refresh als Teil seiner Refresh-Chain.

### 6.6 Aufwand-Korrektur

Spec v1.0 schaetzte ~30 Zeilen Frontend + ~10 Backend. Mit allen v1.1-Aenderungen realistisch:

| Punkt | LOC-Schaetzung |
|---|---|
| P1 (Bid-Tab calls flight_refresh_simbrief) | ~15 Frontend |
| Phase-Gate-Backend + Frontend-Sync | ~10 Backend + ~5 Frontend |
| Persistenz-Feld `simbrief_ofp_id` | ~5 Stats + ~5 PersistedStats + ~10 set/get-Sites |
| Result-DTO Erweiterung `previous_ofp_id` + `changed` | ~10 |
| Toast-Wording + i18n (DE+EN, evtl. IT) | ~15 |
| Activity-Log Erweiterung | ~10 |
| `fetch_simbrief_ofp` Result-Typ-Refactor | ~40 (incl. 3 Caller-Anpassungen) |
| UI-Update-Trigger nach Refresh | ~10 |
| Tests | ~50 |

**Geschaetzt: 150-200 LOC Diff** ueber 5-6 Files. "Kleiner Patch, aber nicht 40 LOC."

Falls v0.7.7-Schnitt zu gross wird: §6.4 (Result-Typ) kann auf v0.7.8 verschoben werden — der `ofp_unusable`-Fall ist heute nicht so haeufig dass die Praezisierung Tag-relevant ist.

---

## 7. Soll-Verhalten (Spec) — **v1.2: ehrliche Scope-Trennung wegen W5**

W5 zwingt uns die v0.7.7-Erwartungen zu trennen. Es gibt zwei Sorten von Verbesserungen:

### 7.1 UX-Schicht (in v0.7.7 lieferbar — auch ohne W5-Loesung)

1. **"Aktualisieren" im Bid-Tab versucht** den OFP-Refresh fuer den aktiven Flug zusaetzlich. Wenn Bid noch da (= ungewoehnlicher Fall in phpVMS-7 — siehe W5): Plan-Werte werden ueberschrieben.
2. **Wenn Bid weg** (= `bid_not_found`): kein Crash, sondern klarer Hinweis-Pill mit ehrlichem Text (siehe §8 Notice-Tabelle — der frueher vorgeschlagene "Cockpit-Refresh nutzen"-Text war falsch).
3. **Discoverability:** Pilot kriegt im Bid-Tab ein Feedback (Pill), statt schweigender Stille.
4. **Schnelles UI-Update:** nach erfolgreichem Refresh aktualisiert sich das Loadsheet binnen 1s.
5. **Persistenz-Foundation (v1.3 erweitert):**
   - `simbrief_ofp_id` wird am `flight_start` gespeichert
   - `simbrief_ofp_generated_at` wird am `flight_start` gespeichert
   - **`flight_id` wird am `flight_start` gespeichert** — aus `Bid.flight_id`, solange der Bid noch da ist (Punkt 2 des v1.3-Reviews)
   - Beide neuen Identifier brauchen wir in v0.7.8 — egal welche W5-Loesungs-Variante. Wenn wir das v0.7.7 verpassen, ist `flight_id` nach Prefile fuer immer weg.

### 7.2 Daten-Pfad-Schicht (BRAUCHT W5-Loesung — NICHT in v0.7.7)

Echte fresh-OFP-Pickup waehrend Boarding ist heute aus zwei Gruenden blockiert:
- **Bid weg nach Prefile** (W5) → Pointer-Quelle existiert nicht mehr
- **Kein alternativer Pointer im AeroACARS-API-Set** — PIREP-Endpoint (PirepFull lib.rs:792) traegt heute kein `simbrief`-Feld; ohne zusaetzliche server-side Hilfe (PAX Studio Endpoint ODER SimBrief-direct-by-Username) kommen wir an die neue OFP-ID nicht heran.

**v0.7.8** (eigene Spec, gerne kombiniert mit §11 strategischer Option):
- Variante A: PAX Studio implementiert serverseitig `/api/paxstudio/flights/{flight_id}/simbrief` (= "latest valid briefing for active flight"). AeroACARS-Anpassung minimal.
- Variante B: AeroACARS speichert Pilot-SimBrief-Username in Settings, holt `xml.fetcher.php?username=X` direkt, verifiziert Flight-Match. PAX-Studio-unabhaengig (= attraktiver).

### 7.3 Was wir NICHT tun in v0.7.7

- Kein Phase-Limit-Aufweichen nach Takeoff
- Kein neuer Score-Logik-Pfad
- Kein Pax-Studio-Reverse-Engineering oder Server-Endpoint-Implementierung
- Kein Auto-Refresh-Polling
- Kein Architektur-Wechsel zu "SimBrief-direkt-by-username" (v0.7.8)
- Kein Versuch "OFP via PirepFull" — der Endpoint traegt das Feld heute nicht, das ist ein separates Schema-Item

---

## 8. Loesungs-Optionen (Detail)

### Option A1 (gewaehlt fuer v0.7.7) — v1.2 mit W5-Fallback + TS-Type-Fix

`BidsList.handleRefresh` ruft `flight_refresh_simbrief` zusaetzlich auf, Phase-Gate im Backend, Result-DTO `SimBriefRefreshResult` (siehe §6.1 DTO-Split).

**v1.2-Implementation-Hint:** `Promise<unknown>[]` verliert die Typinfo fuer `freshProfile`. Stattdessen einzelne benannte Promises oder Profile separat awaiten:

```ts
async function handleRefresh() {
  if (refreshing) return;
  setRefreshing(true);

  // Einzelne benannte Promises statt unknown[]-Array damit TypeScript
  // die Result-Typen behaelt.
  const bidsP = fetchBids();
  const simP = invoke("sim_force_resync").catch(() => null);
  const profileP = invoke<Profile | null>("phpvms_refresh_profile").catch(() => null);

  // hasActiveFlight kommt aus dem Component-State / Prop
  const refreshP: Promise<SimBriefRefreshResult | null> = hasActiveFlight
    ? invoke<SimBriefRefreshResult>("flight_refresh_simbrief").catch(
        (err: { code?: string; message?: string }) => {
          // Erwartete benigne Fehler — silent, oder kleiner Pill je nach Code:
          //   phase_locked    → silent (z.B. Pilot druckt Refresh im Cruise)
          //   no_simbrief_link → silent (kein OFP zum Refreshen)
          //   bid_not_found   → W5! Pilot-Hinweis-Pill (siehe unten)
          if (err?.code === "bid_not_found") {
            setRefreshNotice({
              text: t("bids.ofp_bid_gone"),
              tone: "warn",
            });
          }
          // ofp_fetch_failed / ofp_unusable → Activity-Log (bestehender Pfad)
          return null;
        },
      )
    : Promise.resolve(null);

  const [, , freshProfile, refreshResult] = await Promise.all([
    bidsP, simP, profileP, refreshP,
  ]);

  if (freshProfile && onProfileRefreshed) onProfileRefreshed(freshProfile);

  // v0.7.7: Notice wenn OFP refresht aber unveraendert blieb (W4 sichtbar machen)
  if (refreshResult && !refreshResult.changed) {
    setRefreshNotice({
      text: t("bids.ofp_unchanged", { id: refreshResult.current_ofp_id }),
      tone: "info",
    });
  }

  // §6.5b: Status-Refresh triggern damit Cockpit + Loadsheet sofort die
  // neuen Werte sehen, nicht erst nach 2s-flight_status-Poll
  if (refreshResult?.changed) {
    onActiveFlightUpdated?.();
  }

  setTimeout(() => setRefreshing(false), 400);
}
```

**v1.2 Notice-Wordings (3 Varianten je nach Outcome):**

| Outcome | Notice-Tone | Text (DE) | Text (EN) |
|---|---|---|---|
| Bid weg (`bid_not_found`, = W5-Fall, real-world Mehrheit) | warn | "Bid nicht mehr verfuegbar nach Prefile. OFP-Refresh ueber phpVMS-Pointer ist in diesem Flugzustand nicht moeglich. SimBrief-direkt kommt mit v0.7.8." | "Bid no longer available after prefile. OFP refresh via phpVMS pointer not possible in this flight state. SimBrief-direct coming in v0.7.8." |
| OFP refreshed, unveraendert (`changed=false`) | info | "OFP unveraendert. phpVMS meldet weiterhin OFP-ID {id}. Bitte PAX Studio 'Laden von SB' pruefen." | "OFP unchanged. phpVMS still reports OFP ID {id}. Check PAX Studio 'Load from SB'." |
| OFP refreshed, neu (`changed=true`) | info (optional, oder kein Pill) | "OFP aktualisiert: Block {block} kg, TOW {tow} kg, LDW {ldw} kg." | "OFP refreshed: Block {block} kg, TOW {tow} kg, LDW {ldw} kg." |

**v1.3-Korrektur (Punkt 1):** Frueherer Text "Cockpit-Tab nutzen" war falsch — der Cockpit-Button ruft denselben `flight_refresh_simbrief`-Command auf und scheitert bei W5 genauso. Der Text ist jetzt ehrlich, dass der pointer-basierte Pfad in diesem Flugzustand grundsaetzlich tot ist — egal welcher Button.

**Honesty-Note:** Die `bid_not_found`-Notice ist in v0.7.7 wahrscheinlich der **haeufigste Outcome** (W5), nicht der seltene Edge-Case. Pilot kriegt damit immerhin eine klare Erklaerung statt schweigender Stille. Echtes Fix kommt in v0.7.8.

### Option B-erweitert (Toast wenn unveraendert) — siehe Option A1 oben, ist da integriert

### Option C (Auto-Refresh-Polling) — auf spaeter verschoben, eigener Schnitt

---

## 9. Entscheidungen aus Thomas-Reviews

### v1.1
| Punkt | Entscheidung |
|---|---|
| **W4** (PAX Studio updated `simbrief.id`?) | extern verifizieren, aber **nicht als Blocker fuer P1**. P2 macht es im Toast sichtbar. |
| **Phase-Gate** | `Preflight \| Boarding \| Pushback \| TaxiOut` (inkl. Pushback) — Begruendung "bis Takeoff" |
| **v0.7.7-Scope** | P1 + Backend-Gate + Persistenz-Feld + Toast + UI-Update-Trigger |
| **Auto-Refresh** | weiter spaeter (eigene v0.8.x-Diskussion) |
| **Result-Typ-Refactor in `fetch_simbrief_ofp`** | Stretch-Goal v0.7.7 |

### v1.2 (nach 2. QS-Review)
| Punkt | Entscheidung |
|---|---|
| **W4 widerlegt** | PAX Studio arbeitet beim Sync korrekt — kein server-side Bug dort |
| **W5 kritisch** | Bid weg nach Prefile → `flight_refresh_simbrief` ist im Real-World-Fall ohne Bid-Pointer |
| **v0.7.7-Honest-Scope-Trennung** | UX-Schicht (Notice + Persistenz-Foundation + UI-Update) JA. Daten-Pfad-Fix (echter fresh-OFP-Pickup) NEIN — braucht v0.7.8 |
| **Persistenz-Ort** | `PersistedFlightStats` lokal in `lib.rs:806`, nicht `storage`-Crate |
| **Typ von `simbrief_ofp_generated_at`** | `Option<String>` — Parser liefert String aus `<params><time_generated>`, kein DateTime-Refactor in v0.7.7 |
| **DTO-Split** | Neues `SimBriefRefreshResult` fuer Refresh-Pfad. `SimBriefOfpDto` bleibt fuer Preview unveraendert |
| **Toast-Infrastruktur** | Lokales `refreshNotice`-State im Bid-Tab-Header. Keine Toast-Component-Investition fuer v0.7.7 |
| **Loadsheet-Phase-Gate** | unveraendert `preflight\|boarding`. Pushback-Sichtbarkeit waere eigener UX-Schnitt |
| **Promise-Typing** | benannte Promises statt `Promise<unknown>[]` damit TS die Typen behaelt |
| **v0.7.8-Plan** | SimBrief-direct-by-username (= §11, frueher als v0.8.x) — bekommt eigene Spec |

### v1.3 (nach 3. QS-Review)
| Punkt | Entscheidung |
|---|---|
| **`bid_not_found`-Notice-Text** | korrigiert — der frueher empfohlene "Cockpit-Refresh"-Hinweis war falsch, weil Cockpit-Button denselben toten Pfad nutzt. Neuer Text ist ehrlich dass der pointer-basierte Pfad in diesem Zustand grundsaetzlich tot ist. |
| **`flight_id` als v0.7.7-Foundation** | `ActiveFlight` + `PersistedFlight` bekommen `flight_id: String` (aus `Bid.flight_id`). Muss VOR `prefile_pirep`-Call extrahiert werden, sonst spaeter verloren. Voraussetzung fuer beide v0.7.8-Varianten (PAX-Studio-Endpoint ODER SimBrief-direct). |
| **`simbrief_username`-Settings-Feld** | v0.7.8 braucht eigene Settings-UI + Persistenz (B1) — phpVMS-Profile traegt das heute nicht und ist nicht in AeroACARS-Kontrolle. B2 (phpVMS-API-Erweiterung) waere VA-side Optimierung fuer spaeter. |
| **§12 Reihenfolge** | chronologisch korrigiert (v1.0 → v1.1 → v1.2 → v1.3 latest-last) |

### v1.4 (nach 4. QS-Review)
| Punkt | Entscheidung |
|---|---|
| **`flight_id` Struktur-Placement** | top-level in `ActiveFlight` + `PersistedFlight` (Sibling von `bid_id`), NICHT verschachtelt in `PersistedFlightStats`. Letzteres ist Telemetrie/FSM-State, nicht Identifier. |
| **`simbrief_ofp_generated_at`-Wording** | "raw SimBrief `time_generated` string" statt "ISO-String" — Parser macht heute keine Format-Annahme, wir auch nicht. |
| **flight_start-Snippet** | `let flight_id = bid.flight_id.clone();` — `bid: Bid` ist direkt verfuegbar nach Bid-Lookup in `flight_start` (lib.rs:5152), kein `Option::map` noetig. Die v1.3-`matching_bid.map(...)`-Form stammte aus dem Resume-/Adopt-Pfad (lib.rs:5005), nicht aus `flight_start`. |
| **§10 Tests-Wording** | Notice-Text aus §8 1:1 in Test-Beschreibung uebernommen (kein "Cockpit-Refresh nutzen" mehr). |
| **Pflicht-Tests v0.7.7** | NEU: `flight_start_persists_flight_id_before_prefile` + `resume_flight_preserves_flight_id` + `flight_refresh_simbrief_accepts_pushback` + `..._returns_phase_locked_in_takeoff_roll`. Phase-Gate-Pushback/TakeoffRoll-Boundary explizit getestet. |

---

## 10. Test-Vorschlaege

Backend (Rust) — Pflicht in v0.7.7:
- `flight_refresh_simbrief_returns_phase_locked_in_takeoff_roll` (TakeoffRoll explizit abgelehnt)
- `flight_refresh_simbrief_accepts_pushback` (Pushback explizit erlaubt — v1.1 Phase-Gate-Erweiterung)
- `flight_refresh_simbrief_returns_bid_not_found_when_phpvms_removed_bid` (W5-Real-World-Case)
- `flight_refresh_simbrief_marks_changed_false_when_ofp_id_identical` (= wenn Bid noch da)
- `flight_refresh_simbrief_marks_changed_true_when_ofp_id_new`
- **`flight_start_persists_flight_id_before_prefile`** (v1.4 Pflicht: `flight_id` muss VOR `prefile_pirep` in `ActiveFlight` gespeichert werden, damit W5-Bid-Entfernung den Schluessel nicht klaut)
- **`resume_flight_preserves_flight_id`** (v1.4 Pflicht: nach Tauri-Restart muss `flight_id` aus `PersistedFlight` zurueck in `ActiveFlight` kommen)
- `simbrief_ofp_id_persists_across_persisted_flight_stats_save_load`
- (falls Result-Typ-Refactor in v0.7.7): `simbrief_fetch_maps_404_to_not_found_variant`, `simbrief_fetch_maps_network_to_unreachable`

Frontend (manuell oder per Playwright-Smoke):
- Bid-Tab "Aktualisieren" im Boarding, Bid noch da, neue OFP-ID → Loadsheet zeigt neue Werte ohne Tab-Wechsel
- Bid-Tab "Aktualisieren" im Boarding, Bid weg (W5) → Notice **mit ehrlichem Text** "Bid nicht mehr verfuegbar nach Prefile. OFP-Refresh ueber phpVMS-Pointer ist in diesem Flugzustand nicht moeglich. SimBrief-direkt kommt mit v0.7.8." (v1.3-Korrektur: kein Hinweis auf Cockpit-Refresh — der nutzt denselben toten Pfad)
- Bid-Tab "Aktualisieren" im Boarding bei UNVERAENDERTER OFP-ID → Info-Notice mit OFP-ID
- Bid-Tab "Aktualisieren" in `TakeoffRoll`/`Cruise` → kein Crash, Bid-Liste wird trotzdem aktualisiert (`phase_locked` still ignoriert, kein Notice)

---

## 11. STRATEGISCHE OPTION: "SimBrief-direkt, PAX Studio raus"

Thomas-Vorschlag: *"Ich waere auch immer dafuer die frischen bzw die Daten immer von SB zu holen und Pax Studio raus zu lassen"*

### Was das heisst

Heute haengt AeroACARS an der `simbrief.id`, die phpVMS (PAX Studio) am Bid hinterlegt. Ein direkter Pfad waere:

```
Pilot regeneriert OFP auf simbrief.com
        ↓
AeroACARS fragt SimBrief direkt: "letzter OFP fuer User X?"
        ↓
SimBrief liefert latest OFP (incl. dpt/arr/callsign/etc.)
        ↓
AeroACARS verifiziert: passt dpt/arr/callsign zum aktiven AeroACARS-Flug?
        ↓ (ja)
AeroACARS uebernimmt OFP-Werte
        ↓
(phpVMS-Bid bleibt unberuehrt; nur fuer flight_number-Lookup beim Start verwendet)
```

SimBrief-API-Endpoint dafuer: `GET https://www.simbrief.com/api/xml.fetcher.php?username={username}` — gibt den **letzten** OFP fuer den User zurueck.

### Aufruf-Modell-Vergleich

| Aspekt | Heute (phpVMS-Pointer) | SimBrief-direkt |
|---|---|---|
| Abhaengigkeit von PAX Studio | hoch (Pointer-Update noetig) | gar nicht |
| Pilot-Workflow | regenerate + "Laden von SB" + AeroACARS-Refresh | nur regenerate + AeroACARS-Refresh |
| Erforderliche Pilot-Konfig | nichts (PAX Studio kennt SimBrief-User) | SimBrief-Username einmalig in AeroACARS-Settings |
| Failure-Mode | Pointer outdated → alter Plan | Pilot hat anderen OFP zwischendurch generiert (z.B. fuer einen anderen Flug) → AeroACARS muss flight-match verifizieren |
| Bid-Pax/Cargo-Zahlen | aus PAX-Studio-Subfleet via Bid | weiter aus phpVMS-Bid noetig (SimBrief OFP traegt keinen "echten" Bid-Pax-Stand) |
| VFR/Manual-Flights ohne OFP | unveraendert (kein OFP, kein Refresh) | unveraendert |

### Was waere noetig

1. **Pilot-Settings: NEUES Feld `simbrief_username`** (v1.3-Korrektur Punkt 3).

   AeroACARS' `Profile`-Struktur enthaelt heute **keinen** `simbrief_username`. Das ist KEINE phpVMS-Profile-Property die wir uebernehmen koennten — phpVMS speichert SimBrief-Verknuepfungen anders (am User, nicht am Profile-Object das die API liefert).

   Konsequenz: v0.7.8 braucht **eigene Settings-UI + Persistenz** dafuer. Zwei Wege:

   **(B1) AeroACArS-lokale Settings:**
   - Settings-Tab bekommt Feld "SimBrief-Username"
   - Persistiert via existing `Settings`-Mechanismus (Tauri-Store / disk file)
   - Pilot gibt einmalig ein, AeroACARS nutzt es fuer `xml.fetcher.php?username=X`

   **(B2) phpVMS-API-Erweiterung:**
   - phpVMS koennte ein `simbrief_username` im User-Profile exponieren (`/api/user/me` oder aehnlich)
   - AeroACArS liest beim Login mit
   - Vorteil: VA-zentral konfiguriert, Pilot muss nichts eingeben
   - Nachteil: braucht VA-side phpVMS-Anpassung, nicht in AeroACARS-Kontrolle

   **Empfehlung:** B1 fuer v0.7.8 (= rein AeroACARS-internal). B2 koennte spaeter als Optimierung dazu kommen.

2. **`fetch_simbrief_ofp_latest`-Command:** holt `xml.fetcher.php?username=X`, parsed gleich wie `fetch_simbrief_ofp` (mit zusaetzlichen Headern wie `<origin>`, `<destination>`, `<callsign>` fuer Verifikation).
3. **Flight-Match-Verifikation:** SimBrief-OFP.origin == AeroACARS-flight.dpt UND SimBrief-OFP.destination == AeroACARS-flight.arr UND (optional) callsign passt. Wenn Mismatch → Toast "SimBrief-OFP gehoert nicht zum aktiven Flug ({X} → {Y}), bitte regenerieren oder PAX-Studio-Fallback nutzen".
4. **Fallback-Logik:** wenn kein SimBrief-Username konfiguriert ODER kein passender OFP latest → bestehender phpVMS-Pointer-Pfad als Fallback (greift wenn Bid noch da; sonst `bid_not_found`-Notice wie in v0.7.7).

### Pro / Contra

**Pro:**
- Kein PAX-Studio-Sync-Frust mehr
- Workflow fuer Pilot kuerzer
- AeroACARS waere weniger gekoppelt an phpVMS-Modul-Implementations
- Bei W4 (PAX Studio updated nicht) gar kein Symptom mehr

**Contra:**
- Pilot muss einmalig SimBrief-Username eingeben — Friction fuer Erst-User
- Flight-Match-Verifikation fragt: was wenn Pilot zwischen Bid-Start und OFP-Refresh einen OFP fuer einen anderen Flug generiert hat? Mismatch-Toast verwirrt evtl.
- Bid-Pax/Cargo (Subfleet) braucht weiter phpVMS — wir koennen PAX Studio nicht komplett rausnehmen, nur bei der OFP-ID-Quelle
- VAs ohne PAX Studio (manche andere phpVMS-Themes) bekommen heute schon den phpVMS-Pointer-Pfad — bei SimBrief-direkt muessten wir Default-Fallback dokumentieren

### Empfehlung der Spec (v1.2 — wegen W5 nach vorne gezogen)

**v0.7.7:** UX-Schicht-Fix (siehe §7.1). Adressiert Discoverability + macht W5 fuer den Piloten sichtbar (Bid-weg-Notice). KEIN echter Daten-Pfad-Fix — dafuer braucht es W5-Loesung.

**v0.7.8 (NEU — eigene Spec):** SimBrief-direkt-by-username als zusaetzlicher Pfad. Damit ist der Bid-Pointer-Pfad nicht mehr die einzige Quelle und W5 ist umgangen. Spec-Punkte fuer v0.7.8:
- Settings-Feld "SimBrief Username" (optional)
- Neuer Command `fetch_simbrief_latest_by_user(username)` (api-client + Tauri-side)
- Flight-Match-Verifikation: SimBrief-OFP.origin/destination/callsign muss zum aktiven AeroACARS-Flug passen
- `flight_refresh_simbrief` neu: Pfad-Auswahl
  1. wenn `simbrief_username` gesetzt → SimBrief-direct zuerst, Flight-Match pruefen
  2. wenn (1) fehlt oder Mismatch → bestehender Bid-Pointer-Pfad (greift wenn Bid noch da, sonst `bid_not_found`)
  3. wenn beides leer → klare Fehler-Notice
- Backward-Compat: Pilot ohne SimBrief-Username verhaelt sich wie heute (= Bid-Pointer-Pfad mit W5-Limit)

**Alternative v0.7.8 (Variante A aus §7.2):** PAX Studio Server-Endpoint `GET /api/paxstudio/flights/{flight_id}/simbrief`. AeroACARS-seitig nur 10 Zeilen Pfad-Aenderung. Aber: braucht koordinierte PAX-Studio-Release — nicht in AeroACARS-Kontrolle.

**Empfohlen: Variante B (SimBrief-direct-by-username)** weil rein AeroACARS-internal und PAX-Studio-unabhaengig.

---

## 12. Versionierung dieser Spec

- **v1.0 (2026-05-11):** Initial Stand-Aufnahme + Loesungs-Optionen
- **v1.1 (2026-05-11):** Refinement nach 1. Thomas-Review:
  - §6.1 P2 Persistenz-Feld `simbrief_ofp_id` + `_generated_at` ergaenzt (war Unter-Punkt, jetzt Erstklassig)
  - §6.2 Phase-Gate auf `Preflight | Boarding | Pushback | TaxiOut` korrigiert (Pushback war vorher implizit ausgeschlossen)
  - §6.3 P3 Audit-Trail als Erweiterung des bestehenden "OFP refreshed"-Logs umformuliert
  - §6.4 Fehlersemantik in `fetch_simbrief_ofp` — Vorschlag fuer Result-Typ-Refactor (`Ok(None)` → spezifische Error-Varianten)
  - §6.5 UI-Update-Trigger nach Refresh damit Bid-Tab-Klick nicht 2s-Status-Poll abwartet
  - §6.6 Aufwand-Korrektur: 150-200 LOC realistisch, nicht 30+10
  - §9 4 Entscheidungen aus Thomas-Review festgehalten
  - §10 Tests-Vorschlaege konkretisiert
  - **§11 NEU:** Strategische Option "SimBrief-direkt, PAX Studio raus" dokumentiert — Pro/Contra, Settings-Modell, Fallback-Logik. Empfohlen als v0.8.x mit eigener Spec.
- **v1.2 (2026-05-11):** Nach 2. QS-Review von Thomas (W5-Architektur-Finding):
  - §5 W4 widerlegt (PAX Studio sync ist korrekt) + **W5 NEU** dokumentiert (Bid weg nach Prefile via phpVMS-7 = Hauptbloeker fuer den Daten-Pfad-Fix)
  - §6.1 v1.2-Korrekturen: PersistedFlightStats liegt in `lib.rs:806` (nicht storage-Crate); `simbrief_ofp_generated_at` als `Option<String>` (Parser-Realitaet, kein DateTime-Refactor)
  - §6.1 DTO-Split: `SimBriefRefreshResult` neu, `SimBriefOfpDto` bleibt unveraendert fuer Preview-Pfad
  - §6.2 Loadsheet-Phase-Gate NICHT mitziehen (eigener UX-Schnitt)
  - §6.5a Toast-Infrastruktur: lokaler `refreshNotice`-State statt nicht-existentem `showToast`
  - §7 Honest-Scope-Trennung in UX-Schicht (v0.7.7) vs Daten-Pfad-Schicht (v0.7.8)
  - §8 Code-Beispiel mit TS-Type-Fix (benannte Promises) + W5-Fallback (`bid_not_found`-Notice)
  - §9 zweite Entscheidungs-Tabelle fuer v1.2-Punkte
  - §11 Empfehlung verschoben: SimBrief-direct-by-username jetzt **v0.7.8** statt v0.8.x — wegen W5 fruher noetig
- **v1.3 (2026-05-11):** Nach 3. QS-Review von Thomas (Vorbereitungs-Kanten):
  - §6.1 v1.3-Korrektur Punkt 2: **`flight_id` als v0.7.7-Foundation** — `ActiveFlight` + `PersistedFlightStats` muessen `flight_id: String` aus `Bid.flight_id` extrahieren bevor `prefile_pirep` feuert (sonst spaeter verloren). Voraussetzung fuer v0.7.8 (Variante A + B).
  - §7.1 Persistenz-Foundation um `flight_id` erweitert.
  - §8 `bid_not_found`-Notice-Text korrigiert: frueher empfohlener "Cockpit-Refresh nutzen" war falsch, weil Cockpit-Button denselben toten Pfad ruft. Neuer Text ist ehrlich dass pointer-basierter Pfad in diesem Zustand grundsaetzlich tot ist.
  - §11 `simbrief_username`-Feld explizit als **eigenes Settings-Feld** spezifiziert (nicht aus phpVMS-Profile uebernehmbar). Zwei Varianten B1 (AeroACARS-local) vs B2 (phpVMS-API-Erweiterung), Empfehlung B1.
  - §9 dritte Entscheidungs-Tabelle fuer v1.3-Punkte (Notice-Text, flight_id, simbrief_username, §12-Reihenfolge)
  - §12 chronologische Reihenfolge fixiert (v1.0 → v1.1 → v1.2 → v1.3 latest-last)
- **v1.4 (2026-05-11):** Nach 4. QS-Review von Thomas (Struktur-Placement + Snippet + Tests):
  - §6.1 v1.4-Korrektur Punkt 1: `flight_id` gehoert **top-level in `ActiveFlight` + `PersistedFlight`** (Sibling von `bid_id`), NICHT verschachtelt in `PersistedFlightStats` (= Telemetrie/FSM, nicht Identifier). v1.3-Vorschlag war falsch platziert.
  - §6.1 v1.4-Korrektur Punkt 2: `simbrief_ofp_generated_at`-Wording von "ISO-String" auf "raw SimBrief `time_generated` string" — Parser macht heute keine Format-Annahme.
  - §6.1 v1.4-Korrektur Punkt 3: Snippet nutzt `bid.flight_id.clone()` direkt (in `flight_start` ist `bid: Bid` nach `ok_or_else`, kein Option). Die `matching_bid.map(...)`-Form aus v1.3 stammte versehentlich aus dem Resume-/Adopt-Pfad.
  - §10 v1.4-Korrektur Punkt 4: Notice-Text aus §8 1:1 in Tests-Beschreibung uebernommen — kein "Cockpit-Refresh nutzen" mehr (war noch aus v1.2 stehengeblieben).
  - §10 NEU 4 Pflicht-Tests: `flight_start_persists_flight_id_before_prefile`, `resume_flight_preserves_flight_id`, `flight_refresh_simbrief_accepts_pushback`, `..._returns_phase_locked_in_takeoff_roll`. Phase-Gate-Boundary Pushback/TakeoffRoll explizit getestet.
  - §9 vierte Entscheidungs-Tabelle fuer v1.4-Punkte.
  - §3 Z.76 (Workflow-Schritt 10) klargestellt dass Cockpit-Refresh nur greift wenn der Bid noch existiert (= seltener pre-Prefile-Zustand).
