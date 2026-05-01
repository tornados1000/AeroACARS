# ADR-0004: Ship our own XPLM plugin for X-Plane 11/12

- **Status:** Accepted
- **Date:** 2026-05-01
- **Deciders:** Project owner

## Context

X-Plane 11/12 must be supported on Windows and macOS. Unlike MSFS (which exposes SimConnect over an IPC channel that any external process can use), X-Plane integrations are typically implemented as **in-process plugins** loaded by X-Plane itself, using the **XPLM SDK**.

Three approaches:
1. Use a third-party XP-bridge tool the user must install separately (e.g. ExtPlane, XPUIPC). Not aligned with our "no extra dependencies" stance.
2. Build our own XPLM plugin and bundle it.
3. Use only X-Plane's UDP "data output" (built-in, but limited dataset and no two-way control).

## Decision

**Build our own XPLM plugin** and ship it inside the CloudeAcars installer. The plugin runs inside X-Plane and forwards a curated dataref/event stream to the desktop client over **UDP loopback** on a configurable port (default 49021).

## Rationale

- Matches MSFS behavior in spirit: no third-party dependencies for the end user.
- Full access to all X-Plane datarefs (including ones not in the built-in UDP feed).
- Cross-platform from one source (XPLM is portable; we produce `win.xpl` and `mac.xpl`).
- Industry standard — vmsACARS, smartCARS, FSAcars all do this.

## Implementation guidance

- **Language:** Rust via [`xplm-sys`](https://docs.rs/xplm-sys) bindings preferred. Fall back to C/C++ only if a blocker emerges.
- **Output binaries:**
  - Windows: `CloudeAcars/win.xpl` (or per platform layout per X-Plane SDK conventions)
  - macOS: `CloudeAcars/mac.xpl` (universal, x86_64 + arm64)
- **Installation flow:** During first launch, the desktop client asks the user to point at their X-Plane install folder. The client's setup helper then copies the plugin folder to `<XP>/Resources/plugins/CloudeAcars/`. Uninstall = remove that folder.
- **Wire format:** UDP packets carrying CBOR-encoded `SimSnapshot` messages. Schema in `shared/`.
- **Port:** Default 49021, configurable in client settings to avoid clashes.

## Consequences

- **Positive:** End user installs nothing extra — installer handles everything.
- **Positive:** Same telemetry quality on X-Plane as on MSFS.
- **Negative:** Plugin must be built and code-signed per platform (macOS notarization required for X-Plane to load the plugin without warnings).
- **Negative:** XPLM API changes between X-Plane major versions are minimal but not zero — we test against XP 11 and XP 12 each release.
