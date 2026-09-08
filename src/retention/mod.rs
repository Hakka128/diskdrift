//! Snapshot retention: plan-then-apply pruning of DiskDrift's OWN history.
//!
//! The planner is a pure function (no DB writes) producing a [`PrunePlan`];
//! only [`PruneExecutor`] mutates the database, inside a single transaction, so
//! a failure rolls back the whole prune. Retention NEVER deletes user files —
//! it only deletes `snapshots` rows from DiskDrift's own SQLite database.
//!
//! Buckets are UTC: daily = epoch-day, weekly = epoch-day/7 (a documented
//! 7-day bucket, deliberately not ISO-week, to avoid year/53 edge complexity).

use std::collections::BTreeMap;

use crate::error::{DiskDriftError, Result};
use crate::storage::models::SnapshotRecord;
use crate::storage::Storage;

/// One UTC day in milliseconds.
pub const DAY_MS: i64 = 86_400_000;

/// Retention tiers (see DESIGN-M4 §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionPolicy {
    /// Age ≤ this many days → keep every snapshot.
    pub recent_days: u32,
    /// recent_days < age ≤ daily_days → keep one per UTC day.
    pub daily_days: u32,
    /// daily_days < age ≤ weekly_days → keep one per UTC 7-day bucket.
    pub weekly_days: u32,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            recent_days: 7,
            daily_days: 30,
            weekly_days: 365,
        }
    }
}

impl RetentionPolicy {
    /// `recent >= 1`, `daily > recent`, `weekly > daily`.
    pub fn validate(&self) -> Result<()> {
        let ok = self.recent_days >= 1
            && self.daily_days > self.recent_days
            && self.weekly_days > self.daily_days;
        if !ok {
            return Err(DiskDriftError::BadPolicy(format!(
                "recent_days={} daily_days={} weekly_days={} (need 1 <= recent < daily < weekly)",
                self.recent_days, self.daily_days, self.weekly_days
            )));
        }
        Ok(())
    }
}

/// Per-root keep/remove breakdown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootPrunePlan {
    pub root: String,
    pub snapshot_count: usize,
    pub keep_recent: Vec<i64>,
    pub keep_daily: Vec<i64>,
    pub keep_weekly: Vec<i64>,
    pub remove: Vec<i64>,
}

impl RootPrunePlan {
    pub fn keep(&self) -> usize {
        self.keep_recent.len() + self.keep_daily.len() + self.keep_weekly.len()
    }

    /// All kept snapshot ids for this root (sorted).
    pub fn keep_ids(&self) -> Vec<i64> {
        let mut out = [
            self.keep_recent.clone(),
            self.keep_daily.clone(),
            self.keep_weekly.clone(),
        ]
        .concat();
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// The full retention plan. Preview and `--apply` share exactly this plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrunePlan {
    /// UTC ms at planning time (for human display).
    pub generated_at_ms: i64,
    pub policy: RetentionPolicy,
    pub roots: Vec<RootPrunePlan>,
}

impl PrunePlan {
    pub fn keep_ids(&self) -> Vec<i64> {
        let mut out = Vec::new();
        for r in &self.roots {
            out.extend(r.keep_recent.iter().copied());
            out.extend(r.keep_daily.iter().copied());
            out.extend(r.keep_weekly.iter().copied());
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    pub fn remove_ids(&self) -> Vec<i64> {
        let mut out = Vec::new();
        for r in &self.roots {
            out.extend(r.remove.iter().copied());
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    pub fn total_snapshots(&self) -> usize {
        self.roots.iter().map(|r| r.snapshot_count).sum()
    }

    pub fn total_keep(&self) -> usize {
        self.keep_ids().len()
    }

    pub fn total_remove(&self) -> usize {
        self.remove_ids().len()
    }
}

fn epoch_day(ms: i64) -> i64 {
    ms.div_euclid(DAY_MS)
}

/// Pure planner. `now_ms` is injected so tests can fix time (production passes
/// `Utc::now()`).
pub fn build_plan(
    policy: &RetentionPolicy,
    snapshots: &[SnapshotRecord],
    now_ms: i64,
) -> Result<PrunePlan> {
    policy.validate()?;

    let mut by_root: BTreeMap<String, Vec<&SnapshotRecord>> = BTreeMap::new();
    for s in snapshots {
        by_root.entry(s.root_path.clone()).or_default().push(s);
    }

    let recent_ms = i64::from(policy.recent_days) * DAY_MS;
    let daily_ms = i64::from(policy.daily_days) * DAY_MS;
    let weekly_ms = i64::from(policy.weekly_days) * DAY_MS;

    let mut roots = Vec::with_capacity(by_root.len());
    for (root, mut recs) in by_root {
        // oldest→newest (created_at, then id for determinism).
        recs.sort_by(|a, b| {
            a.created_at_ms
                .cmp(&b.created_at_ms)
                .then_with(|| a.id.cmp(&b.id))
        });

        let mut keep_recent = Vec::new();
        let mut keep_daily = Vec::new();
        let mut keep_weekly = Vec::new();
        let latest_id = recs.last().map(|s| s.id);

        // Daily/weekly buckets: value = latest candidate kept (ascending
        // iteration ⇒ later candidate wins ties deterministically).
        let mut daily: BTreeMap<i64, i64> = BTreeMap::new();
        let mut weekly: BTreeMap<i64, i64> = BTreeMap::new();

        for s in recs.iter() {
            let age = now_ms.saturating_sub(s.created_at_ms);
            if age <= recent_ms {
                keep_recent.push(s.id);
            } else if age <= daily_ms {
                daily.insert(epoch_day(s.created_at_ms), s.id);
            } else if age <= weekly_ms {
                weekly.insert(epoch_day(s.created_at_ms) / 7, s.id);
            }
            // else: beyond the retention window → not kept (removed below).
        }
        keep_daily.extend(daily.values().copied());
        keep_weekly.extend(weekly.values().copied());

        // Invariant: the root's LATEST snapshot is always preserved, even if
        // it would otherwise be culled (only possible when every snapshot of
        // the root is older than the whole window).
        if let Some(id) = latest_id {
            if !keep_recent.contains(&id) && !keep_daily.contains(&id) && !keep_weekly.contains(&id)
            {
                keep_recent.push(id);
            }
        }
        keep_recent.sort_unstable();
        keep_daily.sort_unstable();
        keep_weekly.sort_unstable();

        // Anything not kept is removed — including bucket losers (older
        // snapshots that lost their slot) and everything beyond the window.
        let keep_set: std::collections::BTreeSet<i64> = [&keep_recent, &keep_daily, &keep_weekly]
            .into_iter()
            .flatten()
            .copied()
            .collect();
        let mut remove: Vec<i64> = recs
            .iter()
            .map(|s| s.id)
            .filter(|id| !keep_set.contains(id))
            .collect();
        remove.sort_unstable();

        roots.push(RootPrunePlan {
            root,
            snapshot_count: recs.len(),
            keep_recent,
            keep_daily,
            keep_weekly,
            remove,
        });
    }

    roots.sort_by(|a, b| a.root.cmp(&b.root));
    Ok(PrunePlan {
        generated_at_ms: now_ms,
        policy: *policy,
        roots,
    })
}

/// What actually happened when applying a plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PruneSummary {
    pub snapshots_removed: i64,
}

/// The only component allowed to mutate the database for pruning. Re-checks:
/// never removes a root's latest snapshot, never removes a single-snapshot
/// root, and each root's newest snapshot survives — then executes one
/// transaction so a mid-way failure rolls back (entries cascade via FK).
pub struct PruneExecutor;

impl PruneExecutor {
    pub fn apply(storage: &mut Storage, plan: &PrunePlan) -> Result<PruneSummary> {
        let ids = plan.remove_ids();
        if ids.is_empty() {
            return Ok(PruneSummary {
                snapshots_removed: 0,
            });
        }

        // Safety net (defence in depth; the planner already guarantees this):
        // no root's latest snapshot may be in the removal set. That also covers
        // single-snapshot roots (their only snapshot IS the latest).
        let all = storage.list_snapshots()?;
        let mut latest_id_by_root: BTreeMap<String, i64> = BTreeMap::new();
        for s in &all {
            let entry = latest_id_by_root.entry(s.root_path.clone()).or_insert(s.id);
            if s.id > *entry {
                *entry = s.id;
            }
        }
        let remove_set: std::collections::HashSet<i64> = ids.iter().copied().collect();
        for (root, latest_id) in &latest_id_by_root {
            if remove_set.contains(latest_id) {
                return Err(DiskDriftError::PruneSafety(format!(
                    "refusing to delete the latest snapshot of root `{root}`"
                )));
            }
        }

        let before = storage.count_snapshots()?;
        let removed = storage.delete_snapshots(&ids)?;
        let after = storage.count_snapshots()?;
        debug_assert_eq!(before - removed, after);
        Ok(PruneSummary {
            snapshots_removed: removed,
        })
    }
}
