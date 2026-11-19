//! Every line of per-OS code in Rick, behind one trait set.
//!
//! Other crates program against the traits here and never against an
//! operating system: [`Trash`], [`OpenWith`], [`Terminal`], [`Elevate`],
//! [`Properties`], [`ConfigDir`], [`NativeDialogs`], [`Clipboard`],
//! [`Chrome`], [`DragSource`], [`DropTarget`], [`Volumes`], [`Mount`] (a
//! disk image mounted through the OS: `udisksctl loop-setup`, `hdiutil
//! attach`, `Mount-DiskImage`), [`StandardFolders`], [`RecentlyUsed`],
//! [`Secrets`], [`SingleInstance`] and [`Activation`] (raising the window a
//! second launch's path opened in: `xdg_activation_v1` with the launcher's
//! token, `_NET_ACTIVE_WINDOW`, `NSApp.activate`, `SetForegroundWindow`),
//! and [`Integration`] (the integration items of
//! specs/system-integration.md §6 and §7: list those of this platform and
//! desktop, check one, perform one, undo one; [`integration`]),
//! bundled as [`Platform`]. [`file_manager`] is the freedesktop
//! file-manager interface, `org.freedesktop.FileManager1`, which the
//! running Rick owns on the Linux session bus. [`process`] asks the system whether the shell
//! in a console has handed its terminal to a job. [`Native`] is the real
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
//! `unsafe` lives; [`instance`] is the single-instance endpoint
//! ([`SingleInstance`] names it per user and config directory: a Unix
//! socket or a named pipe) and the one message a second launch hands the
//! running Rick over it.
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
pub mod file_manager;
pub mod fs;
pub mod instance;
pub mod integration;
pub mod paths;
pub mod perms;
pub mod process;
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
pub use integration::{FakeIntegration, Integration};
/// The Linux integration items on an environment of the caller's choosing:
/// what [`Native`] runs on the process's own, and what a test runs on a
/// temporary home.
#[cfg(target_os = "linux")]
pub use linux::integration::{Gsettings, LinuxEnv, LinuxIntegration};
pub use perms::{
    Acl, DarwinAce, FilePermissions, Nfs4Ace, Nfs4Kind, NtAce, PermissionChange, Permissions,
    PosixAce, PosixTag, Principal, PrincipalKind, Principals,
};
/// Re-exported so the UI can build a [`WindowRef`] from the handles its
/// windowing backend gives out without depending on the crate itself.
pub use raw_window_handle;
pub use time::iso8601;
pub use traits::{
    Activation, Application, Chrome, Clipboard, ConfigDir, ControlsSide, DesktopBackground,
    DialogFilter, DragSource, DropTarget, Elevate, ElevatedOutput, FontFile, Fonts, ImageFormat,
    IncomingFiles, Monitor, Motion, Mount, MountWatch, MountsChanged, NativeDialogs, OpenWith,
    Platform, Point, Properties, PropertiesOutcome, RecentItem, RecentlyUsed, Secrets,
    SingleInstance, StandardFolder, StandardFolders, SystemFonts, Terminal, Trash, TrashedItem,
    Volume, VolumeKind, Volumes, WindowChrome, WindowRef,
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

/// The operating system and desktop this process runs on, one line for the
/// About dialog's details (specs/about-dialog.md): on Linux the
/// distribution's `PRETTY_NAME` from `/etc/os-release` and
/// `XDG_CURRENT_DESKTOP` when set (`Ubuntu 24.04.1 LTS · GNOME`); on macOS
/// the product version (`macOS 15.1`); on Windows the edition, display
/// version and build (`Windows 11 Pro 24H2 (build 26100)`). Never fails:
/// what cannot be read is left out, down to the bare name of the system.
#[must_use]
pub fn os_description() -> String {
    #[cfg(target_os = "linux")]
    {
        linux::os_description()
    }
    #[cfg(target_os = "macos")]
    {
        macos::os_description()
    }
    #[cfg(windows)]
    {
        windows::os_description()
    }
}

/// Rick's cache directory: what Rick fetched and can fetch again, kept
/// apart from the settings so that a backup or a sync of the config
/// directory does not carry it. `$XDG_CACHE_HOME/rick` on Linux
/// (`~/.cache/rick` without the variable), `~/Library/Caches/Rick` on
/// macOS, `%LOCALAPPDATA%\Rick\Cache` on Windows. Not created.
///
/// This is the directory of a Rick on the platform's own config directory.
/// A Rick started on another one keeps its cache inside that directory:
/// the application decides which of the two it is.
#[must_use]
pub fn cache_dir() -> std::path::PathBuf {
    cache_dir_of(|name| std::env::var_os(name))
}

/// [`cache_dir`] in an environment of the caller's: `var` answers for a
/// variable's name.
fn cache_dir_of(var: impl Fn(&str) -> Option<std::ffi::OsString>) -> std::path::PathBuf {
    use std::path::PathBuf;
    let set = |name: &str| var(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    #[cfg(target_os = "linux")]
    {
        set("XDG_CACHE_HOME")
            .or_else(|| set("HOME").map(|home| home.join(".cache")))
            .unwrap_or_else(|| PathBuf::from(".cache"))
            .join("rick")
    }
    #[cfg(target_os = "macos")]
    {
        set("HOME")
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Library/Caches/Rick")
    }
    #[cfg(windows)]
    {
        set("LOCALAPPDATA")
            .or_else(|| set("USERPROFILE").map(|profile| profile.join("AppData").join("Local")))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Rick")
            .join("Cache")
    }
}

/// Whether the platform has a properties dialogue of its own
/// ([`Properties::show`] opens it): Windows and macOS. Rick's own dialogue
/// offers it as *System properties…* there.
pub const NATIVE_PROPERTIES: bool = cfg!(any(windows, target_os = "macos"));

/// How this platform names the key the spec calls Mod: *Cmd* on macOS,
/// *Ctrl* on Linux and Windows. A shortcut hint is spelled with it; a
/// binding is stored as `Mod`.
pub const MOD_KEY: &str = if cfg!(target_os = "macos") {
    "Cmd"
} else {
    "Ctrl"
};

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

impl Integration for Native {
    fn integration_items(&self) -> integration::Items {
        #[cfg(target_os = "linux")]
        {
            LinuxIntegration::native().integration_items()
        }
        #[cfg(not(target_os = "linux"))]
        {
            integration::none::items()
        }
    }

    fn check_integration(
        &self,
        item: integration::Item,
        request: &integration::Request,
    ) -> integration::Check {
        #[cfg(target_os = "linux")]
        {
            LinuxIntegration::native().check_integration(item, request)
        }
        #[cfg(not(target_os = "linux"))]
        {
            integration::none::check(item, request)
        }
    }

    fn perform_integration(
        &self,
        item: integration::Item,
        request: &integration::Request,
    ) -> Result<integration::Receipt, Error> {
        #[cfg(target_os = "linux")]
        {
            LinuxIntegration::native().perform_integration(item, request)
        }
        #[cfg(not(target_os = "linux"))]
        {
            integration::none::perform(item, request)
        }
    }

    fn undo_integration(
        &self,
        item: integration::Item,
        request: &integration::Request,
    ) -> Result<(), Error> {
        #[cfg(target_os = "linux")]
        {
            LinuxIntegration::native().undo_integration(item, request)
        }
        #[cfg(not(target_os = "linux"))]
        {
            integration::none::undo(item, request)
        }
    }
}

impl SingleInstance for Native {
    fn instance_endpoint(&self, config_dir: &std::path::Path) -> instance::Endpoint {
        instance::Endpoint::for_config_dir(config_dir)
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

    async fn pick_folder_in(
        &self,
        title: &str,
        start: &std::path::Path,
    ) -> Option<std::path::PathBuf> {
        dialogs::pick_folder_in(title, start).await
    }

    async fn pick_files(&self, title: &str, filters: &[DialogFilter]) -> Vec<std::path::PathBuf> {
        dialogs::pick_files(title, filters).await
    }

    async fn save_file(&self, title: &str, suggested: &str) -> Option<std::path::PathBuf> {
        dialogs::save_file(title, suggested).await
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;

    use super::cache_dir_of;

    /// An environment of the given variables and no others.
    fn env(vars: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<OsString> {
        move |name| {
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        }
    }

    /// The cache directory is where the platform keeps what can be
    /// fetched again, and never inside the config directory.
    #[test]
    fn the_cache_directory_is_the_platform_s_own() {
        #[cfg(target_os = "linux")]
        {
            let xdg = env(&[("XDG_CACHE_HOME", "/var/cache/me"), ("HOME", "/home/me")]);
            assert_eq!(cache_dir_of(xdg), PathBuf::from("/var/cache/me/rick"));
            let home = env(&[("XDG_CACHE_HOME", ""), ("HOME", "/home/me")]);
            assert_eq!(cache_dir_of(home), PathBuf::from("/home/me/.cache/rick"));
            assert_eq!(cache_dir_of(env(&[])), PathBuf::from(".cache/rick"));
        }
        #[cfg(target_os = "macos")]
        {
            let home = env(&[("HOME", "/Users/me")]);
            assert_eq!(
                cache_dir_of(home),
                PathBuf::from("/Users/me/Library/Caches/Rick")
            );
        }
        #[cfg(windows)]
        {
            let local = env(&[("LOCALAPPDATA", r"C:\Users\me\AppData\Local")]);
            assert_eq!(
                cache_dir_of(local),
                PathBuf::from(r"C:\Users\me\AppData\Local\Rick\Cache")
            );
            let profile = env(&[("USERPROFILE", r"C:\Users\me")]);
            assert_eq!(
                cache_dir_of(profile),
                PathBuf::from(r"C:\Users\me\AppData\Local\Rick\Cache")
            );
        }
        // This process's own: under a folder named after Rick.
        let own = super::cache_dir();
        assert!(
            own.to_string_lossy().to_lowercase().contains("rick"),
            "{own:?}"
        );
    }
}
