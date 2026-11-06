//! Conversion between [`VPath`] and [`Path`].
//!
//! `VPath` stores the bytes `OsStr::as_encoded_bytes` yields: raw bytes on
//! Unix, WTF-8 on Windows. Going back to an `OsString` is the one place the
//! encoding is platform-specific, so it lives here.

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
}

#[cfg(test)]
mod tests {
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
