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
use crate::pathutil::is_direct_child;

use super::migrations;
use super::models::{snapshot_column_list, EntryRecord, NewSnapshot, SnapshotRecord};

/// Handle to the DiskDrift database.
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
    /// `None` when the DB file does not exist yet (used by `diskdrift status` so
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

    /// Of one root's snapshots with `created_at <= time_ms`, the one closest to
    /// `time_ms` (created_at DESC; ties broken by smallest id for determinism).
    /// Used by `--since`.
    pub fn get_snapshot_at_or_before(
        &self,
        root: &str,
        time_ms: i64,
    ) -> Result<Option<SnapshotRecord>> {
        let sql = format!(
            "SELECT {} FROM snapshots \
             WHERE root_path = ?1 AND created_at <= ?2 \
             ORDER BY created_at DESC, id ASC LIMIT 1",
            snapshot_column_list()
        );
        Ok(self
            .conn
            .query_row(
                &sql,
                rusqlite::params![root, time_ms],
                SnapshotRecord::from_row,
            )
            .optional()?)
    }

    /// The first snapshot of one root (by id).
    pub fn earliest_snapshot_for_root(&self, root: &str) -> Result<Option<SnapshotRecord>> {
        let sql = format!(
            "SELECT {} FROM snapshots WHERE root_path = ?1 ORDER BY id ASC LIMIT 1",
            snapshot_column_list()
        );
        Ok(self
            .conn
            .query_row(&sql, [root], SnapshotRecord::from_row)
            .optional()?)
    }

    /// One snapshot by id.
    pub fn get_snapshot(&self, id: i64) -> Result<Option<SnapshotRecord>> {
        let sql = format!(
            "SELECT {} FROM snapshots WHERE id = ?1",
            snapshot_column_list()
        );
        Ok(self
            .conn
            .query_row(&sql, [id], SnapshotRecord::from_row)
            .optional()?)
    }

    /// Snapshots of one tracked root, newest first. The `root` must match the
    /// stored `root_path` exactly (it is the scanner-normalized key).
    pub fn get_latest_snapshots_for_root(
        &self,
        root: &str,
        limit: i64,
    ) -> Result<Vec<SnapshotRecord>> {
        let sql = format!(
            "SELECT {} FROM snapshots WHERE root_path = ?1 ORDER BY id DESC LIMIT ?2",
            snapshot_column_list()
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params![root, limit], SnapshotRecord::from_row)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Time series of one path across a root's snapshots.
    ///
    /// A single query (no N+1 loop): `snapshots` LEFT JOIN `entries`, so a
    /// snapshot that does not contain the path still yields a point with
    /// `size = 0` instead of dropping the timestamp. Returns
    /// `(snapshot_id, created_at_ms, size)` **ascending** by snapshot id (which
    /// is monotonic with time).
    pub fn get_history_series(
        &self,
        root: &str,
        path: &str,
        limit: i64,
    ) -> Result<Vec<(i64, i64, u64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.id, s.created_at, COALESCE(e.size, 0)
             FROM snapshots s
             LEFT JOIN entries e ON e.snapshot_id = s.id AND e.path = ?2
             WHERE s.root_path = ?1
             ORDER BY s.id DESC LIMIT ?3",
        )?;
        let rows = stmt.query_map(rusqlite::params![root, path, limit], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                crate::storage::models::i64_to_u64(r.get::<_, i64>(2)?),
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        // We fetched newest-first to apply LIMIT; return ascending for the trend.
        out.reverse();
        Ok(out)
    }

    /// One directory entry by path (PK lookup). `None` when the directory was
    /// not present in that snapshot (added after / removed before).
    pub fn get_entry(&self, snapshot_id: i64, path: &str) -> Result<Option<EntryRecord>> {
        Ok(self
            .conn
            .query_row(
                "SELECT path, size, file_count, dir_count FROM entries \
                 WHERE snapshot_id = ?1 AND path = ?2",
                rusqlite::params![snapshot_id, path],
                EntryRecord::from_row,
            )
            .optional()?)
    }

    /// The **direct** children (exactly one level below `path`) of a snapshot.
    ///
    /// SQL `LIKE` narrows the scan to the subtree (escaped so `%`/`_`/`\` in
    /// names cannot leak), and the component-wise [`is_direct_child`] filter
    /// guarantees boundary correctness (`/a/bar` vs `/a/bar2`).
    pub fn get_direct_children(&self, snapshot_id: i64, path: &str) -> Result<Vec<EntryRecord>> {
        let pattern = subtree_like_pattern(path);
        let mut stmt = self.conn.prepare(
            "SELECT path, size, file_count, dir_count FROM entries \
             WHERE snapshot_id = ?1 AND path LIKE ?2 ESCAPE '\\'",
        )?;
        let parent = Path::new(path);
        let rows = stmt.query_map(
            rusqlite::params![snapshot_id, pattern],
            EntryRecord::from_row,
        )?;
        let mut out = Vec::new();
        for row in rows {
            let entry = row?;
            // Defensive boundary check on top of the LIKE narrow.
            if is_direct_child(Path::new(&entry.path), parent) {
                out.push(entry);
            }
        }
        Ok(out)
    }

    /// Look up an entry treating the path as case-insensitive (NTFS folders are
    /// case-insensitive). Exact match first; on Windows a `COLLATE NOCASE`
    /// fallback covers ASCII case (drives, typical folders). If both miss,
    /// returns `None` — the directory is not present in that snapshot.
    pub fn get_entry_case_insensitive(
        &self,
        snapshot_id: i64,
        path: &str,
    ) -> Result<Option<EntryRecord>> {
        if self.get_entry(snapshot_id, path)?.is_some() {
            return self.get_entry(snapshot_id, path);
        }
        #[cfg(windows)]
        {
            let mut stmt = self.conn.prepare(
                "SELECT path, size, file_count, dir_count FROM entries \
                 WHERE snapshot_id = ?1 AND path = ?2 COLLATE NOCASE LIMIT 1",
            )?;
            let row = stmt
                .query_row(rusqlite::params![snapshot_id, path], EntryRecord::from_row)
                .optional()?;
            Ok(row)
        }
        #[cfg(not(windows))]
        {
            Ok(None)
        }
    }

    /// Number of directory entries recorded for one snapshot.
    pub fn snapshot_entries_count(&self, snapshot_id: i64) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM entries WHERE snapshot_id = ?1",
            [snapshot_id],
            |r| r.get(0),
        )?)
    }

    /// Delete snapshots (entries cascade via FK). Single transaction: any
    /// failure rolls back. Used by `diskdrift prune --apply`.
    pub fn delete_snapshots(&mut self, ids: &[i64]) -> Result<i64> {
        if ids.is_empty() {
            return Ok(0);
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut deleted: i64 = 0;
        for id in ids {
            deleted += tx.execute("DELETE FROM snapshots WHERE id = ?1", [*id])? as i64;
        }
        tx.commit()?;
        Ok(deleted)
    }

    /// Access to the raw connection — used by integration tests to inject
    /// failures (e.g. a trigger that aborts an insert).
    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    /// Compact the database file with `VACUUM` (removes free pages created by
    /// deletes; needs roughly one copy of the DB as temporary space).
    pub fn vacuum(&mut self) -> Result<()> {
        self.conn.execute_batch("VACUUM")?;
        Ok(())
    }
}

fn configure(conn: &Connection) -> Result<()> {
    conn.busy_timeout(Duration::from_millis(5000))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    Ok(())
}

/// Escape a string for use inside a `LIKE ... ESCAPE '\'` pattern.
fn like_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for c in s.chars() {
        if matches!(c, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Build the `LIKE ... ESCAPE '\'` pattern that narrows a query to `parent`
/// and nothing shallower. Every stored child key is `parent` joined with the
/// native separator and one name, so the pattern is `parent + sep + "%"` —
/// except when `parent` already ends with the separator (filesystem roots like
/// `/` or `C:\`), where the separator must not be repeated (`//%` matches
/// nothing).
fn subtree_like_pattern(parent: &str) -> String {
    let parent_esc = like_escape(parent);
    let sep = std::path::MAIN_SEPARATOR_STR;
    let boundary = if parent.ends_with(sep) {
        String::new()
    } else {
        like_escape(sep)
    };
    format!("{parent_esc}{boundary}%")
}

#[cfg(test)]
mod pattern_tests {
    use super::{like_escape, subtree_like_pattern};

    #[test]
    fn escapes_metacharacters_only() {
        assert_eq!(like_escape("a/b"), "a/b");
        assert_eq!(like_escape("a%_\\b"), "a\\%\\_\\\\b");
    }

    #[test]
    fn normal_parent_has_boundary_separator() {
        let pat = subtree_like_pattern("dir");
        assert!(pat.ends_with('%'));
        let literal = pat.trim_end_matches('%');
        let sep_pair = like_escape(std::path::MAIN_SEPARATOR_STR);
        assert!(
            literal.ends_with(&sep_pair),
            "pattern {pat} must bound the parent with the separator"
        );
    }

    #[test]
    fn root_parent_does_not_duplicate_separator() {
        // "/" ends with the platform separator on Unix → no boundary appended.
        #[cfg(not(windows))]
        assert_eq!(subtree_like_pattern("/"), "/%");
        // Windows drive roots likewise end with '\' → no duplicate separator.
        #[cfg(windows)]
        assert!(subtree_like_pattern(r"C:\").ends_with('%'));
    }
}
