//! Conversion between [`VPath`] and [`Path`].
//!
//! `VPath` stores the bytes `OsStr::as_encoded_bytes` yields: raw bytes on
//! Unix, WTF-8 on Windows. Going back to an `OsString` is the one place the
//! encoding is platform-specific, so it lives here.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use types::{Error, VPath};

/// A [`VPath`] for `path`, without loss.
#[must_use]
pub fn from_path(path: &Path) -> VPath {
    VPath::from_os_str(path.as_os_str())
}

/// The [`PathBuf`] for `path`.
///
/// # Errors
///
/// On Windows, `Error::Unsupported` when the bytes are not WTF-8, which only
/// happens for a path that came from a Unix provider; Unix paths accept any
/// bytes.
pub fn to_path(path: &VPath) -> Result<PathBuf, Error> {
    imp::to_path(path)
}

/// The `file://` URI of an absolute path, as drag-and-drop and the
/// freedesktop interfaces want it: `file:///home/me/a%20b.txt`,
/// `file:///C:/Users/me/a%20b.txt`, `file://server/share/x` for a UNC path.
/// Everything but the unreserved characters, `/` and a drive's `:` is
/// percent-encoded, byte by byte.
#[must_use]
pub fn file_uri(path: &Path) -> String {
    imp::file_uri(path)
}

/// The path a `<scheme>:///…` URI names, percent-decoded:
/// `file:///home/me/a%20b.txt` is `/home/me/a b.txt`,
/// `rick:///C:/Users/me` is `C:\Users\me`. The scheme is matched
/// regardless of case; the host may be empty or `localhost` and nothing
/// else (a UNC host is another machine's); a query or fragment is
/// dropped. `None` for anything that is not an absolute path of this
/// machine spelled as such a URI.
#[must_use]
pub fn from_uri(uri: &str, scheme: &str) -> Option<PathBuf> {
    let head = uri.get(..scheme.len())?;
    if !head.eq_ignore_ascii_case(scheme) {
        return None;
    }
    let rest = uri[scheme.len()..].strip_prefix("://")?;
    let (host, path) = rest.find('/').map_or((rest, ""), |i| rest.split_at(i));
    if !(host.is_empty() || host.eq_ignore_ascii_case("localhost")) || path.is_empty() {
        return None;
    }
    let path = path.split(['?', '#']).next()?;
    let bytes = percent_decode(path)?;
    imp::from_uri_path(bytes).ok()
}

/// Percent-decodes `text` byte by byte; `None` for a `%` that no two hex
/// digits follow.
fn percent_decode(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3)?;
            let hex = std::str::from_utf8(hex).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Some(out)
}

/// Percent-encodes everything but the unreserved characters and `/`.
pub(crate) fn percent_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/') {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

#[cfg(unix)]
mod imp {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::path::PathBuf;

    use types::{Error, VPath};

    // Same signature as the Windows version, which can fail.
    #[allow(clippy::unnecessary_wraps)]
    pub(super) fn to_path(path: &VPath) -> Result<PathBuf, Error> {
        Ok(PathBuf::from(OsString::from_vec(path.as_bytes().to_vec())))
    }

    pub(super) fn file_uri(path: &std::path::Path) -> String {
        format!(
            "file://{}",
            super::percent_encode(path.as_os_str().as_encoded_bytes())
        )
    }

    /// The decoded path of a URI: any bytes, from the root.
    pub(super) fn from_uri_path(bytes: Vec<u8>) -> Result<PathBuf, Error> {
        if bytes.first() != Some(&b'/') {
            return Err(Error::Unsupported("a URI's path is absolute"));
        }
        Ok(PathBuf::from(OsString::from_vec(bytes)))
    }
}

#[cfg(windows)]
mod imp {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use std::path::PathBuf;

    use types::{Error, VPath};

    pub(super) fn to_path(path: &VPath) -> Result<PathBuf, Error> {
        let wide = path
            .to_wide()
            .ok_or(Error::Unsupported("path bytes are not WTF-8"))?;
        Ok(PathBuf::from(OsString::from_wide(&wide)))
    }

    pub(super) fn file_uri(path: &std::path::Path) -> String {
        let text = path.to_string_lossy().replace('\\', "/");
        if let Some(unc) = text.strip_prefix("//") {
            // `\\server\share\x`: the server is the URI's host.
            let (host, rest) = unc.split_once('/').unwrap_or((unc, ""));
            return format!("file://{host}/{}", super::percent_encode(rest.as_bytes()));
        }
        // `C:/x`: the drive keeps its colon.
        let encoded = text
            .split_once(':')
            .filter(|(drive, _)| drive.len() == 1)
            .map_or_else(
                || super::percent_encode(text.as_bytes()),
                |(drive, rest)| format!("{drive}:{}", super::percent_encode(rest.as_bytes())),
            );
        format!("file:///{encoded}")
    }

    /// The decoded path of a URI: `/C:/Users/me` is `C:\Users\me`; the
    /// bytes are UTF-8, as a URI of a Windows path is.
    pub(super) fn from_uri_path(bytes: Vec<u8>) -> Result<PathBuf, Error> {
        let text = String::from_utf8(bytes)
            .map_err(|_| Error::Unsupported("a URI's path is UTF-8 on Windows"))?;
        let text = text.replace('/', "\\");
        let text = text
            .strip_prefix('\\')
            .filter(|rest| rest.as_bytes().get(1) == Some(&b':'))
            .map_or(text.as_str(), |rest| rest)
            .to_owned();
        let path = PathBuf::from(text);
        if !path.is_absolute() {
            return Err(Error::Unsupported("a URI's path is absolute"));
        }
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn file_uris_are_percent_encoded() {
        assert_eq!(super::percent_encode(b"/tmp/a b%.txt"), "/tmp/a%20b%25.txt");
        let uri = super::file_uri(std::path::Path::new(if cfg!(windows) {
            "C:\\Users\\me\\a b.txt"
        } else {
            "/home/me/a b.txt"
        }));
        assert!(uri.starts_with("file:///"), "{uri}");
        assert!(uri.ends_with("/me/a%20b.txt"), "{uri}");
        assert!(!uri.contains(' '));
    }

    use super::*;

    /// `file:///…` and `rick:///…` name a path of this machine, decoded;
    /// another host, a relative path or a broken escape name nothing.
    #[test]
    fn uris_decode_to_paths_of_this_machine() {
        let path = Path::new(if cfg!(windows) {
            "C:\\Users\\me\\a b#1.txt"
        } else {
            "/home/me/a b#1.txt"
        });
        let uri = file_uri(path);
        assert_eq!(from_uri(&uri, "file").as_deref(), Some(path));
        let rick = format!("rick{}", &uri["file".len()..]);
        assert_eq!(from_uri(&rick, "rick").as_deref(), Some(path));
        assert_eq!(
            from_uri(&rick.to_uppercase(), "rick").map(|p| p.to_string_lossy().to_lowercase()),
            Some(path.to_string_lossy().to_lowercase())
        );
        assert_eq!(from_uri(&uri, "rick"), None, "another scheme");
        assert_eq!(
            from_uri(&format!("{rick}?x=1#frag"), "rick").as_deref(),
            Some(path),
            "query and fragment dropped"
        );
        assert_eq!(
            from_uri(
                &format!("file://localhost{}", &uri["file://".len()..]),
                "file"
            )
            .as_deref(),
            Some(path),
            "localhost is this machine"
        );
        assert_eq!(
            from_uri("rick://apps", "rick"),
            None,
            "a host is not a path"
        );
        assert_eq!(from_uri("rick://server/share/x", "rick"), None);
        assert_eq!(
            from_uri("rick:///", "rick").is_some(),
            cfg!(unix),
            "the root"
        );
        assert_eq!(from_uri("rick:///a%2", "rick"), None, "a broken escape");
        assert_eq!(from_uri("rick:relative", "rick"), None);
        assert_eq!(from_uri("rick", "rick"), None);
    }

    #[test]
    fn native_paths_round_trip() {
        let path = Path::new("some/dir/file name.txt");
        let vpath = from_path(path);
        assert_eq!(vpath.to_string_lossy(), "some/dir/file name.txt");
        assert_eq!(to_path(&vpath).ok().as_deref(), Some(path));
    }

    #[test]
    fn unicode_names_round_trip() {
        let path = Path::new("ünïcödé/日本語/😀.rs");
        assert_eq!(to_path(&from_path(path)).ok().as_deref(), Some(path));
    }
}
