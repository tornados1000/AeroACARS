# PMDG SDK Integration — Implementation Plan

**Status**: Investigation complete, ready for phase 1 implementation
**Branch**: `feat/pmdg-ng3-sdk`
**Target Release**: v0.2.0 — "Boeing Premium Telemetry"
**Author**: AeroACARS Team
**Date**: 2026-05-03

---

## 1. Why — what's in it for us?

PMDG aircraft (737 NG3, 777X) have an official SimConnect SDK that delivers **cockpit state directly from the aircraft**. Standard MSFS SimVars don't expose this — no other phpVMS ACARS client (vmsACARS, SmartCARS, Volanta) uses this data.

What we can do with it:
- **Real FMA modes** (TOGA / N1 / SPD / ARM / etc.) live in the activity log
- **MCP settings** (selected SPD/HDG/ALT/VS) directly from the MCP instead of reverse-calculated from FCU SimVars
- **V-speeds** (V1/VR/V2/VREF) in the PIREP — computed by the FMC, not guessed
- **FMC values** (cruise alt, distance to TOD, distance to dest, flight number)
- **Real flap position in degrees** (`MAIN_TEFlapsNeedle[2]`) instead of detent quantization
- **Aircraft variant** (737-700/-800/-900/etc.) more precisely than our `aircraft_profile`
- **Autobrake setting** (RTO/OFF/1/2/3/MAX)
- **TO config warning** in the activity log

Story for the users:
> AeroACARS v0.2.0 — Boeing Premium Telemetry
> Direct SDK integration for PMDG 737 + 777. FMA, MCP, V-speeds, FMC data — all live in the cockpit tab and in the PIREP. Detects automatically whether you're flying a PMDG.

---

## 2. Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                        sim-msfs Crate                           │
│                                                                 │
│  ┌─────────────────────┐       ┌─────────────────────┐         │
│  │  Standard Telemetry │       │   PMDG ClientData   │         │
│  │  (existing)         │       │   (new)             │         │
│  │                     │       │                     │         │
│  │  SimConnect Data    │       │  SimConnect Client  │         │
│  │  Definition #1      │       │  Data #2            │         │
│  │  → Telemetry struct │       │  → PMDG_NG3_Data    │         │
│  │                     │       │     OR              │         │
│  │  All aircraft       │       │  → PMDG_777X_Data   │         │
│  │  (Asobo, FBW,       │       │                     │         │
│  │   Fenix, PMDG, INI) │       │  Only when PMDG     │         │
│  │                     │       │  loaded + SDK on    │         │
│  └─────────────────────┘       └─────────────────────┘         │
│           │                              │                      │
│           ▼                              ▼                      │
│      SimSnapshot.* ◄──── merge ──── PmdgSnapshot                │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
                          │
                          ▼
                    AeroACARS App
                  (Cockpit, PIREP, Activity Log)
```

Both subscriptions run **in parallel**. If the pilot flies a non-PMDG aircraft, only the standard telemetry runs. With PMDG: additionally PMDG data.

---

## 3. Aircraft detection flow

```
1. SimConnect_Open() confirms connection
2. SimConnect_RequestSystemState("AircraftLoaded") → returns .air file path
3. Path match:
   - Contains "PMDG 737" / "pmdg-aircraft-738" → NG3 mode
   - Contains "PMDG 777" / "pmdg-aircraft-77er/77w/77f/77l" → 777X mode
   - Otherwise → standard-only
4. On PMDG match:
   a. SimConnect_MapClientDataNameToID(PMDG_NG3_DATA_NAME, ID)
   b. SimConnect_AddToClientDataDefinition(DEFINITION_ID, 0, sizeof(struct), 0, 0)
   c. SimConnect_RequestClientData(... PERIOD_ON_SET, FLAG_CHANGED ...)
5. Subscribe to "AircraftLoaded" changes (SubscribeToSystemEvent "SimStart")
   → on aircraft change: cleanup + re-detect
```

---

## 4. Enabling the SDK (user workflow)

PMDG does NOT send ClientData by default. The pilot must add a line to the aircraft options file:

**For the 737:**
```ini
# E:\MSFS24_Community\Community\pmdg-aircraft-738\work\737NG3_Options.ini
[SDK]
EnableDataBroadcast=1
```

**For the 777:**
```ini
# E:\MSFS24_Community\Community\pmdg-aircraft-77er\work\777X_Options.ini
[SDK]
EnableDataBroadcast=1
```

**UI integration:**
- AeroACARS detects PMDG loaded + ClientData isn't coming (subscription reports no data within 5 s)
- → Settings tab shows an orange notice: "PMDG SDK not enabled. Click here for instructions."
- Click opens a modal with step-by-step instructions + path to the file (auto-detect)
- Optional: "Open file automatically" button (opens the `.ini` file in Notepad)

---

## 5. Implementation steps

Sorted by risk and dependency:

### Phase 5.1 — Rust replication of the headers (low risk)

**Goal:** Replicate both headers in Rust with `#[repr(C)]` for correct memory layout.

**What:**
- New module `crates/sim-msfs/src/pmdg/mod.rs`
- Sub-modules: `pmdg/ng3.rs` and `pmdg/x777.rs`
- Each module defines its `Pmdg{Variant}Data` struct
- Helper function: `pub fn from_bytes(bytes: &[u8]) -> Option<Self>`

**Field mapping:**
| C++ | Rust |
|---|---|
| `bool` | `u8` (C++ bool is 1 byte) |
| `char` | `u8` |
| `unsigned char` | `u8` |
| `short` | `i16` |
| `unsigned short` | `u16` |
| `int` | `i32` |
| `unsigned int` | `u32` |
| `float` | `f32` |
| `char[N]` | `[u8; N]` |

**Critical:** uses `#[repr(C)]` — not `packed`. This matches the MSVC layout (standard for the PMDG build).

**Tests:**
- `assert_eq!(std::mem::size_of::<PmdgNg3Data>(), 7000)` (= sizeof from the header)
- Snapshot test with hexed bytes from a real capture

### Phase 5.2 — SimConnect ClientData subscription (medium risk)

**Goal:** Extend the sim-msfs adapter to optionally run a second ClientData subscription for PMDG data.

**What:**
- New field in `Shared`: `pmdg_data: Mutex<Option<PmdgSnapshot>>` (PmdgSnapshot is an enum with `Ng3(...)` and `X777(...)` variants)
- New connection method: `Connection::register_pmdg_clientdata(variant: PmdgVariant)` — maps name + definition + request
- In `run_dispatch`: extra branch for `SIMCONNECT_RECV_ID_CLIENT_DATA` with the PMDG request ID
- On PMDG data: bytes → struct → into `shared.pmdg_data`

**Edge cases:**
- SimConnect exception when ClientData name isn't registered (= SDK not enabled) → we get `EXCEPTION_DATA_ERROR`
- → User-friendly: set state to `PmdgSdkState::SdkNotEnabled`, UI shows hint

### Phase 5.3 — Aircraft auto-detection (low risk)

**Goal:** Automatically detect PMDG on aircraft change + adjust the subscription.

**What:**
- `SimConnect_RequestSystemState("AircraftLoaded")` right after connection open
- Plus `SubscribeToSystemEvent("SimStart")` for live changes
- Pattern match on the `.air` path:
  ```rust
  fn detect_pmdg_variant(air_path: &str) -> Option<PmdgVariant> {
      let p = air_path.to_lowercase();
      if p.contains("pmdg-aircraft-737") || p.contains("pmdg 737") {
          Some(PmdgVariant::Ng3)
      } else if p.contains("pmdg-aircraft-77") || p.contains("pmdg 777") {
          Some(PmdgVariant::X777)
      } else {
          None
      }
  }
  ```
- On variant change: clean up old subscription, start new one

### Phase 5.4 — SimSnapshot integration (low risk)

**Goal:** Let PMDG data flow into `SimSnapshot`, without duplicating standard fields.

**What:**
- `SimSnapshot` gets a new field:
  ```rust
  pub pmdg: Option<PmdgSnapshot>,
  ```
- `PmdgSnapshot` is an enum with the variants `Ng3(Ng3Snapshot)` and `X777(X777Snapshot)`
- `Ng3Snapshot` / `X777Snapshot` are the "interesting" fields filtered + in readable form (e.g. `mcp_speed_kt`, `mcp_speed_is_mach`, `fma_speed_mode: FmaMode` etc.)
- Adapter code: `to_snapshot()` merges both standard telemetry and PMDG data

**Which fields go into the snapshot:**

| From PMDG_NG3_Data | Snapshot field | Use case |
|---|---|---|
| `MCP_IASMach`, `MCP_IASBlank` | `mcp_speed_kt: Option<f32>` | "MCP set to 250 kt" |
| `MCP_Heading` | `mcp_heading_deg: Option<u16>` | "MCP HDG: 280°" |
| `MCP_Altitude` | `mcp_altitude_ft: Option<u16>` | "MCP ALT: 28000 ft" |
| `MCP_VertSpeed`, `MCP_VertSpeedBlank` | `mcp_vs_fpm: Option<i16>` | "MCP V/S: -1500 fpm" |
| `MCP_annunVNAV/LNAV/LVL_CHG/HDG_SEL/...` | `fma_active_modes: FmaModes` | A/T+A/P FMA |
| `FMC_TakeoffFlaps` | `fmc_takeoff_flaps_deg: Option<u8>` | "Plan TO Flaps: 5°" |
| `FMC_LandingFlaps` | `fmc_landing_flaps_deg: Option<u8>` | "Plan LDG Flaps: 30°" |
| `FMC_V1/VR/V2/LandingVREF` | `fmc_v_speeds: VSpeeds` | PIREP custom field |
| `FMC_CruiseAlt` | `fmc_cruise_alt_ft: Option<u16>` | "FMC Cruise: FL280" |
| `FMC_DistanceToTOD` | `fmc_distance_to_tod_nm: Option<f32>` | TOD indicator |
| `FMC_DistanceToDest` | `fmc_distance_to_dest_nm: Option<f32>` | Cross-check |
| `MAIN_TEFlapsNeedle[0]` | `flaps_position_deg: Option<f32>` | Real flap degrees |
| `MAIN_AutobrakeSelector` | `autobrake: Option<AutobrakeSetting>` | "Autobrake MAX" |
| `AircraftModel` | `aircraft_subvariant: Option<u16>` | "B737-800 SSW" |
| `WeightInKg` | `weight_unit_kg: Option<bool>` | Correct unit |

**FmaMode** as enum:
```rust
pub enum FmaMode {
    Inactive,
    Vnav, Lnav,
    HdgSel, HdgHold,
    LvlChg, AltHold,
    VorLoc, App, Toga, Speed,
    N1, At, Cws,
    // ... etc, depending on which annun booleans are active
}
```

### Phase 5.5 — Activity log integration (medium risk)

**Goal:** Log PMDG events into the ACARS activity log, with dedup.

**Example log entries:**
- `MCP IAS → 230 kt` (when MCP_IASMach changes)
- `MCP HDG → 080°` (when MCP_Heading changes)
- `A/T armed` (when MCP_annunATArm: false→true)
- `A/P CMD A engaged` (when MCP_annunCMD_A: false→true)
- `FMA: VNAV PTH / SPEED / —` (when FMA modes change)
- `Autobrake → MAX` (when MAIN_AutobrakeSelector changes)
- `V1: 145 kt · VR: 148 kt · V2: 152 kt` (once at takeoff roll start)
- `TO Config Warning` (when MAIN_annunTAKEOFF_CONFIG: true)

**Dedup:** `last_logged_*` fields in FlightStats as we already have today. Trigger: "value has changed" + "stable for ≥1 tick." Prevents flicker log.

### Phase 5.6 — PIREP custom fields (low risk)

**What goes into the final PIREP:**

| Custom field | Source | Example |
|---|---|---|
| "V1 / VR / V2" | `fmc_v_speeds` (at takeoff time) | "V1 145 / VR 148 / V2 152" |
| "VREF" | `fmc_v_speeds.vref` (at landing time) | "138 kt" |
| "FMC Cruise Alt" | `fmc_cruise_alt_ft` (plan value) | "FL280" |
| "TO Flaps Plan" | `fmc_takeoff_flaps_deg` | "5°" |
| "TO Flaps Actual" | `flaps_position_deg` (from the takeoff sample) | "5°" |
| "LDG Flaps Plan" | `fmc_landing_flaps_deg` | "30°" |
| "LDG Flaps Actual" | `flaps_position_deg` (from the touchdown sample) | "30°" |
| "Autobrake at Land" | `autobrake` (at landing time) | "MAX" |
| "Aircraft Variant" | `aircraft_subvariant` (NG3) or path (777X) | "B737-800 SSW" |

### Phase 5.7 — UI: SDK status display (medium risk)

**Where:**
- Settings → Debug Panel: new section "PMDG SDK"
- Cockpit tab: small logo / indicator when the PMDG SDK is active

**States:**
- `Inactive` — no PMDG aircraft loaded
- `PmdgDetected` — PMDG detected, ClientData being subscribed
- `Active` — data is coming in (= SDK is enabled)
- `SdkNotEnabled` — PMDG detected, but no data → hint modal

**Modal instructions (DE+EN):**
> You're flying a PMDG 737/777, but AeroACARS isn't receiving extended cockpit data. Enable the PMDG SDK:
>
> 1. Close MSFS
> 2. Open this file: `<auto-detected path>\737NG3_Options.ini`
> 3. Add at the end:
>    ```
>    [SDK]
>    EnableDataBroadcast=1
>    ```
> 4. Save → restart MSFS → reload the flight
>
> [Open file in Notepad]   [Got it, later]

---

## 6. Edge cases

| Case | Behavior |
|---|---|
| PMDG not installed | Standard telemetry only, no UI hint |
| PMDG installed but different aircraft loaded | Standard telemetry only |
| PMDG loaded, SDK not enabled | UI shows hint modal, standard telemetry runs |
| Pilot switches mid-flight from PMDG to Asobo | Clean up PMDG subscription, no pmdg data in the snapshot |
| Pilot switches from Asobo to PMDG | Build up new PMDG subscription |
| PMDG update changes struct layout | We get wrong data → test with `assert size_of`. On mismatch: log warning + fallback to standard-only |
| SimConnect connection drops | Clean up both subscriptions, rebuild on reconnect |

---

## 7. Tests

**Unit tests:**
- `size_of::<PmdgNg3Data>()` matches expected (catch struct layout changes)
- `size_of::<Pmdg777XData>()` matches expected
- Bytes roundtrip: serialize known-state struct → parse back → equal
- `detect_pmdg_variant()` matches all known aircraft paths (PMDG 737, PMDG 777ER, 777W, 777F, 777L)
- FmaMode decoder: all combinations of annun booleans → correct FmaMode

**Integration tests (manual):**
- DEV build with MSFS + PMDG 737 + SDK enabled → MCP values in debug panel
- Change MCP value in the sim → activity log entry appears
- Aircraft change in the sim → state updates correctly
- SDK NOT enabled → hint modal appears after 5 s

---

## 8. Release strategy

**Staged release in the feature branch, no direct merge to main:**

1. **Phase 5.1 + 5.2 + 5.3** (Rust struct + subscription + detection) → DEV build → test with user
2. **Phase 5.4** (SimSnapshot integration) → DEV build → test
3. **Phase 5.5 + 5.6** (activity log + PIREP fields) → DEV build → test with real flight
4. **Phase 5.7** (UI) → final polish
5. **Merge to main** → tag **v0.2.0** → bilingual release notes:
   > AeroACARS v0.2.0 — Boeing Premium Telemetry
   > PMDG 737/777 SDK integration. Real FMA, MCP, V-speeds, FMC data — straight from the cockpit.

**No small hotfix releases in between.** If something in master doesn't work, it's patched separately.

---

## 9. What we are NOT doing today

- ❌ Control channel (output back to PMDG) — we only READ, we don't send commands
- ❌ CDU display reading (`PMDG_NG3_CDU_0`) — interesting but a separate feature, probably phase H.5
- ❌ EFB data (777X) — separate feature
- ❌ Other PMDG aircraft (DC-6, etc.) — comes when requested
- ❌ FSUIPC or MobiFlight bridge — we only use the native PMDG SDK

---

## 10. License & legal

PMDG explicitly permits SDK usage ([forum confirmation](https://forum.pmdg.com/forum/main-forum/pmdg-737-for-msfs/general-discussion-no-support/234073)). The SDK header is delivered as an end-user resource (no NDA, no login required, in plain sight in the aircraft installation).

We replicate the structure in Rust for our own use — this is **fair use** and standard practice (see SPAD.neXt, MobiFlight, FSUIPC, all of which do the same).

In the README we mention:
> AeroACARS supports the PMDG SimConnect SDK for the 737 NG3 and 777X. PMDG is a registered trademark of Precision Manuals Development Group; AeroACARS is neither developed by nor officially endorsed by PMDG.

---

## 11. Open questions before implementation

1. **Verify struct padding** — MSVC vs Rust `#[repr(C)]` — should be identical, but: test once with a real capture (MSFS running, simple C++ tool dumps the first 64 bytes of the struct as hex, we compare).

2. **Volatile header updates** — when PMDG rolls an aircraft update, the header may change. Strategy: run the `assert size_of` test in every release + version pin in the README.

3. **Throttling** — `PERIOD_ON_SET + FLAG_CHANGED` means: only when data changes. But: during an active flight practically everything changes constantly. Do we need client-side rate limiting? Probably not — the ClientData channel is efficiently designed.

4. **Multiple PMDG aircraft** — if the pilot e.g. loads the 737, then the 777 — we must cleanly unsubscribe from the first definition + re-subscribe to the second. Test case.

---

**Ready to start with Phase 5.1.** Implementation in the feature branch, no main merge before user testing passes.
