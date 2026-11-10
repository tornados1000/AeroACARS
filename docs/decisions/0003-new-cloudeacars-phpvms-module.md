# ADR-0003: Build a new `CloudeAcars` phpVMS module rather than fork `VMSAcars`

- **Status:** Accepted
- **Date:** 2026-05-01
- **Deciders:** Project owner

## Context

phpVMS 7 sites already commonly have the `VMSAcars` module installed (the server-side companion of the closed-source vmsACARS client). CloudeAcars needs server-side support for new fields (runway analysis, METAR snapshots, landing scoring, client-version tracking).

Two paths:
- **A:** Fork or extend the existing `VMSAcars` module.
- **B:** Build a new, separate module called `CloudeAcars`.

## Decision

Build a **new, independent `CloudeAcars` phpVMS module** (Option B). The existing `VMSAcars` module stays untouched as reference material in `reference/`.

## Rationale

- Clean schema for new fields without polluting `VMSAcars` migrations.
- Independent versioning — our minimum-version gating logic, our pace.
- VAs can run both modules side-by-side during a migration period.
- Avoids any concern about modifying code we did not author.
- Per project owner: *"B"* (kickoff, 2026-05-01).

## Module surface (initial)

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/cloudeacars/config` | Rules, intervals, custom fields, version gating |
| `GET` | `/api/cloudeacars/version` | Latest client version + download URLs |
| `POST` | `/api/cloudeacars/heartbeat` | Liveness ping |
| `POST` | `/api/cloudeacars/pirep/{id}/landing` | Submit landing-analysis fields |
| `POST` | `/api/cloudeacars/runway-data/missing` | Telemetry — runway not in DB |

Authentication: phpVMS standard `api.auth` (API key in `X-API-Key` header). No new auth scheme.

Database tables (with phpVMS table prefix):
- `cloudeacars_config`
- `cloudeacars_pirep_extra` (1:1 with `pireps`)
- `cloudeacars_client_versions`

For users/bids/flights/fleet/PIREP submit/ACARS positions, the **client uses phpVMS Core API directly** — `CloudeAcars` server module does *not* duplicate those endpoints.

## Consequences

- **Positive:** Clean separation; no risk of breaking sites that still use `vmsACARS`.
- **Positive:** Easy uninstall path — drop the module folder, run `php artisan module:uninstall`.
- **Negative:** A site can have both modules at once; we must document that this is fine and which one a given client talks to (CloudeAcars only ever talks to itself + Core; never to `VMSAcars`).
