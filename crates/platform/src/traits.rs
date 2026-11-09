//! The trait set the rest of Rick programs against.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use async_trait::async_trait;
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};
use types::Error;

use crate::instance::Endpoint;
use crate::perms::Permissions;

/// An entry in the trash, with enough to put it back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashedItem {
    /// Where the entry was before it was trashed.
    pub original: PathBuf,
    /// Where the trash keeps it now.
    pub trashed: PathBuf,
    /// When it was trashed.
    pub deleted_at: SystemTime,
}

/// The platform trash: move to trash, restore, list.
pub trait Trash: Send + Sync {
    /// Moves the entry at `path` to the trash.
    ///
    /// # Errors
    ///
    /// `Error::NotFound` when there is no such entry; `Error::Unsupported`
    /// when the entry's volume has no trash; otherwise the I/O error.
    fn trash(&self, path: &Path) -> Result<TrashedItem, Error>;

    /// Puts a trashed entry back at its original path.
    ///
    /// # Errors
    ///
    /// An `AlreadyExists` I/O error when the original path is taken again;
    /// `Error::NotFound` when the item is no longer in the trash.
    fn restore(&self, item: &TrashedItem) -> Result<(), Error>;

    /// Everything currently in the trash the current user can restore.
    /// Where the platform keeps the original paths to itself (macOS), this
    /// is Rick's own record of what it trashed
    /// ([`trash_record`](crate::trash_record)), so items trashed by other
    /// programs are not listed there.
    ///
    /// # Errors
    ///
    /// The I/O error from reading the trash or the record.
    fn list(&self) -> Result<Vec<TrashedItem>, Error>;

    /// Deletes a trashed entry for good, without restoring it.
    ///
    /// # Errors
    ///
    /// `Error::NotFound` when the item is no longer in the trash; otherwise
    /// the I/O error.
    fn remove(&self, item: &TrashedItem) -> Result<(), Error>;

    /// Deletes everything in the trash for good.
    ///
    /// # Errors
    ///
    /// The I/O or platform error.
    fn empty(&self) -> Result<(), Error>;
}

/// What kind of volume a mount is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeKind {
    /// An internal disk or partition that is not the system disk.
    Fixed,
    /// Removable media: a USB stick, a card, an external drive.
    Removable,
    /// An optical disc.
    Optical,
    /// A network file system the OS mounted (NFS, SMB, SSHFS, 9p, …).
    Network,
}

/// A mounted volume: a drive of the machine or a network mount.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Volume {
    /// The label, or the mount point's last component when it has none;
    /// `System` for the disk the operating system runs from.
    pub name: String,
    /// Where the volume is mounted: a mount point, or a drive root such as
    /// `C:\` on Windows.
    pub mount_point: PathBuf,
    /// Removable, network, optical or an internal disk.
    pub kind: VolumeKind,
    /// The file system, as the OS names it (`ext4`, `apfs`, `NTFS`, `cifs`).
    pub fs_type: String,
    /// The block device behind the mount, where the OS has one and the
    /// eject needs it (`/dev/sdb1` on Linux); `None` elsewhere.
    pub device: Option<String>,
    /// Capacity in bytes, when the OS reports it.
    pub total: Option<u64>,
    /// Free bytes available to the user, when the OS reports it.
    pub free: Option<u64>,
    /// Whether the operating system mounted the volume read-only: an
    /// optical disc, a live USB, a mounted ISO (SPEC.md *This Machine*).
    pub read_only: bool,
}

impl Volume {
    /// Bytes in use, when both the capacity and the free space are known.
    #[must_use]
    pub fn used(&self) -> Option<u64> {
        Some(self.total?.saturating_sub(self.free?))
    }

    /// The share of the capacity in use, 0 to 1, when known.
    #[must_use]
    pub fn utilisation(&self) -> Option<f32> {
        let total = self.total?;
        if total == 0 {
            return None;
        }
        // A share for a bar: the precision loss and the narrowing are fine.
        #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
        Some((self.used()? as f64 / total as f64) as f32)
    }

    /// Whether the volume is media the user can take out.
    #[must_use]
    pub fn is_removable(&self) -> bool {
        matches!(self.kind, VolumeKind::Removable | VolumeKind::Optical)
    }
}

/// The volumes the OS has mounted, for the side panel's This Machine and
/// Network sections. Mounting is not part of 0.2's milestone 2; ejecting
/// removable media is.
pub trait Volumes: Send + Sync {
    /// Every mounted volume the user can browse: the system disk, other
    /// internal disks, removable media and network mounts, each with its
    /// name, mount point, file system and space.
    ///
    /// # Errors
    ///
    /// The I/O or platform error from reading the mount table.
    fn volumes(&self) -> Result<Vec<Volume>, Error>;

    /// Unmounts a removable volume and powers it off or opens the tray where
    /// the OS can, so the medium can be taken out.
    ///
    /// # Errors
    ///
    /// `Error::Unsupported` for a volume that is not removable; otherwise
    /// the I/O or platform error, with what the OS said.
    fn eject(&self, volume: &Volume) -> Result<(), Error>;
}

/// A disk-image format, by the file's extension: what the *Mount* action
/// asks [`Mount::mountable`] about and what the archive provider browses
/// (SPEC.md "Disk images").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageFormat {
    /// An ISO 9660 or UDF disc image, `.iso`.
    Iso,
    /// A raw image of a disk or a partition, `.img`, `.ima`, `.raw`.
    Img,
    /// An Apple disk image, `.dmg`.
    Dmg,
    /// A Virtual PC or `Hyper-V` disk, `.vhd`.
    Vhd,
    /// A `Hyper-V` disk, `.vhdx`.
    Vhdx,
    /// A `VMware` disk, `.vmdk`.
    Vmdk,
    /// A `VirtualBox` disk, `.vdi`.
    Vdi,
    /// A QEMU copy-on-write disk, `.qcow2` or `.qcow`.
    Qcow2,
}

impl ImageFormat {
    /// Every format, in the order of the spec's Disk images table.
    pub const ALL: [Self; 8] = [
        Self::Iso,
        Self::Img,
        Self::Dmg,
        Self::Vhd,
        Self::Vhdx,
        Self::Vmdk,
        Self::Vdi,
        Self::Qcow2,
    ];

    /// The format a file name's extension names, if any; case does not
    /// matter.
    #[must_use]
    pub fn for_name(name: &str) -> Option<Self> {
        let (stem, ext) = name.rsplit_once('.')?;
        if stem.is_empty() {
            return None;
        }
        Some(match ext.to_ascii_lowercase().as_str() {
            "iso" => Self::Iso,
            "img" | "ima" | "raw" => Self::Img,
            "dmg" => Self::Dmg,
            "vhd" => Self::Vhd,
            "vhdx" => Self::Vhdx,
            "vmdk" => Self::Vmdk,
            "vdi" => Self::Vdi,
            "qcow2" | "qcow" => Self::Qcow2,
            _ => return None,
        })
    }

    /// The format's name for messages: `ISO`, `raw image`, `DMG`, …
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Iso => "ISO",
            Self::Img => "raw image",
            Self::Dmg => "DMG",
            Self::Vhd => "VHD",
            Self::Vhdx => "VHDX",
            Self::Vmdk => "VMDK",
            Self::Vdi => "VDI",
            Self::Qcow2 => "QCOW2",
        }
    }
}

/// Mounting a disk image through the operating system's own mechanism and
/// nothing else: `Mount-DiskImage` on Windows (ISO, VHD, VHDX), `hdiutil
/// attach` on macOS (DMG, ISO, raw images), `udisksctl loop-setup` on
/// Linux (ISO and raw images). No qemu-nbd, no FUSE. The mounted image
/// shows up in [`Volumes::volumes`] as a removable volume, and that
/// volume's [`Volumes::eject`] unmounts it; there is no separate unmount.
pub trait Mount: Send + Sync {
    /// The formats this operating system mounts natively; the *Mount*
    /// action hides itself on any other file.
    fn mountable(&self) -> &'static [ImageFormat];

    /// Mounts the image file at `path` through the OS and returns the
    /// volume it became, as [`Volumes::volumes`] lists it.
    ///
    /// # Errors
    ///
    /// `Error::Unsupported` for a format this OS does not mount, or when
    /// the mechanism is missing (udisks not installed on Linux); otherwise
    /// the platform error with what the OS said (a refused or damaged
    /// image).
    fn mount_image(&self, path: &Path) -> Result<Volume, Error>;
}

/// A folder the operating system sets aside for the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum StandardFolder {
    /// The home directory.
    Home,
    /// Documents.
    Documents,
    /// Pictures.
    Pictures,
    /// Music.
    Music,
    /// Video (`Videos` on Linux, `Movies` on macOS).
    Video,
    /// Downloads.
    Downloads,
    /// The desktop.
    Desktop,
}

impl StandardFolder {
    /// Every folder, in the order the side panel's defaults list them.
    pub const ALL: [Self; 7] = [
        Self::Home,
        Self::Documents,
        Self::Pictures,
        Self::Music,
        Self::Video,
        Self::Downloads,
        Self::Desktop,
    ];

    /// The title the side panel gives the folder.
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Documents => "Documents",
            Self::Pictures => "Pictures",
            Self::Music => "Music",
            Self::Video => "Video",
            Self::Downloads => "Downloads",
            Self::Desktop => "Desktop",
        }
    }
}

/// The user's standard folders, from the platform's conventions: XDG
/// user-dirs on Linux, the known folders on Windows, the fixed directories
/// under the home on macOS.
pub trait StandardFolders: Send + Sync {
    /// Where the folder is, when the platform names it and it exists.
    fn standard_folder(&self, folder: StandardFolder) -> Option<PathBuf>;
}

/// One entry of the system's recently-used list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentItem {
    /// The file or folder.
    pub path: PathBuf,
    /// When the user last used it.
    pub used_at: SystemTime,
}

/// The system's recently-used list: the XDG `recently-used.xbel` on Linux,
/// Spotlight's last-used dates on macOS, the Recent folder's shortcuts on
/// Windows. Rick keeps no list of its own.
pub trait RecentlyUsed: Send + Sync {
    /// The items the system remembers, newest first, those that still exist.
    ///
    /// # Errors
    ///
    /// The I/O or platform error from reading the list; `Error::Unsupported`
    /// where the platform keeps none.
    fn recently_used(&self) -> Result<Vec<RecentItem>, Error>;
}

/// An application the platform knows for a file type, or one the user
/// named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Application {
    /// What the user sees: `Text Editor`, `Preview`, `Notepad`.
    pub name: String,
    /// What launches it: a desktop file's path on Linux, an application
    /// bundle's path on macOS, an executable's path on Windows; or any
    /// executable the user typed, on every platform.
    pub id: String,
}

impl Application {
    /// An application from an executable the user typed, named after it.
    #[must_use]
    pub fn executable(command: &str) -> Self {
        let name = command
            .split_whitespace()
            .next()
            .and_then(|program| Path::new(program).file_name())
            .map_or_else(|| command.to_owned(), |n| n.to_string_lossy().into_owned());
        Self {
            name,
            id: command.to_owned(),
        }
    }
}

/// Handing entries to other applications.
pub trait OpenWith: Send + Sync {
    /// Opens the entry with the system default application.
    ///
    /// # Errors
    ///
    /// Fails when the platform refuses or has no handler for the entry.
    fn open(&self, path: &Path) -> Result<(), Error>;

    /// Shows the entry, selected, in the system file manager.
    ///
    /// # Errors
    ///
    /// Fails when the platform refuses or has no file manager.
    fn reveal(&self, path: &Path) -> Result<(), Error>;

    /// The applications the platform offers for the entry's type, the
    /// default first: the desktop files registered for its MIME type on
    /// Linux, Launch Services' list on macOS, the shell's handlers for its
    /// extension on Windows. Empty when the platform knows none.
    ///
    /// # Errors
    ///
    /// The platform error from reading the registrations.
    fn applications(&self, path: &Path) -> Result<Vec<Application>, Error>;

    /// Opens the entry with one application: one of
    /// [`applications`](Self::applications), or any executable the user
    /// named ([`Application::executable`]), which is run with the entry's
    /// path as its argument.
    ///
    /// # Errors
    ///
    /// Fails when the application cannot be started.
    fn open_with(&self, path: &Path, app: &Application) -> Result<(), Error>;
}

/// The user's terminal emulator.
pub trait Terminal: Send + Sync {
    /// Opens a terminal window in `dir`, running `command` in it when one
    /// is given and the user's shell otherwise: `$TERMINAL` or a known
    /// emulator on Linux, Terminal.app on macOS, Windows Terminal or
    /// `cmd` on Windows.
    ///
    /// # Errors
    ///
    /// `Error::Unsupported` when no terminal emulator is found; otherwise
    /// the error from starting it.
    fn open_terminal(&self, dir: &Path, command: Option<&[String]>) -> Result<(), Error>;
}

/// What a command run elevated produced.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ElevatedOutput {
    /// The exit code, when the process ended by itself.
    pub code: Option<i32>,
    /// What it wrote to standard output.
    pub stdout: Vec<u8>,
    /// What it wrote to standard error.
    pub stderr: Vec<u8>,
}

impl ElevatedOutput {
    /// Whether the command ended with exit code 0.
    #[must_use]
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

/// Running one command as the administrator through the platform's own
/// prompt, for the few steps that need it (the privileged helper of a
/// system-wide install): `pkexec` and its polkit prompt on Linux. Rick
/// itself never runs elevated; the program does, and it is a helper that
/// validates its input.
pub trait Elevate: Send + Sync {
    /// Runs `program` with `args` elevated and waits for it to end: through
    /// `pkexec` and its polkit prompt on Linux, Authorization Services on
    /// macOS, User Account Control on Windows. An elevated process on
    /// Windows shares no pipes with this one, so there the arguments get
    /// `--output <file>` appended and the file's contents come back as the
    /// standard output: a program run this way must honour that flag, as
    /// `rick-perms` does.
    ///
    /// # Errors
    ///
    /// `Error::Unsupported` where the platform offers no prompt (no
    /// `pkexec` on Linux); `Error::PermissionDenied` when the user dismissed
    /// the prompt or is not allowed; otherwise the error from starting the
    /// program.
    fn run_elevated(&self, program: &Path, args: &[OsString]) -> Result<ElevatedOutput, Error>;
}

/// What showing properties did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropertiesOutcome {
    /// The platform opened its own dialogue (Windows Properties, macOS Get
    /// Info).
    Native,
    /// The platform has no dialogue; Rick shows these label and value rows.
    Rows(Vec<(String, String)>),
}

/// The system properties of an entry.
pub trait Properties: Send + Sync {
    /// Shows the platform's properties dialogue, or returns the rows for
    /// Rick's own.
    ///
    /// # Errors
    ///
    /// `Error::NotFound` when there is no such entry; otherwise the I/O or
    /// platform error.
    fn show(&self, path: &Path) -> Result<PropertiesOutcome, Error>;
}

/// Where Rick keeps its configuration.
pub trait ConfigDir: Send + Sync {
    /// The directory of `rick.toml` and the extension TOML files:
    /// `$XDG_CONFIG_HOME/rick` on Linux, `~/Library/Application Support/Rick`
    /// on macOS, `%APPDATA%\Rick` on Windows. Not created.
    fn config_dir(&self) -> PathBuf;

    /// The user's home directory: `$HOME` on Linux and macOS, the profile
    /// directory (`%USERPROFILE%`) on Windows; the current directory when
    /// neither is set.
    fn home_dir(&self) -> PathBuf;
}

/// A file-type filter for a file dialogue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DialogFilter {
    /// What the user sees, e.g. `"Images"`.
    pub name: &'static str,
    /// Extensions without the dot, e.g. `&["png", "jpg"]`.
    pub extensions: &'static [&'static str],
}

/// The platform's own file dialogues. Async because the UI thread never
/// blocks: the dialogue runs while the window keeps painting.
#[async_trait]
pub trait NativeDialogs: Send + Sync {
    /// Asks for a directory; `None` when dismissed.
    async fn pick_folder(&self, title: &str) -> Option<PathBuf>;

    /// Asks for one or more files; empty when dismissed.
    async fn pick_files(&self, title: &str, filters: &[DialogFilter]) -> Vec<PathBuf>;

    /// Asks where to save a file; `None` when dismissed.
    async fn save_file(&self, title: &str, suggested: &str) -> Option<PathBuf>;
}

/// The system clipboard, as far as Rick needs it: text.
pub trait Clipboard: Send + Sync {
    /// Replaces the clipboard's contents with `text`.
    ///
    /// # Errors
    ///
    /// `Error::Unsupported` when no clipboard is reachable (no display
    /// server); otherwise the platform error.
    fn set_text(&self, text: &str) -> Result<(), Error>;

    /// The clipboard's text, `None` when it holds none.
    ///
    /// # Errors
    ///
    /// `Error::Unsupported` when no clipboard is reachable; otherwise the
    /// platform error.
    fn text(&self) -> Result<Option<String>, Error>;
}

/// Which side of the title row the window controls sit on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlsSide {
    /// Minimise, maximise and close on the left, as on macOS.
    Left,
    /// On the right, as on Windows and most Linux desktops.
    Right,
}

/// How the window's chrome is drawn when Rick draws its own title row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowChrome {
    /// Where the window controls are.
    pub controls: ControlsSide,
    /// The system keeps drawing its own controls (the macOS traffic lights)
    /// and Rick leaves room for them; otherwise Rick draws minimise,
    /// maximise and close itself.
    pub native_controls: bool,
    /// Width in logical points to leave free for the native controls.
    pub native_controls_width: f32,
}

/// Window chrome placement, per platform.
pub trait Chrome: Send + Sync {
    /// How this platform's windows are decorated when the title bar is
    /// Rick's own.
    fn window_chrome(&self) -> WindowChrome;
}

/// The window a drag starts from or a drop lands in, as the windowing
/// backend describes it: `winit`'s raw window and display handles, and the
/// window's scale. Only the thread that runs the window's event loop may
/// use it, which is the thread every [`DragSource`] and [`DropTarget`]
/// method is called on.
#[derive(Debug, Clone, Copy)]
pub struct WindowRef {
    /// The window: an `NSView` on macOS, an `HWND` on Windows, a
    /// `wl_surface` or an X11 window id on Linux.
    pub window: RawWindowHandle,
    /// The display the window is on: the `wl_display`, the X11 display.
    pub display: RawDisplayHandle,
    /// Physical pixels per logical point of the window, for the backends
    /// that report positions in pixels (X11).
    pub scale: f32,
}

/// A position inside a window, in logical points from its top-left corner,
/// the coordinates egui works in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    /// From the left edge.
    pub x: f32,
    /// From the top edge.
    pub y: f32,
}

/// Starting a drag the rest of the desktop can receive: the half of
/// drag-and-drop the windowing backend does not do for Rick. An
/// `NSDraggingSession` on macOS, OLE's `DoDragDrop` with a `CF_HDROP` on
/// Windows, a `wl_data_source` on Wayland and an XDND source on X11, each
/// from the window `winit` hands out through its raw window handle.
///
/// The drag carries the items' `file://` URIs and the receiver decides what
/// to do with them. The windowing backend never sees the button come up
/// after a drag it did not run, so the UI polls
/// [`DragSource::drag_in_progress`] and puts its pointer state right when
/// the drag ends. Every method is called on the thread that runs the
/// window's event loop.
pub trait DragSource: Send + Sync {
    /// Tells the platform the window drags may start from, once the window
    /// exists. Where starting a drag needs something only seen before the
    /// drag (on Wayland, the serial of the button press that started it),
    /// this is where the platform starts listening for it. `wake` asks the
    /// UI for a frame; the platform calls it, from any thread, when a drag
    /// it watches has news the UI would otherwise not hear of (a drag from
    /// outside entering a Wayland window while the pointer is quiet).
    fn attach(&self, window: &WindowRef, wake: Arc<dyn Fn() + Send + Sync>);

    /// Starts a platform drag of `paths` from `window`, as `file://` URIs,
    /// so another application or another Rick process can take them when
    /// the cursor leaves the window. Returns once the drag is the
    /// platform's: on Windows, where `DoDragDrop` runs its own loop, that is
    /// when the drag has ended; elsewhere the drag goes on in the event
    /// loop and [`drag_in_progress`](Self::drag_in_progress) says when it
    /// has ended.
    ///
    /// # Errors
    ///
    /// `Error::Unsupported`, with the reason, when the platform gives Rick no
    /// way to start one from this window (no display connection, no button
    /// press seen, a window handle of another backend); the UI then keeps
    /// the drag inside the window.
    fn start_drag(&self, window: &WindowRef, paths: &[PathBuf]) -> Result<(), Error>;

    /// Whether a drag started through [`start_drag`](Self::start_drag) is
    /// still running. Polled every frame while one is, so the platform can
    /// do what the drag needs in between (answer the receiver's request for
    /// the data, move the XDND source along).
    fn drag_in_progress(&self) -> bool;
}

/// Files another application drags over or drops on a window, as far as the
/// platform receives them itself. Empty where the windowing backend does
/// that and hands them to egui as hovered and dropped files (macOS, Windows,
/// X11); on Wayland, where `winit` 0.30 has no drop target, the platform's
/// own data device fills these.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IncomingFiles {
    /// A drag from outside is over one of the windows, its files perhaps
    /// not read yet; the UI keeps asking while this holds.
    pub hovering: bool,
    /// The files of the drag over the window right now, once read.
    pub hovered: Vec<PathBuf>,
    /// The files dropped on the window since the last call.
    pub dropped: Vec<PathBuf>,
}

/// The receiving half of a drag from outside the process: where the cursor
/// is, which the windowing backend reports with no position, and the files
/// themselves where the backend reports nothing at all. Every method is
/// called on the thread that runs the window's event loop.
pub trait DropTarget: Send + Sync {
    /// The files a drag from outside hovers over or dropped on Rick's
    /// windows, where the platform tracks them itself (see
    /// [`IncomingFiles`]). Cheap; polled every frame.
    fn incoming_files(&self) -> IncomingFiles;

    /// Where the cursor of a drag from outside is over `window` right now,
    /// for the files the backend or [`incoming_files`](Self::incoming_files)
    /// reports as hovered or dropped. `None` where the platform cannot tell:
    /// the UI then lets the drop land in the focused pane's folder, as the
    /// spec's Drag and drop table says.
    fn drop_position(&self, window: &WindowRef) -> Option<Point>;
}

/// The platform keychain: secrets by a stable key, never in a TOML file.
/// Secret Service on Linux, Keychain on macOS, Credential Manager on
/// Windows; the fake keeps them in memory. The key is the caller's: the
/// connection plugins use `<provider>/<connection>/<field>`.
pub trait Secrets: Send + Sync {
    /// Stores `secret` under `key`, replacing what was there.
    ///
    /// # Errors
    ///
    /// `Error::Unsupported` when no keychain is reachable (no Secret Service
    /// on the bus); otherwise the platform error.
    fn store_secret(&self, key: &str, secret: &str) -> Result<(), Error>;

    /// The secret stored under `key`, or `None` when there is none.
    ///
    /// # Errors
    ///
    /// `Error::Unsupported` when no keychain is reachable; otherwise the
    /// platform error.
    fn get_secret(&self, key: &str) -> Result<Option<String>, Error>;

    /// Forgets the secret under `key`; a key without a secret is fine.
    ///
    /// # Errors
    ///
    /// `Error::Unsupported` when no keychain is reachable; otherwise the
    /// platform error.
    fn remove_secret(&self, key: &str) -> Result<(), Error>;
}

/// A font file the system names: its bytes, the face index inside a
/// collection (`.ttc`), and the family the desktop shows it as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontFile {
    /// The family name, `Inter`, `Segoe UI`, `Menlo`.
    pub family: String,
    /// The file's bytes, unparsed.
    pub bytes: Vec<u8>,
    /// The face in a collection; 0 for a single-face file.
    pub index: u32,
}

/// The fonts the desktop sets its own windows in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Fonts {
    /// The UI font, for everything that is not code.
    pub ui: Option<FontFile>,
    /// The monospace font, for hashes, paths and code.
    pub mono: Option<FontFile>,
}

/// The system's fonts (SPEC.md *Look and feel*, Typography).
pub trait SystemFonts: Send + Sync {
    /// The system's UI font and its monospace font, each when the platform
    /// can name one; the caller keeps its bundled fonts for the rest and
    /// validates the bytes before it uses them.
    fn system_fonts(&self) -> Fonts;
}

/// The system's reduce-motion setting.
pub trait Motion: Send + Sync {
    /// Whether the user asked the system for less motion: Rick's loading
    /// animations then stand still (SPEC.md *Loading animations*).
    fn reduce_motion(&self) -> bool;
}

/// The single-instance endpoint (specs/system-integration.md §3): where
/// the one Rick of this user and config directory listens for the command
/// line of a second launch.
pub trait SingleInstance: Send + Sync {
    /// The endpoint for `config_dir`: a Unix socket in `$XDG_RUNTIME_DIR`
    /// (Linux) or the user's temporary directory (macOS), a named pipe
    /// under `\\.\pipe\` (Windows), named after the user and the
    /// directory, so two installs with different config directories
    /// coexist. The fake answers one inside its own directory.
    fn instance_endpoint(&self, config_dir: &Path) -> Endpoint;
}

/// All platform services behind one object.
pub trait Platform:
    Trash
    + OpenWith
    + Terminal
    + Elevate
    + Properties
    + Permissions
    + ConfigDir
    + NativeDialogs
    + Clipboard
    + Chrome
    + DragSource
    + DropTarget
    + Volumes
    + Mount
    + StandardFolders
    + RecentlyUsed
    + Secrets
    + SystemFonts
    + Motion
    + SingleInstance
{
}

impl<
    T: Trash
        + OpenWith
        + Terminal
        + Elevate
        + Properties
        + Permissions
        + ConfigDir
        + NativeDialogs
        + Clipboard
        + Chrome
        + DragSource
        + DropTarget
        + Volumes
        + Mount
        + StandardFolders
        + RecentlyUsed
        + Secrets
        + SystemFonts
        + Motion
        + SingleInstance,
> Platform for T
{
}
