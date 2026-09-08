//! History service: how a directory (or the tracked root itself) has changed
//! across snapshots.
//!
//! One LEFT-JOIN query builds the whole series (no N+1), missing directories
//! read as 0, and history is ascending by snapshot id so trends read naturally.

use std::path::{Path, PathBuf};

use crate::diff::signed_delta;
use crate::error::{Result, WhyBigError};
use crate::pathutil::{is_under, normalize_target};
use crate::storage::Storage;

/// One point on the history timeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryPoint {
    pub snapshot_id: i64,
    /// Unix epoch milliseconds (UTC).
    pub created_at_ms: i64,
    /// Directory size in that snapshot (0 when the directory did not exist).
    pub size: u64,
    /// `size - previous.size`; `None` for the first point.
    pub delta_from_previous: Option<i128>,
}

/// The full history timeline for one directory.
#[derive(Debug, Clone)]
pub struct HistoryReport {
    pub root: PathBuf,
    pub target: PathBuf,
    pub points: Vec<HistoryPoint>,
    /// `last.size - first.size`.
    pub total_delta: i128,
}

/// The root of the most recent snapshot — the default selection for
/// history/ diff / inspect / top alike.
pub fn latest_root(storage: &Storage) -> Result<String> {
    let snap = storage.latest_snapshot()?.ok_or(WhyBigError::NoSnapshots)?;
    Ok(snap.root_path)
}

/// Compute a history for `raw_target` (or the root itself when omitted).
pub fn history(
    storage: &Storage,
    raw_target: Option<&Path>,
    limit: usize,
) -> Result<HistoryReport> {
    let root_str = latest_root(storage)?;
    let root = PathBuf::from(&root_str);

    let target = match raw_target {
        Some(t) => normalize_target(t),
        None => root.clone(),
    };
    if !is_under(&target, &root) {
        return Err(WhyBigError::PathOutsideRoot {
            path: target.to_string_lossy().into_owned(),
            root: root_str.clone(),
        });
    }

    let target_str = target.to_string_lossy().into_owned();
    // Use the stored spelling when the user's casing differs (Windows NTFS is
    // case-insensitive); fall back to the user's spelling (sizes read 0).
    let stored = resolve_stored_spelling(storage, &root_str, &target_str);
    let join_path = stored.as_deref().unwrap_or(&target_str);

    let series = storage.get_history_series(&root_str, join_path, limit as i64)?;

    let mut points: Vec<HistoryPoint> = Vec::with_capacity(series.len());
    for (idx, (snapshot_id, created_at_ms, size)) in series.iter().enumerate() {
        let delta_from_previous = if idx == 0 {
            None
        } else {
            Some(signed_delta(points[idx - 1].size, *size))
        };
        points.push(HistoryPoint {
            snapshot_id: *snapshot_id,
            created_at_ms: *created_at_ms,
            size: *size,
            delta_from_previous,
        });
    }

    let total_delta = match (points.first(), points.last()) {
        (Some(first), Some(last)) => signed_delta(first.size, last.size),
        _ => 0,
    };

    Ok(HistoryReport {
        root,
        target,
        points,
        total_delta,
    })
}

/// Find the stored key spelling for `target` in the latest snapshot (exact
/// then case-insensitive). Returns `None` when the directory never existed in
/// that snapshot.
fn resolve_stored_spelling(storage: &Storage, root: &str, target: &str) -> Option<String> {
    let snapshots = storage.get_latest_snapshots_for_root(root, 1).ok()?;
    let id = snapshots.into_iter().next()?.id;
    if let Some(e) = storage.get_entry(id, target).ok()? {
        return Some(e.path);
    }
    storage
        .get_entry_case_insensitive(id, target)
        .ok()?
        .map(|e| e.path)
}
