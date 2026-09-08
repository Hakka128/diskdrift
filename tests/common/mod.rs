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
