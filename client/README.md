# client/

AeroACARS desktop client — **Tauri 2** (Rust core) + **React + TypeScript + Vite** (frontend).

Bilingual UI (DE / EN) with `react-i18next`, dark-mode aware via CSS variables driven by `data-theme` on `<html>`.

## Layout

```
client/
├── index.html
├── package.json                  # Frontend deps + scripts
├── vite.config.ts
├── tsconfig.json
├── tsconfig.node.json
├── src/                          # React + TS UI
│   ├── App.tsx                   # Phase 1 dashboard skeleton
│   ├── App.css                   # Theme tokens (light/dark) + base styles
│   ├── main.tsx                  # React entry, applies theme + i18n
│   ├── theme.ts                  # Light/dark switcher with localStorage persist
│   ├── i18n/index.ts             # i18next setup, DE+EN bundles, lang detector
│   └── locales/
│       ├── de/common.json
│       └── en/common.json
└── src-tauri/                    # Rust workspace (Tauri app + internal crates)
    ├── Cargo.toml                # Workspace root + Tauri app package
    ├── tauri.conf.json
    ├── build.rs
    ├── src/
    │   ├── main.rs               # Tauri binary entry
    │   └── lib.rs                # Tracing init + Tauri builder + commands
    └── crates/
        ├── api-client/           # phpVMS HTTPS client
        ├── sim-core/             # SimAdapter trait, SimSnapshot, FlightPhase
        ├── sim-msfs/             # MSFS SimConnect adapter (Windows-only)
        ├── sim-xplane/           # X-Plane UDP listener (Phase 2)
        ├── recorder/             # Flight log + landing analyzer
        ├── storage/              # SQLite offline queue + log
        ├── secrets/              # OS keyring wrapper
        ├── geo/                  # Runway DB + great-circle geometry
        └── metar/                # METAR fetch + parse
```

## Develop

```powershell
# from client/
npm install            # one-time: installs Node deps and the local Tauri CLI
npm run tauri dev      # starts Vite dev server + builds Rust + opens the window
```

The first `cargo build` pulls a lot of crates and is slow (~5–10 min). Subsequent rebuilds are incremental.

## Build (release)

```powershell
npm run tauri build
```

Outputs platform-specific installers (Windows MSI/NSIS, macOS .app/DMG) under `src-tauri/target/release/bundle/`.

## Phase status

- ✅ **Phase 0:** Repo + spec + ADRs.
- 🟡 **Phase 1:** Tauri scaffold (this commit), bilingual UI skeleton, internal crate stubs. `app_info` Tauri command exposed; everything else is TODO inside the crates.
- ⬜ Phase 2: X-Plane plugin + adapter, flight phase FSM, full flight log.
- ⬜ Phase 3: Runway DB, METAR, landing analyzer.
- ⬜ Phase 4: `AeroACARS` phpVMS server module (lives in `../server-module/`).
- ⬜ Phase 5: Update system, code-signing, installer hardening.

## Important constraints

- **MSFS:** SimConnect only, never FSUIPC (ADR-0002).
- **Identifiers + comments:** English. UI strings and end-user docs: bilingual DE/EN.
- **Secrets:** API keys never on disk in plaintext — go through `secrets` crate (OS keyring).
