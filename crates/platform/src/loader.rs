//! Where the system's loader looks for the libraries a library links: what
//! the plugin host tells it, in place of changing the `PATH`.
//!
//! Windows never looks in the folder of the library that asks. What a
//! library links is looked for beside the binary and in the system's
//! folders, and by default in the current directory and in the folders of
//! the `PATH` as well. A plugin that brings libraries in its folder, as the
//! media previewer brings the GStreamer runtime, needs that folder
//! searched: for its own load and for every later one, since GStreamer
//! opens its elements by full path when they are first needed and what an
//! element links lies in the plugin's folder too.
//!
//! The `PATH` would do it, and every program Rick starts would inherit the
//! folder with it. So the loader is told instead, in two calls that hold
//! for this process and for no other:
//!
//! - [`restrict_search`] makes the process look in the application's
//!   folder, in the folders named to it and in the system's folder, and
//!   nowhere else (`SetDefaultDllDirectories` with
//!   `LOAD_LIBRARY_SEARCH_DEFAULT_DIRS`). The current directory and the
//!   `PATH` are no longer searched for libraries. It cannot be undone.
//! - [`name_dir`] names one folder (`AddDllDirectory`), which
//!   [`NamedDir::remove`] takes out again.
//!
//! A library that is already loaded is found whatever the search path
//! says, so a plugin still meets the kernel libraries and `libstd` of the
//! binary that loads it, wherever those were found at start.
//!
//! Linux and macOS have none of this to do: a library there says itself
//! where its libraries are, with a run path. [`restrict_search`] does
//! nothing on them and [`name_dir`] is refused.
//!
//! ```no_run
//! # fn main() -> std::io::Result<()> {
//! use std::path::Path;
//!
//! use platform::loader;
//!
//! loader::restrict_search()?;
//! let folder = loader::name_dir(Path::new(r"C:\Rick\plugins\preview-media"))?;
//! // Load the plugin's library here. When it does not load after all:
//! folder.remove()?;
//! # Ok(())
//! # }
//! ```

use std::io;
use std::path::Path;

/// A folder named to the loader with [`name_dir`]. It stays named for the
/// rest of the process when this value is dropped; [`NamedDir::remove`]
/// takes it out.
#[derive(Debug)]
pub struct NamedDir(imp::Cookie);

impl NamedDir {
    /// Takes the folder out of the loader's search again.
    ///
    /// # Errors
    ///
    /// The system's error.
    pub fn remove(self) -> io::Result<()> {
        imp::remove(&self.0)
    }
}

/// Limits where this process looks for a library it loads by name, and
/// for what a library links, to the application's folder, the folders
/// named with [`name_dir`] and the system's folder: the current directory
/// and the `PATH` go. Once per process and for its whole life; a second
/// call answers as the first did. Windows only; nothing happens elsewhere.
///
/// # Errors
///
/// The system's error, when the search path stays as it was.
pub fn restrict_search() -> io::Result<()> {
    imp::restrict_search()
}

/// Names the folder `dir` to the loader: every load of this process from
/// now on looks there too, after the application's folder and before the
/// system's. The order among several named folders is the system's to
/// choose. Calls [`restrict_search`] first, without which a named folder
/// counts for nothing. Windows only.
///
/// # Errors
///
/// The system's error; `Unsupported` on Linux and macOS.
pub fn name_dir(dir: &Path) -> io::Result<NamedDir> {
    imp::name_dir(dir).map(NamedDir)
}

#[cfg(windows)]
mod imp {
    use std::io;
    use std::path::Path;
    use std::sync::OnceLock;

    use windows::Win32::System::LibraryLoader::{
        AddDllDirectory, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, RemoveDllDirectory,
        SetDefaultDllDirectories,
    };
    use windows::core::PCWSTR;

    use crate::fs::windows::{io_error, wide};

    /// What `AddDllDirectory` answers with: a pointer only the loader
    /// reads, kept as a number so that the value can cross threads.
    #[derive(Debug)]
    pub(super) struct Cookie(usize);

    pub(super) fn restrict_search() -> io::Result<()> {
        /// The Win32 error of the one call, or none.
        static DONE: OnceLock<Option<i32>> = OnceLock::new();
        let failed = DONE.get_or_init(|| {
            // SAFETY: the call takes a set of flags and no pointer. It
            // changes where this process searches for libraries from now
            // on, which is what the caller asks for.
            unsafe { SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_DEFAULT_DIRS) }
                .err()
                .map(|error| io_error(error).raw_os_error().unwrap_or(0))
        });
        match failed {
            None => Ok(()),
            Some(code) => Err(io::Error::from_raw_os_error(*code)),
        }
    }

    pub(super) fn name_dir(dir: &Path) -> io::Result<Cookie> {
        restrict_search()?;
        // The loader takes an absolute path and nothing else.
        let dir = wide(&std::path::absolute(dir)?);
        // SAFETY: `dir` is a NUL-terminated UTF-16 string that outlives
        // the call, which copies it.
        let cookie = unsafe { AddDllDirectory(PCWSTR(dir.as_ptr())) };
        if cookie.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(Cookie(cookie.expose_provenance()))
    }

    pub(super) fn remove(cookie: &Cookie) -> io::Result<()> {
        // SAFETY: the cookie is the one `AddDllDirectory` answered with,
        // and `NamedDir::remove` consumes its only holder, so it is
        // handed back once.
        unsafe { RemoveDllDirectory(std::ptr::with_exposed_provenance(cookie.0)) }.map_err(io_error)
    }
}

#[cfg(not(windows))]
mod imp {
    use std::io;
    use std::path::Path;

    /// Nothing to hold: no folder is ever named here.
    #[derive(Debug)]
    pub(super) struct Cookie;

    // The same signature as on Windows, where the call can fail.
    #[allow(clippy::unnecessary_wraps)]
    pub(super) fn restrict_search() -> io::Result<()> {
        Ok(())
    }

    pub(super) fn name_dir(_dir: &Path) -> io::Result<Cookie> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "a library names its own folders on this platform",
        ))
    }

    // The same signature as on Windows, where the call can fail.
    #[allow(clippy::unnecessary_wraps)]
    pub(super) fn remove(_cookie: &Cookie) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// On Windows the two calls succeed, a second restriction answers as
    /// the first did and a named folder can be taken out again; elsewhere
    /// nothing is restricted and no folder is named. On no platform does
    /// either call touch the environment.
    #[test]
    fn a_folder_is_named_to_the_loader_and_the_environment_stays() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let before: Vec<_> = std::env::vars_os().collect();
        restrict_search().unwrap_or_else(|e| panic!("{e}"));
        restrict_search().unwrap_or_else(|e| panic!("{e}"));
        let named = name_dir(dir.path());
        if cfg!(windows) {
            let named = named.unwrap_or_else(|e| panic!("{e}"));
            named.remove().unwrap_or_else(|e| panic!("{e}"));
            // A folder that is given relative to the current directory is
            // named by its absolute path, which is all the loader takes.
            name_dir(Path::new("."))
                .and_then(NamedDir::remove)
                .unwrap_or_else(|e| panic!("{e}"));
        } else {
            let error = named.err().unwrap_or_else(|| panic!("named a folder"));
            assert_eq!(error.kind(), io::ErrorKind::Unsupported);
        }
        let after: Vec<_> = std::env::vars_os().collect();
        assert_eq!(before, after);
    }
}
