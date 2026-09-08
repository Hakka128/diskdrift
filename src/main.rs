//! Binary entry point — thin: parse → dispatch → render.
//! All business logic lives in the library; this module only maps domain
//! results to terminal text and exit codes.

use std::io::IsTerminal;
use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use chrono::{TimeZone, Utc};
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
use whybig::retention::{self, RetentionPolicy};
use whybig::since;
use whybig::snapshot::service::{SnapshotService, StatusReport};
use whybig::storage::Storage;
use whybig::top::{self, TopMode};

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match &cli.command {
        Command::Init => run_init(&cli),
        Command::Snapshot { path, json } => run_snapshot(&cli, path, *json),
        Command::Status { json } => run_status(&cli, *json),
        Command::Diff {
            from,
            to,
            since,
            limit,
            all,
            json,
        } => run_diff(&cli, *from, *to, since.as_deref(), *limit, *all, *json),
        Command::Inspect { path, limit, json } => run_inspect(&cli, path, *limit, *json),
        Command::History { path, limit, json } => run_history(&cli, path.as_deref(), *limit, *json),
        Command::Top {
            path,
            shrink,
            limit,
            since,
            json,
        } => run_top(
            &cli,
            path.as_deref(),
            *shrink,
            since.as_deref(),
            *limit,
            *json,
        ),
        Command::Prune {
            apply,
            recent_days,
            daily_days,
            weekly_days,
            json,
        } => run_prune(&cli, *apply, *recent_days, *daily_days, *weekly_days, *json),
        Command::Compact => run_compact(&cli),
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

fn run_snapshot(cli: &Cli, raw_path: &Path, json: bool) -> Result<()> {
    let data_dir = config::data_dir(cli.data_dir.as_deref())?;
    let mut service = SnapshotService::new(data_dir)?;

    let outcome = if json {
        // JSON mode: no progress bar, no "Scanning", warnings go into the doc.
        let mut noop = |_n: u64| {};
        service.run_snapshot(raw_path, &mut noop)?
    } else if std::io::stderr().is_terminal() {
        eprintln!("Scanning {} ...", raw_path.display());
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::with_template("{spinner:.cyan} {msg}")
                .unwrap_or_else(|_| ProgressStyle::default_bar()),
        );
        pb.enable_steady_tick(Duration::from_millis(120));
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
        eprintln!("Scanning {} ...", raw_path.display());
        let mut noop = |_n: u64| {};
        service.run_snapshot(raw_path, &mut noop)?
    };

    if json {
        let doc = jsonapi::JsonSnapshotReportV1::build(&outcome);
        println!("{}", jsonapi::serialize(&doc)?);
    } else {
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
    }

    Ok(())
}

fn run_diff(
    cli: &Cli,
    from: Option<i64>,
    to: Option<i64>,
    since_arg: Option<&str>,
    limit: Option<usize>,
    all: bool,
    json: bool,
) -> Result<()> {
    let data_dir = config::data_dir(cli.data_dir.as_deref())?;
    let Some(storage) = Storage::open_if_exists(&data_dir)? else {
        return Err(WhyBigError::NotInitialized);
    };

    let selection = if let Some(d) = since_arg {
        DiffSelection::Since {
            duration: since::parse(d)?,
        }
    } else {
        match (from, to) {
            (Some(f), Some(t)) => DiffSelection::Explicit { from: f, to: t },
            (None, None) => DiffSelection::Default,
            _ => unreachable!("clap requires --from and --to together"),
        }
    };

    let pair = diff::select_pair(&storage, selection)?;
    let diff = diff::compute_pair(&storage, &pair, &pair.after.root_path)?;
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
    let pair = diff::select_pair(&storage, DiffSelection::Default)?;
    let root = pair.after.root_path.clone();
    let report = inspect::inspect(&storage, &pair.before, &pair.after, &root, &target)?;
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
    since_arg: Option<&str>,
    limit: Option<usize>,
    json: bool,
) -> Result<()> {
    let data_dir = config::data_dir(cli.data_dir.as_deref())?;
    let Some(storage) = Storage::open_if_exists(&data_dir)? else {
        return Err(WhyBigError::NotInitialized);
    };
    let selection = if let Some(d) = since_arg {
        DiffSelection::Since {
            duration: since::parse(d)?,
        }
    } else {
        DiffSelection::Default
    };
    let mode = if shrink {
        TopMode::Shrink
    } else {
        TopMode::Growth
    };
    let report = top::top(&storage, selection, path, mode)?;
    if json {
        let doc = jsonapi::JsonTopReportV1::build(&report);
        println!("{}", jsonapi::serialize(&doc)?);
    } else {
        print!("{}", render_top::render(&report, Some(limit.unwrap_or(10))));
    }
    Ok(())
}

fn run_prune(
    cli: &Cli,
    apply: bool,
    recent_days: Option<u32>,
    daily_days: Option<u32>,
    weekly_days: Option<u32>,
    json: bool,
) -> Result<()> {
    let data_dir = config::data_dir(cli.data_dir.as_deref())?;
    let Some(mut storage) = Storage::open_if_exists(&data_dir)? else {
        return Err(WhyBigError::NotInitialized);
    };

    let policy = RetentionPolicy {
        recent_days: recent_days.unwrap_or(7),
        daily_days: daily_days.unwrap_or(30),
        weekly_days: weekly_days.unwrap_or(365),
    };
    let now_ms = Utc::now().timestamp_millis();
    let snapshots = storage.list_snapshots()?;
    let plan = retention::build_plan(&policy, &snapshots, now_ms)?;

    let oldest_removed_ms = {
        let ids: std::collections::BTreeSet<i64> = plan.remove_ids().into_iter().collect();
        snapshots
            .iter()
            .filter(|s| ids.contains(&s.id))
            .map(|s| s.created_at_ms)
            .min()
    };
    let db_size = storage.database_size();

    if json {
        let doc = jsonapi::JsonPruneReportV1::build(
            &plan,
            apply,
            snapshots.len() as i64,
            db_size,
            oldest_removed_ms,
        );
        println!("{}", jsonapi::serialize(&doc)?);
        if apply {
            apply_prune(&mut storage, &plan)?;
        }
        return Ok(());
    }

    if apply {
        let summary = apply_prune(&mut storage, &plan)?;
        println!("Removed {} snapshots.", summary.snapshots_removed);
        println!("Database logical data was reduced. SQLite may retain free pages for reuse.");
        println!("Run `whybig compact` to shrink the file explicitly.");
    } else {
        print_prune_preview(&plan, now_ms, oldest_removed_ms, db_size);
    }
    Ok(())
}

/// Execute a prune plan (guarded by its own invariants + single transaction).
fn apply_prune(
    storage: &mut Storage,
    plan: &retention::PrunePlan,
) -> Result<retention::PruneSummary> {
    retention::PruneExecutor::apply(storage, plan)
}

fn print_prune_preview(
    plan: &retention::PrunePlan,
    now_ms: i64,
    oldest_removed_ms: Option<i64>,
    db_size: u64,
) {
    let as_date = |ms: i64| {
        Utc.timestamp_millis_opt(ms)
            .single()
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "?".to_string())
    };
    let recent_kept: usize = plan.roots.iter().map(|r| r.keep_recent.len()).sum();
    let daily_kept: usize = plan.roots.iter().map(|r| r.keep_daily.len()).sum();
    let weekly_kept: usize = plan.roots.iter().map(|r| r.keep_weekly.len()).sum();

    println!("Retention Preview");
    println!(
        "Snapshots: {}",
        human::number(plan.total_snapshots() as u64)
    );
    println!();
    println!("Would keep:");
    println!("  {} recent snapshot(s)", human::number(recent_kept as u64));
    println!("  {} daily snapshot(s)", human::number(daily_kept as u64));
    println!("  {} weekly snapshot(s)", human::number(weekly_kept as u64));
    println!();
    println!("Would remove:");
    println!(
        "  {} snapshot(s)",
        human::number(plan.total_remove() as u64)
    );
    if let Some(oldest) = oldest_removed_ms {
        println!("Oldest removed:");
        println!("  {}", as_date(oldest));
    }
    println!();
    println!("Database size:");
    println!("  {}", fmt_size::format(db_size));
    println!();
    println!("No data has been deleted.");
    println!("Run:");
    println!("  whybig prune --apply");
    println!("to apply this retention policy.");
    println!();
    println!("(Retention only removes WhyBig's own snapshots; it never touches your files.)");
    let _ = now_ms;
}

fn run_compact(cli: &Cli) -> Result<()> {
    let data_dir = config::data_dir(cli.data_dir.as_deref())?;
    let Some(mut storage) = Storage::open_if_exists(&data_dir)? else {
        return Err(WhyBigError::NotInitialized);
    };
    storage.vacuum()?;
    println!("Database compacted (VACUUM).");
    println!("  new size: {}", fmt_size::format(storage.database_size()));
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
