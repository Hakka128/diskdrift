//! Component-level path utilities.
//!
//! DiskDrift never decides hierarchy from raw strings (`starts_with("/a/b")` would
//! collide with `/a/bar2`). Everything here compares path **components**, which
//! also handles platform separators (Windows stored keys use `\`, which
//! `std::path` parses on both `\` and `/`), drive letters, UNC prefixes and
//! trailing separators.

use std::path::{Component, Path, PathBuf};

/// Is `child` inside (or equal to) `ancestor`? Component-level, with
/// case-insensitive comparison on Windows.
pub fn is_under(child: &Path, ancestor: &Path) -> bool {
    let mut child_components = child.components();
    for ac in ancestor.components() {
        match child_components.next() {
            Some(cc) if comp_eq(&cc, &ac) => {}
            _ => return false,
        }
    }
    true
}

fn canonicalize_scope_path(path: &Path) -> Option<PathBuf> {
    let mut probe = path;
    let mut missing = Vec::new();

    loop {
        if let Ok(base) = std::fs::canonicalize(probe) {
            let mut resolved = normalize_abs(base);

            for component in missing.iter().rev() {
                resolved.push(component);
            }

            return Some(normalize_lexical(&resolved));
        }

        let name = probe.file_name()?.to_os_string();
        missing.push(name);
        probe = probe.parent()?;
    }
}

/// Is `child` inside (or equal to) `ancestor`, allowing filesystem-canonical
/// equivalents such as macOS `/var/...` and `/private/var/...`.
///
/// For paths that no longer exist, canonicalize the nearest existing ancestor
/// and append the missing suffix before comparing.
pub fn is_under_scope(child: &Path, ancestor: &Path) -> bool {
    if is_under(child, ancestor) {
        return true;
    }

    let canonical_child = canonicalize_scope_path(child);
    let canonical_ancestor = canonicalize_scope_path(ancestor);

    match (canonical_child, canonical_ancestor) {
        (Some(child), Some(ancestor)) => is_under(&child, &ancestor),
        _ => false,
    }
}

/// Are two paths the same directory (component-equal)?
pub fn paths_equal(a: &Path, b: &Path) -> bool {
    let ac = a.components().collect::<Vec<_>>();
    let bc = b.components().collect::<Vec<_>>();
    ac.len() == bc.len() && ac.iter().zip(&bc).all(|(x, y)| comp_eq(x, y))
}

/// Is `child` a **direct** child of `parent` (exactly one more component)?
pub fn is_direct_child(child: &Path, parent: &Path) -> bool {
    let pc = parent.components().collect::<Vec<_>>();
    let cc = child.components().collect::<Vec<_>>();
    cc.len() == pc.len() + 1 && cc[..pc.len()].iter().zip(&pc).all(|(a, b)| comp_eq(a, b))
}

/// Human display of `path` relative to `root` (joined with `/`), `None` when
/// `path` is not under `root`. Used by CLI rendering so long absolute paths
/// don't drown the output (the root is printed in the header).
pub fn relative_display(path: &Path, root: &Path) -> Option<String> {
    let rc = root.components().collect::<Vec<_>>();
    let pc = path.components().collect::<Vec<_>>();
    if pc.len() <= rc.len() || !pc[..rc.len()].iter().zip(&rc).all(|(a, b)| comp_eq(a, b)) {
        return None;
    }
    let rest = pc[rc.len()..]
        .iter()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>();
    Some(rest.join("/"))
}

/// Strip the Windows `\\?\` verbatim prefix that `canonicalize` can produce, so
/// recorded keys and the data-dir exclusion use the same spelling users and
/// `%APPDATA%` use. No-op elsewhere.
pub fn normalize_abs(p: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        let s = p.to_string_lossy();
        let s: String = if let Some(rest) = s.strip_prefix(r"\\?\") {
            rest.to_string()
        } else {
            s.into_owned()
        };
        let s = if let Some(rest) = s.strip_prefix("UNC\\") {
            format!(r"\\{}", rest)
        } else {
            s
        };
        PathBuf::from(s)
    }
    #[cfg(not(windows))]
    {
        p
    }
}

/// Lexically normalize a path without touching the filesystem: drop `.`
/// components and fold `..` over preceding normal components (never above the
/// root/prefix). The result always matches how the scanner stores keys
/// (`root.join(".")` must compare equal to `root`).
pub fn normalize_lexical(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut head: Vec<std::ffi::OsString> = Vec::new(); // prefix + root markers
    let mut parts: Vec<std::ffi::OsString> = Vec::new(); // normal components
    for comp in p.components() {
        match comp {
            Component::Prefix(_) | Component::RootDir => {
                head.push(comp.as_os_str().to_os_string());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop();
            }
            Component::Normal(os) => parts.push(os.to_os_string()),
        }
    }
    let mut buf = PathBuf::new();
    for h in &head {
        buf.push(h);
    }
    for part in parts {
        buf.push(part);
    }
    if buf.as_os_str().is_empty() {
        // e.g. input was just "."
        buf.push(".");
    }
    buf
}

/// Normalize a user-supplied path argument the same way for every command:
/// relative → cwd-joined, verbatim prefix stripped, then lexically collapsed.
/// No filesystem access (the directory may no longer exist).
pub fn normalize_target(raw: &Path) -> PathBuf {
    let abs = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|c| c.join(raw))
            .unwrap_or_else(|_| raw.to_path_buf())
    };
    normalize_lexical(&normalize_abs(abs))
}

/// Component equality; case-insensitive on Windows, byte-exact elsewhere.
/// `pub(crate)` — used by the storage direct-children filter too.
#[cfg(windows)]
pub(crate) fn comp_eq(a: &Component<'_>, b: &Component<'_>) -> bool {
    // NTFS is case-insensitive; lossy is an acceptable guard (UTF-16 names are
    // practically always valid UTF-8).
    a.as_os_str().to_string_lossy().to_lowercase() == b.as_os_str().to_string_lossy().to_lowercase()
}

#[cfg(not(windows))]
pub(crate) fn comp_eq(a: &Component<'_>, b: &Component<'_>) -> bool {
    a.as_os_str() == b.as_os_str()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn is_under_detects_ancestry() {
        assert!(is_under(&p("/foo/bar/baz"), &p("/foo")));
        assert!(is_under(&p("/foo/bar"), &p("/foo/bar"))); // equal counts as under
        assert!(!is_under(&p("/foo/bar2"), &p("/foo/bar"))); // prefix collision!
        assert!(!is_under(&p("/foobar"), &p("/foo")));
        assert!(!is_under(&p("/bar"), &p("/foo/bar")));
    }

    #[test]
    fn is_under_scope_preserves_normal_ancestry() {
        assert!(is_under_scope(&p("/foo/bar/baz"), &p("/foo")));
        assert!(is_under_scope(&p("/foo/bar"), &p("/foo/bar")));
        assert!(!is_under_scope(&p("/foo/bar2"), &p("/foo/bar")));
        assert!(!is_under_scope(&p("/other"), &p("/foo")));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn is_under_scope_handles_macos_var_alias() {
        let child = Path::new("/var");
        let ancestor = Path::new("/private/var");

        assert!(is_under_scope(child, ancestor));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn is_under_scope_handles_missing_child_under_macos_var_alias() {
        let child = Path::new("/var/__diskdrift_missing_scope_test__/deleted");
        let ancestor = Path::new("/private/var");

        assert!(is_under_scope(child, ancestor));
    }

    #[test]
    fn direct_child_is_exactly_one_level() {
        assert!(is_direct_child(&p("/foo/bar"), &p("/foo")));
        assert!(!is_direct_child(&p("/foo/bar"), &p("/foo/bar1"))); // unrelated
        assert!(!is_direct_child(&p("/foo/bar/baz"), &p("/foo"))); // two levels
        assert!(!is_direct_child(&p("/foo"), &p("/foo"))); // same
        assert!(!is_direct_child(&p("/foo/bar"), &p("/foo/bar")));
    }

    #[test]
    fn direct_child_handles_slash_on_both_platforms() {
        // std::path::Path on Windows accepts `/` too; on Unix both are literal
        // separators. The key property: trailing separators add no component.
        assert!(is_direct_child(&p("/foo/bar/"), &p("/foo")));
        assert!(is_direct_child(&p("/foo/bar"), &p("/foo/")));
    }

    #[test]
    fn windows_drive_and_unc_do_not_crash() {
        #[cfg(windows)]
        {
            let unc = Path::new(r"\\server\share\foo\bar");
            assert!(is_direct_child(unc, Path::new(r"\\server\share\foo")));
            assert!(is_under(unc, Path::new(r"\\server\share")));
            assert!(is_under(
                Path::new(r"C:\foo\bar"),
                Path::new(r"c:\foo") // different case — case-insensitive on NTFS
            ));
        }
        #[cfg(not(windows))]
        {
            // On Unix a literal "C:\..." is one component; must not panic and
            // must not pretend "bar" is under "foo".
            assert!(!is_under(&p(r"C:\foo\bar"), &p("C:\\foo")));
        }
    }

    #[test]
    fn relative_display_strips_root() {
        assert_eq!(
            relative_display(&p("/root/.docker"), &p("/root")),
            Some(".docker".into())
        );
        assert_eq!(
            relative_display(&p("/root/a/b"), &p("/root")),
            Some("a/b".into())
        );
        assert_eq!(relative_display(&p("/root"), &p("/root")), None);
        assert_eq!(relative_display(&p("/other"), &p("/root")), None);
    }

    #[test]
    fn normalize_abs_is_identity_without_verbatim_prefix() {
        assert_eq!(normalize_abs(p("/a/b")), p("/a/b"));
    }

    #[test]
    fn normalize_lexical_removes_dot_and_folds_dotdot() {
        assert_eq!(normalize_lexical(&p("/a/b/./c")), p("/a/b/c"));
        assert_eq!(normalize_lexical(&p("/a/b/..")), p("/a"));
        assert_eq!(normalize_lexical(&p("/a/../b")), p("/b"));
        // ".." cannot escape above the root.
        assert_eq!(normalize_lexical(&p("/../../x")), p("/x"));
        assert_eq!(normalize_lexical(&p(".")), p("."));
        // root.join(".") collapses back to the root
        assert_eq!(normalize_lexical(&p("/root/.")), p("/root"));
    }
}
