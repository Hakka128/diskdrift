//! Human-friendly number and duration formatting (thousands separators,
//! local timestamps).

use std::time::Duration;

use chrono::{Local, TimeZone};

/// `423_817` → `423,817`.
pub fn number(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// `50 ms` scale → `0.050s`; `8.3s` under a minute; else `1m 23s`.
pub fn duration(d: Duration) -> String {
    let secs = d.as_secs_f64();
    if secs < 1.0 {
        format!("{secs:.3}s")
    } else if secs < 60.0 {
        format!("{secs:.1}s")
    } else {
        let total = d.as_secs();
        format!("{}m {}s", total / 60, total % 60)
    }
}

/// Unix epoch milliseconds → `2026-09-08 14:20:03` (local time).
pub fn datetime_ms(ms: i64) -> String {
    match Local.timestamp_millis_opt(ms) {
        chrono::LocalResult::Single(dt) => dt.format("%Y-%m-%d %H:%M:%S").to_string(),
        _ => "unknown".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_groups_thousands() {
        assert_eq!(number(0), "0");
        assert_eq!(number(999), "999");
        assert_eq!(number(1_000), "1,000");
        assert_eq!(number(423_817), "423,817");
        assert_eq!(number(1_000_000_000), "1,000,000,000");
    }

    #[test]
    fn duration_formats() {
        assert_eq!(duration(Duration::from_millis(500)), "0.500s");
        assert_eq!(
            duration(Duration::from_secs(8) + Duration::from_millis(300)),
            "8.3s"
        );
        assert_eq!(duration(Duration::from_secs(83)), "1m 23s");
    }

    #[test]
    fn datetime_has_expected_shape() {
        // Local rendering of a fixed UTC epoch: don't pin the date itself
        // (timezone-dependent) — assert the shape only.
        let out = datetime_ms(1_609_459_200_084);
        assert_eq!(out.len(), "YYYY-MM-DD HH:MM:SS".len());
        let b = out.as_bytes();
        assert_eq!(b[4], b'-');
        assert_eq!(b[7], b'-');
        assert_eq!(b[10], b' ');
        assert_eq!(b[13], b':');
        assert_eq!(b[16], b':');
    }
}
