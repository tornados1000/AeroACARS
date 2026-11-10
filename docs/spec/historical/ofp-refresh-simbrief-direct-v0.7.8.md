# OFP-Refresh SimBrief-direct (v0.7.8 Datenpfad)

**Status:** Draft v1.5 (Match-Logik erweitert um Pilot-Callsign-Varianten — Release-Blocker-Fix)
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
AeroACARS liest SimBrief-Identifier aus Settings ← v0.7.8 NEU
       │ (Username ODER User-ID, mind. eines)
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

## 3. SimBrief API — direkt verifiziert via Demo-Probe (v1.2)

**Endpoint:** `https://www.simbrief.com/api/xml.fetcher.php`

**Akzeptierte Query-Parameter** (verifiziert durch `?username=simbrief`-Probe):
| Parameter | Pflicht? | Bedeutung |
|---|---|---|
| `username` | optional | SimBrief-Profile-Name (z.B. "simbrief") |
| `userid` | optional | SimBrief-User-ID (numerisch) |
| `static_id` | optional | spezifischer OFP-Slot — wenn weglassen, kommt LATEST |

**Mindestens ein User-Identifier ist Pflicht** (entweder `username` ODER `userid`). `static_id` ist optional — ohne kommt der zuletzt generierte OFP des Users.

Navigraph's "offizielles" Pattern (`?userid=X&static_id=Y`) ist nur EINE Variante fuer punktgenaue OFP-Abfrage. Fuer unseren Use-Case (latest OFP fuer aktiven Flug) reicht `?username=X` oder `?userid=X` — bestaetigt durch Live-Probe.

### XML-Response-Struktur (direkt aus Probe)

```xml
<fetch>
  <userid>1</userid>
  <static_id></static_id>
  <status>Success</status>     ← v1.2 KEY: Error-Indikator
  <time>0.0042</time>
</fetch>
<params>
  <request_id>172403072</request_id>       ← v1.2: canonical changed-flag-Quelle
  <sequence_id>6963c3d8ce43</sequence_id>  ← v1.2 NEU entdeckt, derzeit ungenutzt
  <static_id/>                              ← KANN LEER sein, nicht verlassbar
  <user_id>1</user_id>
  <time_generated>1778461205</time_generated>
  <xml_file>https://www.simbrief.com/ofp/flightplans/xml/...</xml_file>
</params>
<origin>...</origin>
<destination>...</destination>
...
```

### Failure-Modes (v1.3 robust — beide Pfade abdecken)

Spec v1.1 (laut Navigraph-Doku): invalid user → HTTP 400 + XML-Error.
Spec v1.2 (laut Live-Probe `?username=simbrief`): HTTP 200 + `<fetch><status>Error</status>` moeglich.

**v1.3 Konsequenz:** Code muss **beide** Wege abdecken. Reihenfolge in der Pruefung:

| Detection | Bedeutung | Direct-Error-Variante |
|---|---|---|
| HTTP 400 | invalid user (Navigraph-Doku-Pfad) | `UserNotFound` |
| HTTP 5xx | SimBrief offline / maintenance | `Unavailable` |
| HTTP andere non-2xx | unerwartet | `Network` |
| HTTP 200 + `<fetch><status>` != "Success" | invalid user (Live-Probe-Pfad) | `UserNotFound` |
| HTTP 200 + Status Success + Parse-Fehler | unerwartetes XML | `ParseFailed` |
| Network/IO-Error vor Response | Internet weg | `Network` |
| HTTP 200 + Status Success + valid OFP | Erfolg | weitermachen |

Damit ist der Code robust gegen beide bekannten Failure-Pfade — und der `UserNotFound`-Code triggert die korrekte Notice ("SimBrief-Username/User-ID nicht gefunden — pruefe Settings").

### Identifier-Strategie v1.2

**Was wir tatsaechlich brauchen** (Vereinfachung gegenueber v1.1):
- **EINER der zwei** Pilot-Identifier muss in Settings stehen: `username` ODER `userid`
- **`static_id` brauchen wir NICHT** (= kann leer sein, Pilot-OFP-Slot-Konvention nicht zuverlaessig)
- **`request_id`** parsen wir aus dem Response — das ist die canonical changed-flag-Quelle

Damit ist die v1.0/v1.1-Ueberlegung "wir brauchen userid + static_id" **falsch**. Username-only Fetch reicht — wir verifizieren Match per dpt/arr/callsign (= §6).

### Was wir AeroACARS-seitig parsen muessen (v1.3-final)

**v1.3 QS-Entscheidung:** `static_id` kommt **NICHT** in den Parser, **NICHT** in `FlightStats`, **NICHT** in Settings. Begruendung (aus Navigraph-Forum-Thread): `static_id` ist fuer Systeme die OFPs selber **erzeugen** ueber die API. AeroACARS fetcht nur — wir brauchen keinen Slot-Pointer.

`SimBriefOfp` bekommt nur **ein** neues Feld:

```rust
pub struct SimBriefOfp {
    // ... bestehende Felder ...
    /// v0.7.8 (v1.3): `<params><request_id>`. Aendert sich bei JEDER
    /// Re-Generation auf simbrief.com — canonical changed-flag-Quelle.
    /// Leer wenn Tag fehlt (sollte praktisch nie passieren laut Demo-Probe).
    pub request_id: String,
}
```

`sequence_id` wird parser-seitig **ignoriert** (optional `tracing::debug!` mitloggen fuer evtl. spaetere Diagnose, aber NICHT in DTO/Persistenz).

**v1.3 Robust-Error-Erkennung (QS-Punkt 4):** SimBrief liefert Fehler auf **zwei Wegen** — der Parser muss beide behandeln:

```rust
// 1. HTTP-Status pruefen (Navigraph-Doku sagt invalid user → HTTP 400)
if response.status() == StatusCode::BAD_REQUEST {
    return Err(SimBriefDirectError::UserNotFound);
}
if response.status().is_server_error() {
    return Err(SimBriefDirectError::Unavailable);
}
if !response.status().is_success() {
    return Err(SimBriefDirectError::Network);  // unerwarteter Code
}

let xml = response.text().await
    .map_err(|_| SimBriefDirectError::Network)?;

// 2. XML-Status pruefen (Live-Probe sah HTTP 200 + <fetch><status>Error</status>)
let fetch_status = extract_tag(&xml, "fetch")
    .and_then(|inner| extract_tag(inner, "status"))
    .map(|s| s.trim().to_string())
    .unwrap_or_default();
if fetch_status != "Success" {
    return Err(SimBriefDirectError::UserNotFound);
}

// 3. Erst jetzt: OFP-Felder parsen
let ofp = parse_simbrief_ofp(&xml)
    .ok_or(SimBriefDirectError::ParseFailed)?;
```

Damit faengt der Code beide moegliche Failure-Pfade ab — Navigraph-Doku-Pfad UND Live-Probe-Pfad.

**v1.3 Hinweis zu `request_id` vs `bid.simbrief.id`:** Es ist NICHT garantiert dass beide identisch sind:
- `bid.simbrief.id` aus phpVMS = was PAX Studio dort hineingeschrieben hat
- SimBrief-direct `<params><request_id>` = was SimBrief direkt liefert

Resultat: Beim **ersten** Wechsel von Pointer-Pfad zu Direct-Pfad **kann** `changed=true` triggern, auch wenn der Plan inhaltlich identisch ist. Danach (= alle nachfolgenden Direct-Refreshes) ist es stabil, weil AeroACARS persistierte ID = `request_id` vom letzten Refresh. Akzeptabel — Pilot sieht "OFP wurde aus Direct-Pfad neu geladen" und das ist informativ, nicht falsch.

**Verwendete OFP-XML-Felder** (Parser-Stand `api-client/lib.rs:1492+`):
- `<origin><icao_code>` → `ofp.ofp_origin_icao` (existing)
- `<destination><icao_code>` → `ofp.ofp_destination_icao` (existing)
- `<atc><callsign>` → `ofp.ofp_flight_number` (existing, callsign)
- `<params><time_generated>` → `ofp.ofp_generated_at` (existing)

**v1.1-Korrektur Punkt 1 (P1):** Parser MUSS erweitert werden um `<params><request_id>`:

```rust
// api-client/lib.rs: SimBriefOfp struct
pub struct SimBriefOfp {
    // ... bestehende Felder ...
    /// v0.7.8: SimBrief-OFP-ID aus <params><request_id>. Aendert sich
    /// bei jeder Re-Generation auf simbrief.com. Brauchen wir fuer
    /// den changed-Flag-Vergleich (`current_ofp_id` im
    /// `SimBriefRefreshResult`) und um die `simbrief_ofp_id` in
    /// `FlightStats` zu setzen.
    /// Leerer String wenn das Tag fehlt (sollte nicht passieren).
    pub ofp_id: String,
}
```

Parser-Erweiterung in `parse_simbrief_ofp()`:

```rust
let ofp_id = extract_tag(xml, "params")
    .and_then(|inner| extract_tag(inner, "request_id"))
    .map(|s| s.trim().to_string())
    .unwrap_or_default();
```

**Wichtig:** Spec v1.0 sagte "Parser muss NICHT erweitert werden" — das war **falsch**. Ohne `request_id` haetten wir keinen Vergleichs-Anker fuer den changed-Flag.

**v1.1-Korrektur Punkt 5 (P2 — SimBrief-Failure-Codes):** Laut Navigraph Developer Portal (`developers.navigraph.com/docs/simbrief/fetching-ofp-data`) liefert SimBrief bei invalid user / fetch-error **HTTP 400 mit kleinem XML-Error-Body**, nicht primaer 404 oder leerer Response. Failure-Mode-Liste daher korrigiert:

| HTTP Status | Body | Bedeutung | Handling |
|---|---|---|---|
| 200 + valid OFP XML | OFP-Plan | Erfolg | parse + return |
| 400 + small XML error | `<OFP><fetch><status>Error: ...</status></fetch></OFP>` | invalid user / fetch error | spezifischer "user_not_found"-Error |
| 5xx | (variabel) | SimBrief offline / maintenance | "simbrief_unavailable"-Error |
| Network-Error | — | Internet weg | "network_error" |
| 200 + Parse-Fehler | irgendwas was nicht parsed | Unerwartetes XML | "ofp_parse_failed" |

Diese Differenzierung speist in §5 Pfad-Auswahl (Fehler-Priorisierung) und §8 Notice-Tabelle ein.

---

## 4. Settings-Architektur

### 4.1 SB-Identifier: Username oder User-ID? (v1.1 Klaerung)

**SimBrief unterstuetzt beide Identifier-Typen** (Quelle: Navigraph Developer Portal):
- `xml.fetcher.php?username={username}` — z.B. "thomaskant"
- `xml.fetcher.php?userid={numeric_id}` — z.B. "612345"

Eigenschaften im Vergleich:

| Aspekt | Username | User-ID |
|---|---|---|
| Wo zu finden | SimBrief Profile-URL (sichtbar) | SimBrief Account Settings (versteckter) |
| Stabilitaet | aenderbar (selten, aber moeglich) | unveraenderlich |
| Lesbarkeit fuer Pilot | hoch ("thomaskant") | gering ("612345") |
| Robustheit fuer Tool | gut | besser |
| URL-Encoding noetig | ja (kann Sonderzeichen) | nein (nur Ziffern) |

**Entscheidung v1.1 + v1.2-Bestaetigung:** Zwei separate Felder. Pilot muss **mindestens eines** ausfuellen. Wenn beide gefuellt → User-ID hat Vorrang (robuster, unveraenderlich).

```rust
// AppState — beides separat persistiert
pub struct SimBriefSettings {
    pub username: Option<String>,  // z.B. "thomaskant"
    pub user_id:  Option<String>,  // z.B. "612345" (numerisch als String)
}
```

**v1.2-Bestaetigung:** Live-API-Probe `?username=simbrief` lieferte `<status>Success</status>` mit vollem OFP-XML. **Username-only Fetch ist also valide** — kein `static_id` noetig fuer Latest-OFP-Use-Case.

URL-Aufbau zur Laufzeit:
```rust
let url = match (&settings.user_id, &settings.username) {
    (Some(uid), _) if !uid.is_empty() => format!(
        "https://www.simbrief.com/api/xml.fetcher.php?userid={}",
        urlencoding_escape(uid),
    ),
    (_, Some(un)) if !un.is_empty() => format!(
        "https://www.simbrief.com/api/xml.fetcher.php?username={}",
        urlencoding_escape(un),
    ),
    _ => return Err(SimBriefDirectError::NoIdentifier),
};
```

URL-Encoding via `urlencoding_escape` (= bestehendes Pattern in `api-client/lib.rs:1152`).

### 4.2 Storage-Modell

**Frontend (React/TS):**
- 2 localStorage-Keys: `simbrief_username` + `simbrief_user_id`
- Settings-Panel: 2 Text-Inputs + "Prüfen"-Button (siehe §4.4)

**Backend (Rust):**
- `AppState.simbrief_settings: Mutex<SimBriefSettings>`
- Tauri-Commands:
  - `get_simbrief_settings() -> SimBriefSettings`
  - `set_simbrief_settings(username: Option<String>, user_id: Option<String>) -> Result<(), UiError>`
- Persistenz: rein Frontend (localStorage). Beim App-Start wird zurueck-gepusht.

**v1.1-Korrektur Punkt 4 (P2 — App-root Sync):** Spec v1.0 sagte "On mount + on save invoken". Das ist **nicht ausreichend** — wenn der Pilot Settings nach App-Restart nicht oeffnet, bleibt das Backend leer und der Refresh nutzt unverschuldet den Pointer-Pfad.

Korrektur: **`App.tsx` lest localStorage beim Login (oder app-mount) einmal und pusht zurueck**. Pattern:

```tsx
// App.tsx — direkt nach erfolgreichem Login
useEffect(() => {
  if (status.kind !== "loggedIn") return;
  const username = localStorage.getItem("simbrief_username") ?? null;
  const userId = localStorage.getItem("simbrief_user_id") ?? null;
  if (username || userId) {
    void invoke("set_simbrief_settings", {
      username: username || null,
      userId: userId || null,
    }).catch(() => null);
  }
}, [status.kind]);
```

Damit ist Backend sofort nach Login synchron mit dem letzten gespeicherten Wert — auch wenn Pilot Settings nie oeffnet.

### 4.3 Rationale (nicht disk-side persistieren in Backend)
- Konsistenz mit bestehenden Settings (`auto_file` etc.)
- Pro VA-Setup: nutzt jeder Pilot eigene Identifier — keine Inter-Pilot-Sharing-Logik noetig
- SimBrief-Identifier sind semi-public (Username im Profile-URL) — keine besondere Geheimhaltung noetig
- Persistenz via localStorage vermeidet `tauri-store`-Klartext-Logs

### 4.4 Settings-UI (eigene Section, v1.1 Entscheidung)

In `SettingsPanel.tsx` **eigene Section** "SimBrief Integration" (= nicht unter "Allgemein"):

```tsx
<section className="settings-section">
  <h3>{t("settings.simbrief.title")}</h3>
  <p className="settings-hint settings-hint--intro">
    {t("settings.simbrief.intro")}
  </p>

  <label className="settings-row">
    <span>{t("settings.simbrief.username_label")}</span>
    <input
      type="text"
      value={username}
      onChange={(e) => setUsername(e.target.value)}
      onBlur={() => persist({ username: username.trim() || null, user_id: userId.trim() || null })}
      placeholder="z.B. thomaskant"
      autoComplete="off"
      spellCheck={false}
    />
    <small>{t("settings.simbrief.username_hint")}</small>
  </label>

  <label className="settings-row">
    <span>{t("settings.simbrief.userid_label")}</span>
    <input
      type="text"
      inputMode="numeric"
      value={userId}
      onChange={(e) => setUserId(e.target.value.replace(/[^0-9]/g, ""))}
      onBlur={() => persist({ username: username.trim() || null, user_id: userId.trim() || null })}
      placeholder="z.B. 612345"
      autoComplete="off"
      spellCheck={false}
    />
    <small>{t("settings.simbrief.userid_hint")}</small>
  </label>

  <div className="settings-row settings-row--actions">
    <button
      type="button"
      onClick={handleVerify}
      disabled={verifying || (!username.trim() && !userId.trim())}
    >
      {verifying ? "…" : t("settings.simbrief.verify_button")}
    </button>
    {verifyStatus && (
      <span className={`settings-verify-status settings-verify-status--${verifyStatus.tone}`}>
        {verifyStatus.icon} {verifyStatus.text}
      </span>
    )}
  </div>
</section>
```

**v1.1 Username-Validierung (P2-Entscheidung — kein hartes onBlur-Fetch):**
- "Prüfen"-Button macht den Test-Fetch (= ein expliziter Pilot-Klick statt jedem Tippen)
- Status-Anzeige darunter: `✓ Username 'thomaskant' gefunden` oder `⚠ Kein Profil`
- onBlur persistiert nur (kein Netz-Request)
- Persist beim Tippen ist OK — Verbindungs-Test ist separate Aktion

Hint-Texte (DE):
- `settings.simbrief.title`: "SimBrief Integration"
- `settings.simbrief.intro`: "Wenn dein SimBrief-Identifier hier eingetragen ist, kann AeroACARS einen neu generierten OFP direkt von simbrief.com holen — auch wenn der Bid in phpVMS schon entfernt wurde (regulaerer Zustand waehrend Boarding). Du kannst Username oder User-ID nutzen (oder beides). User-ID ist robuster, Username einfacher zu finden."
- `settings.simbrief.username_label`: "SimBrief-Username"
- `settings.simbrief.username_hint`: "Sichtbar in simbrief.com/dashboard/?username=..."
- `settings.simbrief.userid_label`: "SimBrief-User-ID (optional)"
- `settings.simbrief.userid_hint`: "Aus SimBrief Account Settings, rein numerisch"
- `settings.simbrief.verify_button`: "Verbindung pruefen"

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
        let guard = state.simbrief_settings.lock()?;
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
- **Identifier gesetzt + Match-OK** → SimBrief-direct gewinnt, Pointer-Pfad wird NICHT versucht
- **Identifier gesetzt + Mismatch** → klare Fehler-Notice (HARD-Block per v1.1 Entscheidung, kein "trotzdem ueberschreiben")
- **Identifier gesetzt + SimBrief offline/unbekannt** → SOFT-Fallback zu Pointer-Pfad. **Direct-Fehler muss gemerkt werden** und in Notice priorisiert werden falls Pointer auch scheitert (v1.1 P1-2-Korrektur).
- **Kein Identifier** → bestehender Pointer-Pfad (v0.7.7 Verhalten) — Backward-Compat

### 5.1 v1.1 P1-2 Korrektur: Fehler-Priorisierung im Fallback

Spec v1.0-Pseudocode hatte `fetch_via_pointer_path(...)?` — bei Bid-weg-Szenario hat der den Direct-Fehler ueberschrieben mit `bid_not_found`, sodass der Pilot nicht wusste dass sein **Username falsch konfiguriert** war.

Korrektur — Direct-Fehler explizit tracken und composite Notice ausgeben:

```rust
async fn flight_refresh_simbrief(...) -> Result<SimBriefRefreshResult, UiError> {
    // ... Phase-Gate + Snapshot wie v1.0 ...

    let settings = state.simbrief_settings.lock().clone();
    let has_identifier = settings.username.is_some() || settings.user_id.is_some();

    if has_identifier {
        // Pfad A: SimBrief-direct
        match fetch_and_verify_simbrief_direct(
            &settings, &dpt, &arr, &airline_icao, &flight_number,
        ).await {
            Ok(DirectOutcome::Match { sb_id, ofp }) => {
                // Erfolg → wir verlassen den Direct-Pfad mit dem Match.
                proceed_with_ofp(sb_id, ofp).await
            }
            Ok(DirectOutcome::Mismatch { simbrief_origin, simbrief_dest, simbrief_callsign }) => {
                // HARD-Block per v1.1-Entscheidung — kein Fallback.
                Err(UiError::new(
                    "ofp_does_not_match_active_flight",
                    format!("Latest SimBrief OFP belongs to {} → {} ({}). \
                             Please regenerate the OFP for the current flight on simbrief.com.",
                             simbrief_origin, simbrief_dest, simbrief_callsign),
                ))
            }
            Err(direct_err) => {
                // SOFT-Fallback zu Pointer-Pfad, ABER Direct-Error merken.
                tracing::warn!(error = ?direct_err, "SimBrief-direct fetch failed, attempting pointer fallback");
                match fetch_via_pointer_path(client, bid_id).await {
                    Ok((sb_id, ofp)) => proceed_with_ofp(sb_id, ofp).await,
                    Err(pointer_err) => {
                        // Beide Pfade tot — composite Notice:
                        // Direct-Fehler priorisieren (= actionable fuer Pilot).
                        Err(compose_failure(direct_err, pointer_err))
                    }
                }
            }
        }
    } else {
        // Pfad B: kein Identifier → nur Pointer
        let (sb_id, ofp) = fetch_via_pointer_path(client, bid_id).await?;
        proceed_with_ofp(sb_id, ofp).await
    }
}

/// v1.1: composite Failure mit Direct-Priorisierung. Pilot soll wissen
/// wenn die Direct-Konfiguration (Username/UserID) der Grund ist, dass
/// Refresh nicht klappt — nicht nur "Bid weg" als irrefuehrender
/// Sekundaer-Effekt.
fn compose_failure(direct: SimBriefDirectError, pointer: UiError) -> UiError {
    match direct {
        SimBriefDirectError::UserNotFound => UiError::new(
            "simbrief_user_not_found",
            "SimBrief-Username/User-ID nicht gefunden. Pruefe Settings → SimBrief Integration.",
        ),
        SimBriefDirectError::Unavailable => UiError::new(
            "simbrief_unavailable_and_bid_gone",
            "SimBrief gerade nicht erreichbar UND Bid ist nach Prefile weg. \
             Versuche es in ein paar Minuten erneut.",
        ),
        SimBriefDirectError::ParseFailed | SimBriefDirectError::Network => UiError::new(
            "simbrief_direct_failed",
            format!("SimBrief-direct schlug fehl ({:?}). Pointer-Pfad zusaetzlich: {}",
                    direct, pointer.message),
        ),
    }
}
```

Damit ist die Notice-Hierarchie:
1. Direct-Fehler ist primaer → "Username falsch" beats "Bid weg"
2. Pilot weiss sofort wo das Problem sitzt (Settings vs server-side)

---

## 6. Flight-Match-Verifikation

### 6.1 Match-Regeln (v1.1 P1-3 verschaerft)

**Problem mit v1.0 Suffix-Match:** `DLH1100` endet auch auf `100` → false-positive Match wenn Pilot zwischendurch einen anderen Flug (mit ueberlapenden Suffix-Ziffern) regeneriert hat.

**v1.1 Loesung: Normalisierter Airline+Number-Vergleich.**

AeroACARS hat `airline_icao` UND `flight_number` als getrennte Felder in `ActiveFlight` — das ist die saubere Quelle. Wir konstruieren beide Seiten zur Vergleichs-Form:

```rust
/// v1.1: Normalisiert Callsign-Strings auf ein vergleichbares Format.
/// Entfernt Whitespace + Bindestrich + Underscore, uppercase.
/// "DLH-100" → "DLH100", "GSG 100" → "GSG100", "dlh100" → "DLH100".
fn normalize_callsign(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && *c != '-' && *c != '_')
        .collect::<String>()
        .to_ascii_uppercase()
}

/// v1.1: Trennt eine normalisierte Callsign-Form in Airline-Prefix (alpha)
/// und Flight-Number (numeric/alphanumeric Rest). Liefert (prefix, number).
/// "DLH100" → ("DLH", "100"), "100" → ("", "100"), "GSG100A" → ("GSG", "100A").
fn split_callsign(cs: &str) -> (String, String) {
    let split_at = cs.find(|c: char| c.is_ascii_digit()).unwrap_or(cs.len());
    let (prefix, number) = cs.split_at(split_at);
    (prefix.to_string(), number.to_string())
}

fn ofp_matches_active_flight(
    ofp: &SimBriefOfp,
    active_dpt: &str,
    active_arr: &str,
    active_airline_icao: &str,
    active_flight_number: &str,
) -> bool {
    // 1. Origin / Destination MUESSEN matchen (case-insensitive).
    let dpt_ok = ofp.ofp_origin_icao
        .trim()
        .eq_ignore_ascii_case(active_dpt.trim());
    let arr_ok = ofp.ofp_destination_icao
        .trim()
        .eq_ignore_ascii_case(active_arr.trim());
    if !dpt_ok || !arr_ok {
        return false;
    }

    // 2. Callsign-Match: AeroACARS hat airline_icao + flight_number als
    //    getrennte Felder. Wir bauen daraus die kanonische Form, und
    //    vergleichen mit dem SimBrief-OFP-Callsign nach Normalisierung.
    //
    //    Variante A: SimBrief enthaelt das volle Callsign "DLH100"
    //                → wir konstruieren "DLH" + "100" = "DLH100" und vergleichen exakt.
    //    Variante B: SimBrief enthaelt nur die Nummer "100"
    //                (kann passieren je nach Pilot-Profile-Setup auf simbrief.com)
    //                → wir vergleichen NUR die Number-Part.
    //
    //    KEINE blinde ends_with-Logik → kein "DLH1100" matched "100"-Fehler.
    let active_full = format!("{}{}", active_airline_icao.trim(), active_flight_number.trim());
    let active_norm = normalize_callsign(&active_full);
    let (_, active_number) = split_callsign(&active_norm);

    let simbrief_norm = normalize_callsign(&ofp.ofp_flight_number);

    if simbrief_norm.is_empty() {
        // SimBrief liefert kein Callsign → toleranter Mode: dpt+arr genuegen.
        // Real selten — SimBrief-OFP traegt typisch immer einen Callsign.
        return true;
    }

    let (simbrief_prefix, simbrief_number) = split_callsign(&simbrief_norm);

    if simbrief_prefix.is_empty() {
        // SimBrief hat NUR die Number (z.B. "100") → mit aktiver Number vergleichen.
        return simbrief_number == active_number;
    }

    // SimBrief hat full callsign mit Prefix → exakter Vergleich mit
    // konstruiertem Aktiv-Callsign.
    simbrief_norm == active_norm
}
```

**Was die Regel jetzt richtig macht:**
- `active="DLH100"` vs simbrief `"DLH100"` → match (exakt)
- `active="DLH100"` vs simbrief `"100"` → match (number-only-mode)
- `active="DLH100"` vs simbrief `"DLH1100"` → **MISMATCH** (number "100" != "1100")
- `active="DLH100"` vs simbrief `"DLH200"` → **MISMATCH** (number "100" != "200")
- `active="DLH-100"` vs simbrief `"DLH100"` → match (Bindestrich wird normalisiert)
- `active="GSG100A"` vs simbrief `"GSG100A"` → match

**Begruendung gegen v1.0 Suffix-Match:** Die Suffix-Logik (`ends_with`) konnte Pilot-Fehler verschleiern. Mit der neuen Logik gibt es eine klare Mismatch-Notice ("OFP gehoert zu DLH1100, aktiv ist DLH100"), und der Pilot regeneriert sauber.

**Aktiv-Callsign-Quelle:** `ActiveFlight.airline_icao + flight_number` (beide bereits in v0.7.7-State). Bei leerem `airline_icao` (= kein Airline-Match in phpVMS) faellt der Vergleich auf reinen Number-Part zurueck.

### 6.1.1 v1.5: Pilot-Callsign-Varianten (Release-Blocker-Fix)

**Problem (QS Thomas 2026-05-11):** Real-Pilot-Beispiel von GSG:

| Quelle | Wert |
|---|---|
| phpVMS-Bid `airline_icao` | "CFG" |
| phpVMS-Bid `flight_number` | "1504" (= Original-Condor-Flug) |
| phpVMS-Profile `callsign` | "4TK" (= Pilot-personalisiert) |
| SimBrief-OFP `<atc><callsign>` | "CFG4TK" (= Airline + Pilot-Callsign — was viele VAs operativ als ATC-Callsign nutzen) |

Mit der v1.1-Logik (nur `airline_icao + flight_number` als Match-Quelle):
- active_full = "CFG1504"
- simbrief_norm = "CFG4TK"
- "CFG4TK" != "CFG1504" → **MISMATCH → HARD-Block**

Pilot kriegt zurecht die Notice "OFP gehoert zu anderem Flug", obwohl der OFP **fachlich legitim** ist. Das ist ein Release-Blocker fuer v0.7.8 — VAs mit Pilot-personalisierten Callsigns haetten den Refresh nie funktionierend.

### 6.1.2 v1.5 Loesung: Kandidaten-Liste statt strenger Match

`ofp_matches_active_flight` baut eine **Liste valider Callsign-Formen** auf — der SimBrief-Callsign muss gegen EINE davon exakt matchen:

```rust
fn build_candidate_callsigns(
    airline_icao: &str,
    flight_number: &str,
    bid_callsign: Option<&str>,
    pilot_callsign: Option<&str>,
) -> Vec<String> {
    let airline = airline_icao.trim().to_ascii_uppercase();
    let flight_n = flight_number.trim();
    let mut candidates: Vec<String> = Vec::new();

    // (1) airline_icao + flight_number — bestehend
    if !airline.is_empty() && !flight_n.is_empty() {
        candidates.push(normalize_callsign(&format!("{}{}", airline, flight_n)));
    }
    // (2) flight_number allein — bestehend (number-only)
    if !flight_n.is_empty() {
        candidates.push(normalize_callsign(flight_n));
    }
    // (3) v1.5: bid.flight.callsign direkt (= z.B. "CFG4TK" wenn
    //     phpVMS-VA das so konfiguriert hat). Robust gegen jede VA-
    //     Konvention.
    if let Some(cs) = bid_callsign.filter(|s| !s.trim().is_empty()) {
        candidates.push(normalize_callsign(cs));
    }
    // (4) v1.5: airline + profile.callsign (= "CFG4TK"-Pattern wenn
    //     Pilot persoenliches Suffix nutzt aber phpVMS callsign-Feld
    //     leer ist).
    if let Some(pc) = pilot_callsign.filter(|s| !s.trim().is_empty()) {
        if !airline.is_empty() {
            candidates.push(normalize_callsign(&format!("{}{}", airline, pc)));
        }
        // (5) v1.5: profile.callsign allein als number-only-Fallback.
        candidates.push(normalize_callsign(pc));
    }

    candidates.into_iter().filter(|s| !s.is_empty()).collect()
}
```

Match-Funktion vergleicht den SimBrief-Callsign (normalisiert) gegen alle Kandidaten:

```rust
let simbrief_norm = normalize_callsign(ofp_callsign);
let candidates = build_candidate_callsigns(...);
// Match wenn ein Kandidat exakt passt:
candidates.contains(&simbrief_norm)
// Oder fuer number-only SimBrief-Callsign (= ohne Airline-Prefix):
// vergleiche gegen Number-Parts der Kandidaten
```

**Sicherheits-Anker bleibt:** `dpt + arr` muessen IMMER matchen. Auch wenn Callsign-Liste weicher ist, kann der OFP nicht zu einem anderen Flugziel gehoeren.

### 6.1.3 v1.5 Edge-Case-Tabelle

Pilot-Bid: `airline=CFG, flight_number=1504, profile.callsign=4TK, bid.flight.callsign=CFG4TK`

| SimBrief-Callsign | v1.1 Resultat | v1.5 Resultat | Begruendung |
|---|---|---|---|
| `"CFG1504"` | ✓ match | ✓ match (Kandidat 1) | airline+flight_number |
| `"1504"` | ✓ match | ✓ match (Kandidat 2) | number-only |
| `"CFG4TK"` | ✗ MISMATCH | ✓ **match (Kandidat 3 oder 4)** | bid.flight.callsign ODER airline+pilot_callsign |
| `"4TK"` | ✗ MISMATCH | ✓ **match (Kandidat 5)** | pilot_callsign allein |
| `"CFG1100"` | ✗ MISMATCH | ✗ MISMATCH | Number weicht ab, kein Pilot-Callsign-Match |
| `"ABC4TK"` | ✗ MISMATCH | ✗ MISMATCH | Andere Airline, kein Match in Kandidatenliste |
| `"CFG200"` | ✗ MISMATCH | ✗ MISMATCH | Number weicht ab, andere Flug |
| `"DLH1100"` vs active "DLH100" | ✗ MISMATCH (P1-3) | ✗ MISMATCH | Regression-Guard bleibt |

**v1.5-Spec verhindert NICHT den v1.1 P1-3 DLH1100-vs-100-Schutz** — Kandidatenliste ist additiv, exakter Vergleich bleibt der Kern.

### 6.1.4 v1.5 Empirische Klaerung (Thomas-Hinweis)

Es ist nicht sicher ob phpVMS-`/api/user/bids` bei GSG-Pilot-Bids das `flight.callsign`-Feld bereits mit "4TK"/"CFG4TK" befuellt — oder ob es leer/anders ist (z.B. "CFG1504" weil phpVMS die Bid-Flight-Number copy-pasted hat).

**Pragmatischer Ansatz v1.5:** AeroACARS liest **beide** Quellen additiv:
- `Flight.callsign` aus dem Bid (existiert bereits im Parser, `api-client/lib.rs:370`)
- `Profile.callsign` aus dem User-Profile (NEU — Spec v1.5 fuegt das Feld hinzu)

Damit sind wir robust unabhaengig davon was phpVMS tatsaechlich liefert. Wenn beide leer bleiben, fallen wir zurueck auf die v1.1-Logik (= airline+flight_number-Match). Wenn entweder/beide gefuellt, kommen sie als Kandidaten dazu.

Optional empirische Verifikation per Pilot-Test (`https://german-sky-group.eu/api/user/bids` im Browser eingeloggt aufrufen, `bid.flight.callsign`-Feld inspizieren) — bestaetigt welches Feld bei GSG befuellt ist. Nicht-blockierend, da Code beide Felder additiv liest.

### 6.2 Generierungs-Zeit (Optional, NICHT in v0.7.8 Scope)

Spec v1.0/v1.1 hatte ueberlegt: "OFP-`generated_at` > flight_started_at" als zusaetzlichen Check. **Entscheidung v1.0-Spec:** weglassen — fuehrt zu Edge-Cases bei Pilot-Pre-Generierung vor Flight-Start. Match auf dpt/arr/callsign reicht.

---

## 7. Aufwand-Schaetzung (v1.4 realistischer)

| Komponente | LOC |
|---|---|
| Backend: `SimBriefSettings` struct + `AppState.simbrief_settings: Mutex<SimBriefSettings>` + 2 Tauri-Commands | ~40 |
| Backend: `SimBriefOfp.request_id` Parser-Erweiterung + 2 Tests | ~20 |
| Backend: `SimBriefDirectError`-Enum (4 Varianten) + Error-Mapping (HTTP-Code + `<fetch><status>`) | ~50 |
| Backend: `fetch_and_verify_simbrief_direct()` helper | ~70 |
| Backend: `normalize_callsign` + `split_callsign` + `ofp_matches_active_flight` pure functions | ~50 |
| Backend: `flight_refresh_simbrief` Pfad-Auswahl + `compose_failure`-Logik | ~60 |
| Backend: `verify_simbrief_identifier` Tauri-Command fuer "Pruefen"-Button | ~30 |
| Frontend: Settings-Panel "SimBrief Integration"-Section (2 Inputs, "Pruefen"-Button, Status-Anzeige) | ~80 |
| Frontend: App.tsx Login-Mount localStorage → Backend Sync | ~15 |
| Frontend: BidsList Notice-Wording-Updates (Mismatch + bid_not_found) | ~10 |
| Frontend: i18n DE/EN/IT (~8 neue Keys: title/intro/username_*/userid_*/verify/notice_mismatch) | ~50 |
| Tests Backend: 11 Match-Tests + 5 Pfad-Tests + 5 Settings-Tests + 2 Parser-Tests | ~150 |

**Geschaetzt: ~600 LOC Diff** (Polish-Korrektur gegenueber v1.3's ~320). Spec-konform, additiv zu v0.7.7-Foundation (`608630e`), keine Breaking Changes. Tests + i18n machen mehr aus als initial gedacht (Thomas-Hinweis QS v1.3: "eher 400-500 LOC" — v1.4-Detail-Tabelle landet bei 600).

---

## 8. Notice-Outcomes (Erweiterung der v0.7.7 §8-Tabelle)

| Outcome | Notice-Tone | Text (DE) |
|---|---|---|
| SimBrief-direct: OFP matched + changed=true | (kein Notice) | — |
| SimBrief-direct: OFP matched + changed=false | info | "OFP unveraendert. SimBrief liefert weiterhin OFP-ID {{id}}." |
| **SimBrief-direct: Mismatch** (NEU v0.7.8) | warn | "Aktueller SimBrief-OFP gehoert zu Flug {{origin}} → {{destination}} ({{callsign}}). Bitte fuer den aktiven Flug auf simbrief.com neu generieren." |
| SimBrief-direct: Identifier unbekannt → Fallback Pointer | warn | "SimBrief-Username/User-ID nicht gefunden. Pruefe Settings → SimBrief Integration." |
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

### Workflow D: Pilot mit Identifier, SimBrief offline
1. AeroACARS versucht SimBrief-direct → Network-Error (oder HTTP 5xx)
2. SOFT-Fallback auf Pointer-Pfad
3. Wenn Bid noch da → Pointer-Pfad-Ergebnis (selten)
4. Wenn Bid weg → **`compose_failure` priorisiert Direct-Error** (siehe §5.1):
   - Notice: "SimBrief gerade nicht erreichbar; Pointer-Fallback auch nicht moeglich. Versuche es in ein paar Minuten erneut."
   - Pilot weiss damit dass der **primaere** Fehler SimBrief-Verfuegbarkeit ist, nicht "Bid weg" — actionable Info zur richtigen Diagnose.

---

## 10. Test-Vorschlaege

Backend (Rust):

**Match-Verifikation (v1.1 verschaerft — keine Suffix-Logik):**
- `normalize_callsign_strips_hyphens_and_uppercases` ("DLH-100" → "DLH100", "gsg 100" → "GSG100")
- `split_callsign_separates_prefix_and_number` ("DLH100" → ("DLH", "100"))
- `ofp_matches_when_callsigns_exact` ("DLH100" + "DLH100")
- `ofp_matches_when_simbrief_callsign_is_number_only` (active "DLH100" + simbrief "100" → match)
- `ofp_matches_case_insensitive_icao` (DPT/ARR Variants)
- **`ofp_rejects_when_callsign_numbers_overlap_but_differ` (KRITISCHER v1.1-Test: active "DLH100" + simbrief "DLH1100" → MISMATCH)**
- `ofp_rejects_when_callsign_completely_different` ("DLH100" + "AFR300")
- `ofp_rejects_when_dpt_wrong`
- `ofp_rejects_when_arr_wrong`
- `ofp_tolerates_empty_simbrief_callsign` (dpt+arr genuegen wenn SB-Callsign leer)
- `ofp_matches_with_hyphen_in_active` ("DLH-100" vs "DLH100")

**OFP-ID-Parsing (v1.1 NEU):**
- `simbrief_parser_extracts_request_id_from_params`
- `simbrief_parser_handles_missing_request_id_with_empty_string`

**Pfad-Auswahl (v1.1 erweitert um composite-Fehler):**
- `flight_refresh_simbrief_uses_direct_when_identifier_set_and_match`
- `flight_refresh_simbrief_hard_blocks_when_identifier_set_and_mismatch` (HARD-Block, kein Fallback)
- `flight_refresh_simbrief_soft_falls_back_to_pointer_when_simbrief_unavailable`
- `flight_refresh_simbrief_uses_pointer_when_no_identifier`
- **`flight_refresh_simbrief_composite_error_prioritizes_user_not_found_over_bid_not_found` (v1.1 P1-2)**

**Settings:**
- `set_simbrief_settings_persists_both_fields`
- `get_simbrief_settings_returns_none_when_unset`
- `simbrief_identifier_empty_string_treated_as_none`
- `user_id_priority_when_both_filled` (User-ID gewinnt ueber Username wenn beide da)
- `username_url_encoded_in_request` (Sonderzeichen / Spaces escaped)

Frontend (manueller Smoke):
- Settings-Tab: Username eingeben, App neu starten, Wert wieder da
- Bid-Tab-Refresh in Boarding mit Username gesetzt → neue Plan-Werte ohne Pointer
- Bid-Tab-Refresh mit falsch konfiguriertem Username → SOFT-Fallback funktioniert

---

## 11. Entscheidungs-Log

### v1.0-Punkte (in v1.1 entschieden)
- ✓ **Username-Validierung:** **"Pruefen"-Button** (= ein expliziter Pilot-Klick), kein hartes onBlur-Fetch. Status-Anzeige darunter.
- ✓ **Callsign-Match-Strictness:** **Suffix-Match raus**, statt dessen normalisierter `airline_icao + flight_number`-Vergleich. Verhindert "DLH1100 matched 100"-Fehler.
- ✓ **Mismatch-Verhalten:** **HARD-Block** in v0.7.8. Pilot muss regenerieren — kein "trotzdem ueberschreiben"-Override (= falscher Plan + falsche Loadsheet ist nicht hilfreich).
- ✓ **Settings-Tab-Platzierung:** **eigene Section "SimBrief Integration"**.
- ✓ **Test-Strategie:** primaer pure-function-Tests (Match-Logik, Settings-Storage) + manuelle Smoke-Tests fuer SimBrief-API-Interaktion (kein Mocking-Sweep in v0.7.8).

### v1.1-Punkte (in v1.2 entschieden nach API-Probe)
- ✓ **OFP-ID-Quelle:** `<params><request_id>` aus XML — bestaetigt durch Live-Probe. `static_id` kann leer sein, ist nicht zuverlaessig. `sequence_id` (neu entdeckt) wird derzeit ignoriert.
- ✓ **`SimBriefDirectError`-Enum:** getrennt halten (Network / UserNotFound / Unavailable / ParseFailed) — Notice-Wording haengt davon ab. Sammel-Code wuerde Pilot mit weniger actionable Info versorgen.
- ✓ **`compose_failure`-Wording:** kurz halten. Notice gibt Hauptursache + "siehe Activity-Log fuer Details".

### v1.2-Punkte (in v1.3 entschieden nach Thomas-QS auf Navigraph-Dev-Doku)
- ✓ **Username-only Fetch:** offiziell erlaubt (laut Navigraph dev-Doku "Fetching a User's Latest OFP Data"). Username UND User-ID werden beide in Settings unterstuetzt, mind. eines noetig, User-ID > Username Prioritaet.
- ✓ **`sequence_id` ignorieren:** kein Nutzen erkennbar. Optional `tracing::debug!` mitloggen, nicht in DTO/Persistenz.
- ✓ **Pilot-Probe-Test:** nicht-blockierend. Beim ersten Wechsel Pointer → Direct kann `changed=true` triggern auch bei inhaltsgleichem Plan — danach stabilisiert sich. Akzeptabel.

### v1.3-Punkte (final-vor-Code)
**KEINE offenen Punkte mehr.** Spec ist code-ready. Alle Entscheidungen sind im Spec-Body verankert:
1. SimBrief-direct via `?username=X` ODER `?userid=X` (eines reicht)
2. `static_id` komplett raus — nicht Settings, nicht Parser, nicht FlightStats
3. `request_id` aus `<params>` parser-seitig — canonical changed-flag-Quelle
4. Error robust: HTTP-Code + `<fetch><status>`-Tag beides pruefen
5. `sequence_id` nur trace-log, nicht persistiert
6. Callsign-Match per normalisierter `airline_icao + flight_number`-Form (kein Suffix-Match)
7. Mismatch = HARD-Block (kein Override)
8. Settings: eigene Section, "Pruefen"-Button statt onBlur-Spam
9. App-root localStorage-Sync beim Login-Mount

---

## 12. Versionierung dieser Spec

- **v1.0 (2026-05-11):** Initial Draft basierend auf Thomas-Decision "SimBrief-direct, big release bundle".
- **v1.1 (2026-05-11):** Nach 1. QS-Review von Thomas:
  - §3 P1: Parser-Erweiterung um `<params><request_id>` als `ofp.ofp_id` — Spec v1.0 sagte faelschlich "Parser muss NICHT erweitert werden". OHNE OFP-ID kein sauberer `changed`-Flag-Vergleich.
  - §3 P2: SimBrief-Failure-Mode-Liste korrigiert auf laut Navigraph-Doku: HTTP 400 + small XML error fuer invalid user / fetch error (nicht primaer 404/empty).
  - §4 NEU 4.1: Identifier-Klaerung Username vs User-ID. Beide werden unterstuetzt — zwei separate Felder in Settings, User-ID gewinnt wenn beide gesetzt. URL-Encoding fuer Username.
  - §4 P2 (App-root Sync): localStorage-Push beim App-Start/Login in App.tsx, nicht nur on-SettingsPanel-mount. Sonst nach Restart leer.
  - §4 Settings-UI: "Pruefen"-Button statt hartem onBlur-Fetch (= explizite Pilot-Aktion). Eigene Section "SimBrief Integration".
  - §5.1 NEU P1: Fehler-Priorisierung beim Fallback. Direct-Fehler wird gemerkt, wenn Pointer auch scheitert composite Notice mit Direct-Priorisierung (= "Username falsch" beats "Bid weg" als Hinweis fuer den Piloten).
  - §6 P1: Callsign-Suffix-Match raus. Statt dessen normalisierter `airline_icao + flight_number`-Vergleich. `normalize_callsign` + `split_callsign` als Pure-Functions, getestet pro Edge-Case (insbesondere "DLH1100 vs 100"-False-Positive aus v1.0 verhindert).
  - §10 Tests aktualisiert: Suffix-Match-Tests raus, dafuer neue Edge-Cases (DLH1100, leerer Callsign, Hyphen-Variants). OFP-ID-Parsing-Tests neu. Settings-Tests um User-ID + URL-Encoding erweitert.
  - §11 Entscheidungs-Log: 5 v1.0-Punkte entschieden, 3 neue v1.1-Punkte fuer 2. QS.
- **v1.2 (2026-05-11):** Nach direkter SimBrief-API-Probe (Thomas verlinkte Navigraph-Doku + Live-Demo):
  - §3 komplett ueberarbeitet — XML-Response-Struktur direkt aus `?username=simbrief`-Probe gezogen statt aus indirekter Doku-Interpretation.
  - **Vereinfachung gegenueber v1.0/v1.1:** Username-only Fetch funktioniert (Status "Success" bestaetigt). static_id ist NICHT zwingend — kann leer sein.
  - §3 NEU: `<fetch><status>`-Tag als Error-Indikator. SimBrief liefert HTTP 200 + Status-Tag im XML, NICHT primaer HTTP 400 wie v1.1 (= Navigraph-Doku) sagte. Parser muss Status pruefen.
  - §3 NEU: `sequence_id`-Feld entdeckt (Funktion unklar, derzeit ignoriert).
  - §3 SimBriefOfp-Parser: zwei neue Felder `request_id: String` + `static_id: Option<String>`. Spec v1.1 hatte nur `ofp_id` — jetzt klar getrennt.
  - §4.1 URL-Aufbau-Snippet mit Prioritaet user_id > username, beide URL-encoded.
  - §11 v1.1-Punkte alle entschieden (mit Stand der API-Probe), 3 neue v1.2-Punkte fuer dein OK + Pilot-Probe-Test-Vorschlag (bid.simbrief.id vs request_id Identitaet pruefen).
- **v1.3 (2026-05-11):** Final-vor-Code nach Thomas-QS auf Navigraph "Fetching a User's Latest OFP Data"-Doku:
  - §3 SimBriefOfp-Parser: `static_id`-Feld komplett raus (war in v1.2 noch als Option enthalten). Begruendung Forum-Thread: `static_id` ist fuer Systeme die OFPs **erzeugen**, nicht fetchen. AeroACARS fetcht nur — wir brauchen keinen Slot-Pointer. Parser bekommt nur `request_id: String`.
  - §3 `sequence_id` final ignoriert (kein DTO/Persistenz, optional tracing::debug nur fuer Diagnose).
  - §3 Failure-Modes-Tabelle robust: **BEIDE Pfade** (HTTP 400 laut Navigraph-Doku UND `<fetch><status>Error</status>` laut Live-Probe) auf `UserNotFound` mappen. HTTP 5xx → `Unavailable`, andere non-2xx → `Network`, Parse-Fehler → `ParseFailed`.
  - §3 Hinweis dass `request_id` und `bid.simbrief.id` aus phpVMS NICHT garantiert identisch sind — erster Pfad-Wechsel kann `changed=true` triggern auch bei inhaltsgleichem Plan, danach stabil.
  - §11 alle v1.2-Punkte entschieden, **keine offenen Punkte mehr** — Spec ist code-ready.
- **v1.5 (2026-05-11):** Release-Blocker-Fix nach QS-Report Thomas (Real-Pilot-Case CFG1504 vs CFG4TK):
  - §6.1.1 Problem-Beschreibung: Pilot mit Bid airline=CFG flight_number=1504 + persoenlichem Callsign "4TK" generiert OFP mit Callsign "CFG4TK". v1.1-Match-Logik wuerde MISMATCH liefern und HARD-Block ausloesen → fachlich falsch.
  - §6.1.2 Loesung: Kandidaten-Liste statt strenger 1:1-Vergleich. SimBrief-Callsign muss gegen EINEN von 5 Kandidaten exakt matchen (airline+flight_number, flight_number, bid.flight.callsign, airline+pilot_callsign, pilot_callsign).
  - §6.1.3 Edge-Case-Tabelle v1.5: CFG4TK/4TK matchen jetzt korrekt, CFG1100/ABC4TK weiter MISMATCH. v1.1-Regression-Schutz (DLH1100-vs-100) bleibt erhalten.
  - §6.1.4 Empirik-Hinweis: AeroACARS liest BEIDE Quellen additiv (Flight.callsign existiert bereits im Parser, Profile.callsign neu). Robust gegen jede phpVMS-VA-Konfiguration.
  - Code-Anforderungen v1.5:
    * Profile.callsign-Feld in api-client parsen (additiv, serde(default))
    * ActiveFlight.bid_callsign + pilot_callsign (Option<String>)
    * PersistedFlight gleiche Felder mit serde(default) fuer legacy resume
    * ofp_matches_active_flight Signatur erweitert um pilot_callsign + bid_callsign
    * build_candidate_callsigns als neue Pure-Function
    * 4+ neue Tests fuer CFG4TK-Case
  - QS-Status v1.5: **Release-Blocker fuer v0.7.8** — Tag/Release erst nach Code-Update.
- **v1.4 (2026-05-11):** Polish nach Thomas-QS-Approval mit 5 Redaktions-Nits:
  - §2 + §5 Begriffs-Konsistenz: `simbrief_username`-Bezuege in Pseudo-Code auf `simbrief_settings` korrigiert. localStorage-Key-Namen bleiben (sind technisch).
  - §3 + §5 + §8 Notice-Wording: "SimBrief-Username/UserID" → "SimBrief-Username/User-ID" (Bindestrich). §8 Username-unbekannt-Notice auf Identifier-neutral umgeschrieben.
  - §7 Aufwand realistischer: Detail-Tabelle ergibt ~600 LOC (statt v1.3-Schaetzung 320). Thomas-Hinweis: 400-500 LOC; v1.4-Detail-Tabelle landet bei 600 mit i18n + Tests.
  - §9 Workflow D: bei `compose_failure` (SimBrief offline + Bid weg) wird Direct-Error priorisiert ("SimBrief gerade nicht erreichbar") statt schlichtem `bid_not_found`. Konsistent mit §5.1 Composite-Failure-Logik.
  - §12 chronologische Reihenfolge fixiert (v1.0 → v1.1 → v1.2 → v1.3 → v1.4 latest-last) — vorher v1.3 vor v1.2/v1.1 was unleserlich war.
