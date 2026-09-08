//! Rendering for `whybig inspect` (presentation only; the inspect service is
//! in `crate::inspect`).

use std::path::Path;

use crate::inspect::InspectReport;
use crate::output::{human, size};
use crate::pathutil::relative_display;

/// Render an inspect report: header, before/after sizes, total growth, and the
/// contributor list capped at `limit = None` meaning "all".
pub fn render(report: &InspectReport, limit: Option<usize>) -> String {
    let mut out = String::new();
    out.push_str(&report.target);
    out.push('\n');
    out.push_str(&format!("root: {}\n", report.root));
    out.push('\n');

    out.push_str(&format!(
        "{}  {}  (snapshot {})\n",
        human::datetime_ms(report.before.created_at_ms),
        size::format(report.target_before),
        report.before.id
    ));
    out.push_str(&format!(
        "{}  {}  (snapshot {})\n",
        human::datetime_ms(report.after.created_at_ms),
        size::format(report.target_after),
        report.after.id
    ));
    out.push('\n');

    if report.target_delta > 0 {
        out.push_str("Total growth\n");
    } else if report.target_delta < 0 {
        out.push_str("Total shrink\n");
    } else {
        out.push_str("Total change\n");
    }
    out.push_str(&format!("{}\n", size::format_signed(report.target_delta)));
    out.push('\n');

    out.push_str("Contributors\n");
    out.push_str(&"─".repeat(24));
    out.push('\n');

    let target_path = Path::new(&report.target);
    let shown = report.contributors.len().min(limit.unwrap_or(usize::MAX));
    for e in &report.contributors[..shown] {
        let rel =
            relative_display(Path::new(&e.path), target_path).unwrap_or_else(|| e.path.clone());
        out.push_str(&format!("{:>14}  {}/\n", size::format_signed(e.delta), rel));
    }
    if shown < report.contributors.len() {
        out.push_str(&format!(
            "... {} more contributors\n",
            report.contributors.len() - shown
        ));
    }

    out.push_str(&format!(
        "{:>14}  other\n",
        size::format_signed(report.other)
    ));
    out
}
