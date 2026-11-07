//! macOS: `NSFileManager` trash, `open`, Finder's Get Info window,
//! `~/Library/Application Support/Rick`, the mounted volumes from
//! `NSFileManager.mountedVolumeURLs` ejected through `diskutil`, the
//! standard directories under the home, and Spotlight's last-used dates
//! through `mdfind` for the recently-used list.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::SystemTime;

use objc2_foundation::{NSFileManager, NSString, NSURL, NSVolumeEnumerationOptions};
use types::Error;

use crate::Native;
use crate::time::parse_iso8601;
use crate::traits::{
    Chrome, ConfigDir, ControlsSide, DragSource, OpenWith, Properties, PropertiesOutcome,
    RecentItem, RecentlyUsed, StandardFolder, StandardFolders, Trash, TrashedItem, Volume,
    VolumeKind, Volumes, WindowChrome,
};

/// How many recently-used items are kept, newest first.
const RECENT_LIMIT: usize = 500;
/// How far back Spotlight is asked for used files, in seconds (30 days).
const RECENT_WINDOW_SECS: u64 = 30 * 24 * 3600;

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

    fn remove(&self, item: &TrashedItem) -> Result<(), Error> {
        let meta = fs::symlink_metadata(&item.trashed).map_err(|_| Error::NotFound)?;
        if meta.is_dir() {
            fs::remove_dir_all(&item.trashed)
        } else {
            fs::remove_file(&item.trashed)
        }
        .map_err(Error::from_io)
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
        Ok(())
    }
}

impl Volumes for Native {
    /// `mountedVolumeURLs` without hidden volumes, with `statvfs` for the
    /// space and `statfs` for the file system; the system volume (`/`)
    /// first as `System`. Everything under `/Volumes` that is not a network
    /// or optical mount counts as removable.
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
            let is_root = mount_point == Path::new("/");
            let name = if is_root {
                "System".to_owned()
            } else {
                mount_point.file_name().map_or_else(
                    || mount_point.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                )
            };
            let fs_type = fs_type_of(&mount_point);
            let kind = if is_root {
                VolumeKind::Fixed
            } else {
                volume_kind(&mount_point, &fs_type)
            };
            let (total, free) = crate::volumes::space(&mount_point)
                .map_or((None, None), |(t, f)| (Some(t), Some(f)));
            out.push(Volume {
                name,
                mount_point,
                kind,
                fs_type,
                device: None,
                total,
                free,
            });
        }
        out.sort_by(|a, b| a.mount_point.cmp(&b.mount_point));
        Ok(out)
    }

    /// `diskutil eject` on the mount point: unmounts the volume and ejects
    /// the disk or opens the tray.
    fn eject(&self, volume: &Volume) -> Result<(), Error> {
        if !volume.is_removable() {
            return Err(Error::Unsupported("only removable media can be ejected"));
        }
        let output = Command::new("diskutil")
            .arg("eject")
            .arg(volume.mount_point.as_os_str())
            .stdin(Stdio::null())
            .output()
            .map_err(Error::from_io)?;
        if output.status.success() {
            return Ok(());
        }
        let said = String::from_utf8_lossy(&output.stderr);
        let said = said.trim();
        Err(Error::Io(io::Error::other(if said.is_empty() {
            format!("diskutil exited with {}", output.status)
        } else {
            said.to_owned()
        })))
    }
}

/// Network file systems by the name `statfs` reports.
const NETWORK_FS: &[&str] = &[
    "nfs", "smbfs", "afpfs", "webdav", "cifs", "fuse", "osxfuse", "macfuse",
];

/// The file system type name `statfs` reports for a mount point.
fn fs_type_of(mount_point: &Path) -> String {
    rustix::fs::statfs(mount_point)
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
        .unwrap_or_default()
}

/// The kind of a mounted volume: network by file system type, optical by
/// type, removable for anything else under `/Volumes`.
fn volume_kind(mount_point: &Path, fs_type: &str) -> VolumeKind {
    if NETWORK_FS.iter().any(|n| fs_type.starts_with(n)) {
        VolumeKind::Network
    } else if matches!(fs_type, "cd9660" | "udf") {
        VolumeKind::Optical
    } else if mount_point.starts_with("/Volumes") {
        VolumeKind::Removable
    } else {
        VolumeKind::Fixed
    }
}

impl StandardFolders for Native {
    /// The fixed directories under the home: `Documents`, `Pictures`,
    /// `Music`, `Movies`, `Downloads`, `Desktop`.
    fn standard_folder(&self, folder: StandardFolder) -> Option<PathBuf> {
        let home = self.home_dir();
        let path = match folder {
            StandardFolder::Home => home,
            StandardFolder::Documents => home.join("Documents"),
            StandardFolder::Pictures => home.join("Pictures"),
            StandardFolder::Music => home.join("Music"),
            StandardFolder::Video => home.join("Movies"),
            StandardFolder::Downloads => home.join("Downloads"),
            StandardFolder::Desktop => home.join("Desktop"),
        };
        path.is_dir().then_some(path)
    }
}

/// One line of `mdfind -attr kMDItemLastUsedDate`: the path, whitespace,
/// then `kMDItemLastUsedDate = 2026-11-07 10:00:00 +0000`.
fn parse_mdfind_line(line: &str) -> Option<RecentItem> {
    let (path, rest) = line.split_once("kMDItemLastUsedDate")?;
    let value = rest.trim_start().strip_prefix('=')?.trim();
    let (date, rest) = value.split_once(' ')?;
    let clock = rest.split_whitespace().next()?;
    let used_at = parse_iso8601(&format!("{date}T{clock}"))?;
    let path = path.trim_end();
    if path.is_empty() {
        return None;
    }
    Some(RecentItem {
        path: PathBuf::from(path),
        used_at,
    })
}

impl RecentlyUsed for Native {
    /// Spotlight's `kMDItemLastUsedDate` over the home for the last thirty
    /// days, newest first. macOS keeps its Recent Items in a private store
    /// without a public reader; the Spotlight attribute is what the Finder
    /// itself searches on.
    fn recently_used(&self) -> Result<Vec<RecentItem>, Error> {
        let output = Command::new("mdfind")
            .arg("-onlyin")
            .arg(self.home_dir())
            .arg("-attr")
            .arg("kMDItemLastUsedDate")
            .arg(format!(
                "kMDItemLastUsedDate >= $time.now(-{RECENT_WINDOW_SECS})"
            ))
            .stdin(Stdio::null())
            .output()
            .map_err(Error::from_io)?;
        if !output.status.success() {
            let said = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Io(io::Error::other(said.trim().to_owned())));
        }
        let text = String::from_utf8_lossy(&output.stdout);
        let mut items: Vec<RecentItem> = text
            .lines()
            .filter_map(parse_mdfind_line)
            .filter(|item| fs::symlink_metadata(&item.path).is_ok())
            .collect();
        items.sort_by_key(|item| std::cmp::Reverse(item.used_at));
        items.truncate(RECENT_LIMIT);
        Ok(items)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mdfind_lines_give_a_path_and_a_time() {
        let item = parse_mdfind_line(
            "/Users/me/Documents/a b.pdf   kMDItemLastUsedDate = 2026-11-07 10:00:00 +0000",
        )
        .unwrap_or_else(|| panic!("parsed"));
        assert_eq!(item.path, PathBuf::from("/Users/me/Documents/a b.pdf"));
        assert_eq!(
            item.used_at,
            parse_iso8601("2026-11-07T10:00:00").unwrap_or(SystemTime::UNIX_EPOCH)
        );
        assert!(parse_mdfind_line("/Users/me/x.txt").is_none());
        assert!(parse_mdfind_line("kMDItemLastUsedDate = (null)").is_none());
    }
}
