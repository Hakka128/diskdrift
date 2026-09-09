//! `top` service tests — ranking must reuse the diff engine's attribution.

mod common;

use common::{new_harness, Harness};
use diskdrift::diff::{self, DiffSelection};
use diskdrift::output::top as render_top;
use diskdrift::top::{self, TopMode};

fn top_of(h: &Harness, raw: Option<&str>, mode: TopMode) -> diskdrift::top::TopReport {
    let storage = h.storage();
    top::top(
        &storage,
        DiffSelection::Default,
        raw.map(std::path::Path::new),
        mode,
    )
    .unwrap()
}

/// Assert `top` matches `diff` for the same pair (single source of truth).
fn matches_diff(h: &Harness, report: &diskdrift::top::TopReport, mode: TopMode) {
    let storage = h.storage();
    let pair = diff::select_pair(&storage, DiffSelection::Default).unwrap();
    // `report.scope` keeps the user/display spelling; the lower-level diff
    // query needs the stored-canonical lookup spelling (what `top::top`
    // canonicalizes internally) so the comparison side resolves the same rows.
    let lookup_scope = common::stored_path_string(std::path::Path::new(&report.scope));
    let d = diff::compute(&storage, &pair.before, &pair.after, &lookup_scope).unwrap();
    let (ours, theirs) = match mode {
        TopMode::Growth => (&report.entries, &d.grew),
        TopMode::Shrink => (&report.entries, &d.shrank),
    };
    assert_eq!(ours.len(), theirs.len());
    for (o, t) in ours.iter().zip(theirs) {
        assert_eq!(o.path, t.path);
        assert_eq!(o.delta, t.delta);
    }
}

#[test]
fn largest_growth_comes_first_and_matches_diff() {
    let mut h = new_harness();
    h.write("small/f", &[1u8; 10]);
    h.write("middle/f", &[1u8; 10]);
    h.write("big/f", &[1u8; 10]);
    h.snap();
    h.write("big/g", &[1u8; 300]);
    h.write("middle/g", &[1u8; 200]);
    h.write("small/g", &[1u8; 100]);
    h.snap();

    let r = top_of(&h, None, TopMode::Growth);
    let names: Vec<String> = r
        .entries
        .iter()
        .map(|e| {
            std::path::Path::new(&e.path)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(
        names,
        vec!["big".to_string(), "middle".to_string(), "small".to_string()]
    );
    matches_diff(&h, &r, TopMode::Growth);
}

#[test]
fn shrink_ranking_sorts_by_absolute_delta() {
    let mut h = new_harness();
    h.write("small/f", &[1u8; 100]);
    h.write("big/f", &[1u8; 1000]);
    h.write("med/f", &[1u8; 400]);
    h.snap();
    h.remove("big");
    h.remove("med");
    h.remove("small");
    h.snap();

    let r = top_of(&h, None, TopMode::Shrink);
    let names: Vec<String> = r
        .entries
        .iter()
        .map(|e| {
            std::path::Path::new(&e.path)
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
    matches_diff(&h, &r, TopMode::Shrink);
}

#[test]
fn render_applies_limit() {
    let mut h = new_harness();
    for i in 0..8 {
        h.write(&format!("d{i}/f"), &[1u8; 1]);
    }
    h.snap();
    for i in 0..8 {
        h.write(&format!("d{i}/g"), &vec![1u8; i + 1]);
    }
    h.snap();

    let r = top_of(&h, None, TopMode::Growth);
    assert_eq!(r.entries.len(), 8);
    let text = render_top::render(&r, Some(2));
    assert!(text.contains("Top Disk Growth"));
    let ok = text
        .lines()
        .filter(|l| l.trim_start().starts_with('+'))
        .count();
    assert_eq!(ok, 2);
    assert!(text.contains("... 6 more"));
}

#[test]
fn all_zero_yields_empty_ranking() {
    let mut h = new_harness();
    h.write("a/f", &[1u8; 10]);
    h.snap();
    h.snap();
    let r = top_of(&h, None, TopMode::Growth);
    assert!(r.entries.is_empty());
}

#[test]
fn added_directory_appears_in_growth() {
    let mut h = new_harness();
    h.write("a/f", &[1u8; 1]);
    h.snap();
    h.write("fresh/f", &[1u8; 500]);
    h.snap();

    let r = top_of(&h, None, TopMode::Growth);
    let e = r
        .entries
        .iter()
        .find(|e| std::path::Path::new(&e.path).file_name().unwrap() == "fresh")
        .unwrap();
    assert_eq!(e.state, diskdrift::diff::DiffState::Added);
}

#[test]
fn removed_directory_appears_in_shrink() {
    let mut h = new_harness();
    h.write("dead/f", &[1u8; 333]);
    h.snap();
    h.remove("dead");
    h.snap();

    let r = top_of(&h, None, TopMode::Shrink);
    let e = r
        .entries
        .iter()
        .find(|e| std::path::Path::new(&e.path).file_name().unwrap() == "dead")
        .unwrap();
    assert_eq!(e.state, diskdrift::diff::DiffState::Removed);
    assert_eq!(e.delta, -333);
}

#[test]
fn nested_growth_collapses_to_top_level() {
    let mut h = new_harness();
    h.write("parent/child/deep/f", &[1u8; 1]);
    h.snap();
    h.write("parent/child/deep/g", &[1u8; 250]);
    h.snap();

    let r = top_of(&h, None, TopMode::Growth);
    assert_eq!(r.entries.len(), 1);
    assert_eq!(r.entries[0].delta, 250);
    assert!(
        !r.entries[0].path.contains("child"),
        "deep growth must collapse to the top-level directory"
    );
}

#[test]
fn scoped_top_path_ranks_that_director_ys_children() {
    let mut h = new_harness();
    h.write("app/one/f", &[1u8; 1]);
    h.write("app/two/f", &[1u8; 1]);
    h.write("other/x", &[1u8; 1]);
    h.snap();
    h.write("app/one/g", &[1u8; 120]);
    h.write("app/two/g", &[1u8; 60]);
    h.snap();

    let r = top_of(&h, Some(h.abs("app").to_str().unwrap()), TopMode::Growth);
    let names: Vec<String> = r
        .entries
        .iter()
        .map(|e| {
            std::path::Path::new(&e.path)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(names, vec!["one".to_string(), "two".to_string()]);
    // matches what inspect contributors would report (same diff scope)
    matches_diff(&h, &r, TopMode::Growth);
}

#[test]
fn multiple_roots_default_selects_latest() {
    let td = tempfile::TempDir::new().unwrap();
    let root1 = td.path().join("r1");
    let root2 = td.path().join("r2");
    let data_dir = td.path().join("data");
    fn snap_root(
        service: &mut diskdrift::snapshot::service::SnapshotService,
        root: &std::path::Path,
    ) -> i64 {
        service.run_snapshot(root, &mut |_| {}).unwrap().snapshot_id
    }
    let mut service = diskdrift::snapshot::service::SnapshotService::new(data_dir.clone()).unwrap();
    std::fs::create_dir_all(&root1).unwrap();
    std::fs::create_dir_all(&root2).unwrap();
    snap_root(&mut service, &root1);
    snap_root(&mut service, &root2);
    // Growth must live in a *directory* child to produce a diff entry
    // (files directly in the root only move the totals).
    std::fs::create_dir_all(root2.join("grow")).unwrap();
    std::fs::write(root2.join("grow").join("f"), vec![1u8; 700]).unwrap();
    snap_root(&mut service, &root2);

    let storage = diskdrift::storage::Storage::open(&data_dir).unwrap();
    let report = top::top(&storage, DiffSelection::Default, None, TopMode::Growth).unwrap();
    // Only entries of root2 (the latest root) appear.
    // (Strip the Windows \\?\ prefix from canonicalize, as the service does.)
    let canon = plain_path(&std::fs::canonicalize(&root2).unwrap());
    assert_eq!(report.root, canon);
    assert_eq!(report.entries.len(), 1);
    assert!(report.entries[0].path.starts_with(&report.root));
}

/// Strip the Windows verbatim prefix for test comparisons.
fn plain_path(p: &std::path::Path) -> String {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        s.into_owned()
    }
}
