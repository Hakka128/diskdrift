//! Filesystem scanning.
//!
//! This module produces **pure data** — it never touches SQLite (see
//! `DESIGN.md` §1). [`scan`] walks a tree and returns directory-level
//! aggregates (`ScanResult`) plus a summary and warnings.

pub mod aggregate;
pub mod entry;
pub mod walker;

use std::io;
use std::path::PathBuf;
use std::time::Duration;

pub use aggregate::DirAgg;
pub use entry::DirEntry;

/// Input to a scan. `root` should be an absolute, stable spelling of the
/// directory (the snapshot service normalizes it) so that snapshot keys line
/// up across runs.
#[derive(Debug, Clone)]
pub struct ScanOptions {
    /// The directory to scan.
    pub root: PathBuf,
    /// A subtree to ignore entirely (DiskDrift's own data directory).
    pub exclude: Option<PathBuf>,
    /// Maximum directory depth to descend into; `None` means unlimited.
    pub max_depth: Option<usize>,
}

/// Top-level totals of a scan (they always equal the root entry's aggregate).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanSummary {
    pub total_size: u64,
    pub file_count: u64,
    pub dir_count: u64,
    pub skipped_count: u64,
    pub elapsed: Duration,
}

/// The category of a non-fatal scan problem (for future analysis/reporting).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarningKind {
    /// `metadata` on an entry failed — it vanished mid-scan.
    StatFailed,
    /// A directory could not be read (permission denied / vanished).
    UnreadableDir,
    /// The directory iterator itself reported an error.
    EntryReadFailed,
}

/// One non-fatal problem encountered during a scan.
#[derive(Debug, Clone)]
pub struct ScanWarning {
    pub path: PathBuf,
    pub error: String,
    pub kind: WarningKind,
}

/// Final output of a scan: every accounted directory + summary. There are no
/// per-file records here by design.
#[derive(Debug, Clone)]
pub struct ScanResult {
    pub root: PathBuf,
    pub entries: Vec<DirEntry>,
    pub summary: ScanSummary,
    pub warnings: Vec<ScanWarning>,
}

/// Scan `opts.root`, reporting visiting progress through `on_progress`.
///
/// Errors affecting the **root itself** are returned as `Err`; every other
/// problem is folded into `summary.skipped_count` / `warnings`.
pub fn scan(opts: &ScanOptions, on_progress: &mut dyn FnMut(u64)) -> io::Result<ScanResult> {
    let walk_opts = walker::WalkOptions {
        root: &opts.root,
        exclude: opts.exclude.as_deref(),
        max_depth: opts.max_depth,
    };
    walker::run(&walk_opts, on_progress)
}
