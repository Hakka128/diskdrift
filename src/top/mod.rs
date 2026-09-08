//! `diskdrift top` — quick ranking of the biggest growers/shrinkers.
//!
//! This intentionally contains **no** own SQL or delta logic: it is a thin
//! view over the Milestone 2 diff engine (see DESIGN-M3.md §2). `top` without
//! a path ranks the tracked root's direct children; `top <path>` ranks the
//! target's direct children — exactly the same attribution `inspect` shows,
//! just without the `other`/diagnostics framing.

use std::path::Path;

use crate::diff::{self, DiffEntry, DiffSelection, SinceInfo};
use crate::error::{DiskDriftError, Result};
use crate::pathutil::{is_under, normalize_target};
use crate::storage::models::SnapshotRecord;
use crate::storage::Storage;

/// Which side of the change to rank.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopMode {
    /// Everything that grew (largest delta first).
    Growth,
    /// Everything that shrank (most space freed first, by |delta|).
    Shrink,
}

/// Result of a `top` query.
#[derive(Debug, Clone)]
pub struct TopReport {
    pub root: String,
    /// The scope directory (tracked root, or the `<path>` target).
    pub scope: String,
    pub before: SnapshotRecord,
    pub after: SnapshotRecord,
    pub mode: TopMode,
    /// Already sorted by the diff engine (growth desc / shrink |delta| desc).
    pub entries: Vec<DiffEntry>,
    /// Present only when `--since` selected the pair.
    pub since: Option<SinceInfo>,
}

/// Rank the direct-children changes between two snapshots (default selection)
/// at `raw_target` (or the root when `None`).
pub fn top(
    storage: &Storage,
    selection: DiffSelection,
    raw_target: Option<&Path>,
    mode: TopMode,
) -> Result<TopReport> {
    let pair = diff::select_pair(storage, selection)?;
    let root = pair.after.root_path.clone();

    let scope = match raw_target {
        Some(t) => {
            let target = normalize_target(t);
            if !is_under(&target, Path::new(&root)) {
                return Err(DiskDriftError::PathOutsideRoot {
                    path: target.to_string_lossy().into_owned(),
                    root: root.clone(),
                });
            }
            target.to_string_lossy().into_owned()
        }
        None => root.clone(),
    };

    let d = diff::compute_pair(storage, &pair, &scope)?;
    let entries = match mode {
        TopMode::Growth => d.grew,
        TopMode::Shrink => d.shrank,
    };

    Ok(TopReport {
        root,
        scope,
        before: pair.before,
        after: pair.after,
        mode,
        entries,
        since: pair.since,
    })
}
