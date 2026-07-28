# Landing-Rate Detection — Algorithm + Comparison

**Date**: 2026-05-03
**Author**: Reverse-engineering pass on installed competitors (SimCARS v1.1.58, Volanta v1.17.2)
**Status**: Reference document — not an ADR, captured so the design behind our touchdown analyzer doesn't get lost over time
**Scope**: How AeroACARS computes the landing rate (vertical speed at touchdown), why it's more accurate than what the two dominant competitors (SimCARS and Volanta) do, and where the weaknesses of our approach lie

---

## TL;DR

| Aspect | Volanta v1.17.2 | SimCARS v1.1.58 | **AeroACARS** |
|---|---|---|---|
| Sample rate | event-driven (~1 Hz) | event-driven (~1 Hz, only on VS change) | **30 Hz dedicated sampler** |
| Buffer | 0 (only previous tick) | 100 samples (~100 s) | 5 s × 30 Hz ≈ 150 samples |
| Touchdown detection | `on_ground` edge | `on_ground` edge | **AGL threshold with bounce arming** |
| Landing-rate source | live VS at edge | live VS preferred, else `max(\|buffer\|)`, else `Random(-180..-220)` ⚠ | **`max(\|VS\|)` over 5 s look-back buffer** |
| G-force | live tick | peak after touchdown | peak within the 5 s window |
| Bounces | ❌ | ✅ max 5 (separate counters) | ✅ unlimited, AGL-based |
| Sideslip / crab | ❌ | ❌ | ✅ |
| Touchdown V/S curve | ❌ | ❌ | ✅ ±2 s sub-buffer for PIREP notes |
| Random fallback | no | **yes** ⚠ | no |
| Classification | numeric only | numeric only | 5-level (Smooth / Acceptable / Firm / Hard / Severe) on V/S **and** G |

**In short:** Volanta takes the first value that falls into its lap, SimCARS has a buffer but samples too slowly to catch the real touchdown sub-frame, we sample fast enough **and** explicitly search for the peak.

---

## A. Volanta — minimal approach

Volanta is **Electron**, the sim connector lives in the renderer preload. From `dist/preload/preload.js` (deminified):

```js
checkLanding(t) {
  if (
    this.previousData
    && !this.previousData.onGround
    && t.onGround
    && !t.inReplayMode
    && t.groundSpeed > 0
  ) {
    const r = {
      landingRate: t.verticalSpeed,   // ← live VS in exactly this tick
      gForce: t.gForce,
      pitch: t.pitch,
      roll: t.bank,
      groundSpeed: t.groundSpeed,
      latitude: t.latitude,
      longitude: t.longitude,
      heading: t.headingTrue,
      windHeading: t.windHeading,
      windSpeed: t.windSpeed,
      isHidden: false,
      id: "",
      // ...
    };
    // → emitted to handler that posts to volanta backend
  }
}
```

### That's it. Really.

- No buffer
- No peak
- No look-back
- Exactly one value per touchdown
- Called by the sim polling loop (typical cadence: ~1 Hz in Volanta)

### Consequences

1. **Frame race on the touchdown tick:** If the sim reports a frame with V/S ≈ 0 at the exact tick moment (typical when the pilot happened to land well between the bounce spike and the sample tick), the pilot gets a markedly too-buttery landing rate.
2. **No bounce detection.** A hard landing with three bounces looks like a single clean landing in Volanta.
3. **No G-force tracking.** The G value is the one from exactly that sample tick.
4. **Works "well enough" for most flights** because on clean landings the V/S curve passes smoothly through the touchdown point — the single sample value is then not far from the peak. On interesting landings (= hard or bounced) it becomes inaccurate.

---

## B. SimCARS — buffer + state machine

SimCARS is **.NET WinUI 3** shipped as an MSIX package. The sim bridge sits in `SimCARSServer.exe` (a separate process, talking SimConnect directly). From the decompiled sources in `SimCARSServer/DataValues.cs` and `GlobalValues.cs`:

### Global state

```csharp
internal static class GlobalValues {
    public static int Landingrate = 0;
    public static double LandingGForce = 0.0;
    private static bool _landingPossible = false;
    private static bool _simOnGround = true;
    public static int Bounced1 = 0;   // up to 5 bounces tracked individually
    public static int Bounced2 = 0;
    public static int Bounced3 = 0;
    public static int Bounced4 = 0;
    public static int Bounced5 = 0;
    public static DateTime LandingPossibleDateTime = DateTime.MinValue;
    public static ConcurrentQueue<int> verticalSpeeds = new ConcurrentQueue<int>();
}
```

### Buffer filling (in `DataValues.VertikalSpeed.set`)

```csharp
public int VertikalSpeed {
    set {
        if (value != vertikalSpeed) {
            vertikalSpeed = value;
            if (GlobalValues.verticalSpeeds.Count > 100) {
                GlobalValues.verticalSpeeds.TryDequeue(out var _);
            }
            GlobalValues.verticalSpeeds.Enqueue(vertikalSpeed);
        }
    }
}
```

**Important:** The buffer is filled **only on value change** — setter-triggered, not periodic. If V/S is constant at -1200 fpm for 10 seconds (a clean descent phase), **not a single new sample** enters the buffer during that time.

### Arming (in `DataValues.AltitudeOverGround.set`)

```csharp
public int AltitudeOverGround {
    set {
        if (_altitudeOverGround == value) return;
        _altitudeOverGround = value;
        if (_altitudeOverGround > 500
            && !GlobalValues.LandingPossible
            && !SimOnGround
            && GroundSpeed > 60)
        {
            if (GlobalValues.LandingPossibleDateTime != DateTime.MinValue
                && GlobalValues.LandingPossibleDateTime < DateTime.Now.AddSeconds(-30.0))
            {
                // 30 s of AGL > 500 + GS > 60 + airborne → we really are flying.
                // Reset everything, arm the window.
                GlobalValues.Landingrate = 0;
                GlobalValues.LandingGForce = 0.0;
                GlobalValues.Bounced1 = 0;
                /* ... Bounced2..5 likewise ... */
                GlobalValues.verticalSpeeds = new ConcurrentQueue<int>();
                GlobalValues.LandingPossible = true;
            }
            else if (GlobalValues.LandingPossibleDateTime == DateTime.MinValue) {
                GlobalValues.LandingPossibleDateTime = DateTime.Now;  // start the timer
            }
        }
    }
}
```

The **30-second gate** is clever: it prevents a brief hop on the runway or a small GA-plane bounce-up from counting as a "flight." With touch-and-go, however, the pilot never reaches 30 s above 500 ft AGL → landing detection never arms.

### Touchdown detection (in `DataValues.SimOnGround.set`)

```csharp
public bool SimOnGround {
    set {
        // Peak-G tracker — runs continuously as long as Landingrate is set
        if (GlobalValues.Landingrate != 0 && gforce > GlobalValues.LandingGForce) {
            GlobalValues.LandingGForce = gforce;
        }
        if (GlobalValues.SimOnGround == value) return;

        // Touchdown edge: airborne → on_ground, AND landing window is armed
        if (GlobalValues.LandingPossible && value && GlobalValues.SimCarsFlightIsStarted) {
            if (GlobalValues.Landingrate == 0) {
                // First touchdown
                if (VertikalSpeed != 0) {
                    GlobalValues.Landingrate = Math.Abs(VertikalSpeed) * -1;
                } else {
                    // Live VS happened to be 0 → max from the buffer
                    GlobalValues.Landingrate = Math.Abs(
                        GlobalValues.verticalSpeeds.ToList().Max(i => Math.Abs(i))
                    ) * -1;
                    if (GlobalValues.Landingrate == 0) {
                        // Buffer also empty → random fallback
                        GlobalValues.Landingrate = new Random().Next(-180, -220);
                    }
                }
            }
            // Bounces — fill slot if free
            else if (GlobalValues.Bounced1 == 0) GlobalValues.Bounced1 = Math.Abs(VertikalSpeed) * -1;
            else if (GlobalValues.Bounced2 == 0) GlobalValues.Bounced2 = Math.Abs(VertikalSpeed) * -1;
            else if (GlobalValues.Bounced3 == 0) GlobalValues.Bounced3 = Math.Abs(VertikalSpeed) * -1;
            else if (GlobalValues.Bounced4 == 0) GlobalValues.Bounced4 = Math.Abs(VertikalSpeed) * -1;
            else if (GlobalValues.Bounced5 == 0) GlobalValues.Bounced5 = Math.Abs(VertikalSpeed) * -1;
        }
        GlobalValues.SimOnGround = value;
    }
}
```

### Disarm

```csharp
// In GroundSpeed.set
if (GlobalValues.Landingrate != 0 && value < 40) {
    GlobalValues.LandingPossible = false;
}
```

As soon as the aircraft rolls below 40 kt GS, the window is closed. After this point the landing rate no longer changes.

### Assessment of SimCARS

**Strengths:**
- The 30 s arming gate is an elegant "are you really flying?" check
- Buffer as a backup when live V/S = 0 is caught
- Bounce tracking (at least)
- Continuous peak-G tracking after touchdown — good

**Weaknesses:**
- **Sample rate is effectively ~1 Hz.** Buffer filling only on VS change means: a constant descent = no samples = at touchdown the buffer is not "the last 5 seconds" but "the last 100 value changes since arming, which may be 30 minutes ago." The `max(|buffer|)` may therefore pull a spike from 5 minutes ago as the landing rate.
- **The random fallback `(-180..-220)`** is a genuine hack. If live V/S **and** buffer max are both 0 (the pilot touched down with V/S ≈ 0, which can absolutely happen on very smooth landings), an **invented** landing rate is reported. It's in the code and explains why some SimCARS pilots occasionally see a strange landing rate that can't be reproduced via replay.
- **Touch-and-go** is not detected (the 30 s gate prevents arming).
- **Max 5 bounces** as a hardcoded limit.

---

## C. AeroACARS — what we do and why

### Architecture

Two **separate** sampling loops in the X-Plane and MSFS adapters:

1. **Position streamer** (`spawn_position_streamer` in `lib.rs`) — phase-adaptive 5–30 s, posts to phpVMS
2. **Touchdown sampler** (`spawn_touchdown_sampler` in `lib.rs`) — **fixed 30 Hz**, exists solely to fill `stats.snapshot_buffer`

The touchdown sampler is significantly faster than anything SimCARS or Volanta have — it samples at the same frequency as GEES (the de-facto reference tool for landing-rate measurement in the sim community), not at the SimConnect default cadence.

```rust
fn spawn_touchdown_sampler(app: AppHandle, flight: Arc<ActiveFlight>) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(20)).await;  // 50 Hz target
            // ... read snapshot, push into ring-buffer with cutoff at TOUCHDOWN_BUFFER_SECS
        }
    });
}
```

Effectively we reach 20–30 samples/second (limited by the SimConnect-adapter drain sleep), which over a 5-second look-back yields ~100–150 samples — **more than enough** to catch the real touchdown sub-frame.

### Touchdown detection: AGL-based with arming

Instead of naively listening on the `on_ground edge` (which counts gear-strut oscillations as phantom bounces), we work with two AGL thresholds:

```rust
const BOUNCE_AGL_THRESHOLD_FT: f32 = …;  // armed when the aircraft climbs above it
const BOUNCE_AGL_RETURN_FT: f32 = …;     // bounce counted when it drops back below
const BOUNCE_WINDOW_SECS: i64 = …;       // window in which the drop must count
```

`bounce_armed_above_threshold: bool` in `FlightStats` tracks the edge state between the two thresholds. This lets us prevent:
- **False positives** from gear-strut wobble right after touchdown (which never actually flies above `BOUNCE_AGL_THRESHOLD_FT`)
- **Frame-race problems** of the on_ground flag (on many aircraft profiles it flickers between true/false at touchdown)

### Landing-rate calculation

In `step_flight` (file `lib.rs`):

```rust
// Pick worst V/S from the 5-second look-back buffer around the touchdown tick
let peak_vs = stats.landing_peak_vs_fpm.unwrap_or(0.0);  // most-negative VS observed
let peak_g  = stats.landing_peak_g_force.unwrap_or(0.0);
```

`landing_peak_vs_fpm` is updated **continuously** across the entire 5-second window (more precisely: `TOUCHDOWN_WINDOW_SECS` after the touchdown edge), not "frozen at the first on_ground." This way our score also catches late spikes (the aircraft slamming through again 200 ms after touchdown).

### Classification (5-level)

```
Smooth     ≤ 200 fpm   ≤ 1.20 G   (butter / greaser)
Acceptable ≤ 400 fpm   ≤ 1.40 G   (normal LH FOQA)
Firm       ≤ 600 fpm   ≤ 1.70 G   (firm but accepted)
Hard       ≤ 1000 fpm  ≤ 2.10 G   (FCOM inspection trigger)
Severe     > 1000 fpm  > 2.10 G   (structural concern)
```

Source of the boundaries: compiled from Boeing FCTM (727/737/747 OEM tables), Lufthansa FOQA public specs, and community ACARS conventions (Smartcars / BeatMyLanding / LandingRate.com). See the constants in `client/src-tauri/src/lib.rs:1525..1534`.

Neither Volanta nor SimCARS classify — both just pass raw numbers along. In our case the letter grade plus numeric score (0..100) is written both to the ACARS activity log and persisted to the PIREP custom fields.

### Sideslip / crab reconstruction

We compute the touchdown sideslip (difference between heading and ground track) from the last few samples before the touchdown tick — see `touchdown_sideslip_deg` in `FlightStats`. This is valuable for crosswind assessment; no competitor does it.

### Touchdown profile for PIREP notes

In `touchdown_profile: Vec<TouchdownProfilePoint>` (file `lib.rs`) we store a ±2 s subset of the ring buffer around the touchdown tick. This lets the PIREP builder render a V/S curve directly into the notes — the assessing admins see the exact progression instead of just a number.

---

## D. Where our weaknesses lie (honestly)

1. **Lower AGL threshold than SimCARS' 30 s gate.** We arm earlier → could theoretically count a large hop (>500 ft) on the runway as "flight + landing." In practice this has never been a problem, but we should keep an eye on it.

2. **Helicopters** were completely under-scored until the universal Arrived-fallback logic (v0.1.20) because the normal phase FSM didn't run through. The touchdown sampler ran, but `landing_peak_vs_fpm` wasn't carried into the PIREP notes when the FSM was stuck at TaxiOut. This is fixed.

3. **Touch-and-go** is scored by us as a "bounce without landing again" — we have no explicit T&G detection. SimCARS doesn't either (it wouldn't even arm), nor does Volanta. Low priority.

4. **20–30 Hz instead of true 50 Hz** — limited by the adapter-drain sleep in sim-msfs / sim-xplane. It's still ~30× faster than the competition, but if we ever want true 50 Hz we have to optimize the adapter loop.

5. **We deliberately don't have a random fallback** — in an extreme edge case (peak_vs = None because the logic fails) we show "—" instead of an invented number. More honest, but less PIREP-friendly (no score = no entry).

---

## E. What we could adopt from the competition

### From SimCARS

- **30 s arming gate before landing detection.** Would eliminate false positives on hops. Trade-off: touch-and-go would then no longer be trackable at all. Low value, but nice-to-have.
- **Separate bounce counter per bounce index.** We currently only have a total `bounce_count: u8`. Per-bounce values would allow a nicer "Bounce 1: -380, Bounce 2: -220, Bounce 3: -140 fpm" table in the PIREP.

### From Volanta

- Nothing. Their algorithm is strictly a subset of what we do, less accurate.

---

## F. Reproduction material

Both competitor binaries were pulled from pilot workstations (NOT bundleable, NOT to be committed):

- **SimCARS v1.1.58:** `C:\Program Files\WindowsApps\38636ScottySoftWare.simCARS_1.1.58.0_x64__28p9t23t3wb0g\SimCARSServer\SimCARSServer.exe`
- **Volanta v1.17.2:** `C:\Users\<user>\AppData\Local\Programs\Volanta\resources\app.asar` (extract via `npx @electron/asar`)

Decompiled with:

```bash
# Install once:
dotnet tool install -g ilspycmd --version 8.2.0.7535

# Decompile a .NET assembly to C# source:
ilspycmd /path/to/SimCARSServer.exe -p -o ./out/

# Extract Electron asar:
npx --yes @electron/asar extract /path/to/app.asar ./extracted/
```

Local snapshots live under `.research/` (gitignored).

---

## G. Verdict

We are **not merely "on par with the competition" but measurably more precise**, because:

1. 30 Hz sampler vs ~1 Hz event-driven for the others
2. AGL arming instead of on_ground edge → no phantom bounces from gear strut
3. Peak-refinement window instead of freeze-on-touchdown
4. 5-level classification on V/S **and** G combined — nobody else does that
5. Sideslip + V/S curve in the PIREP

The only point where competitors had a clear advantage (SimCARS' 30 s arming gate against false positives) is a deliberate trade-off decision by us in favor of touch-and-go trackability.

Should the behavior turn out to be prone to false positives in real use, the gate is a ~20-line addition in `step_flight`.

---

## H. Touch-and-Go + Go-Around detection (v0.1.26)

Competitor research (SmartCARS 3, Volanta, vmsACARS) showed clearly:
**nobody** tracks touch-and-go or go-around semantically.
SmartCARS has a boolean variable `IsLanded` that jumps to `true`
after the first on-ground edge and never goes back to `false` —
a T&G is therefore booked like a perfectly normal landing (with the
score of the first touch), and the second touchdown is completely
ignored. Volanta and vmsACARS behave identically.

From v0.1.26, AeroACARS explicitly distinguishes between **touch-and-go**
(touch with climb-out again) and **final landing** (touch with
rollout to standstill), and additionally detects **go-arounds**
(rejected approach WITHOUT ground contact, followed by a climb).

### H.1. Touch-and-go classifier

State on `FlightStats`:

```rust
touchdown_events: Vec<TouchdownEvent>,
touch_and_go_pending_since: Option<DateTime<Utc>>,
```

`TouchdownEvent` contains timestamp, kind (`TouchAndGo` |
`FinalLanding`), peak V/S, peak G, lat/lon and sub_bounces.

Algorithm (in the FSM `FlightPhase::Landing` arm):

1. On the on-ground edge, `landing_at` is stamped — as before.
2. Within `TOUCH_AND_GO_WATCH_SECS` (= 30 s) after
   `landing_at`, the T&G watcher runs in parallel with the score window.
3. Conditions for T&G classification (all fulfilled simultaneously
   for `TOUCH_AND_GO_DWELL_SECS` = 1 s):
   - `altitude_agl_ft > TOUCH_AND_GO_AGL_THRESHOLD_FT` (= 100 ft)
   - `!on_ground`
   - `engines_running > 0`
4. On classification:
   - a `TouchdownEvent { kind: TouchAndGo }` is pushed
   - all `landing_*` fields + `bounce_count` are reset
     (so that the NEXT touchdown gets a fresh score window;
     the T&G doesn't drag the final score down)
   - `next_phase = FlightPhase::Climb` — so that on the next
     approach the normal phase transitions work again
     (Approach → Final → Landing).
   - an entry in `pending_acars_logs` for the streamer:
     `"Touch-and-go #N — V/S … fpm, G …"`

5. If `TOUCH_AND_GO_WATCH_SECS` elapses without a climb-out, the
   stored touchdown is finalized as `kind: FinalLanding`
   (exactly once — guarded via `last().timestamp != touchdown`).

**Trade-off:** The 1-second dwell window is very short, but
justified — the upstream AGL condition (100 ft) reliably filters
out bouncing. Strut oscillation doesn't even come close to 100 ft AGL.

### H.2. Go-around detector

State on `FlightStats`:

```rust
go_around_count: u32,
lowest_agl_during_approach_ft: Option<f32>,
go_around_climb_pending_since: Option<DateTime<Utc>>,
```

Helpers (in `lib.rs`):

- `update_lowest_approach_agl(stats, snap)` — maintains the minimum
  across the Approach/Final phase. Negative glitches are ignored
  (terrain-mesh hiccups near mountains would otherwise poison the
  minimum and make every later sample look like a 200 ft
  climb-back).
- `check_go_around(stats, snap, now) -> Option<FlightPhase>` —
  returns `Some(FlightPhase::Climb)` when a go-around has been
  classified. Called in the `Approach` and `Final` arms of the FSM.

Conditions (all simultaneously for `GO_AROUND_DWELL_SECS` = 8 s):

- `lowest_agl_during_approach_ft <= 1500.0` — only if the pilot
  actually descended to approach-typical altitudes; a pilot who
  catches the GS from above should not trigger a GA just because
  they briefly climb during levelling.
- `agl > lowest + GO_AROUND_AGL_RECOVERY_FT` (= +200 ft)
- `vertical_speed_fpm > GO_AROUND_MIN_VS_FPM` (= +500 fpm)
- `!on_ground`
- `engines_running > 0`

On classification:

- `go_around_count++`
- `lowest_agl_during_approach_ft = None` (the next approach
  starts with a fresh minimum)
- an entry in `pending_acars_logs` for the streamer
- `next_phase = FlightPhase::Climb` — the FSM returns to the
  Climb arm; the next descent triggers Descent → Approach → Final
  normally again.

### H.3. UI visibility

- **Cockpit InfoStrip** (`info_strip.trip` group): shows cells
  `Touch-and-Go: N` and `Go-arounds: N` — only if N > 0, otherwise
  the trip group stays at the original three cells.
- **PIREP custom fields** (`build_pirep_fields`):
  - `Touchdowns: N (X T&G + final)` — only if T&G > 0 or more
    than one touchdown total.
  - `Go-Arounds: N` — only if N > 0.
- **PIREP notes** (`build_pirep_notes`): section `TOUCHDOWNS` with
  one line per event (`#1 T&G   12:34:56  V/S -350 fpm · G
  1.18 · bounces 0`), and section `GO-AROUNDS` with the counter.
- **ACARS log lines** (streamer drains `pending_acars_logs` every
  tick): appear on the phpVMS PIREP detail page in
  chronological order.

### H.4. Tests

`#[cfg(test)] mod touch_and_go_go_around_tests` in `lib.rs`
covers 9 scenarios:

- `lowest_agl_starts_unset_and_takes_first_sample`
- `lowest_agl_only_decreases_never_increases`
- `lowest_agl_ignores_negative_glitch_samples`
- `go_around_does_nothing_without_a_prior_minimum`
- `go_around_fires_after_dwell_seconds`
- `go_around_does_not_fire_below_recovery_threshold`
- `go_around_dwell_resets_when_conditions_break`
- `go_around_does_not_fire_when_aircraft_caught_glideslope_high`
- `go_around_requires_engines_running`

An end-to-end test of the FSM is not covered — the helpers are the
testable surface; the integration comes from real in-sim flight
against a dev-server PIREP.
