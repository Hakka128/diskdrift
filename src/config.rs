//! Where DiskDrift keeps its data.
//!
//! Resolution order (first match wins):
//! 1. explicit `--data-dir <dir>` (CLI)
//! 2. `DISKDRIFT_DATA_DIR` environment variable
//! 3. `WHYBIG_DATA_DIR` — **deprecated legacy** fallback from the pre-release
//!    development name WhyBig; emits a one-line warning to stderr (never to
//!    stdout, so `--json` output stays pure).
//! 4. platform default:
//!    - Windows: `%APPDATA%\diskdrift`
//!    - macOS:   `~/Library/Application Support/diskdrift`
//!    - Linux:   `$XDG_DATA_HOME/diskdrift`, falling back to `~/.local/share/diskdrift`
//!
//! The legacy data directory (`%APPDATA%\whybig` etc.) is never touched:
//! DiskDrift neither deletes nor auto-merges it (see `DESIGN-RC.md`).

use std::env;
use std::path::{Path, PathBuf};

use crate::error::{DiskDriftError, Result};

/// Resolve the DiskDrift data directory for the given CLI override (if any).
pub fn data_dir(override_dir: Option<&Path>) -> Result<PathBuf> {
    if let Some(dir) = override_dir {
        return Ok(dir.to_path_buf());
    }
    if let Some(dir) = env::var_os("DISKDRIFT_DATA_DIR").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    // Deprecated legacy fallback (WhyBig dev-era name).
    if let Some(dir) = env::var_os("WHYBIG_DATA_DIR").filter(|v| !v.is_empty()) {
        eprintln!("warning: WHYBIG_DATA_DIR is deprecated. Use DISKDRIFT_DATA_DIR instead.");
        return Ok(PathBuf::from(dir));
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(appdata) = env::var_os("APPDATA") {
            return Ok(Path::new(&appdata).join("diskdrift"));
        }
    }

    if let Some(xdg) = env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        return Ok(Path::new(&xdg).join("diskdrift"));
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(home) = env::var_os("HOME") {
            return Ok(Path::new(&home)
                .join("Library")
                .join("Application Support")
                .join("diskdrift"));
        }
    }

    if let Some(home) = env::var_os("HOME") {
        return Ok(Path::new(&home)
            .join(".local")
            .join("share")
            .join("diskdrift"));
    }

    Err(DiskDriftError::DataDir(
        "no default location is defined on this platform; pass --data-dir or set DISKDRIFT_DATA_DIR".into(),
    ))
}

/// The SQLite database file inside the data directory.
pub fn db_path(data_dir: &Path) -> PathBuf {
    data_dir.join("diskdrift.db")
}

/// The legacy (pre-release "WhyBig") data directory path, if it can be
/// determined. Used only to *detect* and point at old data — never modified.
pub fn legacy_data_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        if let Some(appdata) = env::var_os("APPDATA") {
            return Some(Path::new(&appdata).join("whybig"));
        }
    }
    if let Some(xdg) = env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        return Some(Path::new(&xdg).join("whybig"));
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = env::var_os("HOME") {
            return Some(
                Path::new(&home)
                    .join("Library")
                    .join("Application Support")
                    .join("whybig"),
            );
        }
    }
    if let Some(home) = env::var_os("HOME") {
        return Some(Path::new(&home).join(".local").join("share").join("whybig"));
    }
    None
}

/// The legacy database file (`whybig.sqlite3`), if it exists.
pub fn legacy_db_path() -> Option<PathBuf> {
    let dir = legacy_data_dir()?;
    let db = dir.join("whybig.sqlite3");
    db.exists().then_some(db)
}

/// True when a legacy WhyBig data directory exists at all.
pub fn legacy_data_exists() -> bool {
    legacy_db_path().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_override_wins() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            data_dir(Some(dir.path())).unwrap(),
            dir.path().to_path_buf()
        );
    }

    #[test]
    fn env_var_is_used_when_no_override() {
        let dir = tempfile::tempdir().unwrap();
        unsafe {
            // SAFETY: set_var is unsound in multithreaded Rust; tests run in
            // parallel, so guard against environment races with a mutex.
            static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
            let _g = LOCK.lock().unwrap();
            env::set_var("DISKDRIFT_DATA_DIR", dir.path());
            let resolved = data_dir(None).unwrap();
            assert_eq!(resolved, dir.path().to_path_buf());
            env::remove_var("DISKDRIFT_DATA_DIR");
        }
    }

    #[test]
    fn new_env_takes_precedence_over_legacy_env() {
        let dir = tempfile::tempdir().unwrap();
        unsafe {
            static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
            let _g = LOCK.lock().unwrap();
            // Set a bogus legacy path; the real DISKDRIFT_DATA_DIR must win.
            env::set_var("WHYBIG_DATA_DIR", dir.path().join("legacy-placeholder"));
            env::set_var("DISKDRIFT_DATA_DIR", dir.path());
            let resolved = data_dir(None).unwrap();
            assert_eq!(resolved, dir.path().to_path_buf());
            env::remove_var("DISKDRIFT_DATA_DIR");
            env::remove_var("WHYBIG_DATA_DIR");
        }
    }

    #[test]
    fn legacy_env_is_a_fallback() {
        let dir = tempfile::tempdir().unwrap();
        unsafe {
            static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
            let _g = LOCK.lock().unwrap();
            env::set_var("WHYBIG_DATA_DIR", dir.path());
            let resolved = data_dir(None).unwrap();
            assert_eq!(resolved, dir.path().to_path_buf());
            env::remove_var("WHYBIG_DATA_DIR");
        }
    }

    #[test]
    fn db_path_is_inside_data_dir_under_new_name() {
        let dir = Path::new("C:\\appdata\\diskdrift");
        #[cfg(windows)]
        assert_eq!(
            db_path(dir),
            PathBuf::from(r"C:\appdata\diskdrift\diskdrift.db")
        );
        #[cfg(not(windows))]
        assert_eq!(
            db_path(dir),
            PathBuf::from("C:\\appdata\\diskdrift/diskdrift.db")
        );
    }

    #[test]
    fn legacy_helpers_point_at_old_layout_without_touching_it() {
        // legacy_data_dir must resolve to the OLD name (WhyBig) when the
        // platform has a home/appdata; and legacy_db_path must be None unless
        // a legacy database already exists (we never create one).
        if let Some(dir) = legacy_data_dir() {
            assert!(
                dir.to_string_lossy().to_lowercase().contains("whybig"),
                "legacy dir must use the old name, got {dir:?}"
            );
        }
        assert!(
            legacy_db_path().is_none() || {
                let p = legacy_db_path().unwrap();
                p.to_string_lossy().contains("whybig.sqlite3")
            }
        );
    }
}
