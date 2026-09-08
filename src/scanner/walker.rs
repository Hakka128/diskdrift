//! Explicit-stack depth-first walk over a directory tree.
//!
//! Design points (see `DESIGN.md`):
//! - Iterative stack (not OS recursion) so paths thousands of levels deep
//!   cannot overflow the call stack.
//! - `symlink_metadata`-equivalent semantics per entry: symlinks are never
//!   followed, so symlink loops cannot recurse forever.
//! - Best-effort accounting: permission-denied / vanished entries become
//!   `skipped` counters plus warnings — a single bad entry never fails the run.
//! - Default does not cross filesystem/mount boundaries on Unix (`dev()`);
//!   see `DESIGN.md` §6 for the Windows limitation.

use std::fs::{self, ReadDir};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::pathutil::is_under;

use super::aggregate::DirAgg;
use super::entry::DirEntry;
use super::{ScanResult, ScanSummary, ScanWarning, WarningKind};

/// Configuration for one walk. The root must already be absolute & canonical
/// (or at least a stable absolute spelling); the caller is responsible for
/// normalization so that snapshot keys stay consistent across runs.
pub(crate) struct WalkOptions<'a> {
    pub root: &'a Path,
    /// Subtree to ignore entirely (DiskDrift's own data directory).
    pub exclude: Option<&'a Path>,
    /// Maximum directory depth to descend into (`None` = unlimited).
    pub max_depth: Option<usize>,
}

/// One active directory being visited. Holds an owned [`ReadDir`] so no
/// self-referential borrows are needed.
struct Frame {
    readdir: ReadDir,
    path: PathBuf,
    agg: DirAgg,
    depth: usize,
}

/// The device id of a metadata, where the platform can provide one.
#[cfg(unix)]
fn unix_dev(md: &fs::Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    Some(md.dev())
}

/// On non-Unix platforms we cannot cheaply obtain a stable device id from
/// `std`, so filesystem-boundary detection is disabled (recorded limitation).
#[cfg(not(unix))]
fn unix_dev(_md: &fs::Metadata) -> Option<u64> {
    None
}

/// Walk `opts.root` and produce the aggregate result.
/// Errors from the *root itself* (missing / not a directory / unreadable) are
/// returned to the caller; all nested problems are folded into `skipped`.
pub(crate) fn run(
    opts: &WalkOptions<'_>,
    on_progress: &mut dyn FnMut(u64),
) -> io::Result<ScanResult> {
    let started = Instant::now();

    let root_md = fs::symlink_metadata(opts.root)?;
    if !root_md.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            "root is not a directory",
        ));
    }
    let root_dev = unix_dev(&root_md);

    let root_readdir = fs::read_dir(opts.root)?;
    let mut stack: Vec<Frame> = vec![Frame {
        readdir: root_readdir,
        path: opts.root.to_path_buf(),
        agg: DirAgg::default(),
        depth: 0,
    }];

    let mut entries: Vec<DirEntry> = Vec::new();
    let mut warnings: Vec<ScanWarning> = Vec::new();
    let mut visited: u64 = 0;

    loop {
        let top_next = stack
            .last_mut()
            .expect("root frame is always alive")
            .readdir
            .next();

        // A directory is finished when its iterator is exhausted: pop it,
        // fold it into its parent, and record its entry.
        if top_next.is_none() {
            let frame = stack.pop().expect("root frame is always alive");
            let is_root = stack.is_empty();
            if !is_root {
                let parent = stack.last_mut().expect("non-root frame has a parent");
                parent.agg.merge_child(&frame.agg);
            }
            entries.push(DirEntry {
                path: frame.path,
                agg: frame.agg,
            });
            if is_root {
                break;
            }
            continue;
        }

        match top_next.expect("checked Some above") {
            Err(e) => {
                // The directory iterator itself reported an error (an entry
                // disappeared between listing and reading).
                let top = stack.last_mut().expect("root frame is always alive");
                top.agg.add_skipped();
                let path = top.path.clone();
                warnings.push(ScanWarning {
                    path,
                    error: e.to_string(),
                    kind: WarningKind::EntryReadFailed,
                });
                continue;
            }
            Ok(entry) => {
                visited = visited.saturating_add(1);
                on_progress(visited);

                // metadata() does not traverse symlinks (lstat semantics).
                let md = match entry.metadata() {
                    Ok(md) => md,
                    Err(e) => {
                        let top = stack.last_mut().expect("root frame is always alive");
                        top.agg.add_skipped();
                        warnings.push(ScanWarning {
                            path: entry.path(),
                            error: e.to_string(),
                            kind: WarningKind::StatFailed,
                        });
                        continue;
                    }
                };
                let ft = md.file_type();

                // Symlinks are never followed and never counted (DESIGN §7).
                if ft.is_symlink() {
                    continue;
                }
                if !ft.is_dir() {
                    // Regular or special file (fifo/socket/device): count once.
                    let top = stack.last_mut().expect("root frame is always alive");
                    top.agg.add_file(md.len());
                    continue;
                }

                // From here on: a directory.
                let path = entry.path(); // built only for directories & failures

                if let Some(excl) = opts.exclude {
                    if is_under(&path, excl) {
                        top_dir_only(&mut stack);
                        continue;
                    }
                }
                if let Some(depth_limit) = opts.max_depth {
                    let parent_depth = stack.last().expect("root frame is always alive").depth;
                    if parent_depth + 1 > depth_limit {
                        top_dir_only(&mut stack);
                        continue;
                    }
                }
                if let Some(dev) = root_dev {
                    if let Some(child_dev) = unix_dev(&md) {
                        if child_dev != dev {
                            // Different filesystem: don't descend, count dir only.
                            top_dir_only(&mut stack);
                            continue;
                        }
                    }
                }

                match fs::read_dir(&path) {
                    Ok(child) => {
                        let parent_depth = stack.last().expect("root frame is always alive").depth;
                        stack.push(Frame {
                            readdir: child,
                            path: path.clone(),
                            agg: DirAgg::default(),
                            depth: parent_depth + 1,
                        });
                    }
                    Err(e) => {
                        // Directory unreadable (permission denied / vanished).
                        let top = stack.last_mut().expect("root frame is always alive");
                        top.agg.add_skipped();
                        warnings.push(ScanWarning {
                            path,
                            error: e.to_string(),
                            kind: WarningKind::UnreadableDir,
                        });
                    }
                }
            }
        }
    }

    let root_entry = entries.last().expect("root frame is always recorded");
    let summary = ScanSummary {
        total_size: root_entry.agg.total_size,
        file_count: root_entry.agg.file_count,
        dir_count: root_entry.agg.dir_count,
        skipped_count: warnings.len() as u64,
        elapsed: started.elapsed(),
    };

    Ok(ScanResult {
        root: opts.root.to_path_buf(),
        entries,
        summary,
        warnings,
    })
}

/// Count a policy-excluded directory in the current top frame (1 dir, no data),
/// without descending. See DESIGN §13.
fn top_dir_only(stack: &mut [Frame]) {
    let top = stack.last_mut().expect("root frame is always alive");
    top.agg.add_dir_only();
}
