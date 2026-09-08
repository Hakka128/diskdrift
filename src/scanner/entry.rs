//! The persifiable unit of a snapshot: one directory and its aggregate.

use std::path::PathBuf;

use super::aggregate::DirAgg;

/// A single directory record, ready to be stored (or later diffed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    /// Absolute path of the directory (interior spelling normalized by the root).
    pub path: PathBuf,
    /// Recursive aggregate of everything below it.
    pub agg: DirAgg,
}
