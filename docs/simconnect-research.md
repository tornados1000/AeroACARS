# SimConnect Deep-Dive for CloudeAcars

**Date**: 2026-05-01
**Author**: Research pass for Phase H.4 (sim-msfs adapter)
**Status**: Reference document; not a binding ADR yet
**Scope**: Everything we need to make MSFS 2020 / 2024 telemetry reliable across study-level and stock aircraft, in pure Rust, from a Tauri sidecar process.

---

## TL;DR — the five things that matter

1. **Our current crate (`simconnect-sdk` 0.2.3 by mihai-dinculescu) is archived as of 2026-02-22.** We can keep using it short-term but every architectural decision from here forward should assume it's frozen. There is no "next version" coming.
   Source: [github.com/mihai-dinculescu/simconnect-sdk-rs](https://github.com/mihai-dinculescu/simconnect-sdk-rs).
2. **`ATC PARKING NAME` / `ATC PARKING NUMBER` / `ATC RUNWAY SELECTED` are unreliable in MSFS 2024** and break entire data definitions on some aircraft (Fenix A320 confirmed). The official 2024 path is the **Facilities API** (`SimConnect_AddToFacilityDefinition` + `SimConnect_RequestFacilityData`) — runways and parkings are children of the airport facility and have proper field accessors. FS9GPS variables (incl. `GPS APPROACH RUNWAY`) are explicitly deprecated in 2024. See [GPS Variables — MSFS 2024 SDK](https://docs.flightsimulator.com/msfs2024/html/6_Programming_APIs/GPSVars/GPS_Variables.htm).
3. **The exception-7-kills-everything behaviour is a SimConnect protocol property**, not a bug in our crate: any unrecognised name in a `DataDefinition` invalidates the whole definition until you build a new one. Mitigation = split the subscribe into multiple definitions ("core", "lights", "FBW", "Fenix") so a single bad name only takes out one bucket.
4. **For Fenix specifically**, the `S_*` LVars are switch positions (state) and the `I_*` LVars are indicator lamps (latched). Our flickering AP2 is most likely a non-bug: Fenix Block-2 doesn't drive `I_FCU_AP*` exactly as we'd want for "AP engaged" — community consensus is that some Fenix LVars are deliberately not exposed for AP status, and we should derive AP state from `L:S_FMA_AP_STATUS` / FCU display LVars or fall back to standard `AUTOPILOT MASTER` for Fenix. See [Fenix LVars community thread](https://forums.flightsimulator.com/t/fenix-a320-lvars-and-honeycomb-bravo-ap-functionality/520815).
5. **The realistic long-term answer is a thin MobiFlight WASM bridge or our own WASM module** for LVars on study-level aircraft — not "subscribe to 200 LVars in a derive macro". Plain SimConnect was never designed for that workload; even MobiFlight has had to fight SimVar-flood stutters ([MobiFlight #9](https://github.com/MobiFlight/MobiFlight-WASM-Module/issues/9)).

---

## A. What SimConnect can actually do (officially)

### A.1 Data channels

SimConnect 2024 exposes five distinct channels. We currently only use one of them.

| Channel | API entry points | What it gives | Use for |
|---|---|---|---|
| **SimObject Data** | `SimConnect_AddToDataDefinition`, `RequestDataOnSimObject(_EX1)` | A-vars (standard SimVars) and L-vars on the user aircraft | What we do today: position, lights, AP, fuel |
| **Facility Data** | `SimConnect_AddToFacilityDefinition`, `RequestFacilityData(_EX1)`, `SubscribeToFacilities(_EX1)` | Airports, runways, parkings, approaches, jetways, frequencies — fully structured | Gate capture, runway-selected, taxi-flow validation |
| **Events** | `MapClientEventToSimEvent`, `TransmitClientEvent`, `SubscribeToSystemEvent` | Push events: pause, sim start, AI events, key events | Pause/slew detection, sim-start/quit, eventually injection |
| **ClientData** | `MapClientDataNameToID`, `AddToClientDataDefinition`, `RequestClientData`, `SetClientData` | Shared-memory areas owned by other addons | PMDG SDK, MobiFlight WASM, our own WASM |
| **Input/Output Events** | `EnumerateInputEvents`, `SetInputEvent`, etc. (2024 only) | Hardware-style events used by some MSFS 2024 systems | Future; 2024-exclusive |

**Sources**:
- [SimConnect SDK landing page (2024)](https://docs.flightsimulator.com/msfs2024/html/6_Programming_APIs/SimConnect/SimConnect_SDK.htm)
- [SimConnect API Reference (2020)](https://docs.flightsimulator.com/html/Programming_Tools/SimConnect/SimConnect_API_Reference.htm)
- [SimConnect_AddToFacilityDefinition](https://docs.flightsimulator.com/html/Programming_Tools/SimConnect/API_Reference/Facilities/SimConnect_AddToFacilityDefinition.htm)

**Recommendation**: We're leaving four channels unused. The high-value ones for ACARS are **Facility Data** (gate/runway, replaces the failing `ATC PARKING NAME`) and **System Events** (pause/quit detection that we currently fake with a 5 s stale-timeout). ClientData unlocks PMDG and a future MobiFlight bridge.

---

### A.2 Documented limits

The SDK does not publish a hard "max fields per DataDefinition" number, but there are several real ceilings worth knowing.

| Limit | Value | Source |
|---|---|---|
| String types | `STRING8`, `STRING32`, `STRING64`, `STRING128`, `STRING256`, `STRING260`, `STRINGV` (variable up to 256) | [SIMCONNECT_DATATYPE](https://docs.flightsimulator.com/html/Programming_Tools/SimConnect/API_Reference/Structures_And_Enumerations/SIMCONNECT_DATATYPE.htm) |
| Maximum size of a single client data area | 64 KB (documented for `CreateClientData`) | [SimConnect SDK](https://docs.flightsimulator.com/msfs2024/html/6_Programming_APIs/SimConnect/SimConnect_SDK.htm) |
| Network message buffer | 1 MB nominal; large data definitions get fragmented | [Programming SimConnect Clients (Managed Code)](https://docs.flightsimulator.com/msfs2024/html/6_Programming_APIs/SimConnect/Programming_SimConnect_Clients_Using_Managed_Code.htm) |
| Single-name SimVar lookup failure | **Invalidates the entire DataDefinition** with `SIMCONNECT_EXCEPTION_NAME_UNRECOGNIZED` (=7) — *all* fields stop streaming until you redefine | Behaviour seen in the wild; documented implicitly in [AddToDataDefinition](https://docs.flightsimulator.com/html/Programming_Tools/SimConnect/API_Reference/Events_And_Data/SimConnect_AddToDataDefinition.htm) |
| MSFS 2024 ICAO field size | Increased to 8 bytes for helipads (`SIMCONNECT_ICAO`, `SIMCONNECT_DATA_FACILITY_AIRPORT`) | [SIMCONNECT_DATA_FACILITY_AIRPORT](https://docs.flightsimulator.com/msfs2024/html/6_Programming_APIs/SimConnect/API_Reference/Structures_And_Enumerations/SIMCONNECT_DATA_FACILITY_AIRPORT.htm) |

There's an FSDeveloper thread noting in passing that 50+ field structs work fine ([FSDeveloper thread](https://www.fsdeveloper.com/forum/threads/simvars-quantity.450181/)), so our ~50 fields is *not* the problem — the all-or-nothing failure mode is.

**Recommendation**: Treat every DataDefinition as a "transaction" — if any field is risky (vendor-specific, version-specific, MSFS-2024-only), it goes in its own definition so a failure is local. Today everything is in one struct → one bad name = silent stream.

---

### A.3 Which SimVars actually work in MSFS 2024

Verified-working core variables (cross-checked against the [Aircraft RadioNavigation Variables](https://docs.flightsimulator.com/html/Programming_Tools/SimVars/Aircraft_SimVars/Aircraft_RadioNavigation_Variables.htm) and [Simulation Variables](https://docs.flightsimulator.com/msfs2024/html/6_Programming_APIs/SimVars/Simulation_Variables.htm) pages):

- **Position**: `PLANE LATITUDE`, `PLANE LONGITUDE`, `PLANE ALTITUDE`, `PLANE ALT ABOVE GROUND` ✅
- **Attitude**: `PLANE PITCH/BANK/HEADING DEGREES (TRUE|MAGNETIC)` ✅
- **Speeds**: `GROUND VELOCITY`, `AIRSPEED INDICATED/TRUE`, `VERTICAL SPEED` ✅
- **Engines**: `GENERAL ENG COMBUSTION:n`, `ENG FUEL FLOW PPH:n` ✅
- **Lights**: `LIGHT LANDING/BEACON/STROBE/TAXI/NAV/LOGO` ✅
- **Autopilot**: `AUTOPILOT MASTER/HEADING LOCK/ALTITUDE LOCK/NAV1 LOCK/APPROACH HOLD` ✅
- **Avionics**: `TRANSPONDER CODE:1` (BCO16), `COM ACTIVE FREQUENCY:n`, `NAV ACTIVE FREQUENCY:n` ✅
- **Environment**: `AMBIENT WIND DIRECTION/VELOCITY`, `KOHLSMAN SETTING MB`, `AMBIENT TEMPERATURE` ✅

**Confirmed broken / unreliable in MSFS 2024**:

- `ATC PARKING NAME`, `ATC PARKING NUMBER`, `ATC RUNWAY SELECTED` — exception 7 on Fenix; behavioural regressions reported across 2024 SU2-SU4 ([SU2 beta thread](https://devsupport.flightsimulator.com/t/su2-beta-partial-regression-of-ai-aircraft-parking-when-injected-via-simconnect/13704), [registration not reported to ATC ID](https://devsupport.flightsimulator.com/t/registration-is-not-properly-reported-to-simconnect-atc-id/17335)).
- `ATC ID` — MSFS 2024 sometimes reports the title instead of the registration depending on aircraft config.
- `GPS APPROACH RUNWAY`, `GPS WP NEXT ID`, all **FS9GPS** variables — explicitly deprecated for 2024; "they will still work as they did in MSFS 2020 but all support is discontinued" ([GPS Variables](https://docs.flightsimulator.com/msfs2024/html/6_Programming_APIs/GPSVars/GPS_Variables.htm)).
- Multiplayer-traffic SimVars in SU4 — `RequestDataOnSimObject` returns 0 for many fields ([SU4 release notes](https://www.flightsimulator.com/sim-update-4-msfs-2024/)).
- `SimConnect_RequestFacilitiesList` was broken from SU3 until SU4, then fixed ([devsupport thread](https://devsupport.flightsimulator.com/t/simconnect-requestfaciliteslist-is-not-working-after-su3-update/15394)). This is recent enough that we should expect more 2024-specific regressions.

**Recommendation**: Keep our current "core" SimVars — they're rock-solid. Move all gate/runway lookups out of the SimObject channel and onto Facilities API.

---

### A.4 How to read the approach/landing runway

`ATC RUNWAY SELECTED` is unreliable. The robust 2024 pattern is:

1. Get the destination airport ICAO from the active flight plan (`GPS WP NEXT ID` until it's gone, or — better — from our own user-input OFP).
2. Call `SimConnect_AddToFacilityDefinition` with `OPEN AIRPORT`, `LATITUDE`, `LONGITUDE`, `MAGVAR`, `N_RUNWAYS`, `OPEN RUNWAY`, `PRIMARY_NUMBER`, `PRIMARY_DESIGNATOR`, `SECONDARY_NUMBER`, `SECONDARY_DESIGNATOR`, `HEADING`, `LATITUDE`, `LONGITUDE`, `LENGTH`, `WIDTH`, `CLOSE RUNWAY`, `CLOSE AIRPORT`.
3. Call `SimConnect_RequestFacilityData(DefineID, RequestID, "EDDF", "")`.
4. Receive back the airport plus 1 message *per physical runway* (each contains both ends).
5. At touchdown, find the runway whose lat/lon/heading best matches our touchdown point — that's the approach runway. Independent of ATC.

Source: [FSDeveloper thread with working RUNWAY definition](https://www.fsdeveloper.com/forum/threads/issue-requesting-runway-data-using-facility-methods.457501/).

**Recommendation**: This becomes our **Touchdown Runway Resolver** module. The OFP already gives us the destination ICAO; we cache the runway list once on flight start and once on diversion. No more ATC dependency.

---

### A.5 How to read the current parking stand

Same pattern — parkings are children of `AIRPORT` in the Facilities API. The fields that work are:

- `NAME` — the designator type (`PARKING`, `GATE`, `DOCK`, `RAMP_GA`, `GATE_SMALL`, `GATE_MEDIUM`, `GATE_HEAVY`, etc.)
- `SUFFIX` — letter component (`GATE_A` through `GATE_Z`, plus directional values)
- `NUMBER` — the numeric part (e.g. 12 for "GATE A12")
- `TYPE` — same enum as NAME
- `HEADING` (degrees true)
- `RADIUS` (meters)
- `LATITUDE`, `LONGITUDE`

To get "GATE A12" you assemble `NAME + SUFFIX + NUMBER` client-side. Source: [SimConnect_AddToFacilityDefinition](https://docs.flightsimulator.com/html/Programming_Tools/SimConnect/API_Reference/Facilities/SimConnect_AddToFacilityDefinition.htm).

**Detection algorithm we should implement**:

1. On engine-start at the departure airport, request all `TAXI_PARKING` records for the departure ICAO.
2. Find the parking whose lat/lon is within `RADIUS` meters of our position.
3. That's the block-out gate. Persist into the PIREP.
4. On engine-shutdown at destination, repeat for the arrival ICAO.

This is also how SimBrief, BeyondATC, and FSAcars do it — there's no "current gate" SimVar in 2024 worth using. The new field `N_TAXI_PARKINGS` plus `OPEN TAXI_PARKING` block is the supported path.

**Recommendation**: Mid-term, build `gate_resolver` as a sub-module. Short-term, just leave `parking_name`/`parking_number` as `None` in the snapshot and surface from the user-input OFP. The PIREP doesn't need it for v1.

---

### A.6 MSFS 2020 vs 2024 SimVar differences

There is no consolidated diff. Practical observations from forum / devsupport scanning:

| Area | 2020 | 2024 |
|---|---|---|
| ATC parking SimVars | Worked on most aircraft | Unreliable; exception 7 on Fenix and other study-level ATC name handlers |
| `SubscribeToFacilities(_EX1)` | Works | Broke in SU3, fixed in SU4 |
| FS9GPS vars (`GPS APPROACH *`) | Supported | Deprecated; "no support, may break" |
| Helipad ICAO size | 5 char | 8 char (struct change) |
| `AIRCRAFT AGL`, `AIRCRAFT ALTITUDE ABOVE OBSTACLES` | — | New in 2024 |
| Multiplayer SimVar reads | Worked | Many return 0 in SU4 |

The SimConnect DLL itself is **2020+2024-compatible** as of SU2/SU3 ([devsupport](https://devsupport.flightsimulator.com/t/2020-2024-compatible-microsoft-flightsimulator-simconnect-dll/11844)) — same client can connect to either sim. We don't need separate adapters; we need defensive subscribes.

**Recommendation**: We tag `SimKind` correctly already. Add a sim-kind feature gate inside the adapter so 2024-only fields go in their own DataDefinition that we only register on 2024.

---

## B. Crate options for Rust

### B.1 simconnect-sdk (mihai-dinculescu/simconnect-sdk-rs) — what we use today

- **Status**: **Archived 2026-02-22.** Last release 0.2.3. 13 GitHub stars, 10 forks, 8 releases over its lifetime. No further development. Source: [GitHub repo](https://github.com/mihai-dinculescu/simconnect-sdk-rs).
- **Strengths**: Excellent ergonomics — the `#[derive(SimConnectObject)]` macro is genuinely the nicest Rust-side SimConnect API in existence. Stable for 2020 + 2024 over standard SimVars.
- **Limits we hit**:
  - Macro accepts only `f64`, `bool`, `String`. No `u32` for BCD squawks.
  - One `SimConnectObject` = one `DataDefinition` = single point of failure.
  - `register_object<T>()` registers the *whole* struct at construction time; you cannot dynamically add fields per-aircraft profile without either re-registering (expensive — drops the connection) or maintaining multiple structs.
  - No first-class **Facility Data** support. The crate exposes `Notification::Object` but not `Notification::FacilityData`.
  - No **ClientData** API surface — you can't talk to PMDG or MobiFlight with this crate. (You'd have to drop to the underlying `simconnect-sys` and call FFI manually.)
  - Strings come back as `String` but the FFI uses fixed-length `STRING260` blocks; 50+ fields with multiple strings push the per-message buffer up.
- **Verdict**: Fine for "Phase 1" of an ACARS, becoming a liability now. We will run into the wall on ClientData (PMDG) within the next 1-2 sprints.

### B.2 msfs-rs (FlyByWire) — the heavy-duty alternative

- **Status**: Actively maintained (drives the FBW A32NX and A380X). Source: [github.com/flybywiresim/msfs-rs](https://github.com/flybywiresim/msfs-rs), [docs](https://flybywiresim.github.io/msfs-rs/msfs/sim_connect/struct.SimConnect.html).
- **Two-sided design**: You can build either an **in-sim WASM module** (gauge logic, runs inside MSFS) **or** an **external SimConnect client** (Tauri sidecar, runs alongside MSFS). The README explicitly mentions both ([README](https://github.com/flybywiresim/msfs-rs/blob/main/README.md)).
- **Coverage**: Wraps the full C SimConnect API — `SimConnect_AddToDataDefinition`, `SimConnect_AddToFacilityDefinition`, `SimConnect_RequestFacilityData`, `MapClientDataNameToID`, `RequestClientData`, system events. Has the `data_definition!`/`client_data_definition!` macros for static structs but you can also build definitions imperatively at runtime.
- **Pros**:
  - **Facilities API and ClientData are first-class** — this is the only Rust crate where reading PMDG ClientData is a `register_struct` call, not a pile of `unsafe`.
  - Used in production by FBW so corner cases (re-connect, sim restart) are battle-tested.
  - Supports MSFS 2024 today; the FBW devs land patches when SUx changes break things.
- **Cons**:
  - Bigger API surface, less ergonomic — closer to the C bindings, fewer guard rails. We'd lose some of the "just derive a struct" magic.
  - Build setup: needs the MSFS SDK headers (we already vendor them; no new dependency for end users).
  - Not on crates.io as a polished release — referenced via git dependency.
- **Verdict**: This is the strategic choice. A migration is non-trivial (~1 day for a like-for-like Telemetry struct, plus the time to wire ClientData and Facilities), but it removes the archived-crate risk and unlocks PMDG, MobiFlight, and dynamic per-profile subscribes.

### B.3 Other Rust crates surveyed

| Crate | State | Notes |
|---|---|---|
| `simconnect` (Sequal32/simconnect-rust) | Minimally maintained, 35 commits, no releases, ~4 open issues, predates 2024 | The maintainer wrote: "I have not tested every single function from the api". Was the inspiration for `simconnect-sdk-rs`. Not a real candidate. ([repo](https://github.com/Sequal32/simconnect-rust)) |
| `simconnect-sys` | Raw FFI bindings; static; no high-level wrapper | Useful as an *escape hatch* under a wrapper, not as a primary API |
| `jcramb/simconnect-rs` | Personal fork; minimal activity | Pass |
| `sim_connect_sys` | Old experimental | Pass |

### B.4 Direct FFI / bindgen

Effort: ~1 day to get a `bindgen`-driven `simconnect-sys` working against the bundled MSFS 2024 SDK. Then we'd be writing Rust C-FFI glue ourselves. Realistic costs:

- Every `unsafe` callsite is a maintenance burden.
- We re-implement what `msfs-rs` already does, badly.
- We lose nothing by depending on `msfs-rs` for the wrapper layer.

**Verdict**: Don't. Migrating to msfs-rs is strictly less work and gives us a tested baseline.

### B.5 Multi-profile subscription strategy

Three options, each with tradeoffs. Given the exception-7 failure mode, the answer is clearly (b) or (c):

- **(a) One mega-struct** (today). Single point of failure. Wastes bandwidth on inactive LVars (Fenix-only fields polled when FBW is loaded, etc.). Doesn't scale to 5+ profiles.
- **(b) One DataDefinition per profile bucket** ("core", "fbw", "fenix", "pmdg", "inibuilds"). Register only the buckets the detected profile needs; tear down + re-register when the user changes aircraft. This is what FSAcars-style products do. Failure in one bucket only kills that bucket. **This is the right answer for the next iteration.**
- **(c) Fully dynamic — register a definition per LVar.** Maximum isolation but high message overhead. Overkill.

**Recommendation**: Plan for **(b)**. Implementation: one persistent "core" DataDefinition (~15 SimVars, never changes), plus a profile-specific DataDefinition that's torn down and rebuilt whenever `AircraftProfile::detect` returns a new value.

---

## C. LVars for study-level aircraft

### C.1 FlyByWire A32NX

Source of truth: [github.com/flybywiresim/aircraft/blob/master/fbw-a32nx/docs/a320-simvars.md](https://github.com/flybywiresim/aircraft/blob/master/fbw-a32nx/docs/a320-simvars.md). FBW's docs site mirrors it: [docs.flybywiresim.com](https://docs.flybywiresim.com/aircraft/a32nx/a32nx-api/a32nx-systems-api/).

What we have today is mostly correct. Cross-check:

| Function | LVar (verified) | Unit |
|---|---|---|
| Transponder code (decimal) | `L:A32NX_TRANSPONDER_CODE` | Number |
| AP master engaged | `L:A32NX_AUTOPILOT_ACTIVE` | Bool |
| AP HDG hold | `L:A32NX_AUTOPILOT_HEADING_HOLD_MODE` | Bool |
| AP ALT hold | `L:A32NX_AUTOPILOT_ALTITUDE_HOLD_MODE` | Bool |
| AP LOC | `L:A32NX_AUTOPILOT_LOC_MODE_ACTIVE` | Bool |
| AP APPR | `L:A32NX_AUTOPILOT_APPR_MODE_ACTIVE` | Bool |
| Beacon | `L:LIGHTING_BEACON_0` | Number |
| Strobe | `L:LIGHTING_STROBE_0` | Number |
| Nav | `L:LIGHTING_NAV_0` | Number |
| Landing left/right | `L:LIGHTING_LANDING_2` / `L:LIGHTING_LANDING_3` | Number |
| Nose taxi/T.O. | `L:A32NX_OVHD_INTLT_NOSE_POSITION` | Number (0/1/2) |
| Engine N1 | `L:A32NX_ENGINE_N1:1` / `:2` | % N1 |
| Fuel flow | `L:A32NX_ENGINE_FF:1` / `:2` | kg/h (already kg!) |
| Flaps handle | `L:A32NX_FLAPS_HANDLE_INDEX` (0..4) or `L:A32NX_FLAPS_HANDLE_PERCENT` | Number / Percent |
| Landing-light animation | `A32NX_LANDING_{ID}_POSITION` | Percent |

**Fix**: We're reading flaps via the standard `FLAPS HANDLE PERCENT`. FBW publishes a real lever index (`L:A32NX_FLAPS_HANDLE_INDEX`) which is more accurate for "current detent". Add it.

**Win**: FBW already returns fuel flow in **kg/h** via `L:A32NX_ENGINE_FF:n` — we currently sum `ENG FUEL FLOW PPH` and divide by `LB_TO_KG`. Switching to the LVar saves a unit conversion and matches what the cockpit displays.

### C.2 Fenix A320

Sources:
- Cockpit_Behavior.xml shipped with the aircraft (definitive but local).
- [Fenix support: how to use LVars](https://kb.fenixsim.com/example-of-how-to-use-lvars).
- [D1ngtalk's YourControls config](https://github.com/D1ngtalk/Yourcontrols-config-for-Fenixsim-A320/blob/main/Fenix%20Simulations%20-%20Airbus%20A320.yaml) (community-maintained map).
- [Fragtality/FenixQuartz](https://github.com/Fragtality/FenixQuartz) (reads Quartz display LVars; good ground-truth for "what works").
- [PilotsDeck Fenix profile](https://flightsim.to/file/34345/pilotsdeck-streamdeck-profile-for-fenix-a320/298636).

**Naming convention** (this is the core insight):
- `S_*` = **switch** position (what the pilot has set on the panel). Persists. Read this for switches/knobs.
- `I_*` = **indicator** lamp / latched state (what the aircraft logic says is currently active). Read this for lit-or-not status.
- For overhead-panel lighting and most knobs, `S_*` is what we want.
- For autopilot engagement on Fenix, `I_FCU_AP1` *should* latch on AP engage, but **community reports indicate Fenix deliberately does not expose AP engagement state cleanly through these LVars** ([Honeycomb Bravo thread](https://forums.flightsimulator.com/t/fenix-a320-lvars-and-honeycomb-bravo-ap-functionality/520815)). User quote: *"some of Fenix variables like AP values...are designed to be inaccessible."* This explains our phantom-toggle observation: I_FCU_AP1/2 reflect lamp-test sequences, not AP master state.

**Verified-reliable Fenix LVars** (from YourControls + FenixQuartz cross-reference):

| Function | LVar | Notes |
|---|---|---|
| Beacon | `L:S_OH_EXT_LT_BEACON` | 0=off, 1=on |
| Strobe | `L:S_OH_EXT_LT_STROBE` | 0=off, 1=auto, 2=on |
| Wing | `L:S_OH_EXT_LT_WING` | 0/1 |
| Nav+Logo combined | `L:S_OH_EXT_LT_NAV_LOGO` | 0=off, 1=nav, 2=nav+logo |
| Runway turnoff | `L:S_OH_EXT_LT_RWY_TURNOFF` | 0/1 |
| Landing L/R | `L:S_OH_EXT_LT_LANDING_L` / `_R` | 0=retracted, 1=off, 2=on |
| Nose | `L:S_OH_EXT_LT_NOSE` | 0=off, 1=taxi, 2=T.O. |
| Parking brake | `L:S_MIP_PARKING_BRAKE` | 0/1 |
| Flaps lever | `L:S_FC_FLAPS` | 0..5 (UP, 1, 1+F, 2, 3, FULL) |

**Unreliable for our purposes**:
- `L:I_FCU_AP1`, `L:I_FCU_AP2`, `L:I_FCU_LOC`, `L:I_FCU_APPR` — flicker with unrelated cockpit input on Block-2.
- `L:S_FCU_AP1`, `L:S_FCU_AP2` — these are *button-press* state, pulse 0→1→0 on every press. Spam city. Not what we want.

**The right Fenix AP source**: For AP engagement, fall back to **standard `AUTOPILOT MASTER` SimVar** for Fenix. Fenix's underlying systems do drive the standard SimVars — FenixQuartz's display reading proves this. Our current code already takes this defensive route (`(None, None, None, None, None)` for Fenix AP) but a better fix is to actually use the standard SimVar there.

For squawk on Fenix: there's no clean decimal LVar in the public set. Use `TRANSPONDER CODE:1` (BCO16 SimVar) — it works on Fenix because Fenix wires the standard transponder. Filter rapid changes (>1 change per 5 seconds on the ground) to suppress keypad-edit noise.

### C.3 PMDG 737/777 — ClientData, not LVars

PMDG aircraft do **not** publish LVars for SimConnect clients. Instead they expose a **ClientData area** named `PMDG_NG3_Data` (737), with companion `PMDG_NG3_Control` and `PMDG_NG3_CDU_n` for command and CDU pages.

References:
- [PMDG SDK header (SimInterface fork)](https://github.com/maciekish/SimInterface/blob/master/Windows/PMDGWrapper/PMDG_NGX_SDK.h) — example header showing the struct layout (NGX-era; NG3 is similar).
- [Sim Innovations wiki — PMDG 737NGX variables](https://siminnovations.com/wiki/index.php?title=PMDG_737NGX_variables) — full field map.
- [SPAD.next — Enable PMDG data access](https://docs.spadnext.com/getting-started/untitled/simulation-specifc-steps/msfs-enable-pmdg-data-access) — required user-facing config.
- [PMDG forum: PMDG_NG3_SDK.h location](https://forum.pmdg.com/forum/main-forum/general-discussion-news-and-announcements/180780-is-there-a-737-for-msfs-equivalent-of-pmdg_ng3_sdk-h) — SDK files ship at `Community\pmdg-aircraft-737\Documentation\SDK`.

**How to wire it**:

1. **User-side config**: PMDG ships with SDK access disabled by default. The user must edit `aircraft.cfg` or a `737_Options.ini` to set `EnableDataBroadcast=1` (exact setting depends on PMDG version). This is a hard prerequisite — without it, no client data flows.
2. `SimConnect_MapClientDataNameToID(hSimConnect, "PMDG_NG3_Data", DATA_AREA_ID_NG3_DATA)`.
3. `SimConnect_AddToClientDataDefinition(hSimConnect, DEF_ID_NG3_DATA, 0, sizeof(PMDG_NG3_Data), 0.0, 0)` — add the whole struct as one blob, or specific offsets per field.
4. `SimConnect_RequestClientData(hSimConnect, DATA_AREA_ID_NG3_DATA, REQ_ID, DEF_ID_NG3_DATA, SIMCONNECT_CLIENT_DATA_PERIOD_ON_SET, 0, 0, 0, 0)`.
5. Receive a `SIMCONNECT_RECV_CLIENT_DATA` and `memcpy` into the Rust mirror struct.
6. The SDK header lists every byte offset — follow it exactly (struct packing matters; use `#[repr(C)]`).

**Effort**: 1 day to wire the basic struct, plus we need a copy of the official `PMDG_NG3_SDK.h` to translate to Rust. We'd ship the Rust port (mechanical translation) under our crate, never the original header (PMDG license).

**Major blocker**: `simconnect-sdk` does not expose `MapClientDataNameToID`/`AddToClientDataDefinition` at all. PMDG support requires either dropping to `simconnect-sys` raw FFI **or** migrating to `msfs-rs`. This alone is a strong push toward msfs-rs.

### C.4 INIBuilds A320/A330/A350/A346 Pro

Source: [iniBuilds forum: Key LVars List - A350](https://forum.inibuilds.com/topic/25015-key-lvars-list-a350/) — points at an official PDF spreadsheet. INIBuilds also ships the LVar list at `inibuilds-aircraft-a350\Resources\Documentation`.

The community thread [LVars - Systems](https://forum.inibuilds.com/topic/23163-lvars/) is the entry point. INIBuilds appears to follow a similar `S_*` (state) / `I_*` (indicator) convention to Fenix but their list is shorter and cleaner. A few examples observed in third-party tools:

- `L:INI_LIGHTS_BEACON`
- `L:INI_LIGHTS_STROBE_*`
- `L:INI_AUTOPILOT_AP1_ACTIVE` / `_AP2_ACTIVE`
- `L:INI_FLAPS_LEVER`
- `L:INI_PARKING_BRAKE`

The PDF spreadsheet attached to the forum thread (`A350 LVARs-FEB2025.pdf`) has the complete list — we should download a copy and convert it to a header table for the adapter, same as we do for Fenix.

**Recommendation**: Keep INIBuilds for "Phase H.5" (after FBW + Fenix + PMDG are solid). The user-base overlap with our test VA (GSG) is small enough that it's not blocking.

---

## D. ATC / Gate / Runway — concrete recommendations

### D.1 Why `ATC PARKING NAME` throws exception 7 on Fenix

Best understanding: Fenix, like several other study-level aircraft, **overrides the ATC subsystem** with their own systems-driven implementation. The standard `ATC PARKING NAME` SimVar is implemented inside the default ATC module; when that module is disabled or replaced, the SimVar resolution fails and SimConnect raises `EXCEPTION_NAME_UNRECOGNIZED` (=7). Same root cause as the [registration not reported to ATC ID](https://devsupport.flightsimulator.com/t/registration-is-not-properly-reported-to-simconnect-atc-id/17335) bug.

Known-broken in MSFS 2024 with at least one study-level aircraft loaded:
- `ATC PARKING NAME`
- `ATC PARKING NUMBER`
- `ATC PARKING TYPE`
- `ATC RUNWAY SELECTED`
- `ATC RUNWAY AIRPORT NAME` (intermittent)
- `ATC ID` (returns title instead of registration on some aircraft)

Default Asobo aircraft typically work. Stock GA fleet works. We cannot rely on these for any aircraft we don't ourselves test.

### D.2 Alternative gate capture

Two layered strategies:

**Primary: Facilities API position-lookup** (described in §A.5). The math:

```
fn nearest_parking(my_lat, my_lon, parkings) -> Option<&Parking> {
    parkings.iter()
        .filter(|p| haversine(my_lat, my_lon, p.lat, p.lon) <= p.radius_m + 5.0)
        .min_by_key(|p| haversine(my_lat, my_lon, p.lat, p.lon) as i64)
}
```

The 5 m slack handles the case where the user's nose sits slightly outside the radius circle.

**Fallback: User OFP**. Our flight-creation flow already asks the user for departure/arrival ICAOs; we could trivially also ask for departure/arrival gate. For phpVMS PIREPs this is often acceptable — many real airlines do exactly this in their ACARS UIs.

**Recommendation**: Ship the user-OFP fallback first (1 hour of work, unblocks the PIREP). Build the Facilities-API resolver in parallel as the "auto" mode for Phase H.5.

### D.3 Approach runway

`ATC RUNWAY SELECTED` is unreliable. `GPS APPROACH RUNWAY` is FS9GPS and deprecated. `GPS WP NEXT ID` works in 2020 but support is discontinued in 2024.

The robust approach: **runway capture at touchdown** via Facilities API (§A.4). On the touchdown event:
1. Query the destination airport's runway list.
2. Match touchdown lat/lon + heading against runway centreline + heading.
3. Accept the match within ±5 ° of heading and within ±100 m of centreline.

This is independent of ATC, of the user's flight plan, of any avionics state. It works on every aircraft.

**Recommendation**: This is the implementation we want for the PIREP "landed runway" field. The ATC value, if present and valid, can serve as an early hint (display in UI) but the persisted PIREP value should be the geometry-derived one.

### D.4 `SIM ON RUNWAY` / similar

Asobo did not add a "currently on runway" SimVar. Best you get is `SIM ON GROUND` (we have this) plus computed heuristics. There's a community pattern using `SURFACE TYPE` (returns ASPHALT/CONCRETE/etc.) but that distinguishes runway from grass, not runway from taxiway.

**Recommendation**: Use the runway-list-from-Facilities + position-lookup pattern instead. It's what tools like LittleNavMap and BeyondATC do internally.

---

## E. MobiFlight WASM as a bridge

### E.1 How it works

The MobiFlight WASM module is an in-sim component (loaded as a SimConnect addon by MSFS). It exposes three **ClientData channels** that any SimConnect client can subscribe to:

- `MobiFlight.Command` — receive commands from clients
- `MobiFlight.LVars` — continuously broadcast LVar values
- `MobiFlight.Response` — return non-streaming replies

External clients register via `MF.Clients.Add.<MyName>` on the default command channel. The module then creates dedicated per-client channels (`MyName.LVars`, `MyName.Command`, `MyName.Response`) and confirms with `MF.Clients.Add.<MyName>.Finished`.

To subscribe to an LVar: send `MF.SimVars.Add.(A:GROUND ALTITUDE,Meters)` for numerics, or `MF.SimVars.AddString.(A:GPS WP NEXT ID,String)` for strings. Numerics are 4-byte floats laid out at incrementing offsets in the LVars channel; strings get 128-byte segments.

References:
- [MobiFlight WASM Module README](https://github.com/MobiFlight/MobiFlight-WASM-Module/blob/main/README.md)
- [MobiFlight docs: WASM module](https://docs.mobiflight.com/guides/wasm-module/)
- [MSFSPythonSimConnectMobiFlightExtension](https://github.com/Koseng/MSFSPythonSimConnectMobiFlightExtension) — a Python reference implementation we can crib the protocol off
- Known issue: [#9 micro-stuttering with too many LVars](https://github.com/MobiFlight/MobiFlight-WASM-Module/issues/9)

### E.2 Pros / cons vs direct SimConnect

| | Pros | Cons |
|---|---|---|
| **MobiFlight bridge** | Reads any LVar from any aircraft including arbitrary new ones. Handles A:vars and L:vars uniformly. Already-installed for many flightsim-hardware users. | Requires the user to install MobiFlight (or the standalone WASM module). Stuttering risk if we subscribe to too many LVars. Adds a runtime dependency we don't control. |
| **Direct SimConnect** | Zero user setup. Works against the stock SDK only. | Cannot read aircraft-private LVars on aircraft that don't expose them via standard SimVars. Fragile against MSFS updates. |

### E.3 Installation

User installs MobiFlight Connector (https://www.mobiflight.com/) which ships the WASM module. There's also a [standalone WASM module](https://github.com/MobiFlight/MobiFlight-WASM-Module/releases) that doesn't require the full Connector. Either drops files into the MSFS Community folder.

**This is not user-friendly enough for a "click install CloudeAcars and fly" flow.** Roughly 15-20 % of serious simmers have MobiFlight; the rest do not. We can't make it a hard requirement.

### E.4 Rust support

No first-party Rust crate exists. The protocol is plain SimConnect ClientData, so any crate that exposes ClientData (msfs-rs, or simconnect-sys raw) can implement it in ~200 lines. The Python extension above is ~600 lines of clear code that maps directly to Rust.

**Recommendation**: Treat MobiFlight as the **fallback path for power users**. Detect at startup whether the WASM module is present (probe `MobiFlight.Command` ClientData channel — if mapping succeeds and we get a `MF.Pong` reply within 2 s, it's there). If present, use it for LVars. If absent, fall back to per-profile direct LVar subscription as today. No user-visible step required — it just works better when MobiFlight is installed.

---

## F. Practical recommendations

### F.1 Immediate fixes (this week)

These can land without an architecture change. They're all in `client/src-tauri/crates/sim-msfs/src/lib.rs`.

1. **Split the DataDefinition into buckets**. Today everything is one struct; one bad name silences everything. Refactor into:
   - `CoreTelemetry` — position, attitude, speeds, on-ground, parking-brake, stall/overspeed, gear/flaps, eng combustion, fuel total + flow, environment, transponder, COM/NAV freqs, AP standard, lights standard. Always registered.
   - `FbwTelemetry` — all `L:A32NX_*` and `L:LIGHTING_*` fields. Registered only when profile detect = FbwA32nx.
   - `FenixTelemetry` — all `L:S_*` and `L:I_*` Fenix fields. Registered only when profile detect = FenixA320.

   `simconnect-sdk` lets us call `register_object::<T>()` per type independently; the limitation is that re-registering after disconnect requires a fresh `SimConnect::new`. Workable: detect profile *before* register, register the matching bundle. If the profile changes mid-flight, accept that we miss LVars until the next reconnect (or do a forced reconnect — 100 ms hiccup, acceptable).

2. **Stop trying to read ATC parking from SimConnect**. We already removed those fields. Document this in code comments (we have started — keep going). Surface gate from user OFP only for v1.

3. **Fix Fenix AP**: replace the current `(None, None, …)` defensive fallback with reading the standard `AUTOPILOT MASTER` etc. SimVars (which Fenix wires). Drops the phantom-toggle problem.

4. **Add `L:A32NX_FLAPS_HANDLE_INDEX`** to the FBW bucket — more accurate detent than `FLAPS HANDLE PERCENT`. Switch fuel flow to `L:A32NX_ENGINE_FF:n` (already kg/h, removes a unit conversion).

5. **Add system-event subscription** for `Pause`, `Sim`, `1sec`, `4sec`. Replaces our 5 s stale-timeout hack with a real "is the sim paused/quit?" signal. `simconnect-sdk` exposes this — `Notification::Event(...)`.

### F.2 Mid-term (this month)

6. **Migrate from `simconnect-sdk` to `msfs-rs`** for the adapter. Why now: the crate is archived (frozen), we need ClientData for PMDG, and `msfs-rs` is the only well-maintained Rust path that has it. Migration plan:
   - Add `msfs = { git = "https://github.com/flybywiresim/msfs-rs", branch = "main" }` alongside the existing crate.
   - Port `CoreTelemetry` first, behind a feature flag. Compare snapshots side-by-side for a session.
   - Port profile buckets. Drop the old crate.
   - Estimated ~1–2 days end to end including tests.

7. **Implement the Facilities-API resolver** module. Two responsibilities:
   - Cache the runway and parking lists for the departure and arrival airports at flight start.
   - Provide `runway_at(lat, lon, hdg)` and `parking_at(lat, lon)` lookups.
   This is what powers gate-out, gate-in, and landing-runway PIREP fields without ever asking ATC SimVars.

8. **Add MobiFlight detection**. Probe `MobiFlight.Command`, attach silently if present. Use it as the LVar source for any aircraft profile we haven't explicitly mapped (catch-all for INIBuilds, Aerosoft CRJ, Just Flight 146, etc.).

### F.3 Long-term (Phase H.5+)

9. **Ship our own WASM module** if MobiFlight adoption proves insufficient. Pattern is well-trodden (FSAcars, Volanta, SimToolkitPro all do this). We get full control over LVar polling rates, can read aircraft.cfg fields we currently can't, and can catch sim-state events in real time. Cost: now we have to maintain a WASM module; it ships in a Community-folder layout; users have to install it. This is the same UX hurdle as MobiFlight, but ours.

10. **PMDG SDK support** as a follow-up. Once we're on msfs-rs, ClientData reads are trivial. The hardest part is mechanically transcribing the PMDG_NG3_SDK.h header to Rust struct definitions, and shipping the docs that tell the user how to enable PMDG SDK access.

---

## G. Summary table — what each problem maps to

| Problem we hit | Root cause | Short-term fix | Long-term fix |
|---|---|---|---|
| `ATC PARKING NAME` exception 7 | Fenix overrides ATC subsystem; SimConnect can't resolve the name; failure poisons the whole DataDefinition | Remove from struct (done). Use user-input gate. | Facilities-API parking resolver (§A.5). |
| Fenix LVars partially work | `S_*` vs `I_*` semantics; some Fenix LVars are deliberately not exposed | Use `S_*` for switches, `S_MIP_PARKING_BRAKE` for park brake; standard `AUTOPILOT MASTER` for AP state | MobiFlight bridge or own WASM for any field we can't reach via SimConnect today. |
| `simconnect-sdk` macro accepts only f64/bool/String | Crate design choice; archived 2026-02-22 → won't change | Cast in `telemetry_to_snapshot` (current). | Migrate to `msfs-rs` which exposes the full type set. |
| `I_FCU_AP2` flickers on Fenix | I_* LVars are lamp-test outputs, not engagement state, on Fenix Block-2 | Stop reading I_FCU_*; defer to standard SimVars | Same. |
| Can't read PMDG | `simconnect-sdk` has no ClientData support | (None) | Migrate to `msfs-rs`, add ClientData-based PMDG reader. |
| Can't reliably detect approach runway | `ATC RUNWAY SELECTED` unreliable; FS9GPS deprecated | (None) | Facilities-API runway resolver + touchdown geometry match. |
| Single bad SimVar kills all telemetry | SimConnect protocol behaviour: exception 7 invalidates the whole definition | Split into per-profile DataDefinitions | Same, plus runtime-built definitions on msfs-rs. |

---

## H. References — full source list

### MSFS 2024 SDK
- [SimConnect SDK](https://docs.flightsimulator.com/msfs2024/html/6_Programming_APIs/SimConnect/SimConnect_SDK.htm)
- [SimConnect API Reference](https://docs.flightsimulator.com/html/Programming_Tools/SimConnect/SimConnect_API_Reference.htm)
- [Programming SimConnect Clients (Managed Code)](https://docs.flightsimulator.com/msfs2024/html/6_Programming_APIs/SimConnect/Programming_SimConnect_Clients_Using_Managed_Code.htm)
- [SIMCONNECT_DATATYPE](https://docs.flightsimulator.com/html/Programming_Tools/SimConnect/API_Reference/Structures_And_Enumerations/SIMCONNECT_DATATYPE.htm)
- [SimConnect_AddToDataDefinition](https://docs.flightsimulator.com/html/Programming_Tools/SimConnect/API_Reference/Events_And_Data/SimConnect_AddToDataDefinition.htm)
- [SimConnect_AddToFacilityDefinition](https://docs.flightsimulator.com/html/Programming_Tools/SimConnect/API_Reference/Facilities/SimConnect_AddToFacilityDefinition.htm)
- [SIMCONNECT_DATA_FACILITY_AIRPORT](https://docs.flightsimulator.com/msfs2024/html/6_Programming_APIs/SimConnect/API_Reference/Structures_And_Enumerations/SIMCONNECT_DATA_FACILITY_AIRPORT.htm)
- [GPS Variables (deprecated in 2024)](https://docs.flightsimulator.com/msfs2024/html/6_Programming_APIs/GPSVars/GPS_Variables.htm)
- [Aircraft RadioNavigation Variables](https://docs.flightsimulator.com/html/Programming_Tools/SimVars/Aircraft_SimVars/Aircraft_RadioNavigation_Variables.htm)
- [TaxiwayServiceStand Objects](https://docs.flightsimulator.com/msfs2024/html/2_DevMode/Scenery_Editor/Objects/TaxiwayServiceStand_Objects.htm)
- [TaxiwayParking Objects](https://docs.flightsimulator.com/msfs2024/html/2_DevMode/Scenery_Editor/Objects/TaxiwayParking_Objects.htm)

### Release notes / known issues (MSFS 2024)
- [Sim Update 3 release notes](https://www.flightsimulator.com/sim-update-3-msfs-2024/)
- [Sim Update 4 release notes](https://www.flightsimulator.com/sim-update-4-msfs-2024/)
- [Release Notes index](https://docs.flightsimulator.com/html/Introduction/Release_Notes.htm)
- [Registration not reported to ATC ID](https://devsupport.flightsimulator.com/t/registration-is-not-properly-reported-to-simconnect-atc-id/17335)
- [SU2 beta — AI parking regression](https://devsupport.flightsimulator.com/t/su2-beta-partial-regression-of-ai-aircraft-parking-when-injected-via-simconnect/13704)
- [SimConnect_RequestFacilitiesList broken in SU3](https://devsupport.flightsimulator.com/t/simconnect-requestfaciliteslist-is-not-working-after-su3-update/15394)
- [2020+2024 SimConnect.dll compatibility](https://devsupport.flightsimulator.com/t/2020-2024-compatible-microsoft-flightsimulator-simconnect-dll/11844)
- [Request for additions to Facilities API](https://devsupport.flightsimulator.com/t/request-for-additions-to-simconnect-facilities-api/14870)

### Rust crates
- [simconnect-sdk-rs (archived)](https://github.com/mihai-dinculescu/simconnect-sdk-rs) / [docs.rs](https://docs.rs/simconnect-sdk/latest/simconnect_sdk/) / [crates.io](https://crates.io/crates/simconnect-sdk)
- [msfs-rs (FlyByWire)](https://github.com/flybywiresim/msfs-rs) / [docs](https://flybywiresim.github.io/msfs-rs/msfs/)
- [Sequal32/simconnect-rust](https://github.com/Sequal32/simconnect-rust) (older)
- [Rust support thread on MSFS forums](https://forums.flightsimulator.com/t/rust-support/309267)

### FlyByWire A32NX
- [a320-simvars.md (master source)](https://github.com/flybywiresim/aircraft/blob/master/fbw-a32nx/docs/a320-simvars.md)
- [FBW Systems API docs](https://docs.flybywiresim.com/aircraft/a32nx/a32nx-api/a32nx-systems-api/)
- [FBW Flightdeck API docs](https://docs.flybywiresim.com/aircraft/a32nx/a32nx-api/a32nx-flightdeck-api/)

### Fenix A320
- [FenixSim KB — How to use LVars](https://kb.fenixsim.com/example-of-how-to-use-lvars)
- [FenixSim Support — switch binding example](https://support.fenixsim.com/hc/en-us/articles/12466468901135-Example-of-How-to-Bind-Switches-Knobs-and-Buttons-on-FenixSim-Aircraft-to-External-Hardware)
- [Fragtality/FenixQuartz](https://github.com/Fragtality/FenixQuartz)
- [D1ngtalk/Yourcontrols-config-for-Fenixsim-A320](https://github.com/D1ngtalk/Yourcontrols-config-for-Fenixsim-A320/blob/main/Fenix%20Simulations%20-%20Airbus%20A320.yaml)
- [PilotsDeck Fenix A320 profile](https://flightsim.to/file/34345/pilotsdeck-streamdeck-profile-for-fenix-a320/298636)
- [AAO RPN scripts for Fenix](https://flightsim.to/file/33912/fenix-a320-fcu-and-lighting-rpn-scripts)
- [Fenix LVars / Honeycomb Bravo thread](https://forums.flightsimulator.com/t/fenix-a320-lvars-and-honeycomb-bravo-ap-functionality/520815)
- [Fenix outputs for displays — MobiFlight forum](https://www.mobiflight.com/forum/topic/7167.html)

### PMDG
- [PMDG NGX SDK header (community fork)](https://github.com/maciekish/SimInterface/blob/master/Windows/PMDGWrapper/PMDG_NGX_SDK.h)
- [Sim Innovations wiki — PMDG 737NGX variables](https://siminnovations.com/wiki/index.php?title=PMDG_737NGX_variables)
- [SPAD.next — PMDG data access setup](https://docs.spadnext.com/getting-started/untitled/simulation-specifc-steps/msfs-enable-pmdg-data-access)
- [PMDG forum — NG3 SDK location](https://forum.pmdg.com/forum/main-forum/general-discussion-news-and-announcements/180780-is-there-a-737-for-msfs-equivalent-of-pmdg_ng3_sdk-h)
- [PMDG forum — SDK help](https://forum.pmdg.com/forum/main-forum/pmdg-737-for-msfs/general-discussion-no-support/366180-help-with-737-sdk)
- [PMDG forum — C# SimConnect example](https://forum.pmdg.com/forum/main-forum/pmdg-737-for-msfs/general-discussion-no-support/280900-working-c-simconnect-example-for-pmdg-sdk)
- [Aerosoft Variables and Conditions doc](https://forum.aerosoft.com/applications/core/interface/file/attachment.php?id=159013)
- [PMDG NGX CDU text retrieval (NG3)](https://www.fsdeveloper.com/forum/threads/issues-retrieving-cdu-text-from-pmdg-737-ng3-using-simconnect.455478/)

### iniBuilds
- [iniBuilds A350 product page](https://inibuilds.com/products/inibuilds-a350-airliner-msfs-2024)
- [iniBuilds forum — Key LVars list A350](https://forum.inibuilds.com/topic/25015-key-lvars-list-a350/)
- [iniBuilds forum — LVars systems](https://forum.inibuilds.com/topic/23163-lvars/)

### MobiFlight
- [MobiFlight WASM Module repo](https://github.com/MobiFlight/MobiFlight-WASM-Module)
- [MobiFlight WASM README](https://github.com/MobiFlight/MobiFlight-WASM-Module/blob/main/README.md)
- [MobiFlight docs — WASM module](https://docs.mobiflight.com/guides/wasm-module/)
- [MobiFlight issue #9 — micro-stuttering](https://github.com/MobiFlight/MobiFlight-WASM-Module/issues/9)
- [MSFS Python Mobiflight Extension (reference impl)](https://github.com/Koseng/MSFSPythonSimConnectMobiFlightExtension)
- [MSFS forum — MobiFlight events via SimConnect](https://forums.flightsimulator.com/t/are-mobiflight-extended-wasm-events-for-g1000-controls-accesible-via-simconnect-or-via-fsuipclient-c-dll/501272)

### Facilities API examples
- [FSDeveloper — runway data via Facility methods](https://www.fsdeveloper.com/forum/threads/issue-requesting-runway-data-using-facility-methods.457501/)
- [FSDeveloper — flight plan from SimConnect](https://www.fsdeveloper.com/forum/threads/how-to-get-the-flight-plan-with-simconnect.450509/)
- [FSDeveloper — detect current/nearest airport](https://www.fsdeveloper.com/forum/threads/detect-current-nearest-airport-with-simconnect.457264/)
- [FSDeveloper — SimConnect_AddToFacilityDefinition C# example](https://www.fsdeveloper.com/forum/threads/simconnect_addtofacilitydefinition-in-c.457096/)
- [Pomax/msfs-simconnect-api-wrapper](https://github.com/Pomax/msfs-simconnect-api-wrapper) (JS reference for Facilities)
- [FBW issue #9133 — Facility Data for OANS](https://github.com/flybywiresim/aircraft/issues/9133)
