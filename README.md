# CloudeAcars

> Modern, native, cross-platform ACARS client for [phpVMS 7](https://phpvms.net) — Windows + macOS.
> Modernes, natives, plattformübergreifendes ACARS-Programm für phpVMS 7.

---

## Project status / Projekt-Status

**Phase 0 — Foundation.** Kickoff: 2026-05-01. No runnable code yet.

Phasenplanung:

| Phase | Inhalt | Status |
|---|---|---|
| 0 | Foundation, Spec, Repo, Architektur-Entscheidungen | 🟡 in Arbeit |
| 1 | Tauri-Skeleton, phpVMS-API-Client, MSFS-SimConnect-Adapter, Live-Position, Basis-PIREP | 🔲 |
| 2 | X-Plane XPLM-Plugin + Adapter, Flight-Phase-Detection, vollständiges Flight-Log | 🔲 |
| 3 | Runway-DB, Departure-/Arrival-Detection, METAR-Snapshots, Centerline-/Heading-/Threshold-Analyse | 🔲 |
| 4 | `CloudeAcars` phpVMS-Server-Modul (neue PIREP-Felder, Admin-UI, Migrationen) | 🔲 |
| 5 | Regel-Engine, Custom Fields, Update-System, signierte Installer (Win + macOS) | 🔲 |

---

## What this is / Was das ist

**EN:** A from-scratch replacement for the closed-source vmsACARS client. CloudeAcars is a desktop application that:

- Connects to a phpVMS 7 site via API key.
- Loads the pilot's flights, bids, fleet, and SimBrief plans.
- Connects to the simulator (MSFS 2020/2024 via SimConnect, X-Plane 11/12 via own XPLM plugin).
- Records the flight: position stream, flight phases, fuel, altitude, speeds, G-force, landing rate, etc.
- Streams live position to the phpVMS live map.
- Performs detailed runway and landing analysis (departure/arrival runway detection, centerline deviation, heading deviation, threshold distance, bounces, METAR snapshot).
- Submits a complete PIREP at flight end.

**DE:** Eine komplette Neuentwicklung als Ersatz für den Closed-Source-Client vmsACARS. CloudeAcars ist eine Desktop-Anwendung, die mit phpVMS 7 spricht, den Simulator-Daten live aufzeichnet, Flugphasen erkennt, eine vollständige Start- und Landungsanalyse durchführt und am Ende automatisch einen PIREP überträgt.

---

## Architecture at a glance / Architektur-Überblick

```
┌──────────────────────────────────────────────────────────────────────┐
│                         CloudeAcars Desktop                          │
│                                                                      │
│   ┌────────────────────┐    ┌────────────────────────────────────┐   │
│   │  UI (React + TS)   │◄──►│       Rust Core (Tauri)            │   │
│   │  i18n DE / EN      │    │  • Sim Adapter Trait               │   │
│   │  Dark mode         │    │     ├─ MSFS adapter (SimConnect)   │   │
│   └────────────────────┘    │     └─ X-Plane adapter (UDP→XPLM)  │   │
│                             │  • phpVMS API Client (HTTPS+JSON)  │   │
│                             │  • Flight Recorder & Phase FSM     │   │
│                             │  • Local SQLite queue (offline)    │   │
│                             │  • Runway/METAR analyzer           │   │
│                             └────────────────────────────────────┘   │
└──────────────────────────────────────────────────────────────────────┘
        │                                              ▲
        │ HTTPS / JSON                                 │ UDP localhost
        ▼                                              │
┌──────────────────────────┐               ┌───────────┴──────────────┐
│   phpVMS 7 site          │               │  X-Plane (with our       │
│   • Core API             │               │  bundled XPLM plugin)    │
│   • CloudeAcars module   │               └──────────────────────────┘
└──────────────────────────┘
                                           ┌──────────────────────────┐
                                           │  MSFS 2020 / 2024        │
                                           │  via SimConnect SDK      │
                                           └──────────────────────────┘
```

Detail siehe [`docs/architecture.md`](docs/architecture.md).

---

## Repository layout

| Folder | Purpose |
|---|---|
| [`client/`](client/) | Tauri desktop application (Rust core + React/TypeScript UI) |
| [`server-module/`](server-module/) | New `CloudeAcars` phpVMS 7 module — drop-in into `phpvms/modules/` |
| [`xplane-plugin/`](xplane-plugin/) | XPLM plugin (`.xpl`) shipped with the installer for X-Plane 11/12 |
| [`shared/`](shared/) | Cross-component contracts: JSON schemas, OpenAPI specs, protocol definitions |
| [`docs/`](docs/) | Specification, architecture, ADRs, protocol docs, runway DB notes |
| [`reference/`](reference/) | Read-only reference material (existing vmsACARS module, Windows installer) |
| [`scripts/`](scripts/) | Build, packaging, and dev tooling |

---

## Tech stack / Technologie-Stack

- **Client core:** Rust + [Tauri 2](https://tauri.app)
- **Client UI:** TypeScript + React + Vite + i18n (DE/EN)
- **MSFS integration:** SimConnect SDK only — **no FSUIPC**
- **X-Plane integration:** custom XPLM plugin (Rust via `xplm-sys` or C/C++) ↔ client over local UDP
- **Server module:** PHP 8.2+, Laravel 11+, phpVMS 7 module conventions
- **Local storage:** SQLite (via `rusqlite`) for offline queue + flight log
- **Build:** Tauri bundler → MSI/NSIS for Windows, .app/DMG for macOS

---

## Supported simulators / Unterstützte Simulatoren

| Simulator | Status |
|---|---|
| Microsoft Flight Simulator 2020 | Phase 1 (planned) |
| Microsoft Flight Simulator 2024 | Phase 1 (planned) |
| X-Plane 11 | Phase 2 (planned) |
| X-Plane 12 | Phase 2 (planned) |

---

## License / Lizenz

**Freeware, closed-source.** Free of charge for end users (pilots, virtual airlines). Source code is **not** publicly distributed. See [`LICENSE`](LICENSE) for the placeholder; the full EULA will be finalized before public release (Phase 5).

**DE:** Freeware mit geschlossenem Quellcode. Für Endnutzer (Piloten, virtuelle Airlines) kostenlos. Der Quellcode ist **nicht** öffentlich. Die endgültige Lizenzvereinbarung (EULA) wird vor dem Public Release (Phase 5) finalisiert.

---

## Documentation index / Dokumentations-Index

- [Requirements specification (German, ~32 sections)](docs/spec/requirements.md)
- [Architecture overview](docs/architecture.md)
- [Architectural Decision Records (ADRs)](docs/decisions/)
- [phpVMS protocol notes](docs/protocol/) *(WIP)*
- [Runway database notes](docs/runways/) *(WIP)*
