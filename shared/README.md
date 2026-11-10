# shared/

Cross-component contracts: JSON Schemas, OpenAPI specs, and protocol definitions used by both the client and the server module.

**Status:** Phase 0 — placeholder. Schemas land alongside the components that need them (Phase 1+).

## Planned contents

| Path | Purpose |
|---|---|
| `schemas/sim-snapshot.json` | One snapshot of simulator telemetry, emitted by sim adapters |
| `schemas/flight-log-event.json` | A single event in the flight log |
| `schemas/landing-analysis.json` | Computed landing-analysis fields (centerline, heading, threshold, METAR) |
| `schemas/pirep-extra.json` | Extra PIREP fields submitted via `POST /api/cloudeacars/pirep/{id}/landing` |
| `openapi/cloudeacars.yaml` | OpenAPI 3.1 spec for the new server module endpoints |

## Codegen

At build time we generate:
- **Rust types** via [`schemars`](https://docs.rs/schemars) → consumed by `client/src-tauri/`
- **TypeScript types** via [`json-schema-to-typescript`](https://www.npmjs.com/package/json-schema-to-typescript) → consumed by `client/src/`
- **PHP types** (where useful) for the server module
