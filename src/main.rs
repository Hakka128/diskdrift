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
use whybig::error::Result;
use whybig::output::{human, size as fmt_size};
use whybig::snapshot::service::{SnapshotService, StatusReport};

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match &cli.command {
        Command::Init => run_init(&cli),
        Command::Snapshot { path } => run_snapshot(&cli, path),
        Command::Status => run_status(&cli),
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

fn run_status(cli: &Cli) -> Result<()> {
    let data_dir = config::data_dir(cli.data_dir.as_deref())?;
    let report = SnapshotService::status(data_dir)?;
    print_status(&report);
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
