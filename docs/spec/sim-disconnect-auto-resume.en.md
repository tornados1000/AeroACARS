# Sim-Disconnect Auto-Resume - Working Spec v0.4

**Status:** REVIEW / READY FOR MVP IMPLEMENTATION  
**Date:** 2026-05-12  
**Source:** Condensed from `Sim-Pause Handling - Master Spec v0.3` in the Claude worktree.  
**Goal:** Fix the real AUA-323 pain, without overwhelming the team with the entire 35h master package.

## Summary

We are **not** immediately building the complete pause master system.

We are first building a small, safe MVP:

1. If the sim is gone mid-flight, the flight may continue to pause as it does today.
2. Once the sim delivers data again, AeroACARS resumes automatically.
3. Position/altitude/fuel drift never blocks. It is only logged and shown if relevant.
4. Reposition distance must not flow into `distance_nm`.
5. Pause time is deducted from the flight/block time value.
6. The server must keep sessions together via `pirep_id`, so a 20-minute gap doesn't create two flights.

Everything else from v0.3 remains valuable, but is **not MVP**.

## Why

During the **AUA 323 LOWW->ESGG** incident on **2026-05-11**, the sim froze during descent. Afterward, no positions arrived for about 23 minutes. After a manual resume, the aircraft was already on the ground at ESGG; the server saw this as two sessions:

- Session A: long flight up to descent, no PIREP
- Session B: short ground/arrival session with a PIREP

That's bad for QA, VA review, and pilot trust. The pilot essentially continued the same flight, but our system technically cut it in two.

## Key Decision

**`pirep_id` is the flight identity.**

Not position. Not altitude. Not fuel. Not time since the last position.

If the same `pirep_id` is active, we treat it as the same flight. Large drift is a hint for the pilot and for the audit log, but not a blocker.

## Current Code State

The client already has:

- Sim disconnect detection after `SIM_DISCONNECT_THRESHOLD_S = 30`
- `paused_since`
- `paused_last_known`
- manual resume via `flight_resume_after_disconnect`
- reset of `last_lat` / `last_lon` on resume, so a reposition doesn't count toward distance
- existing UI in the active-flight panel for the pause case

So the current pain isn't "no pause logic at all", but:

- Resume is manual and can happen too late
- Pause time is not cleanly treated as an accumulator
- Server sessions can be split by long gaps
- Spec v0.3 mixes MVP and future expansion

## Time Windows and Crash Cases

These times are the working assumption for the MVP:

| Case | Time window | Expected behavior |
|---|---:|---|
| Sim stops delivering snapshots | 30 seconds | Client sets `paused_since` and keeps the flight locally in a pause state |
| AeroACARS keeps running | 30-second heartbeat cadence | phpVMS PIREP stays active even though the sim is currently not delivering position data |
| Sim crash, AeroACARS stays open | practically unbounded | Heartbeat keeps running; as soon as the sim delivers data again, auto-resume should kick in |
| MSFS/X-Plane restart, AeroACARS stays open | practically unbounded | same case as sim crash; drift is logged, not blocked |
| Blue screen / computer reboot | up to approx. 6 hours after the last heartbeat | After app start, `active_flight.json` should be loaded and the open phpVMS PIREP re-adopted via `pirep_id` |
| Computer off longer than approx. 6 hours | not guaranteed | phpVMS may have cleared the live PIREP; the client must then clearly warn instead of silently creating a wrong new flight |

Important: the 6 hours is not a client timer, but the effective phpVMS/server window after the last sign of life. While AeroACARS is still running, the 30-second heartbeat prevents exactly this scenario.

### What Happens on a Sim Crash?

If only the simulator crashes or restarts, but AeroACARS stays open:

- after 30 seconds without a snapshot, the flight is paused
- AeroACARS keeps sending heartbeats to phpVMS
- the PIREP stays alive
- after a renewed SimConnect/X-Plane snapshot, it automatically resumes
- reposition distance is not counted toward `distance_nm`

This is the most important and simplest recovery case.

### What Happens on a Blue Screen / Computer Reboot?

With a real computer failure, no heartbeat runs anymore. Then only the server window counts:

- **under approx. 6 hours:** resume/adopt should work if `active_flight.json` still exists and phpVMS still knows the PIREP
- **over approx. 6 hours:** resume is no longer guaranteed; the client should show a clear recovery/expired hint

The MVP goal is therefore: after a restart, don't blindly start fresh — first try to find the old flight via `pirep_id`.

## MVP Scope

### F1 - Auto-Resume from Existing Pause State

If `paused_since.is_some()` and `current_snapshot(&app)` again returns `Some(snapshot)`:

- end the pause
- calculate the pause duration
- calculate drift against `paused_last_known`
- set `last_lat` / `last_lon` to `None`
- clear `paused_since` and `paused_last_known`
- write to the activity log
- continue streaming normally

**No drift blockers.**

### F2 - Pause Accumulator

`FlightStats` gets:

```rust
pause_total_duration_secs: i64
pause_segments: Vec<PauseSegment>
```

All new fields must have `serde(default)`, so old `active_flight.json` files keep loading.

On resume:

- ignore pauses < 1 second
- otherwise `pause_total_duration_secs += duration`
- store a segment: start, end, reason, drift summary

### F3 - Correct Flight-/Block-Time

When calculating flight/block time:

```text
effective_duration = raw_duration - pause_total_duration
```

Important: this should apply to PIREP times, not to real UTC timestamps. Timestamps remain real.

### F4 - Keep Server Sessions Together via `pirep_id`

`aeroacars-live` must, for incoming events/positions with `pirep_id`, first try:

```text
find open or recent session by pirep_id
```

Only if no match exists may the old time/callsign/DEP/ARR heuristic take over.

This resolves the AUA-323 case server-side: a long position gap does not create a new session, as long as the `pirep_id` is the same.

### F5 - Minimal UI Instead of a New UI World

For the MVP, the existing pause/resume UI plus activity log is enough:

- Quiet: activity log only
- Noticeable drift: activity log as a warning
- Very large drift: existing active-flight area shows warning text and a cancel hint

No new toast/banner components in the MVP.

## Not MVP

These points remain valuable in the v0.3 master spec, but are deliberately deferred:

- SimConnect `Paused` / `Unpaused` events
- X-Plane plugin paused heartbeat
- Replay detection
- Aircraft change banner
- Bid change detection during pause
- Drift line on map
- full 29-scenario pilot QA
- PIREP payload display of all `pause_segments` in the webapp

Reason: this is its own package. We first need the robust base.

## Drift Levels for MVP

Only for log level and text, not for control:

| Drift | Level | Behavior |
|---|---|---|
| < 1 NM | Info | `Flight automatically resumed` |
| 1-50 NM | Info | `Flight automatically resumed - repositioned X NM` |
| 50-200 NM | Warn | `Noticeable reposition X NM` |
| > 200 NM | Warn | `Very large reposition X NM - check whether the right flight is loaded` |

No level blocks resume.

## Data Model

### PauseSegment

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PauseSegment {
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
    duration_secs: i64,
    reason: PauseReason,
    drift_nm: Option<f64>,
    altitude_delta_ft: Option<f64>,
    fuel_delta_kg: Option<f64>,
}
```

### PauseReason

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
enum PauseReason {
    SimDisconnect,
    ManualResume,
}
```

`SimDisconnect` is enough for the MVP. `ManualResume` can optionally be dropped if it unnecessarily complicates the model.

## MVP Acceptance Criteria

| # | Criterion |
|---|---|
| A1 | Sim delivers no snapshots for >30s -> `paused_since` is set |
| A2 | Sim delivers a snapshot again afterward -> client resumes automatically without a button |
| A3 | Resume with 0 NM drift writes an info log |
| A4 | Resume with 80 NM drift still resumes and writes a warn log |
| A5 | Resume with 500+ NM drift still resumes and points to a possibly wrong flight |
| A6 | Reposition distance is not added to `distance_nm` |
| A7 | 10 minutes of pause reduces the PIREP flight time by 10 minutes |
| A8 | App restart with an old `active_flight.json` without new fields works |
| A9 | Server merges two position blocks with the same `pirep_id` into one session |
| A10 | Old manual resume button remains usable as a fallback |
| A11 | Computer reboot under approx. 6 hours adopts the open PIREP based on `pirep_id`, if `active_flight.json` is present |
| A12 | Computer reboot over approx. 6 hours shows a clear expired/recovery hint and does not silently start a wrong new flight |

## Implementation Order

### Phase 1 - Client Minimal

1. Extend `FlightStats` with a pause accumulator.
2. Helper `resume_from_pause_if_snapshot_available(...)`.
3. Change the existing `is_paused` block in the streamer:
   - if a snapshot exists: auto-resume
   - if no snapshot: keep pausing
4. Manual `flight_resume_after_disconnect` uses the same helper.
5. Flight-/block-time deduction.
6. Unit tests for drift and the pause accumulator.

### Phase 2 - Server Minimal

1. Ensure client/MQTT/recorder events carry `pirep_id` sufficiently.
2. `findSessionByPirepId`.
3. `ensureSession` prioritizes `pirep_id`.
4. Test: a 23-minute gap remains one session.

### Phase 3 - UI Polish

1. Adjust text in the existing pause hint: auto-resume is running, the button is a fallback.
2. Clearer cancel hint on large drift.

## Open Decisions

Only these decisions need to be settled before the MVP:

1. **Deduct pause time:** deduct all pauses, or only airborne ones?  
   Recommendation: deduct all. That's consistent and simple.

2. **Warning threshold:** are 50/200 NM enough for a warning?  
   Recommendation: yes for the MVP. Later this can become phase-/distance-dependent.

3. **Server join:** may an ARRIVED/filed session with the same `pirep_id` be reopened?  
   Recommendation: no. `pirep_filed`/ARRIVED remains terminal. `pirep_id` join only for non-terminal sessions.

## Explicitly Not Deciding Now

To keep the scope clear:

- no aircraft family logic
- no X-Plane plugin version
- no map/drift line
- no complete toast/banner world
- no 29 manual scenarios before the first MVP

## Next Concrete Step

Before implementation, verify once in the code:

- Where `flight_time_min` / `block_time_min` are finally calculated
- Whether all position/MQTT events contain `pirep_id`
- How `ensureSession` currently handles terminal sessions

Once that's clear, Phase 1 can start.

## Development Instruction Phase 1+2

This instruction is the actionable work order for development. The goal is a shared MVP made of client resume and server session join.

### Goal

AeroACARS should cleanly continue a running flight after sim disconnect, sim crash, MSFS/X-Plane restart, or computer reboot, as long as it's the same phpVMS PIREP (`pirep_id`) being referred to.

The flight must no longer fall apart into two recorder/webapp sessions due to a position gap.

### Scope

Only these points are implemented:

1. Client detects sim disconnect after 30 seconds as before.
2. Client resumes automatically as soon as a sim snapshot is available again.
3. Client stores pause segments and adds up `pause_total_duration_secs`.
4. Client deducts pause time from flight/block time.
5. Client resets `last_lat` / `last_lon` on resume, so reposition distance is not counted.
6. Client sends `pirep_id` along in the relevant position/MQTT/recorder payloads.
7. Recorder first matches incoming positions by `pirep_id`.
8. Recorder merges open/recent sessions with the same `pirep_id`.
9. Terminal sessions must not be reopened.

Not part of this order:

- new toast/banner architecture
- drift line on map
- SimConnect pause/unpause events
- X-Plane pause heartbeat
- replay detection
- aircraft change blocker
- large webapp pause segment view

### Client Tasks

Main affected area:

- `client/src-tauri/src/lib.rs`

Implementation:

1. Extend `FlightStats`:

```rust
pause_total_duration_secs: i64
pause_segments: Vec<PauseSegment>
```

All new fields need `#[serde(default)]` or compatible defaults, so old `active_flight.json` files keep loading.

2. Introduce `PauseSegment`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PauseSegment {
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
    duration_secs: i64,
    reason: PauseReason,
    drift_nm: Option<f64>,
    altitude_delta_ft: Option<f64>,
    fuel_delta_kg: Option<f64>,
}
```

3. Build a shared helper:

```text
resume_from_pause_if_snapshot_available(app, flight, snapshot) -> ResumeResult
```

The helper should:

- only work if `paused_since.is_some()`
- calculate pause duration
- calculate drift against `paused_last_known`
- store a pause segment
- increase `pause_total_duration_secs`
- set `last_lat` / `last_lon` to `None`
- clear `paused_since` / `paused_last_known`
- save `active_flight.json`
- write to the activity log

4. Change the streamer loop:

- If a pause state is active and no snapshot is available: keep pausing.
- If a pause state is active and a snapshot is available: call the helper and then continue streaming normally.
- No manual click should be needed for the normal case.

5. Keep the manual resume button:

- `flight_resume_after_disconnect` remains a fallback.
- It should use the same helper or the same core logic, so auto-resume and manual resume don't diverge.

6. Correct the time calculation:

```text
effective_duration_secs = raw_duration_secs - pause_total_duration_secs
```

This applies to flight/block time values sent to phpVMS/PIREP. UTC timestamps remain real times and are not shifted.

7. Check position/MQTT payload:

- Make sure `pirep_id` is included in the payloads during an active flight that the recorder uses for session assignment.
- If `pirep_id` is missing, add the field.

### Server/Recorder Tasks

Main affected areas:

- `aeroacars-live/recorder/src/mqttSubscriber.ts`
- `aeroacars-live/recorder/src/db.ts`

Implementation:

1. In `ensureSession(...)`, check `pirep_id` first:

```text
if payload.pirep_id exists:
    session = find reusable non-terminal session by pirep_id for this pilot
    if session exists:
        use session
```

2. Build a new DB function:

```text
findReusableSessionByPirepId(va_prefix, pilot_id, pirep_id)
```

It may only return sessions that:

- belong to the same pilot
- have the same `pirep_id`
- are not terminally finished
- are open or within the allowed resume/6h window

3. Terminal protection:

Sessions with a final state like `ARRIVED`, `FILED`, `pirep_filed`, or comparable `END_PHASES` must not be used again as active.

4. Keep the fallback:

If no `pirep_id` match exists, the previous heuristic stays active:

- latest session
- callsign/DEP/ARR/aircraft context
- time window

5. Optional merge:

If two non-terminal sessions with the same `pirep_id` have already been created, the recorder may merge them. This is helpful for legacy cases, but not mandatory for the first MVP, as long as new gaps no longer split sessions.

### Behavior on Drift

Drift must not block resume.

| Drift | Behavior |
|---|---|
| < 1 NM | Info log |
| 1-50 NM | Info log with distance |
| 50-200 NM | Warn log |
| > 200 NM | Warn log with a hint to check the right flight |

Even at >200 NM, it resumes, as long as `pirep_id` matches.

### Behavior on Computer Reboot

On app start:

1. load an existing `active_flight.json`
2. use `pirep_id` from the active flight
3. check/adopt whether phpVMS/recorder still knows the PIREP
4. if within approx. 6 hours since the last heartbeat: allow resume
5. if no longer findable/expired: clear recovery message, do not create a silent new flight

### Tests / QA

Minimum tests for the implementation:

1. Sim gone >30s -> `paused_since` is set.
2. Sim comes back -> auto-resume without a button.
3. 10-minute pause -> PIREP time is 10 minutes shorter than raw wall-clock.
4. Reposition 80 NM -> resume + warning, distance is not added.
5. Reposition 500 NM -> resume + clear warning, no block.
6. App restart with an old `active_flight.json` -> no deserialization errors.
7. Recorder receives two position blocks with the same `pirep_id` and a 20-30 minute gap -> one session.
8. Recorder receives the same `pirep_id`, but a terminal session -> no reopening.
9. Computer reboot under approx. 6h -> old PIREP is adopted.
10. Computer reboot over approx. 6h or PIREP not findable -> clear recovery/expired message.

### Definition of Done

- `npm run build` in the client is green
- `cargo check` in the client is green
- existing client tests are green
- `npm run build` in the recorder is green
- new/adjusted tests for pause accumulator and session join exist
- QA with at least one simulated disconnect and one server gap documented

## Development Order v0.7.15 - Sim-Recovery Release

### Release Goal

`v0.7.15` should not just be a small hotfix, but a closed **sim-recovery release**.

The goal is: a running flight should be continued as robustly as possible after sim crash, sim pause, sim restart, X-Plane pause, aircraft change after recovery, and computer reboot, without phpVMS or the recorder producing wrong times, wrong distances, or multiple sessions from it.

### Base Already Included

These points are considered the base and remain part of `v0.7.15`:

1. Phase 1 client auto-resume.
2. Pause accumulator.
3. Pause-time deduction from flight/block time.
4. Reposition distance is not counted.
5. Heartbeat during sim-disconnect pause with `last_good_snap`.
6. `pirep_id` in the position/MQTT payload.
7. Phase 2 server join via `pirep_id`.
8. 6h cutoff for ended/recent session reuse.
9. Terminal protection against reopening filed/arrived sessions.

### Additional Scope for v0.7.15

In `v0.7.15`, the following should also be implemented:

| ID | Topic | Goal |
|---|---|---|
| F5 | SimConnect Pause/Unpause events | Cleanly detect MSFS pause/frozen snapshot and accumulate pause time |
| F6 | X-Plane paused heartbeat | Actively report X-Plane pause, even if old/stationary data keeps arriving |
| F7 | Aircraft change banner | Warn after recovery if the sim aircraft/registration no longer matches the active flight |

`F8 Bid-Change-Detection` remains optional as a light check for `v0.7.15`. If it delays the release, F8 is deferred.

### No Longer Included in v0.7.15

To keep the release from expanding:

- no drift line on map
- no new toast/banner architecture
- no replay detection
- no complete webapp pause-segment view
- no full 29-scenario QA before the first release
- no large refactors outside recovery/pause/resume

### Implementation Order

#### Step 1 - F5 MSFS SimConnect Pause/Unpause

Goal: the client should detect a real MSFS pause, even if SimConnect keeps delivering frozen snapshots.

Implementation:

1. In the MSFS/SimConnect adapter, check whether a pause state from SimConnect is available.
2. Build the pause state into the shared `SimSnapshot` or an accompanying status.
3. Extend the streamer loop:
   - when a sim pause becomes active: set `paused_since`, store `paused_last_known`
   - during sim pause: keep sending heartbeat
   - when sim pause ends: use the same resume helper as auto-resume
4. No hard block on drift.
5. Pause time must flow into `pause_total_duration_secs`.

Acceptance:

- MSFS Esc-pause for 2 minutes -> pause segment of approx. 120 seconds.
- phpVMS heartbeat keeps running during the pause.
- After unpause, the flight resumes without a button.
- Flight time in the PIREP is reduced by the pause time.

#### Step 2 - F7 Aircraft Change Banner

Goal: after resume/recovery, the pilot should see if the currently loaded aircraft doesn't match the active flight.

Implementation:

1. Store existing aircraft data on pause start:
   - `aircraft_icao`
   - registration, if available
   - possibly title/model string, if already present
2. On resume, compare the current snapshot/sim aircraft against the active flight.
3. On mismatch:
   - show a warning in the existing active-flight area
   - write to the activity log
   - don't block resume
4. Don't build a new UI system. Use the existing warning/banner area.

Acceptance:

- Same aircraft -> no warning.
- Same family/ICAO -> no hard warning, at most info if registration differs.
- Different ICAO -> visible warning.
- Warning does not block resume.

#### Step 3 - F6 X-Plane Paused Heartbeat

Goal: X-Plane should be able to report an active pause state to AeroACARS, so pause time is also cleanly counted there.

Implementation:

1. Check in the X-Plane plugin/adapter whether `sim/time/paused` or an equivalent pause state can be read.
2. Transport the pause state to the client.
3. Reuse the shared pause logic:
   - pause active -> `paused_since`
   - heartbeat with the last position
   - pause end -> resume helper
4. If the plugin protocol needs to be extended:
   - introduce a backward-compatible field
   - old plugin versions keep running without F6
   - the UI shows no error, but keeps working as before with disconnect detection

Acceptance:

- X-Plane pause for 2 minutes -> pause segment of approx. 120 seconds.
- Old X-Plane plugin state without a pause field -> no crash, fallback as before.
- Heartbeat keeps running during the pause.
- Flight time is reduced by the pause time.

### F8 Bid-Change-Detection Light Optional

Only implement if there's still clean time left after F5-F7.

Light scope:

- On resume, check whether the active phpVMS PIREP still has the same `pirep_id`.
- If a different bid/PIREP is active: warning and no silent switch.
- Don't build a large bid-change UI.

### QA Matrix v0.7.15

| # | Area | Test |
|---|---|---|
| Q1 | Sim disconnect | Sim delivers no snapshots for >30s -> pause state |
| Q2 | Sim disconnect | Without a snapshot, client sends a heartbeat every 30s with `last_good_snap` |
| Q3 | Auto-resume | Snapshot comes back -> resume without a button |
| Q4 | Pause time | 10-minute pause reduces PIREP time by 10 minutes |
| Q5 | Distance | 80 NM reposition is not added to `distance_nm` |
| Q6 | Server | 25-minute position gap with the same `pirep_id` remains one session |
| Q7 | Server | ARRIVED/PIREP_SUBMITTED session is not reopened |
| Q8 | Reboot | Computer/app restart under 6h adopts the open PIREP |
| Q9 | Reboot | Restart after >6h/PIREP gone shows a recovery hint |
| Q10 | MSFS Pause | MSFS Esc-pause produces a pause segment and heartbeat keeps running |
| Q11 | MSFS Pause | MSFS unpause resumes automatically |
| Q12 | X-Plane Pause | X-Plane pause produces a pause segment if the plugin delivers a pause state |
| Q13 | X-Plane Compat | Old X-Plane plugin without pause state remains compatible |
| Q14 | Aircraft Change | Same aircraft after resume -> no warning |
| Q15 | Aircraft Change | Different aircraft after resume -> visible warning, no block |

### Release Gates v0.7.15

Must be green before build/release:

- Client: `npm run build`
- Client: `cargo check`
- Client: `cargo test`
- Client: existing frontend tests
- Recorder: `npm run build`
- Recorder: `npm test`
- Webapp: `npm run build`
- one real or synthetic MSFS pause/unpause QA
- one X-Plane compatibility QA, at least with an old plugin fallback

### Release Notes Draft

Title:

```text
v0.7.15 - Sim Recovery Release
```

German:

```text
Diese Version verbessert die Wiederaufnahme laufender Fluege nach Simulator-Crash, Pause, Neustart oder kurzer Rechner-Unterbrechung.

- AeroACARS pausiert den Flug automatisch, wenn der Simulator keine Daten mehr liefert.
- Der phpVMS-PIREP wird waehrend einer Sim-Unterbrechung weiter per Heartbeat wachgehalten.
- Sobald der Simulator wieder Daten liefert, wird der Flug automatisch fortgesetzt.
- Pausezeit wird von Flight-/Block-Time abgezogen.
- Reposition-Distanz nach Recovery wird nicht als geflogene Distanz gezaehlt.
- Der Recorder haelt Sessions anhand der `pirep_id` zusammen, damit ein Flug nicht durch Datenluecken in mehrere Sessions zerfaellt.
- MSFS-Pause/Unpause und X-Plane-Pause werden, soweit vom Simulator/Plugin gemeldet, als echte Pause behandelt.
- AeroACARS warnt, wenn nach Recovery ein anderes Flugzeug geladen ist.
```

English:

```text
This release improves recovery for active flights after simulator crashes, pauses, restarts, or short computer interruptions.

- AeroACARS automatically pauses the flight when the simulator stops sending data.
- The phpVMS PIREP is kept alive with heartbeat updates during simulator interruptions.
- The flight automatically resumes once simulator data returns.
- Paused time is subtracted from flight/block time.
- Reposition distance after recovery is not counted as flown distance.
- The recorder keeps sessions together by `pirep_id`, preventing one flight from being split by telemetry gaps.
- MSFS pause/unpause and X-Plane pause, where reported by the simulator/plugin, are treated as real pause time.
- AeroACARS warns if a different aircraft is loaded after recovery.
```

## Code Check 2026-05-12

These points were checked against the current state:

### Client

- `SIM_DISCONNECT_THRESHOLD_S = 30` exists.
- `HEARTBEAT_INTERVAL = 30s` exists; the client keeps phpVMS regularly alive while the app is running.
- `paused_since` and `paused_last_known` exist in `FlightStats`.
- `flight_resume_after_disconnect` exists and already does the important reset of `last_lat` / `last_lon`.
- The streamer currently keeps blocking in the pause state and waits for a manual resume.
- `build_heartbeat_body(...)` currently calculates `flight_time_secs` from `takeoff_at`/`landing_at` or `block_off_at`, but without pause deduction.
- Some old code comments still talk about `acars.live_time` / approx. 2h. For current operation, approx. 6h after the last heartbeat is assumed as the QA baseline; the comments should be cleaned up during implementation.

### Server

- `ensureSession(...)` in `aeroacars-live/recorder/src/mqttSubscriber.ts` currently prioritizes `latest` + `matchesFlightContext(...)` + `RESUME_WINDOW_MS`.
- `ensureSession(...)` currently does not read a `pirep_id` value from the position payload as the first join key.
- `END_PHASES` exists and protects terminal sessions.
- `findSessionByPirepForPilot(...)` exists in `db.ts`, but is meant for client log upload and is not the needed `ensureSession` join.
- `findActiveSessionForPilot(...)` already uses a 6h window server-side for active sessions.

### Consequence

MVP Phase 1 can start purely client-side. For the AUA-323 server split, MVP Phase 2 is additionally needed:

1. Ensure `pirep_id` from the position/MQTT payload.
2. Build `db.findReusableSessionByPirepId(...)`, with protection against terminal sessions.
3. Have `ensureSession(...)` match against this session right at the start.
