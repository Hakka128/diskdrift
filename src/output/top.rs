//! Human rendering for `diskdrift top` (quick ranking views).

use std::path::Path;

use crate::output::{human, size};
use crate::pathutil::relative_display;
use crate::top::{TopMode, TopReport};

/// Render a top ranking. `limit = None` means all.
pub fn render(report: &TopReport, limit: Option<usize>) -> String {
    let mut out = String::new();
    match report.mode {
        TopMode::Growth => out.push_str("Top Disk Growth\n"),
        TopMode::Shrink => out.push_str("Disk Space Freed\n"),
    }
    out.push_str(&format!(
        "{} → {}\n",
        human::date_short(report.before.created_at_ms),
        human::date_short(report.after.created_at_ms)
    ));
    out.push('\n');

    // `--since` honesty (same notice family as diff).
    if let Some(since) = &report.since {
        if since.used_earliest {
            out.push_str(&format!("Requested: {}\n", since.requested_human()));
            out.push_str(&format!(
                "Available history starts: {}\n",
                human::date_short(since.effective_before_ms)
            ));
            out.push_str("Using earliest available snapshot.");
            out.push('\n');
            out.push('\n');
        }
    }

    let scope = Path::new(&report.scope);
    let limit = limit.unwrap_or(usize::MAX);
    let shown = report.entries.len().min(limit);
    for e in &report.entries[..shown] {
        let rel = relative_display(Path::new(&e.path), scope).unwrap_or_else(|| e.path.clone());
        out.push_str(&format!("{:>14}  {}\n", size::format_signed(e.delta), rel));
    }
    if shown < report.entries.len() {
        out.push_str(&format!("... {} more\n", report.entries.len() - shown));
    }
    out
}
