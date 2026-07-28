# X-Plane — Aircraft-specific DataRef overrides (template)

**Purpose:** AeroACARS reads its X-Plane telemetry via standard `sim/...` DataRefs.
With **study-level add-ons** (Hot-Start Challenger 650, ToLiss, FlightFactor, PMDG …),
many cockpit/system functions run through the add-on's **own** DataRefs — the
standard DataRef then stays at `0` / empty. This is exactly how the GSG225 bug
came about: the CL650 doesn't populate `sim/flightmodel2/controls/flap_handle_deploy_ratio`
→ AeroACARS never "saw" the flaps → `LANDING CONFIG: INCOMPLETE`.

**This file is the template** for finding and recording the appropriate
add-on-specific DataRefs for a particular aircraft. Fill it in once per aircraft,
then AeroACARS can get a matching aircraft profile.

---

## How to fill in the template

1. **Install DataRefTool** (X-Plane plugin, free): copy the `DataRefTool` folder
   to `X-Plane/Resources/plugins/`.
2. Load the aircraft, park at a gate, **Plugins → DataRefTool → Show DataRefs**.
3. In the search field, enter the **add-on prefix** — usually the manufacturer namespace:
   - Hot-Start Challenger 650 → `CL650/`
   - ToLiss → `AirbusFBW/`
   - FlightFactor → `1-sim/` or `a350/` / `757/` …
   - PMDG (if X-Plane variant) → manufacturer-specific
4. For each row in the table below: **operate** the function in the sim
   (e.g. deploy flaps) and in DataRefTool check **which add-on DataRef
   moves along**. Enter the name + the observed value range.
5. Hand the filled-in table to the developers → aircraft profile.

> **Tip:** First observe the standard `sim/...` DataRef. If it moves along
> when you operate the function, **no override is needed** — then leave the
> "Aircraft-specific" field empty / mark it with "— (standard ok)."

---

## Part 1 — Physics / flight model: **NO override needed**

These values are always delivered correctly by the X-Plane flight model engine,
regardless of the add-on. Never search for anything here.

Position, altitude (MSL/AGL), heading, pitch/bank, vertical speed, groundspeed,
IAS/TAS, G-force, on-ground, gear normal force, weight (empty/total),
fuel quantity, wind, Mach, OAT, QNH — as well as the **body velocity**
(`sim/flightmodel/forces/local_vx` / `local_vz`), which AeroACARS uses for
sideslip and touchdown analysis. All flight-model values: every add-on drives
them correctly, never search for anything here.

---

## Part 2 — Cockpit / systems: **Override candidates**

These functions are frequently driven by study-level add-ons through their own
DataRefs. Check per aircraft and fill in as needed.

**Priority A — feeds into score / phase detection (search first!):**

| Function | Standard DataRef (AeroACARS today) | Type / expected value | Effect in AeroACARS | Aircraft-specific DataRef |
|---|---|---|---|---|
| Flap position | `sim/flightmodel2/controls/flap_handle_deploy_ratio` | float 0.0–1.0 | Approach stability "Landing Config" | `____________________` |
| Gear position | `sim/flightmodel2/gear/deploy_ratio[0]` | float 0.0–1.0 | Approach stability, phases | `____________________` |
| Engine 1 running | `sim/flightmodel/engine/ENGN_running[0]` | int 0/1 | Phase FSM (pushback/taxi/start) | `____________________` |
| Engine 2 running | `sim/flightmodel/engine/ENGN_running[1]` | int 0/1 | Phase FSM | `____________________` |
| Engine 3 running | `sim/flightmodel/engine/ENGN_running[2]` | int 0/1 | Phase FSM (3+ engines) | `____________________` |
| Engine 4 running | `sim/flightmodel/engine/ENGN_running[3]` | int 0/1 | Phase FSM (4 engines) | `____________________` |
| Parking brake | `sim/cockpit2/controls/parking_brake_ratio` | float 0.0–1.0 | Phase/block logic | `____________________` |

**Priority B — PIREP custom fields / display (cosmetic, search afterwards):**

| Function | Standard DataRef (AeroACARS today) | Type / expected value | Aircraft-specific DataRef |
|---|---|---|---|
| Speedbrake / spoiler | `sim/cockpit2/controls/speedbrake_ratio` | float 0.0–1.0 | `____________________` |
| Spoiler armed | `sim/cockpit2/annunciators/speedbrake` | int 0/1 | `____________________` |
| Autobrake level | `sim/cockpit2/switches/auto_brake_level` | int | `____________________` |
| Stall warning | `sim/cockpit2/annunciators/stall_warning` | int 0/1 | `____________________` |
| Landing lights | `sim/cockpit2/switches/landing_lights_on` | int 0/1 | `____________________` |
| Beacon | `sim/cockpit2/switches/beacon_on` | int 0/1 | `____________________` |
| Strobe | `sim/cockpit2/switches/strobe_lights_on` | int 0/1 | `____________________` |
| Taxi light | `sim/cockpit2/switches/taxi_light_on` | int 0/1 | `____________________` |
| Nav lights | `sim/cockpit2/switches/navigation_lights_on` | int 0/1 | `____________________` |
| Wing light | `laminar/B738/toggle_switch/wing_light_pos` *(already 737-specific)* | int | `____________________` |
| Wheel well light | `laminar/B738/toggle_switch/wheel_well_light_pos` *(already 737-specific)* | int | `____________________` |
| Autopilot master | `sim/cockpit2/autopilot/servos_on` | int 0/1 | `____________________` |
| AP heading mode | `sim/cockpit2/autopilot/heading_status` | int 0/1/2 | `____________________` |
| AP altitude mode | `sim/cockpit2/autopilot/altitude_hold_status` | int 0/1/2 | `____________________` |
| AP nav mode | `sim/cockpit2/autopilot/nav_status` | int 0/1/2 | `____________________` |
| AP approach mode | `sim/cockpit2/autopilot/approach_status` | int 0/1/2 | `____________________` |
| Battery master | `sim/cockpit2/electrical/battery_on[0]` | int 0/1 | `____________________` |
| Avionics master | `sim/cockpit2/electrical/avionics_on` | int 0/1 | `____________________` |
| APU | `sim/cockpit2/electrical/APU_running` | int 0/1 | `____________________` |
| Pitot heat | `sim/cockpit2/ice/ice_pitot_heat_on_pilot` | int 0/1 | `____________________` |
| Transponder mode | `sim/cockpit2/radios/actuators/transponder_mode` | int | `____________________` |
| Takeoff config warning | `laminar/B738/annunciator/takeoff_config` *(already 737-specific)* | int 0/1 | `____________________` |

---

## Part 3 — Example: Hot-Start Challenger 650 (X-Plane), flight GSG225

Known finding: `flap_handle_deploy_ratio` stays at `0`, even though flaps are
fully set. AeroACARS handles this from v0.12.1 fail-soft (LANDING CONFIG =
"not assessable" instead of red "INCOMPLETE," no score penalty). With the
correct CL650 DataRef the landing config could be properly evaluated again.

**Captured by Michel (2026-05-20), DataRefTool:**

| Function | Standard DataRef | CL650-specific DataRef | Type / value |
|---|---|---|---|
| Flap position | `sim/flightmodel2/controls/flap_handle_deploy_ratio` | `abus/CL650/ARINC429/L-DCU-7/words/FCTL/0/FLAPS_LVR` | **int 0–3** (lever detent: 0 / 1 / 20 / 30) |
| Battery master | `sim/cockpit2/electrical/battery_on[0]` | `abus/CL650/modules/DC_ELEC/0/wires/BATT_CTRL_PWR` | int 0/1 |
| Beacon | `sim/cockpit2/switches/beacon_on` | `CL650/overhead/ext_lts/beacon` | int 0/1 |
| Taxi light | `sim/cockpit2/switches/taxi_light_on` | `CL650/overhead/land_lts/recog_taxi` | int 0/1 |

**Important — flaps is a lever detent, not a deploy ratio.** `FLAPS_LVR`
returns `0,1,2,3` (corresponding to flaps 0 / 1 / 20 / 30); AeroACARS expects
`0.0–1.0`. The aircraft profile must convert — for the LANDING CONFIG check
the rule is: **lever ≥ 2 = landing configuration** (flaps 20 and 30 are both
valid landing positions for the CL650).

**Standard DataRefs that are already correct on the CL650** (no override needed —
verified on the GSG225 flight record): gear (`gear/deploy_ratio[0]` read
`1.0` correctly), engines + movement (clean phase chain Boarding→…→Takeoff),
parking brake. For the score-relevant gap on the CL650 only **the flaps DataRef**
is needed — the rest above is priority-B comfort (PIREP fields).

---

## Notes

- **Specify the value range:** an add-on DataRef may be scaled differently
  (e.g. flaps 0–30 instead of 0.0–1.0, or a detent index 0–4). Please note
  the observed range as well — the aircraft profile will then convert.
- **Arrays:** some DataRefs are arrays (`[0]`, `[1]` …). Include the index.
- If the standard `sim/...` DataRef moves correctly along → no override needed,
  leave the field empty.
- This template covers X-Plane only. MSFS study-level (Fenix, PMDG) runs
  through a separate SimVar/LVar profile system.
