//! Diff engine integration tests: run two snapshots on a real temp tree and
//! assert the resulting attribution is correct (no double counting, correct
//! added/removed handling, correct selection, overflow safety).

use std::fs;
use std::path::{Path, PathBuf};

use diskdrift::diff::{self, DiffEntry, DiffSelection, DiffState, SnapshotDiff};
use diskdrift::output::diff as render_diff;
use diskdrift::snapshot::service::SnapshotService;
use diskdrift::storage::Storage;
use tempfile::TempDir;

struct Harness {
    _td: TempDir,
    root: PathBuf,
    data_dir: PathBuf,
    service: SnapshotService,
}

/// Strip the Windows `\\?\` prefix the same way the service does.
fn plain_path(p: &Path) -> String {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        s.into_owned()
    }
}

fn new_harness() -> Harness {
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
    fn snap(&mut self) -> i64 {
        self.service
            .run_snapshot(&self.root, &mut |_| {})
            .unwrap()
            .snapshot_id
    }

    fn write(&self, rel: &str, content: &[u8]) {
        let p = self.root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, content).unwrap();
    }

    fn remove(&self, rel: &str) {
        fs::remove_dir_all(self.root.join(rel)).unwrap();
    }

    fn storage(&self) -> Storage {
        Storage::open(&self.data_dir).unwrap()
    }

    fn set_len(&self, rel: &str, size: u64) {
        let p = self.root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        let f = fs::File::create(p).unwrap();
        f.set_len(size).unwrap();
    }

    fn compute(&self) -> SnapshotDiff {
        let storage = self.storage();
        let pair = diff::select_pair(&storage, DiffSelection::Default).unwrap();
        let (b, a) = (pair.before, pair.after);
        diff::compute(&storage, &b, &a, &a.root_path).unwrap()
    }

    fn abs_path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }
}

fn entry_in<'a>(entries: &'a [DiffEntry], abs: &Path) -> Option<&'a DiffEntry> {
    fn comparable_path(p: &Path) -> PathBuf {
        let plain = PathBuf::from(plain_path(p));
        #[cfg(windows)]
        {
            let mut probe = plain.as_path();
            let mut missing = Vec::new();

            loop {
                if let Ok(base) = std::fs::canonicalize(probe) {
                    let mut resolved = PathBuf::from(plain_path(&base));

                    for component in missing.iter().rev() {
                        resolved.push(component);
                    }

                    return resolved;
                }

                let Some(name) = probe.file_name() else {
                    break;
                };

                missing.push(name.to_os_string());

                let Some(parent) = probe.parent() else {
                    break;
                };

                probe = parent;
            }
        }
        #[cfg(target_os = "macos")]
        {
            if let Ok(rest) = plain.strip_prefix("/private") {
                return Path::new("/").join(rest);
            }
        }

        plain
    }

    let expected = comparable_path(abs);

    entries
        .iter()
        .find(|e| comparable_path(Path::new(&e.path)) == expected)
}

// ─── basic diff ──────────────────────────────────────────────────────────

#[test]
fn identical_snapshots_yield_no_entries() {
    let mut h = new_harness();
    h.write("a/f1", &[1u8; 100]);
    h.write("b/f2", &[2u8; 50]);
    h.snap();
    h.snap(); // nothing changed

    let d = h.compute();
    assert_eq!(d.total_delta, 0);
    assert!(d.grew.is_empty());
    assert!(d.shrank.is_empty());
}

#[test]
fn one_directory_grows() {
    let mut h = new_harness();
    h.write("a/f1", &[1u8; 100]);
    h.write("b/f", &[2u8; 50]);
    h.snap();

    h.write("a/f2", &[3u8; 150]); // a grows by 150
    h.snap();

    let d = h.compute();
    assert_eq!(d.total_delta, 150);
    assert_eq!(d.grew.len(), 1);
    let a = entry_in(&d.grew, &h.abs_path("a")).unwrap();
    assert_eq!(a.delta, 150);
    assert_eq!(a.state, DiffState::Changed);
    assert!(d.shrank.is_empty());
}

#[test]
fn one_directory_shrinks() {
    let mut h = new_harness();
    h.write("a/f", &[1u8; 100]);
    h.write("b/f", &[2u8; 50]);
    h.snap();

    fs::remove_file(h.abs_path("a/f")).unwrap();
    h.snap();

    let d = h.compute();
    assert_eq!(d.total_delta, -100);
    assert!(d.grew.is_empty());
    assert_eq!(d.shrank.len(), 1);
    let a = entry_in(&d.shrank, &h.abs_path("a")).unwrap();
    assert_eq!(a.delta, -100);
    assert_eq!(a.state, DiffState::Changed);
}

#[test]
fn directory_added() {
    let mut h = new_harness();
    h.write("a/f", &[1u8; 100]);
    h.snap();
    h.write("newdir/f", &[2u8; 200]);
    h.snap();

    let d = h.compute();
    let nd = entry_in(&d.grew, &h.abs_path("newdir")).unwrap();
    assert_eq!(nd.state, DiffState::Added);
    assert_eq!(nd.before_size, 0);
    assert_eq!(nd.after_size, 200);
    assert_eq!(nd.delta, 200);
}

#[test]
fn directory_removed() {
    let mut h = new_harness();
    h.write("a/f", &[1u8; 100]);
    h.write("gone/f", &[2u8; 77]);
    h.snap();
    h.remove("gone");
    h.snap();

    let d = h.compute();
    let gone = entry_in(&d.shrank, &h.abs_path("gone")).unwrap();
    assert_eq!(gone.state, DiffState::Removed);
    assert_eq!(gone.before_size, 77);
    assert_eq!(gone.after_size, 0);
    assert_eq!(gone.delta, -77);
}

#[test]
fn multiple_directories_and_sorting() {
    let mut h = new_harness();
    h.write("z/f", &[1u8; 10]);
    h.write("m/f", &[1u8; 20]);
    h.write("a/f", &[1u8; 30]);
    h.snap();

    h.write("a/g", &[1u8; 300]);
    h.write("m/g", &[1u8; 200]);
    h.write("z/g", &[1u8; 100]);
    h.snap();

    let d = h.compute();
    let deltas: Vec<String> = d
        .grew
        .iter()
        .map(|e| {
            Path::new(&e.path)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(
        deltas,
        vec![
            "a".to_string(), // +300
            "m".to_string(), // +200
            "z".to_string()  // +100
        ]
    );
}

#[test]
fn shrank_sorted_by_absolute_delta() {
    let mut h = new_harness();
    h.write("big/f", &[1u8; 1000]);
    h.write("small/f", &[1u8; 100]);
    h.write("med/f", &[1u8; 500]);
    h.snap();

    fs::remove_file(h.abs_path("big/f")).unwrap();
    fs::remove_file(h.abs_path("med/f")).unwrap();
    fs::remove_file(h.abs_path("small/f")).unwrap();
    h.snap();

    let d = h.compute();
    let names: Vec<String> = d
        .shrank
        .iter()
        .map(|e| {
            Path::new(&e.path)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(
        names,
        vec!["big".to_string(), "med".to_string(), "small".to_string()]
    );
}

#[test]
fn unchanged_directories_do_not_appear() {
    let mut h = new_harness();
    h.write("stable/f", &[1u8; 42]);
    h.write("mover/f", &[1u8; 7]);
    h.snap();
    h.write("mover/g", &[1u8; 13]);
    h.snap();

    let d = h.compute();
    assert!(entry_in(&d.grew, &h.abs_path("stable")).is_none());
    assert!(entry_in(&d.shrank, &h.abs_path("stable")).is_none());
    // Only mover changed.
    assert_eq!(d.grew.len(), 1);
}

#[test]
fn root_itself_growth_is_reflected_in_totals() {
    let mut h = new_harness();
    h.write("a/f", &[1u8; 100]);
    h.snap();
    // A file directly in the root, plus a child-dir change.
    fs::write(h.root.join("direct.bin"), [9u8; 64]).unwrap();
    h.write("a/g", &[1u8; 36]);
    h.snap();

    let d = h.compute();
    // total delta includes the direct file (64) + child growth (36)
    assert_eq!(d.total_delta, 100);
    let a = entry_in(&d.grew, &h.abs_path("a")).unwrap();
    assert_eq!(a.delta, 36);
}

// ─── attribution / collapse ──────────────────────────────────────────────

#[test]
fn parent_and_child_growth_is_collapsed_to_parent() {
    let mut h = new_harness();
    h.write("a/b/c/f", &[1u8; 5]);
    h.snap();
    h.write("a/b/c/g", &[1u8; 200]);
    h.snap();

    let d = h.compute();
    // Only the top-level directory `a` appears; its delta equals the full
    // growth. Sum of entries == total ⇒ no double counting.
    assert_eq!(d.grew.len(), 1);
    let a = entry_in(&d.grew, &h.abs_path("a")).unwrap();
    assert_eq!(a.delta, 200);
    let sum: i128 = d.grew.iter().chain(&d.shrank).map(|e| e.delta).sum();
    assert_eq!(sum, d.total_delta);
}

#[test]
fn only_grandchild_grows_still_collapses_to_child() {
    let mut h = new_harness();
    h.write("a/b/f", &[1u8; 1]);
    h.snap();
    h.write("a/b/g", &[1u8; 99]);
    h.snap();

    let d = h.compute();
    // `a` appears (its aggregate grew through b); `b` must not appear at the
    // top level.
    assert_eq!(d.grew.len(), 1);
    assert!(entry_in(&d.grew, &h.abs_path("a/b")).is_none());
    let a = entry_in(&d.grew, &h.abs_path("a")).unwrap();
    assert_eq!(a.delta, 99);
}

#[test]
fn prefix_collision_bar_vs_bar2() {
    let mut h = new_harness();
    h.write("bar/f", &[1u8; 10]);
    h.write("bar2/f", &[1u8; 20]);
    h.snap();

    h.write("bar2/g", &[1u8; 80]); // only bar2 grows
    h.snap();

    let d = h.compute();
    assert_eq!(d.grew.len(), 1, "only bar2 should grow");
    let bar2 = entry_in(&d.grew, &h.abs_path("bar2")).unwrap();
    assert_eq!(bar2.delta, 80);
    assert!(entry_in(&d.grew, &h.abs_path("bar")).is_none());
}

#[test]
fn direct_files_in_root_count_in_total_but_not_as_entries() {
    let mut h = new_harness();
    h.write("a/f", &[1u8; 10]);
    h.snap();
    fs::write(h.root.join("loose.bin"), [7u8; 1000]).unwrap();
    h.snap();

    let d = h.compute();
    // 1000 bytes of direct files are visible only in the total.
    assert_eq!(d.total_delta, 1000);
    let entry_sum: i128 = d.grew.iter().chain(&d.shrank).map(|e| e.delta).sum();
    assert_eq!(entry_sum, 0, "no directory changed, so no entries");
}

// ─── selection + cross-root ──────────────────────────────────────────────

#[test]
fn default_selection_uses_latest_two_of_current_root() {
    let mut h = new_harness();
    h.snap(); // id1 (empty)
    h.write("x/f", &[1u8; 100]);
    h.snap(); // id2
    h.write("x/g", &[1u8; 50]);
    h.snap(); // id3

    let storage = h.storage();
    let pair = diff::select_pair(&storage, DiffSelection::Default).unwrap();
    let (before, after) = (pair.before, pair.after);
    // Must be id2 → id3, not id1 → id3.
    assert_eq!(before.id, 2);
    assert_eq!(after.id, 3);
}

#[test]
fn multiple_roots_never_cross_compare_by_default() {
    let td = TempDir::new().unwrap();
    let data_dir = td.path().join("data");
    let mut service = SnapshotService::new(data_dir.clone()).unwrap();

    let r1 = td.path().join("r1");
    let r2 = td.path().join("r2");
    fs::create_dir_all(&r1).unwrap();
    fs::create_dir_all(&r2).unwrap();
    service.run_snapshot(&r1, &mut |_| {}).unwrap();
    service.run_snapshot(&r2, &mut |_| {}).unwrap();
    fs::write(r2.join("f"), [1u8; 5]).unwrap();
    service.run_snapshot(&r2, &mut |_| {}).unwrap();

    let storage = Storage::open(&data_dir).unwrap();
    let pair = diff::select_pair(&storage, DiffSelection::Default).unwrap();
    let (before, after) = (pair.before, pair.after);
    // Latest overall root is r2; both chosen snapshots belong to r2.
    assert_eq!(before.root_path, after.root_path);
    let r2_canon = plain_path(&fs::canonicalize(&r2).unwrap());
    assert_eq!(after.root_path, r2_canon);
}

#[test]
fn explicit_cross_root_comparison_is_rejected() {
    let td = TempDir::new().unwrap();
    let data_dir = td.path().join("data");
    let mut service = SnapshotService::new(data_dir.clone()).unwrap();

    let r1 = td.path().join("r1");
    let r2 = td.path().join("r2");
    fs::create_dir_all(&r1).unwrap();
    fs::create_dir_all(&r2).unwrap();
    let id1 = service.run_snapshot(&r1, &mut |_| {}).unwrap().snapshot_id;
    let id2 = service.run_snapshot(&r2, &mut |_| {}).unwrap().snapshot_id;

    let storage = Storage::open(&data_dir).unwrap();
    let err =
        diff::select_pair(&storage, DiffSelection::Explicit { from: id1, to: id2 }).unwrap_err();
    assert!(err.to_string().contains("different roots"));
}

#[test]
fn explicit_missing_snapshot_errors_friendly() {
    let mut h = new_harness();
    let id = h.snap();
    let storage = h.storage();
    let err = diff::select_pair(
        &storage,
        DiffSelection::Explicit {
            from: id,
            to: id + 999,
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("not found"));
}

#[test]
fn fewer_than_two_snapshots_is_a_friendly_error() {
    let mut h = new_harness();
    h.snap(); // only one
    let storage = h.storage();
    let err = diff::select_pair(&storage, DiffSelection::Default).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("two snapshots"), "msg: {msg}");
}

#[test]
fn no_snapshots_at_all_is_a_friendly_error() {
    let td = TempDir::new().unwrap();
    let data_dir = td.path().join("data");
    let _ = Storage::init(&data_dir).unwrap(); // init but no snapshots
    let storage = Storage::open(&data_dir).unwrap();
    let err = diff::select_pair(&storage, DiffSelection::Default).unwrap_err();
    assert!(err.to_string().contains("no snapshots"));
}

#[test]
fn special_characters_in_directory_names_are_handled() {
    // `%` and `_` are LIKE metacharacters — escaping must not break them.
    let mut h = new_harness();
    h.write("100% done/f", &[1u8; 100]);
    h.write("under_score/f", &[2u8; 50]);
    h.snap();
    h.write("100% done/g", &[3u8; 25]);
    h.snap();

    let d = h.compute();
    assert_eq!(d.grew.len(), 1);
    let grow = entry_in(&d.grew, &h.abs_path("100% done")).unwrap();
    assert_eq!(grow.delta, 25);
    assert!(entry_in(&d.grew, &h.abs_path("under_score")).is_none());
}

#[test]
fn explicit_same_snapshot_diff_is_allowed_and_empty() {
    let mut h = new_harness();
    let id = h.snap();
    let storage = h.storage();
    let pair = diff::select_pair(&storage, DiffSelection::Explicit { from: id, to: id }).unwrap();
    let (b, a) = (pair.before, pair.after);
    assert_eq!(b.id, a.id);
    let d = diff::compute(&storage, &b, &a, &a.root_path).unwrap();
    assert_eq!(d.total_delta, 0);
    assert!(d.grew.is_empty() && d.shrank.is_empty());
}

// ─── overflow safety ─────────────────────────────────────────────────────

#[test]
fn signed_delta_never_overflows_for_u64_pairs() {
    use diskdrift::diff::signed_delta;
    assert_eq!(signed_delta(u64::MAX, 0), -(u64::MAX as i128));
    assert_eq!(signed_delta(0, u64::MAX), u64::MAX as i128);
    assert_eq!(signed_delta(5, 7), 2);
    assert_eq!(signed_delta(7, 5), -2);
}

#[test]
fn huge_sizes_are_diffed_without_overflow() {
    let mut h = new_harness();
    // Sizes far above the common range; kept at 256/512 MiB because Windows
    // `set_len` allocates real space (not sparse), and the intended overflow
    // guarantee is already unit-tested via signed_delta/i128 saturation.
    h.set_len("data/big.bin", 256 * 1024 * 1024);
    h.snap();
    h.set_len("data/big2.bin", 512 * 1024 * 1024);
    h.snap();

    let d = h.compute();
    assert_eq!(d.total_delta, 512 * 1024 * 1024);
    let data = entry_in(&d.grew, &h.abs_path("data")).unwrap();
    assert_eq!(data.delta, 512 * 1024 * 1024);
}

// ─── rendering (limit / all) ─────────────────────────────────────────────

#[test]
fn render_limit_and_all() {
    let mut h = new_harness();
    for i in 0..20 {
        h.write(&format!("d{i:02}/f"), &[1u8; 1]);
    }
    h.snap();
    for i in 0..20 {
        h.write(&format!("d{i:02}/g"), &vec![1u8; i + 1]);
    }
    h.snap();

    let d = h.compute();
    assert_eq!(d.grew.len(), 20);

    fn size_rows<'a>(text: &'a str, title: &str) -> Vec<&'a str> {
        let lines: Vec<&str> = text.lines().collect();
        // A group is only rendered when it has at least one entry.
        let Some(start) = lines.iter().position(|l| *l == title) else {
            return vec![];
        };
        let start = start + 2; // skip header + separator
        lines[start..]
            .iter()
            .take_while(|l| **l != "Shrank")
            .filter(|l| {
                let t = l.trim_start();
                t.starts_with('+') || t.starts_with('-')
            })
            .copied()
            .collect()
    }

    let limited = render_diff::render(&d, Some(3));
    assert!(limited.contains("... 17 more growing directories"));
    assert_eq!(size_rows(&limited, "Grew").len(), 3);
    assert_eq!(size_rows(&limited, "Shrank").len(), 0);

    let all = render_diff::render(&d, None);
    assert!(!all.contains("more growing"));
    assert_eq!(size_rows(&all, "Grew").len(), 20);
    assert_eq!(size_rows(&all, "Shrank").len(), 0);
}
