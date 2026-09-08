//! Minimal, honest benchmark runner for WhyBig's core pipeline.
//!
//! Measures (in-process, wall-clock):
//!   generation, scanner wall time + files/sec + dirs/sec, DB write time +
//!   DB size, diff latency, history latency.
//!
//! Run: `cargo run --release --example bench -- --files 100000 --mode mixed --seed 7`
//!
//! These numbers are *noisy* environment measurements (OS cache, AV, HDD vs
//! SSD) — record them in BENCHMARKS.md, never present them as a promise.

use std::path::PathBuf;
use std::time::Instant;

use clap::{Parser, ValueEnum};
use tempfile::TempDir;
use whybig::benchtree::{generate_tree, BenchMode, GenConfig};
use whybig::diff::{self, DiffSelection};
use whybig::history;
use whybig::scanner::{self, ScanOptions};
use whybig::snapshot::service::SnapshotService;
use whybig::storage::Storage;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ModeArg {
    Wide,
    Deep,
    Mixed,
    Tiny,
    Large,
}

impl From<ModeArg> for BenchMode {
    fn from(m: ModeArg) -> Self {
        match m {
            ModeArg::Wide => BenchMode::Wide,
            ModeArg::Deep => BenchMode::Deep,
            ModeArg::Mixed => BenchMode::Mixed,
            ModeArg::Tiny => BenchMode::Tiny,
            ModeArg::Large => BenchMode::Large,
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "whybig-bench",
    about = "Benchmark scanner + storage + diff + history in-process"
)]
struct Cli {
    /// Number of files in the generated tree.
    #[arg(long, value_name = "N", default_value_t = 10_000)]
    files: u64,

    #[arg(long, value_enum, default_value_t = ModeArg::Mixed)]
    mode: ModeArg,

    #[arg(long, value_name = "N", default_value_t = 0x5eed)]
    seed: u64,

    /// Optional directory to generate into (default: a temp dir).
    #[arg(long, value_name = "DIR")]
    target: Option<PathBuf>,
}

fn main() {
    let cli = Cli::parse();

    enum Keep {
        // Kept alive for the duration of the bench; the field is intentionally
        // never read.
        #[allow(dead_code)]
        Temp(TempDir),
        Given,
    }
    let (target, keep) = match &cli.target {
        Some(t) => (t.clone(), Keep::Given),
        None => {
            let td = TempDir::new().expect("tempdir");
            (td.path().to_path_buf(), Keep::Temp(td))
        }
    };
    // Keep the temp dir alive until the end of main.
    let _keep = keep;

    let gen_cfg = GenConfig {
        files: cli.files,
        mode: cli.mode.into(),
        seed: cli.seed,
        force: true, // the runner owns this directory in the bench case
    };

    println!("=== WhyBig bench ===");
    println!(
        "files={} mode={:?} seed={} target={} toolchain=release",
        cli.files,
        cli.mode,
        cli.seed,
        target.display()
    );
    println!("note: wall-clock, noisy environment, warm-ish cache");

    let t0 = Instant::now();
    let stats = generate_tree(&target, &gen_cfg).expect("generate");
    println!(
        "generate:           {:8.1} ms  ({} files, {} dirs)",
        t0.elapsed().as_secs_f64() * 1000.0,
        stats.files_created,
        stats.dirs_created
    );

    // ── scan ──────────────────────────────────────────────────────────────
    let opts = ScanOptions {
        root: target.clone(),
        exclude: None,
        max_depth: None,
    };
    let t1 = Instant::now();
    let result = scanner::scan(&opts, &mut |_| {}).expect("scan");
    let scan_secs = t1.elapsed().as_secs_f64();
    println!("scan:               {:8.1} ms", scan_secs * 1000.0);
    println!(
        "  files/sec:        {:10.0}",
        result.summary.file_count as f64 / scan_secs
    );
    println!(
        "  dirs/sec:         {:10.0}",
        result.summary.dir_count as f64 / scan_secs
    );
    println!(
        "  total:            {} files, {} dirs, {} bytes",
        result.summary.file_count, result.summary.dir_count, result.summary.total_size
    );

    // ── persist (two snapshots so diff/history have a pair) ───────────────
    let data_dir = target.join(".whybig-bench-data");
    let mut service = SnapshotService::new(data_dir.clone()).expect("service");

    let t2 = Instant::now();
    let id_a = service
        .run_snapshot(&target, &mut |_| {})
        .expect("snapshot A");
    service
        .run_snapshot(&target, &mut |_| {})
        .expect("snapshot B");
    let write_secs = t2.elapsed().as_secs_f64();
    let storage = Storage::open(&data_dir).expect("storage");
    println!("db write x2:        {:8.1} ms", write_secs * 1000.0);
    // WAL mode keeps most data in `-wal` until checkpoint; measure all files.
    let db_path = storage.database_path();
    let sidecar = |suffix: &str| {
        std::fs::metadata(format!("{}{}", db_path.display(), suffix))
            .map(|m| m.len())
            .unwrap_or(0)
    };
    let db_total = storage.database_size() + sidecar("-wal") + sidecar("-shm");
    println!(
        "  db size:          {} bytes ({} + {} entries rows)",
        db_total,
        storage.count_snapshots().expect("count"),
        storage
            .snapshot_entries_count(id_a.snapshot_id)
            .expect("entries")
    );

    // ── diff ──────────────────────────────────────────────────────────────
    let t3 = Instant::now();
    let pair = diff::select_pair(&storage, DiffSelection::Default).expect("select");
    let (before, after) = (pair.before, pair.after);
    let diff = diff::compute(&storage, &before, &after, &after.root_path).expect("diff");
    println!(
        "diff:               {:8.1} ms  ({} grew + {} shrank)",
        t3.elapsed().as_secs_f64() * 1000.0,
        diff.grew.len(),
        diff.shrank.len()
    );

    // ── history ───────────────────────────────────────────────────────────
    let t4 = Instant::now();
    let hist = history::history(&storage, None, 20).expect("history");
    println!(
        "history (limit 20): {:8.1} ms  ({} points)",
        t4.elapsed().as_secs_f64() * 1000.0,
        hist.points.len()
    );
}
