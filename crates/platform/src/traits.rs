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
    ///
    /// # Errors
    ///
    /// The I/O error from reading the trash.
    fn list(&self) -> Result<Vec<TrashedItem>, Error>;
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

/// All platform services behind one object.
pub trait Platform:
    Trash + OpenWith + Properties + ConfigDir + NativeDialogs + Clipboard + Chrome + DragSource
{
}

impl<T: Trash + OpenWith + Properties + ConfigDir + NativeDialogs + Clipboard + Chrome + DragSource>
    Platform for T
{
}
