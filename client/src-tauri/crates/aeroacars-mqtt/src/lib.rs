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
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rumqttc::{AsyncClient, Event, LastWill, MqttOptions, Packet, QoS, TlsConfiguration, Transport};
use serde::Serialize;
use sim_core::{FlightPhase, SimSnapshot};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use url::Url;

pub mod provision;

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
}

#[derive(Clone, Debug, Serialize)]
struct PositionPayload {
    ts: i64,
    lat: f64,
    lon: f64,
    alt_ft: i32,
    ias_kt: i32,
    tas_kt: i32,
    gs_kt: i32,
    vs_fpm: i32,
    hdg_true: i32,
    hdg_mag: i32,
    on_ground: bool,
    callsign: String,
    aircraft_icao: String,
    dep: String,
    arr: String,
}

#[derive(Clone, Debug, Serialize)]
struct PhasePayload {
    ts: i64,
    phase: &'static str,
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
    pub sideslip_deg: Option<f32>,
    pub headwind_kt: Option<f32>,
    pub crosswind_kt: Option<f32>,
    pub score: Option<i32>,
    pub bounce: Option<bool>,
    pub runway: Option<String>,
    pub airport: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PirepPayload {
    pub ts: i64,
    pub pirep_id: String,
    pub flight_number: String,
    pub dep: String,
    pub arr: String,
    pub block_time_min: Option<i32>,
    pub fuel_used_kg: Option<f32>,
    pub landing_vs_fpm: Option<i32>,
    pub notes: Option<String>,
}

enum Cmd {
    Position(Box<PositionPayload>),
    Phase(PhasePayload),
    Touchdown(Box<TouchdownPayload>),
    Pirep(Box<PirepPayload>),
    Shutdown,
}

#[derive(Clone)]
pub struct Handle {
    tx: mpsc::Sender<Cmd>,
}

impl Handle {
    pub fn position(&self, snap: &SimSnapshot, meta: &FlightMeta) {
        let payload = PositionPayload {
            ts: snap.timestamp.timestamp_millis(),
            lat: snap.lat,
            lon: snap.lon,
            alt_ft: snap.altitude_msl_ft.round() as i32,
            ias_kt: snap.indicated_airspeed_kt.round() as i32,
            tas_kt: snap.true_airspeed_kt.round() as i32,
            gs_kt: snap.groundspeed_kt.round() as i32,
            vs_fpm: snap.vertical_speed_fpm.round() as i32,
            hdg_true: snap.heading_deg_true.round() as i32,
            hdg_mag: snap.heading_deg_magnetic.round() as i32,
            on_ground: snap.on_ground,
            callsign: meta.callsign.clone(),
            aircraft_icao: meta.aircraft_icao.clone(),
            dep: meta.dep_icao.clone(),
            arr: meta.arr_icao.clone(),
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

    let client_id = format!(
        "aeroacars-pilot-{}-{}-{}",
        cfg.va_prefix,
        cfg.pilot_id,
        std::process::id()
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

        while let Some(cmd) = rx.recv().await {
            match cmd {
                Cmd::Position(p) => publish_json(&pub_client, &cfg_for_pub.topic("position"), &p, QoS::AtMostOnce, true).await,
                Cmd::Phase(p) => publish_json(&pub_client, &cfg_for_pub.topic("phase"), &p, QoS::AtLeastOnce, true).await,
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

fn phase_label(p: FlightPhase) -> &'static str {
    match p {
        FlightPhase::Preflight | FlightPhase::Boarding => "PREFLIGHT",
        FlightPhase::Pushback | FlightPhase::TaxiOut => "TAXI_OUT",
        FlightPhase::TakeoffRoll | FlightPhase::Takeoff => "TAKEOFF",
        FlightPhase::Climb => "CLIMB",
        FlightPhase::Cruise => "CRUISE",
        // v0.5.11: Holding-Phase aus AeroACARS — semantisch wie
        // Cruise/Approach am Server, aber für Live-Map-Erkennung
        // separat ausgewiesen.
        FlightPhase::Holding => "HOLDING",
        FlightPhase::Descent => "DESCENT",
        FlightPhase::Approach => "APPROACH",
        FlightPhase::Final => "FINAL",
        FlightPhase::Landing => "LANDING",
        FlightPhase::TaxiIn => "TAXI_IN",
        FlightPhase::BlocksOn | FlightPhase::Arrived | FlightPhase::PirepSubmitted => "ON_BLOCK",
    }
}
