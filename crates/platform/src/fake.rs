//! The platform tests run against: a private trash directory, recorded
//! calls and canned dialogue answers.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

use async_trait::async_trait;
use tempfile::TempDir;
use types::Error;

use crate::perms::{
    FilePermissions, PermissionChange, Permissions, Principal, PrincipalKind, Principals,
};
use crate::traits::{
    Application, Chrome, Clipboard, ConfigDir, ControlsSide, DialogFilter, DragSource, Elevate,
    ElevatedOutput, NativeDialogs, OpenWith, Properties, PropertiesOutcome, RecentItem,
    RecentlyUsed, Secrets, StandardFolder, StandardFolders, Terminal, Trash, TrashedItem, Volume,
    Volumes, WindowChrome,
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
/// the recently-used list are what the test registered; the standard
/// folders are the directories under the fake's home; the secrets live in
/// a map the test can read back.
#[derive(Debug)]
pub struct Fake {
    dir: TempDir,
    items: Mutex<Vec<TrashedItem>>,
    /// Trashed paths are numbered from here, never reused, so an item
    /// trashed after a restore does not land on another item's path.
    trashed_count: Mutex<u64>,
    opened: Mutex<Vec<PathBuf>>,
    opened_with: Mutex<Vec<(PathBuf, Application)>>,
    terminals: Mutex<Vec<(PathBuf, Option<Vec<String>>)>>,
    /// The programs handed to [`Elevate::run_elevated`]; nothing runs.
    elevated: Mutex<Vec<(PathBuf, Vec<OsString>)>>,
    revealed: Mutex<Vec<PathBuf>>,
    clipboard: Mutex<Option<String>>,
    dragged: Mutex<Vec<Vec<String>>>,
    answers: Mutex<Answers>,
    volumes: Mutex<Vec<Volume>>,
    ejected: Mutex<Vec<PathBuf>>,
    recent: Mutex<Vec<RecentItem>>,
    /// `list` answers `Unsupported`, as the macOS trash does.
    unlistable: Mutex<bool>,
    /// The keychain: secrets by key.
    secrets: Mutex<std::collections::BTreeMap<String, String>>,
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
            terminals: Mutex::new(Vec::new()),
            elevated: Mutex::new(Vec::new()),
            revealed: Mutex::new(Vec::new()),
            clipboard: Mutex::new(None),
            dragged: Mutex::new(Vec::new()),
            answers: Mutex::new(Answers::default()),
            volumes: Mutex::new(Vec::new()),
            ejected: Mutex::new(Vec::new()),
            recent: Mutex::new(Vec::new()),
            unlistable: Mutex::new(false),
            secrets: Mutex::new(std::collections::BTreeMap::new()),
        })
    }

    /// Paths and applications handed to [`OpenWith::open_with`], oldest
    /// first.
    #[must_use]
    pub fn opened_with(&self) -> Vec<(PathBuf, Application)> {
        lock(&self.opened_with).clone()
    }

    /// The folders and commands handed to [`Terminal::open_terminal`],
    /// oldest first.
    #[must_use]
    pub fn terminals(&self) -> Vec<(PathBuf, Option<Vec<String>>)> {
        lock(&self.terminals).clone()
    }

    /// The programs and arguments handed to [`Elevate::run_elevated`],
    /// oldest first; the fake runs none of them.
    #[must_use]
    pub fn elevated(&self) -> Vec<(PathBuf, Vec<OsString>)> {
        lock(&self.elevated).clone()
    }

    /// Paths handed to [`OpenWith::open`], oldest first.
    #[must_use]
    pub fn opened(&self) -> Vec<PathBuf> {
        lock(&self.opened).clone()
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

    /// What `pick_folder` answers from now on.
    pub fn answer_folder(&self, folder: Option<PathBuf>) {
        lock(&self.answers).folder = folder;
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
}

impl Terminal for Fake {
    fn open_terminal(&self, dir: &Path, command: Option<&[String]>) -> Result<(), Error> {
        lock(&self.terminals).push((dir.to_path_buf(), command.map(<[String]>::to_vec)));
        Ok(())
    }
}

impl Elevate for Fake {
    /// Records the call and reports success without running anything.
    fn run_elevated(&self, program: &Path, args: &[OsString]) -> Result<ElevatedOutput, Error> {
        lock(&self.elevated).push((program.to_path_buf(), args.to_vec()));
        Ok(ElevatedOutput {
            code: Some(0),
            ..ElevatedOutput::default()
        })
    }
}

impl Permissions for Fake {
    /// The real entry's permissions: the fake's temporary directory is a
    /// real one, and the acceptance tests change modes on it.
    fn permissions(&self, path: &Path) -> Result<FilePermissions, Error> {
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
}

impl DragSource for Fake {
    fn start_drag(&self, paths: &[PathBuf]) -> Result<(), Error> {
        lock(&self.dragged).push(paths.iter().map(|p| crate::paths::file_uri(p)).collect());
        Ok(())
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

impl Chrome for Fake {
    fn window_chrome(&self) -> WindowChrome {
        WindowChrome {
            controls: ControlsSide::Right,
            native_controls: false,
            native_controls_width: 0.0,
        }
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
        fake.start_drag(&[PathBuf::from("/a b")])
            .unwrap_or_else(|e| panic!("{e}"));
        let dragged = fake.dragged();
        assert_eq!(dragged.len(), 1);
        assert!(dragged[0][0].ends_with("/a%20b"), "{dragged:?}");
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
