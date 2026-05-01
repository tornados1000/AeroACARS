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

        // ---- Aircraft state (Phase H.1) ----
        #[simconnect(name = "BRAKE PARKING POSITION", unit = "bool")]
        parking_brake: bool,
        #[simconnect(name = "STALL WARNING", unit = "bool")]
        stall_warning: bool,
        #[simconnect(name = "OVERSPEED WARNING", unit = "bool")]
        overspeed_warning: bool,
        /// 0.0–1.0: 0 = up, 1 = fully down (averaged across gear).
        #[simconnect(name = "GEAR POSITION", unit = "percent over 100")]
        gear_position: f64,
        /// 0.0–1.0: position of the flaps handle.
        #[simconnect(name = "FLAPS HANDLE PERCENT", unit = "percent over 100")]
        flaps_position: f64,
        /// Number of engines currently combusting (≤ NUMBER OF ENGINES).
        #[simconnect(name = "GENERAL ENG COMBUSTION:1", unit = "bool")]
        eng1_firing: bool,
        #[simconnect(name = "GENERAL ENG COMBUSTION:2", unit = "bool")]
        eng2_firing: bool,
        #[simconnect(name = "GENERAL ENG COMBUSTION:3", unit = "bool")]
        eng3_firing: bool,
        #[simconnect(name = "GENERAL ENG COMBUSTION:4", unit = "bool")]
        eng4_firing: bool,

        // ---- Fuel & weight ----
        /// Total fuel on board, pounds. Converted to kg in the snapshot.
        #[simconnect(name = "FUEL TOTAL QUANTITY WEIGHT", unit = "pounds")]
        fuel_total_lb: f64,
        /// Sum of per-engine fuel-flow, pounds/hour. Converted to kg/h.
        #[simconnect(name = "ENG FUEL FLOW PPH:1", unit = "pounds per hour")]
        eng1_ff_pph: f64,
        #[simconnect(name = "ENG FUEL FLOW PPH:2", unit = "pounds per hour")]
        eng2_ff_pph: f64,
        #[simconnect(name = "ENG FUEL FLOW PPH:3", unit = "pounds per hour")]
        eng3_ff_pph: f64,
        #[simconnect(name = "ENG FUEL FLOW PPH:4", unit = "pounds per hour")]
        eng4_ff_pph: f64,

        // ---- Environment ----
        #[simconnect(name = "AMBIENT WIND DIRECTION", unit = "degrees")]
        wind_direction_deg: f64,
        #[simconnect(name = "AMBIENT WIND VELOCITY", unit = "knots")]
        wind_speed_kt: f64,
        #[simconnect(name = "KOHLSMAN SETTING MB", unit = "millibars")]
        qnh_hpa: f64,
        #[simconnect(name = "AMBIENT TEMPERATURE", unit = "celsius")]
        oat_c: f64,

        // ---- Avionics ----
        /// BCD-encoded squawk (e.g. 0x1234 = 1234). SDK only supports f64
        /// for numerics, so we cast back to u32 in `squawk_from_bcd`.
        #[simconnect(name = "TRANSPONDER CODE:1", unit = "BCO16")]
        transponder_bcd: f64,
        #[simconnect(name = "COM ACTIVE FREQUENCY:1", unit = "MHz")]
        com1_mhz: f64,
        #[simconnect(name = "COM ACTIVE FREQUENCY:2", unit = "MHz")]
        com2_mhz: f64,
        #[simconnect(name = "NAV ACTIVE FREQUENCY:1", unit = "MHz")]
        nav1_mhz: f64,
        #[simconnect(name = "NAV ACTIVE FREQUENCY:2", unit = "MHz")]
        nav2_mhz: f64,

        // ---- Exterior lights ----
        #[simconnect(name = "LIGHT LANDING", unit = "bool")]
        light_landing: bool,
        #[simconnect(name = "LIGHT BEACON", unit = "bool")]
        light_beacon: bool,
        #[simconnect(name = "LIGHT STROBE", unit = "bool")]
        light_strobe: bool,
        #[simconnect(name = "LIGHT TAXI", unit = "bool")]
        light_taxi: bool,
        #[simconnect(name = "LIGHT NAV", unit = "bool")]
        light_nav: bool,
        #[simconnect(name = "LIGHT LOGO", unit = "bool")]
        light_logo: bool,

        // ---- Autopilot ----
        #[simconnect(name = "AUTOPILOT MASTER", unit = "bool")]
        ap_master: bool,
        #[simconnect(name = "AUTOPILOT HEADING LOCK", unit = "bool")]
        ap_heading: bool,
        #[simconnect(name = "AUTOPILOT ALTITUDE LOCK", unit = "bool")]
        ap_altitude: bool,
        #[simconnect(name = "AUTOPILOT NAV1 LOCK", unit = "bool")]
        ap_nav: bool,
        #[simconnect(name = "AUTOPILOT APPROACH HOLD", unit = "bool")]
        ap_approach: bool,
    }

    /// Pounds → kilograms (avoirdupois, 6-digit precision).
    const LB_TO_KG: f64 = 0.45359237;

    /// Decode a BCD-packed squawk from SimConnect (each nibble is a digit).
    fn squawk_from_bcd(bcd: f64) -> u16 {
        let bcd = bcd as u32;
        let d3 = ((bcd >> 12) & 0xF) as u16;
        let d2 = ((bcd >> 8) & 0xF) as u16;
        let d1 = ((bcd >> 4) & 0xF) as u16;
        let d0 = (bcd & 0xF) as u16;
        d3 * 1000 + d2 * 100 + d1 * 10 + d0
    }

    /// Build a `SimSnapshot` from raw telemetry. The simulator field is tagged
    /// from the user-selected `SimKind` because SimConnect can't distinguish
    /// MSFS 2020 from MSFS 2024 at the API level.
    fn telemetry_to_snapshot(t: &Telemetry, kind: SimKind) -> SimSnapshot {
        let total_ff_pph = t.eng1_ff_pph + t.eng2_ff_pph + t.eng3_ff_pph + t.eng4_ff_pph;
        let engines_running = [
            t.eng1_firing,
            t.eng2_firing,
            t.eng3_firing,
            t.eng4_firing,
        ]
        .iter()
        .filter(|x| **x)
        .count() as u8;
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
            parking_brake: t.parking_brake,
            stall_warning: t.stall_warning,
            overspeed_warning: t.overspeed_warning,
            // Pause/slew/sim-rate aren't read yet; safe defaults — they
            // matter for replay-style validation, not in-flight telemetry.
            paused: false,
            slew_mode: false,
            simulation_rate: 1.0,
            gear_position: t.gear_position as f32,
            flaps_position: t.flaps_position as f32,
            engines_running,
            fuel_total_kg: (t.fuel_total_lb * LB_TO_KG) as f32,
            // Block→current diff is computed in the recorder; the per-tick
            // snapshot only carries totals.
            fuel_used_kg: 0.0,
            zfw_kg: None,
            payload_kg: None,
            wind_direction_deg: Some(t.wind_direction_deg as f32),
            wind_speed_kt: Some(t.wind_speed_kt as f32),
            qnh_hpa: Some(t.qnh_hpa as f32),
            outside_air_temp_c: Some(t.oat_c as f32),
            aircraft_title: Some(t.title.clone()).filter(|s| !s.is_empty()),
            aircraft_icao: Some(t.atc_model.clone()).filter(|s| !s.is_empty()),
            aircraft_registration: Some(t.atc_id.clone()).filter(|s| !s.is_empty()),
            simulator: kind.as_simulator(),
            sim_version: None,
            // Avionics
            transponder_code: Some(squawk_from_bcd(t.transponder_bcd)),
            com1_mhz: Some(t.com1_mhz as f32),
            com2_mhz: Some(t.com2_mhz as f32),
            nav1_mhz: Some(t.nav1_mhz as f32),
            nav2_mhz: Some(t.nav2_mhz as f32),
            // Lights
            light_landing: Some(t.light_landing),
            light_beacon: Some(t.light_beacon),
            light_strobe: Some(t.light_strobe),
            light_taxi: Some(t.light_taxi),
            light_nav: Some(t.light_nav),
            light_logo: Some(t.light_logo),
            // Autopilot
            autopilot_master: Some(t.ap_master),
            autopilot_heading: Some(t.ap_heading),
            autopilot_altitude: Some(t.ap_altitude),
            autopilot_nav: Some(t.ap_nav),
            autopilot_approach: Some(t.ap_approach),
            // Powerplant totals
            fuel_flow_kg_per_h: Some((total_ff_pph * LB_TO_KG) as f32),
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
