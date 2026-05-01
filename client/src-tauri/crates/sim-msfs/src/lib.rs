//! MSFS 2020 / MSFS 2024 simulator adapter — **SimConnect only, never FSUIPC**.
//!
//! See ADR-0002 in `docs/decisions/0002-msfs-simconnect-only.md`.
//!
//! Reference docs: <https://docs.flightsimulator.com/html/Programming_Tools/SimConnect/SimConnect_SDK.htm>
//!
//! Status: Phase 1 — position, altitude, speeds, heading, on-ground.
//! More telemetry (fuel, payload, gear, flaps, fault flags, sim version) lands
//! incrementally as the recorder and rules engine grow.

#![allow(dead_code)]

#[cfg(target_os = "windows")]
mod adapter {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};

    use chrono::Utc;
    use serde::Serialize;
    use sim_core::{SimKind, SimSnapshot};
    use simconnect_sdk::{Notification, SimConnect, SimConnectObject};

    /// If no SimConnect data arrives within this window we treat the connection
    /// as dead even when SimConnect itself hasn't reported an error. This catches
    /// MSFS crashes and frozen pipes — both surface as "no events" rather than
    /// a clean error from the SDK.
    const STALE_TIMEOUT: Duration = Duration::from_secs(5);

    /// Phase-1 telemetry definition. Field names are SimConnect SimVar strings,
    /// units are SimConnect units. Adding a field here makes it flow through to
    /// `SimSnapshot` via `telemetry_to_snapshot`.
    #[derive(Debug, Clone, SimConnectObject)]
    #[simconnect(period = "second")]
    #[allow(non_snake_case)]
    struct Telemetry {
        #[simconnect(name = "TITLE")]
        title: String,
        #[simconnect(name = "ATC MODEL")]
        atc_model: String,
        /// Tail number / registration set in MSFS (e.g. "D-AILU").
        #[simconnect(name = "ATC ID")]
        atc_id: String,
        #[simconnect(name = "PLANE LATITUDE", unit = "degrees")]
        lat: f64,
        #[simconnect(name = "PLANE LONGITUDE", unit = "degrees")]
        lon: f64,
        #[simconnect(name = "PLANE ALTITUDE", unit = "feet")]
        altitude_msl_ft: f64,
        #[simconnect(name = "PLANE ALT ABOVE GROUND", unit = "feet")]
        altitude_agl_ft: f64,
        #[simconnect(name = "PLANE HEADING DEGREES TRUE", unit = "degrees")]
        heading_true_deg: f64,
        #[simconnect(name = "PLANE HEADING DEGREES MAGNETIC", unit = "degrees")]
        heading_magnetic_deg: f64,
        #[simconnect(name = "GROUND VELOCITY", unit = "knots")]
        groundspeed_kt: f64,
        #[simconnect(name = "AIRSPEED INDICATED", unit = "knots")]
        indicated_airspeed_kt: f64,
        #[simconnect(name = "AIRSPEED TRUE", unit = "knots")]
        true_airspeed_kt: f64,
        #[simconnect(name = "VERTICAL SPEED", unit = "feet per minute")]
        vertical_speed_fpm: f64,
        #[simconnect(name = "PLANE PITCH DEGREES", unit = "degrees")]
        pitch_deg: f64,
        #[simconnect(name = "PLANE BANK DEGREES", unit = "degrees")]
        bank_deg: f64,
        #[simconnect(name = "G FORCE", unit = "GForce")]
        g_force: f64,
        #[simconnect(name = "SIM ON GROUND", unit = "bool")]
        on_ground: bool,
    }

    /// Build a `SimSnapshot` from raw telemetry. The simulator field is tagged
    /// from the user-selected `SimKind` because SimConnect can't distinguish
    /// MSFS 2020 from MSFS 2024 at the API level.
    fn telemetry_to_snapshot(t: &Telemetry, kind: SimKind) -> SimSnapshot {
        SimSnapshot {
            timestamp: Utc::now(),
            lat: t.lat,
            lon: t.lon,
            altitude_msl_ft: t.altitude_msl_ft,
            altitude_agl_ft: t.altitude_agl_ft,
            heading_deg_true: t.heading_true_deg as f32,
            heading_deg_magnetic: t.heading_magnetic_deg as f32,
            pitch_deg: t.pitch_deg as f32,
            bank_deg: t.bank_deg as f32,
            vertical_speed_fpm: t.vertical_speed_fpm as f32,
            groundspeed_kt: t.groundspeed_kt as f32,
            indicated_airspeed_kt: t.indicated_airspeed_kt as f32,
            true_airspeed_kt: t.true_airspeed_kt as f32,
            g_force: t.g_force as f32,
            on_ground: t.on_ground,
            // Fields not yet pulled from SimConnect — populated in later phases.
            parking_brake: false,
            stall_warning: false,
            overspeed_warning: false,
            paused: false,
            slew_mode: false,
            simulation_rate: 1.0,
            gear_position: 0.0,
            flaps_position: 0.0,
            engines_running: 0,
            fuel_total_kg: 0.0,
            fuel_used_kg: 0.0,
            zfw_kg: None,
            payload_kg: None,
            wind_direction_deg: None,
            wind_speed_kt: None,
            qnh_hpa: None,
            outside_air_temp_c: None,
            aircraft_title: Some(t.title.clone()).filter(|s| !s.is_empty()),
            aircraft_icao: Some(t.atc_model.clone()).filter(|s| !s.is_empty()),
            aircraft_registration: Some(t.atc_id.clone()).filter(|s| !s.is_empty()),
            simulator: kind.as_simulator(),
            sim_version: None,
        }
    }

    #[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
    #[serde(rename_all = "snake_case")]
    pub enum ConnectionState {
        /// No worker thread is running.
        Disconnected,
        /// Worker is alive; SimConnect handshake either pending, retrying,
        /// or done but no snapshot received yet.
        Connecting,
        /// Worker is connected and at least one snapshot has arrived.
        Connected,
    }

    struct Shared {
        state: Mutex<ConnectionState>,
        snapshot: Mutex<Option<SimSnapshot>>,
        last_error: Mutex<Option<String>>,
    }

    /// Owns a background thread that talks to MSFS via SimConnect.
    /// `start(kind)` is idempotent; `stop()` is too.
    pub struct MsfsAdapter {
        shared: Arc<Shared>,
        stop: Arc<AtomicBool>,
        thread: Option<thread::JoinHandle<()>>,
        kind: SimKind,
    }

    impl Default for MsfsAdapter {
        fn default() -> Self {
            Self::new()
        }
    }

    impl MsfsAdapter {
        pub fn new() -> Self {
            Self {
                shared: Arc::new(Shared {
                    state: Mutex::new(ConnectionState::Disconnected),
                    snapshot: Mutex::new(None),
                    last_error: Mutex::new(None),
                }),
                stop: Arc::new(AtomicBool::new(false)),
                thread: None,
                kind: SimKind::Msfs2024,
            }
        }

        /// Start the adapter for the given simulator kind. If already running with
        /// the same kind, this is a no-op. If running with a different kind, the
        /// adapter is restarted with the new tag (mainly affects PIREP simulator
        /// reporting; SimConnect itself can't tell 2020 vs 2024 apart).
        pub fn start(&mut self, kind: SimKind) {
            if !kind.is_msfs() {
                self.stop();
                return;
            }
            if self.thread.is_some() && self.kind == kind {
                return;
            }
            self.stop();
            self.kind = kind;
            *self.shared.state.lock().expect("state lock") = ConnectionState::Connecting;
            *self.shared.last_error.lock().expect("err lock") = None;
            self.stop.store(false, Ordering::Relaxed);

            let shared = Arc::clone(&self.shared);
            let stop = Arc::clone(&self.stop);
            let kind_for_thread = kind;
            self.thread = Some(thread::spawn(move || {
                run_loop(shared, stop, kind_for_thread);
            }));
            tracing::info!(?kind, "MSFS adapter started");
        }

        pub fn stop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
            *self.shared.state.lock().expect("state lock") = ConnectionState::Disconnected;
            *self.shared.snapshot.lock().expect("snapshot lock") = None;
            tracing::info!("MSFS adapter stopped");
        }

        pub fn state(&self) -> ConnectionState {
            *self.shared.state.lock().expect("state lock")
        }

        pub fn snapshot(&self) -> Option<SimSnapshot> {
            self.shared.snapshot.lock().expect("snapshot lock").clone()
        }

        pub fn last_error(&self) -> Option<String> {
            self.shared.last_error.lock().expect("err lock").clone()
        }
    }

    fn run_loop(shared: Arc<Shared>, stop: Arc<AtomicBool>, kind: SimKind) {
        // Outer reconnect loop — keep trying to attach until the user explicitly stops us.
        while !stop.load(Ordering::Relaxed) {
            let mut client = match SimConnect::new("CloudeAcars") {
                Ok(c) => c,
                Err(e) => {
                    tracing::debug!(error = %e, "SimConnect not available yet; retrying");
                    *shared.last_error.lock().expect("err") = Some(format!("SimConnect: {e}"));
                    *shared.state.lock().expect("state") = ConnectionState::Connecting;
                    if !sleep_or_stop(&stop, Duration::from_secs(3)) {
                        return;
                    }
                    continue;
                }
            };

            if let Err(e) = client.register_object::<Telemetry>() {
                tracing::warn!(error = %e, "failed to register telemetry");
                *shared.last_error.lock().expect("err") = Some(format!("register: {e}"));
                if !sleep_or_stop(&stop, Duration::from_secs(2)) {
                    return;
                }
                continue;
            }

            tracing::info!("SimConnect handshake done — waiting for first snapshot");
            // Stay in Connecting until we actually receive a snapshot. Otherwise
            // the UI would briefly show stale data from a previous connection,
            // or claim "Connected" when MSFS still hasn't started feeding us.
            *shared.state.lock().expect("state") = ConnectionState::Connecting;
            *shared.last_error.lock().expect("err") = None;

            // Inner dispatch loop — pulls telemetry until we lose the connection.
            // `last_data` flips to `Some(Instant)` on the first snapshot. Once set,
            // we tear down and reconnect if the gap to the next snapshot exceeds
            // STALE_TIMEOUT — that's how we notice MSFS crashes.
            let mut last_data: Option<Instant> = None;
            loop {
                if stop.load(Ordering::Relaxed) {
                    return;
                }

                if let Some(t) = last_data {
                    if t.elapsed() > STALE_TIMEOUT {
                        tracing::warn!(
                            stale_for = ?t.elapsed(),
                            "no SimConnect data for too long — reconnecting"
                        );
                        *shared.last_error.lock().expect("err") = Some(format!(
                            "no telemetry for {}s — sim may have crashed",
                            STALE_TIMEOUT.as_secs()
                        ));
                        break;
                    }
                }

                match client.get_next_dispatch() {
                    Ok(Some(notification)) => match notification {
                        Notification::Object(data) => {
                            if let Ok(t) = Telemetry::try_from(&data) {
                                let snap = telemetry_to_snapshot(&t, kind);
                                *shared.snapshot.lock().expect("snapshot") = Some(snap);
                                if last_data.is_none() {
                                    *shared.state.lock().expect("state") =
                                        ConnectionState::Connected;
                                    tracing::info!("MSFS first snapshot received");
                                }
                                last_data = Some(Instant::now());
                            }
                        }
                        Notification::Quit => {
                            tracing::info!("MSFS sent Quit, will reconnect");
                            break;
                        }
                        Notification::Open => {
                            // Informational; ignore.
                        }
                        _ => {
                            // Forward-compat: simconnect-sdk's Notification is
                            // non-exhaustive; ignore variants we don't handle yet.
                        }
                    },
                    Ok(None) => {
                        thread::sleep(Duration::from_millis(50));
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "SimConnect dispatch error");
                        *shared.last_error.lock().expect("err") = Some(format!("dispatch: {e}"));
                        break;
                    }
                }
            }

            *shared.state.lock().expect("state") = ConnectionState::Connecting;
            *shared.snapshot.lock().expect("snapshot") = None;
        }
        *shared.state.lock().expect("state") = ConnectionState::Disconnected;
    }

    /// Sleep for `dur`, breaking out early when `stop` is set.
    /// Returns `false` if we should exit immediately (stop signalled).
    fn sleep_or_stop(stop: &AtomicBool, dur: Duration) -> bool {
        let step = Duration::from_millis(100);
        let mut left = dur;
        while left > Duration::ZERO {
            if stop.load(Ordering::Relaxed) {
                return false;
            }
            let s = std::cmp::min(step, left);
            thread::sleep(s);
            left = left.saturating_sub(s);
        }
        true
    }
}

#[cfg(target_os = "windows")]
pub use adapter::*;

// ---- Non-Windows stub ----

#[cfg(not(target_os = "windows"))]
mod stub {
    use serde::Serialize;
    use sim_core::{SimKind, SimSnapshot};

    #[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
    #[serde(rename_all = "snake_case")]
    pub enum ConnectionState {
        Disconnected,
        Connecting,
        Connected,
    }

    pub struct MsfsAdapter;

    impl Default for MsfsAdapter {
        fn default() -> Self {
            Self
        }
    }

    impl MsfsAdapter {
        pub fn new() -> Self {
            Self
        }
        pub fn start(&mut self, _kind: SimKind) {}
        pub fn stop(&mut self) {}
        pub fn state(&self) -> ConnectionState {
            ConnectionState::Disconnected
        }
        pub fn snapshot(&self) -> Option<SimSnapshot> {
            None
        }
        pub fn last_error(&self) -> Option<String> {
            Some("MSFS adapter is Windows-only".into())
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub use stub::*;
