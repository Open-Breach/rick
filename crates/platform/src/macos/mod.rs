//! macOS: `NSFileManager` trash, `open`, Finder's Get Info window,
//! `~/Library/Application Support/Rick`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::SystemTime;

use objc2_foundation::{NSFileManager, NSString, NSURL};
use types::Error;

use crate::Native;
use crate::traits::{ConfigDir, OpenWith, Properties, PropertiesOutcome, Trash, TrashedItem};

fn utf8(path: &Path) -> Result<&str, Error> {
    path.to_str()
        .ok_or(Error::Unsupported("path is not valid UTF-8"))
}

impl Trash for Native {
    fn trash(&self, path: &Path) -> Result<TrashedItem, Error> {
        let original = std::path::absolute(path).map_err(Error::from_io)?;
        fs::symlink_metadata(&original).map_err(Error::from_io)?;
        let url = NSURL::fileURLWithPath(&NSString::from_str(utf8(&original)?));
        let manager = NSFileManager::defaultManager();
        let mut resulting = None;
        manager
            .trashItemAtURL_resultingItemURL_error(&url, Some(&mut resulting))
            .map_err(|error| {
                Error::Io(io::Error::other(error.localizedDescription().to_string()))
            })?;
        let trashed = resulting
            .and_then(|url| url.path())
            .map(|s| PathBuf::from(s.to_string()))
            .ok_or(Error::Unsupported(
                "the trash did not report the new location",
            ))?;
        Ok(TrashedItem {
            original,
            trashed,
            deleted_at: SystemTime::now(),
        })
    }

    fn restore(&self, item: &TrashedItem) -> Result<(), Error> {
        if fs::symlink_metadata(&item.trashed).is_err() {
            return Err(Error::NotFound);
        }
        if fs::symlink_metadata(&item.original).is_ok() {
            return Err(Error::Io(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "original path is taken",
            )));
        }
        if let Some(parent) = item.original.parent() {
            fs::create_dir_all(parent).map_err(Error::from_io)?;
        }
        fs::rename(&item.trashed, &item.original).map_err(Error::from_io)
    }

    /// macOS keeps the original locations of trashed items in Finder's own
    /// `.DS_Store`, which has no public reader, so the trash cannot be
    /// listed with restore targets.
    fn list(&self) -> Result<Vec<TrashedItem>, Error> {
        Err(Error::Unsupported(
            "macOS does not expose the original paths of trashed items",
        ))
    }
}

fn run_detached(program: &str, args: &[&std::ffi::OsStr]) -> Result<(), Error> {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(drop)
        .map_err(Error::from_io)
}

impl OpenWith for Native {
    fn open(&self, path: &Path) -> Result<(), Error> {
        let path = std::path::absolute(path).map_err(Error::from_io)?;
        run_detached("open", &[path.as_os_str()])
    }

    fn reveal(&self, path: &Path) -> Result<(), Error> {
        let path = std::path::absolute(path).map_err(Error::from_io)?;
        run_detached("open", &[std::ffi::OsStr::new("-R"), path.as_os_str()])
    }
}

impl Properties for Native {
    fn show(&self, path: &Path) -> Result<PropertiesOutcome, Error> {
        let path = std::path::absolute(path).map_err(Error::from_io)?;
        fs::symlink_metadata(&path).map_err(Error::from_io)?;
        let escaped = utf8(&path)?.replace('\\', "\\\\").replace('"', "\\\"");
        let script = format!(
            "tell application \"Finder\"\nactivate\nopen information window of (POSIX file \"{escaped}\" as alias)\nend tell"
        );
        let status = Command::new("osascript")
            .args(["-e", &script])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(Error::from_io)?;
        if status.success() {
            Ok(PropertiesOutcome::Native)
        } else {
            Err(Error::Io(io::Error::other(
                "Finder refused to show the Get Info window",
            )))
        }
    }
}

impl ConfigDir for Native {
    fn config_dir(&self) -> PathBuf {
        std::env::var_os("HOME")
            .map_or_else(|| PathBuf::from("."), PathBuf::from)
            .join("Library/Application Support/Rick")
    }

    fn home_dir(&self) -> PathBuf {
        std::env::var_os("HOME")
            .filter(|v| !v.is_empty())
            .map_or_else(|| PathBuf::from("."), PathBuf::from)
    }
}
