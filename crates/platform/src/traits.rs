//! The trait set the rest of Rick programs against.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use async_trait::async_trait;
use types::Error;

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

/// Starting a drag the rest of the desktop can receive: the half of
/// drag-and-drop the windowing backend does not do for Rick. Receiving a
/// drop is the backend's (`winit` hands egui the hovered and dropped files
/// on every platform); starting one is this trait's.
pub trait DragSource: Send + Sync {
    /// Starts a platform drag carrying `paths` as `file://` URIs, so another
    /// application or another Rick process can take them when the cursor
    /// leaves the window. Returns once the drag has been handed to the
    /// platform; the drop itself is the receiver's business.
    ///
    /// # Errors
    ///
    /// `Error::Unsupported`, with the reason, where the platform gives Rick no
    /// way to start one from the window it has (see QUESTIONS.md); the UI
    /// then keeps the drag inside the window.
    fn start_drag(&self, paths: &[PathBuf]) -> Result<(), Error>;
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

/// All platform services behind one object.
pub trait Platform:
    Trash
    + OpenWith
    + Properties
    + ConfigDir
    + NativeDialogs
    + Clipboard
    + Chrome
    + DragSource
    + Volumes
    + StandardFolders
    + RecentlyUsed
    + Secrets
{
}

impl<
    T: Trash
        + OpenWith
        + Properties
        + ConfigDir
        + NativeDialogs
        + Clipboard
        + Chrome
        + DragSource
        + Volumes
        + StandardFolders
        + RecentlyUsed
        + Secrets,
> Platform for T
{
}
