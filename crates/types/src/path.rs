//! Provider-neutral paths and locations.

use std::borrow::Cow;
use std::ffi::OsStr;
use std::fmt;
use std::path::Path;

use crate::ProviderId;

/// Provider-neutral path. Keeps the raw bytes a provider handed out and offers
/// a lossy display form.
///
/// The byte representation is the one `std` uses for `OsStr`
/// ([`OsStr::as_encoded_bytes`]): the raw bytes of the name on Unix, and WTF-8
/// (UTF-8 generalised to encode unpaired surrogates) for the UTF-16 names of
/// Windows. Both are stored without loss: arbitrary bytes round-trip through
/// [`VPath::from_bytes`] / [`VPath::as_bytes`], arbitrary UTF-16 round-trips
/// through [`VPath::from_wide`] / [`VPath::to_wide`].
///
/// `Display` renders the lossy form (invalid sequences become U+FFFD);
/// `Debug` shows the string when it is valid UTF-8 and the escaped bytes
/// otherwise, so nothing is hidden in logs.
#[derive(Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VPath(Vec<u8>);

impl VPath {
    /// The empty path.
    #[must_use]
    pub const fn new() -> Self {
        Self(Vec::new())
    }

    /// Wraps raw bytes without inspecting them.
    #[must_use]
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Self {
        Self(bytes.into())
    }

    /// Converts an OS string using the encoding `std` already holds it in.
    /// Lossless on every platform.
    #[must_use]
    pub fn from_os_str(s: &OsStr) -> Self {
        Self(s.as_encoded_bytes().to_vec())
    }

    /// Encodes a UTF-16 sequence, valid or not, as WTF-8. Well-formed surrogate
    /// pairs become one four-byte sequence; unpaired surrogates are kept as
    /// three-byte sequences, so [`VPath::to_wide`] gives the input back
    /// exactly.
    #[must_use]
    pub fn from_wide(wide: &[u16]) -> Self {
        let mut bytes = Vec::with_capacity(wide.len());
        let mut units = wide.iter().copied().peekable();
        while let Some(unit) = units.next() {
            let code_point = match (unit, units.peek()) {
                (0xD800..=0xDBFF, Some(&low @ 0xDC00..=0xDFFF)) => {
                    units.next();
                    0x1_0000 + ((u32::from(unit) - 0xD800) << 10) + (u32::from(low) - 0xDC00)
                }
                _ => u32::from(unit),
            };
            push_generalised_utf8(code_point, &mut bytes);
        }
        Self(bytes)
    }

    /// The raw bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Consumes the path and returns the raw bytes.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }

    /// Number of bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// `true` for the empty path.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// `true` when the last byte is a `/` or `\` separator.
    fn ends_with_separator(&self) -> bool {
        matches!(self.0.last(), Some(b'/' | b'\\'))
    }

    /// The path with `name` appended as one more component, separated by `/`.
    /// Providers treat `/` as the separator; the local provider on Windows
    /// accepts it next to `\`.
    #[must_use]
    pub fn join(&self, name: impl AsRef<[u8]>) -> Self {
        let name = name.as_ref();
        let mut bytes = Vec::with_capacity(self.0.len() + name.len() + 1);
        bytes.extend_from_slice(&self.0);
        if !self.0.is_empty() && !self.ends_with_separator() {
            bytes.push(b'/');
        }
        bytes.extend_from_slice(name);
        Self(bytes)
    }

    /// The last component: the bytes after the last `/` or `\`, ignoring a
    /// trailing separator. `None` for the empty path and for a bare root.
    #[must_use]
    pub fn file_name(&self) -> Option<&[u8]> {
        let trimmed = match self.0.split_last() {
            Some((b'/' | b'\\', rest)) => rest,
            _ => &self.0[..],
        };
        let start = trimmed
            .iter()
            .rposition(|b| matches!(b, b'/' | b'\\'))
            .map_or(0, |i| i + 1);
        let name = &trimmed[start..];
        (!name.is_empty()).then_some(name)
    }

    /// The path as text. Borrows when the bytes are valid UTF-8; otherwise
    /// every unpaired surrogate (from a Windows name) and every other invalid
    /// sequence becomes one U+FFFD.
    #[must_use]
    pub fn to_string_lossy(&self) -> Cow<'_, str> {
        let mut rest = &self.0[..];
        let mut out = match std::str::from_utf8(rest) {
            Ok(text) => return Cow::Borrowed(text),
            Err(_) => String::with_capacity(rest.len()),
        };
        loop {
            match std::str::from_utf8(rest) {
                Ok(text) => {
                    out.push_str(text);
                    return Cow::Owned(out);
                }
                Err(e) => {
                    let (valid, invalid) = rest.split_at(e.valid_up_to());
                    // `valid_up_to` bytes are valid UTF-8 by construction.
                    out.push_str(&String::from_utf8_lossy(valid));
                    out.push('\u{FFFD}');
                    let skip = match invalid {
                        // A WTF-8 surrogate: ED A0..BF 80..BF.
                        [0xED, 0xA0..=0xBF, 0x80..=0xBF, ..] => 3,
                        _ => e.error_len().unwrap_or(invalid.len()).max(1),
                    };
                    rest = &invalid[skip..];
                }
            }
        }
    }

    /// Decodes the bytes as WTF-8 into UTF-16 code units. Returns `None` when
    /// the bytes are not WTF-8, which can only happen for paths built from raw
    /// non-UTF-8 bytes on Unix.
    #[must_use]
    pub fn to_wide(&self) -> Option<Vec<u16>> {
        let bytes = &self.0;
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while let Some(&lead) = bytes.get(i) {
            let (len, mut code_point) = match lead {
                0x00..=0x7F => (1, u32::from(lead)),
                0xC2..=0xDF => (2, u32::from(lead & 0x1F)),
                0xE0..=0xEF => (3, u32::from(lead & 0x0F)),
                0xF0..=0xF4 => (4, u32::from(lead & 0x07)),
                _ => return None,
            };
            let sequence = bytes.get(i..i + len)?;
            for &trail in &sequence[1..] {
                if trail & 0xC0 != 0x80 {
                    return None;
                }
                code_point = (code_point << 6) | u32::from(trail & 0x3F);
            }
            let in_range = match len {
                1 => true,
                2 => code_point >= 0x80,
                3 => code_point >= 0x800,
                _ => (0x1_0000..=0x10_FFFF).contains(&code_point),
            };
            if !in_range {
                return None;
            }
            if let Some(supplementary) = code_point.checked_sub(0x1_0000) {
                out.push(u16::try_from(0xD800 + (supplementary >> 10)).ok()?);
                out.push(u16::try_from(0xDC00 + (supplementary & 0x3FF)).ok()?);
            } else {
                out.push(u16::try_from(code_point).ok()?);
            }
            i += len;
        }
        Some(out)
    }
}

/// Appends `code_point` in generalised UTF-8, which, unlike UTF-8 proper, also
/// encodes the surrogate range U+D800..=U+DFFF.
fn push_generalised_utf8(code_point: u32, out: &mut Vec<u8>) {
    // Every shift leaves at most six significant bits, so the narrowing is exact.
    #[allow(clippy::cast_possible_truncation)]
    let byte = |bits: u32| bits as u8;
    match code_point {
        0..=0x7F => out.push(byte(code_point)),
        0x80..=0x7FF => out.extend_from_slice(&[
            byte(0xC0 | (code_point >> 6)),
            byte(0x80 | (code_point & 0x3F)),
        ]),
        0x800..=0xFFFF => out.extend_from_slice(&[
            byte(0xE0 | (code_point >> 12)),
            byte(0x80 | ((code_point >> 6) & 0x3F)),
            byte(0x80 | (code_point & 0x3F)),
        ]),
        _ => out.extend_from_slice(&[
            byte(0xF0 | (code_point >> 18)),
            byte(0x80 | ((code_point >> 12) & 0x3F)),
            byte(0x80 | ((code_point >> 6) & 0x3F)),
            byte(0x80 | (code_point & 0x3F)),
        ]),
    }
}

impl fmt::Display for VPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_string_lossy())
    }
}

impl fmt::Debug for VPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match std::str::from_utf8(&self.0) {
            Ok(text) => write!(f, "VPath({text:?})"),
            Err(_) => write!(f, "VPath(b\"{}\")", self.0.escape_ascii()),
        }
    }
}

impl From<Vec<u8>> for VPath {
    fn from(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }
}

impl From<&[u8]> for VPath {
    fn from(bytes: &[u8]) -> Self {
        Self(bytes.to_vec())
    }
}

impl From<&str> for VPath {
    fn from(text: &str) -> Self {
        Self(text.as_bytes().to_vec())
    }
}

impl From<String> for VPath {
    fn from(text: String) -> Self {
        Self(text.into_bytes())
    }
}

impl From<&OsStr> for VPath {
    fn from(s: &OsStr) -> Self {
        Self::from_os_str(s)
    }
}

impl From<&Path> for VPath {
    fn from(path: &Path) -> Self {
        Self::from_os_str(path.as_os_str())
    }
}

/// A place a pane can show: a path inside one provider's tree.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Location {
    /// The provider whose tree `path` belongs to.
    pub provider: ProviderId,
    /// The path inside that tree.
    pub path: VPath,
}

impl Location {
    /// Builds a location.
    #[must_use]
    pub fn new(provider: ProviderId, path: impl Into<VPath>) -> Self {
        Self {
            provider,
            path: path.into(),
        }
    }
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.provider, self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_bytes_round_trip() {
        let bytes: Vec<u8> = (0..=255).collect();
        let path = VPath::from_bytes(bytes.clone());
        assert_eq!(path.as_bytes(), &bytes[..]);
        assert_eq!(path.clone().into_bytes(), bytes);
        assert_eq!(path.len(), 256);
        assert!(!path.is_empty());
        assert!(VPath::new().is_empty());
    }

    #[test]
    fn utf8_text_round_trips_as_utf8_and_utf16() {
        let text = "héllo wörld/ファイル/😀.txt";
        let path = VPath::from(text);
        assert_eq!(path.to_string_lossy(), text);
        assert_eq!(path.to_string(), text);
        let wide: Vec<u16> = text.encode_utf16().collect();
        assert_eq!(VPath::from_wide(&wide), path);
        assert_eq!(path.to_wide().as_deref(), Some(&wide[..]));
    }

    #[test]
    fn unpaired_surrogates_round_trip_through_wide() {
        // Lone high before a non-surrogate, lone low, a valid pair, a lone
        // high at the very end.
        let wide = [0x0061, 0xD800, 0x0062, 0xDC00, 0xD83D, 0xDE00, 0xDBFF];
        let path = VPath::from_wide(&wide);
        assert_eq!(path.to_wide().as_deref(), Some(&wide[..]));
        // The lossy form replaces each of the three unpaired units with one
        // U+FFFD and keeps the rest.
        assert_eq!(path.to_string_lossy(), "a\u{FFFD}b\u{FFFD}😀\u{FFFD}");
    }

    #[test]
    fn every_single_unit_and_pair_round_trips() {
        for unit in 0..=u16::MAX {
            let wide = [unit];
            assert_eq!(
                VPath::from_wide(&wide).to_wide().as_deref(),
                Some(&wide[..])
            );
        }
        for high in 0xD800..=0xDBFFu16 {
            let wide = [high, 0xDC00, high, 0xDFFF];
            assert_eq!(
                VPath::from_wide(&wide).to_wide().as_deref(),
                Some(&wide[..])
            );
        }
    }

    #[test]
    fn invalid_wtf8_has_no_wide_form() {
        assert_eq!(VPath::from_bytes(vec![0xFF]).to_wide(), None);
        assert_eq!(VPath::from_bytes(vec![0xC0, 0x80]).to_wide(), None); // overlong
        assert_eq!(VPath::from_bytes(vec![0xE0, 0x80, 0x80]).to_wide(), None); // overlong
        assert_eq!(
            VPath::from_bytes(vec![0xF4, 0x90, 0x80, 0x80]).to_wide(),
            None
        ); // > U+10FFFF
        assert_eq!(VPath::from_bytes(vec![0xE2, 0x82]).to_wide(), None); // truncated
        assert_eq!(VPath::from_bytes(vec![0xE2, 0x41, 0x82]).to_wide(), None); // bad trail
    }

    #[test]
    fn lossy_text_marks_invalid_bytes_once_each() {
        assert_eq!(
            VPath::from_bytes(vec![b'a', 0xFF, 0xFE, b'b']).to_string_lossy(),
            "a\u{FFFD}\u{FFFD}b"
        );
        // A truncated three-byte sequence at the end is one replacement.
        assert_eq!(
            VPath::from_bytes(vec![b'a', 0xE2, 0x82]).to_string_lossy(),
            "a\u{FFFD}"
        );
        assert!(matches!(
            VPath::from("plain").to_string_lossy(),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn os_str_conversion_matches_std_encoding() {
        let os = OsStr::new("some/dir/file.rs");
        assert_eq!(VPath::from(os).as_bytes(), os.as_encoded_bytes());
        assert_eq!(VPath::from(Path::new("a/b")).to_string(), "a/b");
    }

    #[test]
    fn debug_shows_text_or_escaped_bytes() {
        assert_eq!(format!("{:?}", VPath::from("a b")), "VPath(\"a b\")");
        assert_eq!(
            format!("{:?}", VPath::from_bytes(vec![b'a', 0xFF])),
            "VPath(b\"a\\xff\")"
        );
    }

    #[test]
    fn join_and_file_name_follow_slashes_and_backslashes() {
        assert_eq!(VPath::from("/a/b").join("c").to_string(), "/a/b/c");
        assert_eq!(VPath::from("/a/b/").join("c").to_string(), "/a/b/c");
        assert_eq!(VPath::from("").join("c").to_string(), "c");
        assert_eq!(VPath::from("C:\\x\\").join("c").to_string(), "C:\\x\\c");
        assert_eq!(VPath::from("C:\\x").join("c").to_string(), "C:\\x/c");
        assert_eq!(VPath::from("/a/b/c.txt").file_name(), Some(&b"c.txt"[..]));
        assert_eq!(VPath::from("/a/b/").file_name(), Some(&b"b"[..]));
        assert_eq!(VPath::from("C:\\x\\y").file_name(), Some(&b"y"[..]));
        assert_eq!(VPath::from("name").file_name(), Some(&b"name"[..]));
        assert_eq!(VPath::from("/").file_name(), None);
        assert_eq!(VPath::new().file_name(), None);
    }

    #[test]
    fn location_displays_provider_and_path() {
        let loc = Location::new(ProviderId("local"), "/home");
        assert_eq!(loc.to_string(), "local:/home");
    }
}
