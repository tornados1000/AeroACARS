//! Stability-Gate Konstanten. Spec §3.4 + §5.1: Werte 1:1 aus dem
//! Backend (`lib.rs:3042-3049` + `APPROACH_FLARE_CUTOFF_MS = 3000`
//! in `lib.rs:2883`) — NICHT umdefiniert, sonst stille Score-Window-
//! Migration.
//!
//! Bedeutung:
//! - Stability-Gate = `MIN < height <= MAX` (AGL ODER HAT)
//! - Flare-Zone = letzte FLARE_CUTOFF_MS vor TD (zeitbasiert)
//! - Bewertet wird `gate_sample = in_height_band AND NOT in_flare_zone`

pub const STABILITY_GATE_MAX_HEIGHT_FT: f32 = 1000.0;
pub const STABILITY_GATE_MIN_HEIGHT_FT: f32 = 0.0;
pub const STABILITY_GATE_FLARE_CUTOFF_MS: i64 = 3000;
