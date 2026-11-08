//! macOS: `NSFileManager` trash with Rick's own record of what it trashed
//! (the system keeps the original paths in Finder's `.DS_Store`, which has
//! no public reader), `open` and Launch Services' application list for
//! Open with, Terminal.app, Finder's Get Info window,
//! `~/Library/Application Support/Rick`, the mounted volumes from
//! `NSFileManager.mountedVolumeURLs` ejected through `diskutil`, the
//! standard directories under the home, and Spotlight's last-used dates
//! through `mdfind` for the recently-used list.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::SystemTime;

use std::ffi::c_void;

use objc2::rc::Retained;
use objc2_foundation::{NSArray, NSFileManager, NSString, NSURL, NSVolumeEnumerationOptions};
use types::Error;

use crate::Native;
use crate::time::parse_iso8601;
use crate::traits::{
    Application, Chrome, ConfigDir, ControlsSide, DragSource, Elevate, ElevatedOutput, OpenWith,
    Properties, PropertiesOutcome, RecentItem, RecentlyUsed, StandardFolder, StandardFolders,
    Terminal, Trash, TrashedItem, Volume, VolumeKind, Volumes, WindowChrome,
};
use crate::trash_record::{self, TRASH_FILE};

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
    /// since (QUESTIONS.md 74 and 94).
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
    /// `Native` is a zero-sized `Copy` type, so it goes by value.
    fn record_file(self) -> PathBuf {
        self.config_dir().join(TRASH_FILE)
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

#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    /// Launch Services: every application that can open the URL, the
    /// default first. The URL is a `CFURLRef`, the result a `CFArrayRef`
    /// of `CFURLRef`s the caller owns, or null.
    fn LSCopyApplicationURLsForURL(in_url: *const c_void, in_role_mask: u32) -> *mut c_void;
}

/// `kLSRolesAll`.
const LS_ROLES_ALL: u32 = 0xFFFF_FFFF;

/// The display name of an application bundle: its file name without
/// `.app`.
fn app_name(bundle: &Path) -> String {
    bundle.file_stem().map_or_else(
        || bundle.display().to_string(),
        |s| s.to_string_lossy().into_owned(),
    )
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

    fn applications(&self, path: &Path) -> Result<Vec<Application>, Error> {
        let path = std::path::absolute(path).map_err(Error::from_io)?;
        fs::symlink_metadata(&path).map_err(Error::from_io)?;
        let url = NSURL::fileURLWithPath(&NSString::from_str(utf8(&path)?));
        // SAFETY: `NSURL` is toll-free bridged to `CFURLRef`, so the
        // object's pointer is a valid `CFURLRef` for the call, which does
        // not keep it; the result follows the Copy rule (one retain the
        // caller owns) and is a `CFArrayRef` of `CFURLRef`s, bridged to
        // `NSArray<NSURL>`, which `Retained::from_raw` takes ownership of.
        let array: Option<Retained<NSArray<NSURL>>> = unsafe {
            let raw = LSCopyApplicationURLsForURL(Retained::as_ptr(&url).cast(), LS_ROLES_ALL);
            if raw.is_null() {
                None
            } else {
                Retained::from_raw(raw.cast())
            }
        };
        let Some(array) = array else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for i in 0..array.count() {
            let app = array.objectAtIndex(i);
            let Some(bundle) = app.path().map(|s| PathBuf::from(s.to_string())) else {
                continue;
            };
            let id = bundle.to_string_lossy().into_owned();
            if out.iter().any(|a: &Application| a.id == id) {
                continue;
            }
            out.push(Application {
                name: app_name(&bundle),
                id,
            });
        }
        Ok(out)
    }

    fn open_with(&self, path: &Path, app: &Application) -> Result<(), Error> {
        let path = std::path::absolute(path).map_err(Error::from_io)?;
        let is_bundle = Path::new(app.id.trim_end_matches('/'))
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("app"));
        if is_bundle {
            return run_detached(
                "open",
                &[
                    std::ffi::OsStr::new("-a"),
                    std::ffi::OsStr::new(&app.id),
                    path.as_os_str(),
                ],
            );
        }
        let mut words = app.id.split_whitespace();
        let program = words
            .next()
            .ok_or(Error::Unsupported("no application named"))?;
        let mut args: Vec<&std::ffi::OsStr> = words.map(std::ffi::OsStr::new).collect();
        args.push(path.as_os_str());
        run_detached(program, &args)
    }
}

impl Elevate for Native {
    /// Through Authorization Services, the way `osascript`'s `with
    /// administrator privileges` reaches it: the system's own prompt, and
    /// the command run as root once the user has authorised it.
    fn run_elevated(
        &self,
        program: &Path,
        args: &[std::ffi::OsString],
    ) -> Result<ElevatedOutput, Error> {
        let program = std::path::absolute(program).map_err(Error::from_io)?;
        let mut words = vec![shell_quote(utf8(&program)?)];
        for arg in args {
            words.push(shell_quote(arg.to_str().ok_or(Error::Unsupported(
                "an argument that is not valid UTF-8",
            ))?));
        }
        let command = words.join(" ");
        // The command runs inside an AppleScript string: backslashes and
        // quotes are escaped for that string, the shell quoting above for
        // the shell it then runs in.
        let escaped = command.replace('\\', "\\\\").replace('"', "\\\"");
        let script = format!("do shell script \"{escaped}\" with administrator privileges");
        let output = Command::new("osascript")
            .args(["-e", &script])
            .stdin(Stdio::null())
            .output()
            .map_err(Error::from_io)?;
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !output.status.success() {
            // -128 is the user cancelling the prompt; -60007 and -60005 a
            // user who may not administer the machine.
            if stderr.contains("(-128)")
                || stderr.contains("(-60007)")
                || stderr.contains("(-60005)")
            {
                return Err(Error::PermissionDenied);
            }
            return Ok(ElevatedOutput {
                code: output.status.code(),
                stdout: output.stdout,
                stderr: output.stderr,
            });
        }
        Ok(ElevatedOutput {
            code: Some(0),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}

/// `text` as one word of a POSIX shell command.
fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

impl Terminal for Native {
    fn open_terminal(&self, dir: &Path, command: Option<&[String]>) -> Result<(), Error> {
        let dir = std::path::absolute(dir).map_err(Error::from_io)?;
        let Some(command) = command.filter(|c| !c.is_empty()) else {
            return run_detached(
                "open",
                &[
                    std::ffi::OsStr::new("-a"),
                    std::ffi::OsStr::new("Terminal"),
                    dir.as_os_str(),
                ],
            );
        };
        let quote = |text: &str| format!("'{}'", text.replace('\'', "'\\''"));
        let line = std::iter::once(format!("cd {}", quote(utf8(&dir)?)))
            .chain(std::iter::once(
                command
                    .iter()
                    .map(|w| quote(w))
                    .collect::<Vec<_>>()
                    .join(" "),
            ))
            .collect::<Vec<_>>()
            .join(" && ");
        let escaped = line.replace('\\', "\\\\").replace('"', "\\\"");
        let script =
            format!("tell application \"Terminal\"\nactivate\ndo script \"{escaped}\"\nend tell");
        run_detached(
            "osascript",
            &[std::ffi::OsStr::new("-e"), std::ffi::OsStr::new(&script)],
        )
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
