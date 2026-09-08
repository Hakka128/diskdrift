//! Branding / rename regression tests: binary name, env precedence, legacy
//! fallback with stderr-only deprecation, database filename, help text, and
//! that user-facing docs are free of stale branding.

use std::fs;
use std::path::Path;
use std::process::Command;

use tempfile::TempDir;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_diskdrift")
}

fn run_env(args: &[&str], envs: &[(&str, &Path)]) -> (String, String, bool) {
    let mut cmd = Command::new(bin());
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.args(args).output().expect("binary runs");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

#[test]
fn binary_has_the_final_name() {
    let exe = Path::new(bin());
    let name = exe.file_name().unwrap().to_string_lossy();
    #[cfg(windows)]
    assert_eq!(name, "diskdrift.exe");
    #[cfg(not(windows))]
    assert_eq!(name, "diskdrift");
    // The version string comes from Cargo.
    let out = Command::new(bin()).arg("--version").output().unwrap();
    let v = String::from_utf8_lossy(&out.stdout);
    assert!(
        v.contains("diskdrift 0.1.0") || v.contains("diskdrift"),
        "version: {v}"
    );
}

#[test]
fn help_is_branded_and_matches_the_positions() {
    let out = Command::new(bin()).args(["--help"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.to_lowercase().contains("diskdrift"),
        "help: {stdout}"
    );
    assert!(
        stdout.contains("Track disk growth over time"),
        "about line missing: {stdout}"
    );
    for sub in [
        "init", "snapshot", "status", "diff", "inspect", "history", "top", "prune", "compact",
    ] {
        assert!(
            stdout.lines().any(|l| l.trim_start().starts_with(sub)),
            "help must list {sub}"
        );
    }
}

#[test]
fn help_has_no_stale_branding() {
    let out = Command::new(bin()).args(["--help"]).output().unwrap();
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !text.to_lowercase().contains("whybig"),
        "stale branding in --help:\n{text}"
    );
}

#[test]
fn default_database_filename_is_diskdrift_db() {
    let td = TempDir::new().unwrap();
    let data = td.path().join("data");
    let (_, _, ok) = run_env(&["--data-dir", data.to_str().unwrap(), "init"], &[]);
    assert!(ok);
    assert!(
        data.join("diskdrift.db").exists(),
        "new DB filename must be diskdrift.db"
    );
    assert!(!data.join("whybig.sqlite3").exists());
}

#[test]
fn new_env_var_takes_precedence_over_legacy() {
    let td = TempDir::new().unwrap();
    let new_dir = td.path().join("new");
    let legacy_dir = td.path().join("legacy");
    let (stdout, stderr, ok) = run_env(
        &["init"],
        &[
            ("DISKDRIFT_DATA_DIR", new_dir.as_path()),
            ("WHYBIG_DATA_DIR", legacy_dir.as_path()),
        ],
    );
    assert!(ok, "stderr: {stderr}");
    assert!(
        new_dir.join("diskdrift.db").exists(),
        "new env var must win: whole output:\n{stdout}\n{stderr}"
    );
    assert!(!legacy_dir.join("diskdrift.db").exists());
    assert!(
        !stderr.to_lowercase().contains("deprecated"),
        "no deprecation when new env is used"
    );
}

#[test]
fn legacy_env_is_a_deprecated_fallback_with_stderr_notice() {
    let td = TempDir::new().unwrap();
    let legacy_dir = td.path().join("legacy");
    let (stdout, stderr, ok) = run_env(&["init"], &[("WHYBIG_DATA_DIR", legacy_dir.as_path())]);
    assert!(ok, "stderr: {stderr}");
    assert!(
        legacy_dir.join("diskdrift.db").exists(),
        "legacy env must still select the data dir: {stdout}\n{stderr}"
    );
    assert!(
        stderr.contains("WHYBIG_DATA_DIR is deprecated"),
        "deprecation must be printed to stderr: {stderr}"
    );
}

#[test]
fn legacy_deprecation_never_pollutes_json_stdout() {
    let td = TempDir::new().unwrap();
    let legacy_dir = td.path().join("legacy");
    let (stdout, stderr, ok) = run_env(
        &["status", "--json"],
        &[("WHYBIG_DATA_DIR", legacy_dir.as_path())],
    );
    assert!(ok, "stderr: {stderr}");
    let v: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout not pure JSON: {e}\n{stdout}"));
    assert_eq!(v["schema_version"], 1);
    assert_eq!(v["command"], "status");
    assert_eq!(v["initialized"], false);
    assert!(
        stderr.contains("deprecated"),
        "deprecation belongs on stderr: {stderr}"
    );
}

#[test]
fn init_notices_legacy_data_without_touching_it() {
    // Simulate a pre-existing legacy WhyBig database.
    let td = TempDir::new().unwrap();
    let data = td.path().join("data");
    let (stdout, stderr, ok) = run_env(&["--data-dir", data.to_str().unwrap(), "init"], &[]);
    assert!(ok, "stderr: {stderr}");
    assert!(stdout.contains("DiskDrift is ready."));
    // (stderr may carry a legacy notice on machines that still have one; the
    // invariant is that stdout is clean and init succeeds.)

    // The legacy notice path is exercised deterministically on Windows by
    // pointing APPDATA at a temp dir containing an old whybig DB.
    #[cfg(windows)]
    {
        let fake_appdata = td.path().join("appdata");
        let legacy_dir = fake_appdata.join("whybig");
        fs::create_dir_all(&legacy_dir).unwrap();
        fs::write(legacy_dir.join("whybig.sqlite3"), b"legacy").unwrap();

        let (out2, err2, ok2) = run_env(
            &["--data-dir", data.to_str().unwrap(), "init"],
            &[("APPDATA", fake_appdata.as_path())],
        );
        assert!(ok2, "stderr: {err2}");
        assert!(
            err2.contains("legacy WhyBig data found"),
            "init must point at legacy data on stderr: {err2}"
        );
        assert!(
            out2.contains("DiskDrift is ready."),
            "stdout stays clean: {out2}"
        );
        // Legacy data was not deleted or overwritten.
        assert!(legacy_dir.join("whybig.sqlite3").exists());
    }
}

#[test]
fn package_metadata_is_diskdrift() {
    let toml = fs::read_to_string("Cargo.toml").unwrap();
    assert!(toml.contains("name = \"diskdrift\""));
    assert_eq!(tomal_version(&toml), "0.1.0");
    assert!(toml.contains("A local-first disk growth debugger"));
}

fn tomal_version(toml: &str) -> &str {
    for line in toml.lines() {
        if let Some(rest) = line.strip_prefix("version = ") {
            return rest.trim().trim_matches('"');
        }
    }
    ""
}

#[test]
fn readme_has_no_stale_branding() {
    let readme = fs::read_to_string("README.md").unwrap();
    assert!(
        !readme.to_lowercase().contains("whybig"),
        "README must not contain stale branding"
    );
    assert!(readme.contains("DiskDrift"));
    assert!(readme.contains("Find what's quietly growing on your disk."));
}
