//! The platform tests run against: a private trash directory, recorded
//! calls and canned dialogue answers.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

use async_trait::async_trait;
use tempfile::TempDir;
use types::Error;

use crate::instance::Endpoint;
use crate::integration::{self, FakeIntegration, Integration};
use crate::perms::{
    FilePermissions, PermissionChange, Permissions, Principal, PrincipalKind, Principals,
};
use crate::traits::{
    Activation, Application, Chrome, Clipboard, ConfigDir, ControlsSide, DialogFilter, DragSource,
    DropTarget, Elevate, ElevatedOutput, ImageFormat, IncomingFiles, Monitor, Mount, MountWatch,
    MountsChanged, NativeDialogs, OpenWith, Point, Properties, PropertiesOutcome, RecentItem,
    RecentlyUsed, Secrets, SingleInstance, StandardFolder, StandardFolders, Terminal, Trash,
    TrashedItem, Volume, VolumeKind, Volumes, WindowChrome, WindowRef,
};

/// The applications the fake offers for every file, the default first.
pub const FAKE_APPLICATIONS: [(&str, &str); 2] = [
    ("Fake Editor", "fake-editor"),
    ("Fake Viewer", "fake-viewer"),
];

/// The standard folders the fake creates under its home, so the side
/// panel's default places have rows to show.
pub const FAKE_STANDARD_FOLDERS: [StandardFolder; 3] = [
    StandardFolder::Documents,
    StandardFolder::Downloads,
    StandardFolder::Pictures,
];

/// A platform that touches nothing outside its own temporary directory.
///
/// The trash really moves files, into a private directory that disappears
/// with the `Fake`; opening and revealing are recorded instead of launching
/// anything; dialogues answer what the test told them to; the volumes and
/// the recently-used list are what the test registered; mounting a disk
/// image adds a volume under the fake's own directory and is recorded,
/// with the formats it accepts settable per test, so the *Mount* action's
/// availability can be tried for every platform; the standard folders are
/// the directories under the fake's home; the secrets live in a map the
/// test can read back.
/// The listeners [`Volumes::watch_mounts`] was handed.
#[derive(Default)]
struct MountListeners(Mutex<Vec<Arc<MountsChanged>>>);

impl std::fmt::Debug for MountListeners {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MountListeners({})", lock(&self.0).len())
    }
}

#[derive(Debug)]
pub struct Fake {
    dir: TempDir,
    items: Mutex<Vec<TrashedItem>>,
    /// Trashed paths are numbered from here, never reused, so an item
    /// trashed after a restore does not land on another item's path.
    trashed_count: Mutex<u64>,
    opened: Mutex<Vec<PathBuf>>,
    opened_with: Mutex<Vec<(PathBuf, Application)>>,
    /// What `run_detached` was handed: the program, its arguments and its
    /// working directory. Nothing is started.
    ran: Mutex<Vec<(PathBuf, Vec<OsString>, PathBuf)>>,
    /// The files handed to `open_in_text_editor`, one list a call.
    edited: Mutex<Vec<Vec<PathBuf>>>,
    /// Whether an execute bit makes a program here; it does at first.
    execute_bit: Mutex<bool>,
    /// The tokens handed to [`Activation::activate_window`], oldest first;
    /// nothing is raised.
    activations: Mutex<Vec<Option<String>>>,
    terminals: Mutex<Vec<(PathBuf, Option<Vec<String>>)>>,
    /// The programs handed to [`Elevate::run_elevated`]; nothing runs
    /// unless `elevate_runs` says so.
    elevated: Mutex<Vec<(PathBuf, Vec<OsString>)>>,
    /// Whether [`Elevate::run_elevated`] runs the program, as the current
    /// user: for a test of the helper itself.
    elevate_runs: Mutex<bool>,
    revealed: Mutex<Vec<PathBuf>>,
    clipboard: Mutex<Option<String>>,
    /// Whether [`Permissions::permissions`] waits: a test holds the
    /// Properties dialogue's read back with it, as a slow volume would.
    permissions_held: (Mutex<bool>, Condvar),
    dragged: Mutex<Vec<Vec<String>>>,
    /// A drag handed to `start_drag` runs until the test ends it.
    drag_running: Mutex<bool>,
    /// What `drop_position` answers.
    drop_position: Mutex<Option<Point>>,
    /// What the next `incoming_files` answers.
    incoming: Mutex<IncomingFiles>,
    answers: Mutex<Answers>,
    volumes: Mutex<Vec<Volume>>,
    ejected: Mutex<Vec<PathBuf>>,
    /// Whether a disk utility is installed, for *Format…*.
    format_tool: Mutex<bool>,
    /// The monitors `monitors` reports; none at first.
    monitors: Mutex<Vec<Monitor>>,
    /// What `watch_mounts` was handed, called by `mounts_changed`.
    mount_listeners: Arc<MountListeners>,
    /// The mount points handed to `open_format_tool`.
    formatting: Mutex<Vec<PathBuf>>,
    /// The disk-image formats [`Mount::mountable`] answers; the current
    /// platform's unless a test says otherwise.
    mountable: Mutex<&'static [ImageFormat]>,
    /// The image files handed to [`Mount::mount_image`], oldest first.
    mounted: Mutex<Vec<PathBuf>>,
    recent: Mutex<Vec<RecentItem>>,
    /// `list` answers `Unsupported`, as the macOS trash does.
    unlistable: Mutex<bool>,
    /// What [`Motion::reduce_motion`](crate::traits::Motion) answers; off
    /// unless a test turns it on.
    reduce_motion: Mutex<bool>,
    /// The keychain: secrets by key.
    secrets: Mutex<std::collections::BTreeMap<String, String>>,
    /// Who answers [`Integration`]: a [`FakeIntegration`] without items
    /// unless a test puts another in its place.
    integration: Mutex<IntegrationSlot>,
}

/// The fake's [`Integration`], whatever implements it.
struct IntegrationSlot(Arc<dyn Integration>);

impl std::fmt::Debug for IntegrationSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Integration")
    }
}

#[derive(Debug, Default)]
struct Answers {
    folder: Option<PathBuf>,
    files: Vec<PathBuf>,
    save: Option<PathBuf>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // The guarded values are plain vectors; a panic elsewhere leaves them usable.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Fake {
    /// A fake with an empty trash and no dialogue answers.
    ///
    /// # Errors
    ///
    /// Fails when the private temporary directory cannot be created.
    pub fn new() -> Result<Self, Error> {
        let dir = tempfile::Builder::new()
            .prefix("rick-fake-platform-")
            .tempdir()
            .map_err(Error::from_io)?;
        fs::create_dir_all(dir.path().join("trash")).map_err(Error::from_io)?;
        let home = dir.path().join("home");
        fs::create_dir_all(&home).map_err(Error::from_io)?;
        for folder in FAKE_STANDARD_FOLDERS {
            fs::create_dir_all(home.join(folder.title())).map_err(Error::from_io)?;
        }
        Ok(Self {
            dir,
            items: Mutex::new(Vec::new()),
            trashed_count: Mutex::new(0),
            opened: Mutex::new(Vec::new()),
            opened_with: Mutex::new(Vec::new()),
            ran: Mutex::new(Vec::new()),
            edited: Mutex::new(Vec::new()),
            execute_bit: Mutex::new(true),
            activations: Mutex::new(Vec::new()),
            terminals: Mutex::new(Vec::new()),
            elevated: Mutex::new(Vec::new()),
            elevate_runs: Mutex::new(false),
            revealed: Mutex::new(Vec::new()),
            clipboard: Mutex::new(None),
            permissions_held: (Mutex::new(false), Condvar::new()),
            dragged: Mutex::new(Vec::new()),
            drag_running: Mutex::new(false),
            drop_position: Mutex::new(None),
            incoming: Mutex::new(IncomingFiles::default()),
            answers: Mutex::new(Answers::default()),
            volumes: Mutex::new(Vec::new()),
            ejected: Mutex::new(Vec::new()),
            format_tool: Mutex::new(true),
            monitors: Mutex::new(Vec::new()),
            mount_listeners: Arc::default(),
            formatting: Mutex::new(Vec::new()),
            mountable: Mutex::new(crate::NATIVE_MOUNTABLE),
            mounted: Mutex::new(Vec::new()),
            recent: Mutex::new(Vec::new()),
            unlistable: Mutex::new(false),
            reduce_motion: Mutex::new(false),
            secrets: Mutex::new(std::collections::BTreeMap::new()),
            integration: Mutex::new(IntegrationSlot(Arc::new(FakeIntegration::new(
                &crate::os_description(),
                None,
            )))),
        })
    }

    /// Puts `integration` in place of the fake's own, which lists no
    /// items: a [`FakeIntegration`] the test scripted, or the real items
    /// of a platform on a temporary home.
    pub fn set_integration(&self, integration: Arc<dyn Integration>) {
        lock(&self.integration).0 = integration;
    }

    fn integration(&self) -> Arc<dyn Integration> {
        Arc::clone(&lock(&self.integration).0)
    }

    /// Paths and applications handed to [`OpenWith::open_with`], oldest
    /// first.
    #[must_use]
    pub fn opened_with(&self) -> Vec<(PathBuf, Application)> {
        lock(&self.opened_with).clone()
    }

    /// The programs handed to [`OpenWith::run_detached`], oldest first,
    /// each with its arguments and its working directory.
    #[must_use]
    pub fn ran(&self) -> Vec<(PathBuf, Vec<OsString>, PathBuf)> {
        lock(&self.ran).clone()
    }

    /// The files handed to [`OpenWith::open_in_text_editor`], one list a
    /// call, oldest first.
    #[must_use]
    pub fn edited(&self) -> Vec<Vec<PathBuf>> {
        lock(&self.edited).clone()
    }

    /// Whether an execute bit makes a program from now on, as it does on
    /// Linux and macOS and does not on Windows.
    pub fn set_execute_bit(&self, on: bool) {
        *lock(&self.execute_bit) = on;
    }

    /// The folders and commands handed to [`Terminal::open_terminal`],
    /// oldest first.
    #[must_use]
    pub fn terminals(&self) -> Vec<(PathBuf, Option<Vec<String>>)> {
        lock(&self.terminals).clone()
    }

    /// The programs and arguments handed to [`Elevate::run_elevated`],
    /// oldest first; the fake runs none of them unless
    /// [`Fake::set_elevate_runs`] said so.
    #[must_use]
    pub fn elevated(&self) -> Vec<(PathBuf, Vec<OsString>)> {
        lock(&self.elevated).clone()
    }

    /// Makes [`Elevate::run_elevated`] run the program, as the current user
    /// and with its output captured, so a test can exercise the privileged
    /// helper without privileges.
    pub fn set_elevate_runs(&self, runs: bool) {
        *lock(&self.elevate_runs) = runs;
    }

    /// Paths handed to [`OpenWith::open`], oldest first.
    #[must_use]
    pub fn opened(&self) -> Vec<PathBuf> {
        lock(&self.opened).clone()
    }

    /// The tokens handed to [`Activation::activate_window`], oldest
    /// first.
    #[must_use]
    pub fn activations(&self) -> Vec<Option<String>> {
        lock(&self.activations).clone()
    }

    /// Paths handed to [`OpenWith::reveal`], oldest first.
    #[must_use]
    pub fn revealed(&self) -> Vec<PathBuf> {
        lock(&self.revealed).clone()
    }

    /// The text last put on the clipboard, if any.
    #[must_use]
    pub fn clipboard_text(&self) -> Option<String> {
        lock(&self.clipboard).clone()
    }

    /// The file URIs of every drag handed to [`DragSource::start_drag`],
    /// oldest first.
    #[must_use]
    pub fn dragged(&self) -> Vec<Vec<String>> {
        lock(&self.dragged).clone()
    }

    /// Ends the drag `start_drag` began, as the desktop does when the button
    /// comes up: `drag_in_progress` answers `false` from now on.
    pub fn end_drag(&self) {
        *lock(&self.drag_running) = false;
    }

    /// What [`DropTarget::drop_position`] answers from now on: where the
    /// cursor of a drag from outside is, or `None` for a platform that
    /// cannot tell.
    pub fn answer_drop_position(&self, position: Option<Point>) {
        *lock(&self.drop_position) = position;
    }

    /// Files the platform receives itself, as the Wayland data device does:
    /// `hovered` is answered by every `incoming_files` until changed,
    /// `dropped` by the next one only.
    pub fn receive_files(&self, hovered: Vec<PathBuf>, dropped: Vec<PathBuf>) {
        let mut incoming = lock(&self.incoming);
        incoming.hovered = hovered;
        incoming.dropped.extend(dropped);
    }

    /// What `pick_folder` answers from now on.
    pub fn answer_folder(&self, folder: Option<PathBuf>) {
        lock(&self.answers).folder = folder;
    }

    /// Makes [`Permissions::permissions`] wait from now on, until
    /// [`release_permissions`](Self::release_permissions): the read of an
    /// entry's properties takes as long as the test wants.
    pub fn hold_permissions(&self) {
        *lock(&self.permissions_held.0) = true;
    }

    /// Lets the reads held by [`hold_permissions`](Self::hold_permissions)
    /// go on.
    pub fn release_permissions(&self) {
        *lock(&self.permissions_held.0) = false;
        self.permissions_held.1.notify_all();
    }

    /// What `pick_files` answers from now on.
    pub fn answer_files(&self, files: Vec<PathBuf>) {
        lock(&self.answers).files = files;
    }

    /// What `save_file` answers from now on.
    pub fn answer_save(&self, path: Option<PathBuf>) {
        lock(&self.answers).save = path;
    }

    /// Makes [`Trash::list`] answer `Error::Unsupported` from now on, as
    /// the macOS trash does, so the own-record path can be tested on every
    /// OS. Trashing and restoring keep working.
    pub fn set_trash_unlistable(&self, unlistable: bool) {
        *lock(&self.unlistable) = unlistable;
    }

    /// Adds a volume [`Volumes::volumes`] lists from now on; the test's
    /// stand-in for a mount.
    pub fn add_volume(&self, volume: Volume) {
        lock(&self.volumes).push(volume);
    }

    /// Forgets a volume by its mount point; the stand-in for an unmount.
    pub fn remove_volume(&self, mount_point: &Path) {
        lock(&self.volumes).retain(|v| v.mount_point != mount_point);
    }

    /// The mount points handed to [`Volumes::eject`], oldest first.
    #[must_use]
    pub fn ejected(&self) -> Vec<PathBuf> {
        lock(&self.ejected).clone()
    }

    /// The monitors that are connected from now on, in logical pixels.
    pub fn set_monitors(&self, monitors: Vec<Monitor>) {
        *lock(&self.monitors) = monitors;
    }

    /// The stand-in for the system's mount or unmount event: every running
    /// [`Volumes::watch_mounts`] is told.
    pub fn mounts_changed(&self) {
        let listeners = lock(&self.mount_listeners.0).clone();
        for changed in listeners {
            changed();
        }
    }

    /// Whether a disk utility is installed from now on; one is at first.
    pub fn set_format_tool(&self, installed: bool) {
        *lock(&self.format_tool) = installed;
    }

    /// The mount points the system's format tool was opened on, oldest
    /// first.
    #[must_use]
    pub fn format_opened(&self) -> Vec<PathBuf> {
        lock(&self.formatting).clone()
    }

    /// What [`Mount::mountable`] answers from now on: another platform's
    /// list, so a test can try the *Mount* action as Windows or macOS
    /// would show it.
    pub fn set_mountable(&self, formats: &'static [ImageFormat]) {
        *lock(&self.mountable) = formats;
    }

    /// The image files handed to [`Mount::mount_image`], oldest first.
    #[must_use]
    pub fn mounted(&self) -> Vec<PathBuf> {
        lock(&self.mounted).clone()
    }

    /// What [`RecentlyUsed::recently_used`] answers from now on; the
    /// test's stand-in for the system's list.
    pub fn set_recent(&self, items: Vec<RecentItem>) {
        *lock(&self.recent) = items;
    }

    /// Every secret in the fake keychain, by key: what a test checks never
    /// reached a file.
    #[must_use]
    pub fn secrets(&self) -> std::collections::BTreeMap<String, String> {
        lock(&self.secrets).clone()
    }
}

impl Secrets for Fake {
    fn store_secret(&self, key: &str, secret: &str) -> Result<(), Error> {
        lock(&self.secrets).insert(key.to_owned(), secret.to_owned());
        Ok(())
    }

    fn get_secret(&self, key: &str) -> Result<Option<String>, Error> {
        Ok(lock(&self.secrets).get(key).cloned())
    }

    fn remove_secret(&self, key: &str) -> Result<(), Error> {
        lock(&self.secrets).remove(key);
        Ok(())
    }
}

impl Volumes for Fake {
    fn volumes(&self) -> Result<Vec<Volume>, Error> {
        Ok(lock(&self.volumes).clone())
    }

    /// Records the eject and forgets the volume, as the unmount would.
    fn eject(&self, volume: &Volume) -> Result<(), Error> {
        if !volume.is_removable() {
            return Err(Error::Unsupported("only removable media can be ejected"));
        }
        let mut volumes = lock(&self.volumes);
        if !volumes.iter().any(|v| v.mount_point == volume.mount_point) {
            return Err(Error::NotFound);
        }
        volumes.retain(|v| v.mount_point != volume.mount_point);
        lock(&self.ejected).push(volume.mount_point.clone());
        Ok(())
    }

    fn has_format_tool(&self) -> bool {
        *lock(&self.format_tool)
    }

    /// Records the volume; nothing is formatted.
    fn open_format_tool(&self, volume: &Volume) -> Result<(), Error> {
        if !self.has_format_tool() {
            return Err(Error::Unsupported("no disk utility is installed"));
        }
        lock(&self.formatting).push(volume.mount_point.clone());
        Ok(())
    }

    /// Keeps `changed` for [`Fake::mounts_changed`] until the watch is
    /// dropped.
    fn watch_mounts(&self, changed: MountsChanged) -> Option<MountWatch> {
        /// Takes its listener off the list when the watch ends.
        struct Listening {
            listeners: Arc<MountListeners>,
            changed: Arc<MountsChanged>,
        }
        impl Drop for Listening {
            fn drop(&mut self) {
                lock(&self.listeners.0).retain(|other| !Arc::ptr_eq(other, &self.changed));
            }
        }
        let changed = Arc::new(changed);
        lock(&self.mount_listeners.0).push(Arc::clone(&changed));
        Some(MountWatch::new(Listening {
            listeners: Arc::clone(&self.mount_listeners),
            changed,
        }))
    }
}

impl Mount for Fake {
    fn mountable(&self) -> &'static [ImageFormat] {
        *lock(&self.mountable)
    }

    /// Mounts nothing: records the call and adds a volume at a directory
    /// of the fake's own, named after the image, which [`Volumes::eject`]
    /// takes away again like any removable volume.
    fn mount_image(&self, path: &Path) -> Result<Volume, Error> {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let format = ImageFormat::for_name(&name)
            .filter(|f| self.mountable().contains(f))
            .ok_or(Error::Unsupported(
                "this platform does not mount that kind of disk image",
            ))?;
        if !path.is_file() {
            return Err(Error::NotFound);
        }
        let stem = Path::new(&name)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or(name);
        let mount_point = self.dir.path().join("mounts").join(&stem);
        fs::create_dir_all(&mount_point).map_err(Error::from_io)?;
        let volume = Volume {
            name: stem,
            mount_point,
            kind: if format == ImageFormat::Iso {
                VolumeKind::Optical
            } else {
                VolumeKind::Removable
            },
            fs_type: "fake".to_owned(),
            device: None,
            total: None,
            free: None,
            read_only: format == ImageFormat::Iso,
        };
        lock(&self.mounted).push(path.to_path_buf());
        lock(&self.volumes).push(volume.clone());
        Ok(volume)
    }
}

impl StandardFolders for Fake {
    /// The home, or the directory of the folder's title under it when it
    /// exists.
    fn standard_folder(&self, folder: StandardFolder) -> Option<PathBuf> {
        let path = match folder {
            StandardFolder::Home => self.home_dir(),
            other => self.home_dir().join(other.title()),
        };
        path.is_dir().then_some(path)
    }
}

impl RecentlyUsed for Fake {
    fn recently_used(&self) -> Result<Vec<RecentItem>, Error> {
        let mut items = lock(&self.recent).clone();
        items.sort_by_key(|item| std::cmp::Reverse(item.used_at));
        Ok(items)
    }
}

impl Trash for Fake {
    fn trash(&self, path: &Path) -> Result<TrashedItem, Error> {
        let original = std::path::absolute(path).map_err(Error::from_io)?;
        fs::symlink_metadata(&original).map_err(Error::from_io)?;
        let mut items = lock(&self.items);
        let number = {
            let mut count = lock(&self.trashed_count);
            *count += 1;
            *count
        };
        let trashed = self.dir.path().join("trash").join(number.to_string());
        fs::rename(&original, &trashed).map_err(Error::from_io)?;
        let item = TrashedItem {
            original,
            trashed,
            deleted_at: SystemTime::now(),
        };
        items.push(item.clone());
        Ok(item)
    }

    fn restore(&self, item: &TrashedItem) -> Result<(), Error> {
        let mut items = lock(&self.items);
        let index = items
            .iter()
            .position(|i| i.trashed == item.trashed)
            .ok_or(Error::NotFound)?;
        if item.original.symlink_metadata().is_ok() {
            return Err(Error::Io(std::io::Error::from(
                std::io::ErrorKind::AlreadyExists,
            )));
        }
        fs::rename(&item.trashed, &item.original).map_err(Error::from_io)?;
        items.remove(index);
        Ok(())
    }

    fn list(&self) -> Result<Vec<TrashedItem>, Error> {
        if *lock(&self.unlistable) {
            return Err(Error::Unsupported(
                "this fake keeps its original paths to itself, like macOS",
            ));
        }
        Ok(lock(&self.items).clone())
    }

    fn remove(&self, item: &TrashedItem) -> Result<(), Error> {
        let mut items = lock(&self.items);
        let index = items
            .iter()
            .position(|i| i.trashed == item.trashed)
            .ok_or(Error::NotFound)?;
        remove_path(&item.trashed)?;
        items.remove(index);
        Ok(())
    }

    fn empty(&self) -> Result<(), Error> {
        let mut items = lock(&self.items);
        for item in items.drain(..) {
            remove_path(&item.trashed)?;
        }
        Ok(())
    }
}

/// Deletes a file or a whole directory.
fn remove_path(path: &Path) -> Result<(), Error> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(Error::from_io(e)),
    };
    if meta.is_dir() {
        fs::remove_dir_all(path).map_err(Error::from_io)
    } else {
        fs::remove_file(path).map_err(Error::from_io)
    }
}

impl OpenWith for Fake {
    fn open(&self, path: &Path) -> Result<(), Error> {
        lock(&self.opened).push(path.to_path_buf());
        Ok(())
    }

    fn reveal(&self, path: &Path) -> Result<(), Error> {
        lock(&self.revealed).push(path.to_path_buf());
        Ok(())
    }

    /// [`FAKE_APPLICATIONS`] for every entry that exists.
    fn applications(&self, path: &Path) -> Result<Vec<Application>, Error> {
        fs::symlink_metadata(path).map_err(Error::from_io)?;
        Ok(FAKE_APPLICATIONS
            .iter()
            .map(|(name, id)| Application {
                name: (*name).to_owned(),
                id: (*id).to_owned(),
            })
            .collect())
    }

    fn open_with(&self, path: &Path, app: &Application) -> Result<(), Error> {
        lock(&self.opened_with).push((path.to_path_buf(), app.clone()));
        Ok(())
    }

    fn has_execute_bit(&self) -> bool {
        *lock(&self.execute_bit)
    }

    /// Records the call; nothing is started.
    fn run_detached(&self, program: &Path, args: &[OsString], dir: &Path) -> Result<(), Error> {
        lock(&self.ran).push((program.to_path_buf(), args.to_vec(), dir.to_path_buf()));
        Ok(())
    }

    /// Records the files; nothing is opened.
    fn open_in_text_editor(&self, paths: &[PathBuf]) -> Result<(), Error> {
        lock(&self.edited).push(paths.to_vec());
        Ok(())
    }
}

impl Terminal for Fake {
    fn open_terminal(&self, dir: &Path, command: Option<&[String]>) -> Result<(), Error> {
        lock(&self.terminals).push((dir.to_path_buf(), command.map(<[String]>::to_vec)));
        Ok(())
    }
}

impl Elevate for Fake {
    /// Records the call and reports success without running anything, or
    /// runs the program as the current user when [`Fake::set_elevate_runs`]
    /// said so.
    fn run_elevated(&self, program: &Path, args: &[OsString]) -> Result<ElevatedOutput, Error> {
        lock(&self.elevated).push((program.to_path_buf(), args.to_vec()));
        if !*lock(&self.elevate_runs) {
            return Ok(ElevatedOutput {
                code: Some(0),
                ..ElevatedOutput::default()
            });
        }
        let output = std::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(Error::from_io)?;
        Ok(ElevatedOutput {
            code: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}

impl Permissions for Fake {
    /// The real entry's permissions: the fake's temporary directory is a
    /// real one, and the acceptance tests change modes on it.
    fn permissions(&self, path: &Path) -> Result<FilePermissions, Error> {
        let mut held = lock(&self.permissions_held.0);
        while *held {
            held = self
                .permissions_held
                .1
                .wait(held)
                .unwrap_or_else(PoisonError::into_inner);
        }
        drop(held);
        crate::Native.permissions(path)
    }

    fn set_permissions(&self, path: &Path, change: &PermissionChange) -> Result<(), Error> {
        crate::Native.set_permissions(path, change)
    }

    fn principals(&self) -> Result<Principals, Error> {
        crate::Native.principals()
    }

    fn principal(&self, name: &str, kind: PrincipalKind) -> Result<Principal, Error> {
        crate::Native.principal(name, kind)
    }
}

impl Properties for Fake {
    fn show(&self, path: &Path) -> Result<PropertiesOutcome, Error> {
        let meta = fs::symlink_metadata(path).map_err(Error::from_io)?;
        Ok(PropertiesOutcome::Rows(vec![
            ("Name".to_owned(), path.display().to_string()),
            ("Size".to_owned(), meta.len().to_string()),
        ]))
    }
}

impl Clipboard for Fake {
    fn set_text(&self, text: &str) -> Result<(), Error> {
        *lock(&self.clipboard) = Some(text.to_owned());
        Ok(())
    }

    fn text(&self) -> Result<Option<String>, Error> {
        Ok(lock(&self.clipboard).clone())
    }
}

impl DragSource for Fake {
    fn attach(&self, _window: &WindowRef, _wake: Arc<dyn Fn() + Send + Sync>) {}

    fn start_drag(&self, _window: &WindowRef, paths: &[PathBuf]) -> Result<(), Error> {
        lock(&self.dragged).push(paths.iter().map(|p| crate::paths::file_uri(p)).collect());
        *lock(&self.drag_running) = true;
        Ok(())
    }

    fn drag_in_progress(&self) -> bool {
        *lock(&self.drag_running)
    }
}

impl DropTarget for Fake {
    fn incoming_files(&self) -> IncomingFiles {
        let mut incoming = lock(&self.incoming);
        IncomingFiles {
            hovering: !incoming.hovered.is_empty(),
            hovered: incoming.hovered.clone(),
            dropped: std::mem::take(&mut incoming.dropped),
        }
    }

    fn drop_position(&self, _window: &WindowRef) -> Option<Point> {
        *lock(&self.drop_position)
    }
}

impl ConfigDir for Fake {
    fn config_dir(&self) -> PathBuf {
        self.dir.path().join("config")
    }

    fn home_dir(&self) -> PathBuf {
        self.dir.path().join("home")
    }
}

impl Activation for Fake {
    fn activate_window(&self, _window: &WindowRef, token: Option<&str>) -> Result<(), Error> {
        lock(&self.activations).push(token.map(str::to_owned));
        Ok(())
    }

    fn allow_activation(&self) {}
}

impl Integration for Fake {
    fn integration_items(&self) -> integration::Items {
        self.integration().integration_items()
    }

    fn check_integration(
        &self,
        item: integration::Item,
        request: &integration::Request,
    ) -> integration::Check {
        self.integration().check_integration(item, request)
    }

    fn perform_integration(
        &self,
        item: integration::Item,
        request: &integration::Request,
    ) -> Result<integration::Receipt, Error> {
        self.integration().perform_integration(item, request)
    }

    fn undo_integration(
        &self,
        item: integration::Item,
        request: &integration::Request,
    ) -> Result<(), Error> {
        self.integration().undo_integration(item, request)
    }
}

impl SingleInstance for Fake {
    /// An endpoint inside the fake's own directory, whatever the config
    /// directory: a test's instance is nobody else's.
    fn instance_endpoint(&self, _config_dir: &Path) -> Endpoint {
        Endpoint::in_dir(self.dir.path())
    }
}

impl crate::traits::SystemFonts for Fake {
    /// The fonts of the machine the tests run on, so a screenshot shows
    /// what a user there sees; nothing where the machine names none.
    fn system_fonts(&self) -> crate::traits::Fonts {
        crate::fonts::system()
    }
}

impl Fake {
    /// Makes the fake report the system's reduce-motion setting as `on`.
    pub fn set_reduce_motion(&self, on: bool) {
        *self
            .reduce_motion
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = on;
    }
}

impl crate::traits::Motion for Fake {
    fn reduce_motion(&self) -> bool {
        *self
            .reduce_motion
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

impl Chrome for Fake {
    fn window_chrome(&self) -> WindowChrome {
        WindowChrome {
            controls: ControlsSide::Right,
            native_controls: false,
            native_controls_width: 0.0,
        }
    }

    /// What [`Fake::set_monitors`] was given, whatever the scale.
    fn monitors(&self, _pixels_per_point: f32) -> Vec<Monitor> {
        lock(&self.monitors).clone()
    }
}

#[async_trait]
impl NativeDialogs for Fake {
    async fn pick_folder(&self, _title: &str) -> Option<PathBuf> {
        lock(&self.answers).folder.clone()
    }

    async fn pick_files(&self, _title: &str, _filters: &[DialogFilter]) -> Vec<PathBuf> {
        lock(&self.answers).files.clone()
    }

    async fn save_file(&self, _title: &str, _suggested: &str) -> Option<PathBuf> {
        lock(&self.answers).save.clone()
    }
}

/// A window of no backend, enough for the fake, which never looks at it,
/// and for the native platforms' tests, which refuse it.
#[cfg(test)]
pub(crate) fn test_window() -> WindowRef {
    use raw_window_handle::{RawDisplayHandle, RawWindowHandle, WebDisplayHandle, WebWindowHandle};
    WindowRef {
        window: RawWindowHandle::Web(WebWindowHandle::new(1)),
        display: RawDisplayHandle::Web(WebDisplayHandle::new()),
        scale: 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trash_and_restore_round_trip() {
        let fake = Fake::new().unwrap_or_else(|e| panic!("{e}"));
        let work = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let file = work.path().join("doomed.txt");
        fs::write(&file, b"bye").unwrap_or_else(|e| panic!("{e}"));

        let item = fake.trash(&file).unwrap_or_else(|e| panic!("{e}"));
        assert!(!file.exists());
        assert!(item.trashed.exists());
        assert_eq!(fake.list().map(|l| l.len()).ok(), Some(1));

        fake.restore(&item).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(fs::read(&file).ok(), Some(b"bye".to_vec()));
        assert_eq!(fake.list().map(|l| l.len()).ok(), Some(0));
        assert!(matches!(fake.restore(&item), Err(Error::NotFound)));

        // Remove one for good, empty the rest.
        let gone = fake.trash(&file).unwrap_or_else(|e| panic!("{e}"));
        let other = work.path().join("other");
        fs::create_dir(&other).unwrap_or_else(|e| panic!("{e}"));
        fs::write(other.join("x"), b"x").unwrap_or_else(|e| panic!("{e}"));
        let trashed_dir = fake.trash(&other).unwrap_or_else(|e| panic!("{e}"));
        fake.remove(&gone).unwrap_or_else(|e| panic!("{e}"));
        assert!(!gone.trashed.exists());
        assert!(matches!(fake.remove(&gone), Err(Error::NotFound)));
        assert_eq!(fake.list().map(|l| l.len()).ok(), Some(1));
        fake.empty().unwrap_or_else(|e| panic!("{e}"));
        assert!(!trashed_dir.trashed.exists());
        assert_eq!(fake.list().map(|l| l.len()).ok(), Some(0));
        assert!(!file.exists());
        assert!(matches!(
            fake.trash(&work.path().join("nope")),
            Err(Error::NotFound)
        ));
    }

    #[test]
    fn secrets_are_kept_in_memory_by_key() {
        let fake = Fake::new().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(fake.get_secret("s3/work/secret_key").ok(), Some(None));
        fake.store_secret("s3/work/secret_key", "hunter2")
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            fake.get_secret("s3/work/secret_key")
                .ok()
                .flatten()
                .as_deref(),
            Some("hunter2")
        );
        assert_eq!(fake.secrets().len(), 1);
        fake.remove_secret("s3/work/secret_key")
            .unwrap_or_else(|e| panic!("{e}"));
        fake.remove_secret("s3/work/secret_key")
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(fake.get_secret("s3/work/secret_key").ok(), Some(None));
        assert!(fake.secrets().is_empty());
    }

    #[test]
    fn restore_refuses_an_occupied_path() {
        let fake = Fake::new().unwrap_or_else(|e| panic!("{e}"));
        let work = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let file = work.path().join("twice.txt");
        fs::write(&file, b"1").unwrap_or_else(|e| panic!("{e}"));
        let item = fake.trash(&file).unwrap_or_else(|e| panic!("{e}"));
        fs::write(&file, b"2").unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(
            fake.restore(&item),
            Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::AlreadyExists
        ));
    }

    #[test]
    fn drags_and_drops_are_recorded_and_answered() {
        let fake = Fake::new().unwrap_or_else(|e| panic!("{e}"));
        fake.attach(&test_window(), Arc::new(|| {}));
        fake.start_drag(&test_window(), &[PathBuf::from("/a b")])
            .unwrap_or_else(|e| panic!("{e}"));
        let dragged = fake.dragged();
        assert_eq!(dragged.len(), 1);
        assert!(dragged[0][0].ends_with("/a%20b"), "{dragged:?}");
        assert!(fake.drag_in_progress());
        fake.end_drag();
        assert!(!fake.drag_in_progress());
        assert_eq!(fake.drop_position(&test_window()), None);
        fake.answer_drop_position(Some(Point { x: 3.0, y: 4.0 }));
        assert_eq!(
            fake.drop_position(&test_window()),
            Some(Point { x: 3.0, y: 4.0 })
        );
        assert_eq!(fake.incoming_files(), IncomingFiles::default());
        fake.receive_files(vec![PathBuf::from("/h")], vec![PathBuf::from("/d")]);
        assert_eq!(
            fake.incoming_files(),
            IncomingFiles {
                hovering: true,
                hovered: vec![PathBuf::from("/h")],
                dropped: vec![PathBuf::from("/d")],
            }
        );
        assert_eq!(
            fake.incoming_files(),
            IncomingFiles {
                hovering: true,
                hovered: vec![PathBuf::from("/h")],
                dropped: Vec::new(),
            },
            "a drop is reported once"
        );
    }

    #[test]
    fn calls_and_answers_are_recorded() {
        let fake = Fake::new().unwrap_or_else(|e| panic!("{e}"));
        fake.open(Path::new("/a")).unwrap_or_else(|e| panic!("{e}"));
        fake.reveal(Path::new("/b"))
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(fake.opened(), [PathBuf::from("/a")]);
        assert_eq!(fake.revealed(), [PathBuf::from("/b")]);
        fake.answer_folder(Some(PathBuf::from("/picked")));
        let picked = runtime::block_on(fake.pick_folder("Choose"));
        assert_eq!(picked, Some(PathBuf::from("/picked")));
        assert!(fake.config_dir().starts_with(fake.dir.path()));
        fake.set_text("/a\n/b").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(fake.clipboard_text().as_deref(), Some("/a\n/b"));
        assert!(matches!(
            fake.show(fake.dir.path()),
            Ok(PropertiesOutcome::Rows(rows)) if rows[0].0 == "Name"
        ));
        assert_eq!(fake.volumes().map(|v| v.len()).ok(), Some(0));
        let usb = Volume {
            name: "USB".into(),
            mount_point: PathBuf::from("/media/usb"),
            kind: crate::VolumeKind::Removable,
            fs_type: "vfat".into(),
            device: None,
            total: Some(10),
            free: Some(5),
            read_only: false,
        };
        assert_eq!(usb.used(), Some(5));
        assert!(usb.utilisation().is_some_and(|u| (u - 0.5).abs() < 1e-6));
        fake.add_volume(usb.clone());
        assert_eq!(
            fake.volumes()
                .ok()
                .and_then(|v| v.first().map(|v| v.name.clone())),
            Some("USB".to_owned())
        );
        fake.eject(&usb).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(fake.ejected(), [PathBuf::from("/media/usb")]);
        assert_eq!(fake.volumes().map(|v| v.len()).ok(), Some(0));
        assert!(matches!(fake.eject(&usb), Err(Error::NotFound)));
        fake.add_volume(usb.clone());
        fake.remove_volume(Path::new("/media/usb"));
        assert_eq!(fake.volumes().map(|v| v.len()).ok(), Some(0));
        // Mounting an image adds a volume that Eject takes away again.
        assert_eq!(fake.mountable(), crate::NATIVE_MOUNTABLE);
        fake.set_mountable(&[ImageFormat::Iso, ImageFormat::Vhdx]);
        assert_eq!(fake.mountable(), &[ImageFormat::Iso, ImageFormat::Vhdx]);
        let image = fake.dir.path().join("disc.iso");
        fs::write(&image, b"iso").unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(
            fake.mount_image(&fake.dir.path().join("disk.qcow2")),
            Err(Error::Unsupported(_))
        ));
        assert!(matches!(
            fake.mount_image(&fake.dir.path().join("missing.iso")),
            Err(Error::NotFound)
        ));
        let mounted = fake.mount_image(&image).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(mounted.name, "disc");
        assert_eq!(mounted.kind, VolumeKind::Optical);
        assert!(mounted.mount_point.is_dir());
        assert_eq!(fake.mounted(), std::slice::from_ref(&image));
        assert_eq!(fake.volumes().map(|v| v.len()).ok(), Some(1));
        fake.eject(&mounted).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(fake.volumes().map(|v| v.len()).ok(), Some(0));
        assert_eq!(ImageFormat::for_name("A.VHDX"), Some(ImageFormat::Vhdx));
        assert_eq!(ImageFormat::for_name("a.qcow"), Some(ImageFormat::Qcow2));
        assert_eq!(ImageFormat::for_name(".iso"), None);
        assert_eq!(ImageFormat::for_name("notes.txt"), None);
        assert_eq!(ImageFormat::Img.name(), "raw image");
        assert_eq!(ImageFormat::ALL.len(), 8);
        assert_eq!(
            fake.standard_folder(StandardFolder::Home),
            Some(fake.home_dir())
        );
        assert_eq!(
            fake.standard_folder(StandardFolder::Documents),
            Some(fake.home_dir().join("Documents"))
        );
        assert_eq!(fake.standard_folder(StandardFolder::Music), None);
        assert!(fake.recently_used().is_ok_and(|r| r.is_empty()));
        fake.set_recent(vec![
            RecentItem {
                path: PathBuf::from("/old"),
                used_at: SystemTime::UNIX_EPOCH,
            },
            RecentItem {
                path: PathBuf::from("/new"),
                used_at: SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(5),
            },
        ]);
        assert_eq!(
            fake.recently_used()
                .ok()
                .and_then(|r| r.first().map(|i| i.path.clone())),
            Some(PathBuf::from("/new"))
        );
    }
}
