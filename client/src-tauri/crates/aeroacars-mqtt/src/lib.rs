//! MQTT publisher for AeroACARS — feeds the aeroacars-live monitor relay.
//!
//! ## Architecture
//!
//! - One spawned tokio task drives the rumqttc eventloop (so the connection
//!   stays alive, reconnects on failure, etc.).
//! - A second spawned task processes outgoing `Cmd`s from a bounded mpsc.
//! - The `Handle` exposed to callers is just a `Sender<Cmd>` wrapped in
//!   typed methods. All sends are non-blocking via `try_send`; if the
//!   channel is full (broker stalled), low-priority messages (position) are
//!   dropped, but high-priority ones (touchdown, pirep) block briefly.
//!
//! Topic schema mirrors `docs/topic-schema.md` of the aeroacars-live repo:
//!
//! ```text
//! aeroacars/<vaPrefix>/<pilotId>/{position,phase,touchdown,pirep,status}
//! ```
//!
//! `position`/`phase`/`status` are published with `retain=true` so a fresh
//! Monitor subscriber sees the latest known state immediately on connect.
//! `touchdown`/`pirep` are events, not state, and use `retain=false`.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rumqttc::{AsyncClient, Event, LastWill, MqttOptions, Packet, QoS, TlsConfiguration, Transport};
use serde::Serialize;
use sim_core::{FlightPhase, SimSnapshot, Simulator};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use url::Url;

pub mod provision;
pub mod log_upload;

const STATUS_ONLINE: &str = "online";
const STATUS_OFFLINE: &str = "offline";

/// Bounded queue between caller and MQTT-publisher task. ~5 s of position
/// ticks at the fastest cadence (5 s/tick → ~1 msg buffered on average,
/// burst tolerance of 200 msgs).
const CMD_BUFFER: usize = 200;

#[derive(Clone, Debug)]
pub struct MqttConfig {
    /// e.g. `wss://live.kant.ovh/mqtt`
    pub broker_url: String,
    /// Mosquitto user — typically `pilot_<id>`.
    pub username: String,
    pub password: String,
    /// VA prefix for topic routing — `gsg` for German Sky Group.
    pub va_prefix: String,
    /// phpVMS pilot id as string — `42`.
    pub pilot_id: String,
}

impl MqttConfig {
    pub fn validate(&self) -> Result<()> {
        if self.broker_url.is_empty() {
            anyhow::bail!("broker_url is empty");
        }
        let u = Url::parse(&self.broker_url).with_context(|| "invalid broker_url")?;
        if !matches!(u.scheme(), "wss" | "ws" | "mqtts" | "mqtt" | "ssl" | "tcp") {
            anyhow::bail!("broker_url scheme {} not supported", u.scheme());
        }
        if self.username.is_empty() || self.password.is_empty() {
            anyhow::bail!("username and password must be set");
        }
        if self.va_prefix.is_empty() || self.pilot_id.is_empty() {
            anyhow::bail!("va_prefix and pilot_id must be set");
        }
        Ok(())
    }

    fn topic(&self, channel: &str) -> String {
        format!("aeroacars/{}/{}/{}", self.va_prefix, self.pilot_id, channel)
    }
}

#[derive(Clone, Debug)]
pub struct FlightMeta {
    pub callsign: String,
    pub aircraft_icao: String,
    pub dep_icao: String,
    pub arr_icao: String,
    /// v0.5.19: phpVMS-side aircraft registration ("D-ALEU"). Sent
    /// to the live-tracking server in preference to the simulator's
    /// own ATC-ID (which payware addons often set to a generic
    /// placeholder like "FFSTS"). Empty when the bid had no
    /// registration on file — falls back to the snap's value then.
    pub planned_registration: String,
}

/// v0.5.14: rich position telemetry. Goal is "PIREP-grade analysis from
/// live data alone" — server can replay any flight, build approach
/// profiles, score touchdowns, audit FSM transitions, all without
/// needing the recorded JSONL. Sent every 5-30 s (phase-dependent).
///
/// Sizing: typical payload ~600-800 B JSON. At 30 s cadence in cruise
/// that's ~24 KB/h per pilot. At 5 s in approach: ~140 KB/h. Well
/// within Mosquitto+Caddy throughput on the VPS.
#[derive(Clone, Debug, Serialize)]
struct PositionPayload {
    ts: i64,
    /// Current FSM phase as label (PREFLIGHT, TAXI_OUT, TAKEOFF, CLIMB,
    /// CRUISE, HOLDING, DESCENT, APPROACH, FINAL, LANDING, TAXI_IN,
    /// ON_BLOCK). Inlined into every position so the Monitor never has
    /// to wait for a separate phase-topic delivery.
    phase: &'static str,

    // ---- Position ----
    lat: f64,
    lon: f64,
    alt_ft: i32,           // MSL altitude
    agl_ft: i32,           // Above-ground (for approach/landing analysis)

    // ---- Attitude ----
    pitch_deg: f32,
    bank_deg: f32,
    hdg_true: i32,
    hdg_mag: i32,

    // ---- Speeds ----
    ias_kt: i32,
    tas_kt: i32,
    gs_kt: i32,
    vs_fpm: i32,
    mach: Option<f32>,

    // ---- Forces / state ----
    g_force: f32,
    on_ground: bool,
    parking_brake: bool,
    stall_warning: bool,
    overspeed_warning: bool,

    // ---- Configuration ----
    gear_position: f32,    // 0=up, 1=down
    flaps_position: f32,   // 0..1
    spoilers_position: Option<f32>,
    spoilers_armed: Option<bool>,
    engines_running: u8,

    // ---- Fuel ----
    fuel_total_kg: f32,
    fuel_used_kg: f32,
    fuel_flow_kg_h: Option<f32>,
    total_weight_kg: Option<f32>,

    // ---- Environment ----
    wind_dir_deg: Option<f32>,
    wind_speed_kt: Option<f32>,
    oat_c: Option<f32>,
    qnh_hpa: Option<f32>,

    // ---- Autopilot (Boolean state) ----
    ap_master: Option<bool>,
    ap_hdg: Option<bool>,
    ap_alt: Option<bool>,
    ap_nav: Option<bool>,
    ap_app: Option<bool>,

    // ---- Identity ----
    //
    // v0.5.23: alle Identity-Felder sind jetzt Option<String> mit
    // skip_serializing_if. Hintergrund: phpVMS-API liefert manchmal leere
    // ICAO-Codes (Aircraft ohne ICAO-Feld in der DB). Wenn wir diese als
    // `""` serialisieren, ueberschreibt der Server-COALESCE-UPSERT den
    // vorher akkumulierten korrekten Wert mit "". Mit Option<String>+
    // skip_serializing_if = "Option::is_none" verschwindet das Feld
    // komplett aus dem JSON wenn leer → Server faellt sauber auf den
    // alten Wert zurueck. Fuer callsign/dep/arr aequivalent (defensive).
    #[serde(skip_serializing_if = "Option::is_none")]
    callsign: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    aircraft_icao: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    aircraft_registration: Option<String>,
    simulator: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    dep: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    arr: Option<String>,
    /// v0.5.24: Client-Version damit der aeroacars-live-Monitor sieht
    /// welcher Pilot mit welcher Build-Version sendet. Ermöglicht
    /// Version-Compliance-Tracking (= "Pilot X läuft noch v0.5.16-Pre-
    /// Numeric-Fix, Hard-Landing-Check failed silent").
    client_version: &'static str,
}

/// Convert empty/whitespace-only strings to None — used at the JSON-edge
/// to keep payloads clean of "" values that would muddy the server side.
fn non_empty(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() { None } else { Some(t.to_string()) }
}

#[derive(Clone, Debug, Serialize)]
struct PhasePayload {
    ts: i64,
    phase: &'static str,
}

/// v0.5.14: authoritative block snapshot. Fires once when the FSM
/// transitions Preflight/Boarding → Pushback/TaxiOut (= block-off
/// is stamped). Carries fuel + planned-OFP values that are STABLE
/// at this point — `position` payloads during PREFLIGHT show LIVE
/// fuel which can still be loading and is NOT authoritative.
#[derive(Clone, Debug, Serialize)]
pub struct BlockPayload {
    pub ts: i64,
    pub block_fuel_kg: Option<f32>,
    pub planned_block_fuel_kg: Option<f32>,
    pub planned_burn_kg: Option<f32>,
    pub planned_reserve_kg: Option<f32>,
    pub planned_zfw_kg: Option<f32>,
    pub planned_tow_kg: Option<f32>,
    pub planned_ldw_kg: Option<f32>,
    pub planned_max_zfw_kg: Option<f32>,
    pub planned_max_tow_kg: Option<f32>,
    pub planned_max_ldw_kg: Option<f32>,
    pub planned_route: Option<String>,
    pub planned_alternate: Option<String>,
    pub dep_gate: Option<String>,
    pub dep_metar: Option<String>,
}

/// v0.5.14: takeoff snapshot. Fires once when the FSM stamps
/// `takeoff_at` (= aircraft has left the ground). Authoritative
/// TOW + fuel-at-takeoff values for fuel-burn / overweight analysis.
#[derive(Clone, Debug, Serialize)]
pub struct TakeoffPayload {
    pub ts: i64,
    pub takeoff_weight_kg: Option<f32>,
    pub takeoff_fuel_kg: Option<f32>,
    pub takeoff_lat: Option<f64>,
    pub takeoff_lon: Option<f64>,
    pub dep_metar: Option<String>,
    pub dep_runway: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct TouchdownPayload {
    pub ts: i64,
    pub vs_fpm: i32,
    pub ias_kt: i32,
    pub gs_kt: Option<i32>,
    pub pitch_deg: Option<f32>,
    pub bank_deg: Option<f32>,
    pub g_load: Option<f32>,
    pub peak_g_load: Option<f32>,
    pub sideslip_deg: Option<f32>,
    pub headwind_kt: Option<f32>,
    pub crosswind_kt: Option<f32>,
    pub score: Option<i32>,
    pub bounce: Option<bool>,
    pub bounce_count: Option<u8>,
    pub runway: Option<String>,
    pub airport: Option<String>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub heading_true_deg: Option<f32>,
    pub heading_mag_deg: Option<f32>,
    pub landing_weight_kg: Option<f32>,
    pub landing_fuel_kg: Option<f32>,
    pub rollout_distance_m: Option<f32>,
    /// V/S standard deviation over the approach window (fpm) — lower = more stable.
    pub approach_vs_stddev_fpm: Option<f32>,
    /// Bank-angle standard deviation over the approach window (deg).
    pub approach_bank_stddev_deg: Option<f32>,
    pub go_around_count: Option<u32>,
    pub arr_metar: Option<String>,
    /// True if a runway was correlated from the touchdown coord (OurAirports CSV).
    pub runway_match_icao: Option<String>,
    pub runway_match_ident: Option<String>,
    pub runway_match_distance_m: Option<f32>,
    pub runway_match_centerline_offset_m: Option<f32>,
    /// v0.5.22: total length of the matched runway in metres (from the
    /// OurAirports CSV row). Required server-side for the "Bahn-Auslastung"
    /// sub-score (rollout / length × 100) so the live monitor can show
    /// the same breakdown the AeroACARS app shows pilots in-flight.
    pub runway_length_m: Option<f32>,
    /// v0.5.22: (actual_burn − planned_burn) / planned_burn × 100. Same
    /// computation as `LandingRecord.fuel_efficiency_pct` in the client
    /// — drives the "Spritverbrauch" sub-score. None when the bid had
    /// no SimBrief OFP attached (planned-burn unavailable).
    pub fuel_efficiency_pct: Option<f32>,
    // ─── v0.5.23 Touchdown-Forensik ──────────────────────────────────
    //
    // Der Client berechnet bei jeder Landung BEIDE Schaetzer (Lua-30-
    // Sample fuer X-Plane, Time-Tier fuer MSFS) parallel — vorher haben
    // wir nur den finalen Wert publiziert. Mit diesen Feldern kann der
    // Server-seitige Forensik-Workflow vergleichen wie weit die beiden
    // Algorithmen auseinanderlagen + welcher Pfad gewonnen hat. Werte
    // sind Option<...> mit skip_serializing_if damit alte Pilot-Clients
    // (v0.5.22-) ohne diese Daten weiter funktionieren.
    /// "msfs" / "xplane" / "other" — welcher Sim-Adapter den Snapshot
    /// generiert hat. Identisch zum bestehenden simulator-Feld im
    /// position-Payload, hier zusaetzlich ans Touchdown gepinnt damit
    /// die Server-touchdowns-Tabelle ohne JOIN filtern kann.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub simulator: Option<String>,
    /// Lua-Style 30-Sample-AGL-Δ-Schaetzung (Volanta/LandingRate-1-aligned).
    /// Primaer fuer X-Plane, fuer MSFS als Vergleichswert mitgeschickt.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vs_estimate_xp_fpm: Option<i32>,
    /// Time-Tier-AGL-Δ-Schaetzung (750ms/1s/1.5s/2s/3s/12s window-progression).
    /// Fallback fuer MSFS, fuer X-Plane als Vergleichswert mitgeschickt.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vs_estimate_msfs_fpm: Option<i32>,
    /// Welcher Pfad hat den finalen `vs_fpm` geliefert? Werte:
    /// "msfs_simvar_latched" — PLANE TOUCHDOWN NORMAL VELOCITY direkt
    /// "agl_estimate_msfs" — Time-Tier-Schaetzer
    /// "agl_estimate_xp" — Lua-30-Sample-Schaetzer
    /// "sampler_gear_force" — X-Plane Gear-Sampler (50Hz)
    /// "buffer_min" — Buffer-Window-Scan (Last-Resort)
    /// "low_agl_vs_min" — Approach-Tracker-Fallback
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vs_source: Option<String>,
    /// X-Plane Gear-Sampler peak gear_normal_force_n im Touchdown-Frame.
    /// Liefert MSFS nicht (= None auf MSFS).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gear_force_peak_n: Option<f32>,
    /// Lua-Style-Schaetzer adaptive Window-Groesse in ms (= je nach
    /// Sample-Density 500-3000 ms typisch). None wenn der Pfad nicht
    /// gewonnen hat oder keine Samples vorhanden waren.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimate_window_ms: Option<i32>,
    /// Wieviele Samples lagen im Berechnungs-Fenster. <10 = sparsam =
    /// niedrige Konfidenz.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimate_sample_count: Option<u32>,
    // ─── v0.5.25 Approach-Stability v2 ────────────────────────────────
    //
    // Stable-Approach-Gate-konformes Stability-Maß (FAA AC 120-71B /
    // EASA SUPP-32). Window: AGL ≤ 1000 ft. Filter: Vector-Window
    // ausgeklammert. Ground-truth: Glide-Slope-Deviation statt
    // statistische Variance.
    /// Mittlere |actual_vs − target_vs(3°)| im 1000-ft-Gate.
    /// 0 fpm = perfekt, > 200 fpm = unstabil.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approach_vs_deviation_fpm: Option<f32>,
    /// Maximale Deviation unter 500 ft AGL — kritischste Phase.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approach_max_vs_deviation_below_500_fpm: Option<f32>,
    /// Bank-Stddev im 1000-ft-Gate, gefiltert (Vector-Windows weg).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approach_bank_stddev_filtered_deg: Option<f32>,
    /// True wenn unter 1500 ft AGL ATC-RWY-Wechsel beobachtet.
    /// Auf der Webapp-Seite Hinweis-Pill, Score wird neutral-justiert.
    #[serde(skip_serializing_if = "is_false")]
    pub approach_runway_changed_late: bool,
    /// Stable-Approach-Gate-Indikator: bei 1000 ft AGL erreicht?
    /// (= vs_deviation < 200 fpm AND mean_bank < 5°)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approach_stable_at_gate: Option<bool>,
    /// Sample-Count im 1000-ft-Window (Konfidenz-Indikator).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approach_window_sample_count: Option<u32>,
    /// V/S-Jerk: mean |Δvs| sample-to-sample im Gate. Sim-/Aircraft-
    /// agnostic (= jet, turboprop, GA gleichermassen). PRIMAERER
    /// Stabilitaets-Indikator. < 100 fpm/tick = stable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approach_vs_jerk_fpm: Option<f32>,
    /// IAS-Stddev im Gate-Window. Speed-Stability.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approach_ias_stddev_kt: Option<f32>,
    /// Excessive Sink: ≥1 Sample mit V/S < -1000 fpm im Gate.
    /// FAA Sink-Rate-Limit. Auto-Fail-Indikator.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approach_excessive_sink: Option<bool>,
    /// Gear+Flaps am Gate-Eintritt in Landing-Konfig
    /// (Gear≥99% AND Flaps≥70%).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approach_stable_config: Option<bool>,
    /// HAT (Height Above Touchdown) statt AGL als Window-Filter genutzt.
    /// True = arr_airport_elevation_ft bekannt → Mountain-Airport-tauglich.
    /// False = AGL-Fallback (= im Tal-Anflug ueber Berge ggf. ungenau).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approach_used_hat: Option<bool>,
    // ─── v0.5.26 Erweiterte Landung-Metriken ──────────────────────────
    /// Wing-Strike-Severity: |bank_at_td| / aircraft_max_bank_deg × 100%.
    /// 0% = wings level, 100% = am Limit. Aircraft-spezifisch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub landing_wing_strike_severity_pct: Option<f32>,
    /// Distanz Threshold→Touchdown in Metern. Long-Landing-Indikator.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub landing_float_distance_m: Option<f32>,
    /// Touchdown-Zone (1/2/3 nach FAA: erstes/zweites/drittes Drittel).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub landing_touchdown_zone: Option<u8>,
    /// IAS-am-TD − Vref (positiv = zu schnell, negativ = zu langsam).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub landing_vref_deviation_kt: Option<f32>,
    /// Vref-Quelle: "pmdg" / "icao_default" / "unknown".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub landing_vref_source: Option<String>,
    /// Stable-Approach bei DA (= 200 ft AGL/HAT). Strenger als 1000-ft-Gate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approach_stable_at_da: Option<bool>,
    /// Anzahl Stall-Warning-Samples im Approach-Buffer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approach_stall_warning_count: Option<u32>,
    /// Yaw-Rate am Touchdown (deg/sec). Hoch = Ground-Loop-Risk.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub landing_yaw_rate_deg_per_sec: Option<f32>,
    /// Brake-Energy-Proxy in kJ/m. Hoch = brake-pack-thermal-stress.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub landing_brake_energy_proxy: Option<f32>,
}

fn is_false(b: &bool) -> bool { !*b }

#[derive(Clone, Debug, Serialize)]
pub struct PirepPayload {
    pub ts: i64,
    pub pirep_id: String,
    pub flight_number: String,
    pub dep: String,
    pub arr: String,
    pub block_time_min: Option<i32>,
    pub flight_time_min: Option<i32>,
    pub distance_nm: Option<f32>,
    pub fuel_used_kg: Option<f32>,
    pub planned_burn_kg: Option<f32>,
    pub block_fuel_kg: Option<f32>,
    pub takeoff_fuel_kg: Option<f32>,
    pub landing_fuel_kg: Option<f32>,
    pub takeoff_weight_kg: Option<f32>,
    pub landing_weight_kg: Option<f32>,
    pub planned_tow_kg: Option<f32>,
    pub planned_ldw_kg: Option<f32>,
    pub peak_altitude_ft: Option<i32>,
    pub landing_vs_fpm: Option<i32>,
    pub landing_score: Option<i32>,
    pub go_around_count: Option<u32>,
    pub touchdown_count: Option<u32>,
    pub dep_gate: Option<String>,
    pub arr_gate: Option<String>,
    pub approach_runway: Option<String>,
    pub divert: Option<bool>,
    pub diverted_to: Option<String>,
    pub notes: Option<String>,
}

enum Cmd {
    Position(Box<PositionPayload>),
    Phase(PhasePayload),
    Block(Box<BlockPayload>),
    Takeoff(Box<TakeoffPayload>),
    Touchdown(Box<TouchdownPayload>),
    Pirep(Box<PirepPayload>),
    Shutdown,
}

#[derive(Clone)]
pub struct Handle {
    tx: mpsc::Sender<Cmd>,
}

impl Handle {
    pub fn position(&self, snap: &SimSnapshot, meta: &FlightMeta, phase: FlightPhase) {
        let payload = PositionPayload {
            ts: snap.timestamp.timestamp_millis(),
            phase: phase_label(phase),

            // Position
            lat: snap.lat,
            lon: snap.lon,
            alt_ft: snap.altitude_msl_ft.round() as i32,
            agl_ft: snap.altitude_agl_ft.round() as i32,

            // Attitude
            pitch_deg: snap.pitch_deg,
            bank_deg: snap.bank_deg,
            hdg_true: snap.heading_deg_true.round() as i32,
            hdg_mag: snap.heading_deg_magnetic.round() as i32,

            // Speeds
            ias_kt: snap.indicated_airspeed_kt.round() as i32,
            tas_kt: snap.true_airspeed_kt.round() as i32,
            gs_kt: snap.groundspeed_kt.round() as i32,
            vs_fpm: snap.vertical_speed_fpm.round() as i32,
            mach: snap.mach,

            // Forces / state
            g_force: snap.g_force,
            on_ground: snap.on_ground,
            parking_brake: snap.parking_brake,
            stall_warning: snap.stall_warning,
            overspeed_warning: snap.overspeed_warning,

            // Config
            gear_position: snap.gear_position,
            flaps_position: snap.flaps_position,
            spoilers_position: snap.spoilers_handle_position,
            spoilers_armed: snap.spoilers_armed,
            engines_running: snap.engines_running,

            // Fuel
            fuel_total_kg: snap.fuel_total_kg,
            fuel_used_kg: snap.fuel_used_kg,
            fuel_flow_kg_h: snap.fuel_flow_kg_per_h,
            total_weight_kg: snap.total_weight_kg,

            // Environment
            wind_dir_deg: snap.wind_direction_deg,
            wind_speed_kt: snap.wind_speed_kt,
            oat_c: snap.outside_air_temp_c,
            qnh_hpa: snap.qnh_hpa,

            // AP
            ap_master: snap.autopilot_master,
            ap_hdg: snap.autopilot_heading,
            ap_alt: snap.autopilot_altitude,
            ap_nav: snap.autopilot_nav,
            ap_app: snap.autopilot_approach,

            // Identity — alle non_empty(): leere Strings werden zu None und
            // verschwinden aus dem JSON statt "" zu serialisieren. Server-
            // seitige COALESCE-UPSERTs bleiben so frei von Empty-String-
            // Vergiftung der flights-Tabelle.
            callsign: non_empty(&meta.callsign),
            aircraft_icao: non_empty(&meta.aircraft_icao),
            // v0.5.19: prefer phpVMS-side registration (from the bid)
            // over what the sim reports — payware addons often put
            // a placeholder ("FFSTS") in the SimConnect ATC-ID.
            // Falls back to the sim value if the bid had nothing.
            aircraft_registration: if !meta.planned_registration.trim().is_empty() {
                Some(meta.planned_registration.trim().to_string())
            } else {
                snap.aircraft_registration
                    .as_deref()
                    .and_then(non_empty)
            },
            simulator: simulator_label(snap.simulator),
            dep: non_empty(&meta.dep_icao),
            arr: non_empty(&meta.arr_icao),
            client_version: env!("CARGO_PKG_VERSION"),
        };
        match self.tx.try_send(Cmd::Position(Box::new(payload))) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                debug!("mqtt cmd channel full — dropping position tick");
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                debug!("mqtt cmd channel closed — publisher down");
            }
        }
    }

    pub fn phase(&self, phase: FlightPhase, ts: DateTime<Utc>) {
        let payload = PhasePayload {
            ts: ts.timestamp_millis(),
            phase: phase_label(phase),
        };
        let _ = self.tx.try_send(Cmd::Phase(payload));
    }

    pub fn block(&self, payload: BlockPayload) {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let _ = tokio::time::timeout(
                Duration::from_millis(500),
                tx.send(Cmd::Block(Box::new(payload))),
            )
            .await;
        });
    }

    pub fn takeoff(&self, payload: TakeoffPayload) {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let _ = tokio::time::timeout(
                Duration::from_millis(500),
                tx.send(Cmd::Takeoff(Box::new(payload))),
            )
            .await;
        });
    }

    pub fn touchdown(&self, payload: TouchdownPayload) {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            if let Err(e) = tokio::time::timeout(
                Duration::from_millis(250),
                tx.send(Cmd::Touchdown(Box::new(payload))),
            )
            .await
            {
                warn!("dropping touchdown publish: {e}");
            }
        });
    }

    pub fn pirep(&self, payload: PirepPayload) {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            if let Err(e) =
                tokio::time::timeout(Duration::from_millis(500), tx.send(Cmd::Pirep(Box::new(payload)))).await
            {
                warn!("dropping pirep publish: {e}");
            }
        });
    }

    pub fn shutdown(&self) {
        let _ = self.tx.try_send(Cmd::Shutdown);
    }
}

pub fn start(cfg: MqttConfig) -> Result<Handle> {
    cfg.validate()?;

    let (tx, mut rx) = mpsc::channel::<Cmd>(CMD_BUFFER);

    let url = Url::parse(&cfg.broker_url)?;
    let port = url.port_or_known_default().unwrap_or(443);
    let scheme = url.scheme().to_string();

    // rumqttc 0.24: für WS/WSS muss broker_addr die VOLLSTÄNDIGE URL sein
    // (mit Scheme + Pfad), nicht nur der Hostname. split_url() liest das
    // Scheme um den Default-Port zu resolven. Bei TCP/TLS dagegen: nur Host.
    let broker_addr: String = match scheme.as_str() {
        "ws" | "wss" => cfg.broker_url.clone(),
        _ => url.host_str().context("no host in broker_url")?.to_string(),
    };

    // v0.5.14: client_id eindeutig pro start()-Aufruf (PID + ms-Timestamp).
    // Falls die Idempotency-Guard im Caller versehentlich umgangen wird
    // (Race zwischen check und insert in `state.mqtt`), würden zwei
    // Clients mit gleichem client_id sich gegenseitig vom Broker kicken
    // (MQTT-Spec: "Client X already connected, closing old connection").
    // Belt-and-suspenders: unterschiedliche IDs → koexistierende Clients
    // wären zwar unschön (doppelte Pubs), aber kein Connection-Drop.
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let client_id = format!(
        "aeroacars-pilot-{}-{}-{}-{}",
        cfg.va_prefix,
        cfg.pilot_id,
        std::process::id(),
        now_ms
    );
    let status_topic = cfg.topic("status");

    let mut opts = MqttOptions::new(&client_id, &broker_addr, port);
    opts.set_credentials(&cfg.username, &cfg.password);
    opts.set_keep_alive(Duration::from_secs(60));
    opts.set_clean_session(true);
    opts.set_last_will(LastWill::new(
        &status_topic,
        STATUS_OFFLINE,
        QoS::AtLeastOnce,
        true,
    ));

    let transport = match scheme.as_str() {
        "wss" => Transport::Wss(default_tls_config()),
        "ws" => Transport::Ws,
        "mqtts" | "ssl" => Transport::Tls(default_tls_config()),
        "mqtt" | "tcp" => Transport::Tcp,
        s => anyhow::bail!("unsupported scheme: {s}"),
    };
    opts.set_transport(transport);

    info!(client_id = %client_id, broker = %broker_addr, port, "starting MQTT publisher");

    let (client, mut eventloop) = AsyncClient::new(opts, CMD_BUFFER);

    let _drive = tokio::spawn(async move {
        loop {
            match eventloop.poll().await {
                Ok(Event::Incoming(Packet::ConnAck(_))) => {
                    info!("MQTT CONNACK received");
                }
                Ok(_) => {}
                Err(e) => {
                    warn!("MQTT poll error: {e} — backing off 5 s");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    });

    let cfg_for_pub = cfg.clone();
    let pub_client = client.clone();
    let _publisher = tokio::spawn(async move {
        if let Err(e) = pub_client
            .publish(
                cfg_for_pub.topic("status"),
                QoS::AtLeastOnce,
                true,
                STATUS_ONLINE.as_bytes(),
            )
            .await
        {
            warn!("initial status publish failed: {e}");
        }

        // v0.5.14: Initial phase publish. Without this, a pilot
        // sitting at the gate (FSM = Preflight, no transition yet)
        // never publishes a phase → Monitor shows "—". Retained
        // message means new Monitor subscribers see it on connect.
        let initial_phase = PhasePayload {
            ts: chrono::Utc::now().timestamp_millis(),
            phase: phase_label(FlightPhase::Preflight),
        };
        publish_json(
            &pub_client,
            &cfg_for_pub.topic("phase"),
            &initial_phase,
            QoS::AtLeastOnce,
            true,
        )
        .await;

        while let Some(cmd) = rx.recv().await {
            match cmd {
                Cmd::Position(p) => publish_json(&pub_client, &cfg_for_pub.topic("position"), &p, QoS::AtMostOnce, true).await,
                Cmd::Phase(p) => publish_json(&pub_client, &cfg_for_pub.topic("phase"), &p, QoS::AtLeastOnce, true).await,
                Cmd::Block(p) => publish_json(&pub_client, &cfg_for_pub.topic("block"), &p, QoS::AtLeastOnce, true).await,
                Cmd::Takeoff(p) => publish_json(&pub_client, &cfg_for_pub.topic("takeoff"), &p, QoS::AtLeastOnce, true).await,
                Cmd::Touchdown(p) => publish_json(&pub_client, &cfg_for_pub.topic("touchdown"), &p, QoS::AtLeastOnce, false).await,
                Cmd::Pirep(p) => publish_json(&pub_client, &cfg_for_pub.topic("pirep"), &p, QoS::AtLeastOnce, false).await,
                Cmd::Shutdown => {
                    let _ = pub_client
                        .publish(cfg_for_pub.topic("status"), QoS::AtLeastOnce, true, STATUS_OFFLINE.as_bytes())
                        .await;
                    let _ = pub_client.disconnect().await;
                    break;
                }
            }
        }
        debug!("MQTT cmd loop exiting");
    });

    Ok(Handle { tx })
}

async fn publish_json<T: Serialize>(client: &AsyncClient, topic: &str, payload: &T, qos: QoS, retain: bool) {
    let body = match serde_json::to_vec(payload) {
        Ok(b) => b,
        Err(e) => {
            error!("serialize {topic} failed: {e}");
            return;
        }
    };
    if let Err(e) = client.publish(topic, qos, retain, body).await {
        warn!("publish {topic} failed: {e}");
    }
}

fn default_tls_config() -> TlsConfiguration {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let cfg = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    TlsConfiguration::Rustls(Arc::new(cfg))
}

fn simulator_label(sim: Simulator) -> &'static str {
    match sim {
        Simulator::Msfs2020 => "MSFS_2020",
        Simulator::Msfs2024 => "MSFS_2024",
        Simulator::XPlane11 => "XP11",
        Simulator::XPlane12 => "XP12",
        Simulator::Other => "OTHER",
    }
}

fn phase_label(p: FlightPhase) -> &'static str {
    // v0.5.18: granular 1:1 mapping of all 17 internal FSM phases to
    // distinct MQTT labels. Pre-v0.5.18 we collapsed 5 pairs/triples
    // (Preflight+Boarding → PREFLIGHT, Pushback+TaxiOut → TAXI_OUT,
    // TakeoffRoll+Takeoff → TAKEOFF, BlocksOn+Arrived+PirepSubmitted
    // → ON_BLOCK) for "simpler live-map" — but this lost data the
    // server needs for proper flight-phase analytics, rotation
    // timing, post-landing state distinction etc. The server-side
    // mapping table is being updated in lockstep.
    match p {
        FlightPhase::Preflight => "PREFLIGHT",
        FlightPhase::Boarding => "BOARDING",
        FlightPhase::Pushback => "PUSHBACK",
        FlightPhase::TaxiOut => "TAXI_OUT",
        FlightPhase::TakeoffRoll => "TAKEOFF_ROLL",
        FlightPhase::Takeoff => "TAKEOFF",
        FlightPhase::Climb => "CLIMB",
        FlightPhase::Cruise => "CRUISE",
        FlightPhase::Holding => "HOLDING",
        FlightPhase::Descent => "DESCENT",
        FlightPhase::Approach => "APPROACH",
        FlightPhase::Final => "FINAL",
        FlightPhase::Landing => "LANDING",
        FlightPhase::TaxiIn => "TAXI_IN",
        FlightPhase::BlocksOn => "BLOCKS_ON",
        FlightPhase::Arrived => "ARRIVED",
        FlightPhase::PirepSubmitted => "PIREP_SUBMITTED",
    }
}
