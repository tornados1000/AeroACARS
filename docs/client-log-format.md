# Client-JSONL-Logfile — Format-Referenz

> **Datei:** `<app_data_dir>/flight_logs/<pirep_id>.jsonl` (eine Zeile pro Event, append-only)
> **Modul:** [`client/src-tauri/crates/recorder/src/lib.rs`](../client/src-tauri/crates/recorder/src/lib.rs)
> **Upload:** ab v0.5.23 nach erfolgreichem PIREP-File automatisch an aeroacars-live (`POST /api/flight-logs/upload`)

---

## Event-Typen (FlightLogEvent enum)

Jede JSONL-Zeile ist ein `FlightLogEvent` — Tagged Union mit `"type"` als Diskriminator.

| Type | Wann | Felder |
|---|---|---|
| `flight_started` | Fresh Prefile oder Adopt eines existierenden PIREPs | `timestamp`, `pirep_id`, `airline_icao`, `flight_number`, `dpt_airport`, `arr_airport` |
| `flight_resumed` | Tauri-App-Restart mit aktivem Flug | `timestamp`, `pirep_id`, `age_minutes` |
| `phase_changed` | FSM-Transition (z.B. CRUISE → DESCENT) | `timestamp`, `from`, `to`, `altitude_msl_ft`, `groundspeed_kt`, `altitude_agl_ft` |
| `position` | Pro Streamer-Tick (3-30 s je nach Phase) | `timestamp`, `snapshot` (= **kompletter SimSnapshot**, ≈80 Felder) |
| `activity` | User-sichtbare Log-Zeile (Aktivitaeten-Feed) | `timestamp`, `level`, `message`, `detail` |
| `landing_scored` | Touchdown-Analyzer fertig (= finaler Score) | `timestamp`, `score`, `peak_vs_fpm`, `peak_g_force`, `bounce_count` |
| `flight_ended` | PIREP filed/manual/cancelled — schliesst Log | `timestamp`, `pirep_id`, `outcome` (filed/manual/cancelled/forgotten) |

---

## SimSnapshot — die ≈80 Telemetrie-Felder pro Position-Event

[`client/src-tauri/crates/sim-core/src/lib.rs`](../client/src-tauri/crates/sim-core/src/lib.rs)

### Position
- `lat`, `lon` (f64)
- `altitude_msl_ft`, `altitude_agl_ft` (f64)

### Attitude / Heading
- `heading_deg_true`, `heading_deg_magnetic` (f32)
- `pitch_deg`, `bank_deg` (f32)
- `vertical_speed_fpm` (f32)

### Speeds
- `groundspeed_kt`, `indicated_airspeed_kt`, `true_airspeed_kt` (f32)
- `mach` (Option<f32>)

### Velocity (body axes — fuer X-Plane Touchdown-Analyse)
- `velocity_body_x_fps`, `velocity_body_z_fps` (Option<f32>)
- `aircraft_wind_x_kt`, `aircraft_wind_z_kt` (Option<f32>)

### Forces / Sim-State
- `g_force` (f32)
- `on_ground` (bool)
- `gear_normal_force_n` (Option<f32>) — X-Plane bevorzugt fuer Touchdown-Detection
- `parking_brake`, `stall_warning`, `overspeed_warning` (bool)
- `paused`, `slew_mode` (bool)
- `simulation_rate` (f32)

### Configuration
- `gear_position`, `flaps_position` (f32, 0.0-1.0)
- `spoilers_handle_position` (Option<f32>)
- `spoilers_armed` (Option<bool>)
- `pushback_state` (Option<u8>)

### Fuel / Weight
- `fuel_total_kg`, `fuel_used_kg` (f32)
- `fuel_flow_kg_per_h` (Option<f32>)
- `zfw_kg`, `payload_kg`, `total_weight_kg`, `empty_weight_kg` (Option<f32>)

### Touchdown-Snapshot (gesetzt im Touchdown-Frame)
- `touchdown_vs_fpm`, `touchdown_pitch_deg`, `touchdown_bank_deg` (Option<f32>)
- `touchdown_heading_mag_deg` (Option<f32>)
- `touchdown_lat`, `touchdown_lon` (Option<f64>)

### Environment
- `wind_direction_deg`, `wind_speed_kt` (Option<f32>)
- `qnh_hpa`, `outside_air_temp_c`, `total_air_temp_c` (Option<f32>)

### Aircraft Identity
- `aircraft_title` (Option<String>)
- `aircraft_icao`, `aircraft_registration` (Option<String>)
- `sim_version` (Option<String>)

### Radios
- `transponder_code` (Option<u16>)
- `com1_mhz`, `com2_mhz`, `nav1_mhz`, `nav2_mhz` (Option<f32>)

### Lights
- `light_landing`, `light_beacon`, `light_strobe`, `light_taxi`, `light_nav`, `light_logo` (Option<bool>)
- `strobe_state` (Option<u8>) — 0=off, 1=on, 2=auto

### Autopilot Master + Modes
- `autopilot_master`, `autopilot_heading`, `autopilot_altitude`, `autopilot_nav`, `autopilot_approach` (Option<bool>)

### FCU/MCP Setpoints
- `fcu_selected_altitude_ft`, `fcu_selected_heading_deg`, `fcu_selected_speed_kt`, `fcu_selected_vs_fpm` (Option<i32>)

### Misc
- `autobrake` (Option<String>)
- `apu_switch` (Option<bool>)
- `apu_pct_rpm` (Option<f32>)
- `seatbelts_sign`, `no_smoking_sign` (Option<u8>)

---

## Was der Server (via MQTT) **NICHT** bekommt

Diese Felder sind **nur** im Client-JSONL — der MQTT-Stream hat sie nicht:

| Bereich | Felder |
|---|---|
| Velocity-Body-Achsen | `velocity_body_x_fps`, `velocity_body_z_fps`, `aircraft_wind_x_kt`, `aircraft_wind_z_kt` |
| Touchdown-Frame-Snapshot | `touchdown_pitch_deg`, `touchdown_bank_deg`, `touchdown_heading_mag_deg`, `touchdown_lat`, `touchdown_lon` |
| Speeds (erweitert) | `true_airspeed_kt`, `mach` |
| Weight | `zfw_kg`, `payload_kg`, `empty_weight_kg`, `total_weight_kg` |
| Environment | `total_air_temp_c` |
| Identity | `aircraft_title`, `aircraft_registration`, `sim_version` |
| Radios | `transponder_code`, `com1_mhz`, `com2_mhz`, `nav1_mhz`, `nav2_mhz` |
| Lights (erweitert) | `light_taxi`, `light_nav`, `light_logo`, `strobe_state` |
| Autopilot Modes | `autopilot_heading`, `autopilot_altitude`, `autopilot_nav`, `autopilot_approach` |
| FCU Setpoints | `fcu_selected_altitude_ft`, `fcu_selected_heading_deg`, `fcu_selected_speed_kt`, `fcu_selected_vs_fpm` |
| Misc | `autobrake`, `apu_switch`, `apu_pct_rpm`, `seatbelts_sign`, `no_smoking_sign`, `pushback_state`, `gear_normal_force_n`, `simulation_rate` |
| Forces (erweitert) | `parking_brake`, weather event flags |
| Ground-Truth | Alle `Option<>`-Felder die Sim nicht liefert (Trim/Engines/usw via SimSnapshot zukuenftig erweiterbar) |
| **Activity-Log** | **Komplett** — Server hat nichts davon |
| **PhaseChanged-Kontext** | Nur Phase-String, NICHT `from`-Phase oder `altitude_msl_ft`/`groundspeed_kt`/`altitude_agl_ft` zum Zeitpunkt |

---

## Identifizierte Gaps im Client-Log selbst (= Patches die der Client noch braucht)

Der **JSONL-Log fehlt** diese Events die der Client zwar generiert, aber nur via MQTT auf den Server schiebt:

| Event | Wo aktuell | Gap im Log |
|---|---|---|
| `block`-Snapshot | `aeroacars/<va>/<pilot>/block` MQTT | JSONL hat `block_fuel_kg`/`planned_burn_kg` etc. nur in der finalen `flight_ended`-Indirektion (= via `flight.stats`); kein dediziertes Block-Event |
| `takeoff`-Snapshot | `aeroacars/<va>/<pilot>/takeoff` MQTT | Wie oben — kein dedizierter Takeoff-Snapshot im JSONL |
| Touchdown-Events | `aeroacars/<va>/<pilot>/touchdown` MQTT | Nur `landing_scored` (= finales Aggregat). Multi-Touchdown-Pattern (Touch-and-Go-Training) nicht differenzierbar |
| PIREP-Body | `aeroacars/<va>/<pilot>/pirep` MQTT | JSONL hat `flight_ended { outcome }` aber NICHT was tatsaechlich gefilet wurde (Distanz/Zeit/Notes/Custom-Fields) |
| `client_version` | nirgends | Sollte in jedem Position-Snapshot oder mindestens im `flight_started` mit drin sein |

**Vorgeschlagene Erweiterung des FlightLogEvent-Enums:**

```rust
BlockSnapshot {
    timestamp: DateTime<Utc>,
    block_fuel_kg: Option<f32>,
    planned_burn_kg: Option<f32>,
    planned_tow_kg: Option<f32>,
    /* alle Felder aus aeroacars-mqtt::BlockPayload */
},
TakeoffSnapshot {
    timestamp: DateTime<Utc>,
    /* alle Felder aus aeroacars-mqtt::TakeoffPayload */
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

## Upload-Protokoll (v0.5.23+)

**Trigger:** nach erfolgreichem `client.file_pirep()` in `flight_end()` (lib.rs).

**Endpoint:** `POST https://live.kant.ovh/api/flight-logs/upload`

**Headers:**
- `Authorization: Basic <base64(username:password)>` — gleiche Cred wie MQTT-Login
- `X-Pirep-Id: <pirep_id>` — Server validiert dass Session zu authenticated Pilot gehoert
- `Content-Type: application/gzip`

**Body:** raw gzip-Stream der `<app_data>/flight_logs/<pirep_id>.jsonl`-Datei.

**Bandwidth:** typischer 2h-Flug ≈ 2-5 MB raw JSONL → ≈ 300-800 KB gzip. Einmaliger POST, fire-and-forget.

**Server-Speicherort:** `/var/lib/aeroacars-recorder/flight-logs/<va>/<pilot>/<pirep_id>.jsonl.gz`

**Auth:** validiert gegen `provisioned_pilots`-Tabelle (= Mosquitto-Cred-Pool).

**Authorization:** Pilot kann nur Logs zu seinen eigenen Sessions hochladen — Server prueft `findSessionByPirepForPilot(va, pilot, pirep_id)`.

**Idempotency:** Re-Upload mit gleicher `pirep_id` ueberschreibt — bei Korruption / Retry kein Hindernis.

**Failure-Modi:** alle non-fatal — Pilot wird nicht blockiert, JSONL bleibt lokal verfuegbar (siehe `<app_data>/flight_logs/`).

---

## Download (Webapp / VA-Owner)

**UI:** PilotHistory → Session-Detail-Card → "📥 Client-Log (XXX KB)" Button.

**Sichtbarkeit:** nur wenn `session.client_log_uploaded_at != null`. Sonst greyed-out "📥 Kein Log" mit Tooltip.

**Endpoint:** `GET /api/sessions/:id/client-log` (Admin-Cookie-Auth).

---

# Touchdown-Detection-Algorithmen — Referenz

> **Ziel:** sauber definieren was als Touchdown-V/S für jeden Sim gilt + welche Edge-Cases existieren + wie man Disagreements zwischen Algorithmen forensisch löst.
> **Code:** [`client/src-tauri/src/lib.rs`](../client/src-tauri/src/lib.rs) ~Zeile 8200–8400 (`step_flight` touchdown-arm)
> **Telemetrie ab v0.5.23:** TouchdownPayload schickt `simulator` + `vs_estimate_xp_fpm` + `vs_estimate_msfs_fpm` + `vs_source` + `gear_force_peak_n` + `estimate_window_ms` + `estimate_sample_count` an aeroacars-live damit der VA-Owner Algorithmen-Vergleiche im Diagnostics-Tab + LandingAnalysis-Modal sieht.

## Decision-Tree pro Simulator

### MSFS (Microsoft Flight Simulator 2020/2024)

Priority-Chain — der erste non-null Wert gewinnt:

```
1. snap.touchdown_vs_fpm                    →  vs_source = "msfs_simvar_latched"
   (PLANE TOUCHDOWN NORMAL VELOCITY SimVar — frame-genau im Touchdown-Frame)
2. agl_estimate_msfs.fpm                    →  vs_source = "agl_estimate_msfs"
   (Time-Tier 750ms/1s/1.5s/2s/3s/12s window-progression mit Min-Sample-Guards)
3. buffered_vs_min                          →  vs_source = "buffer_min"
   (Last-Resort Buffer-Window-Scan, AGL ≤ 250 ft Filter)
4. (alle null) →  vs_source = "fallback_zero"
```

**Explizit NICHT für MSFS:**
- `sampler_touchdown_vs_fpm` — gear-contact-Rebound-Spike kontaminiert den Wert (v0.5.12 validiert gegen 11 reale Pilot-Flüge)
- `low_agl_vs_min_fpm` — gleiches Risiko
- Lua-Style 30-Sample-Schaetzer — X-Plane-only by design

**Bekannte Edge-Cases:**
- *2026-05-07 LH595 DNAA→EDDF:* tatsächlich -419/-560 fpm (Volanta+LHA bestätigt), gemeldet -1173 fpm. Bug-Klasse: Cross-Contamination aus Refactor der nur X-Plane betreffen sollte. Behoben in v0.5.12.
- *Phase H.4-Zeit:* "0-distance / 0 fuel" PIREPs bei Sim-Crash mid-flight. Manueller PIREP-Pfad umgeht das.

### X-Plane (X-Plane 11/12)

Priority-Chain:

```
1. agl_estimate_xp.fpm                      →  vs_source = "agl_estimate_xp"
   (Lua-Style 30-Sample adaptive AGL-Δ — LandingRate-1.lua-Algorithmus,
    Volanta-aligned. Window-Groesse adaptive: high-fps ≈ 0.5s, low-fps ≈ 2-3s)
2. sampler_touchdown_vs_fpm                 →  vs_source = "sampler_gear_force"
   (Sampler-side Touchdown-Edge bei `gear_normal_force_n > 1.0 N`,
    50 Hz innerhalb 20 ms Edge-Detection)
3. buffered_vs_min                          →  vs_source = "buffer_min"
4. low_agl_vs_min_fpm                       →  vs_source = "low_agl_vs_min"
   (AGL ≤ 250 ft Approach-Tracker, Reset bei Approach-Entry für Go-Arounds)
5. (alle null) →  vs_source = "fallback_zero"
```

**Trigger für sampler-Pfad:** `gear_normal_force_n > 1.0 N`. Echte Touchdowns gehen blitzartig auf mehrere kN (60-300 t Airliner mit 1.0 g Bremsmoment ≥ 588 kN), 1.0 N ist Float-Noise-Filter.

**Bekannte Edge-Cases:**
- *2026-05-07 MYNN→MBGT:* X-Plane Lua-Tool sagte 273 fpm, AeroACARS sagte -394 fpm (~44% zu hoch). Ursache: Time-Tier-Estimator zu starr bei niedriger RREF-Rate. Behoben in v0.5.13 (Umstellung auf Lua-30-Sample-adaptive).
- *2026-05-06 DAL93 EDDB→KJFK:* echter -300 fpm, AeroACARS scorte +35 fpm (smooth) weil Streamer 5 s nach Touchdown wachte und Buffer dann nur Rollout-Samples mit V/S≈0 enthielt. Behoben in v0.4.4 (Sampler-side Edge-Detection bei 50 Hz).

## Forensik-Workflow für VA-Owner

Bei verdächtigem Touchdown:

1. **Webapp → Touchdowns → Filter „⚠ Disagreement"** zeigt alle Touchdowns wo `|vs_estimate_xp − vs_estimate_msfs| > 50 fpm`. Sortier nach `|Δ|` desc.

2. **Klick auf Row** → LandingAnalysis-Modal → Card „🔬 Algorithmen-Forensik" zeigt:
   - Final V/S + welcher Pfad gewann (`vs_source`)
   - Beide Schaetzer-Ergebnisse separat
   - Window-Groesse + Sample-Count (= Konfidenz-Indikator)
   - Gear-Force-Peak (X-Plane only)

3. **Wenn |Δ| > 100 fpm und beide Werte plausibel aussehen** → Edge-Case, lohnt sich anzuschauen.

4. **Falls v0.5.23 Client-Log hochgeladen:** „📥 Client-Log" in PilotHistory → JSONL anschauen. Suche nach:
   - `phase_changed` Events (gab's eine LANDING-Phase? Mehrfache Touch-and-Go?)
   - `position`-Events um den Touchdown-Frame: rohe AGL-Verläufe, `gear_normal_force_n`, `on_ground`-Flanken
   - `activity` Events (User-sichtbare Log-Zeilen — z.B. „SimConnect-Reconnect" könnte Sample-Lücke erklären)

5. **Patch in [`lib.rs`](../client/src-tauri/src/lib.rs) testen** wenn klar ist welche Heuristik klemmt. Test-Cases mit gespeicherten JSONLs validieren (siehe `tests/` im recorder-Crate).

## Server-seitige Forensik-Aggregate

`GET /api/touchdowns/forensik?days=30` liefert pro Simulator:
- `count` — Anzahl Touchdowns im Zeitraum
- `avg_vs_fpm` — Mittel-V/S
- `hard_landings` — Anzahl mit V/S < -600 fpm
- `disagreements` — Anzahl mit `|xp − msfs| > 50 fpm`
- `avg_disagreement_fpm` — Mittel-Delta zwischen den zwei Algorithmen

Wird vom Touchdowns-Tab automatisch oben angezeigt (Card „🔬 Touchdown-Forensik nach Simulator"). Hoher `disagreements`-Anteil pro Sim = systematischer FSM-Edge-Case der noch nicht behoben ist.

## SQL-Beispiele für tieferes Drill-Down

```sql
-- Top 20 Disagreements der letzten 30 Tage
SELECT id, ts, simulator, vs_fpm, vs_estimate_xp_fpm, vs_estimate_msfs_fpm,
       ABS(vs_estimate_xp_fpm - vs_estimate_msfs_fpm) AS delta,
       vs_source, airport
FROM touchdowns
WHERE ts >= strftime('%s','now','-30 days')*1000
  AND vs_estimate_xp_fpm IS NOT NULL
  AND vs_estimate_msfs_fpm IS NOT NULL
ORDER BY delta DESC
LIMIT 20;

-- vs_source-Distribution pro Sim (welcher Pfad gewinnt am häufigsten?)
SELECT simulator, vs_source, COUNT(*) AS n
FROM touchdowns
WHERE ts >= strftime('%s','now','-30 days')*1000
GROUP BY simulator, vs_source
ORDER BY simulator, n DESC;

-- Window-Konfidenz: wieviele Touchdowns hatten <10 Samples im Berechnungs-Fenster?
SELECT simulator, COUNT(*) AS sparse
FROM touchdowns
WHERE estimate_sample_count < 10
  AND ts >= strftime('%s','now','-30 days')*1000
GROUP BY simulator;
```
