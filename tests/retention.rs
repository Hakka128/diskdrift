//! Retention planner/executor tests.
//!
//! `now` is injected so bucket edges (daily/weekly/365d) are exact and
//! deterministic without sleeping.

use chrono::{TimeZone, Utc};
use diskdrift::json::JsonPruneReportV1;
use diskdrift::retention::{build_plan, PruneExecutor, PrunePlan, RetentionPolicy};
use diskdrift::storage::models::{EntryToWrite, NewSnapshot};
use diskdrift::storage::Storage;
use tempfile::TempDir;

const NOW: i64 = 1_768_032_000_000; // 2026-01-10T00:00:00Z

fn dt(y: i32, m: u32, d: u32, h: u32, min: u32) -> i64 {
    Utc.with_ymd_and_hms(y, m, d, h, min, 0)
        .unwrap()
        .timestamp_millis()
}

fn day_ms() -> i64 {
    86_400_000
}

fn mk_snapshot(
    storage: &mut Storage,
    root: &str,
    created_ms: i64,
    with_entry_name: Option<&str>,
) -> i64 {
    let entries = match with_entry_name {
        Some(name) => vec![EntryToWrite {
            path: format!("{root}/{name}"),
            size: 5,
            file_count: 1,
            dir_count: 0,
        }],
        None => vec![],
    };
    let input = NewSnapshot {
        created_at_ms: created_ms,
        root_path: root.to_string(),
        total_size: entries.iter().map(|e| e.size).sum(),
        file_count: entries.len() as u64,
        dir_count: 0,
        scan_duration_ms: 1,
        skipped_count: 0,
        entries,
    };
    storage.save_snapshot(&input).unwrap()
}

fn plan_for(storage: &Storage, policy: RetentionPolicy) -> PrunePlan {
    let snaps = storage.list_snapshots().unwrap();
    build_plan(&policy, &snaps, NOW).unwrap()
}

#[test]
fn recent_snapshots_are_all_kept() {
    let td = TempDir::new().unwrap();
    let mut storage = Storage::init(td.path()).unwrap();
    mk_snapshot(&mut storage, "R", NOW - day_ms(), None);
    mk_snapshot(&mut storage, "R", NOW - 6 * day_ms(), None);
    mk_snapshot(&mut storage, "R", NOW - day_ms() / 2, None);

    let plan = plan_for(&storage, RetentionPolicy::default());
    assert_eq!(plan.total_remove(), 0);
    assert_eq!(plan.total_keep(), 3);
}

#[test]
fn daily_bucketing_keeps_one_per_day() {
    let td = TempDir::new().unwrap();
    let mut storage = Storage::init(td.path()).unwrap();
    let d8 = NOW - 8 * day_ms();
    mk_snapshot(&mut storage, "R", d8, None); // latest of its day -> kept
    mk_snapshot(&mut storage, "R", d8 - 30 * 60_000, None); // older same day -> removed
    mk_snapshot(&mut storage, "R", d8 - 2 * day_ms(), None); // previous day -> kept

    let plan = plan_for(&storage, RetentionPolicy::default());
    let root = &plan.roots[0];
    assert_eq!(root.keep_daily.len(), 2, "one per day");
    assert_eq!(root.remove.len(), 1, "the older same-day snapshot");
}

#[test]
fn weekly_bucketing_keeps_one_per_seven_days() {
    let td = TempDir::new().unwrap();
    let mut storage = Storage::init(td.path()).unwrap();
    let d40 = NOW - 40 * day_ms();
    mk_snapshot(&mut storage, "R", d40, None);
    mk_snapshot(&mut storage, "R", d40 + day_ms(), None); // same week -> older removed
    mk_snapshot(&mut storage, "R", d40 + 8 * day_ms(), None); // different week -> kept

    let plan = plan_for(&storage, RetentionPolicy::default());
    let root = &plan.roots[0];
    assert_eq!(root.keep_weekly.len(), 2);
    assert_eq!(root.remove.len(), 1);
}

#[test]
fn older_than_365_days_removed() {
    let td = TempDir::new().unwrap();
    let mut storage = Storage::init(td.path()).unwrap();
    mk_snapshot(&mut storage, "R", NOW - 400 * day_ms(), None);
    mk_snapshot(&mut storage, "R", NOW - 366 * day_ms(), None);
    mk_snapshot(&mut storage, "R", NOW - 370 * day_ms(), None);
    mk_snapshot(&mut storage, "R", NOW - day_ms(), None); // recent

    let plan = plan_for(&storage, RetentionPolicy::default());
    let root = &plan.roots[0];
    assert_eq!(root.remove.len(), 3, "all beyond the weekly window");
    assert!(root.keep_recent.contains(&4));
}

#[test]
fn latest_snapshot_always_kept_even_when_very_old() {
    let td = TempDir::new().unwrap();
    let mut storage = Storage::init(td.path()).unwrap();
    let old_a = mk_snapshot(&mut storage, "R", NOW - 400 * day_ms(), None);
    let latest = mk_snapshot(&mut storage, "R", NOW - 399 * day_ms(), None);

    let plan = plan_for(&storage, RetentionPolicy::default());
    let root = &plan.roots[0];
    assert_eq!(root.remove, vec![old_a]);
    assert!(root.keep_ids().contains(&latest));
    assert!(!root.keep_ids().contains(&old_a));
}

#[test]
fn single_snapshot_root_is_never_pruned() {
    let td = TempDir::new().unwrap();
    let mut storage = Storage::init(td.path()).unwrap();
    let _id = mk_snapshot(&mut storage, "R", NOW - 500 * day_ms(), None);

    let plan = plan_for(&storage, RetentionPolicy::default());
    let root = &plan.roots[0];
    assert!(root.remove.is_empty());
    assert_eq!(root.keep(), 1);
    assert_eq!(plan.total_remove(), 0);
}

#[test]
fn multiple_roots_are_isolated() {
    let td = TempDir::new().unwrap();
    let mut storage = Storage::init(td.path()).unwrap();
    let d8 = NOW - 8 * day_ms();
    mk_snapshot(&mut storage, "R1", NOW - day_ms(), None);
    mk_snapshot(&mut storage, "R2", d8, None);
    mk_snapshot(&mut storage, "R2", d8 - 30 * 60_000, None);

    let plan = plan_for(&storage, RetentionPolicy::default());
    assert_eq!(plan.roots.len(), 2);
    let r1 = plan.roots.iter().find(|r| r.root == "R1").unwrap();
    let r2 = plan.roots.iter().find(|r| r.root == "R2").unwrap();
    assert_eq!(r1.remove.len(), 0, "single-snapshot root untouched");
    assert_eq!(
        r2.remove.len(),
        1,
        "R2 loses only its older same-day snapshot"
    );
}

#[test]
fn dry_run_does_not_mutate() {
    let td = TempDir::new().unwrap();
    let mut storage = Storage::init(td.path()).unwrap();
    mk_snapshot(&mut storage, "R", NOW - 400 * day_ms(), None);
    mk_snapshot(&mut storage, "R", NOW - day_ms(), None);

    let before = storage.count_snapshots().unwrap();
    let plan = plan_for(&storage, RetentionPolicy::default());
    assert!(plan.total_remove() > 0);
    assert_eq!(
        storage.count_snapshots().unwrap(),
        before,
        "planning must not write"
    );
    assert_eq!(plan.total_keep() + plan.total_remove(), before as usize);
}

#[test]
fn apply_deletes_and_entries_cascade() {
    let td = TempDir::new().unwrap();
    let mut storage = Storage::init(td.path()).unwrap();
    let doomed = mk_snapshot(&mut storage, "R", NOW - 400 * day_ms(), Some("old"));
    let keep = mk_snapshot(&mut storage, "R", NOW - day_ms(), Some("new"));
    assert_eq!(storage.snapshot_entries_count(doomed).unwrap(), 1);

    let plan = plan_for(&storage, RetentionPolicy::default());
    let summary = PruneExecutor::apply(&mut storage, &plan).unwrap();
    assert_eq!(summary.snapshots_removed, 1);
    assert_eq!(storage.count_snapshots().unwrap(), 1);
    assert!(storage.get_snapshot(doomed).unwrap().is_none());
    assert!(storage.get_snapshot(keep).unwrap().is_some());
    // FK cascade removed the entries row of the doomed snapshot.
    assert_eq!(storage.snapshot_entries_count(doomed).unwrap(), 0);
}

#[test]
fn apply_transaction_rolls_back_on_failure() {
    let td = TempDir::new().unwrap();
    let mut storage = Storage::init(td.path()).unwrap();
    mk_snapshot(&mut storage, "R", NOW - 400 * day_ms(), None);
    mk_snapshot(&mut storage, "R", NOW - day_ms(), None);
    let before = storage.count_snapshots().unwrap();

    storage
        .connection()
        .execute_batch(
            "CREATE TRIGGER block_prune BEFORE DELETE ON snapshots \
             BEGIN SELECT RAISE(ABORT, 'injected'); END;",
        )
        .unwrap();

    let plan = plan_for(&storage, RetentionPolicy::default());
    let err = PruneExecutor::apply(&mut storage, &plan).unwrap_err();
    assert!(err.to_string().contains("injected"));
    assert_eq!(
        storage.count_snapshots().unwrap(),
        before,
        "no partial removal after rollback"
    );
    storage
        .connection()
        .execute_batch("DROP TRIGGER block_prune")
        .unwrap();
}

#[test]
fn repeated_apply_is_idempotent() {
    let td = TempDir::new().unwrap();
    let mut storage = Storage::init(td.path()).unwrap();
    mk_snapshot(&mut storage, "R", NOW - 400 * day_ms(), None);
    mk_snapshot(&mut storage, "R", NOW - day_ms(), None);

    let plan = plan_for(&storage, RetentionPolicy::default());
    let first = PruneExecutor::apply(&mut storage, &plan).unwrap();
    assert_eq!(first.snapshots_removed, 1);
    let second = PruneExecutor::apply(&mut storage, &plan).unwrap();
    assert_eq!(second.snapshots_removed, 0, "already gone, nothing to do");
    assert_eq!(storage.count_snapshots().unwrap(), 1);
}

#[test]
fn year_and_week_boundaries_are_stable() {
    // Snapshots straddling the year boundary (2025-12-31 vs 2026-01-01) must
    // bucket deterministically without panics or cross-bucket merging.
    let td = TempDir::new().unwrap();
    let mut storage = Storage::init(td.path()).unwrap();
    let dec31 = dt(2025, 12, 31, 23, 59);
    let jan1 = dt(2026, 1, 1, 0, 1);
    let jan2 = dt(2026, 1, 2, 0, 1);
    mk_snapshot(&mut storage, "R", dec31, None);
    mk_snapshot(&mut storage, "R", jan1, None);
    mk_snapshot(&mut storage, "R", jan2, None);
    mk_snapshot(&mut storage, "R", NOW, None); // latest, recent

    let plan = plan_for(&storage, RetentionPolicy::default());
    let total: usize = plan
        .roots
        .iter()
        .map(|r| r.keep_daily.len() + r.keep_weekly.len() + r.keep_recent.len())
        .sum();
    assert_eq!(total, 4, "each remains in its own bucket / recent");
    assert_eq!(plan.total_remove() + plan.total_keep(), 4);
}

#[test]
fn future_timestamp_is_kept_not_crashed() {
    let td = TempDir::new().unwrap();
    let mut storage = Storage::init(td.path()).unwrap();
    mk_snapshot(&mut storage, "R", NOW + 2 * day_ms(), None); // in the future
    mk_snapshot(&mut storage, "R", NOW, None);

    let plan = plan_for(&storage, RetentionPolicy::default());
    assert_eq!(plan.total_remove(), 0, "future snapshot treated as recent");
    assert_eq!(plan.total_keep(), 2);
}

#[test]
fn invalid_policy_is_rejected() {
    let td = TempDir::new().unwrap();
    let mut storage = Storage::init(td.path()).unwrap();
    mk_snapshot(&mut storage, "R", NOW, None);
    let bad = RetentionPolicy {
        recent_days: 30,
        daily_days: 7,
        weekly_days: 365,
    };
    let snaps = storage.list_snapshots().unwrap();
    let err = build_plan(&bad, &snaps, NOW).unwrap_err();
    assert!(err.to_string().contains("retention policy"));
}

#[test]
fn executor_refuses_to_delete_a_root_latest() {
    let td = TempDir::new().unwrap();
    let mut storage = Storage::init(td.path()).unwrap();
    let solo = mk_snapshot(&mut storage, "R", NOW, None);
    // A malicious/corrupt plan that removes the only (latest) snapshot.
    let plan = PrunePlan {
        generated_at_ms: NOW,
        policy: RetentionPolicy::default(),
        roots: vec![diskdrift::retention::RootPrunePlan {
            root: "R".to_string(),
            snapshot_count: 1,
            keep_recent: vec![],
            keep_daily: vec![],
            keep_weekly: vec![solo],
            remove: vec![solo],
        }],
    };
    let err = PruneExecutor::apply(&mut storage, &plan).unwrap_err();
    assert!(err.to_string().contains("refusing"));
    assert_eq!(storage.count_snapshots().unwrap(), 1);
}

#[test]
fn prune_json_preview_and_applied() {
    let td = TempDir::new().unwrap();
    let mut storage = Storage::init(td.path()).unwrap();
    mk_snapshot(&mut storage, "R", NOW - 400 * day_ms(), None);
    mk_snapshot(&mut storage, "R", NOW - day_ms(), None);
    let plan = plan_for(&storage, RetentionPolicy::default());
    let before = storage.count_snapshots().unwrap();

    let doc = JsonPruneReportV1::build(&plan, false, before, storage.database_size(), None);
    let text = diskdrift::json::serialize(&doc).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["schema_version"], 1);
    assert_eq!(v["command"], "prune");
    assert_eq!(v["applied"], false);
    assert_eq!(v["snapshots_before"], 2);
    assert_eq!(v["snapshots_remove"], 1);
    assert_eq!(v["policy"]["recent_days"], 7);
    assert_eq!(v["roots"][0]["root"], "R");
}
