//! v0.12.2 — X-Plane Aircraft-DataRef-Profile (Study-Level-Add-ons).
//!
//! Spec: docs/spec/v0.12.2-xplane-aircraft-dataref-profiles.md
//!
//! Study-level X-Plane add-ons (Hot-Start CL650, ToLiss, FlightFactor …)
//! run cockpit/system functions through their **own** datarefs and don't
//! always drive the standard `sim/...` ones. The CL650 e.g. never drives
//! `sim/flightmodel2/controls/flap_handle_deploy_ratio`, so AeroACARS
//! could not see the flaps (GSG225 bug → LANDING CONFIG "INCOMPLETE").
//!
//! An `XplaneAircraftProfile` swaps the dataref source for specific
//! `FieldId`s when a known study-level aircraft is detected. Aircraft
//! without a profile keep the static `CATALOG` unchanged (LE5).

use crate::dataref::{FieldId, CATALOG};

/// Tolerance for the near-integer check in `ValueMapping::DetentTable`
/// (LE7). A documented integer detent must arrive close to a whole
/// number; `1.51` is not a clean detent and must not snap to `2`.
pub const DETENT_TOLERANCE: f32 = 0.05;

/// How a raw RREF value is mapped onto the internal field value.
#[derive(Debug, Clone, Copy)]
pub enum ValueMapping {
    /// Take the raw value 1:1 (e.g. int 0/1 booleans).
    Passthrough,
    /// Map an integer detent index → internal `f32` via a lookup table.
    /// Out-of-range / non-integer / non-finite → no value (LE7).
    DetentTable(&'static [f32]),
}

impl ValueMapping {
    /// Map a raw RREF `f32` to the internal field value. Returns `None`
    /// when the override should yield **no** value — the caller then
    /// leaves the field at its standard/`None` state (LE7: no panic,
    /// no silent `1.0`).
    pub fn map(&self, raw: f32) -> Option<f32> {
        match self {
            ValueMapping::Passthrough => Some(raw),
            ValueMapping::DetentTable(table) => {
                if !raw.is_finite() {
                    tracing::warn!(raw, "profile DetentTable: non-finite value ignored");
                    return None;
                }
                let rounded = raw.round();
                if (raw - rounded).abs() > DETENT_TOLERANCE {
                    tracing::debug!(raw, "profile DetentTable: not a clean integer detent");
                    return None;
                }
                let idx = rounded as i32;
                if idx < 0 || idx as usize >= table.len() {
                    tracing::debug!(raw, idx, "profile DetentTable: detent out of range");
                    return None;
                }
                Some(table[idx as usize])
            }
        }
    }
}

/// One dataref-source override: read `field` from `dataref` (an
/// aircraft-specific dataref) instead of the standard `CATALOG` entry,
/// applying `mapping`.
#[derive(Debug, Clone, Copy)]
pub struct DatarefOverride {
    pub field: FieldId,
    pub dataref: &'static str,
    pub mapping: ValueMapping,
}

/// A per-aircraft dataref profile (LE2).
#[derive(Debug, Clone, Copy)]
pub struct XplaneAircraftProfile {
    /// Human-readable name (logging only).
    pub name: &'static str,
    /// Substrings that must **all** appear (case-insensitive) in the
    /// `aircraft_title` for the title-match stage (LE1 stage 1).
    pub title_match: &'static [&'static str],
    /// Signature dataref for the probe stage (LE1 stage 2): if X-Plane
    /// returns a value for it, the aircraft is this profile's aircraft.
    pub probe_dataref: &'static str,
    /// Dataref-source overrides applied while this profile is active.
    pub overrides: &'static [DatarefOverride],
}

impl XplaneAircraftProfile {
    /// LE1 stage 1: does `title` contain every `title_match` substring?
    pub fn matches_title(&self, title: &str) -> bool {
        if self.title_match.is_empty() {
            return false;
        }
        let lower = title.to_lowercase();
        self.title_match
            .iter()
            .all(|needle| lower.contains(&needle.to_lowercase()))
    }
}

/// CL650 flap-lever detent → internal `flaps_position`. Verified against
/// the Hot-Start CL650 documentation (`Wires.txt`, module `FLAP_IND`):
/// lever 0/1/2/3 = flaps 0°/20°/30°/45°. The internal LANDING-CONFIG
/// check is `flaps_position >= 0.70`, so lever 2 (Flaps 30) and 3
/// (Flaps 45) count as landing config, lever 0/1 (Flaps 0/20) do not.
pub const CL650_FLAP_DETENTS: &[f32] = &[0.0, 0.45, 0.80, 1.0];

/// All known aircraft profiles (LE2). The first one whose detection
/// matches wins.
pub const PROFILES: &[XplaneAircraftProfile] = &[
    // Hot-Start Challenger 650 (X-Aviation). GSG225-Befund: treibt die
    // Standard-Flaps-DataRef nicht.
    XplaneAircraftProfile {
        name: "Hot Start CL650",
        title_match: &["challenger 650", "x-aviation"],
        probe_dataref: "abus/CL650/ARINC429/L-DCU-7/words/FCTL/0/FLAPS_LVR",
        overrides: &[
            DatarefOverride {
                field: FieldId::FlapsHandle,
                dataref: "abus/CL650/ARINC429/L-DCU-7/words/FCTL/0/FLAPS_LVR",
                mapping: ValueMapping::DetentTable(CL650_FLAP_DETENTS),
            },
            DatarefOverride {
                field: FieldId::BatteryMaster,
                dataref: "abus/CL650/modules/DC_ELEC/0/wires/BATT_CTRL_PWR",
                mapping: ValueMapping::Passthrough,
            },
            DatarefOverride {
                field: FieldId::LightBeacon,
                dataref: "CL650/overhead/ext_lts/beacon",
                mapping: ValueMapping::Passthrough,
            },
            DatarefOverride {
                field: FieldId::LightTaxi,
                dataref: "CL650/overhead/land_lts/recog_taxi",
                mapping: ValueMapping::Passthrough,
            },
        ],
    },
];

/// LE1 stage 1: first profile whose title-match accepts `title`.
pub fn profile_index_for_title(title: &str) -> Option<usize> {
    PROFILES.iter().position(|p| p.matches_title(title))
}

/// One row of the **active catalog** (LE6): the dataref the adapter
/// actually subscribes for a given `FieldId`, plus its mapping. Built
/// from the static `CATALOG` with the active profile's overrides
/// applied. Index = wire index = position in the active catalog.
#[derive(Debug, Clone, Copy)]
pub struct ActiveEntry {
    pub name: &'static str,
    pub field: FieldId,
    pub mapping: ValueMapping,
}

/// Build the active catalog (LE6). Starts from the static `CATALOG`;
/// when `profile` is `Some`, every override **replaces** the base entry
/// of the same `FieldId` (same index, new dataref + mapping). All
/// current profile overrides target `FieldId`s that already exist in
/// the base catalog, so the active catalog has the **same length and
/// indices** as `CATALOG` — `seen`/`last_values` need no resize.
pub fn build_active_catalog(profile: Option<&XplaneAircraftProfile>) -> Vec<ActiveEntry> {
    CATALOG
        .iter()
        .map(|base| {
            if let Some(p) = profile {
                if let Some(ovr) = p.overrides.iter().find(|o| o.field == base.field) {
                    return ActiveEntry {
                        name: ovr.dataref,
                        field: base.field,
                        mapping: ovr.mapping,
                    };
                }
            }
            ActiveEntry {
                name: base.name,
                field: base.field,
                mapping: ValueMapping::Passthrough,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cl650() -> &'static XplaneAircraftProfile {
        &PROFILES[0]
    }

    #[test]
    fn cl650_profile_matches_title() {
        assert!(cl650().matches_title("Challenger 650 published by X-Aviation"));
        // case-insensitive
        assert!(cl650().matches_title("CHALLENGER 650 — X-AVIATION"));
    }

    #[test]
    fn default_challenger_does_not_match_title() {
        // A default / non-Hot-Start Challenger lacks "X-Aviation".
        assert!(!cl650().matches_title("Bombardier Challenger 650"));
        assert!(!cl650().matches_title("Cessna 172"));
    }

    #[test]
    fn profile_index_for_title_picks_cl650() {
        assert_eq!(
            profile_index_for_title("Challenger 650 published by X-Aviation"),
            Some(0),
        );
        assert_eq!(profile_index_for_title("Felis 747-200"), None);
    }

    #[test]
    fn flaps_detent_table_maps_lever_to_ratio() {
        let m = ValueMapping::DetentTable(CL650_FLAP_DETENTS);
        assert_eq!(m.map(0.0), Some(0.0));
        assert_eq!(m.map(1.0), Some(0.45));
        assert_eq!(m.map(2.0), Some(0.80));
        assert_eq!(m.map(3.0), Some(1.0));
    }

    #[test]
    fn flaps_lever_0_and_1_not_landing_config() {
        let m = ValueMapping::DetentTable(CL650_FLAP_DETENTS);
        assert!(m.map(0.0).unwrap() < 0.70);
        assert!(m.map(1.0).unwrap() < 0.70);
    }

    #[test]
    fn flaps_lever_2_and_3_are_landing_config() {
        let m = ValueMapping::DetentTable(CL650_FLAP_DETENTS);
        assert!(m.map(2.0).unwrap() >= 0.70);
        assert!(m.map(3.0).unwrap() >= 0.70);
    }

    #[test]
    fn detent_table_nonfinite_yields_no_value() {
        let m = ValueMapping::DetentTable(CL650_FLAP_DETENTS);
        assert_eq!(m.map(f32::NAN), None);
        assert_eq!(m.map(f32::INFINITY), None);
    }

    #[test]
    fn detent_table_rejects_non_integer_value() {
        let m = ValueMapping::DetentTable(CL650_FLAP_DETENTS);
        // 1.51 must NOT snap onto detent 2 (LE7 / QS-R2-P2).
        assert_eq!(m.map(1.51), None);
        assert_eq!(m.map(1.49), None);
        // within tolerance is still accepted
        assert_eq!(m.map(2.03), Some(0.80));
    }

    #[test]
    fn detent_table_out_of_range_yields_no_value() {
        let m = ValueMapping::DetentTable(CL650_FLAP_DETENTS);
        assert_eq!(m.map(-1.0), None);
        assert_eq!(m.map(4.0), None);
    }

    #[test]
    fn passthrough_keeps_raw_value() {
        assert_eq!(ValueMapping::Passthrough.map(1.0), Some(1.0));
        assert_eq!(ValueMapping::Passthrough.map(0.0), Some(0.0));
    }

    #[test]
    fn active_catalog_without_profile_equals_base() {
        let active = build_active_catalog(None);
        assert_eq!(active.len(), CATALOG.len());
        for (a, base) in active.iter().zip(CATALOG.iter()) {
            assert_eq!(a.name, base.name);
            assert_eq!(a.field, base.field);
            assert!(matches!(a.mapping, ValueMapping::Passthrough));
        }
    }

    #[test]
    fn active_catalog_override_replaces_same_fieldid() {
        let active = build_active_catalog(Some(cl650()));
        // same length + indices as base (LE6).
        assert_eq!(active.len(), CATALOG.len());
        // the FlapsHandle slot now points at the CL650 dataref.
        let flaps = active
            .iter()
            .find(|e| e.field == FieldId::FlapsHandle)
            .expect("FlapsHandle in catalog");
        assert_eq!(flaps.name, "abus/CL650/ARINC429/L-DCU-7/words/FCTL/0/FLAPS_LVR");
        assert!(matches!(flaps.mapping, ValueMapping::DetentTable(_)));
        // a non-overridden field keeps its base dataref.
        let lat = active
            .iter()
            .find(|e| e.field == FieldId::Latitude)
            .expect("Latitude in catalog");
        assert_eq!(lat.name, "sim/flightmodel/position/latitude");
    }
}
