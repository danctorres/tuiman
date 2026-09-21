//! Compact number and date formatting.

/// `532`, `1.2k`, `48k`, `1.1M`.
pub fn stars(stars: Option<u32>) -> String {
    match stars {
        None => "-".into(),
        Some(n @ 0..=999) => n.to_string(),
        Some(n @ 1_000..=9_949) => format!("{:.1}k", f64::from(n) / 1e3),
        Some(n @ 9_950..=999_499) => format!("{:.0}k", f64::from(n) / 1e3),
        Some(n) => format!("{:.1}M", f64::from(n) / 1e6),
    }
}

/// Time since the last push: `today`, `5d`, `3mo`, `2y`.
pub fn age(pushed_days: Option<u32>, today_days: u32) -> String {
    match pushed_days.map(|d| today_days.saturating_sub(d)) {
        None => "-".into(),
        Some(0) => "today".into(),
        Some(d @ 1..=59) => format!("{d}d"),
        Some(d @ 60..=729) => format!("{}mo", d / 30),
        Some(d) => format!("{}y", d / 365),
    }
}

pub fn date(days: u32) -> String {
    let (y, m, d) = tuiman_index::date::civil_from_days(i64::from(days));
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn star_counts() {
        let cases =
            [(None, "-"), (Some(0), "0"), (Some(999), "999"), (Some(1_000), "1.0k"), (Some(9_949), "9.9k")];
        for (n, want) in cases {
            assert_eq!(stars(n), want);
        }
        assert_eq!(stars(Some(9_950)), "10k");
        assert_eq!(stars(Some(48_312)), "48k");
        assert_eq!(stars(Some(1_100_000)), "1.1M");
    }

    #[test]
    fn ages() {
        let at = |pushed| age(pushed, 20_000);
        assert_eq!(at(None), "-");
        assert_eq!(at(Some(20_000)), "today");
        assert_eq!(at(Some(20_001)), "today", "clock skew never goes negative");
        assert_eq!(at(Some(19_995)), "5d");
        assert_eq!(at(Some(19_900)), "3mo");
        assert_eq!(at(Some(19_000)), "2y");
    }

    #[test]
    fn dates() {
        assert_eq!(date(20_717), "2026-09-21");
    }
}
