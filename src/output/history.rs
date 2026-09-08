//! Human rendering for `diskdrift history` (text timeline; no TUI chart library).

use crate::history::HistoryReport;
use crate::output::{human, size};

/// Render a history timeline, ascending by snapshot id.
pub fn render(report: &HistoryReport) -> String {
    let mut out = String::new();
    let is_root = report.target == report.root;
    if is_root {
        out.push_str("Disk History\n");
        out.push_str(&format!("Root: {}\n", report.root.display()));
    } else {
        // Directory history: header is the target itself.
        out.push_str(&format!("{}\n", report.target.display()));
    }
    out.push('\n');

    for point in &report.points {
        let date = human::date_short(point.created_at_ms);
        let size_s = size::format(point.size);
        let delta_s = point
            .delta_from_previous
            .map(size::format_signed)
            .unwrap_or_default();
        out.push_str(&format!("{date}  {size_s:>10}  {delta_s:>10}\n"));
    }
    out.push('\n');
    out.push_str("Total change\n");
    out.push_str(&format!("{}\n", size::format_signed(report.total_delta)));
    out
}
