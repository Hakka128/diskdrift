//! `--since` duration parsing and human "N ago" wording.
//!
//! Only whole minutes/hours/days/weeks are accepted (no months/years/natural
//! language, per DESIGN-M4 §1). All time math is integer UTC milliseconds.

use crate::error::{Result, WhyBigError};

/// A wall-clock duration in whole seconds (always positive).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DurationSecs(pub i64);

impl DurationSecs {
    /// Integer UTC milliseconds, saturating at i64 bounds.
    pub fn as_ms(self) -> i64 {
        self.0.checked_mul(1000).unwrap_or(i64::MAX)
    }
}

/// Parse `30m`, `24h`, `7d`, `4w`. Rejects `0`, negatives, overflow and
/// unknown suffixes.
pub fn parse(s: &str) -> Result<DurationSecs> {
    let s = s.trim();
    if s.is_empty() {
        return Err(WhyBigError::BadDuration(s.to_string()));
    }
    let (num, unit) = s.split_at(s.len() - 1);
    let n: i64 = num
        .parse()
        .map_err(|_| WhyBigError::BadDuration(s.to_string()))?;
    if n <= 0 {
        return Err(WhyBigError::BadDuration(s.to_string()));
    }
    let secs = match unit {
        "m" => n.checked_mul(60),
        "h" => n.checked_mul(3600),
        "d" => n.checked_mul(86_400),
        "w" => n.checked_mul(604_800),
        _ => return Err(WhyBigError::BadDuration(s.to_string())),
    };
    let secs = secs.ok_or_else(|| WhyBigError::BadDuration(s.to_string()))?;
    Ok(DurationSecs(secs))
}

/// "7 days ago" / "24 hours ago" / "30 minutes ago" for human notices.
pub fn human_ago(secs: i64) -> String {
    fn unit(n: i64, singular: &str, plural: &str) -> String {
        if n == 1 {
            format!("1 {singular} ago")
        } else {
            format!("{n} {plural} ago")
        }
    }
    if secs >= 2 * 604_800 && secs % 604_800 == 0 {
        unit(secs / 604_800, "week", "weeks")
    } else if secs >= 86_400 && secs % 86_400 == 0 {
        unit(secs / 86_400, "day", "days")
    } else if secs >= 3600 && secs % 3600 == 0 {
        unit(secs / 3600, "hour", "hours")
    } else {
        unit((secs / 60).max(1), "minute", "minutes")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_units() {
        assert_eq!(parse("30m").unwrap().0, 30 * 60);
        assert_eq!(parse("24h").unwrap().0, 24 * 3600);
        assert_eq!(parse("7d").unwrap().0, 7 * 86_400);
        assert_eq!(parse("4w").unwrap().0, 4 * 604_800);
        assert_eq!(parse(" 2d ").unwrap().0, 2 * 86_400);
    }

    #[test]
    fn rejects_bad_inputs() {
        for bad in [
            "",
            "0d",
            "-7d",
            "7",
            "d",
            "1y",
            "1mo",
            "abc",
            "1.5d",
            "999999999999999999999d",
        ] {
            assert!(parse(bad).is_err(), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn huge_duration_saturates_ms() {
        // "999999999999999999999d" already rejected at parse; a legal-but-huge
        // value must not overflow when converting to ms.
        let ok = DurationSecs(i64::MAX / 1000);
        let _ = ok.as_ms();
    }

    #[test]
    fn human_wording() {
        assert_eq!(human_ago(7 * 86_400), "7 days ago");
        assert_eq!(human_ago(86_400), "1 day ago");
        assert_eq!(human_ago(2 * 3600), "2 hours ago");
        assert_eq!(human_ago(30 * 60), "30 minutes ago");
        assert_eq!(human_ago(4 * 604_800), "4 weeks ago");
    }
}
