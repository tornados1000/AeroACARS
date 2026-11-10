//! Lightweight "is X-Plane running?" probe.
//!
//! Sends one RREF subscription for a cheap DataRef to localhost:49000
//! and waits up to a short timeout for any response packet. If
//! anything arrives back, X-Plane is running on this host. We
//! immediately unsubscribe (freq=0) so the probe doesn't leave a
//! ghost stream behind.
//!
//! Used by the auto-detection flow in `src/lib.rs` to suggest a
//! sim choice on first launch when the pilot hasn't picked one yet.

use std::net::UdpSocket;
use std::time::Duration;

use crate::rref::{decode_response, encode_request};
use crate::XPLANE_LISTEN_PORT;

/// Probe DataRef name. We use VERTICAL SPEED because every X-Plane
/// build (10/11/12) wires it; even an aircraft sitting on a runway
/// will report 0 fpm, so we get a packet back.
const PROBE_DATAREF: &str = "sim/flightmodel/position/vh_ind_fpm";

/// Probe timeout. 500 ms is generous: X-Plane responds to RREFs
/// within one frame (~16 ms at 60 fps), so anything past 100 ms is
/// "X-Plane is not listening on this port".
const PROBE_TIMEOUT_MS: u64 = 500;

/// Returns `true` if X-Plane is reachable on `127.0.0.1:49000`.
///
/// This call costs one UDP roundtrip; safe to invoke on app boot
/// or from a "Detect Sim" button. Synchronous — runs on whatever
/// thread the caller is in. The caller can wrap in
/// `tauri::async_runtime::spawn_blocking` if it doesn't want the
/// 500 ms latency on the event loop.
pub fn is_xplane_running() -> bool {
    let socket = match UdpSocket::bind("127.0.0.1:0") {
        Ok(s) => s,
        Err(_) => return false,
    };
    if socket
        .set_read_timeout(Some(Duration::from_millis(PROBE_TIMEOUT_MS)))
        .is_err()
    {
        return false;
    }
    let xplane_addr = format!("127.0.0.1:{XPLANE_LISTEN_PORT}");

    // Subscribe at 1 Hz on index 9999 (an arbitrary throwaway).
    let req = encode_request(1, 9999, PROBE_DATAREF);
    if socket.send_to(&req, &xplane_addr).is_err() {
        return false;
    }

    let mut buf = [0u8; 1500];
    let detected = match socket.recv_from(&mut buf) {
        Ok((n, _)) => !decode_response(&buf[..n]).is_empty(),
        Err(_) => false,
    };

    // Best-effort unsubscribe.
    let stop = encode_request(0, 9999, PROBE_DATAREF);
    let _ = socket.send_to(&stop, &xplane_addr);

    detected
}
