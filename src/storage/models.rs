//! Storage-side record shapes.
//!
//! SQLite stores `INTEGER` as `i64`; filesystem sizes are `u64`. Conversion is
//! explicit and total (saturating at `i64` bounds — physically unreachable for
//! real filesystems, but never a crash / silent wrap).

use rusqlite::Row;

/// `u64` → SQLite `i64`, saturating at `i64::MAX` (8 EiB).
pub fn u64_to_i64(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

/// SQLite `i64` → `u64`, clamping negative (should never happen for sizes).
pub fn i64_to_u64(v: i64) -> u64 {
    v.max(0) as u64
}

/// A row from the `snapshots` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotRecord {
    pub id: i64,
    /// Unix epoch milliseconds (UTC).
    pub created_at_ms: i64,
    /// The normalized (canonical) scan root, lossy UTF-8.
    pub root_path: String,
    /// Stored in i64; use the `*_u64` getters for the original u64 values.
    pub total_size: i64,
    pub file_count: i64,
    pub dir_count: i64,
    pub scan_duration_ms: i64,
    pub skipped_count: i64,
    /// "apparent" in Milestone 1 ("allocated" reserved).
    pub size_kind: String,
}

/// Column order is shared by every SELECT in `database.rs` — keep in sync!
pub fn snapshot_column_list() -> &'static str {
    "id, created_at, root_path, total_size, file_count, dir_count, \
     scan_duration_ms, skipped_count, size_kind"
}

impl SnapshotRecord {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            created_at_ms: row.get(1)?,
            root_path: row.get(2)?,
            total_size: row.get(3)?,
            file_count: row.get(4)?,
            dir_count: row.get(5)?,
            scan_duration_ms: row.get(6)?,
            skipped_count: row.get(7)?,
            size_kind: row.get(8)?,
        })
    }

    pub fn total_size_u64(&self) -> u64 {
        i64_to_u64(self.total_size)
    }
    pub fn file_count_u64(&self) -> u64 {
        i64_to_u64(self.file_count)
    }
    pub fn dir_count_u64(&self) -> u64 {
        i64_to_u64(self.dir_count)
    }
    pub fn scan_duration_ms_u64(&self) -> u64 {
        i64_to_u64(self.scan_duration_ms)
    }
    pub fn skipped_count_u64(&self) -> u64 {
        i64_to_u64(self.skipped_count)
    }
}

/// One `entries` row to insert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryToWrite {
    pub path: String,
    pub size: u64,
    pub file_count: u64,
    pub dir_count: u64,
}

/// Full payload for `Storage::save_snapshot`.
#[derive(Debug, Clone)]
pub struct NewSnapshot {
    pub created_at_ms: i64,
    pub root_path: String,
    pub total_size: u64,
    pub file_count: u64,
    pub dir_count: u64,
    pub scan_duration_ms: u64,
    pub skipped_count: u64,
    pub entries: Vec<EntryToWrite>,
}
