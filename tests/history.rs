//! History service tests: root/directory trends, missing directories as 0,
//! multi-root isolation, limits and ordering.

mod common;

use common::{new_harness, Harness};
use diskdrift::error::Result;
use diskdrift::history::{self, HistoryReport};

fn history_for(h: &Harness, raw: Option<&str>, limit: usize) -> Result<HistoryReport> {
    let storage = h.storage();
    history::history(&storage, raw.map(std::path::Path::new), limit)
}

/// History of a directory relative to the harness root (absolute path arg).
fn dir_hist(h: &Harness, rel: &str, limit: usize) -> Result<HistoryReport> {
    history_for(h, Some(h.abs(rel).to_str().unwrap()), limit)
}

#[test]
fn root_history_is_ascending_with_deltas() {
    let mut h = new_harness();
    h.snap(); // id1: empty
    h.write("x/f", &[1u8; 100]);
    h.snap(); // id2
    h.write("x/g", &[1u8; 200]);
    h.write("y/f", &[1u8; 50]);
    h.snap(); // id3

    let r = history_for(&h, None, 20).unwrap();
    assert_eq!(r.points.len(), 3);
    // ascending ids / timestamps
    let ids: Vec<i64> = r.points.iter().map(|p| p.snapshot_id).collect();
    assert_eq!(ids, vec![1, 2, 3]);
    assert!(r
        .points
        .windows(2)
        .all(|w| w[0].created_at_ms <= w[1].created_at_ms));
    // sizes reflect totals (0, 100, 350)
    let sizes: Vec<u64> = r.points.iter().map(|p| p.size).collect();
    assert_eq!(sizes, vec![0, 100, 350]);
    // deltas: None, +100, +250
    assert_eq!(r.points[0].delta_from_previous, None);
    assert_eq!(r.points[1].delta_from_previous, Some(100));
    assert_eq!(r.points[2].delta_from_previous, Some(250));
    assert_eq!(r.total_delta, 350);
}

#[test]
fn directory_history_tracks_a_subdir() {
    let mut h = new_harness();
    h.write("a/f", &[1u8; 10]);
    h.write("b/f", &[1u8; 20]);
    h.snap(); // id1: a=10
    h.write("a/g", &[1u8; 90]);
    h.snap(); // id2: a=100
    h.write("a/h", &[1u8; 12]);
    h.snap(); // id3: a=112

    let r = dir_hist(&h, "a", 20).unwrap();
    let sizes: Vec<u64> = r.points.iter().map(|p| p.size).collect();
    assert_eq!(sizes, vec![10, 100, 112]);
    assert_eq!(r.total_delta, 102);
    assert_eq!(r.target.to_string_lossy(), h.abs("a").to_string_lossy());
}

#[test]
fn directory_added_halfway_reads_zero_before() {
    let mut h = new_harness();
    h.write("a/f", &[1u8; 10]);
    h.snap(); // c does not exist
    h.write("b/f", &[1u8; 5]);
    h.snap(); // c does not exist
    h.write("c/f", &[1u8; 300]);
    h.snap(); // c = 300

    let r = dir_hist(&h, "c", 20).unwrap();
    let sizes: Vec<u64> = r.points.iter().map(|p| p.size).collect();
    assert_eq!(
        sizes,
        vec![0, 0, 300],
        "no drop of the timeline, zeros before add"
    );
}

#[test]
fn directory_removed_halfway_reads_zero_after() {
    let mut h = new_harness();
    h.write("gone/f", &[1u8; 77]);
    h.snap();
    h.write("other/f", &[1u8; 1]);
    h.snap();
    h.remove("gone");
    h.snap();

    let r = dir_hist(&h, "gone", 20).unwrap();
    let sizes: Vec<u64> = r.points.iter().map(|p| p.size).collect();
    assert_eq!(sizes, vec![77, 77, 0]);
    assert_eq!(r.total_delta, -77);
}

#[test]
fn directory_absent_then_returns() {
    let mut h = new_harness();
    h.write("blip/f", &[1u8; 50]);
    h.snap(); // 50
    h.remove("blip");
    h.snap(); // 0
    h.write("blip/f2", &[1u8; 30]);
    h.snap(); // 30

    let r = dir_hist(&h, "blip", 20).unwrap();
    let sizes: Vec<u64> = r.points.iter().map(|p| p.size).collect();
    assert_eq!(sizes, vec![50, 0, 30]);
}

#[test]
fn multiple_roots_are_isolated() {
    let td = tempfile::TempDir::new().unwrap();
    let data_dir = td.path().join("data");
    let mut service = diskdrift::snapshot::service::SnapshotService::new(data_dir.clone()).unwrap();

    let r1 = td.path().join("r1");
    let r2 = td.path().join("r2");
    std::fs::create_dir_all(&r1).unwrap();
    std::fs::create_dir_all(&r2).unwrap();
    service.run_snapshot(&r1, &mut |_| {}).unwrap();
    service.run_snapshot(&r1, &mut |_| {}).unwrap();
    std::fs::write(r2.join("big"), [1u8; 500]).unwrap();
    service.run_snapshot(&r2, &mut |_| {}).unwrap(); // r2 is now the latest root
    std::fs::write(r2.join("more"), [1u8; 300]).unwrap();
    service.run_snapshot(&r2, &mut |_| {}).unwrap();

    let storage = diskdrift::storage::Storage::open(&data_dir).unwrap();
    let r = history::history(&storage, None, 20).unwrap();
    // Only r2's two snapshots appear.
    assert_eq!(r.points.len(), 2);
    let sizes: Vec<u64> = r.points.iter().map(|p| p.size).collect();
    assert_eq!(sizes, vec![500, 800]);
}

#[test]
fn latest_root_selection_matches_diff() {
    let mut h = new_harness();
    h.snap();
    h.snap();
    let r = history_for(&h, None, 20).unwrap();
    let root_name = r.root.to_string_lossy();
    assert_eq!(root_name, h.root.to_string_lossy());
}

#[test]
fn limit_takes_the_latest_n_ascending() {
    let mut h = new_harness();
    for _ in 0..5 {
        h.snap();
    }
    let r = history_for(&h, None, 3).unwrap();
    assert_eq!(r.points.len(), 3);
    // The three newest, ascending: ids [3,4,5].
    let ids: Vec<i64> = r.points.iter().map(|p| p.snapshot_id).collect();
    assert_eq!(ids, vec![3, 4, 5]);
}

#[test]
fn outside_root_is_rejected() {
    let mut h = new_harness();
    h.snap();
    h.snap();
    let outside = h._td.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    let err = history_for(&h, Some(outside.to_str().unwrap()), 20).unwrap_err();
    assert!(err.to_string().contains("outside tracked root"));
}

#[test]
fn single_snapshot_is_a_valid_one_point_history() {
    let mut h = new_harness();
    h.snap();
    let r = history_for(&h, None, 20).unwrap();
    assert_eq!(r.points.len(), 1);
    assert_eq!(r.total_delta, 0);
    assert_eq!(r.points[0].delta_from_previous, None);
}

#[test]
fn zero_snapshots_is_a_friendly_error() {
    let td = tempfile::TempDir::new().unwrap();
    let data_dir = td.path().join("data");
    let _ = diskdrift::storage::Storage::init(&data_dir).unwrap();
    let storage = diskdrift::storage::Storage::open(&data_dir).unwrap();
    let err = history::history(&storage, None, 20).unwrap_err();
    assert!(err.to_string().contains("no snapshots"));
}

#[test]
fn huge_values_are_handled() {
    let mut h = new_harness();
    std::fs::create_dir_all(h.abs("data")).unwrap();
    // 256/512 MiB: large enough to exercise u64 sizes + i128 deltas without
    // allocating GiBs (Windows set_len is not sparse).
    let f = std::fs::File::create(h.abs("data/big.bin")).unwrap();
    f.set_len(256 * 1024 * 1024).unwrap();
    drop(f);
    h.snap();
    let g = std::fs::File::create(h.abs("data/big2.bin")).unwrap();
    g.set_len(512 * 1024 * 1024).unwrap();
    drop(g);
    h.snap();

    let r = dir_hist(&h, "data", 20).unwrap();
    assert_eq!(r.points[0].size, 256 * 1024 * 1024);
    assert_eq!(
        r.points[1].size,
        768 * 1024 * 1024,
        "big.bin (256M) + big2.bin (512M)"
    );
    assert_eq!(r.total_delta, 512 * 1024 * 1024);
}

#[test]
fn unicode_directory_history() {
    let mut h = new_harness();
    h.write("数据/子/f", &[1u8; 40]);
    h.snap();
    h.write("数据/子/g", &[1u8; 60]);
    h.snap();

    let r = dir_hist(&h, "数据/子", 20).unwrap();
    let sizes: Vec<u64> = r.points.iter().map(|p| p.size).collect();
    assert_eq!(sizes, vec![40, 100]);
}
