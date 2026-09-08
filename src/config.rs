//! Where WhyBig keeps its data.
//!
//! Resolution order (first match wins):
//! 1. explicit `--data-dir <dir>` (CLI)
//! 2. `WHYBIG_DATA_DIR` environment variable
//! 3. platform default:
//!    - Windows: `%APPDATA%\whybig`
//!    - macOS:   `~/Library/Application Support/whybig`
//!    - Linux:   `$XDG_DATA_HOME/whybig`, falling back to `~/.local/share/whybig`
//!
//! There is intentionally **no config file** in Milestone 1: all durable state
//! lives in SQLite, and the data-directory choice is explicit via CLI/env.
//! This avoids a `toml`+serde dependency that nothing would read yet.

use std::env;
use std::path::{Path, PathBuf};

use crate::error::{Result, WhyBigError};

/// Resolve the WhyBig data directory for the given CLI override (if any).
pub fn data_dir(override_dir: Option<&Path>) -> Result<PathBuf> {
    if let Some(dir) = override_dir {
        return Ok(dir.to_path_buf());
    }
    if let Some(dir) = env::var_os("WHYBIG_DATA_DIR").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(dir));
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(appdata) = env::var_os("APPDATA") {
            return Ok(Path::new(&appdata).join("whybig"));
        }
    }

    if let Some(xdg) = env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        return Ok(Path::new(&xdg).join("whybig"));
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(home) = env::var_os("HOME") {
            return Ok(Path::new(&home)
                .join("Library")
                .join("Application Support")
                .join("whybig"));
        }
    }

    if let Some(home) = env::var_os("HOME") {
        return Ok(Path::new(&home).join(".local").join("share").join("whybig"));
    }

    Err(WhyBigError::DataDir(
        "no default location is defined on this platform; pass --data-dir or set WHYBIG_DATA_DIR"
            .into(),
    ))
}

/// The SQLite database file inside the data directory.
pub fn db_path(data_dir: &Path) -> PathBuf {
    data_dir.join("whybig.sqlite3")
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
            env::set_var("WHYBIG_DATA_DIR", dir.path());
            let resolved = data_dir(None).unwrap();
            assert_eq!(resolved, dir.path().to_path_buf());
            env::remove_var("WHYBIG_DATA_DIR");
        }
    }

    #[test]
    fn db_path_is_inside_data_dir() {
        let dir = Path::new("C:\\appdata\\whybig");
        #[cfg(windows)]
        assert_eq!(
            db_path(dir),
            PathBuf::from(r"C:\appdata\whybig\whybig.sqlite3")
        );
        #[cfg(not(windows))]
        assert_eq!(
            db_path(dir),
            PathBuf::from("C:\\appdata\\whybig/whybig.sqlite3")
        );
    }
}
