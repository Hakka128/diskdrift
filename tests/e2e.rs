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

// ─── Milestone 3: history / top / JSON ───────────────────────────────────

#[test]
fn history_top_and_json_end_to_end() {
    let td = TempDir::new().unwrap();
    let root = td.path().join("root");
    let root_s = root.to_str().unwrap().to_string();
    let data = td.path().join("data");
    let data_s = data.to_str().unwrap().to_string();
    let app = root.join("app");
    let app_s = app.to_str().unwrap().to_string();

    fs::create_dir_all(&app).unwrap();
    fs::write(app.join("seed.txt"), vec![5u8; 10]).unwrap();
    for i in 1..=3u64 {
        fs::write(
            app.join(format!("mod{i}.bin")),
            vec![1u8; i as usize * 1000],
        )
        .unwrap();
        let r = run(&["--data-dir", &data_s, "snapshot", &root_s]);
        assert!(r.status.success(), "snapshot {i}: {}", r.stderr);
    }

    // history (root) — ascending trend, three points
    let hist = run(&["--data-dir", &data_s, "history"]);
    assert!(hist.status.success(), "stderr: {}", hist.stderr);
    assert!(hist.stdout.contains("Disk History"), "{}", hist.stdout);

    // history <path> — target header present
    let hist_p = run(&["--data-dir", &data_s, "history", &app_s]);
    assert!(hist_p.status.success(), "stderr: {}", hist_p.stderr);
    assert_eq!(
        hist_p.stdout.lines().next().map(str::trim_end),
        Some(app_s.as_str()),
        "history header is the target path"
    );

    // top
    let top = run(&["--data-dir", &data_s, "top"]);
    assert!(top.status.success(), "stderr: {}", top.stderr);
    assert!(top.stdout.contains("Top Disk Growth"), "{}", top.stdout);
    assert!(
        top.stdout.contains("app"),
        "top lists direct children: {}",
        top.stdout
    );

    let top_shrink = run(&["--data-dir", &data_s, "top", "--shrink"]);
    assert!(top_shrink.status.success(), "stderr: {}", top_shrink.stderr);
    assert!(
        top_shrink.stdout.contains("Disk Space Freed"),
        "{}",
        top_shrink.stdout
    );

    // top <path>
    let top_p = run(&["--data-dir", &data_s, "top", &app_s]);
    assert!(top_p.status.success(), "stderr: {}", top_p.stderr);

    // JSON: stdout must be exactly one JSON document with no ANSI/progress.
    let json_cases: Vec<Vec<String>> = vec![
        vec!["diff".into(), "--json".into()],
        vec!["history".into(), "--json".into()],
        vec!["top".into(), "--json".into()],
        vec!["status".into(), "--json".into()],
        vec!["inspect".into(), app_s.clone(), "--json".into()],
    ];
    for args in json_cases {
        let command = args[0].clone();
        let mut full: Vec<&str> = vec!["--data-dir", &data_s];
        full.extend(args.iter().map(|s| s.as_str()));
        let r = run(&full);
        assert!(r.status.success(), "{command} json: {}", r.stderr);
        assert!(
            !r.stdout.contains('\u{1b}'),
            "{command} json must not contain ANSI escapes"
        );
        let v: serde_json::Value = serde_json::from_str(&r.stdout)
            .unwrap_or_else(|e| panic!("{command} stdout not pure JSON: {e}\n{}", r.stdout));
        assert_eq!(v["schema_version"], 1, "{command}");
        assert_eq!(v["command"], command, "{command}");
    }
}

#[test]
fn history_and_top_accept_relative_paths_from_cwd() {
    let td = TempDir::new().unwrap();
    let root = td.path().join("root");
    let data = td.path().join("data");
    let data_s = data.to_str().unwrap().to_string();
    let root_s = root.to_str().unwrap().to_string();
    fs::create_dir_all(root.join("docker")).unwrap();
    fs::write(root.join("docker/f"), vec![1u8; 5]).unwrap();

    for _ in 0..2 {
        let r = run(&["--data-dir", &data_s, "snapshot", &root_s]);
        assert!(r.status.success(), "{}", r.stderr);
    }

    // Run with cwd inside the root so a relative path resolves under it.
    let run_in = |args: &[&str]| {
        let out = Command::new(whybig_bin())
            .current_dir(&root)
            .args(args)
            .output()
            .expect("binary runs");
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        )
    };
    let args = ["--data-dir", &data_s, "history", "docker"];
    let (ok, out) = run_in(&args);
    assert!(ok, "relative history: {out}");
    assert!(out.contains("docker"));

    let args = ["--data-dir", &data_s, "top", "docker"];
    let (ok2, out2) = run_in(&args);
    assert!(ok2, "relative top: {out2}");
}
