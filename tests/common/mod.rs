//! Shared test harness for Milestone 3 integration tests.
//!
//! Different test binaries use different subsets, so unused-item warnings are
//! expected (and would trip `clippy -D warnings`).

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

use diskdrift::snapshot::service::SnapshotService;
use diskdrift::storage::Storage;
use tempfile::TempDir;

pub struct Harness {
    pub _td: TempDir,
    pub root: PathBuf,
    pub data_dir: PathBuf,
    pub service: SnapshotService,
}

pub fn new_harness() -> Harness {
    let td = TempDir::new().unwrap();
    let root = td.path().join("tree");
    fs::create_dir_all(&root).unwrap();
    let data_dir = td.path().join("data");
    let service = SnapshotService::new(data_dir.clone()).unwrap();
    Harness {
        _td: td,
        root,
        data_dir,
        service,
    }
}

impl Harness {
    pub fn snap(&mut self) -> i64 {
        self.service
            .run_snapshot(&self.root, &mut |_| {})
            .unwrap()
            .snapshot_id
    }

    pub fn write(&self, rel: &str, content: &[u8]) {
        let p = self.root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, content).unwrap();
    }

    pub fn remove(&self, rel: &str) {
        let _ = fs::remove_dir_all(self.root.join(rel));
    }

    pub fn storage(&self) -> Storage {
        Storage::open(&self.data_dir).unwrap()
    }

    pub fn abs(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }
}

/// A directory from a scan/`direct children` — kept minimal for assertions.
pub fn rel_of<P: AsRef<Path>>(path: P, root: &Path) -> String {
    path.as_ref()
        .strip_prefix(root)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.as_ref().to_string_lossy().into_owned())
}

/// Test-side normalization for *representational* path equality across
/// filesystem aliases: Windows 8.3 short names (`RUNNER~1` vs `runneradmin`)
/// and macOS `/var` ↔ `/private/var`. Canonicalizes the nearest existing
/// ancestor and re-appends any missing suffix — the same logic the historical
/// local copies in `tests/diff.rs` / `tests/history.rs` use. This must ONLY be
/// used where a test asserts path *identity*; it is not a product lookup.
pub fn comparable_path(p: &Path) -> PathBuf {
    let plain = PathBuf::from(strip_verbatim(p));
    #[cfg(windows)]
    {
        let mut probe = plain.as_path();
        let mut missing: Vec<std::ffi::OsString> = Vec::new();
        loop {
            if let Ok(base) = fs::canonicalize(probe) {
                let mut resolved = PathBuf::from(strip_verbatim(&base));
                for component in missing.iter().rev() {
                    resolved.push(component);
                }
                return resolved;
            }
            let Some(name) = probe.file_name() else { break };
            missing.push(name.to_os_string());
            let Some(parent) = probe.parent() else { break };
            probe = parent;
        }
        plain
    }
    #[cfg(target_os = "macos")]
    {
        if let Ok(rest) = plain.strip_prefix("/private") {
            return Path::new("/").join(rest);
        }
        plain
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        plain
    }
}

/// Strip the Windows `\\?\` verbatim prefix for test comparison.
fn strip_verbatim(p: &Path) -> String {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        s.into_owned()
    }
}

/// The **stored path spelling** the real product would write for an arbitrary
/// path: canonicalize (resolves Windows 8.3 aliases to long names and macOS
/// `/var`→`/private/var`, preserving the canonical `/private` spelling) and
/// strip the `\\?\` prefix — matching `SnapshotService::resolve_root`.
/// When the path does not exist, canonicalize the nearest existing ancestor
/// and re-append the missing suffix; falls back to the input when nothing
/// exists. Synthetic fixtures and comparison helpers that query DB keys must
/// use this; `comparable_path` (identity normalization, macOS strips
/// `/private`) is a *different* concept and is not a storage key.
pub fn stored_path_string(p: &Path) -> String {
    let mut probe = p;
    let mut missing: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if let Ok(base) = fs::canonicalize(probe) {
            let mut resolved = PathBuf::from(strip_verbatim(&base));
            for component in missing.iter().rev() {
                resolved.push(component);
            }
            return resolved.to_string_lossy().into_owned();
        }
        let Some(name) = probe.file_name() else { break };
        missing.push(name.to_os_string());
        let Some(parent) = probe.parent() else { break };
        probe = parent;
    }
    p.to_string_lossy().into_owned()
}

/// `stored_path_string` for a temp dir (its canonical root is the real stored
/// root spelling).
pub fn stored_root_string(td: &TempDir) -> String {
    stored_path_string(td.path())
}
