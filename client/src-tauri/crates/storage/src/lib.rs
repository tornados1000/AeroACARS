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
