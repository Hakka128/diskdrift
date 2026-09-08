//! Command-line interface (clap derive).
//!
//! Presentation layer only: parsing happens here, business logic lives in
//! [`crate::snapshot`] / [`crate::storage`] / [`crate::scanner`].

use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// WhyBig — disk-space growth tracker.
///
/// Disk analyzers tell you what's big. WhyBig tells you what got big:
/// it records directory-size snapshots over time so you can see what grew.
#[derive(Debug, Parser, Clone)]
#[command(
    name = "whybig",
    version,
    about,
    long_about = None,
    subcommand_required = true,
    arg_required_else_help = true
)]
pub struct Cli {
    /// Override the WhyBig data directory
    /// (default: platform app-data dir; also WHYBIG_DATA_DIR).
    #[arg(long, global = true, env = "WHYBIG_DATA_DIR", value_name = "DIR")]
    pub data_dir: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand, Clone)]
pub enum Command {
    /// Create the WhyBig data directory and database (idempotent).
    Init,

    /// Record a size snapshot of a directory tree.
    Snapshot {
        /// The directory to scan (absolute or relative).
        #[arg(value_name = "PATH")]
        path: PathBuf,
    },

    /// Show what changed between two snapshots (top-level attribution).
    Diff {
        /// Older snapshot id to compare (requires --to).
        #[arg(long, value_name = "ID", requires = "to")]
        from: Option<i64>,
        /// Newer snapshot id to compare (requires --from).
        #[arg(long, value_name = "ID", requires = "from")]
        to: Option<i64>,
        /// Show at most N entries per group (default 10).
        #[arg(long, value_name = "N")]
        limit: Option<usize>,
        /// Show every changed entry (overrides --limit).
        #[arg(long)]
        all: bool,
    },

    /// Drill down into a directory's changes ("what grew inside this?").
    Inspect {
        /// A directory inside the tracked root.
        #[arg(value_name = "PATH")]
        path: PathBuf,
        /// Show at most N contributors (default: all).
        #[arg(long, value_name = "N")]
        limit: Option<usize>,
    },

    /// Show the database and latest snapshot summary.
    Status,
}
