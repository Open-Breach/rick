//! Human-readable sizes, dates and types.

use std::time::SystemTime;

use chrono::{DateTime, Datelike, Local, Locale, TimeZone};
use ext::t;
use model::DateFormat;
use types::EntryKind;

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

/// A date as the setting writes it (SPEC.md *Dates*), and exactly. Returns
/// `(written, exact)`: the first is what a date column, the File info
/// card, a footer and a tooltip show; the second, `YYYY-MM-DD HH:MM:SS`
/// whatever the setting, is the tooltip of a date cell itself. Both are
/// empty for an unknown time.
#[must_use]
pub fn date(time: Option<SystemTime>, now: SystemTime, format: DateFormat) -> (String, String) {
    let Some(time) = time else {
        return (String::new(), String::new());
    };
    let time: DateTime<Local> = time.into();
    let now: DateTime<Local> = now.into();
    let exact = time.format("%Y-%m-%d %H:%M:%S").to_string();
    (written(&time, &now, format, system_locale()), exact)
}

/// How dates are written right now: the setting, and the moment today is
/// counted from.
#[derive(Debug, Clone, Copy)]
pub struct Dates {
    /// The moment *Today* and *Yesterday* are counted from.
    pub now: SystemTime,
    /// The setting.
    pub format: DateFormat,
}

impl Dates {
    /// The dates of this moment, as `settings` write them.
    #[must_use]
    pub fn of(settings: &model::Settings) -> Self {
        Self {
            now: SystemTime::now(),
            format: settings.date_format,
        }
    }

    /// [`date`] of `time`: as the setting writes it, and exactly.
    #[must_use]
    pub fn write(&self, time: Option<SystemTime>) -> (String, String) {
        date(time, self.now, self.format)
    }

    /// One date where it stands on its own, as the setting writes it: a
    /// row of the File info card.
    #[must_use]
    pub fn text(&self, time: SystemTime) -> String {
        self.write(Some(time)).0
    }
}

/// A time exactly, `YYYY-MM-DD HH:MM:SS` in local time, whatever the
/// setting.
#[must_use]
pub fn exact(time: SystemTime) -> String {
    let time: DateTime<Local> = time.into();
    time.format("%Y-%m-%d %H:%M:%S").to_string()
}

/// A date as the paste-conflict dialogue writes it, which keeps the format
/// it was drawn with (SPEC.md *Dates*): *Today 15:58*, *Yesterday 09:17*,
/// the weekday with the clock within a week, then the month and the day,
/// and the ISO date for another year. Empty for an unknown time.
#[must_use]
pub fn brief(time: Option<SystemTime>, now: SystemTime) -> String {
    let Some(time) = time else {
        return String::new();
    };
    let time: DateTime<Local> = time.into();
    let now: DateTime<Local> = now.into();
    let days_ago = now
        .date_naive()
        .signed_duration_since(time.date_naive())
        .num_days();
    match days_ago {
        0 => format!("Today {}", time.format("%H:%M")),
        1 => format!("Yesterday {}", time.format("%H:%M")),
        2..=6 => time.format("%a %H:%M").to_string(),
        _ if time.year() == now.year() => time.format("%b %-d").to_string(),
        _ => time.format("%Y-%m-%d").to_string(),
    }
}

/// `time` as `format` writes it on the day of `now`, `locale` being the
/// system's for [`DateFormat::System`]. Days are calendar days in the
/// times' own zone, and the clock has 24 hours but for the system's.
fn written<Tz: TimeZone>(
    time: &DateTime<Tz>,
    now: &DateTime<Tz>,
    format: DateFormat,
    locale: Locale,
) -> String
where
    Tz::Offset: std::fmt::Display,
{
    match format {
        DateFormat::SmartTime => format!("{} {}", smart_day(time, now), time.format("%H:%M")),
        DateFormat::Smart => smart_day(time, now),
        DateFormat::Iso => time.format("%Y-%m-%d %H:%M").to_string(),
        DateFormat::System => system_date(time, locale),
    }
}

/// The day of `time` as the smart formats name it: *Today*, *Yesterday*,
/// the weekday for the two to six days before today, then the month and
/// the day, with the year when it is not this year. A day after today is
/// written as a date, never as a weekday.
fn smart_day<Tz: TimeZone>(time: &DateTime<Tz>, now: &DateTime<Tz>) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let days_ago = now
        .date_naive()
        .signed_duration_since(time.date_naive())
        .num_days();
    match days_ago {
        0 => "Today".to_owned(),
        1 => "Yesterday".to_owned(),
        2..=6 => time.format("%a").to_string(),
        _ if time.year() == now.year() => time.format("%b %-d").to_string(),
        _ => time.format("%b %-d %Y").to_string(),
    }
}

/// The short date and time of `locale`: its date as the locale writes it
/// and the clock without seconds, with 12 or 24 hours as the locale has
/// them.
fn system_date<Tz: TimeZone>(time: &DateTime<Tz>, locale: Locale) -> String
where
    Tz::Offset: std::fmt::Display,
{
    // The locale's own time format carries seconds; whether it counts
    // twelve hours shows in how it writes one o'clock in the afternoon.
    let afternoon = chrono::Utc
        .with_ymd_and_hms(2000, 1, 1, 13, 0, 0)
        .single()
        .map_or_default(|one| one.format_localized("%X", locale).to_string());
    let clock = if afternoon.contains("13") {
        "%H:%M"
    } else {
        "%I:%M %p"
    };
    format!(
        "{} {}",
        time.format_localized("%x", locale),
        time.format_localized(clock, locale)
    )
}

/// The locale the *System* date format writes in: the one the session
/// names for dates (`LC_ALL`, `LC_TIME`, `LANG`, in the order POSIX gives
/// them), else the system's preferred one; looked up once.
fn system_locale() -> Locale {
    static LOCALE: std::sync::OnceLock<Locale> = std::sync::OnceLock::new();
    *LOCALE.get_or_init(|| {
        ["LC_ALL", "LC_TIME", "LANG"]
            .into_iter()
            .filter_map(|key| std::env::var(key).ok())
            .find(|value| !value.is_empty())
            .or_else(sys_locale::get_locale)
            .map_or(Locale::POSIX, |name| locale_named(&name))
    })
}

/// The locale a name stands for, as the environment writes it
/// (`de_CH.UTF-8`, `sr_RS.UTF-8@latin`, `C`) or as a language tag
/// (`de-CH`, `de`); the POSIX locale for a name that is none.
fn locale_named(name: &str) -> Locale {
    // The encoding is not part of the locale's name; a modifier is.
    let (base, modifier) = name
        .split_once('@')
        .map_or((name, None), |(b, m)| (b, Some(m)));
    let base = base.split('.').next().unwrap_or(base).replace('-', "_");
    let with_modifier =
        |base: &str| modifier.map_or_else(|| base.to_owned(), |m| format!("{base}@{m}"));
    let mut candidates = vec![with_modifier(&base), base.clone()];
    if !base.contains('_') {
        // A language without a country: the country of the same name
        // (`de_DE`, `fr_FR`), and the United States for English.
        candidates.push(format!("{base}_{}", base.to_uppercase()));
        if base == "en" {
            candidates.push("en_US".to_owned());
        }
    }
    candidates
        .iter()
        .find_map(|candidate| Locale::try_from(candidate.as_str()).ok())
        .unwrap_or(Locale::POSIX)
}

/// A time as `YYYY-MM-DD HH:MM` in local time: a date where it stands on
/// its own, as in the Properties dialogue.
#[must_use]
pub fn stamp(time: SystemTime) -> String {
    let time: DateTime<Local> = time.into();
    time.format("%Y-%m-%d %H:%M").to_string()
}

/// The type of an entry in words, as the head of a dialogue names it:
/// *Folder*, *Symbolic link*, *JPEG image*, *PDF document*, *Executable
/// file*. A file is named after the family of its extension; one in no
/// family is an *Executable file* when `executable` is set, else its
/// extension and *file*, or plainly *File* (QUESTIONS.md 352).
#[must_use]
pub fn type_name(kind: &EntryKind, name: &str, executable: bool) -> String {
    named_type(kind, name, executable, false)
}

/// [`type_name`] for a file that is known to be no text — no previewer
/// took it for one: where the extension names no family and the file is
/// not executable it is a *Binary file*.
#[must_use]
pub fn binary_type_name(kind: &EntryKind, name: &str, executable: bool) -> String {
    named_type(kind, name, executable, true)
}

fn named_type(kind: &EntryKind, name: &str, executable: bool, binary: bool) -> String {
    match kind {
        EntryKind::Dir => return t!("type-folder"),
        EntryKind::Symlink { .. } => return t!("type-symbolic-link"),
        EntryKind::Other => return t!("type-special-file"),
        EntryKind::File => {}
    }
    let ext = name
        .rsplit_once('.')
        .filter(|(stem, ext)| !stem.is_empty() && !ext.is_empty())
        .map(|(_, ext)| ext.to_ascii_lowercase());
    let Some(ext) = ext else {
        return if executable {
            t!("type-executable")
        } else if binary {
            t!("type-binary")
        } else {
            t!("type-file")
        };
    };
    // The format's own name where it is not the extension.
    let format = match ext.as_str() {
        "jpg" => "JPEG".to_owned(),
        "tif" => "TIFF".to_owned(),
        other => other.to_uppercase(),
    };
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "tif" | "tiff" | "ico" | "heic"
        | "avif" | "svg" => t!("type-image", format = format),
        "mp4" | "mkv" | "mov" | "avi" | "webm" | "m4v" => t!("type-video", format = format),
        "mp3" | "flac" | "wav" | "ogg" | "m4a" | "aac" | "opus" => {
            t!("type-audio", format = format)
        }
        "zip" | "7z" | "rar" | "tar" | "gz" | "bz2" | "xz" | "zst" | "tgz" => {
            t!("type-archive", format = format)
        }
        "pdf" | "doc" | "docx" | "odt" | "rtf" | "xls" | "xlsx" | "ods" | "ppt" | "pptx"
        | "odp" => t!("type-document", format = format),
        _ if executable => t!("type-executable"),
        _ if binary => t!("type-binary"),
        _ => t!("type-file-of", format = format),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_type_is_named_after_its_kind_then_its_family_then_its_extension() {
        crate::english();
        let file = EntryKind::File;
        assert_eq!(type_name(&EntryKind::Dir, "photos.d", false), "Folder");
        assert_eq!(
            type_name(
                &EntryKind::Symlink {
                    target: types::VPath::from("/x"),
                    broken: false
                },
                "rider",
                true
            ),
            "Symbolic link"
        );
        assert_eq!(type_name(&file, "IMG_2041.jpg", false), "JPEG image");
        assert_eq!(type_name(&file, "sign.PDF", false), "PDF document");
        assert_eq!(type_name(&file, "clip.mkv", false), "MKV video");
        assert_eq!(type_name(&file, "backup.tar", true), "TAR archive");
        assert_eq!(type_name(&file, "rider", true), "Executable file");
        assert_eq!(type_name(&file, "build.sh", true), "Executable file");
        assert_eq!(type_name(&file, "random-4k.bin", false), "BIN file");
        assert_eq!(type_name(&file, "Makefile", false), "File");
        assert_eq!(type_name(&file, ".profile", false), "File");
        // No previewer took it for text: it is binary, whatever it is called.
        assert_eq!(
            binary_type_name(&file, "random-4k.bin", false),
            "Binary file"
        );
        assert_eq!(binary_type_name(&file, "clip.mkv", false), "MKV video");
        assert_eq!(binary_type_name(&file, "rider", true), "Executable file");
    }

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
        let smart = DateFormat::SmartTime;
        assert_eq!(date(None, now, smart), (String::new(), String::new()));
        let (today, full) = date(Some(now - Duration::from_mins(1)), now, smart);
        assert!(today.starts_with("Today "), "{today}");
        assert_eq!(full.len(), 19);
        let (yesterday, _) = date(Some(now - Duration::from_hours(24)), now, smart);
        assert!(yesterday.starts_with("Yesterday "), "{yesterday}");
        // The exact value is the same whatever the setting.
        let (iso, exact) = date(Some(now), now, DateFormat::Iso);
        assert_eq!(iso.len(), 16, "{iso}");
        assert!(exact.starts_with(&iso), "{exact} / {iso}");
        assert_eq!(exact, super::exact(now));
        // The paste-conflict dialogue keeps the format it had.
        assert!(brief(Some(now), now).starts_with("Today "));
        let old = brief(Some(now - Duration::from_hours(5 * 365 * 24)), now);
        assert_eq!(old.len(), 10, "{old}");
        assert_eq!(brief(None, now), "");
    }

    /// A moment in UTC, for dates that read the same on every machine.
    fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<chrono::Utc> {
        chrono::Utc
            .with_ymd_and_hms(year, month, day, hour, minute, 7)
            .single()
            .unwrap_or_else(|| panic!("no such moment"))
    }

    /// Issue 149: the four date formats, as SPEC.md *Dates* writes them.
    #[test]
    fn the_four_date_formats_write_a_date_as_the_spec_does() {
        // Today is Thursday 2026-08-27.
        let now = at(2026, 8, 27, 16, 30);
        let cases = [
            (at(2026, 8, 27, 15, 58), "Today 15:58", "Today"),
            (at(2026, 8, 26, 9, 17), "Yesterday 09:17", "Yesterday"),
            // Two to six days back: the weekday.
            (at(2026, 8, 25, 21, 24), "Tue 21:24", "Tue"),
            (at(2026, 8, 21, 0, 5), "Fri 00:05", "Fri"),
            // Seven days back would be today's weekday again: the date.
            (at(2026, 8, 20, 14, 12), "Aug 20 14:12", "Aug 20"),
            (at(2026, 1, 3, 14, 12), "Jan 3 14:12", "Jan 3"),
            // Not this year: with the year.
            (at(2025, 8, 24, 14, 12), "Aug 24 2025 14:12", "Aug 24 2025"),
            // After today: a date, never a weekday.
            (at(2026, 8, 29, 8, 0), "Aug 29 08:00", "Aug 29"),
            (at(2027, 1, 1, 8, 0), "Jan 1 2027 08:00", "Jan 1 2027"),
        ];
        for (time, with_time, without) in cases {
            let write = |format| written(&time, &now, format, Locale::POSIX);
            assert_eq!(write(DateFormat::SmartTime), with_time);
            assert_eq!(write(DateFormat::Smart), without);
        }
        let time = at(2026, 8, 24, 14, 12);
        assert_eq!(
            written(&time, &now, DateFormat::Iso, Locale::POSIX),
            "2026-08-24 14:12"
        );
        // Yesterday is a calendar day, not 24 hours: a minute before
        // midnight is yesterday a minute after it.
        let after_midnight = at(2026, 8, 27, 0, 1);
        assert_eq!(
            written(
                &at(2026, 8, 26, 23, 59),
                &after_midnight,
                DateFormat::SmartTime,
                Locale::POSIX
            ),
            "Yesterday 23:59"
        );
    }

    /// The *System* format follows the locale: the United States write the
    /// month first and count twelve hours, Germany and Britain the day
    /// first and twenty-four.
    #[test]
    fn the_system_date_format_follows_the_locale() {
        let now = at(2026, 8, 27, 16, 30);
        let time = at(2026, 8, 24, 14, 12);
        let write = |locale| written(&time, &now, DateFormat::System, locale);
        assert_eq!(write(Locale::en_US), "08/24/2026 02:12 PM");
        assert_eq!(write(Locale::de_DE), "24.08.2026 14:12");
        assert_eq!(write(Locale::en_GB), "24/08/26 14:12");
    }

    #[test]
    fn a_locale_is_found_by_the_name_the_environment_or_the_system_gives() {
        assert_eq!(locale_named("de_CH.UTF-8"), Locale::de_CH);
        assert_eq!(locale_named("de-CH"), Locale::de_CH);
        assert_eq!(locale_named("en_US"), Locale::en_US);
        assert_eq!(locale_named("fr"), Locale::fr_FR);
        assert_eq!(locale_named("en"), Locale::en_US);
        assert_eq!(locale_named("sr_RS.UTF-8@latin"), Locale::sr_RS_latin);
        assert_eq!(locale_named("C"), Locale::POSIX);
        assert_eq!(locale_named("C.UTF-8"), Locale::POSIX);
        assert_eq!(locale_named("klingon"), Locale::POSIX);
    }
}
