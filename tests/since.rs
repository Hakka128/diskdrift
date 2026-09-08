//! `--since` selection tests (UTC, deterministic, honest fallback).

use chrono::{TimeZone, Utc};
use tempfile::TempDir;
use whybig::diff::{self, DiffSelection, SinceInfo};
use whybig::storage::models::{EntryToWrite, NewSnapshot};
use whybig::storage::Storage;
use whybig::top::{self, TopMode};

const ROOT: &str = "R";

fn save(storage: &mut Storage, created_ms: i64) -> i64 {
    let input = NewSnapshot {
        created_at_ms: created_ms,
        root_path: ROOT.to_string(),
        total_size: 100,
        file_count: 1,
        dir_count: 0,
        scan_duration_ms: 1,
        skipped_count: 0,
        entries: vec![EntryToWrite {
            path: "R/a".to_string(),
            size: 100,
            file_count: 1,
            dir_count: 0,
        }],
    };
    storage.save_snapshot(&input).unwrap()
}

/// Fixed timeline: id1 at T0, id2 at T0+2h, id3 at T0+4h (latest).
fn seeded() -> (TempDir, Storage) {
    let td = TempDir::new().unwrap();
    let mut storage = Storage::init(td.path()).unwrap();
    let t0 = Utc
        .with_ymd_and_hms(2025, 9, 1, 12, 0, 0)
        .unwrap()
        .timestamp_millis();
    save(&mut storage, t0);
    save(&mut storage, t0 + 2 * 3_600_000);
    save(&mut storage, t0 + 4 * 3_600_000);
    (td, storage)
}

fn since_of(pair: &diff::SelectedPair) -> SinceInfo {
    pair.since.expect("since info present")
}

#[test]
fn picks_snapshot_at_or_before_target() {
    let (_td, storage) = seeded();
    // Target = latest - 90 min = t0+2.5h; closest at-or-before is id2 (t0+2h).
    let pair = diff::select_pair(
        &storage,
        DiffSelection::Since {
            duration: whybig::since::DurationSecs(90 * 60),
        },
    )
    .unwrap();
    assert_eq!(pair.after.id, 3);
    assert_eq!(pair.before.id, 2);
    let info = since_of(&pair);
    assert!(!info.used_earliest);
    let latest = storage.latest_snapshot().unwrap().unwrap();
    let target = latest.created_at_ms - 90 * 60_000;
    assert!(info.effective_before_ms <= target);
}

#[test]
fn exact_target_timestamp_is_selected() {
    let (_td, storage) = seeded();
    // Target exactly = id1's created_at; at-or-before picks id1 exactly.
    let pair = diff::select_pair(
        &storage,
        DiffSelection::Since {
            duration: whybig::since::DurationSecs(4 * 3600),
        },
    )
    .unwrap();
    assert_eq!(pair.before.id, 1);
    assert!(!since_of(&pair).used_earliest);
}

#[test]
fn no_snapshot_before_range_uses_earliest_and_flags() {
    let (_td, storage) = seeded();
    // Ask for -8h: target is before id1 -> earliest fallback id1.
    let pair = diff::select_pair(
        &storage,
        DiffSelection::Since {
            duration: whybig::since::DurationSecs(8 * 3600),
        },
    )
    .unwrap();
    assert_eq!(pair.before.id, 1);
    let info = since_of(&pair);
    assert!(info.used_earliest);
    assert_eq!(info.effective_before_ms, pair.before.created_at_ms);
}

#[test]
fn multiple_snapshots_around_target() {
    let (_td, storage) = seeded();
    // Target = id3 - 3h = t0+1h -> closest at-or-before is id1 (t0), not id2.
    let pair = diff::select_pair(
        &storage,
        DiffSelection::Since {
            duration: whybig::since::DurationSecs(3 * 3600),
        },
    )
    .unwrap();
    assert_eq!(pair.before.id, 1);
}

#[test]
fn timestamp_tie_breaks_deterministically_to_oldest() {
    let td = TempDir::new().unwrap();
    let mut storage = Storage::init(td.path()).unwrap();
    let t0 = Utc
        .with_ymd_and_hms(2025, 9, 1, 12, 0, 0)
        .unwrap()
        .timestamp_millis();
    // Three snapshots sharing the SAME created_at.
    save(&mut storage, t0);
    save(&mut storage, t0);
    save(&mut storage, t0);
    let pair = diff::select_pair(
        &storage,
        DiffSelection::Since {
            duration: whybig::since::DurationSecs(24 * 3600),
        },
    )
    .unwrap();
    assert_eq!(pair.after.id, 3);
    assert_eq!(pair.before.id, 1, "tie breaks to the smallest id");
}

#[test]
fn multiple_roots_do_not_cross() {
    let td = TempDir::new().unwrap();
    let mut storage = Storage::init(td.path()).unwrap();
    let t0 = Utc
        .with_ymd_and_hms(2025, 9, 1, 12, 0, 0)
        .unwrap()
        .timestamp_millis();
    let input_r2_old = NewSnapshot {
        created_at_ms: t0,
        root_path: "R2".to_string(),
        total_size: 1,
        file_count: 0,
        dir_count: 0,
        scan_duration_ms: 0,
        skipped_count: 0,
        entries: vec![],
    };
    let _r2a = storage.save_snapshot(&input_r2_old).unwrap();
    let input_r2_new = NewSnapshot {
        created_at_ms: t0 + 3_600_000,
        root_path: "R2".to_string(),
        total_size: 9,
        file_count: 0,
        dir_count: 0,
        scan_duration_ms: 0,
        skipped_count: 0,
        entries: vec![],
    };
    let _r2b = storage.save_snapshot(&input_r2_new).unwrap();

    let pair = diff::select_pair(
        &storage,
        DiffSelection::Since {
            duration: whybig::since::DurationSecs(30 * 60),
        },
    )
    .unwrap();
    assert_eq!(pair.before.root_path, "R2");
    assert_eq!(pair.after.root_path, "R2");
    assert!(pair.before.created_at_ms <= t0 + 3_600_000 - 30 * 60_000);
}

#[test]
fn scoped_top_supports_since() {
    let (_td, storage) = seeded();
    let report = top::top(
        &storage,
        DiffSelection::Since {
            duration: whybig::since::DurationSecs(90 * 60),
        },
        None,
        TopMode::Growth,
    )
    .unwrap();
    assert!(report.since.is_some());
    assert_eq!(report.before.id, 2);
    assert_eq!(report.after.id, 3);
}

#[test]
fn since_returns_clear_error_when_equal_to_latest_only() {
    let td = TempDir::new().unwrap();
    let mut storage = Storage::init(td.path()).unwrap();
    save(&mut storage, 1_700_000_000_000);
    let err = diff::select_pair(
        &storage,
        DiffSelection::Since {
            duration: whybig::since::DurationSecs(30 * 60),
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("earlier"));
}
