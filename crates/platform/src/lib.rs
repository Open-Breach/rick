//! Every line of per-OS code in Rick, behind one trait set.
//!
//! Other crates program against the traits here and never against an
//! operating system: [`Trash`], [`OpenWith`], [`Terminal`], [`Elevate`],
//! [`Properties`], [`ConfigDir`], [`NativeDialogs`], [`Clipboard`],
//! [`Chrome`], [`DragSource`], [`Volumes`], [`StandardFolders`],
//! [`RecentlyUsed`] and [`Secrets`], bundled as [`Platform`]. [`Native`] is the real
//! implementation for the OS the binary runs on; [`Fake`] is the one tests
//! use, with a private trash directory and recorded calls.
//!
//! The [`fs`] module holds the filesystem fast paths the local provider
//! needs and no portable API offers: the bulk directory reader, the native
//! copy, allocated sizes and extent sharing. [`paths`] converts between
//! [`VPath`](types::VPath) and [`std::path::Path`].
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

mod clipboard;
mod dialogs;
mod fake;
pub mod fs;
pub mod paths;
pub mod secrets;
// Only the Linux trash and properties format dates; macOS parses them
// (Spotlight's last-used dates) and Windows does neither so far.
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
pub use traits::{
    Application, Chrome, Clipboard, ConfigDir, ControlsSide, DialogFilter, DragSource, Elevate,
    ElevatedOutput, NativeDialogs, OpenWith, Platform, Properties, PropertiesOutcome, RecentItem,
    RecentlyUsed, Secrets, StandardFolder, StandardFolders, Terminal, Trash, TrashedItem, Volume,
    VolumeKind, Volumes, WindowChrome,
};
pub use trash_record::TRASH_FILE;
use types::Error;

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
