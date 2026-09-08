//! Deterministic demo fixture for README / docs / release smoke.
//!
//! Builds `demo-root/{docker,downloads,projects}`, snapshots "stage A",
//! mutates it deterministically, snapshots "stage B", then prints the same
//! human output the CLI renders (diff + inspect + history + top).
//!
//! Run: `cargo run --release --example demo -- <demo-root>` (default: target/demo)
//!
//! The printed session is what README.md quotes — never hand-edit README
//! numbers; regenerate them from here.

use std::fs;
use std::path::PathBuf;

use whybig::diff::{self, DiffSelection};
use whybig::history;
use whybig::inspect;
use whybig::output::{
    diff as render_diff, history as render_history, human, inspect as render_inspect,
    size as fmt_size, top as render_top,
};
use whybig::snapshot::service::SnapshotService;
use whybig::top::{self, TopMode};

fn write_bytes(path: &std::path::Path, n: usize) {
    if let Some(p) = path.parent() {
        fs::create_dir_all(p).unwrap();
    }
    fs::write(path, vec![0u8; n]).unwrap();
}

fn main() {
    let root = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/demo"));
    // data dir lives inside the demo root → WhyBig excludes it from scans.
    let data_dir = root.join(".whybig-data");
    let mut service = SnapshotService::new(data_dir.clone()).unwrap();

    // ── Stage A ──
    write_bytes(&root.join("docker/containers/a.bin"), 1_000);
    write_bytes(&root.join("docker/buildx/b.bin"), 500);
    write_bytes(&root.join("downloads/movie.bin"), 2_000);
    write_bytes(&root.join("projects/foo/code.txt"), 300);

    println!("$ whybig snapshot {}", root.display());
    let a = service.run_snapshot(&root, &mut |_| {}).unwrap();
    println!("{} files", human::number(a.file_count));
    println!("{} directories", human::number(a.dir_count));
    println!("{}", fmt_size::format(a.total_size));
    println!();
    println!(
        "Snapshot #{} saved in {}",
        a.snapshot_id,
        human::duration(a.elapsed)
    );

    // ── mutate (deterministic stage B) ──
    write_bytes(&root.join("docker/containers/big.bin"), 3_000);
    write_bytes(&root.join("downloads/archive.bin"), 1_500);
    fs::remove_dir_all(root.join("projects")).unwrap();

    println!();
    println!("$ whybig snapshot {}   # a few days later", root.display());
    let b = service.run_snapshot(&root, &mut |_| {}).unwrap();
    println!("{} files", human::number(b.file_count));
    println!("{} directories", human::number(b.dir_count));
    println!("{}", fmt_size::format(b.total_size));
    println!();
    println!(
        "Snapshot #{} saved in {}",
        b.snapshot_id,
        human::duration(b.elapsed)
    );

    // ── diff ──
    let storage = whybig::storage::Storage::open(&data_dir).unwrap();
    let pair = diff::select_pair(&storage, DiffSelection::Default).unwrap();
    let d = diff::compute_pair(&storage, &pair, &pair.after.root_path).unwrap();
    println!();
    println!("$ whybig diff");
    print!("{}", render_diff::render(&d, Some(10)));

    // ── inspect ──
    let docker = root.join("docker");
    let report = inspect::inspect(
        &storage,
        &pair.before,
        &pair.after,
        &pair.after.root_path,
        &docker,
    )
    .unwrap();
    println!("$ whybig inspect {}", docker.display());
    print!("{}", render_inspect::render(&report, None));

    // ── top ──
    let t = top::top(&storage, DiffSelection::Default, None, TopMode::Growth).unwrap();
    println!("$ whybig top");
    print!("{}", render_top::render(&t, Some(10)));

    // ── history —————
    let h = history::history(&storage, None, 20).unwrap();
    println!("$ whybig history");
    print!("{}", render_history::render(&h));
}
