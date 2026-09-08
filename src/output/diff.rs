//! Rendering for `whybig diff` (presentation only; the diff engine is in
//! `crate::diff`).

use std::path::Path;

use crate::diff::{DiffEntry, SnapshotDiff};
use crate::output::{human, size};
use crate::pathutil::relative_display;

/// Render a diff. `limit = None` means "all" (`--all`); otherwise it caps each
/// of grew/shrank to `limit` rows (CLI default 10 when unsupplied).
pub fn render(diff: &SnapshotDiff, limit: Option<usize>) -> String {
    let mut out = String::new();
    out.push_str("Disk Growth\n");
    out.push_str(&format!(
        "{} → {}\n",
        human::datetime_ms(diff.before.created_at_ms),
        human::datetime_ms(diff.after.created_at_ms)
    ));
    out.push_str(&format!("root: {}\n", diff.root));
    out.push('\n');

    // `--since` honesty: when we fell back to the earliest snapshot, tell the
    // user the requested window was not actually covered.
    if let Some(since) = &diff.since {
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

    out.push_str("Total\n");
    out.push_str(&format!("  before:  {}\n", size::format(diff.total_before)));
    out.push_str(&format!("  after:   {}\n", size::format(diff.total_after)));
    out.push_str(&format!(
        "  delta:   {}\n",
        size::format_signed(diff.total_delta)
    ));
    out.push('\n');

    render_group(&mut out, "Grew", &diff.grew, &diff.root, limit, "growing");
    render_group(
        &mut out,
        "Shrank",
        &diff.shrank,
        &diff.root,
        limit,
        "shrinking",
    );
    out
}

fn render_group(
    out: &mut String,
    title: &str,
    entries: &[DiffEntry],
    root: &str,
    limit: Option<usize>,
    verb: &str,
) {
    let shown = entries.len().min(limit.unwrap_or(usize::MAX));
    if shown == 0 {
        return;
    }
    out.push_str(title);
    out.push('\n');
    out.push_str(&"─".repeat(24));
    out.push('\n');

    let root_path = Path::new(root);
    for e in &entries[..shown] {
        let rel = relative_display(Path::new(&e.path), root_path).unwrap_or_else(|| e.path.clone());
        out.push_str(&format!("{:>14}  {}\n", size::format_signed(e.delta), rel));
    }

    if shown < entries.len() {
        out.push_str(&format!(
            "... {} more {} directories\n",
            entries.len() - shown,
            verb
        ));
    }
    out.push('\n');
}
