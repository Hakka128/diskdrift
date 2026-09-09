//! Orchestration: normalize the root → scan (with progress) → transactional
//! persistence → a user-facing outcome. The service is what the CLI calls; it
//! knows neither clap nor terminal rendering.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config;
use crate::error::{DiskDriftError, Result};
use crate::pathutil::{canonicalize_scope_path, is_under, normalize_abs};
use crate::scanner::{self, ScanOptions, ScanResult};
use crate::storage::models::{EntryToWrite, NewSnapshot, SnapshotRecord};
use crate::storage::Storage;

/// Everything the CLI needs to render a completed snapshot.
#[derive(Debug, Clone)]
pub struct SnapshotOutcome {
    pub snapshot_id: i64,
    /// Unix epoch ms (UTC) of the snapshot row.
    pub created_at_ms: i64,
    pub root_path: PathBuf,
    pub total_size: u64,
    pub file_count: u64,
    pub dir_count: u64,
    pub skipped_count: u64,
    pub elapsed: Duration,
    /// Non-fatal scan problems (permission/vanished/IO), carried for `--json`.
    pub warnings: Vec<crate::scanner::ScanWarning>,
}

/// What `diskdrift status` reports.
#[derive(Debug, Clone)]
pub struct StatusReport {
    pub initialized: bool,
    pub data_dir: PathBuf,
    pub db_path: PathBuf,
    pub db_size: u64,
    pub snapshot_count: i64,
    pub earliest: Option<SnapshotRecord>,
    pub latest: Option<SnapshotRecord>,
}

/// Long-lived entry point for the `snapshot`/`init` commands.
pub struct SnapshotService {
    data_dir: PathBuf,
    storage: Storage,
}

impl SnapshotService {
    /// Open (and if needed create + migrate) the storage. Idempotent init.
    pub fn new(data_dir: PathBuf) -> Result<Self> {
        let storage = Storage::init(&data_dir)?;
        Ok(Self { data_dir, storage })
    }

    /// Resolve a user-supplied root to a normalized absolute path:
    /// joins the cwd for relative paths, then canonicalizes (the root's own
    /// symlink is followed exactly once so snapshot keys stay stable).
    pub fn resolve_root(raw: &Path) -> Result<PathBuf> {
        let abs = if raw.is_absolute() {
            raw.to_path_buf()
        } else {
            std::env::current_dir()?.join(raw)
        };
        let canon = std::fs::canonicalize(&abs).map_err(|e| DiskDriftError::RootUnavailable {
            path: abs.clone(),
            reason: e,
        })?;
        let canon = normalize_abs(canon);
        if !canon.is_dir() {
            return Err(DiskDriftError::RootNotADirectory(canon));
        }
        Ok(canon)
    }

    /// Record a snapshot of `raw_root`.
    pub fn run_snapshot(
        &mut self,
        raw_root: &Path,
        on_progress: &mut dyn FnMut(u64),
    ) -> Result<SnapshotOutcome> {
        let root = Self::resolve_root(raw_root)?;
        self.check_root_not_in_data_dir(&root)?;

        // If the data dir lies inside the scan root, exclude its subtree so a
        // scan never records its own (growing) database (`DESIGN.md` §13).
        let data_dir_abs = absolutize(&self.data_dir);
        let exclude = if is_under(&data_dir_abs, &root) {
            Some(data_dir_abs)
        } else {
            None
        };

        let options = ScanOptions {
            root: root.clone(),
            exclude,
            max_depth: None,
        };
        let result =
            scanner::scan(&options, on_progress).map_err(|e| DiskDriftError::RootUnavailable {
                path: root.clone(),
                reason: e,
            })?;

        let created_at_ms = chrono::Utc::now().timestamp_millis();
        let snapshot_id = self.store(&result, &root, created_at_ms)?;

        Ok(SnapshotOutcome {
            snapshot_id,
            created_at_ms,
            root_path: root,
            total_size: result.summary.total_size,
            file_count: result.summary.file_count,
            dir_count: result.summary.dir_count,
            skipped_count: result.summary.skipped_count,
            elapsed: result.summary.elapsed,
            warnings: result.warnings,
        })
    }

    /// Reject scanning a root that lives inside the DiskDrift data directory.
    fn check_root_not_in_data_dir(&self, root: &Path) -> Result<()> {
        let data_dir_abs = absolutize(&self.data_dir);
        if is_under(root, &data_dir_abs) {
            return Err(DiskDriftError::RootInsideDataDir(root.to_path_buf()));
        }
        Ok(())
    }

    /// Persist a scan result (path lossy-UTF8 keys, sizes saturated to i64).
    fn store(&mut self, result: &ScanResult, root: &Path, created_at_ms: i64) -> Result<i64> {
        let entries = result
            .entries
            .iter()
            .map(|e| EntryToWrite {
                path: e.path.to_string_lossy().into_owned(),
                size: e.agg.total_size,
                file_count: e.agg.file_count,
                dir_count: e.agg.dir_count,
            })
            .collect::<Vec<_>>();

        let input = NewSnapshot {
            created_at_ms,
            root_path: root.to_string_lossy().into_owned(),
            total_size: result.summary.total_size,
            file_count: result.summary.file_count,
            dir_count: result.summary.dir_count,
            scan_duration_ms: result.summary.elapsed.as_millis() as u64,
            skipped_count: result.summary.skipped_count,
            entries,
        };
        self.storage.save_snapshot(&input)
    }

    /// Build a status report without materializing storage (no DB → not
    /// initialized yet, not an error).
    pub fn status(data_dir: PathBuf) -> Result<StatusReport> {
        let db_path = config::db_path(&data_dir);
        let Some(storage) = Storage::open_if_exists(&data_dir)? else {
            return Ok(StatusReport {
                initialized: false,
                data_dir,
                db_path,
                db_size: 0,
                snapshot_count: 0,
                earliest: None,
                latest: None,
            });
        };
        Ok(StatusReport {
            initialized: true,
            data_dir,
            db_size: storage.database_size(),
            db_path: storage.database_path(),
            snapshot_count: storage.count_snapshots()?,
            earliest: storage.earliest_snapshot()?,
            latest: storage.latest_snapshot()?,
        })
    }
}

/// Make a path absolute using the process cwd and normalize its spelling to
/// the same stored-canonical form the scan root uses (Windows 8.3 short names
/// resolve to long names; macOS `/var` resolves to `/private/var`). This keeps
/// the data-dir self-exclusion and the reverse "root inside data dir"
/// rejection consistent when the data dir was supplied through an alias
/// spelling while the scan root is canonicalized. When nothing exists to
/// canonicalize, falls back to the plain absolute spelling (unchanged).
fn absolutize(p: &Path) -> PathBuf {
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|c| c.join(p))
            .unwrap_or_else(|_| p.to_path_buf())
    };
    let abs = normalize_abs(abs);
    canonicalize_scope_path(&abs).unwrap_or(abs)
}
