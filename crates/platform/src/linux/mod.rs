//! Linux: the XDG trash specification, `xdg-open` and the desktop files of
//! the MIME databases for Open with, `$TERMINAL` or a known emulator for
//! the terminal, `pkexec` for a command run as root, Rick's own properties
//! rows, `$XDG_CONFIG_HOME/rick`, the mount table from `/proc/mounts` with
//! `udisksctl` for ejecting and for mounting a disk image through a loop
//! device, XDG user-dirs and `recently-used.xbel`, drag and drop
//! through a Wayland data device or XDND, the window raised through
//! `xdg_activation_v1` or `_NET_ACTIVE_WINDOW`, and the integration items
//! of specs/system-integration.md §7.1 ([`integration`]).

mod activate;
mod apps;
mod drag;
mod elevate;
pub(crate) mod file_manager;
pub(crate) mod integration;
mod mount;
mod terminal;
mod trash;
mod user;
mod volumes;
mod wayland;
mod x11;

use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use types::Error;

use crate::Native;
use crate::time::iso8601;
use crate::traits::{
    Activation, Application, Chrome, ConfigDir, ControlsSide, DragSource, DropTarget, Elevate,
    ElevatedOutput, ImageFormat, IncomingFiles, Mount, MountWatch, MountsChanged, OpenWith, Point,
    Properties, PropertiesOutcome, RecentItem, RecentlyUsed, StandardFolder, StandardFolders,
    Terminal, Trash, TrashedItem, Volume, Volumes, WindowChrome, WindowRef,
};

/// The distribution and the desktop: `PRETTY_NAME` of `/etc/os-release`
/// (`NAME` and `VERSION_ID` when it has none, `Linux` when the file is
/// missing), then `XDG_CURRENT_DESKTOP` after a middle dot when set.
pub(crate) fn os_description() -> String {
    let release = fs::read_to_string("/etc/os-release").unwrap_or_default();
    let mut text = distribution(&release).unwrap_or_else(|| "Linux".to_owned());
    if let Some(desktop) = std::env::var("XDG_CURRENT_DESKTOP")
        .ok()
        .map(|d| d.trim().to_owned())
        .filter(|d| !d.is_empty())
    {
        text.push_str(" · ");
        text.push_str(&desktop);
    }
    text
}

/// The distribution's name from an `os-release` text: `PRETTY_NAME`, else
/// `NAME` with `VERSION_ID`, else nothing.
fn distribution(release: &str) -> Option<String> {
    let value = |key: &str| {
        release.lines().find_map(|line| {
            let (name, value) = line.split_once('=')?;
            (name.trim() == key)
                .then(|| value.trim().trim_matches('"').trim_matches('\'').to_owned())
        })
    };
    if let Some(pretty) = value("PRETTY_NAME").filter(|p| !p.is_empty()) {
        return Some(pretty);
    }
    let name = value("NAME").filter(|n| !n.is_empty())?;
    Some(match value("VERSION_ID").filter(|v| !v.is_empty()) {
        Some(version) => format!("{name} {version}"),
        None => name,
    })
}

impl Trash for Native {
    fn trash(&self, path: &Path) -> Result<TrashedItem, Error> {
        trash::trash(path)
    }

    fn restore(&self, item: &TrashedItem) -> Result<(), Error> {
        trash::restore(item)
    }

    fn list(&self) -> Result<Vec<TrashedItem>, Error> {
        trash::list()
    }

    fn remove(&self, item: &TrashedItem) -> Result<(), Error> {
        trash::remove(item)
    }

    fn empty(&self) -> Result<(), Error> {
        trash::empty()
    }
}

impl Volumes for Native {
    fn volumes(&self) -> Result<Vec<Volume>, Error> {
        volumes::list()
    }

    fn eject(&self, volume: &Volume) -> Result<(), Error> {
        volumes::eject(volume)
    }

    fn has_format_tool(&self) -> bool {
        volumes::format_tool().is_some()
    }

    fn open_format_tool(&self, volume: &Volume) -> Result<(), Error> {
        volumes::open_format_tool(volume)
    }

    /// The kernel's own notice of a change to the mount table.
    fn watch_mounts(&self, changed: MountsChanged) -> Option<MountWatch> {
        volumes::watch_mounts(changed)
    }
}

impl Mount for Native {
    /// ISO and raw images, through `udisksctl loop-setup`.
    fn mountable(&self) -> &'static [ImageFormat] {
        crate::NATIVE_MOUNTABLE
    }

    fn mount_image(&self, path: &Path) -> Result<Volume, Error> {
        mount::mount(path)
    }
}

impl StandardFolders for Native {
    fn standard_folder(&self, folder: StandardFolder) -> Option<PathBuf> {
        user::standard_folder(folder, &self.home_dir())
    }
}

impl RecentlyUsed for Native {
    fn recently_used(&self) -> Result<Vec<RecentItem>, Error> {
        user::recently_used(&self.home_dir())
    }
}

fn spawn_detached(program: &str, args: &[&std::ffi::OsStr]) -> Result<(), Error> {
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
        spawn_detached("xdg-open", &[path.as_os_str()])
    }

    fn reveal(&self, path: &Path) -> Result<(), Error> {
        let path = std::path::absolute(path).map_err(Error::from_io)?;
        // The freedesktop file-manager interface selects the item; fall back
        // to opening the parent folder where no D-Bus session is reachable.
        let uri = format!(
            "array:string:file://{}",
            percent_encode(path.as_os_str().as_encoded_bytes())
        );
        let shown = Command::new("dbus-send")
            .args([
                "--session",
                "--dest=org.freedesktop.FileManager1",
                "--type=method_call",
                "/org/freedesktop/FileManager1",
                "org.freedesktop.FileManager1.ShowItems",
                &uri,
                "string:",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        if shown {
            return Ok(());
        }
        let parent = path.parent().unwrap_or(&path);
        spawn_detached("xdg-open", &[parent.as_os_str()])
    }

    fn applications(&self, path: &Path) -> Result<Vec<Application>, Error> {
        apps::applications(path)
    }

    fn open_with(&self, path: &Path, app: &Application) -> Result<(), Error> {
        apps::open_with(path, app)
    }
}

impl Terminal for Native {
    fn open_terminal(&self, dir: &Path, command: Option<&[String]>) -> Result<(), Error> {
        terminal::open(dir, command)
    }
}

impl Elevate for Native {
    fn run_elevated(
        &self,
        program: &Path,
        args: &[std::ffi::OsString],
    ) -> Result<ElevatedOutput, Error> {
        elevate::run(program, args)
    }
}

pub(crate) use crate::paths::percent_encode;
use trash::percent_decode;

impl Properties for Native {
    fn show(&self, path: &Path) -> Result<PropertiesOutcome, Error> {
        let path = std::path::absolute(path).map_err(Error::from_io)?;
        let meta = fs::symlink_metadata(&path).map_err(Error::from_io)?;
        let kind = if meta.is_dir() {
            "Folder"
        } else if meta.file_type().is_symlink() {
            "Symbolic link"
        } else if meta.is_file() {
            "File"
        } else {
            "Special file"
        };
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let location = path
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let mode = meta.permissions().mode();
        let mut rows = vec![
            ("Name".to_owned(), name),
            ("Type".to_owned(), kind.to_owned()),
            ("Location".to_owned(), location),
            ("Size".to_owned(), format!("{} bytes", meta.len())),
            (
                "On disk".to_owned(),
                format!("{} bytes", meta.blocks() * 512),
            ),
            (
                "Modified".to_owned(),
                meta.modified().map(iso8601).unwrap_or_default(),
            ),
            (
                "Accessed".to_owned(),
                meta.accessed().map(iso8601).unwrap_or_default(),
            ),
        ];
        if let Ok(created) = meta.created() {
            rows.push(("Created".to_owned(), iso8601(created)));
        }
        rows.push(("Owner".to_owned(), name_of(meta.uid(), "/etc/passwd")));
        rows.push(("Group".to_owned(), name_of(meta.gid(), "/etc/group")));
        rows.push((
            "Permissions".to_owned(),
            format!("{} ({:04o})", rwx(mode), mode & 0o7777),
        ));
        if meta.file_type().is_symlink()
            && let Ok(target) = fs::read_link(&path)
        {
            rows.push(("Target".to_owned(), target.display().to_string()));
        }
        Ok(PropertiesOutcome::Rows(rows))
    }
}

/// `uid`/`gid` as `name (id)` from a passwd-style file, or just the id.
fn name_of(id: u32, database: &str) -> String {
    fs::read_to_string(database)
        .ok()
        .and_then(|text| {
            text.lines().find_map(|line| {
                let mut fields = line.split(':');
                let name = fields.next()?;
                let found: u32 = fields.nth(1)?.parse().ok()?;
                (found == id).then(|| format!("{name} ({id})"))
            })
        })
        .unwrap_or_else(|| id.to_string())
}

fn rwx(mode: u32) -> String {
    let mut out = String::with_capacity(9);
    for shift in [6u32, 3, 0] {
        let bits = (mode >> shift) & 0o7;
        out.push(if bits & 4 != 0 { 'r' } else { '-' });
        out.push(if bits & 2 != 0 { 'w' } else { '-' });
        out.push(if bits & 1 != 0 { 'x' } else { '-' });
    }
    out
}

impl ConfigDir for Native {
    fn config_dir(&self) -> PathBuf {
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .unwrap_or_else(|| PathBuf::from(".config"))
            .join("rick")
    }

    fn home_dir(&self) -> PathBuf {
        std::env::var_os("HOME")
            .filter(|v| !v.is_empty())
            .map_or_else(|| PathBuf::from("."), PathBuf::from)
    }
}

impl Chrome for Native {
    fn window_chrome(&self) -> WindowChrome {
        WindowChrome {
            controls: ControlsSide::Right,
            native_controls: false,
            native_controls_width: 0.0,
        }
    }

    fn on_x11(&self) -> bool {
        x11_backend(|name| std::env::var(name).ok())
    }
}

/// Whether the windowing backend opens X11 windows, told before it has
/// opened one by the rule it chooses with (winit 0.30, `EventLoop::new`):
/// Wayland when `WAYLAND_DISPLAY`, or failing that `WAYLAND_SOCKET`, names a
/// display, X11 when only `DISPLAY` does. An empty variable counts as unset,
/// and so does one that is not Unicode. `var` reads the environment.
fn x11_backend(var: impl Fn(&str) -> Option<String>) -> bool {
    let set = |name: &str| var(name).is_some_and(|value| !value.is_empty());
    let wayland = set("WAYLAND_DISPLAY") || set("WAYLAND_SOCKET");
    !wayland && set("DISPLAY")
}

impl Activation for Native {
    fn activate_window(&self, window: &WindowRef, token: Option<&str>) -> Result<(), Error> {
        activate::activate(window, token)
    }

    /// Nothing: a Wayland compositor decides by the token, an X11 window
    /// manager by the message's source indication.
    fn allow_activation(&self) {}
}

impl DragSource for Native {
    /// Connects to the display the window is on: on Wayland, Rick's own
    /// seat and pointer start watching for the button press a drag needs.
    fn attach(&self, window: &WindowRef, wake: std::sync::Arc<dyn Fn() + Send + Sync>) {
        drag::attach(window, wake);
    }

    /// A `wl_data_source` offering `text/uri-list`, started from winit's
    /// surface with the press serial Rick's own pointer saw; or an XDND
    /// source on X11, moved along by [`drag_in_progress`](Self::drag_in_progress).
    fn start_drag(&self, window: &WindowRef, paths: &[PathBuf]) -> Result<(), Error> {
        drag::start_drag(window, paths)
    }

    fn drag_in_progress(&self) -> bool {
        drag::drag_in_progress()
    }
}

impl DropTarget for Native {
    /// On Wayland, the files Rick's own data device received; nothing on
    /// X11, where winit reports them.
    fn incoming_files(&self) -> IncomingFiles {
        drag::incoming_files()
    }

    /// The data device's last motion over the window on Wayland; the
    /// pointer queried on X11.
    fn drop_position(&self, window: &WindowRef) -> Option<Point> {
        drag::drop_position(window)
    }
}

#[cfg(test)]
mod tests {
    /// The distribution line: the pretty name first, the name and version
    /// when there is none, nothing from an empty file.
    #[test]
    fn the_distribution_comes_from_os_release() {
        let full = "NAME=\"Ubuntu\"\nVERSION_ID=\"24.04\"\nPRETTY_NAME=\"Ubuntu 24.04.5 LTS\"\n";
        assert_eq!(
            super::distribution(full).as_deref(),
            Some("Ubuntu 24.04.5 LTS")
        );
        let bare = "NAME=Fedora\nVERSION_ID=41\n";
        assert_eq!(super::distribution(bare).as_deref(), Some("Fedora 41"));
        assert_eq!(super::distribution("").as_deref(), None);
        assert_ne!(super::os_description(), "");
    }

    use super::*;

    #[test]
    fn properties_rows_describe_a_file() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let file = dir.path().join("x.txt");
        fs::write(&file, b"12345").unwrap_or_else(|e| panic!("{e}"));
        let rows = match Native.show(&file) {
            Ok(PropertiesOutcome::Rows(rows)) => rows,
            other => panic!("{other:?}"),
        };
        let get = |label: &str| {
            rows.iter()
                .find(|(l, _)| l == label)
                .map(|(_, v)| v.clone())
        };
        assert_eq!(get("Name"), Some("x.txt".to_owned()));
        assert_eq!(get("Type"), Some("File".to_owned()));
        assert_eq!(get("Size"), Some("5 bytes".to_owned()));
        assert!(get("Permissions").is_some_and(|p| p.len() > 9));
        assert!(matches!(
            Native.show(&dir.path().join("missing")),
            Err(Error::NotFound)
        ));
    }

    /// The windows are X11 windows when the environment names an X11
    /// display and no Wayland one, as the windowing backend chooses.
    #[test]
    fn the_windows_are_x11_windows_without_a_wayland_display() {
        let with = |vars: &'static [(&'static str, &'static str)]| {
            x11_backend(|name| {
                vars.iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| (*value).to_owned())
            })
        };
        assert!(with(&[("DISPLAY", ":0")]), "an X11 session");
        assert!(
            with(&[("DISPLAY", ":0"), ("WAYLAND_DISPLAY", "")]),
            "an empty variable is not set"
        );
        // A Wayland session, with or without Xwayland beside it.
        assert!(!with(&[("WAYLAND_DISPLAY", "wayland-0")]));
        assert!(!with(&[
            ("DISPLAY", ":0"),
            ("WAYLAND_DISPLAY", "wayland-0")
        ]));
        assert!(!with(&[("DISPLAY", ":0"), ("WAYLAND_SOCKET", "5")]));
        // No display at all: no window, and no X11 one.
        assert!(!with(&[]));
        assert!(!with(&[("DISPLAY", "")]));
    }

    #[test]
    fn config_dir_follows_xdg() {
        let dir = Native.config_dir();
        assert!(dir.ends_with("rick"), "{dir:?}");
    }

    #[test]
    fn percent_encoding_leaves_safe_bytes_alone() {
        assert_eq!(percent_encode(b"/tmp/a b%.txt"), "/tmp/a%20b%25.txt");
        assert_eq!(rwx(0o754), "rwxr-xr--");
    }

    #[test]
    fn without_a_display_nothing_drags_and_nothing_arrives() {
        // The fake window of the fake's tests is of no Linux backend, so the
        // native platform cannot connect and says so.
        let window = crate::fake::test_window();
        Native.attach(&window, std::sync::Arc::new(|| {}));
        assert!(matches!(
            Native.start_drag(&window, &[PathBuf::from("/tmp/x")]),
            Err(Error::Unsupported(_))
        ));
        assert!(!Native.drag_in_progress());
        assert_eq!(Native.incoming_files(), IncomingFiles::default());
        assert_eq!(Native.drop_position(&window), None);
    }
}
