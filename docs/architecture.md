# CloudeAcars — Architecture Overview

**Status:** Draft (Phase 0 — Foundation). Living document, updated alongside ADRs.

---

## 1. System context

```
                    ┌─────────────────────────────────────┐
                    │  phpVMS 7 site (web)                │
                    │  ┌─────────────┐ ┌────────────────┐ │
                    │  │ Core API    │ │ CloudeAcars    │ │
                    │  │ (auth, ph,  │ │ phpVMS module  │ │
                    │  │ flights,    │ │ (config/rules/ │ │
                    │  │ pireps,…)   │ │ ext PIREP DB)  │ │
                    │  └─────────────┘ └────────────────┘ │
                    └──────────────▲──────────────────────┘
                                   │ HTTPS / JSON
                                   │ Bearer (API key)
                    ┌──────────────┴──────────────────────┐
                    │  CloudeAcars Desktop Client         │
                    │  Tauri 2 (Rust core + React UI)     │
                    │  ┌────────────────┐ ┌────────────┐  │
                    │  │ phpVMS API     │ │ UI (TS)    │  │
                    │  │ Client         │ │ i18n DE/EN │  │
                    │  └────────────────┘ └────────────┘  │
                    │  ┌────────────────┐ ┌────────────┐  │
                    │  │ Sim Adapter    │ │ Flight     │  │
                    │  │ (trait)        │ │ Recorder + │  │
                    │  │  ├ MSFS        │ │ Phase FSM  │  │
                    │  │  └ X-Plane     │ │ + Analyzer │  │
                    │  └───────┬────────┘ └────────────┘  │
                    │          │                           │
                    │  ┌───────▼────────┐ ┌────────────┐  │
                    │  │ Local SQLite   │ │ Secret     │  │
                    │  │ (queue + log)  │ │ Storage    │  │
                    │  │                │ │ (OS keyring)│ │
                    │  └────────────────┘ └────────────┘  │
                    └────┬───────────────────┬────────────┘
                         │ SimConnect IPC    │ UDP loopback
                         ▼                   ▼
                    ┌──────────┐     ┌────────────────┐
                    │ MSFS     │     │ X-Plane 11/12  │
                    │ 2020/24  │     │ + bundled XPLM │
                    │          │     │ plugin (.xpl)  │
                    └──────────┘     └────────────────┘
```

---

## 2. Components

### 2.1 Desktop Client (`client/`)

**Tauri 2** application. Two layers:

| Layer | Tech | Responsibility |
|---|---|---|
| **Core (backend)** | Rust | All non-UI logic: networking, simulator integration, flight recording, persistence, encryption. Runs in Tauri's main process. |
| **UI (frontend)** | TypeScript + React + Vite + i18next | Dashboard, flight selection, live status, flight log view, settings. Communicates with core via Tauri commands + events. |

**Why Tauri?** Small bundle (~10–20 MB), native webview (no Chromium shipping), Rust gives clean FFI for SimConnect, SQLite, and the XPLM bridge. ADR-0001.

#### Core internal modules (Rust crates within a Cargo workspace)

```
client/src-tauri/
├── crates/
│   ├── api-client/        # phpVMS HTTPS client (reqwest), retry/backoff, queue
│   ├── sim-core/          # SimAdapter trait, SimSnapshot model, phase FSM
│   ├── sim-msfs/          # SimConnect adapter (Win only, gated by feature flag)
│   ├── sim-xplane/        # X-Plane adapter (UDP listener, paired with xplane-plugin)
│   ├── recorder/          # Flight log + position history + landing analyzer
│   ├── storage/           # SQLite (rusqlite) — queue, logs, settings cache
│   ├── secrets/           # Cross-platform OS keyring wrapper (keyring crate)
│   ├── geo/               # Runway DB, great-circle math, centerline geometry
│   └── metar/             # METAR fetch + parse (e.g. NOAA / aviationweather.gov)
└── src/                   # Tauri main + command handlers gluing the crates
```

### 2.2 X-Plane plugin (`xplane-plugin/`)

Native plugin (`.xpl`) shipped inside the client installer. Loaded by X-Plane on startup. Reads X-Plane datarefs and pushes telemetry to the desktop client over **UDP loopback** (configurable port, defaulting to `49021`).

- **Language:** Rust via [`xplm-sys`](https://github.com/X-Plane/XPLM-SDK) bindings; fallback C++ if needed.
- **Binary outputs:**
  - `win.xpl` — Windows
  - `mac.xpl` — macOS (universal: x86_64 + arm64)
- **Installation:** copied by the desktop client's setup helper into the user's chosen X-Plane install (`<XP>/Resources/plugins/CloudeAcars/`).

### 2.3 phpVMS server module (`server-module/CloudeAcars/`)

A new phpVMS 7 module, drop-in into `phpvms/modules/`. Strictly no core modifications.

**Endpoints (HTTPS, JSON, `api.auth` middleware = phpVMS API key):**

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/cloudeacars/config` | Client config: rules, intervals, custom fields, version gating |
| `GET` | `/api/cloudeacars/version` | Latest client version + download URLs |
| `POST` | `/api/cloudeacars/heartbeat` | Liveness ping, used for "last seen" admin view |
| `POST` | `/api/cloudeacars/pirep/{id}/landing` | Submit landing-analysis fields (centerline, heading, threshold, METAR) |
| `POST` | `/api/cloudeacars/runway-data/missing` | Telemetry: report a runway not found in our DB |

For everything else (login validation, bids, flights, fleet, PIREP submit, ACARS positions), the client uses **phpVMS Core API** unchanged.

**Database additions (own migrations, prefix-respecting):**

- `cloudeacars_config` — KVP module config (rule thresholds, etc.)
- `cloudeacars_pirep_extra` — 1:1 with `pireps`, holds new fields (runway analysis, METAR snapshots)
- `cloudeacars_client_versions` — known client builds + min-version requirements

### 2.4 Shared protocol (`shared/`)

- JSON Schemas for: `SimSnapshot`, `FlightLogEvent`, `LandingAnalysis`, `PIREPExtra`
- OpenAPI 3.1 spec for the new `CloudeAcars` server endpoints
- Generates Rust types (via `schemars`) and TypeScript types (via `json-schema-to-typescript`) at build time

---

## 3. Data flow — happy path

```
User starts client
    └─► Login screen → POST /api/user/profile (phpVMS Core, API key)
        └─► User signs in → API key stored via OS keyring
            └─► Dashboard: GET /api/user/bids, GET /api/cloudeacars/config

Pilot picks a flight
    └─► Client launches sim adapter (MSFS or XP)
        └─► Sim Snapshot stream begins @ 1–10 Hz

Flight Recorder receives snapshots
    └─► Phase FSM updates phase
        └─► Position queue: every N seconds → POST /api/acars/{id}/position (Core)
            └─► If offline: persist to SQLite queue, retry with exponential backoff
        └─► Flight Log events emitted to UI + persisted

On touchdown
    └─► Landing Analyzer computes:
            • runway ident (nearest runway, heading-aligned)
            • centerline deviation (m)
            • heading deviation (°)
            • threshold distance (m)
            • landing rate, G-force, bounces
            • METAR snapshot (cached at touchdown time)

On flight end
    └─► PIREP body assembled (Core fields)
            POST /api/pireps/prefile + /api/pireps/{id}/file
        └─► Landing-analysis extras posted:
            POST /api/cloudeacars/pirep/{id}/landing
        └─► All position rows flushed
        └─► Local DB marks flight as "submitted"
```

---

## 4. Cross-cutting concerns

| Concern | Approach |
|---|---|
| **Secrets** | API key stored via OS keyring (`keyring` crate). Never in plaintext on disk. |
| **HTTPS** | Enforced. `rustls` (no OpenSSL dependency on Win/macOS). |
| **Offline tolerance** | All outbound API calls go through a SQLite-backed queue. |
| **Concurrency** | `tokio` async runtime in Rust. Each adapter runs in its own task. |
| **i18n** | UI: `react-i18next` with `de.json` + `en.json` resource bundles. Server messages: locale negotiated via `Accept-Language`. |
| **Logging** | `tracing` (Rust) + structured JSON logs to `%APPDATA%/CloudeAcars/logs/` (rolling, max 50 MB). |
| **Updates** | `tauri-plugin-updater` with code-signed payloads (Phase 5). |

---

## 5. What we explicitly are NOT building

- **No FSUIPC integration** — SimConnect only for MSFS. Hard rule, see ADR-0002.
- **No phpVMS core patches** — module-only, even if a feature would be nicer with a small core change.
- **No fork of vmsACARS** — clean-room build, the original installer in `reference/` is for documentation and API-shape inspection only, not for code reuse.
- **No Linux client in Phase 0–5** — kept architecturally possible (Tauri builds on Linux too), but not a delivery target until explicitly requested.

---

## 6. Open architecture questions (to resolve before each phase)

| # | Question | Phase | Owner |
|---|---|---|---|
| A1 | Runway DB source: bundle OurAirports CSV vs. license a commercial set vs. derive from sim data? | 3 | TBD |
| A2 | METAR provider: NOAA aviationweather.gov (free, rate-limited) vs. AVWX vs. CheckWX paid? | 3 | TBD |
| A3 | Update channel signing: Tauri's built-in updater key vs. external (Sigstore)? | 5 | TBD |
| A4 | macOS notarization: paid Apple Developer account required — who pays/owns? | 5 | TBD |
| A5 | Crash reporting: Sentry self-hosted vs. SaaS vs. local-only? | 4–5 | TBD |
