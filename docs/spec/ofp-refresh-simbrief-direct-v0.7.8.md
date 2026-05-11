# OFP-Refresh SimBrief-direct (v0.7.8 Datenpfad)

**Status:** Draft v1.0 for QS Review
**Stand:** 2026-05-11
**Trigger:** v0.7.7 (`608630e` auf main) loest nur die UX-Schicht. W5 (phpVMS-7 entfernt Bid nach Prefile) macht den pointer-basierten Daten-Pfad im Real-Boarding wirkungslos. Pilot kriegt eine ehrliche Notice — aber keine neuen Plan-Werte. Dieser Spec dokumentiert den **echten Daten-Pfad** der v0.7.7 abloest und beide Schichten **in einem Release** ausgeliefert werden.

> **Kern-Entscheidung Thomas (2026-05-11):** SimBrief-direct (Variante B aus dem Vorgaenger-Spec §11) wird umgesetzt. Begruendung: AeroACARS-internal, kein server-coordinated PAX-Studio-Deploy noetig, SimBrief ist ohnehin die Wahrheits-Quelle der OFP-Werte.

---

## 1. Release-Disziplin (zwingend)

**v0.7.7 (Commit `608630e`) darf NICHT als eigenstaendiger Release getagged werden.** Pilot wuerde sonst zurecht melden: *"Aktualisieren sagt nur, dass es nicht geht."*

Das **gemeinsame Release** enthaelt beide Schichten:
- **UX-Schicht** (v0.7.7-Foundation in `608630e`): Persistenz-Felder, Phase-Gate, Notices, `flight_id`, UI-Refresh-Trigger
- **Datenpfad-Schicht** (dieser Spec): SimBrief-direct ohne Bid-Abhaengigkeit

Tag/Release-Version wird im Bundle entschieden — aktuell als **v0.7.7** geplant da das die etablierte PENDING-Marke ist; ggf. v0.7.8 wenn Thomas das so haben moechte.

---

## 2. SimBrief-direct Datenfluss

```
Pilot regeneriert OFP auf simbrief.com
       │
       ▼ (kein Pilot-Klick auf PAX Studio noetig)
SimBrief speichert latest OFP fuer User X
       │
       ▼
[Pilot klickt "⟳ Aktualisieren" im AeroACARS-Bid-Tab]
       │
       ▼
AeroACARS liest simbrief_username aus Settings   ← v0.7.8 NEU
       │
       ▼
GET https://www.simbrief.com/api/xml.fetcher.php?username={username}
       │
       ▼
SimBrief liefert latest OFP (XML mit dpt/arr/callsign/etc.)
       │
       ▼
AeroACARS Flight-Match-Verifikation:
  - origin == ActiveFlight.dpt_airport?
  - destination == ActiveFlight.arr_airport?
  - (optional weicher) callsign-Match?
       │
       ▼
Match → planned_* ueberschreiben, simbrief_ofp_id aktualisieren,
        Notice ggf. "OFP unveraendert" wenn ID identisch
Mismatch → klare Notice mit Erklaerung
```

**Kritisch:** Pointer-Pfad (= `client.get_bids()` + Bid-Lookup) ist **nicht mehr Voraussetzung**. Bid darf weg sein — SimBrief-Username + Flight-Match reichen.

---

## 3. SimBrief API — was wir wissen

**Endpoint:** `https://www.simbrief.com/api/xml.fetcher.php?username={username}`

- **Authentifizierung:** Keine. Username ist soft-identifier; jeder kann jeden Username abfragen (= Pilot-Daten sind public-by-design).
- **Response:** Letzter OFP des Users (egal welcher Flug). XML-Format identisch zur public-by-ID-URL die `fetch_simbrief_ofp` schon parst.
- **Rate-Limit:** SimBrief hat soft-limits (typisch < 5/min/IP). Unproblematisch fuer Pilot-Klick-Workflow.
- **Failure-Modes:**
  - 404 / leerer Response → Username unbekannt
  - HTTP 5xx → SimBrief offline
  - Network-Error → Internet weg
  - Parse-Fehler → unerwartetes XML

**Verwendete OFP-XML-Felder fuer Flight-Match** (alle bereits im Parser, `api-client/lib.rs:1492+`):
- `<origin><icao_code>` → `ofp.ofp_origin_icao` (existing)
- `<destination><icao_code>` → `ofp.ofp_destination_icao` (existing)
- `<atc><callsign>` → `ofp.ofp_flight_number` (existing, callsign)

Parser muss NICHT erweitert werden — alle benoetigten Felder sind schon vorhanden.

---

## 4. Settings-Architektur

### 4.1 Storage-Modell

**Frontend (React/TS):**
- localStorage-Key `simbrief_username` (analog `auto_file`, `debug_mode`, `auto_start`)
- Settings-Panel: Text-Input + Hint "Optional — fuer OFP-Refresh ohne PAX Studio Sync"
- On mount + on save: `invoke("set_simbrief_username", { value })`

**Backend (Rust):**
- `AppState.simbrief_username: Mutex<Option<String>>` (analog zu `airports`-Cache, nicht persistent disk-side)
- Tauri-Commands:
  - `get_simbrief_username() -> Option<String>`
  - `set_simbrief_username(value: Option<String>) -> Result<(), UiError>`
- Persistenz: rein Frontend (localStorage). Bei App-Restart wird der Wert vom Frontend zurueck-gepusht (gleicher Mechanismus wie `set_minimize_to_tray`).

**Rationale (nicht disk-side persistieren in Backend):**
- Konsistenz mit bestehenden Settings (`auto_file` etc.)
- Pro VA-Setup: nutzt jeder Pilot eigenen Username — keine Inter-Pilot-Sharing-Logik noetig
- SimBrief-Username ist semi-public (steht im Profile-URL) — keine besondere Geheimhaltung noetig (anders als Passwords). Wuerde aber dennoch nicht in `tauri-store` mit klartext-Logs landen.

### 4.2 Settings-UI

In `SettingsPanel.tsx` neuer Block:

```tsx
<section className="settings-section">
  <h3>{t("settings.simbrief.title")}</h3>
  <label>
    {t("settings.simbrief.username_label")}
    <input
      type="text"
      value={simbriefUsername}
      onChange={(e) => setSimbriefUsername(e.target.value)}
      onBlur={() => persistSimbriefUsername(simbriefUsername.trim())}
      placeholder="z.B. thomaskant"
      autoComplete="off"
      spellCheck={false}
    />
  </label>
  <p className="settings-hint">{t("settings.simbrief.hint")}</p>
</section>
```

Hint-Text (DE):
> "Wenn dein SimBrief-Username hier eingetragen ist, kann AeroACARS einen neu generierten OFP direkt von simbrief.com holen — auch wenn der Bid in phpVMS schon entfernt wurde (= regulaerer Zustand waehrend Boarding). Ohne Username bleibt der OFP-Refresh auf den phpVMS-Pointer-Pfad beschraenkt, der nach Prefile typisch nicht mehr greift."

---

## 5. Pfad-Auswahl in `flight_refresh_simbrief`

Spec v1.4 §11 hat den Vorschlag — hier verfeinert:

```rust
async fn flight_refresh_simbrief(...) -> Result<SimBriefRefreshResult, UiError> {
    // 1. Phase-Gate (v0.7.7) — unveraendert
    // ... preflight/boarding/pushback/taxi_out check

    // 2. Snapshot active flight info (Lock + Drop)
    let (bid_id, current_phase, previous_ofp_id, flight_id, dpt, arr, flight_number) = {
        let guard = state.active_flight.lock()?;
        let f = guard.as_ref().ok_or(...)?;
        let s = f.stats.lock()?;
        (
            f.bid_id,
            s.phase,
            s.simbrief_ofp_id.clone(),
            f.flight_id.clone(),
            f.dpt_airport.clone(),
            f.arr_airport.clone(),
            f.flight_number.clone(),
        )
    };

    // 3. SimBrief-Username lesen (Lock + Drop)
    let username = {
        let guard = state.simbrief_username.lock()?;
        guard.clone()
    };

    // 4. Pfad-Auswahl
    let (sb_id, ofp) = if let Some(u) = username.filter(|u| !u.trim().is_empty()) {
        // Pfad A: SimBrief-direct (Variante B aus Spec v1.4 §11)
        match fetch_and_verify_simbrief_direct(
            &state, &u, &dpt, &arr, &flight_number,
        ).await {
            Ok(Some(result)) => result,
            Ok(None) => {
                // Username gesetzt, aber kein Match → klare Fehler-Notice.
                // Frontend bekommt das als spezifischer Error-Code damit
                // der Pilot weiss "Username war ok, aber OFP passte nicht
                // zum aktuellen Flug".
                return Err(UiError::new(
                    "ofp_does_not_match_active_flight",
                    "Latest SimBrief OFP belongs to a different flight \
                     ({origin} → {dest} / {callsign}). Please regenerate \
                     the OFP for the current flight on simbrief.com.",
                ));
            }
            Err(e) => {
                // SimBrief offline / Username unknown / Parse-Fehler.
                // Wir fallen zurueck auf Pointer-Pfad — Pilot kriegt
                // damit zumindest eine Chance falls der Bid noch da ist.
                tracing::warn!(error = ?e, "SimBrief-direct fetch failed, falling back to pointer path");
                fetch_via_pointer_path(client, bid_id).await?
            }
        }
    } else {
        // Pfad B: Kein Username gesetzt → bestehender Pointer-Pfad
        fetch_via_pointer_path(client, bid_id).await?
    };

    // 5. ... rest wie v0.7.7 (changed-Flag, planned_* ueberschreiben,
    //     simbrief_ofp_id aktualisieren, Activity-Log, Return-DTO)
}
```

**Wichtig:**
- **Username gesetzt + Match-OK** → SimBrief-direct gewinnt, Pointer-Pfad wird NICHT versucht
- **Username gesetzt + Mismatch** → klare Fehler-Notice (kein Fallback zu Pointer — Pilot soll Bewusstsein darueber haben)
- **Username gesetzt + SimBrief offline/unbekannt** → SOFT-Fallback zu Pointer-Pfad mit Warnung im Activity-Log
- **Kein Username** → bestehender Pointer-Pfad (v0.7.7 Verhalten) — Backward-Compat

---

## 6. Flight-Match-Verifikation

### 6.1 Match-Regeln

```rust
fn ofp_matches_active_flight(
    ofp: &SimBriefOfp,
    active_dpt: &str,
    active_arr: &str,
    active_flight_number: &str,
) -> bool {
    // Origin / Destination MUESSEN matchen (case-insensitive).
    let dpt_ok = ofp.ofp_origin_icao
        .eq_ignore_ascii_case(active_dpt.trim());
    let arr_ok = ofp.ofp_destination_icao
        .eq_ignore_ascii_case(active_arr.trim());

    // Callsign-Match ist SOFT: SimBrief Callsign vs phpVMS
    // flight_number koennen wegen "DLH100" vs "100" abweichen.
    // Match akzeptieren wenn entweder identisch (case-insensitive)
    // ODER ein Suffix-Match (z.B. "100" ist Suffix von "DLH100").
    // Hauptanker bleiben dpt+arr.
    let cs_a = ofp.ofp_flight_number.trim().to_ascii_uppercase();
    let cs_b = active_flight_number.trim().to_ascii_uppercase();
    let cs_soft_ok = cs_a == cs_b
        || cs_a.ends_with(&cs_b)
        || cs_b.ends_with(&cs_a)
        || (cs_a.is_empty() || cs_b.is_empty()); // einer fehlt → tolerieren

    dpt_ok && arr_ok && cs_soft_ok
}
```

**Begruendung der Soft-Match-Logik fuer Callsign:**
- phpVMS-`flight_number` kann je VA-Konvention sein: "100", "DLH100", "GSG-100"
- SimBrief-OFP-Callsign ist was der Pilot in SimBrief eingetragen hat
- Beide muessen NICHT byte-identisch sein — Hauptsache dpt+arr passen UND Callsign ist plausibel verwandt
- Voll-Block: wenn Pilot offensichtlich einen anderen Flug regeneriert hat (z.B. "DLH200" statt "100"), faellt das beim Suffix-Match durch → Mismatch-Fehler

### 6.2 Generierungs-Zeit (Optional, NICHT in v0.7.8 Scope)

Spec v1.0/v1.1 hatte ueberlegt: "OFP-`generated_at` > flight_started_at" als zusaetzlichen Check. **Entscheidung v1.0-Spec:** weglassen — fuehrt zu Edge-Cases bei Pilot-Pre-Generierung vor Flight-Start. Match auf dpt/arr/callsign reicht.

---

## 7. Aufwand-Schaetzung

| Komponente | LOC |
|---|---|
| Backend: `AppState.simbrief_username: Mutex<Option<String>>` + 2 Commands | ~30 |
| Backend: `fetch_and_verify_simbrief_direct()` helper | ~50 |
| Backend: `ofp_matches_active_flight()` pure function + Tests | ~40 |
| Backend: `flight_refresh_simbrief` Pfad-Auswahl (refactor) | ~40 |
| Frontend: Settings-Panel SimBrief-Section | ~50 |
| Frontend: i18n DE/EN/IT (3 keys: title, label, hint) | ~15 |
| Frontend: BidsList neue Notice-Variante `ofp_does_not_match_active_flight` | ~10 |
| Frontend i18n fuer neue Notice | ~6 |
| Tests Backend: 6 Match-Tests + 3 Pfad-Auswahl-Tests | ~80 |

**Geschaetzt: ~320 LOC Diff**. Spec-konform, additiv zu v0.7.7, keine Breaking Changes.

---

## 8. Notice-Outcomes (Erweiterung der v0.7.7 §8-Tabelle)

| Outcome | Notice-Tone | Text (DE) |
|---|---|---|
| SimBrief-direct: OFP matched + changed=true | (kein Notice) | — |
| SimBrief-direct: OFP matched + changed=false | info | "OFP unveraendert. SimBrief liefert weiterhin OFP-ID {{id}}." |
| **SimBrief-direct: Mismatch** (NEU v0.7.8) | warn | "Aktueller SimBrief-OFP gehoert zu Flug {{origin}} → {{destination}} ({{callsign}}). Bitte fuer den aktiven Flug auf simbrief.com neu generieren." |
| SimBrief-direct: Username unbekannt → Fallback Pointer | warn | "SimBrief-Username '{{username}}' nicht gefunden. Pruefe Settings → SimBrief-Username." |
| Kein Username + Bid weg (W5) | warn | (existing v0.7.7) "Bid nicht mehr verfuegbar nach Prefile. Aktiviere SimBrief-direct in Settings fuer den Refresh-Pfad ohne Bid." (Hinweis-Text aktualisiert!) |

**v0.7.8 aktualisiert den v0.7.7 `bid_not_found`-Notice-Text** damit Pilot weiss wie er sich selbst helfen kann.

---

## 9. Akzeptanz an Real-Pilot-Workflows

### Workflow A: Pilot mit SimBrief-Username konfiguriert
1. Pilot regeneriert OFP auf simbrief.com (callsign passt)
2. Pilot klickt "Aktualisieren" im Bid-Tab
3. AeroACARS holt latest OFP direkt von SimBrief
4. Match → Plan-Werte aktualisiert, **kein Notice, Cockpit + Loadsheet zeigen sofort neue Werte**
5. Pilot ist happy

### Workflow B: Pilot mit SimBrief-Username konfiguriert, falscher OFP
1. Pilot regeneriert OFP fuer einen ANDEREN Flug (training run)
2. Pilot klickt "Aktualisieren" im AeroACARS Bid-Tab (= fuer den aktiven kommerziellen Flug)
3. AeroACARS holt latest OFP — Mismatch (anderer dpt/arr/callsign)
4. **Klare Notice:** "Aktueller SimBrief-OFP gehoert zu Flug X → Y (Z). Bitte fuer den aktiven Flug auf simbrief.com neu generieren."

### Workflow C: Pilot OHNE SimBrief-Username (= heutiges v0.7.7-Verhalten)
1. Pilot startet Flug, prefiled, Bid weg
2. Pilot klickt "Aktualisieren"
3. AeroACARS faellt auf Pointer-Pfad → `bid_not_found`
4. **v0.7.8-aktualisierte Notice:** "Bid nicht mehr verfuegbar nach Prefile. Aktiviere SimBrief-direct in Settings fuer den Refresh-Pfad ohne Bid."

### Workflow D: Pilot mit Username, SimBrief offline
1. AeroACARS versucht SimBrief-direct → Network-Error
2. SOFT-Fallback auf Pointer-Pfad
3. Wenn Bid noch da → Pointer-Pfad-Ergebnis (selten)
4. Wenn Bid weg → `bid_not_found`-Notice wie Workflow C

---

## 10. Test-Vorschlaege

Backend (Rust):

**Match-Verifikation:**
- `ofp_matches_active_flight_accepts_exact_match`
- `ofp_matches_active_flight_accepts_callsign_suffix_match` (z.B. "DLH100" vs "100")
- `ofp_matches_active_flight_accepts_case_insensitive`
- `ofp_matches_active_flight_rejects_wrong_dpt`
- `ofp_matches_active_flight_rejects_wrong_arr`
- `ofp_matches_active_flight_rejects_unrelated_callsign`
- `ofp_matches_active_flight_tolerates_empty_callsign`

**Pfad-Auswahl:**
- `flight_refresh_simbrief_uses_direct_when_username_set_and_match`
- `flight_refresh_simbrief_returns_mismatch_error_when_username_set_and_no_match`
- `flight_refresh_simbrief_falls_back_to_pointer_when_simbrief_offline`
- `flight_refresh_simbrief_uses_pointer_when_no_username`

**Settings:**
- `set_simbrief_username_persists_in_state`
- `get_simbrief_username_returns_none_when_unset`
- `simbrief_username_empty_string_treated_as_none`

Frontend (manueller Smoke):
- Settings-Tab: Username eingeben, App neu starten, Wert wieder da
- Bid-Tab-Refresh in Boarding mit Username gesetzt → neue Plan-Werte ohne Pointer
- Bid-Tab-Refresh mit falsch konfiguriertem Username → SOFT-Fallback funktioniert

---

## 11. Offene Punkte fuer Thomas-Review

- [ ] **Username-Validierung:** soll AeroACARS am `onBlur` einen Test-Fetch machen (= Pilot kriegt sofort Bestaetigung "ja, Username ist gueltig"), oder warten bis zum naechsten Refresh? Tendenz: bei Save mit Test-Fetch (= bessere UX).
- [ ] **Callsign-Match strictness:** der "Suffix-Match"-Ansatz (oben) toleriert "DLH100" vs "100". Reicht das, oder gibt es VA-Konventionen die noch toleranter sein muessen? z.B. "GSG-100" vs "GSG100" (mit Bindestrich)?
- [ ] **Mismatch-Verhalten:** Bei Mismatch HARD-Block (= Pilot muss regenerieren) oder SOFT-Confirm (= Pilot kann "trotzdem ueberschreiben" klicken)? Tendenz: HARD-Block fuer v0.7.8, weil falscher OFP = falscher Plan = falsche Loadsheet.
- [ ] **Settings-Tab-Platzierung:** als eigene Section "SimBrief Integration", oder unter "Allgemein"? Tendenz: eigene Section.
- [ ] **Test-Strategie:** SimBrief-API-Mocking in Tests — gibt es Pattern dafuer im Repo? Falls nicht, testen wir nur die pure-functions (Match) + integration-tests via env-flag manuell.

---

## 12. Versionierung dieser Spec

- **v1.0 (2026-05-11):** Initial Draft basierend auf Thomas-Decision "SimBrief-direct, big release bundle".
