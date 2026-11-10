# Overnight Fix Session — 2026-05-01 → 2026-05-02

User went to bed at ~23:00 with a bag of bugs from the evening's
test flight (DLH 155 EDDP→EDDF). I worked through them while they
slept. Backend `cargo check` and frontend `tsc --noEmit` both pass
clean. Nothing pushed to the remote — all commits stay local for
review.

## Commits in order

| Commit | Phase | What |
|---|---|---|
| `40c487b` | 1A–D | Critical PIREP fixes: flight-clear bug, native API fields |
| `1b222ac` | 2 | Touchdown ring buffer for V/S + G recovery |
| `867fb18` | 3 | Activity-log polish (boarding, banner, freqs, throttle, time, final) |
| `33367f1` | 4 + 5 | PUSHBACK STATE SimVar + 19 Fenix LVars from AAO script |

## What got fixed

### Phase 1 — Critical PIREP bugs (commit `40c487b`)

| Bug | Before | After |
|---|---|---|
| Flight not cleared from UI after auto-file | Stuck on cockpit panel waiting for next 2 s poll | `setActiveFlight(null)` directly after `flight_end` Ok |
| Verbleibender Treibstoff = −2113 kg in phpVMS | `block_fuel` not sent in API body | Native `block_fuel` field, lbs, populated from `stats.block_fuel_kg` |
| Flt.Time 1h 22m vs actual 42m | Sent `(now - started_at)` | Sent `(landing_at - takeoff_at)` |
| Flt.Level empty | Field missing entirely | New `peak_altitude_ft` tracker, rounded to 100 ft |
| Landing Rate empty in native column | Only in custom field | Native `landing_rate` field added |
| Score empty | No mapping | LandingScore enum mapped 0..100 (Smooth=100, Severe=0) |

### Phase 2 — Touchdown ring buffer (commit `1b222ac`)

The single-tick "first on_ground=true" capture was catching the
rebound of bouncing landings (Pilot's last test: V/S +48 fpm
positive, G 1.00, 0 bounces vs MSFS overlay −273 fpm / 1.31 G / 1
bounce). Added a 5-second ring buffer of (timestamp, V/S, G,
on-ground) on every tick. At touchdown the FSM scans for the
most-negative V/S in airborne samples and the peak G, takes the
worst of {TOUCHDOWN SimVar, buffer min, current tick} as truth.

**Bounces are still tick-rate-limited** — a sub-second bounce that
fits between two snapshots can't be detected without a higher
SimConnect refresh rate. That's a separate fix (ClientData /
WASM-gauge bridge) for later.

### Phase 3 — Activity-log polish (commit `867fb18`)

* **"Phase: Boarding"** now logs explicitly at `flight_start` so the
  timeline's first checkpoint has a textual counterpart.
* **Aircraft banner** gated on a dedicated `aircraft_banner_logged`
  field instead of the heuristic "all three diff fields are None at
  first tick". Persisted, so resumes don't re-fire it.
* **COM/NAV frequency change logs removed entirely.** Sector
  hand-offs were filling the log with noise; Fenix RMP doesn't
  even sync to the standard SimVars. Squawk stays.
* **Session-restored entries throttled** to once per 60 s.
* **PIREP custom-field timestamps** rendered as `HH:MM:SS UTC`
  instead of full ISO with microseconds.
* **Final phase trigger** lowered from 1500 ft AGL → 700 ft AGL.

### Phase 4 + 5 — Pushback + Fenix LVars (commit `33367f1`)

`PUSHBACK STATE` SimVar drives Pushback → TaxiOut. Value 3 = "no
pushback" = clean signal. Legacy fallback retained.

**19 new Fenix LVars** added to the raw-FFI subscription, names
verified against the AAO script bundle now vendored at
`docs/vendor/FENIX_A3XX_AxisAndOhs_Scripts.xml`:

* Switches (11): seat belts, no smoking, APU master/start,
  anti-ice eng1/eng2/wing/probe-heat, BAT1, BAT2, EXT PWR
* FCU AP buttons (4): AP1, AP2, APPR, ATHR
* FCU encoder displays (4): ALT, HDG, SPD, V/S
* Autobrake indicator lamps (3): LO, MED, MAX

New activity-log entries:
* "Seat belts AUTO/ON/OFF"
* "No smoking AUTO/ON/OFF"
* "Autobrake LO/MED/MAX/OFF"
* "Selected ALT/HDG/SPD/V/S {value}" — 2 s debounced

## Assumptions baked in (NEEDS VERIFICATION)

These are educated guesses based on the AAO script + research
agent's findings. None of them have been live-tested against
Fenix yet.

1. **`L:S_OH_SIGNS` semantics: 0=OFF, 1=AUTO, 2=ON.** The AAO
   script confirms three states but doesn't print the labels. If
   Fenix uses a different convention (e.g. 0=AUTO, 1=ON), the
   activity log labels will be off by one.
2. **`L:S_OH_PROBE_HEAT`: assumed AUTO=0, ON=1.** Real Airbus
   has a 2-position toggle. If Fenix returns 1 for AUTO and 2 for
   ON, our boolean conversion treats both as "on" which is fine
   for the pitot-heat indicator.
3. **`L:S_OH_ELEC_BAT1/2`: assumed 0=OFF, 1=AUTO.** Real Airbus
   pushbutton is OFF/AUTO 2-state.
4. **`L:S_FCU_AP1/AP2`: button-state OR is "AP master engaged".**
   Probably correct on Airbus — both A and B autopilots can be
   independently engaged. If it turns out the button is momentary
   (only true while pressed), the AP indicator will flicker.
5. **`L:E_FCU_ALTITUDE/HEADING/SPEED/VS` units.** Assumed to be
   the displayed integer (ALT in ft, HDG in degrees, SPD in kt,
   VS in fpm). If the LVar is normalized (e.g. ALT ÷ 100), the
   logged values will be wrong by orders of magnitude.
6. **PUSHBACK STATE values: 0/1/2/3.** Standard MSFS, but worth
   verifying that 3 actually fires when the tug disconnects (not
   only when there was never a tug).
7. **Touchdown ring buffer** assumes the snapshot rate stays at
   ~1 Hz. If we ever bump SimConnect to 10+ Hz, the 5 s window
   gives 50+ samples which is fine; if SimConnect ever falls
   below 1 Hz the buffer might be empty when we need it.

## How to verify the assumptions

The tools we built earlier (Settings → Debug) make this fast:

1. **Switch State Panel** — flip cabin signs / autobrake / APU at
   the gate, watch the pills change colour and read the activity
   log entries that follow.
2. **Switch-Detective Diff** — Snapshot A → flip one switch →
   Snapshot B. The diff shows the exact field that changed plus
   its before/after values, so units/labels are immediately
   verifiable.
3. **Inspector** — type any LVar name (e.g. `L:S_OH_SIGNS`),
   watch the live value as you toggle the cockpit switch.

## Known limitations / not done tonight

* **Loadout panel (Pax/Cargo/Fares in Cockpit)** — deferred. Data
  reaches phpVMS correctly already; just no in-cockpit display.
  Maybe a 30 min job tomorrow if you want it.
* **Fenix RMP frequencies** — we removed the COM/NAV frequency
  logging entirely (per your instruction), so the Fenix-RMP-LVar
  research never happened. Frequencies still subscribed for the
  debug panel but not logged.
* **APU LVar verification** — Phase 5 added `L:S_OH_ELEC_APU_MASTER`
  but the assumed semantics (0=OFF, 1=ON) may differ from real
  Fenix behaviour for the Master switch (which is push-to-toggle).
  Test by flipping APU MASTER in cockpit, watching the Inspector
  value.
* **Sub-second bounces** — touchdown ring buffer cannot catch
  bounces shorter than the snapshot interval. Real fix is a
  higher-rate ClientData bridge, out of scope.
* **PIREP times only show UTC**, not local. The custom-field
  column is narrow; adding "21:33 LT (19:33 UTC)" would be 16
  characters per cell vs the current 12. Doable if you want it.

## Compiles / type-checks (verified before commit)

```
cd client/src-tauri && cargo check     → Finished, 10 warnings (rustdoc on /// before const expressions, cosmetic)
cd client && npx tsc --noEmit           → no errors
```

## Test plan for first morning flight

1. Start a fresh Fenix flight from a bid (e.g. another EDDP→EDDF
   so we can compare against last night's reference).
2. **At gate** before pushback: open Settings → Debug, scroll to
   "Schalter & Anzeigen". Verify cabin signs (OFF/AUTO), Battery,
   APU look correct.
3. **Push back** with the MSFS pushback feature (Ctrl+P or whatever
   you use). Watch the log:
   * "Phase: Boarding" should appear right after Flight Started.
   * "Phase: Pushback" only when actually moving.
   * "Phase: TaxiOut" only when MSFS reports tug disconnected (or
     after movement+engines without pushback signal).
4. **Climb** through your first selected ALT/HDG/SPD changes.
   The "Selected ALT 36000" / "Selected HDG 280" log entries
   should fire 2 s after each knob settle.
5. **Approach**: Final should NOT trigger at 1500 ft. Wait for
   ~700 ft AGL.
6. **Land**: check the activity log "Touchdown:" entry. V/S
   should match the MSFS dev-overlay landing rate widget,
   negative number (e.g. -200 fpm). G should be > 1.0 if the
   landing was firmer than butter.
7. **PIREP**: after Arrived phase, the auto-file should fire and
   the cockpit panel should clear immediately (no delay).
8. **In phpVMS** check the PIREP details:
   * "Verbleibender Treibstoff" should be POSITIVE.
   * "Flt.Time" should match takeoff→landing minutes.
   * "Flt.Level" should show your cruise FL.
   * "Landing Rate" should match what we logged.
   * Times should be `HH:MM:SS UTC`, no microseconds.

## If something goes wrong

The commits are individually revertable:

```
git revert 33367f1   # rolls back PUSHBACK STATE + Fenix LVars
git revert 867fb18   # rolls back log polish
git revert 1b222ac   # rolls back touchdown ring buffer
git revert 40c487b   # rolls back PIREP API fields
```

Or to nuke everything from the overnight session:

```
git reset --hard 7f71ecf   # back to where you were before bed
```
