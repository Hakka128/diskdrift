//! JSON API golden/snapshot tests.
//!
//! The public JSON documents are built from domain models (never by
//! serializing internal structs) and asserted against exact `Value` goldens
//! with fixed snapshot timestamps, plus a byte-exact canonical-string golden
//! that locks field order.

use std::path::MAIN_SEPARATOR;

use diskdrift::diff::{self, DiffSelection};
use diskdrift::history;
use diskdrift::json::{
    serialize, JsonDiffReportV1, JsonHistoryReportV1, JsonInspectReportV1, JsonStatusReportV1,
    JsonTopReportV1,
};
use diskdrift::snapshot::service::SnapshotService;
use diskdrift::storage::models::{EntryToWrite, NewSnapshot, SnapshotRecord};
use diskdrift::storage::Storage;
use diskdrift::top::{self, TopMode};
use serde_json::{json, Value};
use tempfile::TempDir;

/// Owned `(before, after)` pair for the fixed snapshots.
fn select_pair_direct(storage: &Storage, id1: i64, id2: i64) -> (SnapshotRecord, SnapshotRecord) {
    let pair = diff::select_pair(storage, DiffSelection::Explicit { from: id1, to: id2 }).unwrap();
    (pair.before, pair.after)
}

const T1: i64 = 1_600_000_000_000; // 2020-09-13T12:26:40Z
const T2: i64 = 1_600_000_100_000;

fn rfc(ms: i64) -> String {
    use chrono::{DateTime, Utc};
    DateTime::<Utc>::from_timestamp_millis(ms)
        .unwrap()
        .to_rfc3339()
}

fn key(root: &str, name: &str) -> String {
    format!("{root}{}{name}", MAIN_SEPARATOR)
}

/// Save a snapshot with a fixed timestamp and explicit sizes (native separators).
fn save_snapshot_fixed(
    storage: &mut Storage,
    root: &str,
    created_ms: i64,
    total: u64,
    entries: &[(&str, u64)],
) -> i64 {
    let rows: Vec<EntryToWrite> = entries
        .iter()
        .map(|(name, size)| EntryToWrite {
            path: key(root, name),
            size: *size,
            file_count: 1,
            dir_count: 0,
        })
        .collect();
    let input = NewSnapshot {
        created_at_ms: created_ms,
        root_path: root.to_string(),
        total_size: total,
        file_count: rows.len() as u64,
        dir_count: 0,
        scan_duration_ms: 1,
        skipped_count: 0,
        entries: rows,
    };
    storage.save_snapshot(&input).unwrap()
}

/// Two fixed snapshots: a=const, b 400→700 (+300), c added (+600).
fn seed_two_snapshots() -> (TempDir, Storage, i64, i64) {
    let td = TempDir::new().unwrap();
    let mut storage = Storage::init(td.path()).unwrap();
    let root = td.path().to_string_lossy().into_owned();
    let id1 = save_snapshot_fixed(&mut storage, &root, T1, 1000, &[("a", 600), ("b", 400)]);
    let id2 = save_snapshot_fixed(
        &mut storage,
        &root,
        T2,
        1900,
        &[("a", 600), ("b", 700), ("c", 600)],
    );
    (td, storage, id1, id2)
}

fn parse(s: &str) -> Value {
    serde_json::from_str(s).expect("valid JSON")
}

#[test]
fn diff_golden_schema_and_values() {
    let (_td, storage, id1, id2) = seed_two_snapshots();
    let (before, after) = select_pair_direct(&storage, id1, id2);
    let d = diff::compute(&storage, &before, &after, &after.root_path).unwrap();
    let doc = JsonDiffReportV1::build(&d);
    let text = serialize(&doc).unwrap();
    let root = after.root_path.clone();

    // 35: negative-ish check happens elsewhere; here the golden is exact.
    let expect = json!({
        "schema_version": 1,
        "command": "diff",
        "root": root,
        "before": {"snapshot_id": id1, "created_at": rfc(T1), "size_bytes": 1000},
        "after": {"snapshot_id": id2, "created_at": rfc(T2), "size_bytes": 1900},
        "total_before_bytes": 1000,
        "total_after_bytes": 1900,
        "delta_bytes": 900,
        "grew": [
            {"path": key(&root, "c"), "before_size_bytes": 0, "after_size_bytes": 600, "delta_bytes": 600, "state": "added"},
            {"path": key(&root, "b"), "before_size_bytes": 400, "after_size_bytes": 700, "delta_bytes": 300, "state": "changed"}
        ],
        "shrank": []
    });
    assert_eq!(parse(&text), expect, "stdout json:\n{text}");
    // The exact serialized fragment locks field order too (this is the dogfood
    // of "key 稳定" — the schema's canonical order is a contract).
    let root_esc = serde_json::to_string(&root).unwrap();
    let c = serde_json::to_string(&key(&root, "c")).unwrap();
    let b = serde_json::to_string(&key(&root, "b")).unwrap();
    let expected = format!(
        r#"{{"schema_version":1,"command":"diff","root":{root_esc},"before":{{"snapshot_id":{id1},"created_at":"{c1}","size_bytes":1000}},"after":{{"snapshot_id":{id2},"created_at":"{c2}","size_bytes":1900}},"total_before_bytes":1000,"total_after_bytes":1900,"delta_bytes":900,"grew":[{{"path":{c},"before_size_bytes":0,"after_size_bytes":600,"delta_bytes":600,"state":"added"}},{{"path":{b},"before_size_bytes":400,"after_size_bytes":700,"delta_bytes":300,"state":"changed"}}],"shrank":[]}}"#,
        root_esc = root_esc,
        id1 = id1,
        c1 = rfc(T1),
        id2 = id2,
        c2 = rfc(T2),
        c = c,
        b = b,
    );
    assert_eq!(
        json_to_string(&doc),
        expected,
        "canonical key order must be stable"
    );
}

fn json_to_string<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap()
}

#[test]
fn inspect_golden_schema_and_values() {
    let (_td, storage, id1, id2) = seed_two_snapshots();
    let (before, after) = select_pair_direct(&storage, id1, id2);
    let root = after.root_path.clone();
    let target = key(&root, "b");
    let report = diskdrift::inspect::inspect(
        &storage,
        &before,
        &after,
        &root,
        std::path::Path::new(&target),
    )
    .unwrap();
    let doc = JsonInspectReportV1::build(&report);
    let text = serialize(&doc).unwrap();

    let expect = json!({
        "schema_version": 1,
        "command": "inspect",
        "root": root,
        "path": target,
        "before": {"snapshot_id": id1, "created_at": rfc(T1), "size_bytes": 400},
        "after": {"snapshot_id": id2, "created_at": rfc(T2), "size_bytes": 700},
        "target_before_bytes": 400,
        "target_after_bytes": 700,
        "target_delta_bytes": 300,
        "other_bytes": 300,
        "contributors": []
    });
    assert_eq!(parse(&text), expect, "stdout json:\n{text}");
}

#[test]
fn history_golden_schema_and_values() {
    let (_td, storage, id1, id2) = seed_two_snapshots();
    let root = storage.latest_snapshot().unwrap().unwrap().root_path;
    let target = key(&root, "c");
    let report = history::history(&storage, Some(std::path::Path::new(&target)), 20).unwrap();
    let doc = JsonHistoryReportV1::build(&report);
    let text = serialize(&doc).unwrap();

    let expect = json!({
        "schema_version": 1,
        "command": "history",
        "root": root,
        "path": target,
        "points": [
            {"snapshot_id": id1, "created_at": rfc(T1), "size_bytes": 0, "delta_from_previous_bytes": null},
            {"snapshot_id": id2, "created_at": rfc(T2), "size_bytes": 600, "delta_from_previous_bytes": 600}
        ],
        "total_delta_bytes": 600
    });
    assert_eq!(parse(&text), expect, "stdout json:\n{text}");
}

#[test]
fn top_golden_schema_and_values() {
    let (_td, storage, id1, id2) = seed_two_snapshots();
    let report = top::top(
        &storage,
        DiffSelection::Explicit { from: id1, to: id2 },
        None,
        TopMode::Growth,
    )
    .unwrap();
    let doc = JsonTopReportV1::build(&report);
    let text = serialize(&doc).unwrap();
    let root = report.root.clone();

    let expect = json!({
        "schema_version": 1,
        "command": "top",
        "root": root,
        "path": root,
        "mode": "grew",
        "before": {"snapshot_id": id1, "created_at": rfc(T1), "size_bytes": 1000},
        "after": {"snapshot_id": id2, "created_at": rfc(T2), "size_bytes": 1900},
        "entries": [
            {"path": key(&root, "c"), "before_size_bytes": 0, "after_size_bytes": 600, "delta_bytes": 600, "state": "added"},
            {"path": key(&root, "b"), "before_size_bytes": 400, "after_size_bytes": 700, "delta_bytes": 300, "state": "changed"}
        ]
    });
    assert_eq!(parse(&text), expect, "stdout json:\n{text}");
}

#[test]
fn status_json_is_stable_and_schema_versioned() {
    let (_td, storage, id1, id2) = seed_two_snapshots();
    let data_dir = storage.database_path().parent().unwrap().to_path_buf();
    // Open a dedicated path through the public status service.
    drop(storage);
    let report = SnapshotService::status(data_dir).unwrap();
    let doc = JsonStatusReportV1::build(&report);
    let text = serialize(&doc).unwrap();
    let v = parse(&text);
    assert_eq!(v["schema_version"], 1);
    assert_eq!(v["command"], "status");
    assert_eq!(v["initialized"], true);
    assert_eq!(v["snapshot_count"], 2);
    assert!(v["database_size_bytes"].is_u64());
    assert_eq!(v["earliest"]["snapshot_id"], id1);
    assert_eq!(v["earliest"]["size_bytes"], 1000);
    assert_eq!(v["latest"]["snapshot_id"], id2);
    assert!(v["data_dir"].is_string());
    assert!(v["database_path"].is_string());
}

#[test]
fn negative_delta_is_preserved_and_signed() {
    let td = TempDir::new().unwrap();
    let mut storage = Storage::init(td.path()).unwrap();
    let root = td.path().to_string_lossy().into_owned();
    let id1 = save_snapshot_fixed(&mut storage, &root, T1, 500, &[("b", 400)]);
    let id2 = save_snapshot_fixed(&mut storage, &root, T2, 200, &[("b", 100)]);

    let (before, after) = select_pair_direct(&storage, id1, id2);
    let d = diff::compute(&storage, &before, &after, &after.root_path).unwrap();
    let v = parse(&serialize(&JsonDiffReportV1::build(&d)).unwrap());
    assert_eq!(v["delta_bytes"], -300);
    let shrank = v["shrank"].as_array().unwrap();
    assert_eq!(shrank.len(), 1);
    assert_eq!(shrank[0]["delta_bytes"], -300);
    assert_eq!(shrank[0]["state"], "changed");
}

#[test]
fn unicode_path_serializes_round_trip() {
    let td = TempDir::new().unwrap();
    let mut storage = Storage::init(td.path()).unwrap();
    let root = td.path().to_string_lossy().into_owned();
    let dir = key(&root, "数据");
    let id1 = save_snapshot_fixed(&mut storage, &root, T1, 100, &[("数据", 100)]);
    let id2 = save_snapshot_fixed(&mut storage, &root, T2, 200, &[("数据", 200)]);

    let (before, after) = select_pair_direct(&storage, id1, id2);
    let d = diff::compute(&storage, &before, &after, &after.root_path).unwrap();
    let v = parse(&serialize(&JsonDiffReportV1::build(&d)).unwrap());
    let grew_path = v["grew"][0]["path"].as_str().unwrap();
    assert_eq!(grew_path, dir);
}

#[test]
fn json_output_has_no_ansi_and_is_a_single_document() {
    let (_td, storage, id1, id2) = seed_two_snapshots();
    let (before, after) = select_pair_direct(&storage, id1, id2);
    let d = diff::compute(&storage, &before, &after, &after.root_path).unwrap();
    let text = serialize(&JsonDiffReportV1::build(&d)).unwrap();
    assert!(!text.contains('\u{1b}'), "no ANSI escape sequences");
    let v: Value = serde_json::from_str(&text).unwrap();
    assert!(v.is_object());
}
