//! Connection setup and the [`Storage`] facade.
//!
//! One connection, WAL journal, `synchronous=NORMAL` (crash-safe; not
//! power-loss-safe, documented in DESIGN §5), foreign keys enforced, a busy
//! timeout for the rare concurrent reader, and a migration header when opened.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior};

use crate::config;
use crate::error::Result;

use super::migrations;
use super::models::{snapshot_column_list, NewSnapshot, SnapshotRecord};

/// Handle to the WhyBig database.
#[derive(Debug)]
pub struct Storage {
    conn: Connection,
    data_dir: PathBuf,
}

impl Storage {
    /// Create the data directory (if needed), open the DB and migrate it.
    /// Idempotent — safe to call repeatedly.
    pub fn init(data_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(data_dir)?;
        Self::open(data_dir)
    }

    /// Open (creating if absent) and migrate. Fails with a clear error if the
    /// file exists but is not a SQLite database, or is not writable.
    pub fn open(data_dir: &Path) -> Result<Self> {
        let db_path = config::db_path(data_dir);
        let conn = Connection::open(&db_path)?;
        configure(&conn)?;
        let mut storage = Self {
            conn,
            data_dir: data_dir.to_path_buf(),
        };
        migrations::migrate(&mut storage.conn)?;
        Ok(storage)
    }

    /// Like [`Storage::open`] but refuses to create the database: returns
    /// `None` when the DB file does not exist yet (used by `whybig status` so
    /// that merely inspecting status never materializes storage).
    pub fn open_if_exists(data_dir: &Path) -> Result<Option<Self>> {
        if !config::db_path(data_dir).exists() {
            return Ok(None);
        }
        Ok(Some(Self::open(data_dir)?))
    }

    /// Absolute path of the database file.
    pub fn database_path(&self) -> PathBuf {
        config::db_path(&self.data_dir)
    }

    /// On-disk size of the database file (0 when missing).
    pub fn database_size(&self) -> u64 {
        std::fs::metadata(self.database_path())
            .map(|m| m.len())
            .unwrap_or(0)
    }

    /// True when the database file exists.
    pub fn database_exists(&self) -> bool {
        self.database_path().exists()
    }

    /// Persist one snapshot and all its directory entries atomically: a single
    /// transaction, so a failure rolls back the whole thing and leaves no
    /// half-snapshot behind.
    pub fn save_snapshot(&mut self, input: &NewSnapshot) -> Result<i64> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        tx.execute(
            "INSERT INTO snapshots \
                (created_at, root_path, total_size, file_count, dir_count, \
                 scan_duration_ms, skipped_count, size_kind) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'apparent')",
            rusqlite::params![
                input.created_at_ms,
                input.root_path,
                crate::storage::models::u64_to_i64(input.total_size),
                crate::storage::models::u64_to_i64(input.file_count),
                crate::storage::models::u64_to_i64(input.dir_count),
                crate::storage::models::u64_to_i64(input.scan_duration_ms),
                crate::storage::models::u64_to_i64(input.skipped_count),
            ],
        )?;
        let id = tx.last_insert_rowid();

        {
            let mut stmt = tx.prepare(
                "INSERT INTO entries (snapshot_id, path, size, file_count, dir_count) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for e in &input.entries {
                stmt.execute(rusqlite::params![
                    id,
                    e.path,
                    crate::storage::models::u64_to_i64(e.size),
                    crate::storage::models::u64_to_i64(e.file_count),
                    crate::storage::models::u64_to_i64(e.dir_count),
                ])?;
            }
        }

        tx.commit()?;
        Ok(id)
    }

    /// Number of stored snapshots.
    pub fn count_snapshots(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM snapshots", [], |r| r.get(0))?)
    }

    /// All snapshots, oldest first.
    pub fn list_snapshots(&self) -> Result<Vec<SnapshotRecord>> {
        let sql = format!(
            "SELECT {} FROM snapshots ORDER BY id ASC",
            snapshot_column_list()
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], SnapshotRecord::from_row)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// The first snapshot ever stored (by id). `None` if there are none.
    pub fn earliest_snapshot(&self) -> Result<Option<SnapshotRecord>> {
        let sql = format!(
            "SELECT {} FROM snapshots ORDER BY id ASC LIMIT 1",
            snapshot_column_list()
        );
        Ok(self
            .conn
            .query_row(&sql, [], SnapshotRecord::from_row)
            .optional()?)
    }

    /// The most recently stored snapshot. `None` if there are none.
    pub fn latest_snapshot(&self) -> Result<Option<SnapshotRecord>> {
        let sql = format!(
            "SELECT {} FROM snapshots ORDER BY id DESC LIMIT 1",
            snapshot_column_list()
        );
        Ok(self
            .conn
            .query_row(&sql, [], SnapshotRecord::from_row)
            .optional()?)
    }

    /// Number of directory entries recorded for one snapshot.
    pub fn snapshot_entries_count(&self, snapshot_id: i64) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM entries WHERE snapshot_id = ?1",
            [snapshot_id],
            |r| r.get(0),
        )?)
    }

    /// Access to the raw connection — used by integration tests to inject
    /// failures (e.g. a trigger that aborts an insert).
    pub fn connection(&self) -> &Connection {
        &self.conn
    }
}

fn configure(conn: &Connection) -> Result<()> {
    conn.busy_timeout(Duration::from_millis(5000))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    Ok(())
}
