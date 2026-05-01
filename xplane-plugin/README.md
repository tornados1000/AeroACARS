# xplane-plugin/

XPLM plugin for X-Plane 11 / X-Plane 12 — bundled with the CloudeAcars desktop client installer and copied into `<X-Plane>/Resources/plugins/CloudeAcars/` on first run.

**Status:** Phase 0 — placeholder. Real plugin code starts in **Phase 2**.

## Plan

- **Language:** Rust via `xplm-sys` bindings.
- **Outputs:**
  - `win.xpl` — Windows
  - `mac.xpl` — macOS universal (x86_64 + arm64)
- **Wire format:** UDP loopback packets, CBOR-encoded `SimSnapshot`.
- **Default port:** 49021 (configurable in client settings).

## What this plugin does

1. Loads inside X-Plane on startup.
2. Subscribes to a curated set of datarefs (position, attitude, speeds, fuel, gear, flaps, on-ground, G-force, wind, QNH, sim version, etc.).
3. At a configurable rate (default 5 Hz) pushes a `SimSnapshot` packet to the desktop client over UDP loopback.
4. Receives lightweight control packets back (e.g. "set tracking enabled", "send ack").
