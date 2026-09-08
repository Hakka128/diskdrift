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

#[cfg(test)]
mod tests {
    use super::format;

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
}
