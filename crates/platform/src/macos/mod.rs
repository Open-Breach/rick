//! macOS: `NSFileManager` trash with Rick's own record of what it trashed
//! (the system keeps the original paths in Finder's `.DS_Store`, which has
//! no public reader), `open`, Finder's Get Info window,
//! `~/Library/Application Support/Rick`, and the mounted volumes from
//! `NSFileManager.mountedVolumeURLs`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::SystemTime;

use objc2_foundation::{NSFileManager, NSString, NSURL, NSVolumeEnumerationOptions};
use types::Error;

use crate::Native;
use crate::traits::{
    Chrome, ConfigDir, ControlsSide, DragSource, OpenWith, Properties, PropertiesOutcome, Trash,
    TrashedItem, Volume, VolumeKind, Volumes, WindowChrome,
};
use crate::trash_record::{self, TRASH_FILE};

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
        let item = TrashedItem {
            original,
            trashed,
            deleted_at: SystemTime::now(),
        };
        // The record is what `list` answers; a record that cannot be
        // written loses the item from the Trash view, not from the trash.
        if let Err(error) = trash_record::record(&self.record_file(), &item) {
            tracing::warn!(%error, "trash record not written");
        }
        Ok(item)
    }

    fn restore(&self, item: &TrashedItem) -> Result<(), Error> {
        if fs::symlink_metadata(&item.trashed).is_err() {
            trash_record::forget(&self.record_file(), &item.trashed)?;
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
        fs::rename(&item.trashed, &item.original).map_err(Error::from_io)?;
        trash_record::forget(&self.record_file(), &item.trashed)
    }

    /// macOS keeps the original locations of trashed items in Finder's own
    /// `.DS_Store`, which has no public reader, so the trash lists Rick's
    /// own record of what it trashed, pruned of what has left the trash
    /// since (QUESTIONS.md 74 and 86).
    fn list(&self) -> Result<Vec<TrashedItem>, Error> {
        trash_record::load(&self.record_file())
    }

    fn remove(&self, item: &TrashedItem) -> Result<(), Error> {
        let meta = fs::symlink_metadata(&item.trashed).map_err(|_| Error::NotFound)?;
        if meta.is_dir() {
            fs::remove_dir_all(&item.trashed)
        } else {
            fs::remove_file(&item.trashed)
        }
        .map_err(Error::from_io)?;
        trash_record::forget(&self.record_file(), &item.trashed)
    }

    /// Empties the user's own trash, `~/.Trash`, which holds what Rick and
    /// the Finder trashed from the system volume; volume trashes
    /// (`/Volumes/<name>/.Trashes/<uid>`) belong to the trash location issue.
    fn empty(&self) -> Result<(), Error> {
        let trash = self.home_dir().join(".Trash");
        let entries = match fs::read_dir(&trash) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(Error::from_io(e)),
        };
        for entry in entries {
            let entry = entry.map_err(Error::from_io)?;
            let path = entry.path();
            if path.file_name().is_some_and(|n| n == ".DS_Store") {
                continue;
            }
            let meta = fs::symlink_metadata(&path).map_err(Error::from_io)?;
            if meta.is_dir() {
                fs::remove_dir_all(&path)
            } else {
                fs::remove_file(&path)
            }
            .map_err(Error::from_io)?;
        }
        trash_record::save(&self.record_file(), &[])
    }
}

impl Native {
    /// Where the trash record lives: `trash.toml` in the config directory.
    fn record_file(&self) -> PathBuf {
        self.config_dir().join(TRASH_FILE)
    }
}

impl Volumes for Native {
    /// `mountedVolumeURLs` without hidden volumes, less the system volume
    /// (`/`), with `statvfs` for the space. Everything under `/Volumes` that
    /// is not an internal disk counts as removable; network mounts are told
    /// by their file system type from `statfs`.
    fn volumes(&self) -> Result<Vec<Volume>, Error> {
        let manager = NSFileManager::defaultManager();
        let urls = manager
            .mountedVolumeURLsIncludingResourceValuesForKeys_options(
                None,
                NSVolumeEnumerationOptions::SkipHiddenVolumes,
            )
            .ok_or(Error::Unsupported("the volume list is unavailable"))?;
        let mut out = Vec::new();
        for url in urls.to_vec() {
            let Some(path) = url.path() else { continue };
            let mount_point = PathBuf::from(path.to_string());
            if mount_point == Path::new("/") {
                continue;
            }
            let name = mount_point.file_name().map_or_else(
                || mount_point.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            );
            let kind = volume_kind(&mount_point);
            let (total, free) = crate::volumes::space(&mount_point)
                .map_or((None, None), |(t, f)| (Some(t), Some(f)));
            out.push(Volume {
                name,
                mount_point,
                kind,
                total,
                free,
            });
        }
        out.sort_by(|a, b| a.mount_point.cmp(&b.mount_point));
        Ok(out)
    }
}

/// Network file systems by the name `statfs` reports.
const NETWORK_FS: &[&str] = &[
    "nfs", "smbfs", "afpfs", "webdav", "cifs", "fuse", "osxfuse", "macfuse",
];

/// The kind of a mounted volume: network by file system type, optical by
/// type, removable for anything else under `/Volumes`.
fn volume_kind(mount_point: &Path) -> VolumeKind {
    let fs_type = rustix::fs::statfs(mount_point)
        .ok()
        .map(|stat| {
            let bytes: Vec<u8> = stat
                .f_fstypename
                .iter()
                .take_while(|&&c| c != 0)
                .map(|c| c.to_ne_bytes()[0])
                .collect();
            String::from_utf8_lossy(&bytes).into_owned()
        })
        .unwrap_or_default();
    if NETWORK_FS.iter().any(|n| fs_type.starts_with(n)) {
        VolumeKind::Network
    } else if matches!(fs_type.as_str(), "cd9660" | "udf") {
        VolumeKind::Optical
    } else if mount_point.starts_with("/Volumes") {
        VolumeKind::Removable
    } else {
        VolumeKind::Fixed
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

impl Chrome for Native {
    fn window_chrome(&self) -> WindowChrome {
        // The traffic lights stay where macOS puts them.
        WindowChrome {
            controls: ControlsSide::Left,
            native_controls: true,
            native_controls_width: 78.0,
        }
    }
}

impl DragSource for Native {
    /// Not available yet: an `AppKit` drag starts from the `NSView` with the
    /// `NSEvent` of the press (`beginDraggingSessionWithItems:event:source:`),
    /// and `winit` 0.30 behind `eframe` 0.36 hands neither out; it exposes
    /// drop reception only (QUESTIONS.md).
    fn start_drag(&self, paths: &[PathBuf]) -> Result<(), Error> {
        let uris: Vec<String> = paths.iter().map(|p| crate::paths::file_uri(p)).collect();
        tracing::debug!(?uris, "platform drag not started");
        Err(Error::Unsupported(
            "starting a drag needs the window's NSView and the press event, which winit does not expose",
        ))
    }
}
