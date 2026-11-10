# AeroACARS Pilot-Client QS-Audit

**Scope:** `E:\CloudeAcars\client\` — Rust (src-tauri + 11 Crates) und Frontend (React/TS/Vite).
**Stand:** v0.7.12 (2026-05-12). Read-only Audit, keine Code-Aenderungen.

Severity-Skala: **Critical** (bricht Feature) / **High** (offensichtlicher Bug oder Dead-Feature-Block) / **Medium** (Cruft, sicher zu entfernen) / **Low** (Doku/Kommentar) / **Info** (Beobachtung).

---

## 1. Dead Code / Unused Functions

### 1.1 Rust — Build-Log-Warnings (vom letzten Build bestaetigt)

| Sev | Location | Was | Empfehlung |
|---|---|---|---|
| Medium | `client/src-tauri/src/lib.rs:12241` | `fn current_premium_status()` nirgends aufgerufen — der direkte X-Plane-Status laeuft ueber `adapter.premium_status()` in `xplane_premium_status` (lib.rs:16961). | Remove safely. Funktion ist ein orphan Wrapper. |
| Medium | `client/src-tauri/src/lib.rs:7230` | `pirep_queue::count()` nirgends gerufen — Caller nutzen `list_all().len()` oder direkten Filesystem-Walk. | Remove safely. |
| Low | `client/src-tauri/src/discord.rs:52` | Feld `airline_icao` von `EventContext` wird gesetzt aber nie in Embed-Buildern (`build_takeoff` etc.) gelesen — Logo-Fallback laeuft nur via `airline_logo_url`. | Entweder im Fallback nutzen, oder entfernen. |
| Low | `client/src-tauri/src/discord.rs:67` | Feld `fuel_used_kg` von `EventContext` wird gesetzt, aber kein Embed liest's. | Remove or wire in Pirep-Embed. |

### 1.2 Rich-Presence-Block — komplett seit v0.4.0 dead, Wiring-Versprechen "v0.4.5" wurde nie eingeloest (jetzt v0.7.12)

| Sev | Location | Was | Empfehlung |
|---|---|---|---|
| **High** | `client/src-tauri/src/discord.rs:491-659` | `enum PresenceState`, `struct RichPresenceService`, `fn build_activity`, `fn string_leak` — **alle** mit `#[allow(dead_code)] // Wiring in den Streamer kommt in v0.4.5`. v0.4.5 ist 8 Releases her. Webhook-Path (`post_event`) ist live, Rich Presence nicht. | Decide: bauen oder entfernen. `discord-rich-presence = "1"` workspace-dep nur fuer diesen Code, ~170 Zeilen totes Modul. |

### 1.3 Weitere `#[allow(dead_code)]`-Spots

| Sev | Location | Was | Empfehlung |
|---|---|---|---|
| Low | `client/src-tauri/src/lib.rs:1272` | `struct TelemetrySample` — Felder werden zwar konsumiert (Kommentar erklaert), aber das `#[allow(dead_code)]` deckt auch echte unused Felder. Kommentar dokumentiert Status. | Keep (Kommentar erklaert) — bei naechster Touch ueberpruefen ob Annotation noch noetig. |
| Medium | `client/src-tauri/src/lib.rs:2198-2213` | 8 FCU-Debounce-Felder (`last_logged_fcu_alt/hdg/spd/vs`, `pending_fcu_alt/hdg/spd/vs`) — werden nur von `fcu_debounce()` benutzt, das selbst dead ist (lib.rs:16368). | Remove together with `fcu_debounce()` (s.u.). Plan war Fenix-LVar-Ersatz, ist nicht passiert. |
| Medium | `client/src-tauri/src/lib.rs:16368` | `fn fcu_debounce()` — Kommentar: "Currently unused — call sites were removed". Plan war Reaktivierung fuer `AUTOPILOT * VAR` SimVars. | Remove or revive. 60 Zeilen toter Code seit Fenix-LVar-Drop. |
| Info | `client/src-tauri/crates/aeroacars-mqtt/src/lib.rs:660` | `fn default_forensics_version_v1()` — serde-default fuer alte PIREP-Deserialisierung. | Keep — wird von serde-Attribut indirekt aufgerufen, Rust-Compiler erkennt's nicht. |
| Info | `client/src-tauri/crates/sim-msfs/src/adapter.rs:1675` | `fn _link_assertions()` — Marker-Funktion damit Linker `Utc::now`/`Simulator::Msfs2024`/`AircraftProfile::Default` referenziert behaelt im non-Windows stub. | Keep — intentional, Kommentar erklaert. |

### 1.4 Orphan React-Komponenten (NICHT importiert, NICHT gerendert)

| Sev | Location | Was | Empfehlung |
|---|---|---|---|
| **High** | `client/src/components/Dashboard.tsx` (gesamte Datei) | `<Dashboard>` wird nirgends in `App.tsx` o.a. importiert. `tabs.dashboard`-Label existiert noch in i18n, kein Tab. | Remove safely. Komponente + i18n-Keys + CSS `App.css:2262 /* === Dashboard === */`. |
| **High** | `client/src/components/FlightInfoPanel.tsx` | Export `FlightInfoPanel` — nirgends importiert. | Remove safely. |
| **High** | `client/src/components/MassPanel.tsx` | Standalone-Komponente, ersetzt durch `LoadsheetMonitor`/`InfoStrip` (Kommentar dort referenziert "standalone MassPanel" als Vergangenheit). | Remove safely. |
| **High** | `client/src/components/PhaseTimeline.tsx` | Nirgends importiert. | Remove safely. |

### 1.5 Verwaiste Tauri-Commands (registriert in `generate_handler!`, NICHT vom Frontend aufgerufen)

| Sev | Location | Was | Empfehlung |
|---|---|---|---|
| Medium | `lib.rs:4251 get_minimize_to_tray` | Frontend ruft nur `set_minimize_to_tray`, liest Init-Wert aus localStorage. | Remove safely (oder Frontend umstellen). |
| Medium | `lib.rs:4279 get_simbrief_settings` | Frontend ruft `set_simbrief_settings`, init aus localStorage. | Remove safely. |
| Medium | `lib.rs:3798 landing_get` | Frontend nutzt `landing_list` + `landing_get_current`. Per-ID-Get nicht aufgerufen. | Remove safely. |
| Medium | `lib.rs:4604 ofp_callsign_warning_dismiss` | Frontend ruft nur `_get`. Banner verschwindet vermutlich auto. | Verify UX — falls Banner manuell dismissed werden soll, ist ein Frontend-Bug; sonst remove. |
| Medium | `lib.rs:16936 xplane_uninstall_plugin` | Doc-Comment in `xplane_plugin_install.rs:15` listet's, aber Frontend hat keinen Button. | Remove or wire (UX-Entscheidung). |
| Medium | `lib.rs:16986 detect_running_sim` | Frontend uebernimmt Sim-Detection ueber Settings-Panel. UDP-Probe-Command von keinem Caller genutzt. | Remove safely. |

---

## 2. Version-Drift / Inkonsistenzen

| Sev | Location | Was | Empfehlung |
|---|---|---|---|
| Info | `client/package.json:4`, `tauri.conf.json:3`, `Cargo.toml:19`, `Cargo.lock` | Alle auf `0.7.12` — keine Drift. | Keep. |
| Low | `client/src-tauri/src/touchdown_v2.rs:53` | Kommentar plant Patch in "v0.7.7" (BOUNCE-Threshold), wir sind v0.7.12 — Plan offenbar verworfen. | Kommentar updaten oder Plan ausfuehren. |
| Low | `client/src-tauri/src/discord.rs:33,491,518,528,602,656` | 6× Kommentar "Wiring kommt in v0.4.5" — wir sind 8 Releases weiter. | s. 1.2 — Entscheidung treffen. |
| Info | Diverse `pre-v0.5.x` / `pre-v0.7.x` Backward-Compat-Kommentare in `lib.rs` (~15 Stellen) | Dokumentieren legitime serde-default-Faelle fuer alte JSON-Files. | Keep — Persistenz-Migration ist real. |

---

## 3. Deprecated / Stale Features

| Sev | Location | Was | Empfehlung |
|---|---|---|---|
| Medium | `client/src-tauri/crates/aeroacars-mqtt/src/lib.rs:290` | `fuel_efficiency_pct` mit `@deprecated since v0.7.6` annotiert. Wird trotzdem weiter gesetzt fuer Backward-Compat (Discord-Embeds extern). | Keep solange externe Konsumenten leben; Spec-Issue zum Sunset-Zeitpunkt anlegen. |
| Medium | `client/src-tauri/src/lib.rs:11708` | Spiegel-Stelle: `fuel_efficiency_pct` Berechnung im Payload-Builder, gleicher `@deprecated`-Block. | Together with mqtt-Field entfernen. |
| Medium | `client/src-tauri/src/lib.rs:11232` | `landing_critical_until bleibt write-only (legacy field)` — Backend setzt's, Code liest's nicht. | Stop schreiben, oder Feld dokumentiert obsolet. |
| Low | `client/src-tauri/crates/landing-scoring/src/sub_fuel.rs:133` | `pub fn sub_fuel_legacy()` — wird per Test verifiziert, ggf. fuer Migration alter Records noetig? Kein non-test-Caller im Workspace. | Check ob Storage-Read von alten PIREPs darauf zurueckfaellt; wenn nein → Remove. |
| Low | `client/src-tauri/crates/landing-scoring/src/sub_stability.rs:17` | `pub fn sub_stability_legacy()` — analog. Kommentar erklaert "bleibt erhalten fuer ...". | Same as sub_fuel_legacy. |
| Low | `client/src-tauri/crates/sim-msfs/src/adapter/telemetry.rs:330,825` | `fuel_total_lb_legacy` als Fallback wenn moderner Fuel-System-SimVar leer ist (Add-Ons). | Keep — pilot-relevanter Fallback, kein Cruft. |
| Low | `client/src-tauri/crates/secrets/src/lib.rs:194-238` | `migrate_from_keyring()` — Kommentar (Cargo.toml secrets:24) sagt "kann nach v0.5.15 entfernt werden, sobald jeder Pilot einmal migriert ist". Wir sind v0.7.12 = 30+ Releases spaeter. | Decision: Migration-Code + `keyring`-Dep entfernen. Pilots die laenger als 1 Jahr nicht updaten muessen eh neu loginen. |

---

## 4. Defekte / Nicht-mehr-funktionierende Pfade

| Sev | Location | Was | Empfehlung |
|---|---|---|---|
| Info | `client/README.md:73` + `crates/sim-msfs/Cargo.toml:7` | Nur Doku-Erwaehnungen "FSUIPC" als ausdruecklich-nicht-genutzt. **Kein Code-Pfad ruft FSUIPC.** | Keep — entspricht User-Memory-Hard-Constraint. |
| Info | "Volanta"-Erwaehnungen (15+ in lib.rs, locales, recorder, mqtt) | Alles als **Referenz-Algorithmus** fuer Sinkrate-Mittelung — kein abgeschalteter Endpoint, sondern Doku zur Vergleichbarkeit der Berechnung. | Keep. |
| Info | "LandingRate-1.lua"-Erwaehnungen (lib.rs:2835, 2963, 13119, 13144, 13215; mqtt:316) | Ebenso: Algorithmus-Referenz (Dan Berry's X-Plane.org Skript), nicht Endpoint. | Keep. |
| Medium | `client/src-tauri/src/lib.rs:8718 flight_end` mit `divertTo`-Parameter | Wird vom Frontend gerufen, OK. **Aber:** `DivertBanner.tsx:62` ruft `flight_end({})` als "Normaler Landing"-Trigger — semantisch unsauber. | Falls UX intentional: Kommentar im Backend-Doc. |

---

## 5. TypeScript-Warnings / Unused-Imports

| Sev | Location | Was | Empfehlung |
|---|---|---|---|
| Info | `npx tsc --noEmit -p tsconfig.json` | **Lief sauber, 0 Errors, 0 Warnings.** | Keep. |
| Low | `App.tsx:209,238`, `ResumeFlightBanner.tsx:134`, `XPlanePremiumPanel.tsx:78` | 4× `// eslint-disable-next-line react-hooks/exhaustive-deps` — bewusst-deps absichtlich weggelassen. | Keep — jedes mit Kontext-Kommentar belegt. |
| Info | Kein `@ts-ignore` / `@ts-expect-error` gefunden. | — | Keep. |

---

## 6. i18n-Luecken

Vergleich `de/common.json` (Master) vs `en/common.json` vs `it/common.json`:

| Sev | Location | Was | Empfehlung |
|---|---|---|---|
| Info | Alle Locales | DE = 881 Keys, EN = 881 Keys, IT = 882 Keys. **Keine Luecken EN<->DE.** | Keep. |
| Low | `it/common.json` | 1 extra Key `actions.language_it` nicht in DE/EN. | Add to DE/EN oder remove from IT. |
| Medium | `de/common.json:7` (+ EN/IT) | `tabs.dashboard: "Dashboard"` — Tab existiert nicht mehr in `App.tsx` (nur cockpit/briefing/landing/log/settings/about). | Remove key. |
| Medium | `de/common.json:40` (+ EN/IT) | `landing.peak_vs: "Peak-Sinkrate"` — Datenfeld `landing_peak_vs_fpm` wird gerendert, aber dieser Label-Key wird NICHT per `t()` aufgerufen. | Remove key (oder wire in LandingPanel). |
| Medium | `de/common.json:82-83` (+ EN/IT) | `landing.plan_tow` + `landing.plan_ldw` — keine `t("landing.plan_tow")` o.ae. im Code. | Remove keys. |
| Info | `de/common.json:446` | `debug_mode_hint` referenziert "Dashboard" im Doku-Text obwohl Tab weg ist. | Update Text auf "Cockpit-Telemetrie". |
| Info | Restliche 240 vom Tot-Check geflaggte Keys | Alle ueber dynamische Prefixe genutzt: `landing.sub.*`, `landing.info.*`, `landing.rat.*`, `landing.tip.*` werden in `LandingPanel.tsx:1027,1053,1066,1097` via `t(\`landing.sub.${s.key}\`)` etc. resolved. | Keep — false positive. |

---

## 7. Tauri-Command-Audit

**Total registriert:** 60 commands in `generate_handler!` (`lib.rs:18084-18144`).
**Davon vom Frontend genutzt:** 54.
**Orphan-Commands:** 6 (siehe 1.5).

| Sev | Location | Was | Empfehlung |
|---|---|---|---|
| Medium | `lib.rs:18101 get_minimize_to_tray` | s. 1.5 | Remove. |
| Medium | `lib.rs:18104 get_simbrief_settings` | s. 1.5 | Remove. |
| Medium | `lib.rs:18123 landing_get` | s. 1.5 | Remove. |
| Medium | `lib.rs:18094 ofp_callsign_warning_dismiss` | s. 1.5 | Remove or wire. |
| Medium | `lib.rs:18138 xplane_uninstall_plugin` | s. 1.5 | Remove or wire. |
| Medium | `lib.rs:18139 detect_running_sim` | s. 1.5 | Remove. |

---

## 8. Dependencies

### Frontend (`package.json`)
| Sev | Crate | Version | Empfehlung |
|---|---|---|---|
| Info | `react` | 19.1 | aktuell |
| Info | `@tauri-apps/api` / `cli` | ^2 (Tauri v2) | aktuell |
| Info | `vite` | ^7.0.4 | aktuell |
| Info | `typescript` | ~5.8.3 | aktuell |
| Low | `i18next` | ^23.16.4 | v24 stable seit ~2026-Q1 — Major-Upgrade kommt aber mit Breaking-Changes; aktuell ist 23 weiter supported. Keep. |
| Info | `vitest` | ^2 | aktuell |
| Low | `jsdom` | ^25 | v26 raus — minor risk, keine Pflicht. |

### Rust Workspace (Cargo.lock)
| Sev | Crate | Version | Empfehlung |
|---|---|---|---|
| Info | `tauri` | 2.11.0 | aktuell |
| Info | `tokio` | 1.52.1 | aktuell |
| Info | `reqwest` | 0.12.28 (+ 0.13.3 transitive) | aktuell |
| Info | `rusqlite` | 0.32.1 | aktuell |
| Info | `chrono` | 0.4.44 | aktuell |
| Low | `rumqttc` | 0.24.0 | v0.25 raus. Pin in `aeroacars-mqtt/Cargo.toml:30` mit Kommentar — Upgrade-Pfad bekannt aber non-trivial (rustls-0.22-Pin haengt dran). Keep, Issue tracken. |
| Low | `rustls` | 0.22.4 (+ 0.23.40 transitive) | Doppelter rustls im Tree — 0.22 von rumqttc gepinnt, 0.23 von reqwest. Wird sich loesen wenn rumqttc 0.25 ankommt. Keep. |
| Low | `keyring` | 3.6.3 | Crate aktuell, **aber:** s. 3.4 — nur fuer One-Shot-Migration drin. Wenn Migration weg, Dep weg. |
| Low | `discord-rich-presence` | 1.1.0 | Nur fuer dead-code in `discord.rs` 1.2. Wenn Rich-Presence-Block weg, Dep weg (Webhook nutzt reqwest, nicht IPC). |
| Low | `bindgen` | 0.69.5 | v0.70/0.71 raus — kein Critical-Path-Upgrade. Sim-MSFS-Build laeuft. |
| Info | `schemars` | 0.8.22 (+ 0.9 + 1.2 transitive) | Mehrere Versionen im Tree (Tauri zieht eigene). Workspace deklariert 0.8, kein direkter Code-Path nutzt's aktuell (Cargo.toml workspace.dependencies). | Pruefen ob `schemars = "0.8"` in workspace-deps noch noetig — schein dead. |
| Info | `csv` | 1.4.0 | aktuell |
| Info | `regex` | 1.12.3 | aktuell |
| Info | `zip` | 2.4.2 (+ 4.6.1 transitive) | aktuell |

---

## Top-10 — Empfohlene Aufraeum-Aktionen (priorisiert)

1. **High — Rich Presence Block entfernen oder bauen.** `discord.rs:491-659` ist seit v0.4.0 als "kommt in v0.4.5" gelabelt, wir sind v0.7.12. ~170 Zeilen Dead-Code + Crate-Dep `discord-rich-presence`.
2. **High — 4 verwaiste React-Komponenten entfernen.** `Dashboard.tsx`, `FlightInfoPanel.tsx`, `MassPanel.tsx`, `PhaseTimeline.tsx` werden nirgends importiert. Plus CSS-Reste `App.css:2262 /* === Dashboard === */` und i18n-Keys `tabs.dashboard` / `landing.peak_vs` / `landing.plan_tow` / `landing.plan_ldw`.
3. **High — 6 orphan Tauri-Commands entfernen.** `get_minimize_to_tray`, `get_simbrief_settings`, `landing_get`, `ofp_callsign_warning_dismiss`, `xplane_uninstall_plugin`, `detect_running_sim` — alle registriert, kein Frontend-Caller.
4. **Medium — `fcu_debounce()` + 8 zugehoerige Struct-Felder entfernen.** `lib.rs:2198-2213` + `lib.rs:16368`. Fenix-LVar-Plan ist verworfen, Reaktivierungspfad spekulativ.
5. **Medium — Build-Warning-Funktionen entfernen.** `current_premium_status` (`lib.rs:12241`), `pirep_queue::count` (`lib.rs:7230`).
6. **Medium — `secrets::migrate_from_keyring()` + `keyring`-Dep aufloesen.** Migration war fuer v0.5.15, jetzt 30+ Releases her. Wenig wahrscheinlich dass jemand mit pre-v0.5.15-Keychain noch upgradet.
7. **Medium — `discord::EventContext` Tot-Felder aufraeumen.** `airline_icao` und `fuel_used_kg` werden gesetzt aber nicht in Embeds gerendert. Entweder wire-in, sonst remove.
8. **Medium — Workspace-Dep `schemars = "0.8"` ueberpruefen.** Wird im Workspace deklariert, aber kein direkter Code-Path nutzt's mehr (nur transitive via Tauri).
9. **Low — i18n-Putzaktion** (Top-3 echte Tot-Keys + 1 IT-Extra + Doku-Text "Dashboard" in `debug_mode_hint`).
10. **Low — Stale Kommentare updaten:** `discord.rs` 6× "v0.4.5"-Versprechen; `touchdown_v2.rs:53` "v0.7.7"-Patch-Plan; `lib.rs:11232` "landing_critical_until write-only legacy".

---

## Beobachtungen ohne Action-Item

- **0 TypeScript-Errors, 0 -Warnings** — Build-Hygiene auf der TS-Seite ist sauber.
- **i18n-Parity DE/EN/IT** ist tatsaechlich gut — nur 1 Key IT-extra.
- **Persistenz-Migration (pre-v0.5.x / pre-v0.7.x)** ist auf 15+ Stellen dokumentiert und intentional fuer alte gespeicherte JSON-Files. Kein Cruft.
- **Volanta / LandingRate-1.lua / FAR 25.473** sind Algorithmus-Referenzen, kein abgeschalteter Endpoint — entspricht der Architektur-Entscheidung "VS exakt am on_ground-Edge".
- **FSUIPC-Erwaehnungen** existieren NUR in der Doku als ausdruecklich-nicht-genutzt. Hard-Constraint respektiert.
- **Rust-Crates** sind sauber gepinned und versioniert — kein Crate hinkt mehr als 1 Minor hinterher.
