# Changelog

Alle nennenswerten Änderungen an AeroACARS. Format: lose an [Keep a Changelog](https://keepachangelog.com/) angelehnt; Versionsnummern folgen [Semantic Versioning](https://semver.org/) (Patch: Bugfix, Minor: Feature, Major: Breaking).

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
