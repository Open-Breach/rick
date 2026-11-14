//! macOS: `NSFileManager` trash with Rick's own record of what it trashed
//! (the system keeps the original paths in Finder's `.DS_Store`, which has
//! no public reader), `open` and Launch Services' application list for
//! Open with, Terminal.app, Finder's Get Info window,
//! `~/Library/Application Support/Rick`, the mounted volumes from
//! `NSFileManager.mountedVolumeURLs` ejected through `diskutil`, the
//! standard directories under the home, Spotlight's last-used dates
//! through `mdfind` for the recently-used list, and drag and drop through
//! an `NSDraggingSession` and the mouse location.

mod drag;

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
    Activation, Application, Chrome, ConfigDir, ControlsSide, DragSource, DropTarget, Elevate,
    ElevatedOutput, ImageFormat, IncomingFiles, Monitor, Mount, OpenWith, Point, Properties,
    PropertiesOutcome, RecentItem, RecentlyUsed, StandardFolder, StandardFolders, Terminal, Trash,
    TrashedItem, Volume, VolumeKind, Volumes, WindowChrome, WindowRef,
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

/// `macOS` with the product version `sw_vers` reports, `macOS` alone when
/// it does not answer.
pub(crate) fn os_description() -> String {
    let version = Command::new("sw_vers")
        .arg("-productVersion")
        .stdin(Stdio::null())
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .filter(|v| !v.is_empty());
    match version {
        Some(version) => format!("macOS {version}"),
        None => "macOS".to_owned(),
    }
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
            let read_only = read_only_of(&mount_point);
            out.push(Volume {
                name,
                mount_point,
                kind,
                fs_type,
                device: None,
                total,
                free,
                read_only,
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

    /// Disk Utility is part of the system.
    fn has_format_tool(&self) -> bool {
        Path::new(DISK_UTILITY).exists()
    }

    /// Opens Disk Utility, where the volume is erased; it takes no volume
    /// to start on.
    fn open_format_tool(&self, _volume: &Volume) -> Result<(), Error> {
        if !self.has_format_tool() {
            return Err(Error::Unsupported("no disk utility is installed"));
        }
        run_detached(
            "open",
            &[
                std::ffi::OsStr::new("-a"),
                std::ffi::OsStr::new(DISK_UTILITY),
            ],
        )
    }
}

/// Where Disk Utility is on every macOS Rick supports.
const DISK_UTILITY: &str = "/System/Applications/Utilities/Disk Utility.app";

/// Network file systems by the name `statfs` reports.
const NETWORK_FS: &[&str] = &[
    "nfs", "smbfs", "afpfs", "webdav", "cifs", "fuse", "osxfuse", "macfuse",
];

/// The file system type name `statfs` reports for a mount point.
fn fs_type_of(mount_point: &Path) -> String {
    rustix::fs::statfs(mount_point).ok().map_or_default(|stat| {
        let bytes: Vec<u8> = stat
            .f_fstypename
            .iter()
            .take_while(|&&c| c != 0)
            .map(|c| c.to_ne_bytes()[0])
            .collect();
        String::from_utf8_lossy(&bytes).into_owned()
    })
}

/// `MNT_RDONLY` of `sys/mount.h`: the volume is mounted read-only.
const MNT_RDONLY: u32 = 0x0000_0001;

/// Whether the file system at `mount_point` is mounted read-only, from
/// `statfs`'s flags.
fn read_only_of(mount_point: &Path) -> bool {
    rustix::fs::statfs(mount_point).is_ok_and(|stat| stat.f_flags & MNT_RDONLY != 0)
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

impl Mount for Native {
    /// DMG, ISO and raw images, through `hdiutil attach`.
    fn mountable(&self) -> &'static [ImageFormat] {
        crate::NATIVE_MOUNTABLE
    }

    /// `hdiutil attach -plist`: the image's file systems are mounted under
    /// `/Volumes`, and the first mount point the property list names is
    /// the volume returned; `diskutil eject` on it detaches the image.
    fn mount_image(&self, path: &Path) -> Result<Volume, Error> {
        let path = std::path::absolute(path).map_err(Error::from_io)?;
        let name = path
            .file_name()
            .map_or_default(|n| n.to_string_lossy().into_owned());
        let format = ImageFormat::for_name(&name)
            .filter(|f| crate::NATIVE_MOUNTABLE.contains(f))
            .ok_or(Error::Unsupported(
                "macOS mounts DMG, ISO and raw disk images only",
            ))?;
        let output = Command::new("hdiutil")
            .args(["attach", "-plist"])
            .arg(&path)
            .stdin(Stdio::null())
            .output()
            .map_err(Error::from_io)?;
        if !output.status.success() {
            let said = String::from_utf8_lossy(&output.stderr);
            let said = said.trim();
            return Err(Error::Io(io::Error::other(if said.is_empty() {
                format!("hdiutil exited with {}", output.status)
            } else {
                said.to_owned()
            })));
        }
        let text = String::from_utf8_lossy(&output.stdout);
        let mount_point = plist_mount_points(&text)
            .into_iter()
            .next()
            .ok_or_else(|| {
                Error::Io(io::Error::other(
                    "hdiutil attach mounted no file system of the image",
                ))
            })?;
        if let Ok(volumes) = self.volumes()
            && let Some(volume) = volumes.into_iter().find(|v| v.mount_point == mount_point)
        {
            return Ok(volume);
        }
        let fs_type = fs_type_of(&mount_point);
        let (total, free) =
            crate::volumes::space(&mount_point).map_or((None, None), |(t, f)| (Some(t), Some(f)));
        Ok(Volume {
            name: mount_point
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or(name),
            mount_point,
            kind: if format == ImageFormat::Iso {
                VolumeKind::Optical
            } else {
                VolumeKind::Removable
            },
            fs_type,
            device: None,
            total,
            free,
            read_only: format == ImageFormat::Iso,
        })
    }
}

/// The `mount-point` strings of `hdiutil attach -plist`'s output, in
/// order: one per file system the image carries.
fn plist_mount_points(text: &str) -> Vec<PathBuf> {
    const KEY: &str = "<key>mount-point</key>";
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find(KEY) {
        rest = &rest[at + KEY.len()..];
        let Some(start) = rest.find("<string>") else {
            break;
        };
        let after = &rest[start + "<string>".len()..];
        let Some(end) = after.find("</string>") else {
            break;
        };
        out.push(PathBuf::from(xml_unescape(&after[..end])));
        rest = &after[end..];
    }
    out
}

/// The five XML entities a property list escapes, undone.
fn xml_unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
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

    fn has_execute_bit(&self) -> bool {
        true
    }

    /// `open -t`: the default text editor, whatever the files' types.
    fn open_in_text_editor(&self, paths: &[PathBuf]) -> Result<(), Error> {
        let mut args: Vec<&std::ffi::OsStr> = vec![std::ffi::OsStr::new("-t")];
        args.extend(paths.iter().map(|path| path.as_os_str()));
        run_detached("open", &args)
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

    /// The screens of `AppKit`, which counts in points already, the first
    /// being the one with the menu bar. Its origin is the bottom left
    /// corner of that screen with y upwards; a window is placed from the
    /// top left with y downwards, so the rectangles are turned over.
    fn monitors(&self, _pixels_per_point: f32) -> Vec<Monitor> {
        let Some(mtm) = objc2::MainThreadMarker::new() else {
            return Vec::new();
        };
        let screens = objc2_app_kit::NSScreen::screens(mtm);
        let Some(top) = screens
            .iter()
            .next()
            .map(|main| main.frame().origin.y + main.frame().size.height)
        else {
            return Vec::new();
        };
        #[allow(clippy::cast_possible_truncation)]
        let turned = |rect: objc2_foundation::NSRect| {
            [
                rect.origin.x as f32,
                (top - (rect.origin.y + rect.size.height)) as f32,
                rect.size.width as f32,
                rect.size.height as f32,
            ]
        };
        screens
            .iter()
            .enumerate()
            .map(|(index, screen)| Monitor {
                id: screen.localizedName().to_string(),
                primary: index == 0,
                bounds: turned(screen.frame()),
                work: turned(screen.visibleFrame()),
            })
            .collect()
    }
}

impl Activation for Native {
    /// The window made key and ordered front, then the application
    /// activated over the others; on the main thread, where `AppKit` lives.
    fn activate_window(&self, window: &WindowRef, _token: Option<&str>) -> Result<(), Error> {
        let mtm = objc2::MainThreadMarker::new()
            .ok_or(Error::Unsupported("windows are raised on the main thread"))?;
        let view = drag::view_of(window, mtm)?;
        let ns_window = view
            .window()
            .ok_or(Error::Unsupported("the view is not in a window"))?;
        ns_window.makeKeyAndOrderFront(None);
        let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
        // `activate()` exists from macOS 14 only; the older call still
        // works everywhere Rick runs.
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
        Ok(())
    }

    fn allow_activation(&self) {}
}

impl DragSource for Native {
    /// Nothing to prepare: `AppKit` has the view and the event when the drag
    /// starts.
    fn attach(&self, _window: &WindowRef, _wake: std::sync::Arc<dyn Fn() + Send + Sync>) {}

    /// An `NSDraggingSession` from the window's view, the files as `NSURL`
    /// items with the first one's icon under the cursor.
    fn start_drag(&self, window: &WindowRef, paths: &[PathBuf]) -> Result<(), Error> {
        drag::start_drag(window, paths)
    }

    fn drag_in_progress(&self) -> bool {
        drag::drag_in_progress()
    }
}

impl DropTarget for Native {
    /// winit's view receives the drops and hands them to egui.
    fn incoming_files(&self) -> IncomingFiles {
        IncomingFiles::default()
    }

    /// The mouse location converted into the window's view.
    fn drop_position(&self, window: &WindowRef) -> Option<Point> {
        drag::drop_position(window)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hdiutil_property_lists_name_their_mount_points() {
        let text = "<plist><dict><key>system-entities</key><array><dict><key>dev-entry</key><string>/dev/disk4</string></dict><dict><key>content-hint</key><string>Apple_HFS</string><key>dev-entry</key><string>/dev/disk4s2</string><key>mount-point</key><string>/Volumes/Tom &amp; Jerry</string></dict></array></dict></plist>";
        assert_eq!(
            plist_mount_points(text),
            [PathBuf::from("/Volumes/Tom & Jerry")]
        );
        assert_eq!(plist_mount_points("<plist/>"), Vec::<PathBuf>::new());
        assert!(matches!(
            Native.mount_image(Path::new("/tmp/disk.vhdx")),
            Err(Error::Unsupported(_))
        ));
    }

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
