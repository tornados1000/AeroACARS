# NexusAir ACARS

> Modern, open-source ACARS client for [phpVMS 7](https://phpvms.net) — Tauri 2 + Rust + React.
> Made with ❤️ in Gifhorn — by Thomas Kant.

[![License: MIT](https://img.shields.io/badge/License-MIT-green.svg)](LICENSE)
[![Platform: Windows](https://img.shields.io/badge/Platform-Windows-blue.svg)](#installation)
[![phpVMS 7](https://img.shields.io/badge/phpVMS-7-orange.svg)](https://phpvms.net)

---

## Was ist NexusAir ACARS?

Ein moderner, plattformübergreifender ACARS-Client für phpVMS 7. Erfasst
Telemetrie aus Flight Simulators, scort Landungen mit industrie-validierten
Schwellen, korreliert Touchdowns auf Runway-Centerline-Genauigkeit und
shippt saubere PIREPs zu deinem phpVMS-Server.

**Aktuell unterstützt:**

- ✅ **MSFS 2020 / MSFS 2024** — über raw SimConnect FFI (Windows-only,
  kein FSUIPC nötig)
- ✅ **X-Plane 11 / X-Plane 12** — über native UDP DataRefs
  (cross-platform, kein Plugin nötig)

---

## Für wen läuft NexusAir ACARS? · Who can run NexusAir ACARS?

🇩🇪 **Der Quellcode ist frei — die offiziellen Apps laufen nur für die German Sky Group.**

NexusAir ACARS ist Open Source (MIT-Lizenz). Jede Virtual Airline darf den Code
klonen, anpassen und für ihre **eigene** phpVMS-7-Instanz selbst bauen — das ist
ausdrücklich erwünscht.

Die **offiziell veröffentlichten Builds** (die Installer aus den
[GitHub-Releases](https://github.com/MANFahrer-GF/AeroACARS/releases)) sind
dagegen fest auf die German Sky Group konfiguriert: Login, Live-Tracking und
PIREP-Submission funktionieren ausschließlich mit einem GSG-Pilotenaccount —
ein Account einer anderen VA wird abgewiesen. Wer NexusAir ACARS für eine andere VA
nutzen möchte, baut sich aus dem Quellcode einen eigenen Client gegen die
eigene Infrastruktur.

🇬🇧 **The source is free — the official apps only run for German Sky Group.**

NexusAir ACARS is open source (MIT). Any virtual airline is welcome to clone, adapt
and build the code for its **own** phpVMS 7 instance.

The **officially released builds** (the installers in the
[GitHub releases](https://github.com/MANFahrer-GF/AeroACARS/releases)) are, by
contrast, hard-wired to the German Sky Group: login, live tracking and PIREP
submission only work with a GSG pilot account — accounts of other VAs are
rejected. To use NexusAir ACARS for a different VA, build your own client from
source against your own infrastructure.

---

## Installation

Lade dir das Paket für deine Plattform aus dem [Latest Release](https://github.com/MANFahrer-GF/AeroACARS/releases/latest) herunter.

### Windows (10 / 11, x64)

1. `NexusAir ACARS_<version>_x64-setup.exe` (NSIS-Installer) herunterladen und ausführen
2. SmartScreen-Warnung wegklicken: „Weitere Informationen" → „Trotzdem ausführen" — wir sind noch nicht code-signed
3. NexusAir ACARS startet nach der Installation automatisch
4. Login mit deinem phpVMS-API-Key

### macOS (Apple Silicon — M1 / M2 / M3 / M4)

1. `NexusAir ACARS_<version>_aarch64.dmg` herunterladen
2. DMG öffnen → NexusAir ACARS-Icon in den Applications-Ordner ziehen
3. **Beim ersten Start:** Gatekeeper blockt die App, weil sie nicht über die Apple Notarization gegangen ist. Du hast zwei Wege:
   - **Per Rechtsklick:** Im Finder auf NexusAir ACARS rechtsklicken → „Öffnen" → „Öffnen" im Dialog bestätigen. Danach merkt sich macOS die Erlaubnis und startet die App ab dann normal.
   - **Per Terminal** (falls Rechtsklick die Option nicht zeigt — kommt bei strengeren Gatekeeper-Einstellungen vor):
     ```bash
     xattr -dr com.apple.quarantine "/Applications/NexusAir ACARS.app"
     ```
4. Login mit deinem phpVMS-API-Key

> **Hinweis:** Intel Macs werden derzeit nicht offiziell gebaut. Wenn dafür Bedarf besteht: ein Issue aufmachen, der Tauri-Build kann ohne große Mühe um `x86_64-apple-darwin` erweitert werden.

### Auto-Updates

Ab v0.1.0+ erscheinen neue Versionen direkt als Update-Banner in der App — kein manueller Download mehr nötig. Der Updater verifiziert die Bundles per Ed25519-Signatur, also auch ohne Code-Signing/Notarization sicher.

---

## Was kann NexusAir ACARS?

### Live-Telemetrie + Flugverfolgung
- Phase-Detection-FSM (16 Phasen: Boarding → Pushback → TaxiOut → Takeoff → Climb → Cruise → Descent → Approach → Final → Landing → TaxiIn → BlocksOn → Arrived → PIREP)
- Position-Streaming an phpVMS mit phasen-adaptiver Cadence
- Offline-Queue für Position-Posts wenn das Netzwerk wegbricht

### Touchdown-Analyse (industriegrade)
- 50 Hz Sampling (matches GEES, höher als MSFS' default)
- V/S-Capture aus latched SimVar (MSFS) oder Buffer-Min ±250 ms (GEES-Pattern)
- Peak-G im 800-ms-Fenster nach Aufprall (Strut-Rebound ausgeschlossen)
- AGL-basierte Bounce-Detection (35→5 ft, BeatMyLanding-aligned)
- Native Sideslip aus VEL_BODY_X/Z (`atan2`)
- Headwind/Crosswind aus airframe-relativen Wind-Komponenten
- Score-Schwellen aus Boeing 737 FCOM, Airbus A320 FCOM, LH FOQA, vmsACARS-Defaults

### Runway-Korrelation
- OurAirports.com Runway-Datensatz (47.681 Bahnen, 4 MB) embedded
- Touchdown-Lat/Lon → exakte Runway + Centerline-Distance + Threshold-Distance

### PIREP-Submission
- Voller Notes-Block (TIMES / TOUCHDOWN / RUNWAY / FUEL / DISTANCE / METAR)
- ~40 Custom Fields (Title-Case + snake_case für Leaderboards)
- Auto-File bei `Arrived`, mit manueller Override-Option
- Bid-Delete via korrektem `/api/user/bids` Endpoint

### Comfort-Features
- Auto-Start-Watcher: Aufzeichnung beginnt automatisch wenn Aircraft am Bid-Departure-Airport steht
- Persistente Activity-Log mit Crash-Recovery (per-Flight reset)
- Live-Sim-Inspector im Debug-Modus (MSFS SimVars/LVars + X-Plane DataRefs)
- METAR-Snapshots Dep/Arr automatisch beim Takeoff/Final

---

## Tech-Stack

- **Backend:** Rust (Tauri 2, raw SimConnect FFI für MSFS, std::net für X-Plane UDP)
- **Frontend:** React 19 + TypeScript + Vite
- **Persistence:** OS-Keyring für API-Keys, JSON-Sidecars für Activity-Log + Active-Flight-State
- **Updater:** Tauri-Plugin-Updater mit Ed25519-Signatur, GitHub Releases als Source

---

## Schultern, auf denen NexusAir ACARS steht

- **OurAirports** — Public-domain Runway-Datensatz
- **BeatMyLanding** — Touchdown-Window-Calibration und Bounce-Detection-Pattern
- **GEES** — Open-Source-Landingrate-Logger; reverse-engineered für V/S-Sign-Convention und native Sideslip-Berechnung
- **LandingToast** — Live-VS-at-OnGround-Edge-Pattern
- **Tauri 2 + Rust + React** — App-Framework
- **MSFS SDK + X-Plane SDK** — Sim-Integration

---

## Entwicklung

```bash
# Voraussetzung: Rust toolchain, Node.js 20+, ggf. MSFS 2024 SDK für sim-msfs build
git clone https://github.com/MANFahrer-GF/AeroACARS.git
cd AeroACARS/client
npm install
npm run tauri dev          # Dev-Mode mit Hot-Reload
npm run tauri build -- --bundles nsis   # Release-Installer bauen
```

---

## X-Plane: Study-Level-Add-ons (CL650, ToLiss, FlightFactor …)

Tiefe Study-Level-Flugzeuge in X-Plane steuern Cockpit- und System-Funktionen oft über **eigene DataRefs** statt der Standard-`sim/...`-DataRefs. NexusAir ACARS liest die Standard-DataRefs — bedient ein Add-on sie nicht, „sieht" NexusAir ACARS z.B. die Flaps-Stellung nicht (Folge: `LANDING CONFIG: INCOMPLETE`, obwohl Flaps gesetzt waren).

Ab **v0.12.1** wird ein nicht lesbarer Wert fair behandelt — Anzeige „nicht bewertbar" statt rotem Fehler, **kein** Punktabzug. Für eine echte, flugzeug-genaue Auswertung gibt es eine ausfüllbare Vorlage, mit der die add-on-eigenen DataRefs erfasst werden:

→ **[docs/xplane-aircraft-dataref-overrides.md](docs/xplane-aircraft-dataref-overrides.md)**

Wer ein Study-Level-Flugzeug fliegt: einmal die Tabelle mit dem [DataRefTool](https://datareftool.com/) ausfüllen und über den [Issue-Tracker](https://github.com/MANFahrer-GF/AeroACARS/issues) einreichen — dann kann das Flugzeug ein passendes DataRef-Profil bekommen.

---

## Troubleshooting / Logs

Wenn NexusAir ACARS sich komisch verhält und du dem Issue-Tracker etwas Substanzielles mitschicken willst — hier ist, was wo liegt.

### Wo NexusAir ACARS Daten ablegt

Alle Dateien liegen unter dem Tauri-Standard-`app_data_dir` mit Bundle-ID `com.aeroacars.app`:

| Plattform | Vollständiger Pfad |
|---|---|
| **Windows** | `%APPDATA%\com.aeroacars.app\` <br>(typisch: `C:\Users\<dein-user>\AppData\Roaming\com.aeroacars.app\`) |
| **macOS** | `~/Library/Application Support/com.aeroacars.app/` |

In Windows kannst du den Ordner direkt mit `Win+R` → `%APPDATA%\com.aeroacars.app` öffnen. In macOS mit Finder → `Cmd+Shift+G` → den Pfad einfügen.

### Was drin liegt

| Datei | Was es ist |
|---|---|
| `flight_logs/<pirep_id>.jsonl` | **Per-Flug-Recorder** — eine Zeile pro Event (Position, Phasen-Übergang, Touchdown-Score, METAR-Snapshot). Append-only JSONL, beste Quelle für „warum hat der Flug X gemacht?". Eine Datei pro PIREP. |
| `activity_log.json` | **In-App-Activity-Feed** — exakt die Zeilen, die im Cockpit-Tab erscheinen, persistiert über Restarts. |
| `active_flight.json` | Snapshot des aktuell laufenden Flugs für die Resume-Funktion. Existiert nur während ein Flug läuft. |
| `landing_history.json` | Historische Landungen für den „Landung"-Tab. |
| `position_queue.bin` | Offline-Backlog: Positionen die wegen Netzwerkproblemen noch nicht hochgeladen werden konnten. Wird automatisch geleert sobald wieder online. |
| `site.json`, `sim.json` | Lokale Settings (phpVMS-URL, gewählter Sim). Kein API-Key — der liegt im OS-Keyring. |

Der **API-Key** liegt **nicht** als Datei vor. Er wird über das OS-Keyring (Windows Credential Manager / macOS Keychain) gespeichert. Kein Plaintext auf Disk.

### Tracing / Console-Logs

Die Rust-tracing-Ausgaben (HTTP-Requests, SimConnect-Status, Phasen-Berechnung im Detail) gehen aktuell nur auf **stderr** — sie landen **nicht** auf der Disk. Wenn du sie brauchst:

- **Windows:** NexusAir ACARS aus einer PowerShell-Konsole starten: `& "C:\Program Files\NexusAir ACARS\NexusAir ACARS.exe"` — dann erscheinen die tracing-Zeilen im Terminal.
- **macOS:** Aus dem Terminal: `/Applications/NexusAir\ ACARS.app/Contents/MacOS/NexusAir\ ACARS`

Verbosity-Level steuern via `RUST_LOG`:

```bash
# Standardmodus (info)
RUST_LOG=info  ./NexusAir\ ACARS

# Volles Debug für unseren Code, info für alles andere
RUST_LOG=info,aeroacars=debug  ./NexusAir\ ACARS
```

### Issue melden

Wenn was schiefgeht, ist die wertvollste Info im Bug-Report:

1. Die `flight_logs/<pirep_id>.jsonl` des betroffenen Flugs (zippen, anhängen)
2. Der relevante Ausschnitt aus `activity_log.json`
3. Falls reproduzierbar: ein paar Zeilen tracing-Output mit `RUST_LOG=info,aeroacars=debug` aus dem Terminal-Run

Issues bitte über → [github.com/MANFahrer-GF/AeroACARS/issues](https://github.com/MANFahrer-GF/AeroACARS/issues)

---

## License

MIT — siehe [LICENSE](LICENSE).

---

**Contact:** Thomas Kant · German Sky Group · [github.com/MANFahrer-GF](https://github.com/MANFahrer-GF)
