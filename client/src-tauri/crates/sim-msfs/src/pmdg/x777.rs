//! PMDG 777X SimConnect SDK data structures.
//!
//! Mirrors `PMDG_777X_SDK.h` (Copyright PMDG, ships with the
//! pmdg-aircraft-77er / -77w / -77f / -77l installations under
//! `Documentation/SDK/PMDG_777X_SDK.h`).
//!
//! # Status
//!
//! **Phase 5.1 stub** — only the SimConnect identifiers are
//! defined. The full `Pmdg777XRawData` struct replication (~1675
//! lines of header → equivalent Rust struct) lands in a follow-up
//! commit. The pattern mirrors `ng3.rs` with the differences:
//!
//! * Three CDU channels (vs. two on NG3) — captain, F/O, AUX FO
//! * Different MCP layout: includes FPA mode toggle, AUTO bank limit
//! * EFB display ClientData channel exists (NG3 doesn't have one)
//! * `MCP_FPA: f32` field for Flight Path Angle approach mode
//!
//! Until then, AeroACARS detects 777X aircraft + logs that the SDK
//! integration isn't wired yet — falls back to standard MSFS SimVars
//! for that aircraft. Same auto-detection path, just no PMDG-data
//! upgrade.

#![allow(dead_code)]

// ---------------------------------------------------------------
// SimConnect ClientData identifiers — the bit we DO need today, so
// the auto-detection can correctly identify 777X aircraft via the
// `AircraftLoaded` system state and decline the data subscription
// gracefully (rather than crashing on an unknown variant).
// ---------------------------------------------------------------

pub const PMDG_777X_DATA_NAME: &str = "PMDG_777X_Data";
pub const PMDG_777X_DATA_ID: u32 = 0x504D_4447;
pub const PMDG_777X_DATA_DEFINITION: u32 = 0x504D_4448;
pub const PMDG_777X_CONTROL_NAME: &str = "PMDG_777X_Control";
pub const PMDG_777X_CONTROL_ID: u32 = 0x504D_4449;
pub const PMDG_777X_CONTROL_DEFINITION: u32 = 0x504D_444A;
pub const PMDG_777X_CDU_0_NAME: &str = "PMDG_777X_CDU_0";
pub const PMDG_777X_CDU_1_NAME: &str = "PMDG_777X_CDU_1";
pub const PMDG_777X_CDU_2_NAME: &str = "PMDG_777X_CDU_2";

/// Aircraft variant per the .air path. Unlike NG3 (which exposes a
/// numeric `AircraftModel` field in the data block), the 777X SDK
/// uses path-based detection only.
#[allow(non_camel_case_types)] // Match Boeing model designators verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pmdg777XVariant {
    /// 777-200LR (`pmdg-aircraft-77l`)
    B777_200LR,
    /// 777-300ER (`pmdg-aircraft-77er`)
    B777_300ER,
    /// 777-200F freighter (`pmdg-aircraft-77f`)
    B777_200F,
    /// 777-W variant (`pmdg-aircraft-77w`)
    B777_W,
    Unknown,
}

impl Pmdg777XVariant {
    pub fn from_air_path(p: &str) -> Self {
        let lp = p.to_lowercase();
        if lp.contains("pmdg-aircraft-77l") || lp.contains("777-200lr") {
            Self::B777_200LR
        } else if lp.contains("pmdg-aircraft-77er") || lp.contains("777-300er") {
            Self::B777_300ER
        } else if lp.contains("pmdg-aircraft-77f") || lp.contains("777f") {
            Self::B777_200F
        } else if lp.contains("pmdg-aircraft-77w") {
            Self::B777_W
        } else {
            Self::Unknown
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::B777_200LR => "777-200LR",
            Self::B777_300ER => "777-300ER",
            Self::B777_200F => "777-200F",
            Self::B777_W => "777-W",
            Self::Unknown => "777 (unknown)",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Pmdg777XVariant;

    #[test]
    fn variant_path_detection() {
        assert_eq!(
            Pmdg777XVariant::from_air_path(
                "E:\\MSFS24_Community\\Community\\pmdg-aircraft-77er\\..."
            ),
            Pmdg777XVariant::B777_300ER
        );
        assert_eq!(
            Pmdg777XVariant::from_air_path("/something/pmdg-aircraft-77l/..."),
            Pmdg777XVariant::B777_200LR
        );
        assert_eq!(
            Pmdg777XVariant::from_air_path("/random/path"),
            Pmdg777XVariant::Unknown
        );
    }
}
