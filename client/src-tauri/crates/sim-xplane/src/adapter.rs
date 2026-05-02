//! Connection / lifecycle for the X-Plane UDP DataRef adapter.
//!
//! Mirrors the public shape of `MsfsAdapter`: `new` / `start` / `stop`
//! / `state` / `snapshot` / `last_error`. The streamer in
//! `src/lib.rs` polls `snapshot()` at the position-streamer cadence
//! and the touchdown sampler polls it at 50 Hz — same code path as
//! MSFS, the adapter is what changes.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use sim_core::{SimKind, SimSnapshot, Simulator};
use tokio::net::UdpSocket;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::dataref::{XPlaneState, CATALOG};
use crate::rref::{decode_response, encode_request};
use crate::{SUBSCRIPTION_HZ, XPLANE_LISTEN_PORT};

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
    /// task, read by `snapshot()` and `subscribed_datarefs()`.
    parsed: Mutex<XPlaneState>,
    /// Per-index "has X-Plane sent us this DataRef yet?" flag — for
    /// the debug panel. Index = position in CATALOG.
    seen: Mutex<Vec<bool>>,
    /// Per-index last raw float value (for debug panel display).
    last_values: Mutex<Vec<f32>>,
    /// Tells the worker task to stop. Watched in the select loop.
    stop_tx: watch::Sender<bool>,
}

pub struct XPlaneAdapter {
    shared: Arc<AdapterShared>,
    worker: Option<JoinHandle<()>>,
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
        let (stop_tx, _stop_rx) = watch::channel(false);
        let shared = Arc::new(AdapterShared {
            state: Mutex::new(ConnectionState::Disconnected),
            last_error: Mutex::new(None),
            parsed: Mutex::new(XPlaneState::default()),
            seen: Mutex::new(vec![false; CATALOG.len()]),
            last_values: Mutex::new(vec![0.0; CATALOG.len()]),
            stop_tx,
        });
        Self {
            shared,
            worker: None,
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
        for v in self.shared.seen.lock().unwrap().iter_mut() {
            *v = false;
        }
        for v in self.shared.last_values.lock().unwrap().iter_mut() {
            *v = 0.0;
        }
        // Reset the stop signal to `false` so a fresh listener
        // doesn't read a stale `true` left over from a previous
        // stop().
        self.shared.stop_tx.send(false).ok();
        let shared_for_task = Arc::clone(&self.shared);
        let stop_rx = self.shared.stop_tx.subscribe();
        let kind_for_task = kind;
        let handle = tokio::spawn(async move {
            run_listener(shared_for_task, stop_rx, kind_for_task).await;
        });
        self.worker = Some(handle);
        tracing::info!(?kind, "X-Plane adapter started");
    }

    pub fn stop(&mut self) {
        if let Some(handle) = self.worker.take() {
            self.shared.stop_tx.send(true).ok();
            // Detach: we don't await because `stop()` is called from
            // sync contexts. The task will exit on the next watch
            // wakeup and free the socket.
            handle.abort();
        }
        *self.shared.state.lock().unwrap() = ConnectionState::Disconnected;
    }

    pub fn state(&self) -> ConnectionState {
        *self.shared.state.lock().unwrap()
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
        Some(parsed.to_snapshot(sim))
    }

    pub fn last_error(&self) -> Option<String> {
        self.shared.last_error.lock().unwrap().clone()
    }

    /// Return the catalog with each DataRef's most recent received
    /// value. Used by the Settings → Debug panel — analogous to the
    /// MSFS Inspector but auto-populated (no add-watch UI needed
    /// because the X-Plane catalog is fixed at compile time).
    pub fn subscribed_datarefs(&self) -> Vec<DatarefSample> {
        let seen = self.shared.seen.lock().unwrap();
        let last = self.shared.last_values.lock().unwrap();
        CATALOG
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

/// The async listener task. Binds a UDP socket on an ephemeral local
/// port, sends one RREF subscription per CATALOG entry to
/// 127.0.0.1:49000, then loops forever decoding response packets
/// into `shared.parsed` until `stop_rx` flips to `true`.
async fn run_listener(
    shared: Arc<AdapterShared>,
    mut stop_rx: watch::Receiver<bool>,
    _kind: SimKind,
) {
    // Bind on 0 → kernel picks a free port. X-Plane will respond
    // back to whatever source port our outgoing RREF packet had,
    // so we don't need a fixed port.
    let socket = match UdpSocket::bind("127.0.0.1:0").await {
        Ok(s) => s,
        Err(e) => {
            *shared.last_error.lock().unwrap() = Some(format!("bind failed: {e}"));
            *shared.state.lock().unwrap() = ConnectionState::Disconnected;
            tracing::error!(error = %e, "could not bind XPlane UDP socket");
            return;
        }
    };
    let local_addr = socket
        .local_addr()
        .map(|a| a.to_string())
        .unwrap_or_else(|_| "?".into());
    tracing::info!(local = %local_addr, "X-Plane UDP socket bound");

    let xplane_addr = format!("127.0.0.1:{XPLANE_LISTEN_PORT}");

    // Subscribe every DataRef in the catalog. Index = position.
    for (i, entry) in CATALOG.iter().enumerate() {
        let req = encode_request(SUBSCRIPTION_HZ as i32, i as i32, entry.name);
        if let Err(e) = socket.send_to(&req, &xplane_addr).await {
            tracing::warn!(error = %e, dataref = entry.name, "RREF subscribe send failed");
            // Don't bail on a single failure — others might still arrive.
        }
    }

    // Listen.
    let mut buf = vec![0u8; 8192];
    loop {
        tokio::select! {
            _ = stop_rx.changed() => {
                if *stop_rx.borrow() {
                    break;
                }
            }
            recv = socket.recv_from(&mut buf) => {
                match recv {
                    Ok((n, _peer)) => {
                        let pairs = decode_response(&buf[..n]);
                        if pairs.is_empty() {
                            continue;
                        }
                        let mut parsed = shared.parsed.lock().unwrap();
                        let mut seen = shared.seen.lock().unwrap();
                        let mut last = shared.last_values.lock().unwrap();
                        for p in pairs {
                            parsed.apply(p.index, p.value);
                            if let Some(slot) = seen.get_mut(p.index as usize) {
                                *slot = true;
                            }
                            if let Some(slot) = last.get_mut(p.index as usize) {
                                *slot = p.value;
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
                    Err(e) => {
                        tracing::warn!(error = %e, "X-Plane UDP recv error");
                        // Brief sleep to avoid hot-spin on persistent errors.
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }
            }
        }
    }

    // Best-effort: send freq=0 RREF for every catalog entry so we
    // don't leave X-Plane streaming into the void after we exit.
    for (i, entry) in CATALOG.iter().enumerate() {
        let req = encode_request(0, i as i32, entry.name);
        let _ = socket.send_to(&req, &xplane_addr).await;
    }
    tracing::info!("X-Plane UDP listener stopped");
}
