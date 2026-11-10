# ADR-0002: Use SimConnect only for MSFS — no FSUIPC

- **Status:** Accepted
- **Date:** 2026-05-01
- **Deciders:** Project owner

## Context

MSFS 2020 and MSFS 2024 must be supported by the CloudeAcars client. Two community-standard ways to read sim telemetry exist:

1. **SimConnect** — the official Microsoft API, ships with MSFS, free, header + lib + redistributable DLL.
2. **FSUIPC** — a paid third-party add-on (Pete Dowson) with a richer feature set in some areas; widely used by older add-ons.

## Decision

Use **SimConnect exclusively**. Do not link, ship, depend on, or fall back to FSUIPC for any feature.

## Rationale (project owner statement)

> *"ich möchte euch kein FSUIPC mehr nutzen nur SimConnect in MSFS"* (kickoff, 2026-05-01)

End users should have to install nothing beyond MSFS itself to run CloudeAcars. FSUIPC is a paid add-on; requiring it would be a hard barrier to adoption.

## Implementation guidance

- Bind only against the official SimConnect SDK (`SimConnect.h`, `SimConnect.lib`, `SimConnect.dll`). The SDK ships with MSFS — accessible via Devmode → SDK Installer.
- Reference docs: <https://docs.flightsimulator.com/html/Programming_Tools/SimConnect/SimConnect_SDK.htm>
- Rust integration plan:
  1. **Phase 1:** Use a published crate (e.g. `simconnect-sdk` or `simconnect`) for fastest start.
  2. **Later (if needed):** Generate own bindings via `bindgen` against the SDK header for full control or to support new MSFS-2024-specific events.
- If a data point seems missing in SimConnect, exhaust SimConnect-internal options first:
  - Standard SimVars
  - Custom L: vars
  - WASM gauge → SimConnect bridge
  - RPN-driven custom events
- **Never** consider FSUIPC, even as a fallback.

## Consequences

- **Positive:** Zero third-party dependency for end users. No license fees. Aligned with Microsoft's official direction.
- **Positive:** SimConnect is statically more stable across MSFS updates than FSUIPC has historically been.
- **Negative:** A handful of niche datapoints that FSUIPC exposes via offsets are not directly available — we accept the engineering effort to obtain them via SimConnect-native means.

## References

- [SimConnect SDK documentation (official)](https://docs.flightsimulator.com/html/Programming_Tools/SimConnect/SimConnect_SDK.htm)
- Memory: `feedback_simconnect_only.md`
