# Changelog

Alle nennenswerten Änderungen an AeroACARS. Format: lose an [Keep a Changelog](https://keepachangelog.com/) angelehnt; Versionsnummern folgen [Semantic Versioning](https://semver.org/) (Patch: Bugfix, Minor: Feature, Major: Breaking).

---

## [v0.5.37] — 2026-05-08

🇮🇹 **Italienische Übersetzung + Fix für Sprach-Reset nach Update.**

### 🇮🇹 Italiano (für Marco)

- Volle Übersetzung des UI in Italienisch (`locales/it/common.json`, ~250 Keys)
- Aviation-Begriffe korrekt: crociera, discesa, decollo, atterraggio, riattaccata, etc.
- Standard-ICAO-Abkürzungen behalten (IAS, GS, AGL, MSL, V/S, kt, ft, fpm)
- `LANGUAGE_LABELS`-Map exportiert für saubere Anzeige im Switcher

### 🐞 Sprach-Reset-Bug

User-Report: nach jedem App-Update fiel die Sprache auf Englisch zurück, obwohl Browser-Locale Deutsch war.

**Root-Cause:** `i18next-browser-languagedetector` mit `caches: ["localStorage"]` schreibt nur dann nach localStorage wenn `i18n.changeLanguage()` explizit gerufen wird. Bei reiner Auto-Detection (Browser-Locale) bleibt der localStorage-Key leer → nach Update fängt die Detection wieder bei Null an, und WebView2 könnte die Locale anders berichten.

**Fix:**
- Beim Ersten-Run nach Auto-Detection: erkannte Sprache EXPLIZIT in `localStorage["aeroacars.lang"]` schreiben
- Neue helper-Funktion `setLanguage(lang)` die `i18n.changeLanguage()` + `localStorage.setItem()` koppelt
- SettingsPanel nutzt `setLanguage()` statt `changeLanguage()` direkt

### 🆕 Sprach-Switcher

SettingsPanel-Dropdown zeigt jetzt alle 3 Sprachen (DE, EN, IT) dynamisch aus `SUPPORTED_LANGUAGES`. Marco kann manuell auf Italienisch umschalten — Auswahl persistiert über App-Updates.

Versions-Bump 0.5.36 → 0.5.37.

---

## [v0.5.36] — 2026-05-08

🛩 **VFR/Manual-Mode: Aircraft-Mismatch wird Warnung statt Block.**

User-Stance: „wir sollten dem Piloten vertrauen". Der bisherige Hard-Block beim Aircraft-Type-Mismatch im VFR/Manual-Modus widersprach dem — Pilot hat im Picker bewusst eine Aircraft gewählt, aber falls X-Plane die ICAO als `ATCCOM.AC_MODEL XXX.0.text` meldet oder Custom-Liveries den Title verändern, fiel der Loose-Match durch und der Start wurde geblockt.

### 🆕 Was sich ändert

- Backend: neuer Error-Code `aircraft_mismatch_warning` (statt `aircraft_mismatch`) im VFR-Pfad
- ManualFlightPlan bekommt `acknowledge_aircraft_mismatch: bool` Feld
- Erst-Versuch ohne Flag → liefert Warnung zurück
- Modal zeigt **gelbes Warn-Banner** + **„Trotzdem starten"-Button**
- Klick → Re-Submit mit `acknowledge=true` → Check wird übersprungen
- Mismatch wird im Recorder weiter geloggt (für Forensik) aber blockt nicht

### IFR-Mode unberührt

`flight_start` (IFR mit SimBrief-OFP) liefert weiter den harten `aircraft_mismatch`-Error. Begründung: bei IFR ist der OFP die Source-of-Truth und ein Mismatch deutet auf einen Buchungs-Fehler hin.

Versions-Bump 0.5.35 → 0.5.36.

---

## [v0.5.35] — 2026-05-08

🐞 **Touchdown-V/S Capture für GA + sparse-DataRef-Cases gefixt — Position-Sampling adaptiv unter 1000ft AGL.**

### Hintergrund

User-Report aus dem GSG-301 GA-Flug (Cessna 152 in X-Plane 12, Forensik-Log analysiert): peak_vs_fpm=-33 fpm gemeldet, peak_g_force=1.36 — passte nicht zueinander. Die echte TD-V/S war vermutlich -300 bis -400 fpm.

**Root-Cause-Analyse aus dem JSONL:**
- Position-Sampling auf **0.1 Hz** (= 10.4s Mean-Interval, 91% der Frames mit >10s Lücken)
- 10s vor TD: AGL 145 ft, V/S **-360 fpm** (letzter airborne Sample)
- TD-Frame: Lücke von 10.44s → kompletter Touchdown-Moment fiel durch
- Lua-30-Sample-Estimator spannte das Fenster über den ganzen Approach (statt Flare) und gab geglätteten Mittelwert zurück

### 🆕 Fix 1 — Adaptive Position-Rate

`adaptive_tick_interval()`: Tick-Cadence je nach Phase + AGL:
- Cruise/Climb/Descent: 3s (unverändert)
- Approach/Final/Takeoff bei AGL <1000ft: **2s** (= 0.5 Hz)
- AGL <500ft: **1s** (= 1 Hz)
- AGL <100ft (Flare/Wheels-Up): **500ms** (= 2 Hz)

### 🆕 Fix 2 — JSONL-Append pro Tick

Vorher: JSONL-Append war IM phpVMS-OK-Branch → wurde nur bei erfolgreichem phpVMS-POST geschrieben (8-30s Cadence). Jetzt: nach MQTT-Publish, vor phpVMS-POST → jeder Tick im Log.

### 🆕 Fix 3 — V/S-Estimator Sparse-Sampling-Fallback

Neuer `last_low_agl_vs_fpm`-Tracker in FlightStats: speichert die letzte airborne V/S unter 500ft AGL mit Timestamp. Wird kontinuierlich pro Tick upgedated.

X-Plane Priority-Chain neu:
- Bevorzugt `agl_estimate_xp` falls Fenster <3s (= echte Flare)
- Falls Fenster ≥3s = unplausible (= sparse-Sampling-Spread): verwendet `last_low_agl_vs_fpm` falls innerhalb 15s
- Bei beiden vorhanden: nimmt den deeperen (= numerisch kleineren)

Neue `vs_source`-Labels:
- `agl_estimate_xp_or_last_low` (beide vorhanden, deeper gewählt)
- `last_low_agl_vs` (Estimator implausibel, last_low gerettet)
- `agl_estimate_xp_implausible_window` (last resort)

### 🆕 Fix 4 — Go-Around-Detector empfindlicher

- `GO_AROUND_AGL_RECOVERY_FT`: 200 → **150 ft** (sparse Sampling)
- `GO_AROUND_MIN_VS_FPM`: 500 → **300 fpm** (slow GA Aircraft klettern selten >500fpm)

### Erwartung für GA-Flüge ab v0.5.35

Bei Cessna 152 in X-Plane mit Standard-DataRef-Rate:
- Position-Frames im Final: 1 alle Sekunde (statt 1 alle 10s)
- Touchdown-V/S richtig gefangen via `last_low_agl_vs_fpm` falls Lua-Estimator wegen Sim-FPS sparse läuft
- Go-Around bei Cub/C152-Style climb-out korrekt detektiert

Versions-Bump 0.5.34 → 0.5.35.

---

## [v0.5.34] — 2026-05-08

🛡 **JSONL-Forensik-Logs jetzt vollstaendig — alles was MQTT publiziert landet auch im Log.**

### Hintergrund

Beim Recovery-Vorfall heute hatten wir versucht aus den JSONL-Forensik-Logs die verlorenen Touchdown-Daten zu rekonstruieren. Problem: das `landing_scored`-Event im JSONL hatte nur 4 Felder (`score`, `peak_vs_fpm`, `peak_g_force`, `bounce_count`) — die ~50 detaillierten Forensik-Felder die der Live-MQTT-Touchdown-Topic publiziert (Approach-Stability v2, Landing-Quality, Wind-Komponenten, Runway-Match, V/S-Estimator-Vergleiche, etc.) fehlten komplett.

### 🆕 Neue JSONL-Events

`recorder::FlightLogEvent` bekommt 4 neue Variants — alle parallel zum jeweiligen MQTT-Topic geschrieben:

- **`TouchdownComplete`** — kompletter `TouchdownPayload` (= alle ~50 Felder die der Live-Recorder bekommt)
- **`PirepFiled`** — kompletter `PirepPayload` (Block/Flight-Time, Fuel-Aggregate, Distance, Peak-Altitude, Landing-Score, Go-Arounds, Touchdown-Count, Gates, Approach-Runway, Divert)
- **`BlockSnapshot`** — Out-Of-Block Pre-Flight-Snapshot
- **`TakeoffSnapshot`** — Wheels-Up-Snapshot

Format: `{ "type": "...", "timestamp": "...", "payload": {...} }` — `payload` ist `serde_json::Value` damit das Schema mitwachsen kann ohne dass alte Logs unparsbar werden.

### Was das ermoeglicht

Falls die Server-DB jemals wieder Daten verliert, kann ein offline Recovery-Tool die Touchdown-/PIREP-Rows **1:1** aus dem JSONL rekonstruieren — keine Approximationen mehr, keine fehlenden Felder.

### Backwards-Compat

`LandingScored` (v0.5.0+) bleibt erhalten als kleinerer Event fuer Tools die nur den Score-Indikator brauchen. `TouchdownComplete` wird zusaetzlich geschrieben.

Versions-Bump 0.5.33 → 0.5.34.

---

## [v0.5.33] — 2026-05-08

🐞 **Aircraft-Picker funktioniert jetzt richtig: alle Flugzeuge, nur Ground+Active, voll DE+EN.**

### 🐞 Behoben

**Problem (v0.5.32):**
v0.5.32 versuchte `/api/fleet/{id}/aircraft` aufzurufen — diesen Endpoint gibt es in phpVMS-V7 **nicht** (nur `/api/fleet/aircraft/{id}` für ein einzelnes Aircraft per ID). Resultat: alle per-Subfleet-Calls liefen ins 404, wurden „graceful skipped", Picker zeigte „Keine Aircraft in deiner Fleet verfügbar" trotz vorhandener Flugzeuge.

**Fix in v0.5.33:**
- `GET /api/fleet?limit=100&page=N` paginiert (verifiziert via offizielle phpVMS-Docs + Source-Code)
- `SubfleetResource` enthaelt `aircraft`-Array bereits inline → kein N+1
- Pages-Loop bis non-volle Page (Cap 50 Pages)
- Neuer `SubfleetWithAircraft`-Typ mit `#[serde(default)] aircraft: Vec<AircraftDetails>`
- `get_all_aircraft()` flatten ueber alle Subfleets

### 🆕 Filter (Pilot-Wunsch)

**Nur tatsächlich verfügbare Flugzeuge im Picker:**
- `state == 0` (PARKED — nicht IN_USE / IN_AIR)
- `status == "A"` (ACTIVE — nicht MAINTENANCE / STORED / RETIRED / SCRAPPED / WRITTEN_OFF)
- Tracing-Log: `before=N after=M` für Diagnose

### 🌍 Vollständige DE+EN-Lokalisierung

**Neue i18n-Keys (35+):**
- `manual_flight.*` — Header, Step-Titles, Loading, Empty, Search, List-Total, No-Match, Submit-Buttons, **alle 6 Form-Felder** (Block-Fuel, Flight-Time, Cruise-Level, Route, Alternate, ZFW) je mit Label + Placeholder + Help-Text
- `bid_card.*` — VFR-Start-Button + Tooltip, komplette Hint-Box (Title + IFR/VFR-Zeilen)
- `flight.error.*` (10 Codes) jetzt **auch im Manual-Modal** lokalisiert (war vorher roher Code wie `no_sim_snapshot: ...`)

### ✏️ Sprache

- „Aircraft" → „Flugzeug" überall im UI (DE)
- „Aircraft" → „Aircraft" (EN, weil das im Englischen korrekt ist)
- Empty-State-Meldung neu: nennt konkret die Filter-Gründe (Einsatz/Luft/Wartung)

Versions-Bump 0.5.32 → 0.5.33.

---

## [v0.5.32] — 2026-05-08

🐞 **Aircraft-Picker zeigt jetzt einzelne Aircraft, nicht Subfleets.**

### 🐞 Behoben

**Problem (User-Feedback aus v0.5.30/31):**
Im VFR/Manual-Mode-Aircraft-Picker tauchten Einträge wie „DLH-A319-CFM-SL", „BAW-A319-IAE-WTF" auf — das sind **Subfleet-Namen, keine Aircraft-Registrations**. Pilot konnte daraus keinen einzelnen Flieger auswählen („mit einem Subfleet kann ich nicht fliegen").

**Root-Cause:**
phpVMS-V7-Endpoint `GET /api/fleet` liefert **Subfleets** (= Sammlung von Aircraft eines Typs), nicht einzelne Aircraft. Unsere v0.5.27-Implementation hat den Response naiv in `AircraftDetails` deserialisiert — das hat zwar deserialisiert (alle Felder sind `Option`), aber `registration`/`icao` der Subfleet-Liste sind eben Subfleet-Felder, nicht Aircraft-Felder.

**Fix in v0.5.32:**
- Neuer `SubfleetSummary`-Typ in `api-client` für korrekte Subfleet-Deserialisierung (`id`, `name`, `icao`, `type`)
- Neue Methode `Client::get_all_aircraft()`: aggregiert über alle Subfleets via N+1-Pattern (`GET /api/fleet/{id}/aircraft` pro Subfleet)
- Per-Subfleet-Failures werden geloggt aber nicht propagiert — ein einzelner kaputter Subfleet crashed nicht den Picker
- `fleet_list_at_airport` ruft jetzt `get_all_aircraft()` statt `get_fleet()` auf
- phpVMS-Subfleet-Rank-Restriktion wirkt weiter server-seitig (= Pilot sieht nur was er fliegen darf)

Versions-Bump 0.5.31 → 0.5.32.

---

## [v0.5.31] — 2026-05-08

🎯 **Mode-Hint-Box deutlicher: klare Regel statt Marketing-Text.**

### 🔧 Geändert

User-Feedback: der v0.5.29-Hinweis war zu unscheinbar/unklar. „IFR Start: nutzt SimBrief-OFP" sagt nicht eindeutig dass es **PFLICHT** ist. „VFR Start funktioniert auch ohne SB" sagt nicht eindeutig dass es **OPTIONAL** ist.

**v0.5.31 — neue Hint-Box mit klarer Regel-Struktur:**

```
┌─────────────────────────────────────────────────────────────┐
│ 💡 Welchen Button nutzen?                                   │
│                                                             │
│ 🛫 IFR Start    NUR mit SimBrief-OFP (Plan-Daten kommen    │
│                  aus dem OFP).                              │
│                                                             │
│ 🛩 VFR Start    AUCH OHNE SimBrief-OFP — du gibst Aircraft  │
│                  + Block-Fuel selbst ein.                   │
└─────────────────────────────────────────────────────────────┘
```

- **Titel** „Welchen Button nutzen?" macht Frage explizit
- **Zwei klare Zeilen** mit Icon + Button-Name (color-coded blau/gelb) + Regel
- **Bold-Highlights** auf dem entscheidenden Wort: „NUR mit" vs „AUCH OHNE"
- **Karten-Border** statt linker Border-Strich — visuell prominenter

Versions-Bump 0.5.30 → 0.5.31.

---

## [v0.5.30] — 2026-05-08

🎯 **Aircraft-Picker zeigt jetzt die GESAMTE Fleet — keine Airport-/State-Einschränkung.**

### 🔧 Geändert

**Problem (User-Feedback aus v0.5.27/28-Tests):**
Beim VFR/Manual-Mode-Aircraft-Picker für einen LEPA-Bid:

> „Keine Aircraft am LEPA verfügbar (alle in use, in Maintenance, oder phpVMS-Endpoint nicht eingerichtet)."

Pilot konnte keinen Flug starten obwohl Aircraft in der Fleet existieren — sie standen aber an anderen Airports.

**v0.5.30 Lösung:**
- **Kein Airport-Filter mehr** — alle Aircraft die der Pilot fliegen darf werden angezeigt (= /api/fleet, Subfleet-Rank-Restriktion bleibt server-seitig)
- **Kein State-Filter mehr** — auch in-use / in-flight / Maintenance Aircraft werden angezeigt mit visuellem Indikator
- **Smart-Sort**: Aircraft am Departure-Airport stehen oben in der Liste, dann nach State (parked vor in-use), dann alphabetisch
- **Visuelle Indikatoren** in der Liste:
  - Grün-fettes `@LEPA`-Tag wenn Aircraft am Dep-Airport steht
  - Status-Pill: `🔒 in Use` (gelb) / `✈ in Flight` (cyan) / `🔧 Maintenance` (rot) bei nicht-parked Aircraft
- **Header zeigt Count**: "12 Aircraft gesamt · Aircraft am LEPA stehen oben"

**Falls Pilot ein in-use/Maintenance-Aircraft auswählt:** phpVMS-Prefile lehnt mit klarer Fehlermeldung ab — Pilot kann dann anderes wählen.

### 🔧 Implementation

- **Rust**: `fleet_list_at_airport()` ruft jetzt nur `client.get_fleet()` (= alle Aircraft), nicht mehr `/api/airports/{icao}/aircraft`. icao-Parameter bleibt für Sort-Priority. State-Filter (`state == 0`) entfernt.
- **Frontend**: Aircraft-List-Item zeigt Airport + State-Pill. Empty-State-Message angepasst.

Versions-Bump 0.5.29 → 0.5.30.

---

## [v0.5.29] — 2026-05-08

🎯 **Pilot entscheidet komplett selbst — Auto-IFR/VFR-Kategorisierung entfernt, durch klaren Hinweis-Text ersetzt.**

### 🔧 Geändert

**v0.5.28 hatte automatische IFR/VFR-Pills** auf jeder Bid-Card (gruen/gelb basierend auf `flight_type`-Code). Das war zwar "nur Hinweis", fühlte sich aber wie eine Kategorisierung an — Bids wurden mit einem Label versehen.

**v0.5.29: Pills entfernt, statt dessen klare Text-Box** unter den Action-Buttons:

> 💡 **IFR Start**: nutzt SimBrief-OFP (Block-Fuel/Route/Weights aus dem Plan).
> **VFR Start**: funktioniert auch ohne SB — du wählst Aircraft + Block-Fuel selbst.
> Pilot entscheidet je nach Flug.

**Konsequenz:**
- Keine Auto-Detection mehr nach `flight_type` (= keine Annahme „dieser Bid ist IFR")
- Keine farblichen Kategorien-Pills
- Hinweis-Text steht IMMER da (nicht conditional)
- Beide Buttons immer sichtbar wenn kein aktiver Flug läuft
- Trust-the-Pilot in Reinform

### 🔧 Implementation

- `flightRulesHint()`-Helper entfernt (= Auto-Detection-Logik)
- IFR/VFR-Pill-JSX in BidsList aus dem Header entfernt
- Neuer `.bid-card__mode-hint`-Block unter den Buttons mit kompaktem Erklärungs-Text
- CSS umgebaut: `.bid-card__rules-badge--*` entfernt, neuer `.bid-card__mode-hint` Style (subtle grau-bordered)

Versions-Bump 0.5.28 → 0.5.29.

---

## [v0.5.28] — 2026-05-08

🎯 **UX-Polish für VFR/Manual-Mode: klarere Button-Labels + IFR/VFR-Hinweis-Pill auf Bid-Cards.**

Folgepatch zu v0.5.27. Funktionalitaet identisch, nur bessere Lesbarkeit + Hinweise. Kein Verhaltens-Aenderung — Pilot entscheidet weiter selbst (= keine harte Enforcement nach flight_type).

### ✨ Neu

**1. Button-Labels eindeutig:**
- "Start Flight" → **"🛫 IFR Start (SimBrief)"**
- "🛩 VFR/Manual-Mode" / "🛩 Manual-Override" → einheitlich **"🛩 VFR Start (manuell)"**

**2. Hover-Tooltips erklaeren wann zu nutzen:**
- IFR-Button: „Standard-Flug nach IFR-Regeln, basiert auf deinem SimBrief-OFP. Block-Fuel, Route, Weights und Alternates kommen aus dem OFP."
- VFR-Button: „Manueller Flug-Start ohne SimBrief-OFP — z.B. fuer VFR, kleine Pisten oder Pattern-Training. Du waehlst Aircraft + Block-Fuel selbst. Auch nutzbar als Aircraft-Override fuer Bids mit SimBrief-OFP."

**3. IFR/VFR-Hinweis-Pill auf jeder Bid-Card** (Header-Meta-Row):
- **IFR-Pill** (gruen): bei flight_type ∈ {J, F, C, M, I, V, S, R} — Scheduled, Charter, Mil, Special
- **VFR-Pill** (gelb): bei flight_type containing "VFR", oder ∈ {G, T, X} — General Aviation, Training, Test
- Kein Pill: bei unbekanntem oder leerem flight_type

Reine **Information** — KEINE Filter, KEINE Pflicht. Pilot kann auch IFR-Bid mit VFR-Manual-Mode fliegen wenn er will, oder VFR-Bid mit SimBrief-OFP. Trust-the-Pilot-Design.

**4. Tooltip-Hint auf der Pill:**
- IFR-Pill: „IFR-typischer Bid (Scheduled / Charter). Empfohlener Flow: SimBrief-OFP + 'IFR Start'-Button. Du kannst aber auch VFR/Manual fliegen."
- VFR-Pill: „VFR-typischer Bid (GA / Training / Test). Empfohlener Flow: 'VFR Start (manuell)'-Button. Du kannst aber auch SimBrief nutzen falls vorhanden."

### 🔧 Implementation

- **BidsList.tsx**: neuer Helper `flightRulesHint(type)` mit Detection-Logik. Pill rendert nur wenn Hint != null. Button-Labels in JSX angepasst.
- **App.css**: `.bid-card__rules-badge--ifr` (gruen) + `--vfr` (gelb) parallel zu existierendem type-badge.

### ⚠ Hinweise

- Wenn dein VA-flight_type-Schema nicht in {J,F,C,M,I,V,S,R,G,T,X} fällt: kein Hinweis-Pill. Zwei Optionen: phpVMS-Admin → Flight-Type-Codes auf ICAO-Standard setzen, ODER `flight_type` mit "VFR" / "IFR" als Substring (z.B. "VFR-Pattern" oder "IFR-Charter").
- Detection-Pattern ist in `flightRulesHint()` lokalisiert — bei VA-spezifischen Konventionen einfach die Switch-Case erweitern.

---

## [v0.5.27] — 2026-05-08

🎯 **VFR/Manual-Flight-Mode — Flug-Start ohne SimBrief-OFP für VFR-Flüge, kleine Pisten, GA.**

### ✨ Neu

**Problem:** AeroACARS hat bisher SimBrief-OFP für jeden Bid verlangt (siehe `lib.rs` Z.4848: `"no aircraft on this bid — please prepare a SimBrief OFP first"`). Für VFR-Flüge unterstützt SimBrief aber kein OFP-Routing — Pilot konnte zwar Bid in phpVMS erstellen, AeroACARS verweigerte aber den Start.

**Lösung:** Neuer „🛩 VFR/Manual-Mode" Button auf jeder Bid-Card. Pilot wählt:

1. **Aircraft-Picker** mit Suche
   - phpVMS-API `GET /api/airports/{icao}/aircraft` (mit Fallback auf `/api/fleet`)
   - Filter: nur Aircraft im State `parked` (= verfügbar)
   - Sim-Default-Auswahl: AeroACARS sieht den im Sim geladenen Aircraft → vorausgewählt mit Match-Erkennung über Registration ODER ICAO
   - Volltext-Suche über ICAO / Registration / Name

2. **Manual-Flight-Plan-Form**
   - **Pflicht-Felder**: Block-Fuel (kg), erwartete Flugzeit (min) — sonst keine Fuel-Score / ETA möglich
   - **Optional**: Cruise-Level (ft), Route (free-text), Alternate (ICAO), ZFW (kg)

3. Klick „🛩 Flug starten" → identischer Flow wie Standard-`flight_start` aber ohne SimBrief-Pflicht.

### 🔧 Implementation

**Client (lib.rs):**
- Neue Tauri-Commands `fleet_list_at_airport(icao)` + `flight_start_manual(bid_id, plan)`
- `ManualFlightPlan` Deserialize-Struct mit Pflicht-Feldern + Optionals
- Identischer Pre-Flight-Gate (ground + dpt-distance), Aircraft-Mismatch-Check, PIREP-Prefile, Streamer-Spawn
- `FlightStats.flight_plan_source: "simbrief" / "manual" / None` (carry-through im PIREP-Body als notes-Prefix)
- `planned_burn_kg` Fallback: 90% des block_fuel falls Pilot's planned_burn nicht angibt

**API-Client (api-client/src/lib.rs):**
- Neue Methoden `client.get_aircraft_at_airport(icao)` + `client.get_fleet()`

**Frontend (TS/React):**
- Neue Komponente `<ManualFlightModal>` mit 2-Stage-Workflow (Aircraft → Plan)
- 130+ Zeilen CSS für das Modal (matching dark theme)
- Manual-Mode-Button in BidsList:
  - Bei Bid OHNE simbrief: „🛩 VFR/Manual-Mode" als gleichwertige Action
  - Bei Bid MIT simbrief: „🛩 Manual-Override" (= falls Pilot anderes Aircraft fliegen will)
- Sim-Snapshot wird als simHint übergeben für Aircraft-Default-Auswahl + Block-Fuel-Default

**Backward-kompatibel:** existierender `flight_start`-Flow bleibt unverändert. SimBrief-Bids gehen weiter den OFP-Path, Manual-Mode ist additiv.

### ⚠ Hinweise

- **Aircraft-Subfleet-Validation**: phpVMS enforced server-side — Pilot mit Rank-N kann keine Aircraft fliegen die Rank N+1 brauchen. Manual-Picker zeigt aber alle Aircraft am Departure-Airport.
- **Fuel-Planung**: ohne explicit `planned_burn_kg` nehmen wir 90% des Block-Fuel als Trip-Schätzung. Realistischer wäre 75% (= mit Reserve), aber 90% ist bei VFR/GA üblicher.
- **PIREP-Notes**: bei Manual-Mode wird automatisch `Manual/VFR-Mode (kein SimBrief-OFP). Block: XXX kg, ETA: YY min` in den PIREP-Notes-Block geschrieben damit VA-Owner sieht dass es ein Manual-Flug war.

---

## [v0.5.26] — 2026-05-08

🎯 **9 neue Landung-Sicherheits-Indikatoren + DA-Gate (200 ft) + sim-/aircraft-spezifische Limits.**

Folgepatch zu v0.5.25 — die Approach-Stability-v2 deckte den **Anflug-Pfad** korrekt ab. v0.5.26 ergänzt **per-Touchdown-Sicherheits-Metriken** und einen strengeren **Decision-Altitude-Gate-Check**.

### ✨ Neu — Sicherheits-Indikatoren am Touchdown

**1. Wing-Strike-Severity (%)**
Bank am TD relativ zum aircraft-spezifischen Wing-Strike-Limit. 0% = wings level, 100% = am Limit. Conservative-Defaults pro ICAO (CL60: 6°, A321: 7°, B737: 8°, C172: 15°, etc.). Über 60% gibt Coaching-Hinweis, über 80% = Alert.

**2. Float-Distance (m)**
Distanz Threshold-Crossing → Touchdown. Long-Landing-Indikator. Standard 300-400 m. > 1000 m = Runway-Overrun-Risk auf kurzen Bahnen.

**3. Touchdown-Zone (1/2/3)**
FAA-Drittel-Klassifikation: Zone 1 = erstes Drittel (correct), Zone 2 = mittleres (long), Zone 3 = letztes (overshoot). Aircraft-Type-unabhängig.

**4. Vref-Deviation (kt)**
IAS am TD vs. Vref. **Source-Chain**: PMDG-FMC (MSFS-only) → ICAO-Kategorie-Default → unbekannt. Vref-DB enthält 30+ Aircraft-Types von B748 bis C172.

**5. Stable-At-DA (200 ft AGL/HAT)**
Strengerer 200-ft-Sub-Gate-Check (= ICAO Decision-Altitude-Standard für CAT-I-ILS). Tighter Cutoffs als beim 1000-ft-Gate: jerk < 80, bank < 3°, ias < 8 kt.

### ✨ Neu — Aggregat-Metriken

**6. Stall-Warning-Counter** — Anzahl `stall_warning=true`-Samples im gesamten Approach-Buffer. Indiziert ob Pilot Speed-Margin zu eng hatte.

**7. Yaw-Rate am TD (°/s)** — heading-Änderung im 1-sec-Window vor TD. Hoch = Ground-Loop-Risk bei Crosswind-Landing.

**8. Brake-Energy-Proxy (kJ/m)** — `(½ × Mass × IAS²) / Rollout`. Indiziert Brake-Pack-Thermal-Stress.

**9. Aircraft-spezifische Limits-DB** (ICAO-basiert)
Hardcoded `aircraft_limits_for(icao)` mit `max_bank_landing_deg` + `typical_vref_kt` für 30+ Standard-Types. Fallback `8°/None` für unbekannte ICAO. Pilot/VA-Override via DBasic Tech-Limits weiter erste Priorität.

### ✨ Neu — UX

**Neue „🎯 Landing-Quality"-Card im LandingAnalysis-Modal** zusätzlich zur Approach-Stability-Card. Zeigt 6 MetricTiles (Wing-Strike-Risk / TD-Zone / Float-Distance / Vref-Dev / Yaw-Rate / Brake-Energy) mit Tone-Coding und ausführlichen Hover-Tooltips.

**Erweiterte Coaching-Texte** in der Approach-Stability-Card:
- „Wing-Strike-Risk 85% — Bank am TD nahe Aircraft-Limit. Crosswind-Korrektur über Sideslip (Wing-Down + Rudder), nicht über Crab-into-flare-only."
- „Touchdown im letzten Drittel der Bahn (Zone 3) — Runway-Overrun-Risk auf kurzen Bahnen. Pre-flare nicht zu lang, früher abfangen."
- „IAS am TD -8 kt unter Vref — Stall-Risiko."
- „Stabil bei 1000 ft, aber NICHT mehr bei 200 ft (DA). Final-Phase wackelig."
- „⚠ 3 Stall-Warning-Events im Approach detektiert. Speed-Margin zu eng."

### 🔧 Implementation

- **Client (`lib.rs`)**: `aircraft_limits_for(icao)` Lookup-DB mit 30+ Types. `compute_approach_stability_v2` erweitert um DA-Gate (200 ft Filter) + Stall-Counter. Per-Touchdown-Section im File-PIREP-Path: Wing-Strike-Severity, Float-Distance + TD-Zone aus runway_match, Vref-Deviation mit Source-Chain, Yaw-Rate aus 1-sec-snapshot_buffer-Lookback, Brake-Energy-Formel.
- **MQTT-Payload**: 9 neue Felder (alle `Option<>`, `skip_serializing_if`).
- **Server (`recorder`)**: 9 neue Spalten in `touchdowns`-Tabelle (idempotente ALTER), insertTouchdown extrahiert, /api/touchdowns liefert sie typed.
- **Webapp**: Neue `_LandingQualityCard.tsx` mit 6 MetricTiles. ApproachStabilityCard um Coaching-Texte erweitert. TouchdownDto um 9 Felder.
- DB-Backup pre-deploy: `aeroacars-live.db.backup-pre-landing-quality`.

### ⚠ Hinweise

- **MSFS-Bank**: Sign noch nicht geflippt (im Gegensatz zu Pitch in v0.5.24). Wenn Wing-Strike-Severity-Daten nach Real-World-Tests komisch aussehen → Patch nachschieben.
- **Vref-Quelle "icao_default"**: konservativ pro Aircraft-Type, Pilot-Vref-Addends (Wind/Gust/Ice) NICHT berücksichtigt → Deviation-Werte nur als grobes Indiz, PMDG-FMC-Vref ist autoritativ wenn verfügbar.
- **Brake-Energy-Proxy**: ohne `landing_weight_kg` aus PMDG/Sim wird Default 50.000 kg verwendet — Werte ohne LDW-SimVar relativ.

---

## [v0.5.25] — 2026-05-08

🎯 **Approach-Stability v2: Stable-Approach-Gate-konformes Stabilitäts-Maß. Pilot versteht endlich was der Score bedeutet.**

### 🐛 Behoben

**Approach-Stability-Algorithmus war inkorrekt für Real-World-Cases.**

Pre-v0.5.25-Algorithmus berechnete `σ V/S` und `σ Bank` über das gesamte Approach+Final-Buffer-Window (= 5000 ft AGL bis Touchdown). Probleme:

- **ATC-Vectoring-Turns** (Bank 20-30° auf Anweisung) wurden als Pilot-Instabilität bestraft
- **Initial-Descent-Step-Downs** (Flaps-Stages, Speed-Down) erhöhten σ V/S obwohl Flugverhalten korrekt
- **σ um Mittelwert** misst NICHT Glide-Slope-Abweichung — ein Pilot der konstant -1100 fpm hält (über Glide-Slope) bekommt perfekten σ-Score
- **Mountain-Airports** (LSGS, LFKB) — AGL fluktuiert über Bergkämmen, Window-Filter falsch
- **GA-Anflüge** wurden mit 3°-ILS-Schwellwerten verglichen — C172 auf 5° Visual-Approach falsch bewertet
- **Späte RWY-Wechsel** (ATC ändert von 09L auf 09R bei 1200 ft AGL) bestraften Pilot für die ausgeführte Anweisung

### ✨ Neu — Approach-Stability v2

**HAT statt AGL als Window-Filter** (Mountain-Airport-tauglich)
Höhenfilter über `MSL_altitude − arr_airport_elevation` statt `AGL`. AeroACARS-Client sucht arr-Airport-Elevation aus dem phpVMS-API-Cache (state.airports.elevation). Fallback auf AGL wenn unbekannt — `approach_used_hat`-Flag in PIREP zeigt welche Methode genutzt wurde.

**5 Primär-Metriken (Score-relevant) im 1000-ft-Gate:**

1. **V/S-Jerk** — mean `|Δvs|` sample-to-sample. **Sim/Aircraft-agnostisch** (Jet, Turboprop, GA gleichermaßen). Schwellwerte: < 100 fpm/tick = sehr stabil, > 300 fpm/tick = unstabil.

2. **Bank σ (filtered)** — Standard-Deviation Bank, **Vector-Windows ausgenommen** (5 sec vor/nach RWY-Change). Pilot wird nicht für ATC-Turn bestraft.

3. **IAS σ** — Speed-Stabilität. < 5 kt = on-target, > 15 kt = große Schwankungen.

4. **Excessive-Sink-Flag** — `True` wenn ein Sample im Gate `V/S < -1000 fpm`. FAA-Limit-Verletzung (Pflicht-Go-Around).

5. **Stable-Config-Flag** — Gear ≥ 99% AND Flaps ≥ 70% am Gate-Eintritt.

**Composite Stable-At-Gate-Indikator:** `stable = jerk_ok AND bank_ok AND ias_ok AND !excessive_sink AND config_ok`. Pilot kriegt klares Boolean: ✓ STABLE GATE oder ⚠ UNSTABLE GATE.

**Sekundär (informativ, nicht Score-relevant):**
- V/S-Deviation vs 3°-ILS-Profil — relevant für ILS-Anflüge, bei GA/Visual nur informativ
- Max V/S-Deviation unter 500 ft AGL — kritischste Phase
- Late-RWY-Change-Detection — bestraft Pilot nicht, zeigt Hinweis-Pill

### ✨ Neu — UX-Transparenz für Piloten

**Neue ApproachStabilityCard im LandingAnalysis-Modal:**
- Composite-Indicator-Pill ✓ STABLE oder ⚠ UNSTABLE direkt im Card-Header
- Confidence-Hinweis (HAT vs. AGL, Sample-Count)
- 7 MetricTiles mit individuellen Tone-Bewertungen + ausführliche Hover-Tooltips erklären was jede σ bedeutet + Schwellwerte
- **Coaching-Section** mit konkreten Tipps wenn Score schlecht:
  - „V/S-Jerk 350 fpm/tick — du hast die Sinkrate stark verändert. Stabiles Sinkprofil halten: kleine Korrekturen früh, nicht große Korrekturen spät."
  - „Bank-σ 6.2° über 5° — späte Lineup-Korrekturen vermeiden, früh auf den Localizer einschneiden."
  - „RWY-Wechsel unter 1500 ft AGL detektiert — wurdest nicht bestraft, aber im Real-Op: stabil-prüfen oder go-around."
- Bei sauberem Anflug: ✨-Lob-Box

### 🔧 Implementation

- **Client (`lib.rs`)**: `ApproachBufferSample`-Struct erweitert um msl_ft / ias_kt / heading_true_deg / gear_position / flaps_position / selected_runway. `compute_approach_stability_v2(buf, arr_elevation)` implementiert HAT-Window + V/S-Jerk + IAS-σ + Excessive-Sink + Stable-Config. arr_airport_elevation_ft wird beim ersten Streamer-Tick aus dem state.airports-Cache (phpVMS-API) gelesen.
- **MQTT-Payload (`aeroacars-mqtt`)**: TouchdownPayload um 5 Felder erweitert (`approach_vs_jerk_fpm`, `approach_ias_stddev_kt`, `approach_excessive_sink`, `approach_stable_config`, `approach_used_hat`).
- **Server (`recorder`)**: 5 neue Spalten in `touchdowns`-Tabelle (idempotente ALTER), insertTouchdown extrahiert mit boolToInt-Helper, /api/touchdowns liefert sie typed.
- **Webapp**: TouchdownDto erweitert. Neue ApproachStabilityCard als eigene Datei (`_ApproachStabilityCard.tsx`) mit responsivem 4-tile-Grid, Coaching-Texten, Late-RWY-Change-Pill.
- DB-Backup pre-deploy: `aeroacars-live.db.backup-pre-approach-stability-v2`.

### ⚠ Hinweise

- Pre-v0.5.25-Touchdowns zeigen die alte σ-Auswertung als Fallback in der Card (mit Hinweis).
- HAT-Window erfordert dass arr_airport_elevation_ft im phpVMS-API-Cache landete — passiert beim Bid-Pickup ohnehin via `airport_get`-Command. Wenn nicht: Fallback auf AGL mit Confidence-Warnung.
- 3°-Glide-Slope-Target ist nur sekundär — der primäre Score (V/S-Jerk) funktioniert sim/aircraft-agnostisch und ist NICHT auf 3°-ILS-Profile zugeschnitten.

---

## [v0.5.24] — 2026-05-08

🎯 **Pitch-Sign-Fix für MSFS + frame-genaues Wheels-Up-Capture für Tail-Strike-Detection.** Plus Client-Version-Tag im MQTT-Stream für Version-Compliance-Tracking.

### 🐛 Behoben

**1. MSFS pitch-sign invertiert (= alle MSFS-Pilot-PIREPs hatten falsches Vorzeichen)**

MSFS-SimConnect hat eine inverse Konvention: `PLANE PITCH DEGREES` reportet **positive Werte wenn die Nase UNTER dem Horizont** ist (= Universal-Aviation-Konvention macht es umgekehrt: positiv = nose-up). AeroACARS las den Wert ohne Sign-Flip und schrieb daher invertierte Werte in alle MSFS-Pilot-PIREPs:

- A321-Flare bei +5° real → gespeichert als -5° (Pilot sieht „Nose-down landing" obwohl er normal flared)
- A321-Rotation bei +11° real → gespeichert als -11°
- Alle `Touchdown Sideslip`, `Landing Pitch`, `Takeoff Pitch` Custom-Fields betroffen

phpVMS DisposableSpecial-Triggers nutzten `abs()` und maskierten den Bug an der Trigger-Stelle. Aber Pilot-PIREP-Detail-Views zeigten die unsinnigen negativen Werte. **Fix:** sign-flip im MSFS-Adapter-Boundary (`telemetry.rs::SimSnapshot{}`-Builder + `adapter.rs` Touchdown-Block-Reader). X-Plane bleibt unangetastet — dortige `sim/flightmodel/position/theta`-DataRef ist konventions-konform.

**2. Takeoff-Pitch-Capture frame-genau (= Tail-Strike-Check präziser)**

Bisher wurden `takeoff_pitch_deg` / `takeoff_bank_deg` im Streamer-Tick gestempelt — das ist 3-30s Cadence je nach Phase, also potenziell mehrere Sekunden NACH dem echten Wheels-Up-Frame. Bei diesen Sekunden hat das Aircraft schon weiter pitch-up rotiert (Initial-Climb), der gestempelte Wert war oft 2-3° höher als der eigentliche Rotations-Pitch.

**Fix:** der bestehende 50Hz-Touchdown-Sampler-Task fängt jetzt auch den umgekehrten Edge ab (`prev_in_air=false → in_air_now=true` = Wheels-Up). Capture innerhalb 20ms im physischen Lift-Off-Frame. Phase-Transition-Code in `step_flight` verwendet `sampler_takeoff_pitch_deg.or(snap.pitch_deg)` als Priority-Chain — Sampler-Wert wins wenn vorhanden, sonst Streamer-Tick als Fallback. Wirkt beide Sims (X-Plane via `gear_normal_force_n` Edge, MSFS via `on_ground` Edge).

Bei tail-strike-empfindlichen Aircraft wie der A321 (~9.7° max safe pitch) erspart das 2-3° False-Positive-Drift im phpVMS DisposableSpecial Tail-Strike-Check.

### ✨ Neu

**3. `client_version`-Field im MQTT-PositionPayload**

Pro Position-Tick schickt der Client jetzt `client_version: "0.5.24"`. Der aeroacars-live-Monitor sieht damit pro Pilot welche Build-Version sendet — nützlich für:

- Version-Compliance-Tracking („Pilot X läuft noch v0.5.15, hat den Numeric-Fix nicht")
- Bug-Korrelation („alle disagreement-Touchdowns kommen von v0.5.18-")
- Updater-Monitoring („wieviele % der Piloten sind auf der neuesten Version?")

Server-seitig: das Field landet in `flights.last_position_json` (= im rohen JSON-Snapshot pro Pilot) und kann dort per `json_extract()` aggregiert werden. Native Tabellen-Spalte folgt in einem Server-Patch falls nötig.

### ⚠ Hinweise

- Existierende PIREPs werden NICHT retroaktiv korrigiert — nur neu eingehende MSFS-Touchdowns ab v0.5.24-Pilot-Version haben korrektes Pitch-Vorzeichen
- DisposableSpecial-Tail-Strike-Triggers funktionieren weiterhin via `abs()` — Pre-v0.5.24-Daten triggern korrekt, Post-v0.5.24-Daten ebenfalls (MSFS-Werte jetzt mit positivem Vorzeichen)
- Update via Auto-Updater empfohlen damit MSFS-Pilot-PIREPs ab sofort intuitive Pitch-Werte zeigen

---

## [v0.5.23] — 2026-05-08

🎯 **Forensik-Werkzeuge: aeroacars-live-Monitor sieht jetzt alles was der Client sieht.** Plus harte Fixes für Session-Splitting bei Hin/Rück-Flügen und leere ICAO-Felder.

### ✨ Neu

**1. Auto-Upload des kompletten Flight-Logs nach PIREP-File**

Nach erfolgreichem `file_pirep` lädt der Client das komplette JSONL-Logfile automatisch als gzip an aeroacars-live (`POST /api/flight-logs/upload`). Der VA-Owner kann es dann über den **„📥 Client-Log"-Button** in der History-Detail-View herunterladen — ohne den Piloten kontaktieren zu müssen.

- Auth via dieselbe MQTT-Cred-Pair die schon in der Provisioning-Phase im OS-Keyring liegt — keine zusätzliche Konfiguration
- Fire-and-forget — Failure ist non-fatal, JSONL bleibt lokal verfügbar
- Pilot kriegt Activity-Log-Eintrag mit Größen-Statistik (z.B. „2342 KB raw → 412 KB gzip (18% Kompression)")
- Bandwidth: typischer 2h-Flug ≈ 200-800 KB komprimiert
- Server-Storage: Auto-Purge auf VPS nach 90 Tagen vorgesehen

**Forensik-Wert:** der JSONL-Stream hat **mehr** als der MQTT-Server-Stream:
- ≈80 SimSnapshot-Felder pro Position-Tick (statt ≈35 via MQTT)
- Vollständiger User-Activity-Log
- PhaseChanged-Events mit altitude/groundspeed-Kontext bei jeder FSM-Transition
- Velocity-Body-Achsen, FCU-Setpoints, alle Lichter, COM/NAV-Radios, Autobrake, APU-State, Pushback-State, Seatbelts-/No-Smoking-Sign

**2. Touchdown-Algorithmen-Forensik im aeroacars-live-Monitor**

Bei jedem Touchdown laufen MSFS-Time-Tier- und X-Plane-Lua-30-Sample-Schätzer schon parallel — jetzt kriegt aeroacars-live alle Zwischenergebnisse und kann **Algorithmen-Disagreements** (= |xp_estimate − msfs_estimate| > 50 fpm) sichtbar machen für FSM-Edge-Case-Analyse.

Neue Felder in TouchdownPayload:
- `simulator`: „msfs" / „xplane" / „other"
- `vs_estimate_xp_fpm`: Lua-30-Sample-Schätzung
- `vs_estimate_msfs_fpm`: Time-Tier-Schätzung
- `vs_source`: welcher Pfad gewann („msfs_simvar_latched" / „agl_estimate_msfs" / „agl_estimate_xp" / „sampler_gear_force" / „buffer_min" / „low_agl_vs_min")
- `gear_force_peak_n`: X-Plane-Sampler-Wert
- `estimate_window_ms`: Window-Größe des gewinnenden Schätzers
- `estimate_sample_count`: Samples im Berechnungs-Fenster

Webapp-seitig: Touchdowns-Tab kriegt **„🔬 Touchdown-Forensik nach Simulator"-Card** + Sim-Filter („MSFS / X-Plane / Alle") + **„⚠ Disagreement"**-Filter + LandingAnalysis-Modal kriegt **„🔬 Algorithmen-Forensik"-Card** mit beiden Schätzern nebeneinander.

### 🐛 Behoben

**3. MQTT-Identity-Felder nicht mehr als leere Strings serialisiert**

PositionPayload-Felder `callsign` / `aircraft_icao` / `dep` / `arr` / `aircraft_registration` sind jetzt `Option<String>` mit `#[serde(skip_serializing_if = "Option::is_none")]`. Empty/whitespace-only Werte verschwinden komplett aus dem JSON statt als `""` gesendet zu werden.

**Hintergrund:** phpVMS-API liefert manchmal leere ICAO-Codes (Aircraft ohne `icao_code`-Feld in der VA-DB). Wenn der Client diese als `""` serialisierte, überschrieb der Server-COALESCE-UPSERT den vorher korrekt akkumulierten Wert in `flights.aircraft_icao` mit `""`. Resultat: Sessions starteten mit `aircraft_icao = NULL` obwohl der Pilot tatsächlich einen ICAO-getaggten Flieger flog.

### 🔬 Forensik-Workflow für VA-Owner (neu möglich ab v0.5.23)

1. Webapp Touchdowns-Tab → Filter „⚠ Disagreement" zeigt alle Landungen wo MSFS- und X-Plane-Schätzer auseinanderlagen
2. Klick auf Touchdown → 🔬-Card zeigt beide Werte + welcher gewann + Window-Konfidenz
3. Wenn |Δ| > 100 fpm und beide plausibel → Edge-Case lohnt anzuschauen
4. PilotHistory → Session-Detail → 📥 Client-Log → JSONL für rohe AGL-Samples + Activity-Log
5. Patch in `lib.rs` + Test-Cases mit gespeicherten JSONLs validieren

Vollständige Algorithmus-Referenz: [`docs/client-log-format.md`](docs/client-log-format.md).

### 📊 Server-Side (aeroacars-live, parallel deployed)

- DB-Schema: 3 neue Spalten in `flight_sessions` (client_log_path/size/uploaded_at) + 7 neue Spalten in `touchdowns` (simulator + 6 Forensik-Felder)
- Migration v1: Backfill aircraft_icao aus flights-Tabelle für historische Sessions
- Sustainable Session-Splitter mit drei orthogonalen Detektoren: Metadata-Mismatch / PIREP-Terminator / Phase-Regression — verhindert „Hin+Rück landet in einer Session"-Bug
- Defense-in-depth: Server-seitiger `sanitizeStr` im flights-UPSERT als Fallback für alte Clients ohne v0.5.23-Fix

### ⚠ Hinweise zum Update

- Backward-kompatibel: alte Server (ohne neue Forensik-Felder) ignorieren die neuen Optional-Payload-Felder. Neue Server (= aeroacars-live ab heute) extrahieren typed.
- Pilot-PCs ohne v0.5.23 schicken weiter PositionPayload mit `""` als Empty-Marker — Server-Defense fängt das ab.
- Bestehende Sessions in der DB bleiben unverändert (Migration v1 fixt nur was sicher fixbar ist).

---

## [v0.5.13] — 2026-05-07

🎯 **X-Plane Touchdown jetzt bit-genau LandingRate-1-aligned (Lua adaptive 30-sample method).**

Pilot-Bericht 2026-05-07: X-Plane-Flug MYNN→MBGT auf v0.5.11/v0.5.12 zeigte -394 fpm Touchdown — LandingRate-1.lua-Tool im selben Sim sagte 273 fpm. ~44% zu hoch.

**Ursache:** Mein bisheriger Time-Tier-Estimator (750ms / 1s / 1.5s / 2s / 3s / 12s mit fixen Min-Sample-Counts) ist zu starr. Bei niedriger X-Plane-RREF-Rate fallen kurze Tiers wegen Sample-Underflow durch, längere Tiers gewinnen — und ziehen Pre-Flare-Sinkraten in die Touchdown-Berechnung mit rein.

**Lua's Methode** (LandingRate-1, Dan Berry 2014+) macht's anders:
```lua
new_table("lrl_agl", 30)  -- 30 Samples, NICHT 1 Sekunde fix
```
Adaptives Fenster — bei 60 fps Render = ~0.5s, bei 30 fps = ~1s, bei 10 fps = ~3s. Selbstkalibrierend, robust gegen Framerate-Schwankungen.

### 🐛 Behoben

**X-Plane Touchdown-Capture komplett auf Lua-Style umgestellt.**

| Sim | Algorithmus | Datei |
|---|---|---|
| **X-Plane** (NEU) | Lua-style 30-sample adaptive AGL-Δ | `lib.rs` + `plugin.cpp` |
| MSFS (UNVERÄNDERT) | Time-tier estimator als Fallback nach latched SimVar | `lib.rs` |

**Schlüssel-Änderungen für X-Plane:**
- Neue Funktion `estimate_xplane_touchdown_vs_lua_style` — nimmt die letzten 30 Samples aus dem Sampler-Buffer, berechnet AGL-Avg-Midpoint-Rate exakt wie Lua's `lrl_agl` table
- Plugin (`plugin.cpp`): Time-Tier-Loop entfernt, durch 30-Sample-Method ersetzt
- AGL-Guards bleiben unverändert (TD ≤ 5 ft / on_ground=true, Window-Start ≤ 250 ft)
- Plugin sendet weiter `captured_vs_source` Diagnose-Metadata, jetzt `"lua_30_sample"`

### MSFS unverändert

**Nichts MSFS-relevantes wurde angefasst.** v0.5.12-Behavior für MSFS bleibt 1:1:
1. Latched SimVar (PLANE TOUCHDOWN NORMAL VELOCITY) — primary, GEES-aligned
2. Time-tier AGL-Δ — fallback (separate Funktion `estimate_xplane_touchdown_vs_from_agl` bleibt erhalten)
3. Buffer-Min — last resort

**Sampler bleibt für MSFS explizit aus** (war v0.5.12-Fix gegen Spike-Contamination).

### Validation

| Flug | Sim | v0.5.12 | v0.5.13 (erwartet) |
|---|---|---|---|
| MYNN→MBGT (Pilot) | X-Plane 12 | -394 fpm | ~-273 fpm (matcht LandingRate-1.lua) |
| 11 Flüge (Pete + Michael) | MSFS | korrekt seit v0.5.12 | unverändert korrekt |

### 🛠 Intern

- Tests: 87 grün
- `agl_estimate_xp` (Lua-style) und `agl_estimate_msfs` (time-tier) koexistieren als getrennte Funktionen
- Plugin baut clean auf Win/Mac/Linux via CI
- Frontend: 0 Änderungen

---

## [v0.5.12] — 2026-05-07

🚨 **KRITISCHER MSFS-Hotfix — Touchdown-Capture wieder GEES-aligned wie pre-v0.5.x.**

Pilot-Bericht: MSFS-Flug Lufthansa LH595 DNAA→EDDF zeigte -1173 fpm Touchdown bei G 1.12 — physikalisch widersprüchlich. Volanta + LHA-Tools sagten -560 fpm, MSFS-internal latched SimVar -419 fpm. Plus 11 weitere MSFS-Flüge analysiert (Pilot „Pete"): bei 90% war die latched SimVar `null`, Werte kamen aus Fallback-Pfaden — manche kontaminiert durch Sampler-Spike-Artefakte.

### 🐛 Behoben

**Bug-Klasse:** v0.5.0+ hat den X-Plane-Style Sampler (`sampler_touchdown_vs_fpm` via fnrml_gear bzw. on_ground-Edge-Fallback) in `step_flight()` auch für MSFS-Flüge einreihen lassen. MSFS hatte vorher (v0.3.5–v0.4.3) eine saubere zweistufige Logik: `latched MSFS SimVar → buffer-min`. Mit v0.5.0 schob sich der Sampler **vor** den latched-Wert in der Priority-Chain — und bei MSFS-Touchdown-Frames liefert der Sampler oft eine Spike-Reading durch Gear-Contact-Rebound-Oszillation.

**Fix — Sim-aware Capture-Trennung:**

```
MSFS-Pfad:
  1. snap.touchdown_vs_fpm  ← MSFS-latched SimVar (PLANE TOUCHDOWN
                              NORMAL VELOCITY — frame-genau, vom Sim
                              selbst gemessen, GEES-aligned)
  2. AGL-Δ Estimator        ← Geometrische Wahrheit als Fallback
                              für die ~90% der Flüge wo MSFS die
                              latched SimVar nicht setzt
  3. Buffer-Min (AGL≤250)   ← Last-resort

X-Plane-Pfad (unverändert seit v0.5.11):
  1. AGL-Δ Estimator (LandingRate-1)
  2. sampler_touchdown_vs_fpm (fnrml_gear)
  3. Buffer-Min
  4. low_agl_vs_min_fpm
```

**Schlüssel-Änderungen:**

- **Sampler-Pfad explizit AUS für MSFS** — der Sampler-Capture wird bei `is_msfs == true` gar nicht mehr konsultiert
- **AGL-Guard relaxed:** Touchdown-Sample wird akzeptiert wenn `on_ground=true` ODER `AGL ≤ 5 ft` (vorher nur strict AGL≤5). MSFS reportet AGL ≈ 9-14 ft auch bei `on_ground=true` — sim-quirk, nicht pre-touchdown
- **`negative_only` Filter** auf alle Quellen — physikalisch unmögliche positive „Landing-Rates" werden geblockt

**Validation:**

| Flug | Pilot | v0.5.11 (kaputt) | v0.5.12 (Fix) |
|---|---|---|---|
| LH595 DNAA→EDDF (B738) | Michael | -1173 fpm phantom | ~-419 fpm (matcht MSFS-internal) |
| 11 MSFS-Flüge (EDDF-Routen) | Pete | -132 bis -346 (zufällig OK) | konsistent über AGL-Δ-Pfad |
| Pre-v0.5.x Verhalten | (jede Pilot) | n/a | **wiederhergestellt + besser** |

### 🛠 Intern

- `step_flight()` enthält jetzt `match snap.simulator { ... }`-Branch
- AGL-Δ Estimator akzeptiert MSFS-AGL-Quirk (on_ground=true override)
- 87 Tests grün (alle 5 X-Plane-Touchdown-Regression-Tests bleiben gültig)
- Wirkt für alle MSFS-Versionen (Msfs2020 + Msfs2024)

---

## [v0.5.11] — 2026-05-07

🚀 **Großes Release — drei zusammenhängende Themen:**
1. **FSM-Audit** für alle Flugzeug-Klassen (Airliner / GA / Heli / Glider / Seaplane) inkl. Touch-and-Go, Go-Around, Holding-Pattern, Pause/Slew-Robustheit
2. **X-Plane Touchdown-Erfassung neu architektiert** nach LandingRate-1-Methode (AGL-Δ statt VSI), Plugin entmachtet
3. **MQTT Live-Tracking** zur aeroacars-live VPS — komplett unsichtbar im Hintergrund

87 Tests grün. Frontend ohne Änderungen.

---

### 🛩 Teil 1: FSM-Phasen-Audit (alle Aircraft-Klassen)

Pilot-Frage: „können wir alle Flugphasen für GA / Airliner / Heli prüfen?" — ja. v0.5.11 ist das Ergebnis einer vollständigen FSM-Audit mit Tiefen-Analyse der False-Positive-Risiken **bevor** gepusht wurde.

**🚁 Helikopter vertikaler Start aus Taxi**
TaxiOut→TakeoffRoll erwartet GS>30 kt am Boden, Helis erreichen das nie. Vorher: FSM hängt für ganzen Flug in TaxiOut.
→ Fix: TaxiOut → Takeoff direkt wenn `on_ground` true→false + AGL>5 ft + VS>100 fpm (Hardening gegen on_ground-Flicker).

**🚁 Helikopter pure-hover Departure aus Boarding**
Heli die direkt vom Gate vertikal abheben gehen nie auf TaxiOut → stuck in Boarding.
→ Fix: Boarding → Takeoff direkt + AGL>3 ft + VS>100 fpm.

**✈ Glider (Tow + Winch)**
engines>0 Anforderung in Heli-Pfaden gedroppt → Glider-Tow funktioniert (Glider ist airborne mit GS>0 aber engines=0).

**🛟 Seaplane Wasser-Operationen**
Boarding→TaxiOut akzeptiert jetzt Wasser-Oberfläche (`AGL<5 + |VS|<50` ≈ ground-equivalent). TaxiOut→Takeoff Catchall für Seaplanes wo on_ground=false bleibt: `!on_ground + AGL>50 + VS>100 + !slew + !paused`.

**🛩 GA Niedrigflug-Sackgasse**
Cessna mit Cruise auf 3000 ft AGL erreichte vorher nie Climb→Descent (braucht VS<-500). Climb→Descent triggert jetzt in DREI Szenarien:
- Standard TOD (Airliner): vs<-500 + lost>200 ft
- Low-altitude approach: vs<-100 + AGL<3000 + lost>500 ft
- Near-ground catchall: AGL<2000 + lost>800 ft + vs<0

**🔄 Touch-and-Go + Go-Around: climb_peak_msl Reset**
Beide Handler springen zurück zu Climb, aber der climb_peak_msl-Tracker wurde vorher nur bei Takeoff→Climb zurückgesetzt → Stale-Peak nach T&G/GA hätte mein neuen Low-Altitude-Trigger fälschlich feuern lassen.

**⏸ Pause + Slew Guard**
Während sim-pause oder slew-mode friert die FSM-Logik ein (kein Phasenwechsel), aber Position-Recording, Distanz-Tracking, Heartbeat laufen weiter. Verhindert dass eingefrorene snapshots Holding-Detektor-Timer fälschlich ablaufen lassen.

**🎯 NEUE Phase: Holding**
ICAO-konforme Holding-Pattern-Erkennung (sustained turn 90s + level flight). Triggert aus Cruise (high-altitude hold) oder Approach (low-altitude approach hold). Exit über bank<5° für 30s ODER aktiver Sinkflug → Approach.

**Audit-Endergebnis:**

| Aircraft | Vorher | Nach v0.5.11 |
|---|---|---|
| Airliner FL340 | ✅ | ✅ unverändert |
| Cessna 172 @ 3000 ft | ❌ stuck in Climb | ✅ alle Phasen |
| Bell 407 vertikal | ❌ stuck in TaxiOut | ✅ alle Phasen |
| EC135 pure-hover | ❌ stuck in Boarding | ✅ alle Phasen |
| Glider Aerotow / Winch | ❌ engines>0 lockt aus | ✅ alle Phasen |
| Seaplane (Wasser) | ❌ stuck in Boarding | ✅ alle Phasen |
| Pattern + Touch-and-Go | ⚠️ stuck nach 2. Anflug | ✅ Multi-T&G stabil |
| Missed Approach + GA | ⚠️ 2. Approach instabil | ✅ stabil |
| ATC Holding-Pattern | (nicht erkannt) | ✅ neue HOLDING-Phase |

**⚠️ Verworfen aus pre-release v0.5.10:** Der dortige Climb→Cruise low-altitude-Pfad (vs.abs()<100 + lost.abs()<100) wäre während aktivem Climb fälschlich gefeuert (lost-from-peak ist immer ~0 beim aktiven Climb). Komplett rausgenommen — GA bleibt in Climb bis Descent.

---

### 🎯 Teil 2: X-Plane Touchdown-Erfassung — Architektur-Refactor

Pilot-Analyse 2026-05-07: „warum kriegen LandingRate.lua und Volanta plausible Werte und wir nicht? Plus: most-negative-anywhere-in-approach kann pre-flare-Sinkraten als Touchdown ausgeben."

**Bug-Klasse:** v0.5.5+ trackte den negativsten VS-Wert über den GANZEN Approach. Ein steiler Pre-Flare-Sinkflug bei 943 ft AGL (z.B. -1346 fpm) hätte den echten gentle Touchdown bei 0 ft AGL überschrieben → Phantom-Hard-Landing-Reports.

**Fix — neue clean Hierarchie für Touchdown-VS-Erfassung:**

1. **PRIMÄR: AGL-Δ Estimator** mit Window-Tiers (750 ms / 1 s / 1.5 s / 2 s / 3 s / 12 s sparse-fallback)
   - LandingRate-1-Algorithmus (etabliert seit ~2014, gleiche Methode wie Volanta)
   - **Strikte Guards:** AGL ≤ 5 ft am Touchdown-Frame, AGL ≤ 250 ft am Window-Start
   - Pre-flare-Höhen können physisch nicht in die Berechnung kommen
2. Sampler-Edge-Capture (negative_only filtered)
3. MSFS-latched Touchdown-SimVar (negative_only)
4. Tighter buffer-window-scan + AGL≤250 Filter (negative_only)
5. `low_agl_vs_min_fpm` (umbenannt von `approach_vs_min_fpm`, jetzt nur AGL≤250 trackend)

**`negative_only` Filter:** alle Fallback-Quellen werden gefiltert — eine positive Landing-Rate ist physikalisch unmöglich.

**Plugin entmachtet:**
- Plugin-Code spiegelt gleichen Algorithmus + AGL≤250-Limit
- Plugin-Buffer hat 128 Samples (~3.8 s history)
- Plugin sendet Diagnose-Metadaten (`captured_vs_source`, `captured_vs_window_ms`, `captured_vs_samples`) im Touchdown-Paket
- Plugin liefert weiterhin `captured_vs_fpm` aber Client kann mit eigener AGL-Estimate **überschreiben** wenn er bessere Samples hat
- Plugin-Reinstall **nicht zwingend** — alte Plugin-Versionen werden durch Client-Logik korrekt gefiltert

**5 Regression-Tests** (alle grün): rebound-VSI / pre-flare-spike / butter-landing / all-positive-VS / negative_only-Filter.

---

### 📡 Teil 3: MQTT Live-Tracking zur aeroacars-live VPS

**NEUE Crate** `client/src-tauri/crates/aeroacars-mqtt/` integriert. Komplett unsichtbares Hintergrund-Feature (KEINE UI, KEIN Settings-Tab, KEIN Toggle).

**Auto-Provisioning** beim Login:
- Client ruft `https://live.kant.ovh/api/provision` mit phpVMS-API-Key auf
- Server validiert API-Key gegen phpVMS-Backend, liefert MQTT-Credentials zurück
- Credentials werden im OS-Keyring gecacht — Re-Install = same credentials (idempotent)
- Logout flusht Cache + sauberer Shutdown

**5 Hook-Points im Streamer:**
- **Position** (high-frequency, retained, QoS 0) — bei jedem position-tick
- **Phase** (low-frequency, retained, QoS 1) — bei FSM-Phasenwechsel inkl. neue HOLDING-Phase
- **Touchdown** (event, QoS 1) — wenn `announce_landing_score` ein Score-Message generiert
- **PIREP** (event, QoS 1) — nach `file_pirep` success
- **Shutdown** (clean OFFLINE flush mit 200ms-Pause) — auf RunEvent::ExitRequested

**Sicherheitseigenschaften:**
- MQTT-Connect über `wss://live.kant.ovh/mqtt` (TLS via rustls, kein OpenSSL-dep)
- `try_send` mit bounded queue → broker stall kann NIE den Streamer hot-path blocken
- Provision-Failure ist non-fatal — AeroACARS funktioniert exakt wie ohne MQTT
- LWT (last-will-and-testament) sorgt dafür dass beim Crash der OFFLINE-Status kommt
- Topic-ACL: jeder Pilot kann nur in `aeroacars/<va>/<seine-id>/#` publishen

**Wichtig für VA-Admins:** der Server-seitige Monitor-Frontend muss noch um die neue `HOLDING`-Phase erweitert werden — bis dahin fällt das Frontend auf den raw-String zurück (kein Funktionsverlust, nur Cosmetics).

---

### 🛠 Intern

- Tests: 87 grün (82 vorher + 5 neue Regression-Tests)
- Backend kompiliert cross-platform clean
- Plugin baut auf Windows (Mac/Linux via CI)
- Frontend: 0 Änderungen
- pre-release v0.5.10 wurde verworfen (Climb→Cruise alt-path zu riskant, T&G/GA-Reset fehlte)

---

## [v0.5.9] — 2026-05-07

🩹 **Climb→Descent FSM-Bug: ein einzelner VS-Spike beendete den Steigflug.**

Pilot Michael (MSFS, EGPH→HEGN B738): bei Climb auf FL050 hat ein einzelner -742 fpm-Spike (Level-Off-Maneuver) die FSM auf Descent geflippt. Aircraft stieg weiter durch FL340 und cruiste, aber FSM blieb 50+ Min in Descent hängen weil es keinen Descent→Climb Rücktransitionspfad gibt.

### 🐛 Behoben

Climb→Descent verlangt jetzt **zusätzlich** dass das Aircraft **echten Höhenverlust** vom Climb-Peak hat (>200 ft MSL).

```
Vorher: vs < -500 fpm                                    → Descent
Jetzt:  vs < -500 fpm AND lost_from_climb_peak > 200 ft → Descent
```

Single-Sample-Spikes (Turbulenz, Auto-Pilot-Trim, ATC-Level-Off) werden gefiltert. Erst wenn das Aircraft tatsächlich >200 ft Höhe verliert, gilt's als Descent. Echter Top-of-Descent verliert sofort tausende Fuß → triggert problemlos.

### 🛠 Intern
- Neues Feld `climb_peak_msl` in FlightStats (persistiert)
- Reset bei Takeoff→Climb (Re-Takeoff nach Divert)
- Wirkt für **MSFS und X-Plane** (FSM ist sim-agnostisch)
- Tests: 82 grün

---

## [v0.5.8] — 2026-05-07

🎯 **Multi-Window AGL-Δ + Plugin-Update — komplette Algorithmus-Konvergenz mit Volanta-Niveau.**

Pilot-Hinweis: „Volanta nutzt kein Plugin mehr und kriegt trotzdem korrekte Werte." Bestätigt unsere Strategie — der AGL-Δ-Algorithmus aus v0.5.7 ist self-sufficient ohne Plugin. v0.5.8 robustifiziert ihn weiter.

### 🆕 Multi-Window AGL-Derivative

Statt nur 2 s evaluiert der Client/Plugin jetzt **drei Fenster gleichzeitig** (1 s, 2 s, 3 s) und nimmt das negativste:
- **Hard Landing** (kein Flare): alle drei Fenster geben gleiche Werte
- **Airliner-Standard-Flare** (~3 s): 2 s-Fenster fängt den Pre-Flare-Sinkflug
- **GA Long-Flare** (~5 s): 3 s-Fenster deckt den relevanten Slice ab
- **Floater** (lange flache Approach): 1 s-Fenster misst nur die letzten Sekunden = sanfte Butter-Rate

### 🆕 Plugin (v0.5.8) — gleiche Methode

Plugin's Ring-Buffer hat jetzt auch AGL-Werte (war vorher nur VS+Pitch). Multi-Window-AGL-Δ läuft im Plugin self-sufficient. Kombiniert mit running airborne-VS-min als Backup.

**Aber wichtig:** Plugin ist optional. Volanta beweist dass die UDP-RREF-Daten von X-Plane (Port 49000) reichen — der Algorithmus macht den Unterschied, nicht der Plugin.

### 🛠 Intern
- Client: drei parallele AGL-Fenster, most-negative wins
- Plugin: VS-Buffer von 64 → 128 Samples (~3.8 s history bei 30 fps)
- Tests: 82 grün

---

## [v0.5.7] — 2026-05-07

🎯 **Methoden-Wechsel: VS wird jetzt aus AGL-Δ berechnet (LandingRate-1-Algorithmus, seit ~10 Jahren in der X-Plane-Welt erprobt).**

Pilot-Frage „warum kommen LandingRate.lua und Volanta immer auf richtige Werte und wir nicht?" — weil die einen fundamental anderen Ansatz nutzen den wir bisher nicht hatten.

### 🐛 Behoben

**Vorher** lasen wir die Sinkrate direkt aus `local_vy` / `vh_ind_fpm` (Flight-Model-Output). Beim Flare reduziert das Flight-Model die VSI absichtlich auf nahe 0 für gutes Stick-Feel — der Flieger sinkt physikalisch noch weiter, aber die VSI-Anzeige lügt schon. Egal wie clever wir Buffer-Min-Suche oder Running-Min nutzen, die Quelldaten sind kompromittiert.

**Jetzt** nutzen wir denselben Algorithmus wie LandingRate-1.lua (Dan Berry, 2014+) und Volanta:

```
gVS = (current_AGL - avg_AGL_letzte_2s) / (Zeitspanne / 2) * 60
```

Statt VSI lesen wir die **tatsächliche AGL-Differenz** über ein 2-Sekunden-Fenster. Das ist reine Geometrie — die Geometrie kann nicht durch Flight-Model-Tricks verfälscht werden. Bei einem Anflug von 81 ft AGL → 0 ft in 2 Sekunden gibt das exakt den echten Sinkflug, unabhängig von dem was VSI behauptet.

**Most-negative-wins** Hierarchie beim Final → Landing:
1. **AGL-Differential** (PRIMÄR — geometrische Wahrheit, wenn Sample-Density ausreicht)
2. Running Approach-Min (v0.5.5 Fallback)
3. Sampler-Edge-Capture (v0.4.4 Edge-Detection)
4. Buffer-Window-Scan (Legacy)
5. Live snap.vs (Last resort)

### 🛠 Intern
- Tests: 82 grün
- AGL-Daten waren schon im snapshot_buffer, kein neues Tracking nötig
- Wirkt mit ODER ohne Plugin (rein client-seitig)
- Plugin-Algorithmus folgt in v0.5.8 (gleicher Ansatz im C++)

---

## [v0.5.6] — 2026-05-06

🩹 **Plugin-Pendant zur v0.5.5-Touchdown-Logik.**

v0.5.5 hat den Bug im Tauri-Client gefixt; v0.5.6 fixt jetzt auch den Plugin-Code damit beide Schichten konsistent korrekt sind. Plugin sendet jetzt von sich aus den richtigen Wert.

### 🐛 Behoben

Plugin trackt jetzt auch eine **`g_airborne_vs_min`** — den negativsten pitch-korrigierten VS-Wert über den GESAMTEN airborne Segment (ground→air bis air→ground). Beim Touchdown-Edge wird der Wert mit dem Lookback-Window-Min und dem Live-VS verglichen — most-negative wins.

Zusammen mit der v0.5.5-Client-Logik gibt es jetzt **doppelte Korrektheit**:
- Plugin liefert von sich aus richtige `captured_vs_fpm` aus dem ganzen Anflug
- Client überschreibt nochmal mit dem eigenen Tracker falls Plugin doch falsch liegt

Reset-Logik im Plugin:
- Bei jedem ground→air Edge (Takeoff, Go-Around-Lift-off): Tracker = 0
- Nach erfolgreichem Touchdown-Capture: Tracker = 0 (Touch-and-Go bereit)
- Bei Plugin-Reload (`XPluginStop`): Tracker = 0

### ⚠️ Pilot-Aktion

1. v0.5.6 Auto-Update annehmen (Tauri-Client)
2. Settings → Debug → **„Plugin installieren"** klicken (lädt v0.5.6-Plugin)
3. **X-Plane neu starten** — neuer Plugin lädt erst beim X-Plane-Start

Dann ist das Plugin self-sufficient korrekt, auch ohne Client-Tracker-Override.

---

## [v0.5.5] — 2026-05-06

🩹 **Hotfix: Touchdown-VS bei aggressivem Flare wird endlich richtig erfasst.**

Pilot-Test (B738, MWCR Pattern, score 60/100 „firm" mit absurden Werten **VS +57 fpm bei G 1.52**): die Werte sind physikalisch widersprüchlich — 57 fpm = unmerklich, G 1.52 = harte Landung. Echte Sinkrate war ca. -500 fpm während des Anflugs (sichtbar im JSONL bei AGL 81 ft).

### 🐛 Behoben

Der 50-Hz-Sampler hatte ein zu schmales Lookback-Fenster (500 ms) und konnte bei aggressivem Flare nur **Post-Touchdown-Rebound-Samples** im Buffer finden — alle mit positivem VS. Resultat: das Min-Search fand keinen Sinkflug, gab den Rebound-Wert zurück.

**Doppelte Verteidigung in v0.5.5:**

1. **Running Peak-Descent-Tracker (`approach_vs_min_fpm`).** Ab Approach-Entry wird jeden 20-ms-Tick der **kleinste pitch-korrigierte VS-Wert** über die gesamte Approach + Final-Phase getrackt — unabhängig vom Sampler-Buffer. Selbst wenn X-Plane nur 1-2 Hz RREF liefert, fängt das den echten Peak-Sinkflug ein. Reset bei jedem neuen Approach (Go-Around-sicher).

2. **Sampler-Lookback erweitert von 500 ms auf 2 s.** Belt-and-suspenders gegen Buffer-Race-Bedingungen bei niedrigen RREF-Raten.

Beim Final → Landing wird nun der **negativste der drei Werte** genommen: Sampler-Edge-Capture vs. Buffer-Window-Scan vs. Running-Approach-Min. Most-negative wins.

### 🛠 Intern
- Tests: 82 grün
- Patch wirkt **mit oder ohne** installiertes X-Plane-Premium-Plugin — Plugin gibt frame-genaue Werte direkt vom flight-loop, der Tracker ist Backup für Plugin-lose Setups
- Persistierung des Trackers nicht nötig — er lebt nur innerhalb einer einzigen Approach-Phase

---

## [v0.5.4] — 2026-05-06

🩹 **Hotfix: Pattern-Flüge auf niedriger Höhe bleiben in Cruise hängen.**

Pilot-JSONL-Log: kurzer MWCR → MWCR Pattern-Test (B738), Cruise-Höhe 5000 ft AGL, 16 Min Flugdauer, normale Landung mit Aufsetzen — Ergebnis: keine Landing-Rate erfasst, Phase ging direkt von Cruise → Arrived.

### 🐛 Behoben

**Bug 1: Cruise→Descent forderte > 5000 ft Höhenverlust.** Der Cruise-Peak war bei 5002 ft MSL, beim Aufsetzen MSL 29 ft → Höhenverlust 4973 ft, **knapp unter** der 5000-ft-Schwelle. FSM blieb in Cruise, der Universal-Arrived-Fallback hat dann am Ende stumm direkt nach Arrived gesprungen — ohne durch Final→Landing zu gehen, also keine Touchdown-Erfassung.

Fix: Eskape-Klausel — Cruise→Descent triggert jetzt entweder bei (a) > 5000 ft Höhenverlust (wie bisher, für Airliner-TOD) **oder** (b) AGL < 3000 ft + Sinkflug (Pattern/GA-Bereich). Step-Downs bei FL360 lösen weiterhin keinen falschen Phasenwechsel aus.

**Bug 2: Universal-Arrived-Fallback verlor Touchdown-Daten.** Selbst wenn der 50-Hz-Sampler den Edge intern erfasst hatte, wurden VS/G nicht in den PIREP geschrieben weil der Code-Pfad „Final→Landing" der einzige war der das tat.

Fix: Rescue-Pfad — wenn Arrived-Fallback feuert UND der Sampler einen Touchdown gespeichert hat, werden `landing_rate_fpm`, `landing_peak_vs_fpm`, `landing_g_force`, `landing_peak_g_force` aus den Sampler-Werten gefüllt. Zweite Verteidigungslinie selbst wenn die FSM-Hauptkette ausfällt.

### 🛠 Intern
- Tests: 82 grün
- Beide Fixes wirken auch ohne installiertes X-Plane-Premium-Plugin (Sampler ist nativer Teil des Tauri-Clients)

---

## [v0.5.3] — 2026-05-06

🚨 **KRITISCHER Hotfix — Port-Konflikt mit X-Plane behoben.**

Pilot-Bericht mit Screenshot der X-Plane-Netzwerkeinstellungen zeigte: „Fehler bei der Initialisierung des UDP-Netzwerkausgangs (Port 49001). Lokales Netzwerk wird deaktiviert." Mein Plugin hatte 49001 für die Loopback-Kommunikation gewählt — **das ist aber X-Planes eigener Sende-Port**. Beide Apps stritten um denselben Socket → X-Plane konnte sein UDP-Netzwerk nicht initialisieren.

### 🐛 Behoben

- **Port von 49001 → 52000** in Plugin (`AEROACARS_UDP_PORT`) und Client (`PREMIUM_UDP_PORT`). 52000 ist:
  - **Weit außerhalb** X-Planes 49000-49003 Bereich (Send/Receive)
  - **Nicht** der X-Plane-Connect-Port (49520, NASA-Research-Tool)
  - In IANA Dynamic-Range, kein bekannter Service
  - Komplett konfliktfrei für 99,9% der Setups

### ⚠️ Pilot-Aktion erforderlich

1. AeroACARS-Update auf v0.5.3 installieren (auto-update)
2. Settings → Debug → Plugin **neu installieren** (lädt v0.5.3-Plugin von GitHub)
3. **X-Plane neu starten** — die Fehlermeldung über deaktiviertes lokales Netzwerk verschwindet, X-Planes UDP-Netzwerk arbeitet wieder normal

Plugin- und Client-Port müssen synchron sein — die v0.5.3-Auto-Install-Funktion zieht automatisch das passende Plugin-ZIP, daher reicht ein Klick auf „Plugin installieren" nach dem Client-Update.

### 🛠 Intern

- Neuer Defensive-Comment-Block in beiden Source-of-Truth-Konstanten warnt explizit vor X-Planes 49000-49003 Range
- Tests: 82 grün (unverändert)
- Plugin-Source ist nur an einer Konstante geändert, alle anderen Logiken stabil

---

## [v0.5.2] — 2026-05-06

🩹 **Hotfix: kein flackerndes Konsolen-Fenster mehr beim Settings-Tab-Klick.**

Pilot-Feedback nach v0.5.1: „beim Klick auf den Tab Einstellungen öffnet sich ein unsichtbares Fenster". Das war eine echte (leere) `cmd.exe`-Konsole, die kurz aufflackerte und den Fokus stahl — verursacht durch den `reg.exe query` aus der X-Plane-Pfad-Auto-Erkennung.

### 🐛 Behoben
- **`CREATE_NO_WINDOW`-Flag** für den `reg.exe`-Subprocess. Windows zeigt jetzt keine Konsole mehr an, kein Fokus-Stehlen, kein Flackern.

Patch nur Windows-relevant. Mac/Linux unverändert.

---

## [v0.5.1] — 2026-05-06

🩹 **Hotfix für v0.5.0-Regression — Settings-Tab hängt beim ersten Öffnen.**

Pilot-Feedback nach v0.5.0-Install: „Einstellungsseite ist hakelig beim Scrollen, Sprache konnte erst nicht verstellt werden." Klassischer Synchronization-Bug — der neue X-Plane-Premium-Panel rief auf seinem ersten Render einen synchronen Tauri-Command (`xplane_detect_install_path`) auf, der intern `reg.exe query` als Subprocess startete. Auf dem Main-Thread = blockiert den ganzen IPC-Kanal für ~200-800 ms, während dem **kein einziger anderer Command** durchkommt — daher Sprachwechsel-Hang + Scroll-Lag.

### 🐛 Behoben

- **`xplane_detect_install_path` ist jetzt async + `spawn_blocking`** — der `reg.exe`-Query läuft auf einem Worker-Thread, IPC bleibt frei, Settings-Panel reagiert sofort.
- **`xplane_uninstall_plugin` ebenfalls async** — beugt potenziellem Stall bei langsamen `remove_dir_all` (Windows Defender, Netzlaufwerke) vor.

### 🛠 Intern

- Selbe Pattern wie `detect_running_sim` (das schon seit v0.3.0 async ist).
- Tests: 82 grün (unverändert).

---

## [v0.5.0] — 2026-05-06

🚀 **„X-Plane Premium" — Frame-genaue Touchdown-Erfassung via nativem Plugin.**

Größtes Feature seit Release: ein optionaler nativer X-Plane-Plugin (XPLM SDK 4.3.0, C++17), der die Touchdown-Edge **innerhalb** des X-Plane-Flight-Loops erfasst — frame-genau, mit 500 ms Lookback-Buffer für die Peak-Sinkrate. Löst endgültig die seit v0.4.2 jagende „6 fpm Landing Rate trotz harter Landung"-Klasse von Bugs.

### 🆕 X-Plane Premium Plugin

**Was es tut:**
- Liest `fnrml_gear` (Gear-Normalkraft) jeden Frame und erkennt den exakten Frame des Aufsetzens (xgs-Methode, etablierte X-Plane-Konvention seit ~10 Jahren).
- Ermittelt die Peak-Sinkrate aus einem 500 ms-Lookback-Ring-Buffer **vor** dem Edge — so dass das gemessene VS dem tatsächlichen Anflug entspricht, nicht dem schon ausgependelten Wert nach Bodenkontakt.
- Pitch-Korrektur: `vs × cos(pitch)` (xgs-Konvention) — projiziert Welt-Y-Geschwindigkeit auf die Body-Achse.
- Sendet einen einmaligen JSON-„touchdown"-Paket über UDP an die AeroACARS-App auf `127.0.0.1:49001`.
- Re-armiert sich bei AGL > 50 ft, Touch-and-Go funktioniert also korrekt.

**Cross-Platform:**
- Windows x64 (`win.xpl`, MSVC, statisches CRT — keine DLL-Abhängigkeiten beim Piloten)
- macOS Universal (`mac.xpl`, x86_64 + arm64 in einer Datei)
- Linux x64 (`lin.xpl`, GCC)

**Sicherheit (NIE den Sim crashen):**
- Alle DataRef-Handles NULL-geprüft, alle Errors via `XPLMDebugString` geloggt, nie propagiert.
- Compile mit `-fno-exceptions -fno-rtti` (keine C++-Exceptions über die C-ABI-Plugin-Boundary).
- Non-blocking UDP `sendto()` — kein Stallen des Flight-Loops, auch nicht wenn der Client offline ist.
- Keinerlei Filesystem-Writes, keine Registry-Edits — Plugin ist read-only gegen X-Plane-State.
- Sauberes Reverse-Order-Cleanup in `XPluginStop`.

**Wire Format:** Versionierte Line-delimited-JSON über UDP-Loopback. Schema-`v:1`, zwei Pakettypen: `telemetry` (jeden Tick) + `touchdown` (one-shot pro Landung).

### 🆕 Auto-Install im AeroACARS-Client

Settings → Debug → „X-Plane Premium Plugin"-Karte:
- **Auto-Erkennung** des X-Plane-Hauptordners (Windows-Registry · macOS Standard-Pfade · Linux Standard-Pfade)
- **Manueller Pfad-Override** wenn die Auto-Erkennung nichts findet
- **„Plugin installieren"-Button** lädt die zur installierten Client-Version passende Plugin-Zip von GitHub und entpackt nach `<X-Plane>/Resources/plugins/AeroACARS/`
- **Status-Badge** „📡 live" sobald das Plugin Pakete sendet

### 🆕 Listener im Tauri-Client

- Neuer UDP-Listener (`crates/sim-xplane/src/premium.rs`) bindet `127.0.0.1:49001`, parst JSON-Pakete, surft Status + Touchdown-Events nach lib.rs.
- Touchdown-Sampler: wenn ein Premium-Paket eintrifft, **überschreibt** dessen `captured_vs_fpm` / `captured_g_normal` die RREF-basierte Edge-Detection — Frame-Genauigkeit, kein UDP-Eviction-Race mehr.
- RREF-Pfad bleibt voll funktional: Piloten ohne Plugin merken keinen Unterschied, ihre Flüge laufen wie vorher.

### 🛠 Intern

- Neuer Workspace-Member `xplane-plugin/` mit Cross-Platform-CMake-Build
- X-Plane SDK 4.3.0 vendored unter `xplane-plugin/third_party/XPSDK430/` (BSD-Lizenz, freie Commercial-Use)
- 6 neue Unit-Tests für den Premium-Packet-Parser
- 3 neue Tauri-Commands: `xplane_premium_status`, `xplane_detect_install_path`, `xplane_install_plugin`, `xplane_uninstall_plugin`
- GitHub-Actions-Pipeline erweitert: Plugin-Build-Matrix (Win/Mac/Linux) + Plugin-Package-Job, der die drei `.xpl` zu `AeroACARS-XPlane-Plugin-vX.Y.Z.zip` zusammenfasst und ans Release uploaded
- Bilingual i18n (DE+EN) für alle neuen Strings

### 🐛 Behoben (X-Plane only)

- **Landing-Rate-Bug aus v0.4.2/v0.4.3 final beseitigt:** Sampler-side Edge-Detection auf `fnrml_gear` (statt nur Streamer-side `on_ground`-Flag). Funktioniert sowohl mit als auch ohne Premium-Plugin — ohne Plugin macht der Sampler die Edge-Detection auf seinen 50-Hz-Snapshots, mit Plugin übernimmt das Plugin frame-genau.
- **Pitch-Korrektur bei VS-Capture:** Konsistent mit xgs (`vs × cos(theta_rad)`) im Sampler und im Plugin.

---

## [v0.4.3] — 2026-05-05

X-Plane-spezifischer Touchdown-VS-Fix nach Pilot-Live-Test heute Abend.

### 🐛 Behoben (X-Plane only)
- **Landing-Rate / peak_vs_fpm war bei X-Plane immer ~0** auch bei klar härteren Landungen. Pilot-Log heute (EWL6822 LEPA→EDDG, A320, sichtbare Sinkrate -350 fpm beim Aufsetzen): AeroACARS scorete „smooth, peak_vs_fpm: +5.7" — Touchdown-Window enthielt nur Post-Rollout-Daten.

  **Ursache:** Wir lasen `sim/flightmodel/position/vh_ind_fpm` — das ist die **VSI-Anzeige** wie im echten Cockpit, mit absichtlichem Damping (mehrere Sekunden Smoothing). Beim physischen Touchdown ist der gesmoothte Wert schon nahe 0, der echte Sinkflug ist als langsamer „Decay" über die letzten Sekunden verteilt — im 500ms-Touchdown-Window nicht mehr als Peak erkennbar.

  **Fix:** Switch auf `sim/flightmodel/position/local_vy` — die rohe vertikale Y-Achsen-Geschwindigkeit (m/s, real-time, kein Smoothing). Konvertierung im Setter: `value * 196.8504` (= 3.28084 ft/m × 60 sec/min). Das ist der gleiche DataRef den die etablierten X-Plane-Landing-Rate-Plugins (xgs, LRM, „A New Landing Rate Display") seit ~10 Jahren verwenden.

  Bei MSFS unverändert (ist ohnehin ein anderer Code-Pfad mit SimConnect-`PLANE TOUCHDOWN NORMAL VELOCITY`).

### 🛠 Intern
- DataRef-Switch in `client/src-tauri/crates/sim-xplane/src/dataref.rs`
- Verifiziert gegen X-Plane Developer-Doku + Production-Plugins (xgs, LRM)
- Tests: 76 grün

---

## [v0.4.2] — 2026-05-05

UX-Polish nach Pilot-Feedback heute Abend.

### 🆕 Neu
- **PIREP-Erfolgs-Banner** im Cockpit-Tab nach erfolgreichem File. Grüner Banner mit Callsign + Route bleibt 8 s sichtbar, manuell schließbar via X. Vorher verschwand das ActiveFlightPanel still — Pilot wusste nicht ob's wirklich gefilt wurde oder hängengeblieben ist.
- **Hint-Banner im Landung-Tab** wenn keine SimBrief-Plan-Werte vorhanden sind (alle SOLL-Spalten leer wären). Erklärt warum statt nur stille Striche.
- **SimBrief-OFP-Status im Activity-Log** beim Flight-Start. Vorher: Fetch-Fehler nur in Tracing-Logs (unsichtbar für Pilot). Jetzt drei Activity-Log-Einträge je nach Outcome:
  - ✅ „SimBrief OFP geladen" mit Plan-Block / Trip / TOW
  - ⚠️ „SimBrief-OFP konnte nicht geladen werden" mit OFP-ID + Erklärung
  - ⚠️ „SimBrief-OFP-Fetch fehlgeschlagen" mit Error-Detail
  - ℹ️ „Kein SimBrief-OFP für diesen Flug" wenn der Bid gar keinen hatte

So sehen Piloten beim nächsten Mal sofort wenn der OFP-Fetch silently scheitert (was zum heutigen „Landung-Tab hat keine SOLL-Werte"-Bug geführt hat).

### 🛠 Intern
- Neue UI-Component für PIREP-Success-Banner in `CockpitView.tsx` mit 8s Auto-Dismiss + Manual-Close
- i18n DE+EN für alle neuen Texte
- Tests: 76 grün

---

## [v0.4.1] — 2026-05-05

Real-Pilot-Workflow: was tun wenn der Sim mid-flight wegbricht?

### 🆕 Neu: Sim-Disconnect-Handling

Wenn der Streamer länger als 30 s keine brauchbaren Sim-Daten mehr bekommt (Sim-Crash, Quit, Window-Switch-Glitch), passiert jetzt:

1. **Flug wird automatisch in den Pause-Status gesetzt** — keine Position-Updates mehr an phpVMS, kein Phase-FSM, kein Activity-Log-Spam
2. **Die letzten bekannten Werte werden eingefroren** und sowohl im **Activity-Log** als auch in einem **gelben Banner im Cockpit-Tab** angezeigt:
   - Latitude / Longitude
   - Heading + Altitude
   - Fuel on Board
   - ZFW (Leergewicht)
3. **Heartbeat läuft weiter** — phpVMS' Live-Tracking-Cron killt den PIREP NICHT während der Pause (sonst wäre nach 2 h Schluss)
4. **„🔄 Flug wiederaufnehmen"-Button** im Banner — Pilot startet den Sim neu, lädt das Flugzeug an die richtige Position (oder bewusst andere — kein 5-NM-Restriction wie bei smartCARS), klickt den Button → Streamer macht weiter
5. **KEIN Auto-Resume:** auch wenn der Sim plötzlich wieder Daten liefert wartet das Backend auf den manuellen Klick (sonst würden Mid-Air-Position-Sprünge wild ins PIREP gehen)
6. **Reposition-Audit-Log:** beim Resume wird die Distanz zwischen alter und neuer Position berechnet. Bei großen Sprüngen (> 500 nm) als WARN-Level damit's für VA-Audits sichtbar ist
7. **Distance-Reset bei Resume:** Reposition-Sprung fließt **nicht** in die geloggte Flugdistanz ein. PIREP `distance_nm` zeigt nur tatsächlich geflogene Distanz, der Reposition-Δ wird separat als Activity-Log-Zeile festgehalten

Bewusst KEINE 5-NM/2000-ft-Restriktion wie bei smartCARS — der Pilot entscheidet wo er weitermacht, der Audit-Log macht's nachvollziehbar.

### 🛠 Intern
- Neuer Tauri-Command `flight_resume_after_disconnect` mit Δ-Distanz-Audit
- `FlightStats` erweitert um `paused_since` + `paused_last_known: PausedSnapshot`
- `ActiveFlightInfo` flow-through dieser Felder ans Frontend
- Neue Cockpit-Component `<DisconnectBanner>` (i18n DE+EN)
- Konstanten: `SIM_DISCONNECT_THRESHOLD_S=30`, `REPOSITION_WARN_DELTA_NM=500.0`
- Tests: 76 grün

---

## [v0.4.0] — 2026-05-05

Erstes Minor-Release der 0.4er-Reihe. Hauptthema: **Discord-Integration**.

### 🎉 Neu — Discord-Webhook

Vier Lifecycle-Events werden jetzt automatisch in den GSG-Discord-Channel gepostet, im Stil etablierter VA-Bots:
- ✈️ **Takeoff** (grün) — mit Block-Fuel + Plan-Δ + TOW
- 🛬 **Landung** (orange) — mit Landing-Rate + Score + Distance
- 📋 **PIREP filed** (violett) — kompletter Flugbericht
- ⚠️ **Divert** (amber) — mit Geplant/Tatsächlich-Vergleich

Layout angelehnt an den GSG-Bot-Stil:
- Author-Bar oben mit phpVMS-Pilot-ID + Name (z.B. „GSG0001 - Thomas K")
- Title als „Flight CHH3184/C.PF has landed"
- 3-Spalten-Felder: Dep.Airport / Arr.Airport / Equipment
- 2-Spalten-Felder: Flight Time / Distance
- **Großes Airline-Logo unten** — kommt direkt aus phpVMS (`bid.flight.airline.logo`), keine externe Hosting-Abhängigkeit. Wenn die VA das Logo-Feld in phpVMS pflegt, erscheint es automatisch.

Webhook-URL ist hardcoded für GSG (`#flights`-Channel). Posts laufen fire-and-forget (`tokio::spawn`) — Discord-Latenz blockt nie den Flugverlauf.

### 🛠 Intern
- Neues Modul `client/src-tauri/src/discord.rs` mit Embed-Builder + HTTP-Helper
- `ActiveFlight`/`PersistedFlight` erweitert um `airline_logo_url: Option<String>` (aus Bid-Relation; persistiert für Resume)
- `AppState.cached_pilot: Mutex<Option<(String, String)>>` — wird beim Login + Refresh aus dem phpVMS-Profile gefüllt, für die „GSG0001 - Pilot Name"-Zeile
- Discord Rich Presence Service (Crate `discord-rich-presence v1`) eingebaut aber noch nicht gewired — kommt in v0.4.1
- Tests: 76 grün

---

## [v0.3.5] — 2026-05-05

Drei X-Plane / phpVMS-Bugs nach Pilot-Test heute morgen.

### Behoben
- **MSL-Höhe weicht im Cruise um ~1.000 ft ab.** Wir lasen `sim/flightmodel/position/elevation` (= TRUE MSL, geographische Höhe über Sea Level), das aber bei nicht-ISA-Atmosphäre vom Indicated-Altitude abweicht. Pilot Michel D. sah heute auf YBBN→NWWW bei FL390 / OAT −46 °C → AeroACARS meldete 40.009 ft, PFD korrekt 39.000 ft (Differenz exakt die ISA-Deviation × 4 ft/°C). Im Sinkflug konvergierten die Werte wieder. Jetzt: `sim/cockpit2/gauges/indicators/altitude_ft_pilot` — Indicated, exakt was der Pilot sieht.
- **QNH-Anzeige zeigte unmögliche Werte (z.B. 198 hPa).** Der gelesene DataRef `sim/weather/barometer_current_inhg` ist der **Umgebungsdruck am Flugzeug**, nicht die Kollsman-Einstellung. Bei FL390 sind ~187 hPa Außendruck korrekt — aber das ist nicht was im Höhenmesser-Fenster steht. Jetzt: `sim/cockpit2/gauges/actuators/barometer_setting_in_hg_pilot` — die tatsächliche Altimeter-Setting (1013 hPa bei STD, real QNH bei lokal). Achtung: heißt `barometer_*` nicht `altimeter_*` (X-Plane-Naming-Inkonsistenz, gegen FlyWithLua + X-RAAS-Plugin verifiziert).
- **„Geflogene Route: 100%" während Boarding** auf der phpVMS-Live-Seite. v0.3.0 versuchte das durch Senden von `None` als `distance` während der Pre-Flight-Phase zu beheben — funktionierte nicht weil PHP's `empty()` sowohl `null` als auch `0` als „empty" erkennt und den 100%-Fallback triggert (1/1 = 100). Jetzt: minimaler Floor von 0.001 nm bis echte Distanz akkumuliert ist → `empty(0.001)` = false → Division läuft real → 0.001 / Plan-Distanz ≈ 0% bis Pushback.

---

## [v0.3.4] — 2026-05-04

Hot-Patch: v0.3.3 hatte einen TypeScript-Build-Fehler im CI (`'fnumMismatch' is declared but its value is never read`) — die Build-Jobs für Windows + macOS schlugen fehl, der `publish`-Step wurde geskipped, also kamen keine Installer am Release an. Inhaltlich = v0.3.3, nur sauber gebaut.

### Behoben
- **TS6133-Fehler in `BidsList.tsx`** — Cross-Product-Match-Logik entfernt nachdem v0.3.3 sie aus `ofpMismatch` rausgenommen hatte; die Variablen waren danach unused. Strict-Mode tot.

---

## [v0.3.3] — 2026-05-04 *(broken release — keine Build-Artefakte)*

Patch nach v0.3.2 — drei kleine UX-Fixes rund um die OFP-Mismatch-Detection.

### Behoben
- **Falscher OFP-Mismatch-Banner bei legitimen Plan-Varianten.** Der Match zwischen Bid-Flugnummer und SimBrief-OFP-Callsign war zu strikt. Beispiel: Bid „EWL 4368", OFP-Callsign „EWL4TK" (Pilot nutzt persönlichen ATC-Callsign in SimBrief). Der Banner feuerte fälschlich „SimBrief-OFP passt nicht zur Buchung", obwohl Aircraft + Origin + Destination alle übereinstimmten. Match-Logik jetzt bidirektional als Cross-Product aller Bid-Variants (Flight-Number + Callsign, mit/ohne Airline-ICAO-Prefix) gegen alle OFP-Variants. Plus: Flight-Number-Diff alleine triggert NICHT mehr den Banner — Aircraft / Origin / Destination sind die einzigen Signale stark genug für einen „altes OFP"-Befund. Eine Callsign-Differenz bei sonst stimmiger Route + Aircraft ist fast immer ein legitimer persönlicher ATC-Callsign.
- **Kein Hinweis wenn überhaupt kein OFP an die Buchung gebunden ist.** Vorher rätselte der Pilot warum die Plan-Cards leer sind. Jetzt blauer Info-Banner: „Kein SimBrief-OFP für diese Buchung — erstelle einen auf simbrief.com".

---

## [v0.3.2] — 2026-05-04

Patch-Release direkt nach v0.3.1. Zwei Pilot-Reports vom Live-Test:

### 🐛 Behoben
- **„Discard flight" / „Forget locally" / „Logs löschen" funktionierten auf macOS nicht.** Tauri auf macOS nutzt WKWebView, und WKWebView droppt `window.confirm()` und `window.alert()`-Aufrufe stillschweigend — der Dialog kommt nie, der Aufruf returnt sofort `false`/`undefined`, der Button-Handler springt raus. Auf Windows (WebView2) hat's funktioniert, daher fiel's vorher nicht auf. Alle 6 betroffenen Stellen (`ActiveFlightPanel`, `LandingPanel`, `ActivityLogPanel`, `SettingsPanel`, `ResumeFlightBanner`) nutzen jetzt eine neue In-App-`<ConfirmDialog>`-Component (kein Native-Dialog, kein Plugin, garantiert cross-platform).
- **Loadsheet im Cockpit verglich gegen einen veralteten OFP-Stand.** Real-Pilot-Workflow: Pilot regeneriert auf simbrief.com einen neuen OFP nachdem der Flug schon gestartet ist (Pax/Cargo/Reserve geändert). AeroACARS hatte die Plan-Werte beim Flight-Start eingefroren — der „Refresh"-Button im My-Flights-Tab refreshte nur die Bid-Card-Vorschau, nicht den aktiven Flug-Snapshot. Resultat: Loadsheet zeigte falsche Δ-Werte gegen die Plan-Variante, die der Pilot gar nicht mehr nutzt.

### ✨ Neu
- **OFP-Refresh-Button im Cockpit-Tab** (sichtbar in den Phasen Preflight / Boarding / TaxiOut). Klick → Backend zieht den aktuellen Bid + frische SimBrief-OFP, überschreibt `planned_block` / `planned_tow` / `planned_zfw` / `planned_route` / `planned_alternate` / `max_*` und persistiert sofort. Das Loadsheet vergleicht ab dem Klick gegen den neuen Plan. Activity-Log-Eintrag „OFP refreshed" mit den drei Hauptwerten als Audit-Trail.
- **`<ConfirmDialog>` + `useConfirm()`-Hook** als neue UI-Primitive. Kann von künftigen Components mitgenutzt werden — Esc cancelt, Enter confirmt, Backdrop-Click cancelt, optionaler `destructive`-Mode (rot statt blau). i18n-Keys: `confirm_dialog.default_title` / `confirm` / `cancel`.

### 🛠 Intern
- Neuer Tauri-Command `flight_refresh_simbrief()` — pullt Bid → SimBrief-OFP → mass-assigned planned_*-Felder unter dem `active_flight`-Lock. Verifiziert Bid-ID nach dem Await damit ein parallel-discarded Flight nicht überschrieben wird.

---

## [v0.3.1] — 2026-05-04

Konsolidierter 0.3.x-Release. Bündelt das komplette SimBrief-Integration-Paket (Phase H.7), erweiterte X-Plane-Telemetrie, Live-Block-Fuel-Fix, das Loadsheet-Feature, OFP-Mismatch-Detection, UX-Polish nach dem GSG-Live-Test sowie das **neue Divert-Manual-PIREP-Routing**.

### 🌟 Highlights
- **Divert-Manual-PIREP** — landet jetzt sauber im PENDING-Bucket des VA-Admins statt fälschlich auto-akzeptiert zu werden. Pilot klickt „Divert nach XXX" → PIREP wird als manueller Eintrag mit dem tatsächlichen Landing-Airport für Admin-Review markiert.
- **Loadsheet-Feature** — Live-Anzeige Block-Fuel / ZFW / TOW während Boarding plus Score-Bewertung im Landung-Tab.
- **SimBrief Soll/Ist-Vergleich** — kompletter Plan-vs-Actual-Block im Landung-Tab, farbcodiert mit aviation-tauglichen Schwellen (5/10 %).
- **OFP-Mismatch-Detection** — erkennt wenn der zuletzt von SimBrief geladene OFP nicht zur aktuellen Buchung passt.
- **X-Plane Auto-Reconnect + neue Telemetrie** — startet sich selbst neu, liefert Wing-/Wheel-Well-Lights + TO-Config-Warning für 737 Zibo/LevelUp + universelle Autobrake/XPDR-Labels.

> Hintergrund: v0.3.0 war als Tag bereits gesetzt, aber ohne Release-Notes. Statt rückwirkend zu rekonstruieren bündeln wir alles unter v0.3.1 — alles, was seit v0.2.4 reingegangen ist.

### 🐛 X-Plane Bug-Fixes
- **Gear-DataRef [0]-Index-Fix.** `sim/flightmodel2/gear/deploy_ratio[0]` mit explizitem Index — fixt „Gear UP am Boden" bei LevelUp 737 (gleiches RREF-Pattern wie der Engine-Bug damals).
- **Auto-Reconnect hart abgesichert.** Re-Subscribe-Loop alle 5 s wenn State ≠ Connected. Funktioniert in allen Szenarien: AeroACARS startet vor X-Plane, X-Plane Restart, X-Plane Crash, Aircraft-Wechsel mit Daten-Stillstand.

### ✨ X-Plane Erweiterungen
- **Autobrake-Stufe als Label** (universell, alle Aircraft) — `RTO/OFF/1/2/3/MAX`.
- **XPDR-Mode als Label** (universell) — `OFF/STBY/XPNDR/TEST/ALT/TA/TA-RA`.
- **Wing-Lights** (Boeing 737 Zibo / LevelUp).
- **Wheel-Well-Lights** (737 Zibo / LevelUp).
- **Takeoff-Config-Warning** (737 Zibo / LevelUp) — Warnung im Cockpit-Status wenn Flaps / Trim / Spoiler nicht für TO konfiguriert.

### 📡 phpVMS Live-Display
- **Live-Block-Fuel im `UpdateBody`** wird bei jedem Heartbeat mitgeschickt. phpVMS leitet „Verbleibender Treibstoff = block_fuel − fuel_used" daraus ab; ohne Feld defaultete block_fuel auf 0, Anzeige zeigte „−<fuel_used>" für den ganzen Flug („−17008 kg"-Bug).
- **Bid-Card erweitert** um Aircraft-Type + Marketing-Name + Load-Chips (Pax blau, Cargo orange) + Flight-Type-Badge (PAX/CARGO/CHARTER/REPO). Reihenfolge der Plan-Cards aviation-korrekt: Block → Trip → Reserve | ZFW → TOW → LDW | Alt.
- **SimBrief-Plan-Vorschau** im Briefing per `fetch_simbrief_preview` direkt auf der Bid-Card — Pilot sieht Block / Trip / Reserve / TOW / LDW / ZFW / Alternate VOR dem Tanken, ohne den OFP-Link zu öffnen.

### 🛫 SimBrief-Integration (Phase H.7)
- **API-Client für SimBrief XML-Fetcher** (`xml.fetcher.php`, beide ID-Varianten — numerische SimBrief-ID und Username). Backend-Anbindung läuft automatisch über die phpVMS-Bid-Relation, kein explizites Setup im Settings-Tab nötig.
- **`FlightStats` erweitert** um Plan-Felder: `planned_block_fuel_kg` / `planned_burn_kg` / `planned_reserve_kg` / `planned_zfw_kg` / `planned_tow_kg` / `planned_ldw_kg` / `planned_taxi_kg` / `max_zfw_kg` / `max_tow_kg` / `max_ldw_kg` + Aircraft-Reg + Plan-Route + Plan-Alternate.
- **Landung-Tab** mit komplettem Fuel + Weight + ZFW Soll/Ist/Δ — farbcodiert grün/gelb/rot. Schwellen praxisnah: <5 % grün, 5-10 % gelb, >10 % rot (vorher 1/3 % — viel zu eng für realen Flugbetrieb).
- **Overweight-Warnungen** wenn IST > MAX bei TOW / LDW / ZFW (`LoadsheetMonitor.tsx`).
- **OFP-Mismatch-Detection.** Vergleicht 4 Signale zwischen SimBrief-OFP und phpVMS-Buchung: Aircraft-Type, Origin, Destination, Flight-Number / ATC-Callsign (mit 4 Match-Formaten: direkt, ICAO-Prefix, ATC-Callsign, ATC mit Airline-Prefix). Bei Mismatch werden OFP-Werte komplett ausgeblendet damit keine falschen Daten angezeigt werden — Pilot sieht klaren Banner und weiß: neuen OFP generieren.

### 📋 Loadsheet-Feature (neu in 0.3.x)
- **`LoadsheetMonitor` im Cockpit-Tab** — sichtbar nur in Phase Preflight / Boarding (verschwindet ab TaxiOut). 3 Zeilen mit IST / SOLL / Δ / MAX für Block-Fuel / ZFW / TOW. Inline-Hints: „✓ Bereit für Pushback" / „🛢 Tankvorgang läuft — noch X kg fehlen" / „👥 Boarding läuft — noch X kg fehlen" / „💡 +X kg über Plan".
- **`LoadsheetScore` im Landung-Tab.** Score 0-100 basierend auf Δ% pro Wert (Block/TOW/LDW/ZFW): >5 % = -5 Punkte, >10 % = -15 Punkte. Score-Farbe ≥90 grün, ≥70 gelb, sonst rot. Plus Breakdown-Liste mit ✓/⚠/✕ pro Wert.
- **„Über-Tankt"-Hint im Activity-Log** beim Block-Off-Trigger wenn Block-IST > Plan + Reserve + 500 kg Toleranz. Sanft formuliert („Sehr viel Sprit an Bord, höherer Burn unterwegs zu erwarten") — keine Warnung, nur Cost-Index-Bewusstsein.
- **Loadsheet-Activity-Log @ Block-Off** einmalig „📋 Loadsheet @ Block-off" + Detailzeile (Block / ZFW / TOW). Wandert sowohl in den Cockpit-Activity-Log als auch in den phpVMS-PIREP-ACARS-Log. Dedup über `loadsheet_logged_at_blockoff` Flag (überlebt Resume-after-Crash).

### 🎨 UX-Polish nach GSG-Live-Test
- **Loadsheet im InfoStrip-Stil** (gleiche Optik wie der MASSE/FLUG/TRIP-Strip oben). Keine eigene Box — gehört visuell zum aktiven Flug-Block. Inline-Δ-Suffix statt eigener Spalte: „BLOCK 6.334 kg +0", „TOW 64.544 kg +227". Toggle-Button [▾]/[▸] zum Ein-/Ausklappen.
- **Wetter-Briefing 1-Zeilen-Format** ersetzt die alten 2 Cards: `ABFLUG EDDW 010°/6 kt · 👁 ≥ 10 km · 18°/12° · 1013 hPa  🌦 -SHRA  [▸ METAR]`. METAR-Text aufklappbar. Spart ~200 px Höhe.
- **Wetter-Phänomen-Pills** mit Icon + Code (🌦 SHRA / ⛈ TSRA / ☁ OVC / 🌫 FG) parsed aus dem METAR-Rawtext + Bewölkungs-Fallback.
- **Sicht-Fallback** aus Raw-METAR (`9999` → „≥ 10 km", `CAVOK` → ☀) wenn der Backend-Parser nichts liefert.
- **Visibility-Threshold 9.5 km** statt 10.0 km für die ≥10 km-Anzeige (Aviation-Konvention `9999 m = "10 km oder mehr"`).
- **Cockpit-Tab kompakter:** LiveTapes ~10 % schmaler (Padding 10/14 → 8/12, Schrift 22 → 20 px). RouteMap erst ab Pushback einblenden — vor Pushback ist 0 % Strecke logisch unsinnig.
- **PMDG-Status False-Positive-Fix.** SDK-Warnung wurde fälschlich gefired wenn Sim noch nicht connected, Aircraft im Loading, oder PMDG NG3 in der 20-60s Init-Phase. Jetzt 4-stufiger Check: simState=connected + aircraft_loaded + 20 s Geduld nach Subscribe + ever_received=false.

### ⚙️ Auto-Start-UX
- **Activity-Log-Hint wenn Auto-Start nicht greifen kann.** Drei spezifische Reasons mit jeweils eigener Meldung, throttled 1×/60 s pro Reason: Triebwerke an / Flugzeug rollt / in der Luft.
- **Auto-Start-Skip-Banner im Briefing-Tab.** Gelber Banner mit Begründung im Briefing-Tab — vorher musste der Pilot im Settings-Activity-Log nachschauen oder rätseln warum nichts passiert.
- **Auto-Start-State im Backend persistiert** (`app_config_dir/auto_start.json`). Bisher war `localStorage` die Source of Truth — nach Force-Kill / Hot-Reload im Tauri-Dev-Mode gelegentlich inkonsistent zum Watcher. Frontend zieht beim Mount den Backend-Wert und syncht localStorage als reinen Cache.

### 🛬 Divert-PIREP-Routing (Fix vom 2026-05-04)
- **Diverts werden nicht mehr fälschlich auto-akzeptiert.** phpVMS' `Acars\PirepController::file()` prüft beim Submit nur die Rang-Regel `auto_approve_acars` und ignoriert ein vorher per Smuggle gesetztes `source=MANUAL`. Sobald der PIREP danach `ACCEPTED` ist, blockt `checkReadOnly()` jeden weiteren State-Update — `state→PENDING` schlug mit „PIREP is read-only" fehl.

  **Neuer Pfad:** Bei Divert wird `/file` komplett übersprungen. Stattdessen ein einziger `update_pirep`-Call der `state=PENDING`, `source=MANUAL`, `arr_airport_id`, alle Stats und Timestamps mass-assigned **solange der PIREP noch IN_PROGRESS ist**. Verifiziert gegen phpvms@dev: `PirepController::update` + `parsePirep()` schieben alles per Mass-Assign auf den Pirep-Record, alle nötigen Felder sind in `$fillable`. Der PIREP landet sauber im PENDING-Bucket des VA-Admins ohne Auto-Approve-Trigger.
- **Activity-Log-Display-Fix.** Zeigt bei Divert die echte Arrival-ICAO mit „(DIVERT, planned X)" Suffix statt der alten Plan-Destination — sowohl im Auto-Path als auch im Manual-Path.

### 🛠 Intern
- `UpdateBody` (api-client) erweitert um `arr_airport_id`, `landing_rate`, `score`, `submitted_at`, `block_on_time` für den Divert-Mass-Assign-Pfad.
- `PirepFull.distance` entfernt — phpVMS gibt das Feld inkonsistent als Objekt oder Zahl zurück, wir brauchen's für den State-Check eh nicht.
- `SimSnapshot` erweitert um `light_wing`, `light_wheel_well`, `xpdr_mode_label`, `takeoff_config_warning` als universelle Felder. PMDG-Adapter füllt sie via `snapshot()`-merge, X-Plane-Adapter via DataRefs. Activity-Log liest direkt aus `snap.*` statt aus `snap.pmdg.*` → einheitlicher Pfad.
- Tests: 76 grün (unverändert).

### 📭 Bewusst nicht in 0.3.x
Diese Punkte standen mal auf dem Master-Plan, sind aber nicht enthalten — Code-Verifikation per Grep:
- **Aircraft-Reg-Verifikation (SimBrief vs. Sim).** War in v0.1.x drin, wegen MSFS-2024 Pilot-Profil-Override mit False-Positives wieder ausgebaut. Bleibt skipped bis ein WASM-Livery-Reader steht.
- **Settings-Tab SimBrief-ID/Username-Eingabefeld + Test-Button + Status-Pill.** SimBrief-Anbindung läuft automatisch über die phpVMS-Bid-Relation, daher kein expliziter Setup-Schritt nötig.
- **One-Time Update-Banner im Cockpit-Tab nach erstem Start.** Aus dem gleichen Grund nicht implementiert.
- **„Tipp"-Hinweise im Activity-Log wenn ohne SB-ID gestartet.** Same.

---

## Frühere Versionen

Notes für v0.2.x und v0.1.x liegen in den jeweiligen Release-Commit-Messages (`git log --oneline | grep "release:"`). Die Tags `v0.3.0` (Dev-Build, 2026-05-03) und v0.3.1 markieren denselben funktionalen Release-Zweig — alles, was zwischen v0.2.4 und v0.3.1 reingewachsen ist, steht oben unter `[v0.3.1]`.
