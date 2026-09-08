//! Diff engine: what changed between two snapshots at a given scope level.
//!
//! Pure domain logic — it knows the `Storage` (read-only) but nothing about
//! terminals or rendering (see `output::diff` for that).
//!
//! Attribution model (see `DESIGN-M2.md` §4): a diff lists only the **direct
//! children** of the scope directory. Diff (`whybig diff`) uses the tracked
//! root as scope → top-level attribution. Inspect (`whybig inspect <path>`)
//! uses the target as scope → next-level drill-down. This is what keeps the
//! tool a "what got big" debugger instead of a dump of every delta.

use std::collections::BTreeMap;
use std::path::Path;

use crate::error::{Result, WhyBigError};
use crate::pathutil::{is_direct_child, paths_equal};
use crate::storage::models::SnapshotRecord;
use crate::storage::Storage;

/// Whether a directory appeared, disappeared or merely changed size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffState {
    /// Present in after, absent in before (treated as before = 0).
    Added,
    /// Present in before, absent in after (treated as after = 0).
    Removed,
    /// Present in both (may have grown or shrunk).
    Changed,
}

/// One directory's change between two snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffEntry {
    /// Full normalized path (the storage key).
    pub path: String,
    pub before_size: u64,
    pub after_size: u64,
    /// `after - before`, always signed 128-bit → never overflows u64 pairs.
    pub delta: i128,
    pub state: DiffState,
}

/// The result of comparing two snapshots at a scope.
#[derive(Debug, Clone)]
pub struct SnapshotDiff {
    /// The scope directory (tracked root for `whybig diff`).
    pub root: String,
    pub before: SnapshotRecord,
    pub after: SnapshotRecord,
    pub total_before: u64,
    pub total_after: u64,
    pub total_delta: i128,
    /// Entries that grew, sorted by delta descending.
    pub grew: Vec<DiffEntry>,
    /// Entries that shrank, sorted by |delta| descending (delta ascending).
    pub shrank: Vec<DiffEntry>,
}

/// How the two snapshots to compare are chosen.
#[derive(Debug, Clone, Copy)]
pub enum DiffSelection {
    /// Latest two snapshots of the most recently tracked root.
    Default,
    /// Explicit ids (validated to share a root by [`select_pair`]).
    Explicit { from: i64, to: i64 },
}

/// Signed delta on `u64` sizes — never overflows (i128 arithmetic).
pub fn signed_delta(before: u64, after: u64) -> i128 {
    after as i128 - before as i128
}

/// Compose two `FileSystem`-level snapshots into a diff.
pub fn compute(
    storage: &Storage,
    before: &SnapshotRecord,
    after: &SnapshotRecord,
    scope: &str,
) -> Result<SnapshotDiff> {
    let merged = merge_children_deltas(storage, before.id, after.id, scope)?;

    let mut grew = Vec::new();
    let mut shrank = Vec::new();
    for e in merged {
        if e.delta > 0 {
            grew.push(e);
        } else if e.delta < 0 {
            shrank.push(e);
        }
        // zero deltas were already dropped by merge_children_deltas.
    }
    grew.sort_by(|x, y| y.delta.cmp(&x.delta).then_with(|| x.path.cmp(&y.path)));
    shrank.sort_by(|x, y| x.delta.cmp(&y.delta).then_with(|| x.path.cmp(&y.path)));

    let total_before = before.total_size_u64();
    let total_after = after.total_size_u64();
    Ok(SnapshotDiff {
        root: scope.to_string(),
        before: before.clone(),
        after: after.clone(),
        total_before,
        total_after,
        total_delta: signed_delta(total_before, total_after),
        grew,
        shrank,
    })
}

/// Merge the **direct children** of `scope` across two snapshots into per-directory
/// deltas (zero deltas dropped), sorted by |delta| descending (ties by path).
/// Shared by the diff engine (which re-groups grew/shrank) and by inspect.
pub fn merge_children_deltas(
    storage: &Storage,
    before_id: i64,
    after_id: i64,
    scope: &str,
) -> Result<Vec<DiffEntry>> {
    let before = storage.get_direct_children(before_id, scope)?;
    let after = storage.get_direct_children(after_id, scope)?;

    // path → (before_size·present, after_size·present).
    let mut sizes: BTreeMap<String, (Option<u64>, Option<u64>)> = BTreeMap::new();
    for e in &before {
        sizes.insert(e.path.clone(), (Some(e.size), None));
    }
    for e in &after {
        match sizes.entry(e.path.clone()) {
            std::collections::btree_map::Entry::Occupied(mut occ) => occ.get_mut().1 = Some(e.size),
            std::collections::btree_map::Entry::Vacant(vac) => {
                vac.insert((None, Some(e.size)));
            }
        }
    }

    let scope_path = Path::new(scope);
    let mut out = Vec::with_capacity(sizes.len());
    for (path, (b, a)) in sizes {
        // The storage query already guarantees direct children; this guard is
        // a second line of defence against boundary regressions.
        if !is_direct_child(Path::new(&path), scope_path) {
            continue;
        }
        let before_size = b.unwrap_or(0);
        let after_size = a.unwrap_or(0);
        let delta = signed_delta(before_size, after_size);
        if delta == 0 {
            continue;
        }
        let state = if b.is_none() {
            DiffState::Added
        } else if a.is_none() {
            DiffState::Removed
        } else {
            DiffState::Changed
        };
        out.push(DiffEntry {
            path,
            before_size,
            after_size,
            delta,
            state,
        });
    }

    out.sort_by(|x, y| {
        let ay = y.delta.abs();
        let ax = x.delta.abs();
        // more-impactful first; deterministic tie-break by path
        ay.cmp(&ax).then_with(|| x.path.cmp(&y.path))
    });
    Ok(out)
}

/// Choose and validate the `(before, after)` snapshot pair.
///
/// - `Default`: the most recent snapshot's root, then its two latest snapshots
///   (older first). Errors appear when there is no snapshot, or fewer than two
///   for that root.
/// - `Explicit`: both ids must exist; a different-root pair is rejected so
///   snapshots are never (silently) compared across roots.
pub fn select_pair(
    storage: &Storage,
    selection: DiffSelection,
) -> Result<(SnapshotRecord, SnapshotRecord)> {
    match selection {
        DiffSelection::Explicit { from, to } => {
            let before = storage
                .get_snapshot(from)?
                .ok_or(WhyBigError::SnapshotNotFound(from))?;
            let after = storage
                .get_snapshot(to)?
                .ok_or(WhyBigError::SnapshotNotFound(to))?;
            if !paths_equal(Path::new(&before.root_path), Path::new(&after.root_path)) {
                return Err(WhyBigError::CrossRootDiff);
            }
            Ok((before, after))
        }
        DiffSelection::Default => {
            let latest = storage.latest_snapshot()?.ok_or(WhyBigError::NoSnapshots)?;
            let root = latest.root_path.clone();
            let snaps = storage.get_latest_snapshots_for_root(&root, 2)?;
            if snaps.len() < 2 {
                return Err(WhyBigError::NotEnoughSnapshots {
                    root,
                    count: snaps.len() as u64,
                });
            }
            // get_latest_snapshots_for_root returns newest first.
            Ok((snaps[1].clone(), snaps[0].clone()))
        }
    }
}
