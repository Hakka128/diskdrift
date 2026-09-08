//! Domain errors.
//!
//! Every error that can reach the user writes a complete, human-readable
//! message in its `Display` impl; the CLI layer never dumps internal
//! backtraces at the user. `anyhow` is reserved for the binary entry point,
//! which does not need to add context on top of these messages.

use std::path::PathBuf;

use thiserror::Error;

/// The result type used across the library.
pub type Result<T> = std::result::Result<T, DiskDriftError>;

/// All user-visible failures.
#[derive(Debug, Error)]
pub enum DiskDriftError {
    /// Cannot determine (or construct) the DiskDrift data directory.
    #[error("cannot resolve the DiskDrift data directory: {0}")]
    DataDir(String),

    /// The snapshot root cannot be used (missing / not a dir / vanished).
    #[error("cannot scan root `{path}`: {reason}")]
    RootUnavailable {
        path: PathBuf,
        #[source]
        reason: std::io::Error,
    },

    #[error("cannot scan root `{0}`: not a directory")]
    RootNotADirectory(PathBuf),

    /// The chosen root is inside the DiskDrift data directory itself.
    #[error(
        "root `{0}` lies inside the DiskDrift data directory; scanning it would \
         record the database while it grows. Choose a different root"
    )]
    RootInsideDataDir(PathBuf),

    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("database migration failed: {0}")]
    Migration(String),

    #[error("no snapshots yet — run `diskdrift snapshot <path>` first")]
    NoSnapshots,

    #[error("DiskDrift is not initialized here. Run `diskdrift init` first")]
    NotInitialized,

    #[error("snapshot {0} not found")]
    SnapshotNotFound(i64),

    #[error("need at least two snapshots of root `{root}` to compare (only {count} available)")]
    NotEnoughSnapshots { root: String, count: u64 },

    #[error("Cannot compare snapshots from different roots.")]
    CrossRootDiff,

    #[error("path `{0}` is not a directory in either snapshot")]
    PathNotInSnapshots(String),

    #[error("path `{path}` is outside tracked root `{root}`")]
    PathOutsideRoot { path: String, root: String },

    #[error("json serialization failed: {0}")]
    Json(String),

    #[error("invalid `--since` duration `{0}` (expected e.g. 30m, 24h, 7d, 4w; whole units only)")]
    BadDuration(String),

    #[error("no snapshot of root `{root}` earlier than {requested} — record more snapshots first")]
    NoEarlierSnapshot { root: String, requested: String },

    #[error("invalid retention policy: {0}")]
    BadPolicy(String),

    #[error("refusing to prune: {0}")]
    PruneSafety(String),

    #[error("Database was created by a newer version of DiskDrift. Please update DiskDrift.")]
    NewerSchema,

    #[error("unexpected database content: {0}")]
    CorruptData(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
