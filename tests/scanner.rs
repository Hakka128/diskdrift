//! Scanner integration tests — run against real filesystem fixtures in
//! `tempfile` temp dirs. These exercise the walker (not storage).

use std::fs;
use std::path::Path;

use tempfile::TempDir;
use whybig::scanner::{scan, DirEntry, ScanOptions, ScanResult};

/// Scan one directory with default options (no exclusions, unlimited depth).
fn scan_dir(dir: &Path) -> ScanResult {
    let opts = ScanOptions {
        root: dir.to_path_buf(),
        exclude: None,
        max_depth: None,
    };
    scan(&opts, &mut |_| {}).expect("scan should succeed")
}

/// Look up one directory entry by path.
fn entry<'a>(res: &'a ScanResult, path: &Path) -> &'a DirEntry {
    res.entries
        .iter()
        .find(|e| e.path == path)
        .unwrap_or_else(|| panic!("no entry for {}", path.display()))
}

fn write_file(path: &Path, content: &[u8]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

/// A simple fixture: root/{a.txt,b.txt,sub/c.txt}.
fn small_tree() -> TempDir {
    let td = TempDir::new().unwrap();
    write_file(&td.path().join("a.txt"), &[1u8; 100]);
    write_file(&td.path().join("b.txt"), &[2u8; 200]);
    write_file(&td.path().join("sub").join("c.txt"), &[3u8; 50]);
    td
}

// ─── size / count basics ────────────────────────────────────────────────

#[test]
fn plain_files_and_nested_dirs_are_aggregated() {
    let td = small_tree();
    let res = scan_dir(td.path());

    assert_eq!(res.summary.total_size, 350);
    assert_eq!(res.summary.file_count, 3);
    assert_eq!(res.summary.dir_count, 1);
    assert_eq!(res.summary.skipped_count, 0);
    assert_eq!(res.entries.len(), 2, "root + sub only, never per-file rows");

    let root = entry(&res, td.path());
    assert_eq!(root.agg.total_size, 350);
    assert_eq!(root.agg.file_count, 3);
    assert_eq!(root.agg.dir_count, 1);

    let sub = entry(&res, &td.path().join("sub"));
    assert_eq!(sub.agg.total_size, 50);
    assert_eq!(sub.agg.file_count, 1);
    assert_eq!(sub.agg.dir_count, 0);
}

#[test]
fn empty_file_counts_as_one_zero_length_file() {
    let td = TempDir::new().unwrap();
    fs::write(td.path().join("empty.txt"), b"").unwrap();
    let res = scan_dir(td.path());
    assert_eq!(res.summary.file_count, 1);
    assert_eq!(res.summary.total_size, 0);
}

#[test]
fn large_file_reports_its_apparent_size() {
    let td = TempDir::new().unwrap();
    write_file(&td.path().join("big.bin"), &vec![0u8; 8 * 1024 * 1024]);
    let res = scan_dir(td.path());
    assert_eq!(res.summary.file_count, 1);
    assert_eq!(res.summary.total_size, 8 * 1024 * 1024);
}

#[test]
fn sparse_file_uses_logical_size_not_allocated() {
    // set_len creates a sparse file: allocated << logical. WhyBig reports the
    // *apparent* (logical) size — asserting that documented behavior.
    let td = TempDir::new().unwrap();
    let f = fs::File::create(td.path().join("sparse.bin")).unwrap();
    f.set_len(256 * 1024 * 1024).unwrap();
    drop(f);

    let res = scan_dir(td.path());
    assert_eq!(res.summary.file_count, 1);
    assert_eq!(res.summary.total_size, 256 * 1024 * 1024);
}

#[test]
fn empty_directory_side_by_side_with_nested_ones() {
    let td = TempDir::new().unwrap();
    fs::create_dir_all(td.path().join("a_dir")).unwrap();
    fs::create_dir_all(td.path().join("b_dir").join("sub_dir")).unwrap();

    let res = scan_dir(td.path());
    // root merges: a_dir(0) -> +1, b_dir(1) -> +2  => 3
    assert_eq!(res.summary.dir_count, 3);
    assert_eq!(res.summary.file_count, 0);
    assert_eq!(res.summary.total_size, 0);

    let a = entry(&res, &td.path().join("a_dir"));
    assert_eq!(a.agg.dir_count, 0);
    assert_eq!(a.agg.total_size, 0);
    let sub = entry(&res, &td.path().join("b_dir").join("sub_dir"));
    assert_eq!(sub.agg.dir_count, 0);
}

#[cfg(unix)]
#[test]
fn deep_nested_directories_do_not_overflow_the_stack() {
    // Explicit-stack walker: 5,000 levels must be fine (a recursive walker
    // would blow the call stack long before this). Unix-only: creating a
    // 5,000-deep tree exceeds Windows MAX_PATH on many CI runners, and
    // per-entry metadata on Windows is O(depth) — both make the same fixture
    // pathological there without testing anything new.
    let td = TempDir::new().unwrap();
    let mut p = td.path().to_path_buf();
    for i in 0..5_000 {
        p.push(format!("d{i}"));
    }
    fs::create_dir_all(&p).unwrap();
    write_file(&p.join("leaf.txt"), b"x");

    let res = scan_dir(td.path());
    assert_eq!(res.summary.dir_count, 5_000);
    assert_eq!(res.summary.file_count, 1);
    assert_eq!(res.summary.total_size, 1);
}

#[cfg(windows)]
#[test]
fn deep_directory_tree_is_aggregated() {
    // Depth comfortably below MAX_PATH; proves the walker handles nesting
    // (and Windows path joining) correctly without relying on long paths.
    let td = TempDir::new().unwrap();
    let mut p = td.path().to_path_buf();
    for i in 0..60 {
        p.push(format!("d{i}"));
    }
    fs::create_dir_all(&p).unwrap();
    write_file(&p.join("leaf.txt"), b"x");

    let res = scan_dir(td.path());
    assert_eq!(res.summary.dir_count, 60);
    assert_eq!(res.summary.file_count, 1);
    assert_eq!(res.summary.total_size, 1);
}

#[test]
fn many_small_files_are_counted() {
    let td = TempDir::new().unwrap();
    let dir = td.path().join("flat");
    fs::create_dir_all(&dir).unwrap();
    for i in 0..3_000 {
        write_file(&dir.join(format!("f{i:04}.txt")), &[7u8; 10]);
    }
    let res = scan_dir(td.path());
    assert_eq!(res.summary.file_count, 3_000);
    assert_eq!(res.summary.total_size, 30_000);
    assert_eq!(res.summary.dir_count, 1);
}

#[test]
fn unicode_spaces_and_emoji_paths_are_handled() {
    let td = TempDir::new().unwrap();
    write_file(&td.path().join("数据").join("测试.txt"), b"11");
    write_file(
        &td.path().join("h\u{e9}llo w\u{f6}rld").join("a b c.txt"),
        b"22",
    );
    write_file(&td.path().join("🎉 party").join("emoji-🎉.txt"), b"33");
    write_file(&td.path().join("名前.txt"), b"44");

    let res = scan_dir(td.path());
    assert_eq!(res.summary.file_count, 4);
    assert_eq!(res.summary.total_size, 8);
    assert_eq!(res.summary.dir_count, 3);
}

#[test]
fn exclusion_subtree_contributes_nothing() {
    let td = TempDir::new().unwrap();
    write_file(&td.path().join("keep.txt"), b"12345");
    write_file(&td.path().join("excluded").join("inner.txt"), b"999");
    write_file(
        &td.path().join("excluded").join("deeper").join("x.txt"),
        b"1",
    );

    let opts = ScanOptions {
        root: td.path().to_path_buf(),
        exclude: Some(td.path().join("excluded")),
        max_depth: None,
    };
    let res = scan(&opts, &mut |_| {}).unwrap();

    assert_eq!(res.summary.file_count, 1);
    assert_eq!(res.summary.total_size, 5);
    // the excluded dir itself is counted as 1 dir (dir-only), contents dropped
    assert_eq!(res.summary.dir_count, 1);
    // ...but it produces no entry row (we never descended into it)
    assert!(
        res.entries.iter().all(|e| !e.path.ends_with("excluded")),
        "excluded subtree must not produce an entry row"
    );
}

// ─── symlinks (Unix; creating symlinks needs privileges on Windows) ─────

#[cfg(unix)]
mod symlinks {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn symlink_to_file_is_not_followed_or_counted() {
        let td = TempDir::new().unwrap();
        write_file(&td.path().join("target.txt"), &[9u8; 100]);
        symlink(td.path().join("target.txt"), td.path().join("link.txt")).unwrap();

        let res = scan_dir(td.path());
        assert_eq!(res.summary.total_size, 100, "link must not add target size");
        assert_eq!(res.summary.file_count, 1, "only the real file counts");
        assert_eq!(res.summary.skipped_count, 0);
    }

    #[test]
    fn symlink_to_directory_target_is_not_walked() {
        let td = TempDir::new().unwrap();
        write_file(&td.path().join("real").join("a.txt"), b"1");
        write_file(&td.path().join("target_dir").join("b.txt"), &[5u8; 60]);
        symlink(td.path().join("target_dir"), td.path().join("dirlink")).unwrap();

        let res = scan_dir(td.path());
        // target_dir counted once (through its real path), never through link
        assert_eq!(res.summary.total_size, 61);
        assert_eq!(res.summary.file_count, 2);
        assert_eq!(res.summary.dir_count, 2); // real + target_dir
    }

    #[test]
    fn broken_symlink_is_not_an_error() {
        let td = TempDir::new().unwrap();
        write_file(&td.path().join("real.txt"), b"xx");
        symlink(td.path().join("does-not-exist"), td.path().join("dangling")).unwrap();

        let res = scan_dir(td.path());
        assert_eq!(res.summary.skipped_count, 0, "broken link is not an error");
        assert_eq!(res.summary.file_count, 1);
    }

    #[test]
    fn symlink_loop_terminates() {
        let td = TempDir::new().unwrap();
        write_file(&td.path().join("a").join("f.txt"), b"1");
        write_file(&td.path().join("b").join("g.txt"), b"2");
        // a -> b and b -> a form a cycle when followed; we never follow.
        symlink(td.path().join("b"), td.path().join("a").join("to_b")).unwrap();
        symlink(td.path().join("a"), td.path().join("b").join("to_a")).unwrap();
        // self-referential link pointing at root
        symlink(td.path(), td.path().join("back_to_root")).unwrap();

        let res = scan_dir(td.path());
        assert_eq!(res.summary.skipped_count, 0);
        assert_eq!(res.summary.file_count, 2);
        assert_eq!(res.summary.total_size, 3);
    }

    #[test]
    fn permission_denied_directory_is_skipped_not_fatal() {
        let td = TempDir::new().unwrap();
        write_file(&td.path().join("ok.txt"), b"123");
        let secret = td.path().join("secret");
        write_file(&secret.join("hidden.bin"), &[1u8; 10_000]);

        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&secret, fs::Permissions::from_mode(0o000)).unwrap();

        let res = scan_dir(td.path());
        fs::set_permissions(&secret, fs::Permissions::from_mode(0o755)).unwrap();

        assert_eq!(res.summary.file_count, 1, "the readable file still counts");
        assert_eq!(res.summary.total_size, 3);
        assert_eq!(res.summary.skipped_count, 1, "the blocked dir is one skip");
        assert_eq!(res.warnings.len(), 1);
        assert!(res.warnings[0].path.ends_with("secret"));
    }

    #[test]
    fn hard_links_are_counted_per_directory_entry() {
        let td = TempDir::new().unwrap();
        write_file(&td.path().join("a.txt"), &[4u8; 100]);
        fs::hard_link(td.path().join("a.txt"), td.path().join("b.txt")).unwrap();

        let res = scan_dir(td.path());
        assert_eq!(
            res.summary.total_size, 200,
            "each link contributes its size"
        );
        assert_eq!(res.summary.file_count, 2);
    }
}

// ─── root-level behavior ─────────────────────────────────────────────────

#[test]
fn scanning_a_file_root_errors_friendly() {
    let td = TempDir::new().unwrap();
    let f = td.path().join("not-a-dir.txt");
    write_file(&f, b"x");

    let opts = ScanOptions {
        root: f,
        exclude: None,
        max_depth: None,
    };
    let err = scan(&opts, &mut |_| {}).unwrap_err();
    assert!(err.to_string().contains("not a directory"));
}

#[test]
fn scanning_a_missing_root_errors() {
    let td = TempDir::new().unwrap();
    let missing = td.path().join("missing");
    let opts = ScanOptions {
        root: missing,
        exclude: None,
        max_depth: None,
    };
    assert!(scan(&opts, &mut |_| {}).is_err());
}

#[test]
fn progress_callback_is_invoked() {
    let td = small_tree();
    let mut seen = 0u64;
    let res = scan_dir_progress(td.path(), &mut |n| seen = n);
    assert!(seen >= res.summary.file_count + res.summary.dir_count);
    assert!(seen >= 3);
}

fn scan_dir_progress(dir: &Path, on_progress: &mut dyn FnMut(u64)) -> ScanResult {
    let opts = ScanOptions {
        root: dir.to_path_buf(),
        exclude: None,
        max_depth: None,
    };
    scan(&opts, on_progress).unwrap()
}

// ─── races: mutation during scan (robustness, best-effort) ───────────────
//
// These can't be made fully deterministic (the OS schedules the mutation),
// so they assert *invariants*: a scan racing against deletion/growth must
// complete successfully and its counters must stay inside consistent bounds.

fn trip_when(
    threshold: u64,
    flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> impl FnMut(u64) {
    move |n| {
        if n >= threshold {
            flag.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

fn wait_for(flag: &std::sync::atomic::AtomicBool) {
    while !flag.load(std::sync::atomic::Ordering::Relaxed) {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[test]
fn files_deleted_during_scan_never_fail_the_scan() {
    let td = TempDir::new().unwrap();
    let dir = td.path().join("big");
    std::fs::create_dir_all(&dir).unwrap();
    for i in 0..4_000 {
        write_file(&dir.join(format!("del-{i:05}.tmp")), &[7u8; 4]);
    }
    write_file(&dir.join("keep-0.txt"), &[1u8; 100]);
    write_file(&dir.join("keep-1.txt"), &[1u8; 100]);

    let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let deleter = {
        let dir = dir.clone();
        let flag = std::sync::Arc::clone(&flag);
        std::thread::spawn(move || {
            wait_for(&flag);
            if let Ok(rd) = std::fs::read_dir(&dir) {
                for e in rd.flatten() {
                    if e.file_name().to_string_lossy().starts_with("del-") {
                        let _ = std::fs::remove_file(e.path());
                    }
                }
            }
        })
    };

    let opts = ScanOptions {
        root: dir.clone(),
        exclude: None,
        max_depth: None,
    };
    let res =
        scan(&opts, &mut trip_when(512, std::sync::Arc::clone(&flag))).expect("scan never fails");
    deleter.join().unwrap();

    assert_eq!(
        res.summary.total_size % 4,
        0,
        "all deletable files are 4 bytes"
    );
    assert!(
        res.summary.total_size >= 200,
        "keep files are always counted"
    );
    assert!(
        res.summary.total_size <= 4 * 4_000 + 200,
        "never above the full tree"
    );
    assert!(res.summary.file_count >= 2);
    assert!(
        res.summary.skipped_count <= 4_000,
        "at worst every del file is skipped"
    );
}

#[test]
fn directory_deleted_during_scan_never_fails_the_scan() {
    let td = TempDir::new().unwrap();
    for i in 0..20 {
        write_file(&td.path().join(format!("d{i}")).join("f.txt"), &[3u8; 10]);
    }
    let victim = td.path().join("d5");

    let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let deleter = {
        let victim = victim.clone();
        let flag = std::sync::Arc::clone(&flag);
        std::thread::spawn(move || {
            wait_for(&flag);
            let _ = std::fs::remove_dir_all(&victim);
        })
    };

    let opts = ScanOptions {
        root: td.path().to_path_buf(),
        exclude: None,
        max_depth: None,
    };
    let res = scan(&opts, &mut trip_when(16, flag)).expect("scan never fails");
    deleter.join().unwrap();

    // Between 190 (d5 fully gone before being seen) and 200 (fully counted).
    assert!(res.summary.total_size >= 190, "at most one dir is affected");
    assert!(res.summary.total_size <= 200);
}

#[test]
fn file_growing_during_scan_is_sampled_without_failure() {
    let td = TempDir::new().unwrap();
    let grow = td.path().join("grow.bin");
    write_file(&grow, &[0u8; 100]);
    write_file(&td.path().join("small.txt"), &[1u8; 1]);

    let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let grower = {
        let grow = grow.clone();
        let flag = std::sync::Arc::clone(&flag);
        std::thread::spawn(move || {
            wait_for(&flag);
            let _ = std::fs::File::options()
                .write(true)
                .open(&grow)
                .and_then(|f| f.set_len(100 * 1024 * 1024));
        })
    };

    let opts = ScanOptions {
        root: td.path().to_path_buf(),
        exclude: None,
        max_depth: None,
    };
    let res = scan(&opts, &mut trip_when(1, flag)).expect("scan never fails");
    grower.join().unwrap();

    // A point-in-time sample: one of the two apparent lengths, never partial.
    assert!(
        res.summary.total_size == 101 || res.summary.total_size == 100 * 1024 * 1024 + 1,
        "size is exactly one sampled value: {}",
        res.summary.total_size
    );
}
