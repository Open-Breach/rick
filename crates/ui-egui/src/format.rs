//! Human-readable sizes, dates and types.

use std::time::SystemTime;

use chrono::{DateTime, Datelike, Local};
use ext::t;
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
        assert_eq!(date(None, now), (String::new(), String::new()));
        let (today, full) = date(Some(now - Duration::from_mins(1)), now);
        assert!(today.starts_with("Today "), "{today}");
        assert_eq!(full.len(), 19);
        let (yesterday, _) = date(Some(now - Duration::from_hours(24)), now);
        assert!(yesterday.starts_with("Yesterday "), "{yesterday}");
        let (old, _) = date(Some(now - Duration::from_hours(5 * 365 * 24)), now);
        assert_eq!(old.len(), 10, "{old}");
    }
}
