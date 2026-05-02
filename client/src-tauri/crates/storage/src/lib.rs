//! Local persistence layer.
//!
//! Phase K scope — offline queue for ACARS position posts. When the
//! phpVMS network call fails (no internet, server hiccup, rate-limit),
//! the streamer enqueues the position into a file-based queue. The next
//! successful tick drains it before the new post, so phpVMS sees the
//! correct chronological order even after a network gap.
//!
//! File-based instead of SQLite for now: a flight is unlikely to queue
//! more than a few hundred rows in a real-world outage, and a JSON file
//! keeps the dependency surface minimal. SQLite remains an option in
//! the workspace if we ever need indexed queries (flight log, settings
//! cache, analytics) — see requirements spec §26.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

const QUEUE_FILE: &str = "position_queue.json";
/// Cap on retained queued positions. Past this point the oldest are
/// dropped — a multi-hour outage isn't worth blocking forever or
/// blowing up the file. ~1000 rows ≈ 3 h of cruise-cadence (30 s) or
/// ~1.5 h of ground/approach (10 s).
const QUEUE_MAX_ROWS: usize = 1000;

/// File name for the landing-history store inside the app data dir.
const LANDINGS_FILE: &str = "landings.json";
/// Cap on retained landing records. ~500 landings ≈ several years of
/// active virtual-airline flying for one pilot. Past this we drop the
/// oldest so the file can't grow unbounded.
const LANDINGS_MAX_ROWS: usize = 500;

/// One pending position post, ready to be replayed once connectivity
/// returns. The serialised `position` is opaque from the storage crate's
/// point of view — it's just whatever JSON the API client wants to
/// `POST /api/pireps/{pirep_id}/acars/position` with.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedPosition {
    /// PIREP the position belongs to. The streamer uses this to route
    /// drained rows to the right phpVMS endpoint, and the queue
    /// implicitly partitions by PIREP so an old flight's leftovers
    /// don't bleed into a new one (they're discarded by `pirep_id`
    /// mismatch on drain).
    pub pirep_id: String,
    /// Serialized `PositionEntry` JSON. We don't import api-client into
    /// the storage crate to avoid a circular dependency.
    pub position: serde_json::Value,
}

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

/// File-backed FIFO queue of pending position posts. Cheap to read /
/// write — entire file is loaded, mutated, and rewritten atomically.
/// At ~1 KB per row × 1000 rows = ~1 MB worst case, that's fine.
pub struct PositionQueue {
    path: PathBuf,
}

impl PositionQueue {
    /// Open (or implicitly create) the queue file in the given app
    /// data directory. The file itself is only written when something
    /// is actually enqueued.
    pub fn open(app_data_dir: impl AsRef<Path>) -> Result<Self, StorageError> {
        let dir = app_data_dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir)?;
        Ok(Self {
            path: dir.join(QUEUE_FILE),
        })
    }

    /// Append a position to the queue. Drops the oldest entry when the
    /// queue is at capacity so a long outage can't unbounded-grow the
    /// file. Returns the new queue length on success.
    pub fn enqueue(&self, item: QueuedPosition) -> Result<usize, StorageError> {
        let mut items = self.read_all()?;
        items.push(item);
        if items.len() > QUEUE_MAX_ROWS {
            // Drop oldest until we're back at the cap.
            let drop_count = items.len() - QUEUE_MAX_ROWS;
            items.drain(0..drop_count);
            tracing::warn!(
                drop_count,
                kept = items.len(),
                "position queue at capacity — dropped oldest rows"
            );
        }
        self.write_all(&items)?;
        Ok(items.len())
    }

    /// Read every queued row without modifying the file. Used to
    /// inspect (e.g. show the user a count in the UI) and to drain.
    pub fn read_all(&self) -> Result<Vec<QueuedPosition>, StorageError> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let bytes = std::fs::read(&self.path)?;
        if bytes.is_empty() {
            return Ok(Vec::new());
        }
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// Write the given list as the new full queue contents. Atomic via
    /// write-then-rename so a crash in the middle leaves the previous
    /// file intact.
    fn write_all(&self, items: &[QueuedPosition]) -> Result<(), StorageError> {
        let tmp = self.path.with_extension("json.tmp");
        let bytes = serde_json::to_vec(items)?;
        std::fs::write(&tmp, &bytes)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    /// Replace the on-disk queue with the supplied list. Used by the
    /// streamer's drain logic: read all → try posting each → write
    /// back the rows that still failed.
    pub fn replace(&self, items: &[QueuedPosition]) -> Result<(), StorageError> {
        if items.is_empty() {
            // No need to keep an empty file around — remove it so the
            // next read returns an empty Vec without I/O.
            if self.path.exists() {
                let _ = std::fs::remove_file(&self.path);
            }
            return Ok(());
        }
        self.write_all(items)
    }

    /// Current queue length (cheap — reads the file once). Useful for
    /// surfacing "X positions queued offline" in the dashboard.
    pub fn len(&self) -> Result<usize, StorageError> {
        Ok(self.read_all()?.len())
    }

    /// Drop every queued row regardless of PIREP. Called on
    /// `flight_cancel` / `flight_forget` to avoid replaying positions
    /// for a flight the user discarded.
    pub fn clear(&self) -> Result<(), StorageError> {
        if self.path.exists() {
            std::fs::remove_file(&self.path)?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Landing history store
// ---------------------------------------------------------------------------
//
// The Landing tab in the desktop UI shows a per-flight breakdown of every
// touchdown: score / letter grade, V/S, peak G, runway match, sideslip,
// approach stability, rollout distance, and the SimBrief plan-vs-actual
// fuel comparison. Persisted as a JSON array on disk so a Tauri restart
// doesn't lose history; same atomic-write pattern as the position queue.
//
// One record per filed PIREP — we don't store in-progress flights here.
// Records are immutable once written.

/// Single subsample of the touchdown profile around the on-ground edge.
/// Mirrors `TouchdownProfilePoint` in the main crate; duplicated here
/// to keep storage independent from sim/api crates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LandingProfilePoint {
    /// Milliseconds relative to the touchdown edge (negative = before).
    pub t_ms: i32,
    pub vs_fpm: f32,
    pub g_force: f32,
    pub agl_ft: f32,
    pub on_ground: bool,
    pub heading_true_deg: f32,
    pub groundspeed_kt: f32,
    pub indicated_airspeed_kt: f32,
    pub pitch_deg: f32,
    pub bank_deg: f32,
}

/// Runway-correlation result (mirrors `runway::RunwayMatch`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LandingRunwayMatch {
    pub airport_ident: String,
    pub runway_ident: String,
    pub surface: String,
    pub length_ft: f64,
    pub centerline_distance_m: f64,
    pub centerline_distance_abs_ft: f64,
    pub side: String,
    pub touchdown_distance_from_threshold_ft: f64,
}

/// One landing record — written once when the PIREP is filed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LandingRecord {
    /// PIREP id (phpVMS UUID). Doubles as the record's primary key.
    pub pirep_id: String,
    /// When the touchdown occurred (UTC).
    pub touchdown_at: chrono::DateTime<chrono::Utc>,
    /// When the record was written (= PIREP file time).
    pub recorded_at: chrono::DateTime<chrono::Utc>,
    /// Callsign / flight number (display only).
    pub flight_number: String,
    pub airline_icao: String,
    pub dpt_airport: String,
    pub arr_airport: String,
    pub aircraft_registration: Option<String>,
    pub aircraft_icao: Option<String>,
    pub aircraft_title: Option<String>,
    /// Sim that produced the flight ("MSFS" | "X-PLANE").
    pub sim_kind: Option<String>,

    // Score
    pub score_numeric: i32,
    pub score_label: String,
    pub grade_letter: String,

    // Touchdown vitals
    pub landing_rate_fpm: f32,
    pub landing_peak_vs_fpm: Option<f32>,
    pub landing_g_force: Option<f32>,
    pub landing_peak_g_force: Option<f32>,
    pub landing_pitch_deg: Option<f32>,
    pub landing_bank_deg: Option<f32>,
    pub landing_speed_kt: Option<f32>,
    pub landing_heading_deg: Option<f32>,
    pub landing_weight_kg: Option<f64>,
    pub touchdown_sideslip_deg: Option<f32>,
    pub bounce_count: u8,

    // Wind at touchdown
    pub headwind_kt: Option<f32>,
    pub crosswind_kt: Option<f32>,

    // Approach stability + rollout (Stage 1)
    pub approach_vs_stddev_fpm: Option<f32>,
    pub approach_bank_stddev_deg: Option<f32>,
    pub rollout_distance_m: Option<f64>,

    // SimBrief plan vs actual (Stage 2)
    pub planned_block_fuel_kg: Option<f32>,
    pub planned_burn_kg: Option<f32>,
    pub planned_tow_kg: Option<f32>,
    pub planned_ldw_kg: Option<f32>,
    pub planned_zfw_kg: Option<f32>,
    pub actual_trip_burn_kg: Option<f32>,
    pub fuel_efficiency_kg_diff: Option<f32>,
    pub fuel_efficiency_pct: Option<f32>,
    pub takeoff_weight_kg: Option<f64>,
    pub takeoff_fuel_kg: Option<f32>,
    pub landing_fuel_kg: Option<f32>,
    pub block_fuel_kg: Option<f32>,

    // Runway
    pub runway_match: Option<LandingRunwayMatch>,

    // Touchdown profile (V/S + G curve, ~150 samples)
    #[serde(default)]
    pub touchdown_profile: Vec<LandingProfilePoint>,
}

/// File-backed JSON store of past landings.
pub struct LandingStore {
    path: PathBuf,
}

impl LandingStore {
    /// Open (or implicitly create) the landings file in the app data dir.
    pub fn open(app_data_dir: impl AsRef<Path>) -> Result<Self, StorageError> {
        let dir = app_data_dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir)?;
        Ok(Self {
            path: dir.join(LANDINGS_FILE),
        })
    }

    /// Append (or replace by `pirep_id`) a record. We dedupe so a
    /// re-filed PIREP doesn't show up twice; the last write wins.
    pub fn upsert(&self, record: LandingRecord) -> Result<(), StorageError> {
        let mut items = self.read_all()?;
        if let Some(pos) = items.iter().position(|r| r.pirep_id == record.pirep_id) {
            items[pos] = record;
        } else {
            items.push(record);
        }
        if items.len() > LANDINGS_MAX_ROWS {
            let drop_count = items.len() - LANDINGS_MAX_ROWS;
            items.drain(0..drop_count);
        }
        self.write_all(&items)
    }

    /// Read all records, newest first.
    pub fn list(&self) -> Result<Vec<LandingRecord>, StorageError> {
        let mut items = self.read_all()?;
        items.sort_by(|a, b| b.touchdown_at.cmp(&a.touchdown_at));
        Ok(items)
    }

    /// Look up one record by PIREP id.
    pub fn get(&self, pirep_id: &str) -> Result<Option<LandingRecord>, StorageError> {
        Ok(self
            .read_all()?
            .into_iter()
            .find(|r| r.pirep_id == pirep_id))
    }

    /// Replace the on-disk list with the given vector. Used by the
    /// Landing tab's delete-record flow.
    pub fn replace_all(&self, items: &[LandingRecord]) -> Result<(), StorageError> {
        if items.is_empty() {
            if self.path.exists() {
                let _ = std::fs::remove_file(&self.path);
            }
            return Ok(());
        }
        self.write_all(items)
    }

    fn read_all(&self) -> Result<Vec<LandingRecord>, StorageError> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let bytes = std::fs::read(&self.path)?;
        if bytes.is_empty() {
            return Ok(Vec::new());
        }
        // Tolerate corruption / older shape: empty list is better than
        // taking down the whole UI.
        match serde_json::from_slice(&bytes) {
            Ok(v) => Ok(v),
            Err(e) => {
                tracing::warn!(error = %e, "landings.json unreadable — starting fresh");
                Ok(Vec::new())
            }
        }
    }

    fn write_all(&self, items: &[LandingRecord]) -> Result<(), StorageError> {
        let tmp = self.path.with_extension("json.tmp");
        let bytes = serde_json::to_vec(items)?;
        std::fs::write(&tmp, &bytes)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}
