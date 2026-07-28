# Client JSONL log file — format reference

> **File:** `<app_data_dir>/flight_logs/<pirep_id>.jsonl` (one line per event, append-only)
> **Module:** [`client/src-tauri/crates/recorder/src/lib.rs`](../client/src-tauri/crates/recorder/src/lib.rs)
> **Upload:** from v0.5.23, automatically sent to aeroacars-live after a successful PIREP file (`POST /api/flight-logs/upload`)

---

## Event types (FlightLogEvent enum)

Each JSONL line is a `FlightLogEvent` — a tagged union with `"type"` as the discriminator.

| Type | When | Fields |
|---|---|---|
| `flight_started` | Fresh prefile or adoption of an existing PIREP | `timestamp`, `pirep_id`, `airline_icao`, `flight_number`, `dpt_airport`, `arr_airport` |
| `flight_resumed` | Tauri app restart with an active flight | `timestamp`, `pirep_id`, `age_minutes` |
| `phase_changed` | FSM transition (e.g. CRUISE → DESCENT) | `timestamp`, `from`, `to`, `altitude_msl_ft`, `groundspeed_kt`, `altitude_agl_ft` |
| `position` | Per streamer tick (3–30 s depending on phase) | `timestamp`, `snapshot` (= **complete SimSnapshot**, ≈80 fields) |
| `activity` | User-visible log line (activity feed) | `timestamp`, `level`, `message`, `detail` |
| `landing_scored` | Touchdown analyzer done (= final score) | `timestamp`, `score`, `peak_vs_fpm`, `peak_g_force`, `bounce_count` |
| `flight_ended` | PIREP filed/manual/cancelled — closes the log | `timestamp`, `pirep_id`, `outcome` (filed/manual/cancelled/forgotten) |

---

## SimSnapshot — the ≈80 telemetry fields per position event

[`client/src-tauri/crates/sim-core/src/lib.rs`](../client/src-tauri/crates/sim-core/src/lib.rs)

### Position
- `lat`, `lon` (f64)
- `altitude_msl_ft`, `altitude_agl_ft` (f64)

### Attitude / heading
- `heading_deg_true`, `heading_deg_magnetic` (f32)
- `pitch_deg`, `bank_deg` (f32)
- `vertical_speed_fpm` (f32)

### Speeds
- `groundspeed_kt`, `indicated_airspeed_kt`, `true_airspeed_kt` (f32)
- `mach` (Option<f32>)

### Velocity (body axes — for X-Plane touchdown analysis)
- `velocity_body_x_fps`, `velocity_body_z_fps` (Option<f32>)
- `aircraft_wind_x_kt`, `aircraft_wind_z_kt` (Option<f32>)

### Forces / sim state
- `g_force` (f32)
- `on_ground` (bool)
- `gear_normal_force_n` (Option<f32>) — preferred by X-Plane for touchdown detection
- `parking_brake`, `stall_warning`, `overspeed_warning` (bool)
- `paused`, `slew_mode` (bool)
- `simulation_rate` (f32)

### Configuration
- `gear_position`, `flaps_position` (f32, 0.0–1.0)
- `spoilers_handle_position` (Option<f32>)
- `spoilers_armed` (Option<bool>)
- `pushback_state` (Option<u8>)

### Fuel / weight
- `fuel_total_kg`, `fuel_used_kg` (f32)
- `fuel_flow_kg_per_h` (Option<f32>)
- `zfw_kg`, `payload_kg`, `total_weight_kg`, `empty_weight_kg` (Option<f32>)

### Touchdown snapshot (set in the touchdown frame)
- `touchdown_vs_fpm`, `touchdown_pitch_deg`, `touchdown_bank_deg` (Option<f32>)
- `touchdown_heading_mag_deg` (Option<f32>)
- `touchdown_lat`, `touchdown_lon` (Option<f64>)

### Environment
- `wind_direction_deg`, `wind_speed_kt` (Option<f32>)
- `qnh_hpa`, `outside_air_temp_c`, `total_air_temp_c` (Option<f32>)

### Aircraft identity
- `aircraft_title` (Option<String>)
- `aircraft_icao`, `aircraft_registration` (Option<String>)
- `sim_version` (Option<String>)

### Radios
- `transponder_code` (Option<u16>)
- `com1_mhz`, `com2_mhz`, `nav1_mhz`, `nav2_mhz` (Option<f32>)

### Lights
- `light_landing`, `light_beacon`, `light_strobe`, `light_taxi`, `light_nav`, `light_logo` (Option<bool>)
- `strobe_state` (Option<u8>) — 0=off, 1=on, 2=auto

### Autopilot master + modes
- `autopilot_master`, `autopilot_heading`, `autopilot_altitude`, `autopilot_nav`, `autopilot_approach` (Option<bool>)

### FCU/MCP setpoints
- `fcu_selected_altitude_ft`, `fcu_selected_heading_deg`, `fcu_selected_speed_kt`, `fcu_selected_vs_fpm` (Option<i32>)

### Misc
- `autobrake` (Option<String>)
- `apu_switch` (Option<bool>)
- `apu_pct_rpm` (Option<f32>)
- `seatbelts_sign`, `no_smoking_sign` (Option<u8>)

---

## What the server (via MQTT) does **NOT** receive

These fields are **only** in the client JSONL — the MQTT stream doesn't have them:

| Area | Fields |
|---|---|
| Velocity body axes | `velocity_body_x_fps`, `velocity_body_z_fps`, `aircraft_wind_x_kt`, `aircraft_wind_z_kt` |
| Touchdown-frame snapshot | `touchdown_pitch_deg`, `touchdown_bank_deg`, `touchdown_heading_mag_deg`, `touchdown_lat`, `touchdown_lon` |
| Speeds (extended) | `true_airspeed_kt`, `mach` |
| Weight | `zfw_kg`, `payload_kg`, `empty_weight_kg`, `total_weight_kg` |
| Environment | `total_air_temp_c` |
| Identity | `aircraft_title`, `aircraft_registration`, `sim_version` |
| Radios | `transponder_code`, `com1_mhz`, `com2_mhz`, `nav1_mhz`, `nav2_mhz` |
| Lights (extended) | `light_taxi`, `light_nav`, `light_logo`, `strobe_state` |
| Autopilot modes | `autopilot_heading`, `autopilot_altitude`, `autopilot_nav`, `autopilot_approach` |
| FCU setpoints | `fcu_selected_altitude_ft`, `fcu_selected_heading_deg`, `fcu_selected_speed_kt`, `fcu_selected_vs_fpm` |
| Misc | `autobrake`, `apu_switch`, `apu_pct_rpm`, `seatbelts_sign`, `no_smoking_sign`, `pushback_state`, `gear_normal_force_n`, `simulation_rate` |
| Forces (extended) | `parking_brake`, weather event flags |
| Ground truth | All `Option<>` fields the sim doesn't provide (trim/engines/etc. extendable via SimSnapshot in future) |
| **Activity log** | **Entirely** — the server has none of it |
| **PhaseChanged context** | Only the phase string, NOT the `from` phase or `altitude_msl_ft`/`groundspeed_kt`/`altitude_agl_ft` at the time |

---

## Identified gaps in the client log itself (= patches the client still needs)

The **JSONL log is missing** these events, which the client does generate but only pushes to the server via MQTT:

| Event | Where currently | Gap in the log |
|---|---|---|
| `block` snapshot | `aeroacars/<va>/<pilot>/block` MQTT | JSONL has `block_fuel_kg`/`planned_burn_kg` etc. only in the final `flight_ended` indirection (= via `flight.stats`); no dedicated block event |
| `takeoff` snapshot | `aeroacars/<va>/<pilot>/takeoff` MQTT | As above — no dedicated takeoff snapshot in the JSONL |
| Touchdown events | `aeroacars/<va>/<pilot>/touchdown` MQTT | Only `landing_scored` (= final aggregate). Multi-touchdown patterns (touch-and-go training) can't be differentiated |
| PIREP body | `aeroacars/<va>/<pilot>/pirep` MQTT | JSONL has `flight_ended { outcome }` but NOT what was actually filed (distance/time/notes/custom fields) |
| `client_version` | nowhere | Should be included in every position snapshot or at least in `flight_started` |

**Proposed extension of the FlightLogEvent enum:**

```rust
BlockSnapshot {
    timestamp: DateTime<Utc>,
    block_fuel_kg: Option<f32>,
    planned_burn_kg: Option<f32>,
    planned_tow_kg: Option<f32>,
    /* all fields from aeroacars-mqtt::BlockPayload */
},
TakeoffSnapshot {
    timestamp: DateTime<Utc>,
    /* all fields from aeroacars-mqtt::TakeoffPayload */
},
TouchdownEvent {
    timestamp: DateTime<Utc>,
    vs_fpm: i32,
    g_force: f32,
    airport: Option<String>,
    runway: Option<String>,
    bounce: bool,
    score: Option<i32>,
},
PirepFiled {
    timestamp: DateTime<Utc>,
    pirep_id: String,
    flight_time_min: Option<i32>,
    distance_nm: Option<f32>,
    fuel_used_kg: Option<f32>,
    landing_score: Option<i32>,
    custom_fields_count: usize,
},
ClientInfo {
    timestamp: DateTime<Utc>,
    version: String, // env!("CARGO_PKG_VERSION")
    os: String,
    sim: String,    // "msfs" / "xplane" / "unknown"
},
```

---

## Upload protocol (v0.5.23+)

**Trigger:** after a successful `client.file_pirep()` in `flight_end()` (lib.rs).

**Endpoint:** `POST https://live.kant.ovh/api/flight-logs/upload`

**Headers:**
- `Authorization: Basic <base64(username:password)>` — same credential as the MQTT login
- `X-Pirep-Id: <pirep_id>` — the server validates that the session belongs to the authenticated pilot
- `Content-Type: application/gzip`

**Body:** raw gzip stream of the `<app_data>/flight_logs/<pirep_id>.jsonl` file.

**Bandwidth:** a typical 2h flight ≈ 2–5 MB raw JSONL → ≈ 300–800 KB gzip. A single POST, fire-and-forget.

**Server storage location:** `/var/lib/aeroacars-recorder/flight-logs/<va>/<pilot>/<pirep_id>.jsonl.gz`

**Auth:** validated against the `provisioned_pilots` table (= Mosquitto credential pool).

**Authorization:** a pilot can only upload logs for their own sessions — the server checks `findSessionByPirepForPilot(va, pilot, pirep_id)`.

**Idempotency:** a re-upload with the same `pirep_id` overwrites — no obstacle in case of corruption / retry.

**Failure modes:** all non-fatal — the pilot isn't blocked, the JSONL stays available locally (see `<app_data>/flight_logs/`).

---

## Download (web app / VA owner)

**UI:** PilotHistory → session-detail card → "📥 Client log (XXX KB)" button.

**Visibility:** only if `session.client_log_uploaded_at != null`. Otherwise a greyed-out "📥 No log" with a tooltip.

**Endpoint:** `GET /api/sessions/:id/client-log` (admin cookie auth).

---

# Touchdown-detection algorithms — reference

> **Goal:** cleanly define what counts as touchdown V/S for each sim + which edge cases exist + how to forensically resolve disagreements between algorithms.
> **Code:** [`client/src-tauri/src/lib.rs`](../client/src-tauri/src/lib.rs) ~line 8200–8400 (`step_flight` touchdown arm)
> **Telemetry from v0.5.23:** TouchdownPayload sends `simulator` + `vs_estimate_xp_fpm` + `vs_estimate_msfs_fpm` + `vs_source` + `gear_force_peak_n` + `estimate_window_ms` + `estimate_sample_count` to aeroacars-live so the VA owner sees algorithm comparisons in the Diagnostics tab + LandingAnalysis modal.

## Decision tree per simulator

### MSFS (Microsoft Flight Simulator 2020/2024)

Priority chain — the first non-null value wins:

```
1. snap.touchdown_vs_fpm                    →  vs_source = "msfs_simvar_latched"
   (PLANE TOUCHDOWN NORMAL VELOCITY SimVar — frame-accurate in the touchdown frame)
2. agl_estimate_msfs.fpm                    →  vs_source = "agl_estimate_msfs"
   (time-tier 750ms/1s/1.5s/2s/3s/12s window progression with min-sample guards)
3. buffered_vs_min                          →  vs_source = "buffer_min"
   (last-resort buffer-window scan, AGL ≤ 250 ft filter)
4. (all null) →  vs_source = "fallback_zero"
```

**Explicitly NOT for MSFS:**
- `sampler_touchdown_vs_fpm` — the gear-contact rebound spike contaminates the value (v0.5.12 validated against 11 real pilot flights)
- `low_agl_vs_min_fpm` — same risk
- Lua-style 30-sample estimator — X-Plane-only by design

**Known edge cases:**
- *2026-05-07 LH595 DNAA→EDDF:* actually -419/-560 fpm (confirmed by Volanta+LHA), reported -1173 fpm. Bug class: cross-contamination from a refactor that should only have affected X-Plane. Fixed in v0.5.12.
- *Phase H.4 era:* "0-distance / 0 fuel" PIREPs on a sim crash mid-flight. The manual PIREP path bypasses this.

### X-Plane (X-Plane 11/12)

Priority chain:

```
1. agl_estimate_xp.fpm                      →  vs_source = "agl_estimate_xp"
   (Lua-style 30-sample adaptive AGL-Δ — LandingRate-1.lua algorithm,
    Volanta-aligned. Window size adaptive: high-fps ≈ 0.5s, low-fps ≈ 2-3s)
2. sampler_touchdown_vs_fpm                 →  vs_source = "sampler_gear_force"
   (sampler-side touchdown edge at `gear_normal_force_n > 1.0 N`,
    50 Hz within 20 ms edge detection)
3. buffered_vs_min                          →  vs_source = "buffer_min"
4. low_agl_vs_min_fpm                       →  vs_source = "low_agl_vs_min"
   (AGL ≤ 250 ft approach tracker, reset on approach entry for go-arounds)
5. (all null) →  vs_source = "fallback_zero"
```

**Trigger for the sampler path:** `gear_normal_force_n > 1.0 N`. Real touchdowns spike instantly to several kN (a 60–300 t airliner at 1.0 g braking moment ≥ 588 kN); 1.0 N is a float-noise filter.

**Known edge cases:**
- *2026-05-07 MYNN→MBGT:* the X-Plane Lua tool said 273 fpm, AeroACARS said -394 fpm (~44% too high). Cause: the time-tier estimator was too rigid at a low RREF rate. Fixed in v0.5.13 (switched to Lua 30-sample adaptive).
- *2026-05-06 DAL93 EDDB→KJFK:* real -300 fpm, AeroACARS scored +35 fpm (smooth) because the streamer woke 5 s after touchdown and the buffer then contained only rollout samples with V/S≈0. Fixed in v0.4.4 (sampler-side edge detection at 50 Hz).

## Forensic workflow for the VA owner

For a suspicious touchdown:

1. **Web app → Touchdowns → filter "⚠ Disagreement"** shows all touchdowns where `|vs_estimate_xp − vs_estimate_msfs| > 50 fpm`. Sort by `|Δ|` desc.

2. **Click on a row** → LandingAnalysis modal → card "🔬 Algorithm forensics" shows:
   - Final V/S + which path won (`vs_source`)
   - Both estimator results separately
   - Window size + sample count (= confidence indicator)
   - Gear-force peak (X-Plane only)

3. **If |Δ| > 100 fpm and both values look plausible** → edge case, worth a look.

4. **If a v0.5.23 client log was uploaded:** "📥 Client log" in PilotHistory → view the JSONL. Look for:
   - `phase_changed` events (was there a LANDING phase? Multiple touch-and-gos?)
   - `position` events around the touchdown frame: raw AGL trajectories, `gear_normal_force_n`, `on_ground` edges
   - `activity` events (user-visible log lines — e.g. "SimConnect reconnect" could explain a sample gap)

5. **Test a patch in [`lib.rs`](../client/src-tauri/src/lib.rs)** once it's clear which heuristic is stuck. Validate test cases with stored JSONLs (see `tests/` in the recorder crate).

## Server-side forensic aggregates

`GET /api/touchdowns/forensik?days=30` returns, per simulator:
- `count` — number of touchdowns in the period
- `avg_vs_fpm` — mean V/S
- `hard_landings` — count with V/S < -600 fpm
- `disagreements` — count with `|xp − msfs| > 50 fpm`
- `avg_disagreement_fpm` — mean delta between the two algorithms

Shown automatically at the top of the Touchdowns tab (card "🔬 Touchdown forensics by simulator"). A high `disagreements` share per sim = a systematic FSM edge case not yet fixed.

## SQL examples for deeper drill-down

```sql
-- Top 20 disagreements of the last 30 days
SELECT id, ts, simulator, vs_fpm, vs_estimate_xp_fpm, vs_estimate_msfs_fpm,
       ABS(vs_estimate_xp_fpm - vs_estimate_msfs_fpm) AS delta,
       vs_source, airport
FROM touchdowns
WHERE ts >= strftime('%s','now','-30 days')*1000
  AND vs_estimate_xp_fpm IS NOT NULL
  AND vs_estimate_msfs_fpm IS NOT NULL
ORDER BY delta DESC
LIMIT 20;

-- vs_source distribution per sim (which path wins most often?)
SELECT simulator, vs_source, COUNT(*) AS n
FROM touchdowns
WHERE ts >= strftime('%s','now','-30 days')*1000
GROUP BY simulator, vs_source
ORDER BY simulator, n DESC;

-- Window confidence: how many touchdowns had <10 samples in the computation window?
SELECT simulator, COUNT(*) AS sparse
FROM touchdowns
WHERE estimate_sample_count < 10
  AND ts >= strftime('%s','now','-30 days')*1000
GROUP BY simulator;
```
