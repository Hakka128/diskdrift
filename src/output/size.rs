//! Human-readable byte sizes (base 1024, one decimal).
//!
//! Examples: `999 B`, `1.0 KB`, `238.7 GB`.

const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];

/// Format a non-negative byte count.
pub fn format(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit_idx = 0;
    while value >= 1024.0 && unit_idx < UNITS.len() - 1 {
        value /= 1024.0;
        unit_idx += 1;
    }
    format!("{value:.1} {}", UNITS[unit_idx])
}

/// Format a signed byte delta, always with an explicit sign: `+17.4 GB`,
/// `-840 MB`, `0 B`. Input is `i128` so `u64` pairs can never overflow.
pub fn format_signed(delta: i128) -> String {
    let sign = if delta < 0 {
        "-"
    } else if delta > 0 {
        "+"
    } else {
        ""
    };
    let abs = delta.unsigned_abs();
    let body = if abs > u128::from(u64::MAX) {
        // Unreachable for real filesystems (≥ 16 EiB); keep formatting total.
        "16384.0 PB".to_string()
    } else {
        format(abs as u64)
    };
    format!("{sign}{body}")
}

#[cfg(test)]
mod tests {
    use super::{format, format_signed};

    #[test]
    fn boundaries() {
        assert_eq!(format(0), "0 B");
        assert_eq!(format(1), "1 B");
        assert_eq!(format(1023), "1023 B");
        assert_eq!(format(1024), "1.0 KB");
        assert_eq!(format(1024 * 1024), "1.0 MB");
        assert_eq!(format(1024 * 1024 * 1024), "1.0 GB");
    }

    #[test]
    fn representative_values() {
        assert_eq!(format(238_720_000_000), "222.3 GB");
        assert_eq!(format(730 * 1024 * 1024), "730.0 MB");
        assert_eq!(format(17_400_000_000), "16.2 GB");
    }

    #[test]
    fn large_does_not_overflow_units() {
        // Still formats, clamps at PB.
        assert_eq!(format(u64::MAX), "16384.0 PB");
    }

    #[test]
    fn signed_formatting() {
        assert_eq!(format_signed(0), "0 B");
        assert_eq!(format_signed(17_400_000_000), "+16.2 GB");
        assert_eq!(format_signed(-17_400_000_000), "-16.2 GB");
        assert_eq!(format_signed(1024), "+1.0 KB");
        assert_eq!(format_signed(-1), "-1 B");
        // i128 deltas beyond u64 still format without panic/overflow.
        assert_eq!(format_signed(i128::from(u64::MAX) + 1), "+16384.0 PB");
        assert_eq!(format_signed(-(i128::from(u64::MAX) + 1)), "-16384.0 PB");
    }
}
