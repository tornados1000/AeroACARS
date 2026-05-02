//! Windows-only raw SimConnect adapter.
//!
//! Owns a worker thread that connects to SimConnect, registers a
//! single data definition, subscribes to per-second updates and
//! pushes parsed [`SimSnapshot`]s into a shared mutex. The public
//! [`MsfsAdapter`] API is the same as the legacy adapter so the rest
//! of the application doesn't need to change.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use chrono::Utc;
use serde::Serialize;
use sim_core::{AircraftProfile, SimKind, SimSnapshot, Simulator};

mod sys;
mod telemetry;

use telemetry::{InspectorState, Touchdown, TELEMETRY_FIELDS, TOUCHDOWN_FIELDS};
pub use telemetry::{InspectorWatch, WatchKind, WatchValue};

// IDs used in our SimConnect calls — chosen freely as long as they're
// unique within the connection. Data definition #1 holds the per-tick
// telemetry; #2 the touchdown snapshot, which only the simulation
// itself fills (and only at the moment the gear hits the ground).
// Splitting them means a touchdown SimVar rejection can never shift
// the live telemetry layout — same reason we left the old crate
// behind.
const DEFINITION_ID: sys::SIMCONNECT_DATA_DEFINITION_ID = 1;
const REQUEST_ID: sys::SIMCONNECT_DATA_REQUEST_ID = 1;
const TOUCHDOWN_DEFINITION_ID: sys::SIMCONNECT_DATA_DEFINITION_ID = 2;
const TOUCHDOWN_REQUEST_ID: sys::SIMCONNECT_DATA_REQUEST_ID = 2;
/// Definition #3: live inspector watchlist, re-registered on every
/// add/remove. Lives in its own slot so a typo in a user-supplied
/// SimVar name can't take down the per-tick telemetry.
const INSPECTOR_DEFINITION_ID: sys::SIMCONNECT_DATA_DEFINITION_ID = 3;
const INSPECTOR_REQUEST_ID: sys::SIMCONNECT_DATA_REQUEST_ID = 3;
const STALE_TIMEOUT: Duration = Duration::from_secs(5);

/// Public connection state mirrored to the frontend.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
}

/// External-facing MSFS adapter. Cheap to clone-state; drives a
/// background worker thread that talks to SimConnect.
pub struct MsfsAdapter {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
}

struct Shared {
    state: Mutex<ConnectionState>,
    snapshot: Mutex<Option<SimSnapshot>>,
    last_error: Mutex<Option<String>>,
    /// Last touchdown sample as seen on data definition #2. Updated
    /// asynchronously by SimConnect — we merge it into each emitted
    /// `SimSnapshot` so downstream consumers see a unified view.
    touchdown: Mutex<Option<Touchdown>>,
    /// User-driven SimVar/LVar inspector watchlist. UI mutates the
    /// vec via add_watch / remove_watch (which sets `dirty=true`),
    /// the worker re-registers definition #3 on the next tick.
    inspector: Mutex<InspectorState>,
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
                touchdown: Mutex::new(None),
                inspector: Mutex::new(InspectorState::default()),
            }),
            worker: None,
            stop: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Start the worker thread. Idempotent: a second call is a no-op
    /// while a worker is already running.
    pub fn start(&mut self, kind: SimKind) {
        if self.worker.is_some() {
            return;
        }
        if !kind.is_msfs() {
            *self.shared.state.lock().unwrap() = ConnectionState::Disconnected;
            return;
        }
        self.stop = Arc::new(AtomicBool::new(false));
        let shared = Arc::clone(&self.shared);
        let stop = Arc::clone(&self.stop);
        *shared.state.lock().unwrap() = ConnectionState::Connecting;
        *shared.last_error.lock().unwrap() = None;
        tracing::info!(?kind, "MSFS raw adapter started");
        let handle = thread::Builder::new()
            .name("sim-msfs-worker".into())
            .spawn(move || worker_loop(shared, stop, kind))
            .expect("could not spawn sim-msfs worker thread");
        self.worker = Some(handle);
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.worker.take() {
            // Give the worker a moment to wind down cleanly. We don't
            // join indefinitely — SimConnect_Close inside the worker
            // can hang if MSFS itself is gone.
            let _ = h.join();
        }
        *self.shared.state.lock().unwrap() = ConnectionState::Disconnected;
        tracing::info!("MSFS raw adapter stopped");
    }

    pub fn state(&self) -> ConnectionState {
        *self.shared.state.lock().unwrap()
    }

    pub fn snapshot(&self) -> Option<SimSnapshot> {
        self.shared.snapshot.lock().unwrap().clone()
    }

    pub fn last_error(&self) -> Option<String> {
        self.shared.last_error.lock().unwrap().clone()
    }

    // ---- Inspector (Phase B) ----

    /// Add a SimVar/LVar to the live inspector watchlist. Returns the
    /// stable id assigned to this entry — pass it to `remove_watch`.
    /// Re-registration of SimConnect data definition #3 happens on the
    /// next worker tick (asynchronous, sub-second).
    pub fn add_watch(&self, name: String, unit: String, kind: WatchKind) -> u32 {
        let mut g = self.shared.inspector.lock().unwrap();
        g.add(name, unit, kind)
    }

    pub fn remove_watch(&self, id: u32) {
        let mut g = self.shared.inspector.lock().unwrap();
        g.remove(id);
    }

    /// Snapshot of the current watchlist (cloning so the caller doesn't
    /// hold the inspector mutex). Each entry carries its latest value
    /// — `value: None` means we haven't received a tick yet.
    pub fn watches(&self) -> Vec<InspectorWatch> {
        self.shared.inspector.lock().unwrap().watches.clone()
    }
}

impl Drop for MsfsAdapter {
    fn drop(&mut self) {
        self.stop();
    }
}

// ---- Worker loop ----

fn worker_loop(shared: Arc<Shared>, stop: Arc<AtomicBool>, kind: SimKind) {
    // Outer reconnect loop. SimConnect_Open returns E_FAIL while MSFS
    // isn't running; we simply retry every 2s until it's up.
    while !stop.load(Ordering::Relaxed) {
        match Connection::open("CloudeAcars") {
            Ok(mut conn) => {
                tracing::info!("SimConnect_Open succeeded — registering data definition");
                if let Err(e) = conn.register_telemetry() {
                    set_error(&shared, format!("RegisterDataDefinition failed: {e}"));
                    tracing::error!(error = %e, "register_telemetry failed");
                    drop(conn);
                    sleep_or_stop(&stop, Duration::from_secs(2));
                    continue;
                }
                // Touchdown registration is best-effort: a failure
                // there should NOT take down live telemetry. Log and
                // proceed.
                if let Err(e) = conn.register_touchdown() {
                    tracing::warn!(error = %e, "register_touchdown failed — touchdown values will stay None");
                }
                if let Err(e) = conn.request_data_per_second() {
                    set_error(&shared, format!("RequestDataOnSimObject failed: {e}"));
                    tracing::error!(error = %e, "request_data_per_second failed");
                    drop(conn);
                    sleep_or_stop(&stop, Duration::from_secs(2));
                    continue;
                }
                if let Err(e) = conn.request_touchdown_per_second() {
                    tracing::warn!(error = %e, "request_touchdown_per_second failed — touchdown values will stay None");
                }
                run_dispatch(&shared, &stop, &mut conn, kind);
                // run_dispatch only returns when stop is signalled or
                // the connection has gone stale. Either way, drop and
                // try again at the top of the loop.
            }
            Err(e) => {
                let msg = format!("SimConnect_Open failed: {e}");
                set_error(&shared, msg);
                *shared.state.lock().unwrap() = ConnectionState::Connecting;
            }
        }
        sleep_or_stop(&stop, Duration::from_secs(2));
    }
    *shared.state.lock().unwrap() = ConnectionState::Disconnected;
}

fn run_dispatch(
    shared: &Arc<Shared>,
    stop: &Arc<AtomicBool>,
    conn: &mut Connection,
    kind: SimKind,
) {
    let mut last_data = Instant::now();
    let mut got_first = false;
    let simulator = kind.as_simulator();
    // Force inspector re-registration after a reconnect — the new
    // SimConnect handle starts with an empty definition table even
    // if the user already populated the watchlist before the drop.
    if !shared.inspector.lock().unwrap().watches.is_empty() {
        shared.inspector.lock().unwrap().dirty = true;
    }

    while !stop.load(Ordering::Relaxed) {
        // Re-register the inspector watchlist whenever the UI has
        // mutated it. The dirty flag avoids hot-looping the
        // SimConnect call in the steady state.
        let needs_inspector_register = {
            let g = shared.inspector.lock().unwrap();
            g.dirty
        };
        if needs_inspector_register {
            let watches = shared.inspector.lock().unwrap().watches.clone();
            match conn.register_inspector(&watches) {
                Ok(()) => {
                    if !watches.is_empty() {
                        if let Err(e) = conn.request_inspector_per_second() {
                            tracing::warn!(error = %e, "request_inspector failed");
                        }
                    }
                    shared.inspector.lock().unwrap().dirty = false;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "register_inspector failed; will retry");
                }
            }
        }

        // Drain whatever messages SimConnect has queued for us.
        loop {
            match conn.get_next_dispatch() {
                Ok(None) => break, // queue empty
                Ok(Some(DispatchMsg::Open)) => {
                    tracing::info!("SimConnect_RECV_OPEN — handshake done");
                }
                Ok(Some(DispatchMsg::Quit)) => {
                    tracing::warn!("SimConnect sent QUIT — dropping connection");
                    return;
                }
                Ok(Some(DispatchMsg::Exception {
                    exception,
                    send_id,
                    index,
                })) => {
                    // This is the diagnostic the legacy crate didn't
                    // give us — log the exact SimVar that failed.
                    let field = TELEMETRY_FIELDS.get(index as usize).map(|f| f.name);
                    tracing::warn!(
                        exception,
                        send_id,
                        index,
                        ?field,
                        "SIMCONNECT_RECV_EXCEPTION — SimVar request was rejected"
                    );
                }
                Ok(Some(DispatchMsg::SimObjectData { request_id, bytes })) => {
                    last_data = Instant::now();
                    match request_id {
                        REQUEST_ID => {
                            let mut snap = telemetry::parse(&bytes, simulator);
                            // Merge in the most recent touchdown sample
                            // so consumers see a unified snapshot.
                            if let Some(td) = *shared.touchdown.lock().unwrap() {
                                if !td.is_uninitialised() {
                                    snap.touchdown_vs_fpm =
                                        Some((td.vs_fps * 60.0) as f32);
                                    snap.touchdown_pitch_deg = Some(td.pitch_deg as f32);
                                    snap.touchdown_bank_deg = Some(td.bank_deg as f32);
                                    snap.touchdown_heading_mag_deg =
                                        Some(td.heading_mag_deg as f32);
                                    snap.touchdown_lat = Some(td.lat_rad.to_degrees());
                                    snap.touchdown_lon = Some(td.lon_rad.to_degrees());
                                }
                            }
                            if !got_first {
                                got_first = true;
                                *shared.state.lock().unwrap() = ConnectionState::Connected;
                                tracing::info!(
                                    aircraft = ?snap.aircraft_title,
                                    profile = ?snap.aircraft_profile,
                                    "MSFS first snapshot received"
                                );
                                log_first_snapshot_diagnostics(&snap);
                            }
                            *shared.snapshot.lock().unwrap() = Some(snap);
                        }
                        TOUCHDOWN_REQUEST_ID => {
                            let td = Touchdown::from_block(&bytes);
                            *shared.touchdown.lock().unwrap() = Some(td);
                        }
                        INSPECTOR_REQUEST_ID => {
                            shared.inspector.lock().unwrap().ingest(&bytes);
                        }
                        other => {
                            tracing::trace!(request_id = other, "unknown SimObjectData request_id");
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "SimConnect dispatch error");
                    return;
                }
            }
        }

        // Stale watchdog: if no data has arrived for a while assume
        // MSFS crashed or the pipe died, and let the outer loop
        // re-open the connection.
        if got_first && last_data.elapsed() > STALE_TIMEOUT {
            tracing::warn!("no SimConnect data for {:?} — reconnecting", STALE_TIMEOUT);
            return;
        }

        thread::sleep(Duration::from_millis(50));
    }
}

fn log_first_snapshot_diagnostics(snap: &SimSnapshot) {
    tracing::info!(
        fuel_total_kg = snap.fuel_total_kg,
        total_weight_kg = ?snap.total_weight_kg,
        aircraft_title = ?snap.aircraft_title,
        aircraft_profile = ?snap.aircraft_profile,
        "raw SimConnect first-snapshot fuel/weight diagnostic"
    );
}

fn set_error(shared: &Arc<Shared>, msg: String) {
    *shared.last_error.lock().unwrap() = Some(msg);
}

fn sleep_or_stop(stop: &Arc<AtomicBool>, dur: Duration) {
    let step = Duration::from_millis(100);
    let mut left = dur;
    while !left.is_zero() {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        let s = std::cmp::min(step, left);
        thread::sleep(s);
        left = left.saturating_sub(s);
    }
}

// ---- Connection wrapper ----

/// Owns the SimConnect handle and provides the higher-level operations
/// the worker loop drives. `Drop` calls `SimConnect_Close`.
struct Connection {
    handle: sys::HANDLE,
}

impl Connection {
    fn open(name: &str) -> Result<Self, String> {
        let cname = std::ffi::CString::new(name).expect("connection name must be plain ASCII");
        let mut handle: sys::HANDLE = std::ptr::null_mut();
        let hr = unsafe {
            sys::SimConnect_Open(
                &mut handle,
                cname.as_ptr(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                0,
            )
        };
        if hr != 0 {
            return Err(format!("HRESULT 0x{hr:08X}"));
        }
        Ok(Self { handle })
    }

    /// Register every entry in `TELEMETRY_FIELDS` in order.
    fn register_telemetry(&mut self) -> Result<(), String> {
        for (idx, field) in TELEMETRY_FIELDS.iter().enumerate() {
            let cname = std::ffi::CString::new(field.name)
                .map_err(|_| "SimVar name contained NUL".to_string())?;
            let cunit = std::ffi::CString::new(field.unit)
                .map_err(|_| "Unit string contained NUL".to_string())?;
            let datatype = match field.kind {
                telemetry::FieldKind::Float64 => sys::SIMCONNECT_DATATYPE_FLOAT64,
                telemetry::FieldKind::Int32 => sys::SIMCONNECT_DATATYPE_INT32,
                telemetry::FieldKind::String256 => sys::SIMCONNECT_DATATYPE_STRING256,
            };
            let hr = unsafe {
                sys::SimConnect_AddToDataDefinition(
                    self.handle,
                    DEFINITION_ID,
                    cname.as_ptr(),
                    cunit.as_ptr(),
                    datatype,
                    0.0,
                    u32::MAX,
                )
            };
            if hr != 0 {
                return Err(format!(
                    "AddToDataDefinition for SimVar #{idx} \"{}\" returned 0x{hr:08X}",
                    field.name
                ));
            }
        }
        Ok(())
    }

    /// Register the touchdown sample fields under definition #2.
    /// Best-effort: we already log per-field exceptions in the
    /// dispatch loop, so a partial registration here is recoverable.
    fn register_touchdown(&mut self) -> Result<(), String> {
        for (idx, field) in TOUCHDOWN_FIELDS.iter().enumerate() {
            let cname = std::ffi::CString::new(field.name)
                .map_err(|_| "SimVar name contained NUL".to_string())?;
            let cunit = std::ffi::CString::new(field.unit)
                .map_err(|_| "Unit string contained NUL".to_string())?;
            let datatype = match field.kind {
                telemetry::FieldKind::Float64 => sys::SIMCONNECT_DATATYPE_FLOAT64,
                telemetry::FieldKind::Int32 => sys::SIMCONNECT_DATATYPE_INT32,
                telemetry::FieldKind::String256 => sys::SIMCONNECT_DATATYPE_STRING256,
            };
            let hr = unsafe {
                sys::SimConnect_AddToDataDefinition(
                    self.handle,
                    TOUCHDOWN_DEFINITION_ID,
                    cname.as_ptr(),
                    cunit.as_ptr(),
                    datatype,
                    0.0,
                    u32::MAX,
                )
            };
            if hr != 0 {
                return Err(format!(
                    "AddToDataDefinition for touchdown SimVar #{idx} \"{}\" returned 0x{hr:08X}",
                    field.name
                ));
            }
        }
        Ok(())
    }

    /// Re-register the inspector data definition from scratch using
    /// the supplied watchlist. Always clears the existing definition
    /// first so a removed entry actually goes away — SimConnect has
    /// no per-field "remove" call. An empty watchlist is valid (just
    /// clears the definition and skips the request).
    fn register_inspector(&mut self, watches: &[InspectorWatch]) -> Result<(), String> {
        let hr = unsafe {
            sys::SimConnect_ClearDataDefinition(self.handle, INSPECTOR_DEFINITION_ID)
        };
        // ClearDataDefinition returns S_OK even when the definition
        // didn't exist yet — non-zero is a real error.
        if hr != 0 {
            return Err(format!("ClearDataDefinition returned 0x{hr:08X}"));
        }
        for (idx, w) in watches.iter().enumerate() {
            let cname = std::ffi::CString::new(w.name.as_str())
                .map_err(|_| format!("watch #{idx} name contained NUL"))?;
            let cunit = std::ffi::CString::new(w.unit.as_str())
                .map_err(|_| format!("watch #{idx} unit contained NUL"))?;
            let datatype = match w.kind {
                WatchKind::Number => sys::SIMCONNECT_DATATYPE_FLOAT64,
                WatchKind::Bool => sys::SIMCONNECT_DATATYPE_INT32,
                WatchKind::String => sys::SIMCONNECT_DATATYPE_STRING256,
            };
            let hr = unsafe {
                sys::SimConnect_AddToDataDefinition(
                    self.handle,
                    INSPECTOR_DEFINITION_ID,
                    cname.as_ptr(),
                    cunit.as_ptr(),
                    datatype,
                    0.0,
                    u32::MAX,
                )
            };
            if hr != 0 {
                return Err(format!(
                    "AddToDataDefinition for inspector watch \"{}\" returned 0x{hr:08X}",
                    w.name
                ));
            }
        }
        Ok(())
    }

    fn request_inspector_per_second(&mut self) -> Result<(), String> {
        let hr = unsafe {
            sys::SimConnect_RequestDataOnSimObject(
                self.handle,
                INSPECTOR_REQUEST_ID,
                INSPECTOR_DEFINITION_ID,
                sys::SIMCONNECT_OBJECT_ID_USER,
                sys::SIMCONNECT_PERIOD_SECOND,
                0,
                0,
                0,
                0,
            )
        };
        if hr != 0 {
            return Err(format!("HRESULT 0x{hr:08X}"));
        }
        Ok(())
    }

    fn request_touchdown_per_second(&mut self) -> Result<(), String> {
        // Bumped from SECOND to VISUAL_FRAME (~30 Hz) for the same
        // reason the main telemetry runs that fast: the FSM's
        // Final → Landing tick can fire just before the next
        // SECOND-period touchdown update has propagated the freshly
        // latched values into shared.touchdown, leaving the V/S
        // capture stale by up to 1 second. At ~30 Hz the latch is
        // visible to the next snapshot within ~33 ms.
        let hr = unsafe {
            sys::SimConnect_RequestDataOnSimObject(
                self.handle,
                TOUCHDOWN_REQUEST_ID,
                TOUCHDOWN_DEFINITION_ID,
                sys::SIMCONNECT_OBJECT_ID_USER,
                sys::SIMCONNECT_PERIOD_VISUAL_FRAME,
                0,
                0,
                0,
                0,
            )
        };
        if hr != 0 {
            return Err(format!("HRESULT 0x{hr:08X}"));
        }
        Ok(())
    }

    /// Subscribe to live telemetry at VISUAL_FRAME cadence (~30 Hz).
    /// The 1 Hz SECOND rate we ran on previously was too sparse for
    /// touchdown capture: the actual ground-contact subframe dropped
    /// between two snapshots, the ring buffer only had 5 entries in
    /// the 5-second look-back window, and the recorded V/S routinely
    /// caught the bounce-rebound rather than the impact (logged
    /// "V/S -4 fpm" while MSFS reported -114 fpm). At 30 Hz the
    /// buffer holds 150 entries → impossible to miss the actual
    /// touchdown frame.
    ///
    /// CPU cost is negligible: the dispatch loop already drains all
    /// queued messages each tick via `get_next_dispatch`, so the
    /// only difference is more byte-level parsing per second
    /// (~30 KB/s of data).
    fn request_data_per_second(&mut self) -> Result<(), String> {
        let hr = unsafe {
            sys::SimConnect_RequestDataOnSimObject(
                self.handle,
                REQUEST_ID,
                DEFINITION_ID,
                sys::SIMCONNECT_OBJECT_ID_USER,
                sys::SIMCONNECT_PERIOD_VISUAL_FRAME,
                0,
                0,
                0,
                0,
            )
        };
        if hr != 0 {
            return Err(format!("HRESULT 0x{hr:08X}"));
        }
        Ok(())
    }

    /// Pull one message off the SimConnect queue, returning None when
    /// the queue is empty. Distinguishes the receiver IDs we actually
    /// care about; the rest are logged at trace level and dropped.
    fn get_next_dispatch(&mut self) -> Result<Option<DispatchMsg>, String> {
        let mut p_data: *mut sys::SIMCONNECT_RECV = std::ptr::null_mut();
        let mut cb_data: sys::DWORD = 0;
        let hr = unsafe { sys::SimConnect_GetNextDispatch(self.handle, &mut p_data, &mut cb_data) };
        if hr == sys::E_FAIL {
            // Empty queue — not an error in SimConnect-land.
            return Ok(None);
        }
        if hr != 0 {
            return Err(format!("GetNextDispatch returned 0x{hr:08X}"));
        }
        if p_data.is_null() || cb_data == 0 {
            return Ok(None);
        }
        let recv = unsafe { &*p_data };
        let id = recv.dwID;
        let msg = match id {
            sys::SIMCONNECT_RECV_ID_OPEN => Some(DispatchMsg::Open),
            sys::SIMCONNECT_RECV_ID_QUIT => Some(DispatchMsg::Quit),
            sys::SIMCONNECT_RECV_ID_EXCEPTION => {
                let exc = unsafe { &*(p_data as *const sys::SIMCONNECT_RECV_EXCEPTION) };
                Some(DispatchMsg::Exception {
                    exception: exc.dwException,
                    send_id: exc.dwSendID,
                    index: exc.dwIndex,
                })
            }
            sys::SIMCONNECT_RECV_ID_SIMOBJECT_DATA => {
                // dwData[1] in the SDK header — first byte of the
                // payload — is at the same offset as
                // `SIMCONNECT_RECV_SIMOBJECT_DATA::dwData`. We copy
                // the bytes out so the dispatch ptr can be reused.
                let recv_data = unsafe { &*(p_data as *const sys::SIMCONNECT_RECV_SIMOBJECT_DATA) };
                let request_id = recv_data.dwRequestID;
                let header_size = std::mem::size_of::<sys::SIMCONNECT_RECV_SIMOBJECT_DATA>();
                let total = cb_data as usize;
                if total < header_size {
                    return Ok(None);
                }
                let payload_start = header_size - std::mem::size_of::<sys::DWORD>();
                let payload_len = total - payload_start;
                let bytes = unsafe {
                    let base = p_data as *const u8;
                    std::slice::from_raw_parts(base.add(payload_start), payload_len)
                };
                Some(DispatchMsg::SimObjectData {
                    request_id,
                    bytes: bytes.to_vec(),
                })
            }
            _ => None,
        };
        Ok(msg)
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe { sys::SimConnect_Close(self.handle) };
            self.handle = std::ptr::null_mut();
        }
    }
}

unsafe impl Send for Connection {}
unsafe impl Sync for Connection {}

#[derive(Debug)]
enum DispatchMsg {
    Open,
    Quit,
    Exception {
        exception: u32,
        send_id: u32,
        index: u32,
    },
    SimObjectData {
        request_id: u32,
        bytes: Vec<u8>,
    },
}

// Marker so the file always references kind/Utc when stub'd out.
#[allow(dead_code)]
fn _link_assertions() {
    let _ = Utc::now();
    let _ = Simulator::Msfs2024;
    let _ = AircraftProfile::Default;
}
