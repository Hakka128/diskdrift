//! Inspect service: drill down one level inside a directory.
//!
//! Answers "this block grew — which of its children grew?" It lists the
//! **direct children** of the target directory, plus a residual `other` row
//! that accounts for files directly in the directory (which are not persisted
//! as separate rows) and any accounting gap:
//!
//! `other = target_delta - Σ(children_delta)`

use std::path::{Path, PathBuf};

use crate::diff::{merge_children_deltas, signed_delta, DiffEntry};
use crate::error::{Result, WhyBigError};
use crate::pathutil::{is_under, normalize_lexical};
use crate::storage::models::SnapshotRecord;
use crate::storage::Storage;

/// Everything the CLI needs to render an inspect report.
#[derive(Debug, Clone)]
pub struct InspectReport {
    pub target: String,
    /// The tracked root the pair of snapshots belongs to.
    pub root: String,
    pub before: SnapshotRecord,
    pub after: SnapshotRecord,
    /// Aggregate size of the target directory (0 when absent in that snapshot).
    pub target_before: u64,
    pub target_after: u64,
    pub target_delta: i128,
    /// Direct-child deltas (|delta| descending), zero deltas excluded.
    pub contributors: Vec<DiffEntry>,
    /// `target_delta - Σ(contributors)`. Can be positive or negative.
    pub other: i128,
}

/// Normalize a user-supplied inspect target: relative paths join the cwd and
/// the Windows verbatim prefix is stripped — same rules as the scanner. No
/// filesystem access: the directory may no longer exist.
/// Normalize a user-supplied inspect target with the shared path rules.
pub fn normalize_target(raw: &Path) -> PathBuf {
    crate::pathutil::normalize_target(raw)
}

/// Compute an inspect report for `target` within `root`.
///
/// `target` must be inside (or equal to) `root`. When the directory exists in
/// neither snapshot → `PathNotInSnapshots`. New/deleted directories handle the
/// missing side as 0.
pub fn inspect(
    storage: &Storage,
    before: &SnapshotRecord,
    after: &SnapshotRecord,
    root: &str,
    target: &Path,
) -> Result<InspectReport> {
    let target = normalize_lexical(target);
    let target_str = target.to_string_lossy().into_owned();
    if !is_under(&target, Path::new(root)) {
        return Err(WhyBigError::PathOutsideRoot {
            path: target_str,
            root: root.to_string(),
        });
    }

    let after_entry = lookup(storage, after, &target_str)?;
    let before_entry = lookup(storage, before, &target_str)?;
    if before_entry.is_none() && after_entry.is_none() {
        return Err(WhyBigError::PathNotInSnapshots(target_str));
    }

    // Use the *stored* spelling as the scope for child queries so they match
    // the stored keys exactly (also handles ASCII case on Windows).
    let scope = after_entry
        .as_ref()
        .map(|e| e.path.clone())
        .or_else(|| before_entry.as_ref().map(|e| e.path.clone()))
        .unwrap_or_else(|| target_str.clone());

    let target_before = before_entry.as_ref().map(|e| e.size).unwrap_or(0);
    let target_after = after_entry.as_ref().map(|e| e.size).unwrap_or(0);
    let target_delta = signed_delta(target_before, target_after);

    let contributors = merge_children_deltas(storage, before.id, after.id, &scope)?;
    let children_sum: i128 = contributors.iter().map(|e| e.delta).sum();
    let other = target_delta - children_sum;

    Ok(InspectReport {
        target: target_str,
        root: root.to_string(),
        before: before.clone(),
        after: after.clone(),
        target_before,
        target_after,
        target_delta,
        contributors,
        other,
    })
}

/// Exact-then-case-insensitive (Windows) entry lookup.
fn lookup(
    storage: &Storage,
    snapshot: &SnapshotRecord,
    target: &str,
) -> Result<Option<crate::storage::models::EntryRecord>> {
    match storage.get_entry(snapshot.id, target)? {
        Some(e) => Ok(Some(e)),
        None => storage.get_entry_case_insensitive(snapshot.id, target),
    }
}
