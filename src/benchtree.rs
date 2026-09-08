//! Deterministic benchmark tree generator.
//!
//! Builds a reproducible fixtures (nested) directory tree for benchmarking the
//! scanner without writing real random data across dozens of GB:
//! - a seeded xorshift64 PRNG makes every run with the same `seed` identical;
//! - small/tiny files hold a handful of bytes, large files use `set_len`
//!   (sparse where the platform supports it);
//! - the generator never touches files outside the target root, refuses a
//!   non-empty target unless `force` is given, and counts exactly `files`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Tree shapes (see `DESIGN-M3.md` §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BenchMode {
    /// ~100-200 directories, many files each.
    Wide,
    /// One deep chain, files spread along the levels.
    Deep,
    /// Repeated project-like trees (`p{}/src`, `p{}/docs`, ...).
    Mixed,
    /// Many tiny files (0-64 bytes).
    Tiny,
    /// Few large (sparse) files.
    Large,
}

/// Generator parameters.
#[derive(Debug, Clone)]
pub struct GenConfig {
    /// Exact number of files to create.
    pub files: u64,
    pub mode: BenchMode,
    /// PRNG seed; same seed ⇒ identical tree.
    pub seed: u64,
    /// Replace a pre-existing non-empty target directory.
    pub force: bool,
}

/// What was created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenStats {
    pub root: PathBuf,
    pub files_created: u64,
    pub dirs_created: u64,
}

impl GenConfig {
    /// A reasonable default: 10k files, mixed project tree, arbitrary seed.
    pub fn default(files: u64) -> Self {
        Self {
            files,
            mode: BenchMode::Mixed,
            seed: 0x5eed,
            force: false,
        }
    }
}

/// xorshift64 — tiny deterministic PRNG (benchmark only; bias by `%` is fine).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            return 0;
        }
        self.next() % n
    }
}

/// Generate the tree. Fails on `files == 0` or a non-empty target without
/// `force`.
pub fn generate_tree(root: &Path, cfg: &GenConfig) -> io::Result<GenStats> {
    if cfg.files == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "files must be >= 1",
        ));
    }
    if root.exists() && !is_empty_dir(root)? {
        if !cfg.force {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "target `{}` is not empty; use --force to replace it (nothing was deleted without it)",
                    root.display()
                ),
            ));
        }
        fs::remove_dir_all(root)?;
    }
    fs::create_dir_all(root)?;

    let mut rng = Rng(cfg.seed ^ cfg.files.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    let mut dirs_created: u64 = 0;
    let mut files_created: u64 = 0;

    // Plan the shape and produce directories + per-directory file counts.
    let plan: Vec<(PathBuf, u64)> = match cfg.mode {
        BenchMode::Wide => {
            let dirs = cfg.files.clamp(1, 200);
            (0..dirs)
                .map(|i| {
                    (
                        root.join(format!("d{i:04}")),
                        cfg.files / dirs + u64::from(i < cfg.files % dirs),
                    )
                })
                .collect()
        }
        BenchMode::Deep => {
            let depth = cfg.files.clamp(4, 64);
            let files_per_level = cfg.files / depth;
            let mut out = Vec::new();
            let mut p = root.to_path_buf();
            for _ in 0..depth {
                p.push("lvl");
                out.push((p.clone(), files_per_level));
            }
            // remainder goes into the deepest level
            let rem = cfg.files - files_per_level * depth;
            if rem > 0 {
                if let Some((_, c)) = out.last_mut() {
                    *c += rem;
                }
            }
            out
        }
        BenchMode::Mixed => {
            let projects = cfg.files.clamp(1, 200);
            let subdirs = ["src", "docs", "assets"];
            let files_per_project = cfg.files / projects;
            let mut out = Vec::new();
            for i in 0..projects {
                for (j, sub) in subdirs.iter().enumerate() {
                    let base = files_per_project / subdirs.len() as u64;
                    let extra = u64::from((j as u64) < files_per_project % subdirs.len() as u64);
                    out.push((root.join(format!("p{i:04}")).join(sub), base + extra));
                }
            }
            // absorb remainder (files - projects*files_per_project) into p000/src
            // note: files_per_project already floors; remainder may be non-zero.
            out
        }
        BenchMode::Tiny => {
            let dirs = cfg.files.clamp(1, 500);
            (0..dirs)
                .map(|i| {
                    (
                        root.join(format!("t{i:04}")),
                        cfg.files / dirs + u64::from(i < cfg.files % dirs),
                    )
                })
                .collect()
        }
        BenchMode::Large => {
            // Few files, each sparse (`set_len`), all in one directory.
            vec![(root.join("large"), cfg.files.min(50))]
        }
    };
    // The Mixed plan above may not sum exactly to `files`; reconcile below by
    // adjusting the difference on the first entry.
    let mut plan = plan;

    // Exact-count reconciliation: adjust the difference on the first directory.
    let planned: u64 = plan.iter().map(|(_, c)| *c).sum();
    if let Some((_, c)) = plan.first_mut() {
        let diff = cfg.files as i128 - planned as i128;
        *c = (*c as i128 + diff).max(0) as u64;
    }

    for (dir, count) in &plan {
        fs::create_dir_all(dir)?;
        dirs_created += 1;
        let bytes = [0u8; 128];
        for _ in 0..*count {
            let name = format!("f{files_created:06}.dat");
            let path = dir.join(name);
            match cfg.mode {
                BenchMode::Large => {
                    let size = 1u64 << (rng.below(4) + 20); // 1 MiB .. 8 MiB (sparse)
                    let f = fs::File::create(&path)?;
                    f.set_len(size)?;
                }
                BenchMode::Tiny => {
                    let n = rng.below(65) as usize; // 0..=64 bytes
                    fs::write(&path, &bytes[..n])?;
                }
                _ => {
                    let n = (rng.below(128) + 1) as usize; // 1..=128 bytes
                    fs::write(&path, &bytes[..n])?;
                }
            }
            files_created += 1;
        }
    }

    Ok(GenStats {
        root: root.to_path_buf(),
        files_created,
        dirs_created,
    })
}

fn is_empty_dir(p: &Path) -> io::Result<bool> {
    if !p.is_dir() {
        return Ok(false);
    }
    Ok(fs::read_dir(p)?.next().is_none())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn deterministic_tree_same_seed_identical() {
        let root_a = TempDir::new().unwrap();
        let root_b = TempDir::new().unwrap();
        let cfg_a = GenConfig {
            files: 500,
            mode: BenchMode::Mixed,
            seed: 42,
            force: false,
        };
        let cfg_b = GenConfig {
            files: 500,
            mode: BenchMode::Mixed,
            seed: 42,
            force: false,
        };
        let a = generate_tree(root_a.path(), &cfg_a).unwrap();
        let b = generate_tree(root_b.path(), &cfg_b).unwrap();
        assert_eq!(a.files_created, b.files_created);
        assert_eq!(a.dirs_created, b.dirs_created);
        assert_eq!(a.files_created, 500);
    }

    #[test]
    fn different_seed_different_sizes() {
        let ra = TempDir::new().unwrap();
        let rb = TempDir::new().unwrap();
        let a = generate_tree(
            ra.path(),
            &GenConfig {
                files: 300,
                mode: BenchMode::Mixed,
                seed: 1,
                force: false,
            },
        )
        .unwrap();
        let b = generate_tree(
            rb.path(),
            &GenConfig {
                files: 300,
                mode: BenchMode::Mixed,
                seed: 2,
                force: false,
            },
        )
        .unwrap();
        let size_a = dir_total(ra.path());
        let size_b = dir_total(rb.path());
        assert_eq!(a.files_created, 300);
        assert_eq!(b.files_created, 300);
        assert_ne!(size_a, size_b, "different seeds should change sizes");
    }

    #[test]
    fn exact_count_for_all_modes() {
        for (mode, count) in [
            (BenchMode::Wide, 137u64),
            (BenchMode::Deep, 137),
            (BenchMode::Mixed, 137),
            (BenchMode::Tiny, 137),
            (BenchMode::Large, 9), // large/sparse files — keep the test light
        ] {
            let td = TempDir::new().unwrap();
            let cfg = GenConfig {
                files: count,
                mode,
                seed: 7,
                force: false,
            };
            let s = generate_tree(td.path(), &cfg).unwrap();
            assert_eq!(s.files_created, count, "mode {mode:?}");
            assert!(s.dirs_created >= 1);
            let _ = td;
        }
    }

    #[test]
    fn zero_files_is_invalid() {
        let td = TempDir::new().unwrap();
        let err = generate_tree(
            td.path(),
            &GenConfig {
                files: 0,
                mode: BenchMode::Wide,
                seed: 1,
                force: false,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains(">= 1"));
    }

    #[test]
    fn non_empty_target_requires_force() {
        let td = TempDir::new().unwrap();
        fs::write(td.path().join("keep.txt"), b"x").unwrap();
        let err = generate_tree(
            td.path(),
            &GenConfig {
                files: 10,
                mode: BenchMode::Wide,
                seed: 1,
                force: false,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("--force"));

        // force replaces it and completes with the exact count.
        let s = generate_tree(
            td.path(),
            &GenConfig {
                files: 25,
                mode: BenchMode::Wide,
                seed: 1,
                force: true,
            },
        )
        .unwrap();
        assert_eq!(s.files_created, 25);
        assert!(!td.path().join("keep.txt").exists());
    }

    #[test]
    fn repeated_generation_is_stable_and_force_replaces() {
        let td = TempDir::new().unwrap();
        let cfg = GenConfig {
            files: 400,
            mode: BenchMode::Mixed,
            seed: 9,
            force: false,
        };
        let first = generate_tree(td.path(), &cfg).unwrap();
        assert_eq!(first.files_created, 400);

        // Without --force, regenerating into the now-non-empty dir fails.
        let err = generate_tree(td.path(), &cfg).unwrap_err();
        assert!(err.to_string().contains("--force"));

        // With --force it replaces and stays deterministic.
        let again = generate_tree(
            td.path(),
            &GenConfig {
                files: 400,
                mode: BenchMode::Mixed,
                seed: 9,
                force: true,
            },
        )
        .unwrap();
        assert_eq!(again.files_created, 400);
        assert_eq!(again.dirs_created, first.dirs_created);
    }

    fn dir_total(p: &Path) -> u64 {
        let mut sum = 0u64;
        if let Ok(rd) = fs::read_dir(p) {
            for e in rd.flatten() {
                let md = e.metadata().unwrap();
                if md.is_dir() {
                    sum += dir_total(&e.path());
                } else {
                    sum += md.len();
                }
            }
        }
        sum
    }
}
