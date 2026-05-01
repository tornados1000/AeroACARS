# Architectural Decision Records (ADRs)

This directory captures *load-bearing* architectural decisions for CloudeAcars.

**Format:** [Michael Nygard's ADR template](https://cognitect.com/blog/2011/11/15/documenting-architecture-decisions).

**Filename convention:** `NNNN-kebab-case-title.md` where `NNNN` is a zero-padded sequence number, never reused.

**Lifecycle:** Once accepted, an ADR is immutable except for the `Status` field and a `Superseded by ADR-XXXX` note. Don't rewrite history; create a new ADR that supersedes the old one.

## Index

| # | Title | Status |
|---|---|---|
| [0001](0001-tauri-rust-react-stack.md) | Use Tauri (Rust + React) for the cross-platform desktop client | Accepted |
| [0002](0002-msfs-simconnect-only.md) | Use SimConnect only for MSFS — no FSUIPC | Accepted |
| [0003](0003-new-cloudeacars-phpvms-module.md) | Build a new `CloudeAcars` phpVMS module rather than fork `VMSAcars` | Accepted |
| [0004](0004-xplane-bundled-xplm-plugin.md) | Ship our own XPLM plugin for X-Plane 11/12 | Accepted |
| [0005](0005-license-freeware-closed-source.md) | License model: freeware (free for users), closed-source | Accepted |
| [0006](0006-bilingual-de-en.md) | Bilingual UI and docs (DE + EN) from day one | Accepted |
