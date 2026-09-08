//! Snapshot + storage integration tests: the service ↔ SQLite boundary,
//! transactions, foreign keys, corruption handling and idempotent init.

use std::path::{Path, PathBuf};

use diskdrift::scanner::{scan, ScanOptions};
use diskdrift::snapshot::service::{SnapshotService, StatusReport};
use diskdrift::storage::models::{i64_to_u64, u64_to_i64, NewSnapshot};
use diskdrift::storage::{migrations, Storage};
use tempfile::TempDir;

/// Scan a fixture dir and save it via the service, returning the outcome.
fn snapshot_fixture(service: &mut SnapshotService, dir: &Path) -> i64 {
    service
        .run_snapshot(dir, &mut |_| {})
        .expect("snapshot succeeds")
        .snapshot_id
}

// ─── init ────────────────────────────────────────────────────────────────

#[test]
fn init_creates_an_empty_migrated_database() {
    let td = TempDir::new().unwrap();
    let data_dir = td.path().join("data");
    let storage = Storage::init(&data_dir).unwrap();

    assert!(storage.database_path().exists());
    assert_eq!(storage.count_snapshots().unwrap(), 0);
    // schema is at the latest migration (v2 adds the meta table)
    let conn = storage.connection();
    assert_eq!(migrations::schema_version(conn).unwrap(), 2);
    // both tables + index exist
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type IN ('table','index') ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert!(tables.contains(&"snapshots".to_string()));
    assert!(tables.contains(&"entries".to_string()));
    assert!(tables.contains(&"idx_entries_path".to_string()));
}

#[test]
fn init_is_idempotent_and_preserves_data() {
    let td = TempDir::new().unwrap();
    let data_dir = td.path().join("data");

    let mut first = Storage::init(&data_dir).unwrap();
    let id = first.save_snapshot(&dummy_snapshot(&data_dir, 0)).unwrap();

    // Re-init must not error, re-create, or lose data.
    let second = Storage::init(&data_dir).unwrap();
    assert_eq!(second.count_snapshots().unwrap(), 1);
    assert_eq!(second.latest_snapshot().unwrap().unwrap().id, id);
}

// ─── save / load round-trip ──────────────────────────────────────────────

#[test]
fn snapshot_round_trip_persists_correct_totals() {
    let td = TempDir::new().unwrap();
    let root = td.path().join("tree");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("a.txt"), vec![1u8; 100]).unwrap();
    std::fs::write(root.join("b.txt"), vec![2u8; 200]).unwrap();
    std::fs::write(root.join("sub").join("c.txt"), vec![3u8; 50]).unwrap();

    let data_dir = td.path().join("data");
    let mut service = SnapshotService::new(data_dir.clone()).unwrap();
    snapshot_fixture(&mut service, &root);

    let storage = Storage::open(&data_dir).unwrap();
    let rec = storage.latest_snapshot().unwrap().unwrap();
    assert_eq!(rec.total_size_u64(), 350);
    assert_eq!(rec.file_count_u64(), 3);
    assert_eq!(rec.dir_count_u64(), 1);
    assert_eq!(rec.skipped_count_u64(), 0);
    assert_eq!(storage.snapshot_entries_count(rec.id).unwrap(), 2);
    // root path stored as the canonical absolute path (verbatim prefix,
    // which `canonicalize` emits on Windows, is stripped by normalize_abs).
    let canonical = std::fs::canonicalize(&root).unwrap();
    assert_eq!(rec.root_path, plain_path(&canonical));
}

#[test]
fn entries_match_the_scan_exactly() {
    let td = TempDir::new().unwrap();
    let data_dir = td.path().join("data");
    let mut service = SnapshotService::new(data_dir.clone()).unwrap();

    // First scan the raw result, then persist it through the service.
    let root = td.path().join("t");
    std::fs::create_dir_all(root.join("d1")).unwrap();
    std::fs::create_dir_all(root.join("d2").join("d2a")).unwrap();
    std::fs::write(root.join("x.txt"), b"hello").unwrap();
    std::fs::write(root.join("d1").join("y.txt"), vec![1u8; 42]).unwrap();

    let opts = ScanOptions {
        root: std::fs::canonicalize(&root).unwrap(),
        exclude: None,
        max_depth: None,
    };
    let res = scan(&opts, &mut |_| {}).unwrap();

    let id = snapshot_fixture(&mut service, &root);
    let storage = Storage::open(&data_dir).unwrap();
    assert_eq!(
        storage.snapshot_entries_count(id).unwrap(),
        res.entries.len() as i64
    );

    // Spot-check one stored row.
    let conn = storage.connection();
    let mut stmt = conn
        .prepare(
            "SELECT path, size, file_count, dir_count FROM entries \
                  WHERE snapshot_id=?1 AND path LIKE '%d1'",
        )
        .unwrap();
    let row = stmt
        .query_row([id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })
        .unwrap();
    assert!(row.0.ends_with("d1"));
    assert_eq!(u64_to_i64(42), row.1);
    assert_eq!(row.2, u64_to_i64(1));
    assert_eq!(row.3, u64_to_i64(0));
}

// ─── transaction atomicity & foreign keys ────────────────────────────────

#[test]
fn aborted_snapshot_leaves_no_half_database() {
    let td = TempDir::new().unwrap();
    let data_dir = td.path().join("data");
    let mut storage = Storage::init(&data_dir).unwrap();

    // Inject a failure *inside* the transaction: any INSERT into `entries`
    // aborts. The snapshot row has already been written at that point, so this
    // is the exact "half-snapshot" risk.
    storage
        .connection()
        .execute_batch(
            "CREATE TRIGGER abort_entries BEFORE INSERT ON entries \
             BEGIN SELECT RAISE(ABORT, 'injected'); END;",
        )
        .unwrap();

    let input = dummy_snapshot(&data_dir, 3);
    let err = storage.save_snapshot(&input).unwrap_err();
    assert!(err.to_string().contains("injected"));

    // Forgive the trigger, then confirm nothing was left behind.
    storage
        .connection()
        .execute_batch("DROP TRIGGER abort_entries")
        .unwrap();

    let mut storage2 = Storage::open(&data_dir).unwrap();
    assert_eq!(
        storage2.count_snapshots().unwrap(),
        0,
        "no snapshot row survived"
    );
    // ...and the database still works.
    let ok = storage2
        .save_snapshot(&dummy_snapshot(&data_dir, 1))
        .unwrap();
    assert_eq!(storage2.latest_snapshot().unwrap().unwrap().id, ok);
}

#[test]
fn deleting_a_snapshot_cascades_to_entries() {
    let td = TempDir::new().unwrap();
    let data_dir = td.path().join("data");
    let mut storage = Storage::init(&data_dir).unwrap();

    let id = storage
        .save_snapshot(&dummy_snapshot(&data_dir, 2))
        .unwrap();
    assert_eq!(storage.snapshot_entries_count(id).unwrap(), 2);

    // DELETE via the FK ON DELETE CASCADE.
    storage
        .connection()
        .execute("DELETE FROM snapshots WHERE id = ?1", [id])
        .unwrap();
    assert_eq!(storage.snapshot_entries_count(id).unwrap(), 0);
    assert_eq!(storage.count_snapshots().unwrap(), 0);
}

#[test]
fn multiple_snapshots_are_independent() {
    let td = TempDir::new().unwrap();
    let data_dir = td.path().join("data");
    let mut service = SnapshotService::new(data_dir.clone()).unwrap();

    let root1 = td.path().join("t1");
    let root2 = td.path().join("t2");
    std::fs::create_dir_all(&root1).unwrap();
    std::fs::create_dir_all(&root2).unwrap();
    std::fs::write(root1.join("a.txt"), vec![9u8; 15]).unwrap();
    std::fs::write(root2.join("b.txt"), vec![9u8; 30]).unwrap();

    let id1 = snapshot_fixture(&mut service, &root1);
    let id2 = snapshot_fixture(&mut service, &root2);
    assert!(id2 > id1);

    let storage = Storage::open(&data_dir).unwrap();
    assert_eq!(storage.count_snapshots().unwrap(), 2);
    let list = storage.list_snapshots().unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].id, id1);
    assert_eq!(list[1].id, id2);
    // latest is the second scan
    assert_eq!(storage.latest_snapshot().unwrap().unwrap().id, id2);
    assert_eq!(
        storage.latest_snapshot().unwrap().unwrap().total_size_u64(),
        30
    );
    // first snapshot's entries untouched
    assert_eq!(storage.snapshot_entries_count(id1).unwrap(), 1);
}

// ─── damage / unwritable databases ────────────────────────────────────────

#[test]
fn corrupt_database_file_yields_a_clear_error() {
    let td = TempDir::new().unwrap();
    let data_dir = td.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    // Write garbage where the DB should be.
    std::fs::write(
        data_dir.join("diskdrift.db"),
        b"this is not a sqlite database",
    )
    .unwrap();

    let err = Storage::open(&data_dir).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.to_lowercase().contains("database") || msg.contains("file is not a database"),
        "unhelpful error: {msg}"
    );
}

#[cfg(unix)]
#[test]
fn unwritable_database_reports_a_clear_open_error() {
    use std::os::unix::fs::PermissionsExt;
    let td = TempDir::new().unwrap();
    let data_dir = td.path().join("data");
    let _ = Storage::init(&data_dir).unwrap();

    let db = data_dir.join("diskdrift.db");
    // mode 000 denies even open(O_RDWR) → deterministic SQLITE_CANTOPEN.
    std::fs::set_permissions(&db, std::fs::Permissions::from_mode(0o000)).unwrap();

    let err = Storage::open(&data_dir).unwrap_err();
    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("open") || msg.contains("permission") || msg.contains("denied"),
        "expected a clear open error, got: {msg}"
    );

    std::fs::set_permissions(&db, std::fs::Permissions::from_mode(0o644)).unwrap();
}

// ─── oversized values ─────────────────────────────────────────────────────

#[test]
fn u64_i64_conversion_is_total_and_saturating() {
    assert_eq!(u64_to_i64(0), 0);
    assert_eq!(u64_to_i64(i64::MAX as u64), i64::MAX);
    // Beyond i64::MAX saturates instead of wrapping/panicking.
    assert_eq!(u64_to_i64(u64::MAX), i64::MAX);
    assert_eq!(i64_to_u64(0), 0);
    assert_eq!(i64_to_u64(-5), 0, "negative clamps to 0");
    assert_eq!(i64_to_u64(i64::MAX), i64::MAX as u64);
}

// ─── status ───────────────────────────────────────────────────────────────

#[test]
fn status_reports_not_initialized_without_creating_database() {
    let td = TempDir::new().unwrap();
    let data_dir = td.path().join("data");
    let report = SnapshotService::status(data_dir.clone()).unwrap();
    assert!(!report.initialized);
    assert_eq!(report.snapshot_count, 0);
    // status must not materialize the database
    assert!(!data_dir.join("diskdrift.db").exists());
}

#[test]
fn status_reports_full_snapshot_metadata() {
    let td = TempDir::new().unwrap();
    let data_dir = td.path().join("data");
    let mut service = SnapshotService::new(data_dir.clone()).unwrap();
    let root = td.path().join("t");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("f.bin"), vec![3u8; 640]).unwrap();

    snapshot_fixture(&mut service, &root);

    let report: StatusReport = SnapshotService::status(data_dir.clone()).unwrap();
    assert!(report.initialized);
    assert_eq!(report.snapshot_count, 1);
    let latest = report.latest.unwrap();
    assert_eq!(latest.total_size_u64(), 640);
    assert_eq!(latest.file_count_u64(), 1);
    assert_eq!(latest.size_kind, "apparent");
    assert!(report.db_size > 0);
    assert!(report.db_path.exists());
}

// ─── root resolution ──────────────────────────────────────────────────────

#[test]
fn resolve_root_handles_absolute_relative_and_current_dir() {
    let td = TempDir::new().unwrap();
    let abs_root = td.path().join("tree");
    std::fs::create_dir_all(&abs_root).unwrap();

    let via_abs = SnapshotService::resolve_root(&abs_root).unwrap();
    let expected = PathBuf::from(plain_path(&std::fs::canonicalize(&abs_root).unwrap()));
    assert_eq!(via_abs, expected);

    // relative path resolved against cwd: the crate root has `src/`
    let rel = SnapshotService::resolve_root(Path::new("src")).unwrap();
    assert!(rel.is_absolute());
    assert!(rel.to_string_lossy().ends_with("src"));

    // current directory
    let dot = SnapshotService::resolve_root(Path::new(".")).unwrap();
    assert!(dot.is_absolute());

    // missing root → clear user error
    let missing = td.path().join("nope");
    let err = SnapshotService::resolve_root(&missing).unwrap_err();
    assert!(err.to_string().contains("cannot scan root"));

    // a file as root → "not a directory"
    let file = td.path().join("plain.txt");
    std::fs::write(&file, b"x").unwrap();
    let err = SnapshotService::resolve_root(&file).unwrap_err();
    assert!(err.to_string().contains("not a directory"));
}

#[test]
fn snapshot_excludes_its_own_data_dir_and_rejects_reverse() {
    let td = TempDir::new().unwrap();
    let root = td.path().join("proj");
    let data_dir = root.join(".diskdrift-data"); // data dir is *inside* the scan root
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("real.txt"), vec![1u8; 50]).unwrap();

    let mut service = SnapshotService::new(data_dir.clone()).unwrap();
    let id = service
        .run_snapshot(&root, &mut |_| {})
        .unwrap()
        .snapshot_id;

    // The data dir lives under the scanned root; its (growing) DB must not be
    // counted, or every snapshot would grow forever.
    let storage = Storage::open(&data_dir).unwrap();
    let rec = storage.latest_snapshot().unwrap().unwrap();
    assert_eq!(rec.id, id);
    assert_eq!(rec.total_size_u64(), 50, "only real.txt is counted");
    assert_eq!(rec.file_count_u64(), 1);

    // Reverse: a root *inside* the data dir is rejected outright.
    let err = service.run_snapshot(&data_dir, &mut |_| {}).unwrap_err();
    assert!(err
        .to_string()
        .contains("inside the DiskDrift data directory"));
}

// ─── helpers ──────────────────────────────────────────────────────────────

/// Mirror of the service's own `normalize_abs`: strip the Windows `\\?\`
/// verbatim prefix so tests compare against the same spelling production uses.
fn plain_path(p: &Path) -> String {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        s.into_owned()
    }
}

fn dummy_snapshot(data_dir: &Path, n_entries: i64) -> NewSnapshot {
    let entries = (0..n_entries)
        .map(|i| diskdrift::storage::models::EntryToWrite {
            path: format!("dir{}", i),
            size: 1_000 + i as u64,
            file_count: 2,
            dir_count: 1,
        })
        .collect();
    NewSnapshot {
        created_at_ms: 1_600_000_000_000,
        root_path: data_dir.to_string_lossy().into_owned(),
        total_size: 1_000_000,
        file_count: 10,
        dir_count: 3,
        scan_duration_ms: 1234,
        skipped_count: 0,
        entries,
    }
}
