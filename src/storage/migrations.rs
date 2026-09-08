//! Schema migrations, tracked with `PRAGMA user_version`.
//!
//! Each migration is a plain SQL script applied inside its own transaction and
//! advances `user_version` by one. `init` and every open run this, so a stale
//! database is upgraded transparently and an absent one is created empty.

use rusqlite::Connection;

use crate::error::{DiskDriftError, Result};

/// Migrations, indexed by version-1. Never edit a released script — append a
/// new one instead.
const MIGRATIONS: &[&str] = &[
    // v1 — initial schema (see DESIGN.md §5 for rationale of the two extra
    // columns vs. the suggested schema: `skipped_count` is required by
    // `diskdrift status`, `size_kind` reserves the allocated-size dimension).
    "\
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
    ",
    // v2 — a `meta` key/value table: general purpose (store app metadata) and
    // the vehicle for real migration tests (v1 → v2 upgrades preserve data).
    "\
    CREATE TABLE meta (\
        key TEXT PRIMARY KEY,\
        value TEXT NOT NULL\
    );\
    ",
];

/// Latest schema version this binary understands.
pub fn latest_version() -> i64 {
    MIGRATIONS.len() as i64
}

/// Bring `conn` up to the latest schema version. Idempotent; refuses databases
/// from a NEWER DiskDrift.
pub fn migrate(conn: &mut Connection) -> Result<()> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;

    if current > latest_version() {
        return Err(DiskDriftError::NewerSchema);
    }

    for (idx, script) in MIGRATIONS.iter().enumerate() {
        let target = (idx + 1) as i64;
        if target <= current {
            continue;
        }
        let mut apply = || -> rusqlite::Result<()> {
            let tx = conn.transaction()?;
            tx.execute_batch(script)?;
            tx.pragma_update(None, "user_version", target)?;
            tx.commit()?;
            Ok(())
        };
        apply().map_err(|e| DiskDriftError::Migration(format!("v{target}: {e}")))?;
    }
    Ok(())
}

/// Current `PRAGMA user_version`.
pub fn schema_version(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("PRAGMA user_version", [], |r| r.get(0))?)
}
