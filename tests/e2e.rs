//! End-to-end tests that run the compiled `whybig` binary the way a user
//! would, against a committed fixture tree and temp data directories.

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
