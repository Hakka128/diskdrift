//! Directory-level aggregation counters.
//!
//! A [`DirAgg`] is the recursive total for one directory: everything below it,
//! summed. DiskDrift persists only these aggregates, never per-file records, so a
//! snapshot of a million files stays proportional to the *directory* count.

/// Recursive totals for one directory.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DirAgg {
    /// Sum of apparent sizes (bytes) of every accounted file below this dir.
    pub total_size: u64,
    /// Number of accounted files below this dir (regular + special files).
    pub file_count: u64,
    /// Number of directories below this dir (recursive; excludes the dir itself,
    /// includes policy-excluded child dirs that were counted but not descended).
    pub dir_count: u64,
    /// Entries that could not be accounted for (permission denied / vanished /
    /// I/O errors) — these are reported as warnings, never as failures.
    pub skipped: u64,
}

impl DirAgg {
    /// Fold a finished child directory into this (parent) aggregate.
    ///
    /// The `+1` accounts for the child directory itself.
    pub fn merge_child(&mut self, child: &DirAgg) {
        self.total_size = self.total_size.saturating_add(child.total_size);
        self.file_count = self.file_count.saturating_add(child.file_count);
        self.dir_count = self
            .dir_count
            .saturating_add(child.dir_count.saturating_add(1));
        self.skipped = self.skipped.saturating_add(child.skipped);
    }

    /// Count one file with its apparent size.
    pub fn add_file(&mut self, size: u64) {
        self.file_count = self.file_count.saturating_add(1);
        self.total_size = self.total_size.saturating_add(size);
    }

    /// Count one directory that exists below us but is not descended into
    /// (policy exclusion: own data dir / cross-filesystem / depth cap).
    pub fn add_dir_only(&mut self) {
        self.dir_count = self.dir_count.saturating_add(1);
    }

    /// Count one entry that could not be accounted for.
    pub fn add_skipped(&mut self) {
        self.skipped = self.skipped.saturating_add(1);
    }
}
