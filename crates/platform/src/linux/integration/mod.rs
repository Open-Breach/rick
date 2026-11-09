//! The Linux integration items (specs/system-integration.md §7.1), each
//! in its own module: the install location (checked here, performed by
//! the installer plugin), the app launcher entry ([`launcher`]), the
//! default file manager ([`mime`]), the `org.freedesktop.FileManager1`
//! activation file ([`service`]), the `rick` command ([`command`]) and
//! the dock favourite of GNOME, KDE Plasma, COSMIC and Cinnamon
//! ([`dock`]).
//!
//! Everything works on a [`LinuxEnv`], the directories and the binary an
//! item is about, so the same code runs against the user's real home and
//! against the temporary one of a test; nothing here reads the process
//! environment except [`LinuxEnv::from_process`]. `gsettings` is behind
//! the [`Gsettings`] trait for the same reason.

mod command;
mod dock;
mod launcher;
mod mime;
mod service;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use types::Error;

pub use dock::Gsettings;

use crate::integration::{Actual, Check, Desktop, Integration, Item, Items, Receipt, Request};

/// The app id Rick installs itself under: `~/.local/opt/rick`.
const APP_ID: &str = "rick";
/// The name of the binary inside the install folder.
const BINARY: &str = "rick";
/// The desktop entry's file name, the one identifier of
/// specs/app-icons.md.
const DESKTOP_ID: &str = "rick.desktop";

/// Where the integration items live and which Rick they are about: the
/// user's directories as the XDG variables name them, the desktop, and
/// the running binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinuxEnv {
    /// `$HOME`.
    pub home: PathBuf,
    /// `$XDG_DATA_HOME`, `~/.local/share` when unset: the launcher entry,
    /// the icons and the D-Bus service file go here.
    pub data_home: PathBuf,
    /// `$XDG_CONFIG_HOME`, `~/.config` when unset: `mimeapps.list`, the
    /// desktops' own configuration and Rick's `path.sh` are here.
    pub config_home: PathBuf,
    /// `$XDG_DATA_DIRS`, where the desktop files of the other
    /// applications are looked up after `data_home`.
    pub data_dirs: Vec<PathBuf>,
    /// `$XDG_CURRENT_DESKTOP`.
    pub current_desktop: String,
    /// The line naming the distribution and the desktop.
    pub system: String,
    /// The running binary.
    pub exe: PathBuf,
    /// The directories of `$PATH`, where the desktop's tools are found.
    pub path: Vec<PathBuf>,
    /// `$DBUS_SESSION_BUS_ADDRESS`: the bus that is told to read its
    /// service files again after one was written. `None` without a bus,
    /// and in a test.
    pub session_bus: Option<String>,
}

impl LinuxEnv {
    /// The environment of this process.
    #[must_use]
    pub fn from_process() -> Self {
        let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
        let home = var("HOME").map_or_else(|| PathBuf::from("."), PathBuf::from);
        let data_home =
            var("XDG_DATA_HOME").map_or_else(|| home.join(".local/share"), PathBuf::from);
        let config_home =
            var("XDG_CONFIG_HOME").map_or_else(|| home.join(".config"), PathBuf::from);
        let data_dirs = var("XDG_DATA_DIRS").map_or_else(
            || {
                vec![
                    PathBuf::from("/usr/local/share"),
                    PathBuf::from("/usr/share"),
                ]
            },
            |dirs| std::env::split_paths(&dirs).collect(),
        );
        Self {
            home,
            data_home,
            config_home,
            data_dirs,
            current_desktop: std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default(),
            system: super::os_description(),
            exe: std::env::current_exe().unwrap_or_else(|_| PathBuf::from(BINARY)),
            path: var("PATH")
                .map(|path| std::env::split_paths(&path).collect())
                .unwrap_or_default(),
            session_bus: std::env::var("DBUS_SESSION_BUS_ADDRESS")
                .ok()
                .filter(|address| !address.is_empty()),
        }
    }

    /// An environment whose every directory is under `home`, for a test:
    /// `.local/share` and `.config` there, no system data directories, an
    /// empty `PATH`, the desktop and the binary as given.
    #[must_use]
    pub fn under(home: &Path, exe: &Path, current_desktop: &str) -> Self {
        Self {
            home: home.to_path_buf(),
            data_home: home.join(".local/share"),
            config_home: home.join(".config"),
            data_dirs: Vec::new(),
            current_desktop: current_desktop.to_owned(),
            system: if current_desktop.is_empty() {
                "Linux".to_owned()
            } else {
                format!("Linux · {current_desktop}")
            },
            exe: exe.to_path_buf(),
            path: Vec::new(),
            session_bus: None,
        }
    }

    /// A path with `~` for the home directory, as a [`Receipt`] lists it.
    #[must_use]
    pub fn tilde(&self, path: &Path) -> String {
        match path.strip_prefix(&self.home) {
            Ok(rest) => format!("~/{}", rest.to_string_lossy()),
            Err(_) => path.to_string_lossy().into_owned(),
        }
    }

    /// The path a receipt's `~` form names.
    #[must_use]
    pub fn expand(&self, text: &str) -> PathBuf {
        text.strip_prefix("~/")
            .map_or_else(|| PathBuf::from(text), |rest| self.home.join(rest))
    }

    /// Where Rick installs itself for the user: `~/.local/opt/rick`.
    #[must_use]
    pub fn install_dir(&self) -> PathBuf {
        self.home.join(".local/opt").join(APP_ID)
    }

    /// The version folder the running binary is in, when it runs from the
    /// standard location: `~/.local/opt/rick/<version>`.
    fn installed_version(&self) -> Option<PathBuf> {
        let exe = fs::canonicalize(&self.exe).ok()?;
        let root = fs::canonicalize(self.install_dir()).ok()?;
        let rest = exe.strip_prefix(&root).ok()?;
        let version = rest.components().next()?;
        // The binary is inside a version folder, not the folder itself.
        (rest.components().count() > 1).then(|| root.join(version))
    }

    /// The Rick the items point at: the running binary, through the
    /// `current` link when it runs from the standard location, so an
    /// update that flips the link takes every item with it.
    #[must_use]
    pub fn binary(&self) -> PathBuf {
        let Some(version) = self.installed_version() else {
            return self.exe.clone();
        };
        let current = self.install_dir().join("current");
        let through = fs::canonicalize(&self.exe)
            .ok()
            .and_then(|exe| exe.strip_prefix(&version).ok().map(|rel| current.join(rel)));
        match through {
            Some(path) if same_file(&path, &self.exe) => path,
            _ => self.exe.clone(),
        }
    }

    /// The executable a desktop `Exec` or a link names: itself when it
    /// has a directory, otherwise the first of that name on the `PATH`.
    fn program(&self, name: &str) -> Option<PathBuf> {
        if name.contains('/') {
            return Some(PathBuf::from(name));
        }
        self.path
            .iter()
            .map(|dir| dir.join(name))
            .find(|candidate| candidate.is_file())
    }

    /// Whether the program a desktop `Exec` line or a service file's
    /// `Exec` starts is `binary`.
    fn launches(&self, exec: &str, binary: &Path) -> bool {
        super::apps::split_exec(exec)
            .first()
            .and_then(|program| self.program(program))
            .is_some_and(|program| program == binary || same_file(&program, binary))
    }
}

/// Whether two paths name the same file, links followed.
fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// The value of `key` in `[section]` of an INI-like text (a desktop
/// entry, a D-Bus service file, `mimeapps.list`), the first one.
fn ini_value(text: &str, section: &str, key: &str) -> Option<String> {
    let mut inside = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            inside = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) == Some(section);
            continue;
        }
        if !inside || line.starts_with('#') {
            continue;
        }
        if let Some((name, value)) = line.split_once('=')
            && name.trim() == key
        {
            return Some(value.trim().to_owned());
        }
    }
    None
}

/// Whether `path` is in the way of an item: something is there that the
/// receipt does not list.
fn is_foreign(env: &LinuxEnv, path: &Path, receipt: &Receipt) -> bool {
    fs::symlink_metadata(path).is_ok() && !receipt.files.contains(&env.tilde(path))
}

/// Refuses a perform while files Rick did not write are in the item's way
/// and the request does not say to replace them.
fn refuse_foreign(check: &Check, request: &Request) -> Result<(), Error> {
    match check.foreign.first() {
        Some(path) if !request.replace => Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("{} exists and was not written by Rick", path.display()),
        ))),
        _ => Ok(()),
    }
}

/// Lists `path` in the receipt, once.
fn record(env: &LinuxEnv, receipt: &mut Receipt, path: &Path) {
    let text = env.tilde(path);
    if !receipt.files.contains(&text) {
        receipt.files.push(text);
    }
}

/// Writes `bytes` to `path` through a temporary file renamed over it,
/// creating the directories above it.
fn write_file(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let parent = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(Error::from_io)?;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".tmp-{}", std::process::id()));
    let tmp = PathBuf::from(tmp);
    fs::write(&tmp, bytes).map_err(Error::from_io)?;
    fs::rename(&tmp, path).map_err(|error| {
        let _ = fs::remove_file(&tmp);
        Error::from_io(error)
    })
}

/// Writes `bytes` to `path` unless the file holds them already, and lists
/// the path in the receipt when it was written: a file that was there
/// with the same contents is not Rick's to remove later.
fn write_recorded(
    env: &LinuxEnv,
    path: &Path,
    bytes: &[u8],
    receipt: &mut Receipt,
) -> Result<(), Error> {
    if fs::read(path).is_ok_and(|existing| existing == bytes) {
        return Ok(());
    }
    // A link or a directory under the name is replaced by the file.
    if fs::symlink_metadata(path).is_ok_and(|meta| meta.is_symlink()) {
        fs::remove_file(path).map_err(Error::from_io)?;
    }
    write_file(path, bytes)?;
    record(env, receipt, path);
    Ok(())
}

/// Removes the file or link at `path`; one that is gone is fine.
fn remove(path: &Path) -> Result<(), Error> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Error::from_io(error)),
    }
}

/// Runs a desktop tool when it is on the `PATH` of the environment, and
/// waits for it; a missing tool or one that fails is only logged, the
/// desktop picks the change up without it sooner or later.
fn run_tool(env: &LinuxEnv, name: &str, args: &[&std::ffi::OsStr]) {
    let Some(program) = env.program(name) else {
        return;
    };
    let status = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    match status {
        Ok(status) if status.success() => {}
        Ok(status) => tracing::debug!(%status, tool = name, "desktop tool failed"),
        Err(error) => tracing::debug!(%error, tool = name, "desktop tool not run"),
    }
}

/// The integration items of a Linux desktop, on one [`LinuxEnv`].
#[derive(Clone)]
pub struct LinuxIntegration {
    env: LinuxEnv,
    gsettings: Arc<dyn Gsettings>,
}

impl std::fmt::Debug for LinuxIntegration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LinuxIntegration")
            .field("env", &self.env)
            .finish_non_exhaustive()
    }
}

impl LinuxIntegration {
    /// The items on `env`, with `gsettings` for the two desktops that keep
    /// their favourites there.
    #[must_use]
    pub fn new(env: LinuxEnv, gsettings: Arc<dyn Gsettings>) -> Self {
        Self { env, gsettings }
    }

    /// The items on this process's environment, with the `gsettings`
    /// program.
    #[must_use]
    pub fn native() -> Self {
        let env = LinuxEnv::from_process();
        let gsettings = Arc::new(dock::Program::on(&env.path));
        Self::new(env, gsettings)
    }

    /// The environment the items are on.
    #[must_use]
    pub fn env(&self) -> &LinuxEnv {
        &self.env
    }

    /// The install location: set up when the running binary is inside a
    /// version folder of `~/.local/opt/rick`.
    fn check_install_location(&self) -> Check {
        let env = &self.env;
        if let Some(version) = env.installed_version() {
            return Check {
                actual: Actual::SetUp,
                place: Some(version),
                ..Check::default()
            };
        }
        Check {
            actual: Actual::Missing,
            place: Some(env.install_dir()),
            target: env.exe.parent().map(Path::to_path_buf),
            ..Check::default()
        }
    }
}

/// What the Linux items answer for *All users* until the installer's
/// helper for the system scope exists.
const NO_SYSTEM_SCOPE: &str = "setting Rick up for all users is not available yet";
/// What the platform answers for the item the installer plugin performs.
const INSTALLERS: &str = "the install location is set up by the installer plugin";

impl Integration for LinuxIntegration {
    fn integration_items(&self) -> Items {
        Items {
            system: self.env.system.clone(),
            desktop: Desktop::from_current_desktop(&self.env.current_desktop),
            items: Item::ALL.to_vec(),
        }
    }

    fn check_integration(&self, item: Item, request: &Request) -> Check {
        if request.scope == crate::integration::Scope::System {
            return Check::unsupported(crate::integration::Reason::AllUsers);
        }
        let env = &self.env;
        let binary = request.binary.clone().unwrap_or_else(|| env.binary());
        match item {
            Item::InstallLocation => self.check_install_location(),
            Item::LauncherEntry => launcher::check(env, &binary, &request.receipt),
            Item::DefaultFileManager => mime::check(env),
            Item::ShowInFolder => service::check(env, &binary, &request.receipt),
            Item::RickCommand => command::check(env, &binary, &request.receipt),
            Item::DockFavourite => dock::check(env, &*self.gsettings),
        }
    }

    fn perform_integration(&self, item: Item, request: &Request) -> Result<Receipt, Error> {
        if request.scope == crate::integration::Scope::System {
            return Err(Error::Unsupported(NO_SYSTEM_SCOPE));
        }
        let env = &self.env;
        let binary = request.binary.clone().unwrap_or_else(|| env.binary());
        match item {
            Item::InstallLocation => Err(Error::Unsupported(INSTALLERS)),
            Item::LauncherEntry => launcher::perform(env, &binary, request),
            Item::DefaultFileManager => mime::perform(env, request),
            Item::RickCommand => command::perform(env, &binary, request),
            Item::ShowInFolder => service::perform(env, &binary, request),
            Item::DockFavourite => dock::perform(env, &*self.gsettings, request),
        }
    }

    fn undo_integration(&self, item: Item, request: &Request) -> Result<(), Error> {
        let env = &self.env;
        match item {
            Item::InstallLocation => Err(Error::Unsupported(INSTALLERS)),
            Item::LauncherEntry => launcher::undo(env, &request.receipt),
            Item::DefaultFileManager => mime::undo(env, &request.receipt),
            Item::RickCommand => command::undo(env, &request.receipt),
            Item::ShowInFolder => service::undo(env, &request.receipt),
            Item::DockFavourite => dock::undo(env, &*self.gsettings, &request.receipt),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::integration::{Reason, Scope};

    /// A home of the test's own with a Rick unpacked beside it, as the
    /// release archive unpacks: `<dir>/home` and `<dir>/unpacked/rick`.
    pub(crate) struct Home {
        pub dir: tempfile::TempDir,
        pub env: LinuxEnv,
    }

    impl Home {
        pub(crate) fn new(desktop: &str) -> Self {
            let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
            let home = dir.path().join("home");
            let unpacked = dir.path().join("unpacked");
            fs::create_dir_all(&home).unwrap_or_else(|e| panic!("{e}"));
            fs::create_dir_all(&unpacked).unwrap_or_else(|e| panic!("{e}"));
            let exe = unpacked.join(BINARY);
            fs::write(&exe, b"#!/bin/sh\n").unwrap_or_else(|e| panic!("{e}"));
            let env = LinuxEnv::under(&home, &exe, desktop);
            Self { dir, env }
        }

        pub(crate) fn integration(&self) -> LinuxIntegration {
            LinuxIntegration::new(self.env.clone(), Arc::new(dock::tests::Settings::default()))
        }

        /// Moves the binary into the standard location, as the installer
        /// leaves it: a version folder and the `current` link.
        pub(crate) fn install(&mut self, version: &str) {
            let app = self.env.install_dir();
            let folder = app.join(version);
            fs::create_dir_all(&folder).unwrap_or_else(|e| panic!("{e}"));
            fs::copy(&self.env.exe, folder.join(BINARY)).unwrap_or_else(|e| panic!("{e}"));
            let current = app.join("current");
            let _ = fs::remove_file(&current);
            std::os::unix::fs::symlink(version, &current).unwrap_or_else(|e| panic!("{e}"));
            self.env.exe = folder.join(BINARY);
        }
    }

    pub(crate) fn write(path: &Path, text: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap_or_else(|e| panic!("{e}"));
        }
        fs::write(path, text).unwrap_or_else(|e| panic!("{e}"));
    }

    /// Every file under `dir` with its contents, to show a check changed
    /// nothing.
    pub(crate) fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut out = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir)
                .unwrap_or_else(|e| panic!("{e}"))
                .flatten()
            {
                let path = entry.path();
                let meta = fs::symlink_metadata(&path).unwrap_or_else(|e| panic!("{e}"));
                if meta.is_dir() {
                    out.push((path.clone(), Vec::new()));
                    stack.push(path);
                } else if meta.is_symlink() {
                    let target = fs::read_link(&path).unwrap_or_else(|e| panic!("{e}"));
                    out.push((path, target.to_string_lossy().into_owned().into_bytes()));
                } else {
                    let bytes = fs::read(&path).unwrap_or_else(|e| panic!("{e}"));
                    out.push((path, bytes));
                }
            }
        }
        out.sort();
        out
    }

    #[test]
    fn linux_lists_the_six_items_and_names_the_desktop() {
        let home = Home::new("ubuntu:GNOME");
        let items = home.integration().integration_items();
        assert_eq!(items.items, Item::ALL);
        assert_eq!(items.desktop, Some(Desktop::Gnome));
        assert_eq!(items.system, "Linux · ubuntu:GNOME");
        assert_eq!(
            Home::new("XFCE").integration().integration_items().desktop,
            None
        );
    }

    #[test]
    fn a_rick_unpacked_somewhere_is_outside_the_install_location() {
        let mut home = Home::new("GNOME");
        let check = home
            .integration()
            .check_integration(Item::InstallLocation, &Request::default());
        assert_eq!(check.actual, Actual::Missing);
        assert_eq!(check.place, Some(home.env.install_dir()));
        assert_eq!(check.target, Some(home.dir.path().join("unpacked")));
        // The items point at the binary where it is.
        assert_eq!(home.env.binary(), home.env.exe);

        home.install("0.3.0");
        let check = home
            .integration()
            .check_integration(Item::InstallLocation, &Request::default());
        assert_eq!(check.actual, Actual::SetUp);
        assert_eq!(
            check
                .place
                .as_deref()
                .map(|p| p.ends_with(".local/opt/rick/0.3.0")),
            Some(true),
            "{check:?}"
        );
        // And, installed, through the `current` link.
        assert_eq!(
            home.env.binary(),
            home.env.install_dir().join("current").join(BINARY)
        );
    }

    #[test]
    fn a_folder_named_like_the_install_location_is_not_a_version() {
        // The binary directly in ~/.local/opt/rick, no version folder.
        let mut home = Home::new("GNOME");
        let app = home.env.install_dir();
        fs::create_dir_all(&app).unwrap_or_else(|e| panic!("{e}"));
        fs::copy(&home.env.exe, app.join(BINARY)).unwrap_or_else(|e| panic!("{e}"));
        home.env.exe = app.join(BINARY);
        let check = home
            .integration()
            .check_integration(Item::InstallLocation, &Request::default());
        assert_eq!(check.actual, Actual::Missing);
    }

    #[test]
    fn checking_every_item_changes_nothing() {
        let home = Home::new("GNOME");
        let integration = home.integration();
        let before = snapshot(home.dir.path());
        for item in Item::ALL {
            let check = integration.check_integration(item, &Request::default());
            assert_ne!(check.actual, Actual::SetUp, "{item:?}");
        }
        assert_eq!(snapshot(home.dir.path()), before);
    }

    #[test]
    fn all_users_is_not_available_yet() {
        let home = Home::new("GNOME");
        let integration = home.integration();
        let system = Request {
            scope: Scope::System,
            ..Request::default()
        };
        for item in Item::ALL {
            assert_eq!(
                integration.check_integration(item, &system).actual,
                Actual::Unsupported(Reason::AllUsers),
                "{item:?}"
            );
            assert!(matches!(
                integration.perform_integration(item, &system),
                Err(Error::Unsupported(_))
            ));
        }
        // The install location is never the platform's to perform.
        assert!(matches!(
            integration.perform_integration(Item::InstallLocation, &Request::default()),
            Err(Error::Unsupported(_))
        ));
    }

    #[test]
    fn paths_are_recorded_with_a_tilde_and_read_back() {
        let home = Home::new("");
        let path = home.env.home.join(".local/bin/rick");
        assert_eq!(home.env.tilde(&path), "~/.local/bin/rick");
        assert_eq!(home.env.expand("~/.local/bin/rick"), path);
        assert_eq!(home.env.tilde(Path::new("/usr/bin/rick")), "/usr/bin/rick");
        assert_eq!(home.env.expand("/usr/bin/rick"), Path::new("/usr/bin/rick"));
        assert_eq!(
            ini_value("[A]\nx=1\n[B]\n# x=0\nx = 2\n", "B", "x").as_deref(),
            Some("2")
        );
        assert_eq!(ini_value("[A]\nx=1\n", "B", "x"), None);
    }
}
