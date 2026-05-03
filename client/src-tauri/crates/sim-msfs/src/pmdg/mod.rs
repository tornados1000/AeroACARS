//! PMDG SimConnect SDK integration.
//!
//! Two PMDG aircraft families ship custom SimConnect ClientData
//! channels exposing the full cockpit state — **way** more than the
//! standard MSFS SimVars expose:
//!
//! * **NG3** — covers the PMDG 737-700/-800/-900 (and BBJ/BCF/BDSF
//!   variants of those airframes). Channel name `PMDG_NG3_Data`.
//! * **777X** — covers the PMDG 777-200LR / -300ER / 777F / 777W.
//!   Channel name `PMDG_777X_Data`.
//!
//! Both families share an identical SimConnect subscription pattern —
//! `MapClientDataNameToID`, `AddToClientDataDefinition`, `RequestClientData`
//! with `PERIOD_ON_SET + FLAG_CHANGED`. Only the names, IDs, and
//! struct shapes differ.
//!
//! # Activation by the pilot
//!
//! PMDG does NOT broadcast the data by default. The pilot must edit
//! the aircraft's options file and add:
//!
//! ```ini
//! [SDK]
//! EnableDataBroadcast=1
//! ```
//!
//! Files (auto-detected from the aircraft's installation path):
//!
//! * 737NG3: `<MSFS_Community>/pmdg-aircraft-738/work/737NG3_Options.ini`
//! * 777X:   `<MSFS_Community>/pmdg-aircraft-77er/work/777X_Options.ini`
//!
//! AeroACARS detects "PMDG aircraft loaded but no data flowing" and
//! surfaces a Settings-tab modal with the exact path and the lines
//! to add. See the `pmdg-sdk-integration.md` doc.
//!
//! # Memory layout — `#[repr(C)]`
//!
//! The PMDG SDK headers are MSVC C++. C++ `bool` is 1 byte, the rest
//! is the standard MSVC layout (4-byte int, 4-byte float, 8-byte
//! double, no struct-end padding for trailing-byte arrays). Rust
//! `#[repr(C)]` gives us the same layout because the alignment rules
//! match for all primitive types we use.
//!
//! Critical: **never** use `#[repr(packed)]` here — that would force
//! 1-byte alignment which differs from the MSVC default and would
//! mis-parse fields whose offset depends on alignment padding (e.g.
//! a `float` after a sequence of `bool`s).
//!
//! Each `*_RawData` struct mirrors the SDK header **field-by-field
//! in the exact same order**. We then expose a higher-level
//! `*_Snapshot` struct with only the fields useful to AeroACARS,
//! converted to ergonomic Rust types (`bool` for the C++ `bool`,
//! `Option<u16>` for "0 means not set", etc.).

pub mod ng3;
pub mod x777;

/// Variant tag for the PMDG aircraft we're talking to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmdgVariant {
    /// PMDG 737 NG3 family (700/800/900 + BBJ/BCF/BDSF).
    Ng3,
    /// PMDG 777X family (777-200LR / -300ER / 777F / 777W).
    X777,
}

impl PmdgVariant {
    /// Match the active aircraft's `.air` file path against the known
    /// PMDG installation prefixes. Returns `None` for non-PMDG aircraft.
    ///
    /// Pulled from `SimConnect_RequestSystemState("AircraftLoaded")`
    /// — that returns the .air file path the user loaded. Examples:
    /// `"E:\MSFS24_Community\Community\pmdg-aircraft-738\SimObjects\Airplanes\PMDG 737-800\aircraft.cfg"`
    pub fn detect_from_air_path(air_path: &str) -> Option<Self> {
        let p = air_path.to_lowercase();
        // Both naming conventions seen in real installations.
        if p.contains("pmdg-aircraft-737")
            || p.contains("pmdg-aircraft-738")
            || p.contains("pmdg-aircraft-739")
            || p.contains("pmdg 737")
        {
            Some(Self::Ng3)
        } else if p.contains("pmdg-aircraft-77")
            || p.contains("pmdg 777")
        {
            Some(Self::X777)
        } else {
            None
        }
    }

    /// Human-readable label for log lines / UI.
    pub fn label(self) -> &'static str {
        match self {
            Self::Ng3 => "PMDG 737 NG3",
            Self::X777 => "PMDG 777X",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PmdgVariant;

    #[test]
    fn detect_ng3_from_typical_paths() {
        assert_eq!(
            PmdgVariant::detect_from_air_path(
                "E:\\MSFS24_Community\\Community\\pmdg-aircraft-738\\SimObjects\\Airplanes\\PMDG 737-800\\aircraft.cfg"
            ),
            Some(PmdgVariant::Ng3)
        );
        assert_eq!(
            PmdgVariant::detect_from_air_path("/Some/Path/PMDG 737-900/aircraft.cfg"),
            Some(PmdgVariant::Ng3)
        );
    }

    #[test]
    fn detect_x777_from_typical_paths() {
        assert_eq!(
            PmdgVariant::detect_from_air_path(
                "E:\\MSFS24_Community\\Community\\pmdg-aircraft-77er\\SimObjects\\Airplanes\\PMDG 777-300ER\\aircraft.cfg"
            ),
            Some(PmdgVariant::X777)
        );
        assert_eq!(
            PmdgVariant::detect_from_air_path("path/pmdg-aircraft-77w/something"),
            Some(PmdgVariant::X777)
        );
        assert_eq!(
            PmdgVariant::detect_from_air_path("PMDG 777-200LR"),
            Some(PmdgVariant::X777)
        );
    }

    #[test]
    fn detect_returns_none_for_other_aircraft() {
        assert_eq!(
            PmdgVariant::detect_from_air_path(
                "/Asobo/SimObjects/Airplanes/Asobo_A320_NEO/aircraft.cfg"
            ),
            None
        );
        assert_eq!(
            PmdgVariant::detect_from_air_path(
                "/Community/flybywire-aircraft-a320-neo/aircraft.cfg"
            ),
            None
        );
        assert_eq!(PmdgVariant::detect_from_air_path(""), None);
    }
}
