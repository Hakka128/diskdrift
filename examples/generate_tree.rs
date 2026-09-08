//! Deterministic benchmark-tree generator CLI.
//!
//! Build a reproducible fixture tree without writing GBs of random data:
//! `cargo run --release --example generate_tree -- <target> --files 100000 --mode mixed --seed 7`

use std::path::PathBuf;

use clap::{Parser, ValueEnum};
use whybig::benchtree::{generate_tree, BenchMode, GenConfig};

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
    name = "whybig-generate-tree",
    about = "Generate a deterministic benchmark tree"
)]
struct Cli {
    /// Directory to fill (must be empty unless --force).
    #[arg(value_name = "TARGET")]
    target: PathBuf,

    /// Exact number of files to create.
    #[arg(long, value_name = "N", default_value_t = 10_000)]
    files: u64,

    /// PRNG seed; same seed ⇒ identical tree.
    #[arg(long, value_name = "N", default_value_t = 0x5eed)]
    seed: u64,

    /// Tree shape.
    #[arg(long, value_enum, default_value_t = ModeArg::Mixed)]
    mode: ModeArg,

    /// Replace a non-empty target directory (nothing is deleted without this).
    #[arg(long)]
    force: bool,
}

fn main() {
    let cli = Cli::parse();
    let cfg = GenConfig {
        files: cli.files,
        mode: cli.mode.into(),
        seed: cli.seed,
        force: cli.force,
    };
    match generate_tree(&cli.target, &cfg) {
        Ok(stats) => {
            println!(
                "generated {} files in {} directories at {}",
                stats.files_created,
                stats.dirs_created,
                stats.root.display()
            );
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}
