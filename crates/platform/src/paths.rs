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
