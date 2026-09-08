//! Migration hardening tests: real v1→v2 upgrade, rerun safety, rollback on
//! failure, and rejection of newer-schema databases.

use std::path::PathBuf;

use diskdrift::config;
use diskdrift::storage::migrations::schema_version;
use diskdrift::storage::Storage;
use rusqlite::Connection;
use tempfile::TempDir;

const V1_SQL: &str = "\
    CREATE TABLE snapshots (\
        id INTEGER PRIMARY KEY,\
        created_at INTEGER NOT NULL,\
        root_path TEXT NOT NULL,\
        total_size INTEGER NOT NULL,\
        file_count INTEGER NOT NULL,\
        dir_count INTEGER NOT NULL,\
        scan_duration_ms INTEGER NOT NULL,\
        skipped_count INTEGER NOT NULL DEFAULT 0,\
        size_kind TEXT NOT NULL DEFAULT 'apparent'\
    );\
    CREATE TABLE entries (\
        snapshot_id INTEGER NOT NULL,\
        path TEXT NOT NULL,\
        size INTEGER NOT NULL,\
        file_count INTEGER NOT NULL,\
        dir_count INTEGER NOT NULL,\
        PRIMARY KEY (snapshot_id, path),\
        FOREIGN KEY (snapshot_id) REFERENCES snapshots(id) ON DELETE CASCADE\
    );\
    CREATE INDEX idx_entries_path ON entries(path);\
";

fn db_path(data_dir: &std::path::Path) -> PathBuf {
    config::db_path(data_dir)
}

fn create_v1(data_dir: &std::path::Path) {
    let conn = Connection::open(db_path(data_dir)).unwrap();
    conn.execute_batch(V1_SQL).unwrap();
    conn.execute_batch(
        "INSERT INTO snapshots \
         (id, created_at, root_path, total_size, file_count, dir_count, scan_duration_ms, skipped_count, size_kind) \
         VALUES (1, 1700000000000, 'R', 4242, 7, 2, 88, 0, 'apparent')",
    )
    .unwrap();
    conn.execute_batch(
        "INSERT INTO entries (snapshot_id, path, size, file_count, dir_count) \
         VALUES (1, 'R/a', 4242, 7, 0)",
    )
    .unwrap();
    conn.pragma_update(None, "user_version", 1).unwrap();
    drop(conn);
}

#[test]
fn old_v1_database_upgrades_and_preserves_data() {
    let td = TempDir::new().unwrap();
    let data_dir = td.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    create_v1(&data_dir);

    // Opening via the current binary must upgrade v1 → v2.
    let storage = Storage::open(&data_dir).unwrap();
    assert_eq!(schema_version(storage.connection()).unwrap(), 2);

    // The meta table (v2) exists.
    let tables: Vec<String> = storage
        .connection()
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='meta'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(tables, vec!["meta".to_string()]);

    // Existing data still readable.
    let rec = storage.latest_snapshot().unwrap().unwrap();
    assert_eq!(rec.root_path, "R");
    assert_eq!(rec.total_size_u64(), 4242);
    assert_eq!(storage.snapshot_entries_count(1).unwrap(), 1);
    let entry = storage.get_entry(1, "R/a").unwrap().unwrap();
    assert_eq!(entry.size, 4242);
}

#[test]
fn migration_rerun_is_safe() {
    let td = TempDir::new().unwrap();
    let data_dir = td.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    create_v1(&data_dir);

    let _ = Storage::open(&data_dir).unwrap();
    let _ = Storage::open(&data_dir).unwrap(); // second open, idempotent
    let storage = Storage::open(&data_dir).unwrap();
    assert_eq!(schema_version(storage.connection()).unwrap(), 2);
    assert_eq!(storage.count_snapshots().unwrap(), 1);
}

#[test]
fn migration_failure_leaves_original_database_untouched() {
    // A v0 database that already contains a `snapshots` table with the WRONG
    // shape: v1's CREATE TABLE snapshots will fail → migration aborts and
    // user_version must remain unchanged (each migration is one transaction).
    let td = TempDir::new().unwrap();
    let data_dir = td.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    {
        let conn = Connection::open(db_path(&data_dir)).unwrap();
        conn.execute_batch("CREATE TABLE snapshots (wrong INTEGER);")
            .unwrap();
        conn.pragma_update(None, "user_version", 0).unwrap();
    }

    let err = Storage::open(&data_dir).unwrap_err();
    assert!(err.to_string().contains("migration"), "err: {err}");

    // user_version was not advanced, table untouched.
    let conn = Connection::open(db_path(&data_dir)).unwrap();
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 0);
}

#[test]
fn newer_schema_is_rejected_without_modification() {
    let td = TempDir::new().unwrap();
    let data_dir = td.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    {
        let conn = Connection::open(db_path(&data_dir)).unwrap();
        conn.pragma_update(None, "user_version", 999).unwrap();
    }

    let err = Storage::open(&data_dir).unwrap_err();
    assert!(
        err.to_string().contains("newer version of DiskDrift"),
        "err: {err}"
    );

    // The database was not modified (version stays 999).
    let conn = Connection::open(db_path(&data_dir)).unwrap();
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 999);
}

#[test]
fn fresh_database_reaches_latest_version() {
    let td = TempDir::new().unwrap();
    let data_dir = td.path().join("data");
    let storage = Storage::init(&data_dir).unwrap();
    assert_eq!(schema_version(storage.connection()).unwrap(), 2);
}
