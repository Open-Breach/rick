//! The fonts the desktop sets its own windows in, for the interface (SPEC.md
//! *Look and feel*, Typography): the system's UI font and its monospace
//! font as files, found the way each operating system names them. Nothing
//! here parses a font; the caller validates the bytes before it uses them
//! and falls back to its bundled fonts for whatever is missing.

use std::path::Path;

use crate::traits::{FontFile, Fonts};

/// The system's UI and monospace fonts, each when this operating system
/// can name one: fontconfig's `sans-serif` and `monospace` matches on
/// Linux, the San Francisco files (Helvetica Neue and Menlo before them as
/// fallbacks) on macOS, Segoe UI and Consolas on Windows.
#[must_use]
pub fn system() -> Fonts {
    #[cfg(target_os = "linux")]
    {
        Fonts {
            ui: fc_match("sans-serif"),
            mono: fc_match("monospace"),
        }
    }
    #[cfg(target_os = "macos")]
    {
        Fonts {
            ui: first_of(&[
                ("San Francisco", "/System/Library/Fonts/SFNS.ttf"),
                ("San Francisco", "/System/Library/Fonts/SFNSText.ttf"),
                ("Helvetica Neue", "/System/Library/Fonts/HelveticaNeue.ttc"),
            ]),
            mono: first_of(&[
                ("SF Mono", "/System/Library/Fonts/SFNSMono.ttf"),
                ("Menlo", "/System/Library/Fonts/Menlo.ttc"),
                ("Monaco", "/System/Library/Fonts/Monaco.ttf"),
            ]),
        }
    }
    #[cfg(windows)]
    {
        let dir = std::env::var_os("WINDIR")
            .map_or_else(|| std::path::PathBuf::from(r"C:\Windows"), Into::into)
            .join("Fonts");
        let file = |name: &str| dir.join(name).display().to_string();
        Fonts {
            ui: first_of(&[("Segoe UI", &file("segoeui.ttf"))]),
            mono: first_of(&[
                ("Cascadia Mono", &file("CascadiaMono.ttf")),
                ("Consolas", &file("consola.ttf")),
            ]),
        }
    }
}

/// Reads a font file when it exists and starts like a font.
fn read(family: &str, path: &Path, index: u32) -> Option<FontFile> {
    let bytes = std::fs::read(path).ok()?;
    looks_like_font(&bytes).then(|| FontFile {
        family: family.to_owned(),
        bytes,
        index,
    })
}

/// The first of the candidates that is on disk.
#[cfg(any(target_os = "macos", windows))]
fn first_of(candidates: &[(&str, &str)]) -> Option<FontFile> {
    candidates
        .iter()
        .find_map(|(family, path)| read(family, Path::new(path), 0))
}

/// fontconfig's best match for a pattern (`sans-serif`, `monospace`): the
/// file, the face index in it and the family, through `fc-match`, which
/// every desktop with fontconfig has; nothing without it.
#[cfg(target_os = "linux")]
fn fc_match(pattern: &str) -> Option<FontFile> {
    let output = std::process::Command::new("fc-match")
        .arg("-f")
        .arg("%{file}\t%{index}\t%{family}")
        .arg(pattern)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let mut parts = text.trim_end().splitn(3, '\t');
    let file = parts.next()?;
    let index = parts.next()?.trim().parse().unwrap_or(0);
    // fontconfig lists every name of the family, comma-separated; the
    // first is the one the desktop shows.
    let family = parts
        .next()
        .unwrap_or_default()
        .split(',')
        .next()
        .unwrap_or_default()
        .trim();
    read(family, Path::new(file), index)
}

/// Whether the bytes start as a TrueType, OpenType or collection file does;
/// a cheap check before the caller parses them for real.
#[must_use]
pub fn looks_like_font(bytes: &[u8]) -> bool {
    matches!(
        bytes.get(..4),
        Some([0x00, 0x01, 0x00, 0x00] | b"OTTO" | b"true" | b"ttcf")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_magic_numbers_of_the_font_formats_pass_and_text_does_not() {
        assert!(looks_like_font(&[0, 1, 0, 0, 9, 9]));
        assert!(looks_like_font(b"OTTO...."));
        assert!(looks_like_font(b"true...."));
        assert!(looks_like_font(b"ttcf...."));
        assert!(!looks_like_font(b"<svg"));
        assert!(!looks_like_font(b"\x00\x01"));
        assert!(!looks_like_font(b""));
    }

    #[test]
    fn what_the_system_names_is_a_font_file_or_nothing() {
        let fonts = system();
        for file in [fonts.ui, fonts.mono].into_iter().flatten() {
            assert!(looks_like_font(&file.bytes), "{}", file.family);
            assert!(!file.family.is_empty());
        }
    }

    #[test]
    fn a_missing_file_is_none() {
        assert!(read("Nowhere", Path::new("/nowhere/at/all.ttf"), 0).is_none());
    }
}
