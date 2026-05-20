//! Connection / lifecycle for the X-Plane UDP DataRef adapter.
//!
//! Mirrors the public shape of `MsfsAdapter`: `new` / `start` / `stop`
//! / `state` / `snapshot` / `last_error`. The streamer in
//! `src/lib.rs` polls `snapshot()` at the position-streamer cadence
//! and the touchdown sampler polls it at 50 Hz — same code path as
//! MSFS, the adapter is what changes.
//!
//! Implementation: synchronous `std::net::UdpSocket` + a dedicated
//! `std::thread`. We deliberately avoid tokio here: tokio
//! requires the caller to be inside an async runtime, but
//! `sim_set_kind` is a synchronous Tauri command and can be invoked
//! from any thread Tauri picks. The early build that used
//! `tokio::spawn` from inside `start()` crashed the app on sim
//! switch because no runtime was available on the calling thread.
//! `std::thread` works from any context.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Time without any RREF packet after which we declare the X-Plane
/// connection stale. Mirrors `STALE_TIMEOUT` on the MSFS adapter
/// (5 s) — long enough to survive a brief sim pause / loading screen,
/// short enough that a quit-and-reload doesn't leave us showing the
/// pre-quit position to the briefing tab. Live bug 2026-05-03: pilot
/// loaded MSFS at default airport, switched flight to SCEL, saw
/// "3142.5 nm von SCEL" because adapter still served the old position.
/// Same class of bug exists here — fix it preemptively.
const STALE_TIMEOUT: Duration = Duration::from_secs(5);

use sim_core::{SimKind, SimSnapshot, Simulator};

use crate::dataref::{XPlaneState, CATALOG};
use crate::premium::{PremiumListener, PremiumStatus, PremiumTouchdown};
use crate::profile::{build_active_catalog, profile_index_for_title, ActiveEntry, PROFILES};
use crate::rref::{decode_response, encode_request};
use crate::web_api::{AircraftInfo, DrefIdCache, WebApiClient};
use crate::{SUBSCRIPTION_HZ, XPLANE_LISTEN_PORT};

/// v0.12.2 (LE1): RREF index base for the aircraft-profile probes.
/// Probe subscriptions get one index each starting here — far above any
/// `CATALOG` index, so a probe packet is unambiguously identifiable and
/// never collides with a real catalog entry.
const DISCOVERY_INDEX_BASE: i32 = 10_000;

/// How often the Web API poller asks X-Plane for the aircraft info.
/// Aircraft identity rarely changes mid-flight (load a new plane =
/// new flight), so 30 s is plenty.
const AIRCRAFT_POLL_INTERVAL_SECS: u64 = 30;

#[derive(Debug, Clone, Copy, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
}

/// One DataRef catalog entry as exposed for the Settings → Debug
/// panel. The frontend renders these in a table so the pilot can
/// see exactly which DataRefs we subscribe to and their last value.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DatarefSample {
    pub index: u32,
    pub name: &'static str,
    pub value: f32,
    /// True if we've ever received a value for this index from
    /// X-Plane. Useful for spotting DataRefs the sim rejected
    /// (older XP build, missing payware, etc.) — they stay `false`
    /// and zero forever.
    pub has_value: bool,
}

struct AdapterShared {
    state: Mutex<ConnectionState>,
    last_error: Mutex<Option<String>>,
    /// Parsed accumulated DataRef state. Mutated from the listener
    /// thread, read by `snapshot()` and `subscribed_datarefs()`.
    parsed: Mutex<XPlaneState>,
    /// Per-index "has X-Plane sent us this DataRef yet?" flag — for
    /// the debug panel.
    seen: Mutex<Vec<bool>>,
    /// Per-index last raw float value (for debug panel display).
    last_values: Mutex<Vec<f32>>,
    /// v0.12.2 (LE6): the **active catalog** the listener is currently
    /// subscribed to — the static `CATALOG` with the detected aircraft
    /// profile's dataref overrides applied. Same length/indices as
    /// `CATALOG`. The listener owns the working copy and publishes it
    /// here so the debug panel shows the dataref names actually in use.
    active_catalog: Mutex<Vec<ActiveEntry>>,
    /// Aircraft identity from the X-Plane 12.1+ Web API. Empty
    /// `AircraftInfo` (all fields None) until the poller's first
    /// successful response, OR forever if the Web API is unreachable
    /// (X-Plane <12.1, or pilot didn't enable Settings → Network →
    /// Web Server).
    aircraft: Mutex<AircraftInfo>,
    /// Tells the worker thread to stop. Polled in the recv loop.
    stop: AtomicBool,
}

pub struct XPlaneAdapter {
    shared: Arc<AdapterShared>,
    worker: Option<JoinHandle<()>>,
    /// Web API poller (X-Plane 12.1+ Settings → Network → Web Server).
    /// Independently joined so we always tear down both threads on
    /// `stop()` even if one already exited.
    web_api_worker: Option<JoinHandle<()>>,
    /// Listener for the optional AeroACARS X-Plane Plugin (v0.5.0+).
    /// When the pilot has the plugin installed, this thread receives
    /// JSON telemetry/touchdown packets on UDP 52000 and surfaces a
    /// frame-perfect touchdown event. Inert if no plugin is present
    /// — bind succeeds, no packets ever arrive, RREF path handles
    /// everything as before.
    premium: PremiumListener,
    /// Cached SimKind so `snapshot()` knows whether to stamp
    /// `Simulator::XPlane11` or `XPlane12`.
    kind: SimKind,
}

impl Default for XPlaneAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl XPlaneAdapter {
    pub fn new() -> Self {
        let shared = Arc::new(AdapterShared {
            state: Mutex::new(ConnectionState::Disconnected),
            last_error: Mutex::new(None),
            parsed: Mutex::new(XPlaneState::default()),
            seen: Mutex::new(vec![false; CATALOG.len()]),
            last_values: Mutex::new(vec![0.0; CATALOG.len()]),
            active_catalog: Mutex::new(build_active_catalog(None)),
            aircraft: Mutex::new(AircraftInfo::default()),
            stop: AtomicBool::new(false),
        });
        Self {
            shared,
            worker: None,
            web_api_worker: None,
            premium: PremiumListener::new(),
            kind: SimKind::XPlane12,
        }
    }

    /// Start the listener for a given X-Plane version. Idempotent —
    /// if a worker is already running we stop it and start fresh
    /// (the `kind` may have changed between calls).
    pub fn start(&mut self, kind: SimKind) {
        if !kind.is_xplane() {
            tracing::warn!(?kind, "XPlaneAdapter::start called with non-XPlane kind, ignoring");
            return;
        }
        self.stop();
        self.kind = kind;
        // Reset state for a fresh run.
        *self.shared.state.lock().unwrap() = ConnectionState::Connecting;
        *self.shared.last_error.lock().unwrap() = None;
        *self.shared.parsed.lock().unwrap() = XPlaneState::default();
        *self.shared.aircraft.lock().unwrap() = AircraftInfo::default();
        // v0.12.2: a fresh run starts on the base catalog — profile
        // detection re-runs from scratch against the new sim session.
        *self.shared.active_catalog.lock().unwrap() = build_active_catalog(None);
        for v in self.shared.seen.lock().unwrap().iter_mut() {
            *v = false;
        }
        for v in self.shared.last_values.lock().unwrap().iter_mut() {
            *v = 0.0;
        }
        self.shared.stop.store(false, Ordering::SeqCst);
        let shared_for_udp = Arc::clone(&self.shared);
        let udp_handle = std::thread::Builder::new()
            .name("xplane-udp".into())
            .spawn(move || run_listener(shared_for_udp))
            .expect("spawn xplane-udp thread");
        self.worker = Some(udp_handle);
        let shared_for_web = Arc::clone(&self.shared);
        let web_handle = std::thread::Builder::new()
            .name("xplane-web-api".into())
            .spawn(move || run_web_api_poller(shared_for_web))
            .expect("spawn xplane-web-api thread");
        self.web_api_worker = Some(web_handle);
        // Start the premium plugin listener too. No-op unless the
        // optional X-Plane Plugin is installed — see `premium.rs`.
        self.premium.start();
        tracing::info!(?kind, "X-Plane adapter started");
    }

    pub fn stop(&mut self) {
        let had_worker = self.worker.is_some() || self.web_api_worker.is_some();
        if had_worker {
            self.shared.stop.store(true, Ordering::SeqCst);
        }
        if let Some(handle) = self.worker.take() {
            // Wait briefly for the thread to exit gracefully so we
            // unsubscribe RREFs before returning. 250 ms is plenty —
            // the recv loop has a 100 ms read timeout.
            let _ = handle.join();
        }
        if let Some(handle) = self.web_api_worker.take() {
            // The Web API poller wakes from its sleep every 100 ms to
            // re-check the stop flag; join is fast.
            let _ = handle.join();
        }
        // Tear down the premium listener last so it can drain any
        // in-flight packet before close. Idempotent.
        self.premium.stop();
        *self.shared.state.lock().unwrap() = ConnectionState::Disconnected;
    }

    /// Status of the AeroACARS X-Plane Plugin connection (v0.5.0+).
    /// `active=true` when we've received a packet within the last
    /// 3 s — drives the "X-PLANE PREMIUM" badge in the UI.
    pub fn premium_status(&self) -> PremiumStatus {
        self.premium.status()
    }

    /// Drain a pending plugin-emitted touchdown event, if any. The
    /// flight sampler in the main app calls this each tick after
    /// the standard `snapshot()` read; if Some, the values override
    /// the sampler's own RREF-based touchdown detection (frame-
    /// perfect timing, lookback-peak VS).
    pub fn take_premium_touchdown(&self) -> Option<PremiumTouchdown> {
        self.premium.take_touchdown()
    }

    /// Last error from the premium listener (e.g. bind failure).
    /// Independent from `last_error()` so RREF and premium errors
    /// don't clobber each other.
    pub fn premium_last_error(&self) -> Option<String> {
        self.premium.last_error()
    }

    pub fn state(&self) -> ConnectionState {
        *self.shared.state.lock().unwrap()
    }

    /// Force-clear the parsed RREF state so `snapshot()` returns
    /// `None` until X-Plane delivers a fresh batch of values. Used by
    /// the UI's "Re-check sim position" button when the pilot
    /// suspects the cached lat/lon is stale (e.g. flight switched in
    /// X-Plane but our 5 s STALE_TIMEOUT hasn't fired because UDP
    /// kept trickling stray packets through the load). Connection
    /// state is downgraded to Connecting so the UI shows "waiting
    /// for sim position …" until the next real packet lands.
    pub fn clear_snapshot(&self) {
        *self.shared.parsed.lock().unwrap() = XPlaneState::default();
        for v in self.shared.seen.lock().unwrap().iter_mut() {
            *v = false;
        }
        for v in self.shared.last_values.lock().unwrap().iter_mut() {
            *v = 0.0;
        }
        *self.shared.state.lock().unwrap() = ConnectionState::Connecting;
        tracing::info!("X-Plane snapshot cleared by user (force-resync)");
    }

    pub fn snapshot(&self) -> Option<SimSnapshot> {
        let parsed = self.shared.parsed.lock().unwrap();
        if !parsed.got_first_packet {
            return None;
        }
        let sim = match self.kind {
            SimKind::XPlane11 => Simulator::XPlane11,
            _ => Simulator::XPlane12,
        };
        let mut snap = parsed.to_snapshot(sim);
        // Overlay aircraft identity from the Web API poller (X-Plane
        // 12.1+ Settings → Network → Web Server). Stays None until the
        // first successful poll, OR forever when the Web API isn't
        // reachable (X-Plane <12.1, or pilot didn't enable it). The
        // SimSnapshot fields default to None in that path so the
        // existing "(unknown)" UI label still shows.
        let aircraft = self.shared.aircraft.lock().unwrap();
        if aircraft.descrip.is_some() {
            snap.aircraft_title = aircraft.descrip.clone();
        }
        if aircraft.icao.is_some() {
            snap.aircraft_icao = aircraft.icao.clone();
        }
        if aircraft.tailnum.is_some() {
            snap.aircraft_registration = aircraft.tailnum.clone();
        }
        Some(snap)
    }

    pub fn last_error(&self) -> Option<String> {
        self.shared.last_error.lock().unwrap().clone()
    }

    /// Return the catalog with each DataRef's most recent received
    /// value. Used by the Settings → Debug panel — analogous to the
    /// MSFS Inspector but auto-populated.
    ///
    /// v0.12.2: reads the **active catalog** so the panel shows the
    /// dataref names actually in use — including a detected aircraft
    /// profile's overrides (e.g. the CL650 flaps dataref).
    pub fn subscribed_datarefs(&self) -> Vec<DatarefSample> {
        let seen = self.shared.seen.lock().unwrap();
        let last = self.shared.last_values.lock().unwrap();
        let active = self.shared.active_catalog.lock().unwrap();
        active
            .iter()
            .enumerate()
            .map(|(i, e)| DatarefSample {
                index: i as u32,
                name: e.name,
                value: last.get(i).copied().unwrap_or(0.0),
                has_value: seen.get(i).copied().unwrap_or(false),
            })
            .collect()
    }
}

impl Drop for XPlaneAdapter {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The blocking listener thread. Binds a UDP socket on an ephemeral
/// local port, subscribes the active catalog (+ aircraft-profile
/// probes) to 127.0.0.1:49000, then loops decoding responses until
/// `shared.stop` flips to `true`.
///
/// v0.12.2: the listener also runs aircraft-profile detection (LE1) —
/// title-match via the Web API overlay plus an RREF probe — and on a
/// match rebuilds the active catalog with the profile's dataref
/// overrides and re-subscribes (LE6).
fn run_listener(shared: Arc<AdapterShared>) {
    use std::net::UdpSocket;

    let socket = match UdpSocket::bind("127.0.0.1:0") {
        Ok(s) => s,
        Err(e) => {
            *shared.last_error.lock().unwrap() = Some(format!("bind failed: {e}"));
            *shared.state.lock().unwrap() = ConnectionState::Disconnected;
            tracing::error!(error = %e, "could not bind XPlane UDP socket");
            return;
        }
    };
    // Non-blocking-ish: 100 ms read timeout so we re-check the
    // stop flag at least every 100 ms.
    if let Err(e) = socket.set_read_timeout(Some(Duration::from_millis(100))) {
        tracing::warn!(error = %e, "could not set XPlane UDP read timeout");
    }
    let local_addr = socket
        .local_addr()
        .map(|a| a.to_string())
        .unwrap_or_else(|_| "?".into());
    tracing::info!(local = %local_addr, "X-Plane UDP socket bound");

    let xplane_addr = format!("127.0.0.1:{XPLANE_LISTEN_PORT}");

    // ---- v0.12.2: active catalog + aircraft-profile state ----
    // `active` is the static CATALOG with the detected profile's
    // dataref overrides applied (LE6) — same length/indices as CATALOG.
    let mut active: Vec<ActiveEntry> = build_active_catalog(None);
    // Index into PROFILES of the active profile, None until detected.
    let mut active_profile: Option<usize> = None;
    // Which profiles' probe datarefs X-Plane has answered (LE1 stage 2).
    let mut probe_responded: Vec<bool> = vec![false; PROFILES.len()];
    // Last aircraft title evaluated — to detect an aircraft swap.
    let mut last_title: Option<String> = None;

    // ---- Hard-armoured re-subscribe (v0.3.0) ----
    // Send the full RREF subscription set for the given catalog. Called
    // at startup, on profile activation, and periodically while not
    // Connected. Idempotent on X-Plane's side — a duplicate RREF at the
    // same index just refreshes the rate / re-binds the dataref.
    let subscribe_catalog = |sock: &UdpSocket, cat: &[ActiveEntry]| {
        for (i, entry) in cat.iter().enumerate() {
            let req = encode_request(SUBSCRIPTION_HZ as i32, i as i32, entry.name);
            if let Err(e) = sock.send_to(&req, &xplane_addr) {
                tracing::trace!(
                    error = %e,
                    dataref = entry.name,
                    "RREF subscribe send failed (will retry on next tick)"
                );
            }
        }
    };
    // v0.12.2 (LE1 stage 2): one probe subscription per profile, at a
    // reserved discovery index. Low rate — we only need to learn the
    // dataref exists, not stream it.
    let subscribe_probes = |sock: &UdpSocket| {
        for (pi, prof) in PROFILES.iter().enumerate() {
            let req = encode_request(1, DISCOVERY_INDEX_BASE + pi as i32, prof.probe_dataref);
            let _ = sock.send_to(&req, &xplane_addr);
        }
    };
    // Cancel all probe subscriptions (freq = 0) — done once a profile
    // is active, the probe has served its purpose.
    let unsubscribe_probes = |sock: &UdpSocket| {
        for (pi, prof) in PROFILES.iter().enumerate() {
            let stop = encode_request(0, DISCOVERY_INDEX_BASE + pi as i32, prof.probe_dataref);
            let _ = sock.send_to(&stop, &xplane_addr);
        }
    };

    // Initial subscribe — base catalog + profile probes.
    subscribe_catalog(&socket, &active);
    subscribe_probes(&socket);
    *shared.active_catalog.lock().unwrap() = active.clone();

    // Listen.
    let mut buf = vec![0u8; 8192];
    let mut last_packet_at: Option<Instant> = None;
    let mut last_resubscribe_at = Instant::now();
    /// How often to re-send the full subscription set while we're
    /// not yet Connected. 5 seconds matches `STALE_TIMEOUT` so the
    /// recovery feels coherent.
    const RESUBSCRIBE_INTERVAL: Duration = Duration::from_secs(5);

    while !shared.stop.load(Ordering::SeqCst) {
        match socket.recv_from(&mut buf) {
            Ok((n, _peer)) => {
                let pairs = decode_response(&buf[..n]);
                if pairs.is_empty() {
                    continue;
                }
                last_packet_at = Some(Instant::now());
                let mut parsed = shared.parsed.lock().unwrap();
                let mut seen = shared.seen.lock().unwrap();
                let mut last = shared.last_values.lock().unwrap();
                for p in pairs {
                    // v0.12.2 (LE1): discovery-index packets are PROBE
                    // responses — they only prove a profile's dataref
                    // exists. Intercepted BEFORE `apply_field`; they
                    // never mutate a snapshot value.
                    if p.index >= DISCOVERY_INDEX_BASE {
                        let pi = (p.index - DISCOVERY_INDEX_BASE) as usize;
                        if let Some(slot) = probe_responded.get_mut(pi) {
                            *slot = true;
                        }
                        continue;
                    }
                    // Normal catalog packet: index → active entry →
                    // FieldId, with the profile's ValueMapping applied.
                    if let Some(entry) = active.get(p.index as usize) {
                        if let Some(mapped) = entry.mapping.map(p.value) {
                            parsed.apply_field(entry.field, mapped);
                        }
                        // seen/last reflect the RAW value X-Plane sent.
                        if let Some(slot) = seen.get_mut(p.index as usize) {
                            *slot = true;
                        }
                        if let Some(slot) = last.get_mut(p.index as usize) {
                            *slot = p.value;
                        }
                    }
                }
                if parsed.got_first_packet {
                    let mut s = shared.state.lock().unwrap();
                    if *s != ConnectionState::Connected {
                        *s = ConnectionState::Connected;
                        tracing::info!("X-Plane: first RREF packet received → Connected");
                    }
                }
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                // No data this tick — check stale timeout + resubscribe-
                // due timer below, then loop.
            }
            Err(e) => {
                tracing::warn!(error = %e, "X-Plane UDP recv error");
                std::thread::sleep(Duration::from_millis(100));
            }
        }

        // ---- v0.12.2 (LE1/LE6): aircraft-profile detection ----
        // Read the current aircraft title (Web API overlay; may be None
        // when the Web API is off or under X-Plane 11).
        let current_title = shared.aircraft.lock().unwrap().descrip.clone();
        // Aircraft swap → reset detection back to the base catalog.
        if current_title != last_title {
            if last_title.is_some() && active_profile.is_some() {
                tracing::info!("X-Plane: aircraft changed — resetting dataref profile");
                active_profile = None;
                for r in probe_responded.iter_mut() {
                    *r = false;
                }
                active = build_active_catalog(None);
                *shared.active_catalog.lock().unwrap() = active.clone();
                subscribe_catalog(&socket, &active);
                subscribe_probes(&socket);
            }
            last_title = current_title.clone();
        }
        // Detect a profile while none is active — title OR probe (LE1).
        if active_profile.is_none() {
            let by_title = current_title.as_deref().and_then(profile_index_for_title);
            let by_probe = probe_responded.iter().position(|&r| r);
            if let Some(pi) = by_title.or(by_probe) {
                let via = if by_title.is_some() { "title" } else { "probe" };
                tracing::info!(
                    profile = PROFILES[pi].name,
                    via,
                    "X-Plane: aircraft DataRef profile activated"
                );
                active_profile = Some(pi);
                // Rebuild the active catalog + re-subscribe with fresh
                // dataref names (LE6); the probes have done their job.
                active = build_active_catalog(Some(&PROFILES[pi]));
                *shared.active_catalog.lock().unwrap() = active.clone();
                subscribe_catalog(&socket, &active);
                unsubscribe_probes(&socket);
            }
        }

        // Stale-snapshot guard: if we WERE connected but haven't
        // seen any packet for STALE_TIMEOUT, treat the connection
        // as dropped — clear the parsed state (so snapshot() returns
        // None until fresh data arrives) and downgrade the connection
        // state. The next packet repopulates parsed and snaps us back
        // to Connected without intervention.
        if let Some(at) = last_packet_at {
            if at.elapsed() > STALE_TIMEOUT {
                let mut parsed = shared.parsed.lock().unwrap();
                if parsed.got_first_packet {
                    tracing::warn!(
                        "X-Plane: no RREF packets for {:?} — clearing snapshot, marking connecting",
                        STALE_TIMEOUT
                    );
                    *parsed = XPlaneState::default();
                    let mut seen = shared.seen.lock().unwrap();
                    for v in seen.iter_mut() {
                        *v = false;
                    }
                    let mut last = shared.last_values.lock().unwrap();
                    for v in last.iter_mut() {
                        *v = 0.0;
                    }
                    *shared.state.lock().unwrap() = ConnectionState::Connecting;
                }
                // Reset so we don't fire the warning every tick.
                last_packet_at = None;
            }
        }

        // ---- Hard-armoured re-subscribe poll (v0.3.0) ----
        // Whenever we're not Connected, periodically resend the full
        // subscription set (active catalog + probes) so the connection
        // recovers from a cold start or an X-Plane restart on its own.
        let state_now = *shared.state.lock().unwrap();
        if state_now != ConnectionState::Connected
            && last_resubscribe_at.elapsed() >= RESUBSCRIBE_INTERVAL
        {
            tracing::debug!("X-Plane: not connected — re-sending RREF subscriptions");
            subscribe_catalog(&socket, &active);
            if active_profile.is_none() {
                subscribe_probes(&socket);
            }
            last_resubscribe_at = Instant::now();
        }
    }

    // Best-effort: send freq=0 RREF for every active catalog entry and
    // every probe so we don't leave X-Plane streaming into the void.
    for (i, entry) in active.iter().enumerate() {
        let req = encode_request(0, i as i32, entry.name);
        let _ = socket.send_to(&req, &xplane_addr);
    }
    unsubscribe_probes(&socket);
    tracing::info!("X-Plane UDP listener stopped");
}

/// Long-running poller that reads aircraft identity from the X-Plane
/// 12.1+ Web API (`http://localhost:8086`) and stashes the result in
/// `shared.aircraft`. Runs in its own thread so it can do blocking
/// HTTP without stalling the 50 Hz UDP listener.
///
/// Cadence is sparse (`AIRCRAFT_POLL_INTERVAL_SECS`) because aircraft
/// identity rarely changes mid-flight. On repeated failures
/// (X-Plane <12.1, or Web API not enabled in Settings → Network)
/// we back off further so we don't spam.
fn run_web_api_poller(shared: Arc<AdapterShared>) {
    let client = WebApiClient::new();
    let mut id_cache = DrefIdCache::default();
    let mut consecutive_failures: u32 = 0;
    let mut last_logged_path: Option<String> = None;
    tracing::info!("X-Plane Web API poller started");
    while !shared.stop.load(Ordering::SeqCst) {
        match client.fetch_aircraft_info(&mut id_cache) {
            Ok(info) => {
                if consecutive_failures > 0 {
                    tracing::info!(
                        "X-Plane Web API recovered after {} failed polls",
                        consecutive_failures
                    );
                }
                consecutive_failures = 0;
                if info.has_any() {
                    // Log on first detection AND on aircraft change
                    // (e.g. pilot loaded a different plane). Identity
                    // by `relative_path` since it's the .acf path —
                    // unique even when two planes share a description.
                    let path = info.relative_path.clone();
                    let changed = last_logged_path != path;
                    if changed {
                        tracing::info!(
                            descrip = ?info.descrip,
                            icao = ?info.icao,
                            tailnum = ?info.tailnum,
                            "X-Plane aircraft detected via Web API"
                        );
                        last_logged_path = path;
                    }
                }
                *shared.aircraft.lock().unwrap() = info;
            }
            Err(e) => {
                consecutive_failures += 1;
                // Log once at info level on the first failure so the
                // pilot has something to grep for. Subsequent failures
                // stay at debug — a permanently-disabled Web API
                // would otherwise spam.
                if consecutive_failures == 1 {
                    tracing::info!(
                        error = %e,
                        "X-Plane Web API unavailable — aircraft identity will stay (unknown). \
                         Enable in X-Plane → Settings → Network → Web Server (X-Plane 12.1+)."
                    );
                } else {
                    tracing::debug!(error = %e, "X-Plane Web API poll failed");
                }
            }
        }
        // Sleep between polls. Back off after repeated failures so we
        // don't keep slamming a sim that obviously isn't going to
        // answer. Wake every 100 ms to re-check the stop flag.
        let secs = if consecutive_failures > 5 {
            AIRCRAFT_POLL_INTERVAL_SECS * 4
        } else {
            AIRCRAFT_POLL_INTERVAL_SECS
        };
        let ticks = secs * 10;
        for _ in 0..ticks {
            if shared.stop.load(Ordering::SeqCst) {
                tracing::info!("X-Plane Web API poller stopped");
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    tracing::info!("X-Plane Web API poller stopped");
}
