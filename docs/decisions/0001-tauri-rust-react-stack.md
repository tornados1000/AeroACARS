# ADR-0001: Use Tauri (Rust + React) for the cross-platform desktop client

- **Status:** Accepted
- **Date:** 2026-05-01
- **Deciders:** Project owner

## Context

CloudeAcars must run **natively on Windows and macOS** (Linux optional later) with:

- A modern UI (dashboard, flight log, live status, dark mode, i18n DE/EN).
- High-frequency simulator data ingestion (1–10 Hz from SimConnect / X-Plane).
- Local persistent storage (SQLite-based offline queue + flight log).
- Small installer footprint (pilots are end users, not engineers).
- Bundled X-Plane plugin shipped from the same installer.
- Future code-signing for both OSes.

## Considered options

| Option | Pros | Cons |
|---|---|---|
| **Tauri 2 (Rust core + Web UI)** | Small bundle (~10–20 MB), native webview, Rust gives clean FFI to SimConnect/XPLM via `bindgen`, excellent SQLite support (`rusqlite`), modern UI ecosystem (React/Svelte/Vue). Cross-OS code signing well documented. | Two languages (Rust + TS). Rust learning curve. |
| Electron + Node | Largest UI ecosystem, fastest UI dev. | 150+ MB bundle, native sim integration is painful (N-API/NAPI-RS or external Rust sidecar — at which point we have Tauri's downsides without its upsides). |
| Qt (C++ or PySide) | Mature, native widgets. | UI feels dated, more boilerplate, smaller modern frontend ecosystem. |
| .NET MAUI / Avalonia | Excellent on Windows, SimConnect bindings native to .NET. | macOS support is the weaker leg of MAUI; less mature than Tauri on macOS. |
| Flutter Desktop | One language, decent UI. | Native sim integration via FFI is workable but less idiomatic; smaller desktop ecosystem. |

## Decision

Use **Tauri 2** with a **Rust core** and a **React + TypeScript + Vite** frontend.

- Rust crates for: `simconnect` (Win-only adapter), `xplm-sys`-based bridge (X-Plane), `rusqlite`, `reqwest` (HTTPS client), `keyring` (OS secret storage), `tracing` (logging).
- Frontend with `react-i18next` for DE/EN, dark mode via Tailwind / shadcn-ui (TBD in a later ADR).

## Consequences

- **Positive:** Small installer; cross-OS works with one codebase; sim integration in Rust is the cleanest option among all considered.
- **Positive:** `tauri-plugin-updater` gives us code-signed auto-updates for free in Phase 5.
- **Negative:** Two languages to staff/learn. We commit to Rust for all backend/sim/recording logic.
- **Negative:** Rust + MSVC linker means Windows developers must install Visual Studio Build Tools (one-time setup).

## References

- [Tauri docs](https://tauri.app/start/)
- [Rust + Tauri sample apps](https://github.com/tauri-apps/awesome-tauri)
