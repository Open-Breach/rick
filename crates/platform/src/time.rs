//! Calendar conversion without a dependency: `SystemTime` to the
//! `YYYY-MM-DDTHH:MM:SS` form the XDG trash spec and the properties rows use,
//! and back.

use std::time::{Duration, SystemTime};

/// Formats `time` as `YYYY-MM-DDTHH:MM:SS` in UTC. Times before 1970 render
/// as the epoch.
#[must_use]
pub fn iso8601(time: SystemTime) -> String {
    let secs = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let days = i64::try_from(secs / 86_400).unwrap_or(i64::MAX);
    let (year, month, day) = civil_from_days(days);
    let rem = secs % 86_400;
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Parses what [`iso8601`] produces, with or without a trailing `Z`.
pub(crate) fn parse_iso8601(text: &str) -> Option<SystemTime> {
    let text = text.trim().trim_end_matches('Z');
    let (date, clock) = text.split_once('T')?;
    let mut date = date.split('-');
    let (year, month, day) = (
        date.next()?.parse::<i64>().ok()?,
        date.next()?.parse::<u32>().ok()?,
        date.next()?.parse::<u32>().ok()?,
    );
    let mut clock = clock.split(':');
    let (hour, minute, second) = (
        clock.next()?.parse::<u64>().ok()?,
        clock.next()?.parse::<u64>().ok()?,
        clock.next()?.parse::<u64>().ok()?,
    );
    let days = days_from_civil(year, month, day)?;
    let secs = u64::try_from(days).ok()? * 86_400 + hour * 3600 + minute * 60 + second;
    Some(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
}

// Howard Hinnant's algorithms, valid for the proleptic Gregorian calendar.

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = u32::try_from(doy - (153 * mp + 2) / 5 + 1).unwrap_or(1);
    let month = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).unwrap_or(1);
    (if month <= 2 { year + 1 } else { year }, month, day)
}

fn days_from_civil(year: i64, month: u32, day: u32) -> Option<i64> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year.rem_euclid(400);
    let mp = i64::from(if month > 2 { month - 3 } else { month + 9 });
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_and_a_known_date_round_trip() {
        assert_eq!(iso8601(SystemTime::UNIX_EPOCH), "1970-01-01T00:00:00");
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_762_300_801); // 2025-11-05
        assert_eq!(iso8601(t), "2025-11-05T00:00:01");
        for secs in [0u64, 86_399, 951_782_400, 1_709_164_800, 4_102_444_800] {
            let t = SystemTime::UNIX_EPOCH + Duration::from_secs(secs);
            assert_eq!(parse_iso8601(&iso8601(t)), Some(t), "{secs}");
        }
        assert_eq!(
            parse_iso8601("2024-02-29T12:34:56Z"),
            parse_iso8601("2024-02-29T12:34:56")
        );
        assert_eq!(parse_iso8601("not a date"), None);
        assert_eq!(parse_iso8601("2024-13-01T00:00:00"), None);
    }
}
