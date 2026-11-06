//! The platform tests run against: a private trash directory, recorded
//! calls and canned dialogue answers.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

use async_trait::async_trait;
use tempfile::TempDir;
use types::Error;

use crate::traits::{
    Chrome, Clipboard, ConfigDir, ControlsSide, DialogFilter, NativeDialogs, OpenWith, Properties,
    PropertiesOutcome, Trash, TrashedItem, WindowChrome,
};

/// A platform that touches nothing outside its own temporary directory.
///
/// The trash really moves files, into a private directory that disappears
/// with the `Fake`; opening and revealing are recorded instead of launching
/// anything; dialogues answer what the test told them to.
#[derive(Debug)]
pub struct Fake {
    dir: TempDir,
    items: Mutex<Vec<TrashedItem>>,
    opened: Mutex<Vec<PathBuf>>,
    revealed: Mutex<Vec<PathBuf>>,
    clipboard: Mutex<Option<String>>,
    answers: Mutex<Answers>,
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
        fs::create_dir_all(dir.path().join("home")).map_err(Error::from_io)?;
        Ok(Self {
            dir,
            items: Mutex::new(Vec::new()),
            opened: Mutex::new(Vec::new()),
            revealed: Mutex::new(Vec::new()),
            clipboard: Mutex::new(None),
            answers: Mutex::new(Answers::default()),
        })
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
}

impl Trash for Fake {
    fn trash(&self, path: &Path) -> Result<TrashedItem, Error> {
        let original = std::path::absolute(path).map_err(Error::from_io)?;
        fs::symlink_metadata(&original).map_err(Error::from_io)?;
        let mut items = lock(&self.items);
        let trashed = self
            .dir
            .path()
            .join("trash")
            .join(format!("{}", items.len()));
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
        Ok(lock(&self.items).clone())
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
        assert!(matches!(
            fake.trash(&work.path().join("nope")),
            Err(Error::NotFound)
        ));
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
        assert!(matches!(
            fake.show(fake.dir.path()),
            Ok(PropertiesOutcome::Rows(rows)) if rows[0].0 == "Name"
        ));
    }
}
