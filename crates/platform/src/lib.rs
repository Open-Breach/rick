//! Every line of per-OS code in Rick, behind one trait set.
//!
//! Other crates program against the traits here and never against an
//! operating system: [`Trash`], [`OpenWith`], [`Terminal`], [`Elevate`],
//! [`Properties`], [`ConfigDir`], [`NativeDialogs`], [`Clipboard`],
//! [`Chrome`], [`DragSource`], [`DropTarget`], [`Volumes`], [`Mount`] (a
//! disk image mounted through the OS: `udisksctl loop-setup`, `hdiutil
//! attach`, `Mount-DiskImage`), [`StandardFolders`], [`RecentlyUsed`] and
//! [`Secrets`], bundled as [`Platform`]. [`Native`] is the real
//! implementation for the OS the binary runs on; [`Fake`] is the one tests
//! use, with a private trash directory and recorded calls.
//!
//! Drag and drop across processes is split between the windowing backend
//! and this crate: `winit` receives drops on macOS, Windows and X11 and
//! hands them to egui without a cursor position, so [`DropTarget`] supplies
//! the position (and, on Wayland, where `winit` 0.30 receives nothing, the
//! files too); [`DragSource`] starts the drag the other way, from the
//! window the backend describes through a [`WindowRef`] of raw handles.
//!
//! The [`fs`] module holds the filesystem fast paths the local provider
//! needs and no portable API offers: the bulk directory reader, the native
//! copy, allocated sizes and extent sharing. [`paths`] converts between
//! [`VPath`](types::VPath) and [`std::path::Path`]; [`agent`] opens the
//! SSH agent's socket, a Unix socket or the OpenSSH named pipe; [`perms`]
//! reads and changes an entry's owner, group, mode bits and ACL, and
//! speaks to `rick-perms`, the helper that does so as the administrator;
//! [`sevenzip`] binds the 7-Zip library (`7z.so`, `7z.dll`) at run time
//! for the disk images the archive plugins browse, the one place its
//! `unsafe` lives.
//!
//! Where the OS keeps the original paths of trashed items to itself
//! (macOS), the trash lists Rick's own record of what it trashed instead,
//! kept in [`trash_record`] under the config directory. [`Secrets`] is the
//! platform keychain through the `keyring` crate ([`secrets`]): Secret
//! Service on Linux, Keychain on macOS, Credential Manager on Windows.
//!
//! This is the only crate allowed `unsafe` and `cfg(target_os)`. Every
//! `unsafe` block carries a `// SAFETY:` comment. The [`Clipboard`] is text
//! only and goes through `arboard` on all three platforms.

// The point of this crate: FFI to the OS lives here and nowhere else.
#![allow(unsafe_code)]

pub mod agent;
mod clipboard;
mod dialogs;
pub mod fonts;
pub mod motion;
// The URI list is written and read on Linux only; macOS and Windows hand
// the OS paths and get paths back. The tests exercise it everywhere.
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
mod dnd;
mod fake;
pub mod fs;
pub mod paths;
pub mod perms;
pub mod secrets;
pub mod sevenzip;
// The Linux trash formats dates and macOS parses them (Spotlight's
// last-used dates); the Properties rows format them everywhere.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod time;
mod traits;
pub mod trash_record;
#[cfg(unix)]
mod volumes;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
compile_error!("Rick supports Linux, macOS and Windows");

pub use async_trait::async_trait;
pub use fake::{FAKE_APPLICATIONS, FAKE_STANDARD_FOLDERS, Fake};
pub use perms::{
    Acl, DarwinAce, FilePermissions, Nfs4Ace, Nfs4Kind, NtAce, PermissionChange, Permissions,
    PosixAce, PosixTag, Principal, PrincipalKind, Principals,
};
/// Re-exported so the UI can build a [`WindowRef`] from the handles its
/// windowing backend gives out without depending on the crate itself.
pub use raw_window_handle;
pub use time::iso8601;
pub use traits::{
    Application, Chrome, Clipboard, ConfigDir, ControlsSide, DialogFilter, DragSource, DropTarget,
    Elevate, ElevatedOutput, FontFile, Fonts, ImageFormat, IncomingFiles, Motion, Mount,
    NativeDialogs, OpenWith, Platform, Point, Properties, PropertiesOutcome, RecentItem,
    RecentlyUsed, Secrets, StandardFolder, StandardFolders, SystemFonts, Terminal, Trash,
    TrashedItem, Volume, VolumeKind, Volumes, WindowChrome, WindowRef,
};
pub use trash_record::TRASH_FILE;
use types::Error;

/// The disk-image formats this operating system mounts natively, as
/// [`Mount::mountable`] answers for [`Native`] and the [`Fake`] by default:
/// ISO, VHD and VHDX on Windows, DMG, ISO and raw images on macOS, ISO and
/// raw images on Linux.
pub const NATIVE_MOUNTABLE: &[ImageFormat] = native_mountable();

const fn native_mountable() -> &'static [ImageFormat] {
    #[cfg(target_os = "linux")]
    {
        &[ImageFormat::Iso, ImageFormat::Img]
    }
    #[cfg(target_os = "macos")]
    {
        &[ImageFormat::Dmg, ImageFormat::Iso, ImageFormat::Img]
    }
    #[cfg(windows)]
    {
        &[ImageFormat::Iso, ImageFormat::Vhd, ImageFormat::Vhdx]
    }
}

/// Whether the platform has a properties dialogue of its own
/// ([`Properties::show`] opens it): Windows and macOS. Rick's own dialogue
/// offers it as *System properties…* there.
pub const NATIVE_PROPERTIES: bool = cfg!(any(windows, target_os = "macos"));

/// The platform the binary runs on.
///
/// Zero-sized; every method dispatches to the OS implementation chosen at
/// compile time.
#[derive(Debug, Clone, Copy, Default)]
pub struct Native;

impl Native {
    /// The native platform.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl SystemFonts for Native {
    fn system_fonts(&self) -> Fonts {
        fonts::system()
    }
}

impl Motion for Native {
    fn reduce_motion(&self) -> bool {
        motion::system()
    }
}

impl Secrets for Native {
    fn store_secret(&self, key: &str, secret: &str) -> Result<(), Error> {
        secrets::store(key, secret)
    }

    fn get_secret(&self, key: &str) -> Result<Option<String>, Error> {
        secrets::get(key)
    }

    fn remove_secret(&self, key: &str) -> Result<(), Error> {
        secrets::remove(key)
    }
}

#[async_trait]
impl NativeDialogs for Native {
    async fn pick_folder(&self, title: &str) -> Option<std::path::PathBuf> {
        dialogs::pick_folder(title).await
    }

    async fn pick_files(&self, title: &str, filters: &[DialogFilter]) -> Vec<std::path::PathBuf> {
        dialogs::pick_files(title, filters).await
    }

    async fn save_file(&self, title: &str, suggested: &str) -> Option<std::path::PathBuf> {
        dialogs::save_file(title, suggested).await
    }
}
