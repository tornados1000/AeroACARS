# scripts/

Build, packaging, and developer-tooling scripts for CloudeAcars.

**Status:** Phase 0 — placeholder. Scripts will be added on demand.

## Planned scripts (Phase 1–5)

| Script | Purpose |
|---|---|
| `bootstrap.ps1` / `bootstrap.sh` | Install Tauri CLI + project deps after Rust+Node are present |
| `build-client.ps1` | Build a release Tauri bundle for the host OS |
| `build-xplane-plugin.ps1` | Build `win.xpl` and `mac.xpl` |
| `gen-types.ps1` | Generate Rust + TS types from `shared/schemas/` |
| `sign-release.ps1` | Code-sign the Windows installer (Phase 5) |
| `notarize-mac.sh` | Notarize the macOS app bundle (Phase 5) |
