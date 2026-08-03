# NexusAir ACARS

> Modern, open-source ACARS client for [phpVMS 7](https://phpvms.net) — Tauri 2 + Rust + React.
> Made with ❤️ in Gifhorn — by Thomas Kant.

[![License: MIT](https://img.shields.io/badge/License-MIT-green.svg)](LICENSE)
[![Platform: Windows](https://img.shields.io/badge/Platform-Windows-blue.svg)](#installation)
[![phpVMS 7](https://img.shields.io/badge/phpVMS-7-orange.svg)](https://phpvms.net)

---

## What is NexusAir ACARS?

A modern, cross-platform ACARS client for phpVMS 7. It captures telemetry from
flight simulators, scores landings against industry-validated thresholds,
correlates touchdowns to runway-centerline accuracy, and ships clean PIREPs to
your phpVMS server.

**Currently supported:**

- ✅ **MSFS 2020 / MSFS 2024** — via raw SimConnect FFI (Windows-only,
  no FSUIPC needed)
- ✅ **X-Plane 11 / X-Plane 12** — via native UDP DataRefs
  (cross-platform, no plugin needed)

---

## Who can run NexusAir ACARS?

🇬🇧 **The source is free — the official apps only run for German Sky Group.**

NexusAir ACARS is open source (MIT license). Any virtual airline is welcome to clone,
adapt and build the code for its **own** phpVMS 7 instance — that is explicitly
encouraged.

The **officially released builds** (the installers from the
[GitHub releases](https://github.com/MANFahrer-GF/AeroACARS/releases)) are, by
contrast, hard-wired to the German Sky Group: login, live tracking and PIREP
submission only work with a GSG pilot account — an account of another VA is
rejected. Anyone who wants to use NexusAir ACARS for a different VA builds their own
client from source against their own infrastructure.

---

## Installation

Download the package for your platform from the [Latest Release](https://github.com/MANFahrer-GF/AeroACARS/releases/latest).

### Windows (10 / 11, x64)

1. Download and run `NexusAir ACARS_<version>_x64-setup.exe` (NSIS installer)
2. Dismiss the SmartScreen warning: "More info" → "Run anyway" — we are not yet code-signed
3. NexusAir ACARS starts automatically after installation
4. Log in with your phpVMS API key

### macOS (Apple Silicon — M1 / M2 / M3 / M4)

1. Download `NexusAir ACARS_<version>_aarch64.dmg`
2. Open the DMG → drag the NexusAir ACARS icon into the Applications folder
3. **On first launch:** Gatekeeper blocks the app because it hasn't gone through Apple Notarization. You have two options:
   - **Via right-click:** In Finder, right-click NexusAir ACARS → "Open" → confirm "Open" in the dialog. macOS then remembers the permission and launches the app normally from then on.
   - **Via Terminal** (if right-click doesn't show the option — happens with stricter Gatekeeper settings):
     ```bash
     xattr -dr com.apple.quarantine "/Applications/NexusAir ACARS.app"
     ```
4. Log in with your phpVMS API key

> **Note:** Intel Macs are currently not built officially. If there's demand for it: open an issue — the Tauri build can be extended to `x86_64-apple-darwin` without much effort.

### Auto-updates

From v0.1.0+, new versions appear directly as an update banner in the app — no more manual downloads needed. The updater verifies the bundles by Ed25519 signature, so it's secure even without code-signing/notarization.

---

## What can NexusAir ACARS do?

### Live telemetry + flight tracking
- Phase-detection FSM (16 phases: Boarding → Pushback → TaxiOut → Takeoff → Climb → Cruise → Descent → Approach → Final → Landing → TaxiIn → BlocksOn → Arrived → PIREP)
- Position streaming to phpVMS with phase-adaptive cadence
- Offline queue for position posts when the network drops out

### Touchdown analysis (industrial-grade)
- 50 Hz sampling (matches GEES, higher than MSFS' default)
- V/S capture from latched SimVar (MSFS) or buffer-min ±250 ms (GEES pattern)
- Peak-G within the 800 ms window after impact (strut rebound excluded)
- AGL-based bounce detection (35→5 ft, BeatMyLanding-aligned)
- Native sideslip from VEL_BODY_X/Z (`atan2`)
- Headwind/crosswind from airframe-relative wind components
- Score thresholds from Boeing 737 FCOM, Airbus A320 FCOM, LH FOQA, vmsACARS defaults

### Runway correlation
- OurAirports.com runway dataset (47,681 runways, 4 MB) embedded
- Touchdown lat/lon → exact runway + centerline distance + threshold distance

### PIREP submission
- Full notes block (TIMES / TOUCHDOWN / RUNWAY / FUEL / DISTANCE / METAR)
- ~40 custom fields (Title-Case + snake_case for leaderboards)
- Auto-file on `Arrived`, with manual override option
- Bid deletion via the correct `/api/user/bids` endpoint

### Comfort features
- Auto-start watcher: recording begins automatically when the aircraft is at the bid departure airport
- Persistent activity log with crash recovery (per-flight reset)
- Live sim inspector in debug mode (MSFS SimVars/LVars + X-Plane DataRefs)
- METAR snapshots Dep/Arr automatically on takeoff/final

---

## Tech stack

- **Backend:** Rust (Tauri 2, raw SimConnect FFI for MSFS, std::net for X-Plane UDP)
- **Frontend:** React 19 + TypeScript + Vite
- **Persistence:** OS keyring for API keys, JSON sidecars for activity log + active-flight state
- **Updater:** Tauri plugin-updater with Ed25519 signature, GitHub Releases as source

---

## Shoulders NexusAir ACARS stands on

- **OurAirports** — Public-domain runway dataset
- **BeatMyLanding** — Touchdown-window calibration and bounce-detection pattern
- **GEES** — Open-source landing-rate logger; reverse-engineered for the V/S sign convention and native sideslip calculation
- **LandingToast** — Live-VS-at-OnGround-edge pattern
- **Tauri 2 + Rust + React** — App framework
- **MSFS SDK + X-Plane SDK** — Sim integration

---

## Development

```bash
# Prerequisites: Rust toolchain, Node.js 20+, and the MSFS 2024 SDK if building sim-msfs
git clone https://github.com/MANFahrer-GF/AeroACARS.git
cd AeroACARS/client
npm install
npm run tauri dev          # Dev mode with hot-reload
npm run tauri build -- --bundles nsis   # Build the release installer
```

---

## X-Plane: study-level add-ons (CL650, ToLiss, FlightFactor …)

Deep study-level aircraft in X-Plane often drive cockpit and system functions
through their **own DataRefs** instead of the standard `sim/...` DataRefs.
NexusAir ACARS reads the standard DataRefs — if an add-on doesn't populate them,
NexusAir ACARS can't "see" the flap setting, for example (consequence:
`LANDING CONFIG: INCOMPLETE` even though flaps were set).

From **v0.12.1**, an unreadable value is handled fairly — shown as "not
assessable" instead of a red error, with **no** score penalty. For a real,
aircraft-accurate evaluation there is a fillable template that captures the
add-on's own DataRefs:

→ **[docs/xplane-aircraft-dataref-overrides.md](docs/xplane-aircraft-dataref-overrides.md)**

If you fly a study-level aircraft: fill in the table once with the
[DataRefTool](https://datareftool.com/) and submit it via the
[issue tracker](https://github.com/MANFahrer-GF/AeroACARS/issues) — then the
aircraft can get a matching DataRef profile.

---

## Troubleshooting / logs

If NexusAir ACARS behaves oddly and you want to send something substantial to the
issue tracker — here's what lives where.

### Where NexusAir ACARS stores data

All files live under the Tauri-standard `app_data_dir` with bundle ID
`com.aeroacars.app`:

| Platform | Full path |
|---|---|
| **Windows** | `%APPDATA%\com.aeroacars.app\` <br>(typically: `C:\Users\<your-user>\AppData\Roaming\com.aeroacars.app\`) |
| **macOS** | `~/Library/Application Support/com.aeroacars.app/` |

On Windows you can open the folder directly with `Win+R` → `%APPDATA%\com.aeroacars.app`. On macOS with Finder → `Cmd+Shift+G` → paste the path.

### What's in it

| File | What it is |
|---|---|
| `flight_logs/<pirep_id>.jsonl` | **Per-flight recorder** — one line per event (position, phase transition, touchdown score, METAR snapshot). Append-only JSONL, the best source for "why did the flight do X?". One file per PIREP. |
| `activity_log.json` | **In-app activity feed** — exactly the lines that appear in the cockpit tab, persisted across restarts. |
| `active_flight.json` | Snapshot of the currently running flight for the resume function. Exists only while a flight is running. |
| `landing_history.json` | Historical landings for the "Landing" tab. |
| `position_queue.bin` | Offline backlog: positions that couldn't be uploaded yet due to network problems. Cleared automatically once back online. |
| `site.json`, `sim.json` | Local settings (phpVMS URL, chosen sim). No API key — that lives in the OS keyring. |

The **API key** is **not** stored as a file. It is kept in the OS keyring
(Windows Credential Manager / macOS Keychain). No plaintext on disk.

### Tracing / console logs

The Rust tracing output (HTTP requests, SimConnect status, detailed phase
computation) currently goes only to **stderr** — it does **not** land on disk.
If you need it:

- **Windows:** Start NexusAir ACARS from a PowerShell console: `& "C:\Program Files\NexusAir ACARS\NexusAir ACARS.exe"` — the tracing lines then appear in the terminal.
- **macOS:** From the terminal: `/Applications/NexusAir\ ACARS.app/Contents/MacOS/NexusAir\ ACARS`

Control the verbosity level via `RUST_LOG`:

```bash
# Standard mode (info)
RUST_LOG=info  ./NexusAir\ ACARS

# Full debug for our code, info for everything else
RUST_LOG=info,aeroacars=debug  ./NexusAir\ ACARS
```

### Reporting an issue

When something goes wrong, the most valuable info in a bug report is:

1. The `flight_logs/<pirep_id>.jsonl` of the affected flight (zip it, attach it)
2. The relevant excerpt from `activity_log.json`
3. If reproducible: a few lines of tracing output with `RUST_LOG=info,aeroacars=debug` from the terminal run

Please file issues via → [github.com/MANFahrer-GF/AeroACARS/issues](https://github.com/MANFahrer-GF/AeroACARS/issues)

---

## License

MIT — see [LICENSE](LICENSE).

---

**Contact:** Thomas Kant · German Sky Group · [github.com/MANFahrer-GF](https://github.com/MANFahrer-GF)
