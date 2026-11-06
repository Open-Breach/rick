//! Human-readable sizes and dates.

use std::time::SystemTime;

use chrono::{DateTime, Datelike, Local};

/// `1.2 MiB`, `640 B`, or an em dash for an unknown size.
#[must_use]
pub fn size(bytes: Option<u64>) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    let Some(bytes) = bytes else {
        return "—".to_owned();
    };
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// A date as the list shows it: relative for recent entries, full on hover.
/// Returns `(short, full)`; both empty for an unknown time.
#[must_use]
pub fn date(time: Option<SystemTime>, now: SystemTime) -> (String, String) {
    let Some(time) = time else {
        return (String::new(), String::new());
    };
    let time: DateTime<Local> = time.into();
    let now: DateTime<Local> = now.into();
    let full = time.format("%Y-%m-%d %H:%M:%S").to_string();
    let days_ago = now
        .date_naive()
        .signed_duration_since(time.date_naive())
        .num_days();
    let short = match days_ago {
        0 => format!("Today {}", time.format("%H:%M")),
        1 => format!("Yesterday {}", time.format("%H:%M")),
        2..=6 => time.format("%a %H:%M").to_string(),
        _ if time.year() == now.year() => time.format("%b %-d").to_string(),
        _ => time.format("%Y-%m-%d").to_string(),
    };
    (short, full)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn sizes_pick_a_unit() {
        assert_eq!(size(None), "—");
        assert_eq!(size(Some(0)), "0 B");
        assert_eq!(size(Some(1023)), "1023 B");
        assert_eq!(size(Some(1024)), "1.0 KiB");
        assert_eq!(size(Some(1536)), "1.5 KiB");
        assert_eq!(size(Some(123 * 1024 * 1024)), "123 MiB");
        assert_eq!(size(Some(5 * 1024 * 1024 * 1024)), "5.0 GiB");
    }

    #[test]
    fn dates_are_relative_when_recent() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000 + 12 * 3600);
        assert_eq!(date(None, now), (String::new(), String::new()));
        let (today, full) = date(Some(now - Duration::from_secs(60)), now);
        assert!(today.starts_with("Today "), "{today}");
        assert_eq!(full.len(), 19);
        let (yesterday, _) = date(Some(now - Duration::from_hours(24)), now);
        assert!(yesterday.starts_with("Yesterday "), "{yesterday}");
        let (old, _) = date(Some(now - Duration::from_hours(5 * 365 * 24)), now);
        assert_eq!(old.len(), 10, "{old}");
    }
}
