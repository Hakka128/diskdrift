//! Binary entry point — thin: parse → dispatch → render.
//! All business logic lives in the library; this module only maps domain
//! results to terminal text and exit codes.

use std::io::IsTerminal;
use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use whybig::cli::{Cli, Command};
use whybig::config;
use whybig::diff::{self, DiffSelection};
use whybig::error::{Result, WhyBigError};
use whybig::history;
use whybig::inspect;
use whybig::json as jsonapi;
use whybig::output::{
    diff as render_diff, history as render_history, human, inspect as render_inspect,
    size as fmt_size, top as render_top,
};
use whybig::snapshot::service::{SnapshotService, StatusReport};
use whybig::storage::Storage;
use whybig::top::{self, TopMode};

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match &cli.command {
        Command::Init => run_init(&cli),
        Command::Snapshot { path } => run_snapshot(&cli, path),
        Command::Status { json } => run_status(&cli, *json),
        Command::Diff {
            from,
            to,
            limit,
            all,
            json,
        } => run_diff(&cli, *from, *to, *limit, *all, *json),
        Command::Inspect { path, limit, json } => run_inspect(&cli, path, *limit, *json),
        Command::History { path, limit, json } => run_history(&cli, path.as_deref(), *limit, *json),
        Command::Top {
            path,
            shrink,
            limit,
            json,
        } => run_top(&cli, path.as_deref(), *shrink, *limit, *json),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run_init(cli: &Cli) -> Result<()> {
    let data_dir = config::data_dir(cli.data_dir.as_deref())?;
    let db_path = config::db_path(&data_dir);
    // new() creates the directory, opens and migrates the database.
    SnapshotService::new(data_dir.clone())?;
    println!("WhyBig is ready.");
    println!("  data directory: {}", data_dir.display());
    println!("  database:       {}", db_path.display());
    Ok(())
}

fn run_snapshot(cli: &Cli, raw_path: &Path) -> Result<()> {
    let data_dir = config::data_dir(cli.data_dir.as_deref())?;
    let mut service = SnapshotService::new(data_dir)?;

    eprintln!("Scanning {} ...", raw_path.display());

    let outcome = if std::io::stderr().is_terminal() {
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::with_template("{spinner:.cyan} {msg}")
                .unwrap_or_else(|_| ProgressStyle::default_bar()),
        );
        pb.enable_steady_tick(std::time::Duration::from_millis(120));
        // Throttle redraws: `set_message` allocates, so skip most entries.
        let mut on_progress = |n: u64| {
            if n.is_multiple_of(512) {
                pb.set_message(format!("{} entries", human::number(n)));
            }
        };
        let outcome = service.run_snapshot(raw_path, &mut on_progress)?;
        pb.finish_and_clear();
        outcome
    } else {
        let mut on_progress = |_n: u64| {};
        service.run_snapshot(raw_path, &mut on_progress)?
    };

    println!("{} files", human::number(outcome.file_count));
    println!("{} directories", human::number(outcome.dir_count));
    println!("{}", fmt_size::format(outcome.total_size));
    println!();
    println!(
        "Snapshot #{} saved in {}",
        outcome.snapshot_id,
        human::duration(outcome.elapsed)
    );

    if outcome.skipped_count > 0 {
        eprintln!();
        eprintln!("Warnings:");
        eprintln!(
            "{} entries could not be read",
            human::number(outcome.skipped_count)
        );
    }

    Ok(())
}

fn run_diff(
    cli: &Cli,
    from: Option<i64>,
    to: Option<i64>,
    limit: Option<usize>,
    all: bool,
    json: bool,
) -> Result<()> {
    let data_dir = config::data_dir(cli.data_dir.as_deref())?;
    let Some(storage) = Storage::open_if_exists(&data_dir)? else {
        // Choosing a comparison on an uninitialized database is an error the
        // user can act on (like `status`, but diff has nothing to show).
        return Err(WhyBigError::NotInitialized);
    };

    let selection = match (from, to) {
        (Some(f), Some(t)) => DiffSelection::Explicit { from: f, to: t },
        (None, None) => DiffSelection::Default,
        _ => unreachable!("clap requires --from and --to together"),
    };
    let (before, after) = diff::select_pair(&storage, selection)?;
    let diff = diff::compute(&storage, &before, &after, &after.root_path)?;
    if json {
        let doc = jsonapi::JsonDiffReportV1::build(&diff);
        println!("{}", jsonapi::serialize(&doc)?);
    } else {
        print!("{}", render_diff::render(&diff, resolve_limit(limit, all)));
    }
    Ok(())
}

fn run_inspect(cli: &Cli, raw_path: &Path, limit: Option<usize>, json: bool) -> Result<()> {
    let data_dir = config::data_dir(cli.data_dir.as_deref())?;
    let Some(storage) = Storage::open_if_exists(&data_dir)? else {
        return Err(WhyBigError::NotInitialized);
    };

    let target = inspect::normalize_target(raw_path);
    let (before, after) = diff::select_pair(&storage, DiffSelection::Default)?;
    let root = after.root_path.clone();
    let report = inspect::inspect(&storage, &before, &after, &root, &target)?;
    if json {
        let doc = jsonapi::JsonInspectReportV1::build(&report);
        println!("{}", jsonapi::serialize(&doc)?);
    } else {
        print!("{}", render_inspect::render(&report, limit));
    }
    Ok(())
}

fn run_history(cli: &Cli, path: Option<&Path>, limit: Option<usize>, json: bool) -> Result<()> {
    let data_dir = config::data_dir(cli.data_dir.as_deref())?;
    let Some(storage) = Storage::open_if_exists(&data_dir)? else {
        return Err(WhyBigError::NotInitialized);
    };
    let report = history::history(&storage, path, limit.unwrap_or(20))?;
    if json {
        let doc = jsonapi::JsonHistoryReportV1::build(&report);
        println!("{}", jsonapi::serialize(&doc)?);
    } else {
        print!("{}", render_history::render(&report));
    }
    Ok(())
}

fn run_top(
    cli: &Cli,
    path: Option<&Path>,
    shrink: bool,
    limit: Option<usize>,
    json: bool,
) -> Result<()> {
    let data_dir = config::data_dir(cli.data_dir.as_deref())?;
    let Some(storage) = Storage::open_if_exists(&data_dir)? else {
        return Err(WhyBigError::NotInitialized);
    };
    let mode = if shrink {
        TopMode::Shrink
    } else {
        TopMode::Growth
    };
    let report = top::top(&storage, DiffSelection::Default, path, mode)?;
    if json {
        let doc = jsonapi::JsonTopReportV1::build(&report);
        println!("{}", jsonapi::serialize(&doc)?);
    } else {
        print!("{}", render_top::render(&report, Some(limit.unwrap_or(10))));
    }
    Ok(())
}

/// `--all` → no limit; otherwise `--limit N` or the default 10.
fn resolve_limit(limit: Option<usize>, all: bool) -> Option<usize> {
    if all {
        None
    } else {
        Some(limit.unwrap_or(10))
    }
}

fn run_status(cli: &Cli, json: bool) -> Result<()> {
    let data_dir = config::data_dir(cli.data_dir.as_deref())?;
    let report = SnapshotService::status(data_dir)?;
    if json {
        let doc = jsonapi::JsonStatusReportV1::build(&report);
        println!("{}", jsonapi::serialize(&doc)?);
    } else {
        print_status(&report);
    }
    Ok(())
}

fn print_status(report: &StatusReport) {
    if !report.initialized {
        println!("WhyBig is not initialized here.");
        println!("  expected database: {}", report.db_path.display());
        println!("Run `whybig init` to get started.");
        return;
    }

    println!("Data directory: {}", report.data_dir.display());
    println!(
        "Database:       {} ({})",
        report.db_path.display(),
        fmt_size::format(report.db_size)
    );
    println!(
        "Snapshots:      {}",
        human::number(report.snapshot_count as u64)
    );

    if let Some(earliest) = &report.earliest {
        println!(
            "Earliest:       {}  (root: {})",
            human::datetime_ms(earliest.created_at_ms),
            earliest.root_path
        );
    }

    match &report.latest {
        Some(latest) => {
            println!(
                "Latest:         {}  (root: {})",
                human::datetime_ms(latest.created_at_ms),
                latest.root_path
            );
            println!(
                "  total size:   {}",
                fmt_size::format(latest.total_size_u64())
            );
            println!("  file count:   {}", human::number(latest.file_count_u64()));
            println!(
                "  directory count: {}",
                human::number(latest.dir_count_u64())
            );
            println!(
                "  scan duration:{}",
                human::duration(Duration::from_millis(latest.scan_duration_ms_u64()))
            );
            println!(
                "  skipped:      {}  (size kind: {})",
                human::number(latest.skipped_count_u64()),
                latest.size_kind
            );
        }
        None if report.initialized => {
            println!("No snapshots yet — run `whybig snapshot <path>`.");
        }
        None => {}
    }
}
