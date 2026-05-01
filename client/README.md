# client/

Tauri 2 desktop application — **CloudeAcars** client.

**Stack:** Rust core (`src-tauri/`) + React + TypeScript + Vite frontend (`src/`).

**Status:** Phase 0 — placeholder. Tauri scaffold will be created in Phase 1 once the Rust + Node toolchains are installed (see root `README.md`).

## Phase 1 scaffold (planned)

```
client/
├── src/                       # React + TS UI
│   ├── locales/{de,en}/       # i18n bundles
│   ├── components/
│   ├── pages/
│   └── main.tsx
├── src-tauri/                 # Rust core
│   ├── crates/
│   │   ├── api-client/        # phpVMS HTTPS client
│   │   ├── sim-core/          # SimAdapter trait + phase FSM
│   │   ├── sim-msfs/          # SimConnect adapter (Win-only)
│   │   ├── sim-xplane/        # X-Plane UDP adapter
│   │   ├── recorder/          # Flight log + landing analyzer
│   │   ├── storage/           # SQLite queue + log
│   │   ├── secrets/           # OS keyring wrapper
│   │   ├── geo/               # Runway DB + geometry
│   │   └── metar/             # METAR fetch + parse
│   ├── src/                   # Tauri main + command handlers
│   ├── Cargo.toml             # Rust workspace
│   └── tauri.conf.json
├── package.json
└── vite.config.ts
```
