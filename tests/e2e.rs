//! End-to-end tests that run the compiled `whybig` binary the way a user
//! would, against a committed fixture tree and temp data directories.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

fn whybig_bin() -> &'static str {
    env!("CARGO_BIN_EXE_whybig")
}

fn fixture_tree() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tree")
}

struct Run {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}

fn run(args: &[&str]) -> Run {
    let out = Command::new(whybig_bin())
        .args(args)
        .output()
        .expect("binary runs");
    Run {
        status: out.status,
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

#[test]
fn init_is_idempotent_via_cli() {
    let td = TempDir::new().unwrap();
    let data = td.path().join("data");
    let data_s = data.to_str().unwrap();

    let r1 = run(&["--data-dir", data_s, "init"]);
    assert!(r1.status.success(), "stderr: {}", r1.stderr);
    assert!(
        r1.stdout.contains("WhyBig is ready."),
        "stdout: {}",
        r1.stdout
    );

    // Second run must succeed and must not break the existing database.
    let r2 = run(&["--data-dir", data_s, "init"]);
    assert!(r2.status.success(), "stderr: {}", r2.stderr);
    assert!(data.join("whybig.sqlite3").exists());
}

#[test]
fn snapshot_then_status_end_to_end() {
    let td = TempDir::new().unwrap();
    let data = td.path().join("data");
    let data_s = data.to_str().unwrap();
    let tree_s = fixture_tree().to_str().unwrap().to_string();

    let snap = run(&["--data-dir", data_s, "snapshot", &tree_s]);
    assert!(snap.status.success(), "stderr: {}", snap.stderr);
    assert!(snap.stdout.contains("files"), "stdout: {}", snap.stdout);
    assert!(
        snap.stdout.contains("Snapshot #1 saved in"),
        "stdout: {}",
        snap.stdout
    );

    let st = run(&["--data-dir", data_s, "status"]);
    assert!(st.status.success(), "stderr: {}", st.stderr);
    assert!(st.stdout.contains("Snapshots:"), "stdout: {}", st.stdout);
    assert!(st.stdout.contains("Latest"), "stdout: {}", st.stdout);
    assert!(st.stdout.contains("root:"), "stdout: {}", st.stdout);
}

#[test]
fn snapshot_records_a_second_snapshot_with_a_new_id() {
    let td = TempDir::new().unwrap();
    let data = td.path().join("data");
    let data_s = data.to_str().unwrap();
    let tree_s = fixture_tree().to_str().unwrap().to_string();

    let r1 = run(&["--data-dir", data_s, "snapshot", &tree_s]);
    assert!(r1.status.success());
    let r2 = run(&["--data-dir", data_s, "snapshot", &tree_s]);
    assert!(r2.status.success());
    assert!(
        r2.stdout.contains("Snapshot #2 saved in"),
        "stdout: {}",
        r2.stdout
    );
}

#[test]
fn status_before_init_is_friendly_not_a_crash() {
    let td = TempDir::new().unwrap();
    let data = td.path().join("data");
    let r = run(&["--data-dir", data.to_str().unwrap(), "status"]);
    assert!(r.status.success(), "stderr: {}", r.stderr);
    assert!(r.stdout.contains("not initialized"), "stdout: {}", r.stdout);
}

#[test]
fn snapshot_of_a_missing_path_fails_cleanly() {
    let td = TempDir::new().unwrap();
    let data = td.path().join("data");
    let missing = td.path().join("no-such-dir");
    let r = run(&[
        "--data-dir",
        data.to_str().unwrap(),
        "snapshot",
        missing.to_str().unwrap(),
    ]);
    assert!(!r.status.success());
    // stderr begins with "Scanning …" then the user-facing error line.
    assert!(r.stderr.contains("error:"), "stderr: {}", r.stderr);
}

#[test]
fn no_subcommand_shows_usage() {
    let r = run(&["--data-dir", "unused"]);
    // clap requires a subcommand → prints help/usage to stderr, non-zero exit.
    assert!(!r.status.success());
    assert!(
        r.stderr.contains("Usage") || r.stderr.contains("USAGE"),
        "stderr: {}",
        r.stderr
    );
}

// ─── Milestone 2: diff / inspect end-to-end ──────────────────────────────

#[test]
fn diff_and_inspect_end_to_end() {
    let td = TempDir::new().unwrap();
    let root = td.path().join("root");
    fs::create_dir_all(root.join("docker").join("containers")).unwrap();
    fs::create_dir_all(root.join("downloads")).unwrap();
    fs::write(root.join("docker/containers/a.bin"), vec![1u8; 1000]).unwrap();
    fs::write(root.join("downloads/b.bin"), vec![2u8; 500]).unwrap();
    let data = td.path().join("data");
    let data_s = data.to_str().unwrap().to_string();
    let root_s = root.to_str().unwrap().to_string();

    // Snapshot A
    let a = run(&["--data-dir", &data_s, "snapshot", &root_s]);
    assert!(a.status.success(), "stderr: {}", a.stderr);

    // Mutate: docker containers grow by 3000; downloads shrink by 400.
    fs::write(root.join("docker/containers/big.bin"), vec![1u8; 3000]).unwrap();
    fs::remove_file(root.join("downloads/b.bin")).unwrap();
    fs::write(root.join("downloads/new.bin"), vec![2u8; 100]).unwrap();

    // Snapshot B
    let b = run(&["--data-dir", &data_s, "snapshot", &root_s]);
    assert!(b.status.success(), "stderr: {}", b.stderr);

    // diff: top-level attribution only
    let diff = run(&["--data-dir", &data_s, "diff"]);
    assert!(diff.status.success(), "stderr: {}", diff.stderr);
    assert!(
        diff.stdout.contains("Disk Growth"),
        "stdout: {}",
        diff.stdout
    );
    assert!(diff.stdout.contains("Grew"), "stdout: {}", diff.stdout);
    assert!(
        diff.stdout.contains(" Shrank") || diff.stdout.contains("Shrank"),
        "stdout: {}",
        diff.stdout
    );

    // docker grew (direct child of root)…
    let grew_line = diff
        .stdout
        .lines()
        .find(|l| l.contains("docker") && l.trim_start().starts_with('+'))
        .expect("docker appears in Grew");
    assert!(grew_line.contains("docker"), "grew line: {grew_line}");
    // …and downloads shrank.
    let shrank_line = diff
        .stdout
        .lines()
        .find(|l| l.contains("downloads") && l.trim_start().starts_with('-'))
        .expect("downloads appears in Shrank");
    assert!(
        shrank_line.contains("downloads"),
        "shrank line: {shrank_line}"
    );

    // inspect drills into docker: containers is the contributor.
    let docker_path = root.join("docker");
    let ins = run(&[
        "--data-dir",
        &data_s,
        "inspect",
        docker_path.to_str().unwrap(),
    ]);
    assert!(ins.status.success(), "stderr: {}", ins.stderr);
    assert!(ins.stdout.contains("containers/"), "stdout: {}", ins.stdout);
    let contributors_line = ins
        .stdout
        .lines()
        .find(|l| l.contains("containers/"))
        .expect("contributors has containers");
    assert!(
        contributors_line.trim_start().starts_with('+'),
        "containers grew: {contributors_line}"
    );
}

#[test]
fn diff_with_fewer_than_two_snapshots_errors_via_cli() {
    let td = TempDir::new().unwrap();
    let root = td.path().join("root");
    fs::create_dir_all(&root).unwrap();
    let data = td.path().join("data");
    run(&[
        "--data-dir",
        data.to_str().unwrap(),
        "snapshot",
        root.to_str().unwrap(),
    ]);

    let r = run(&["--data-dir", data.to_str().unwrap(), "diff"]);
    assert!(!r.status.success());
    assert!(
        r.stderr.contains("two snapshots") || r.stderr.contains("error:"),
        "stderr: {}",
        r.stderr
    );
}

#[test]
fn diff_on_uninitialized_db_errors_friendly_via_cli() {
    let td = TempDir::new().unwrap();
    let data = td.path().join("data");
    let r = run(&["--data-dir", data.to_str().unwrap(), "diff"]);
    assert!(!r.status.success());
    assert!(r.stderr.contains("not initialized"), "stderr: {}", r.stderr);
}

#[test]
fn inspect_outside_root_errors_via_cli() {
    let td = TempDir::new().unwrap();
    let root = td.path().join("root");
    fs::create_dir_all(&root).unwrap();
    let data = td.path().join("data");
    run(&[
        "--data-dir",
        data.to_str().unwrap(),
        "snapshot",
        root.to_str().unwrap(),
    ]);
    run(&[
        "--data-dir",
        data.to_str().unwrap(),
        "snapshot",
        root.to_str().unwrap(),
    ]);

    let outside = td.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    let r = run(&[
        "--data-dir",
        data.to_str().unwrap(),
        "inspect",
        outside.to_str().unwrap(),
    ]);
    assert!(!r.status.success());
    assert!(
        r.stderr.contains("outside tracked root"),
        "stderr: {}",
        r.stderr
    );
}
