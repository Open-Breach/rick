//! The settings file: `rick.toml` in the config directory.

use std::path::{Path, PathBuf};

use model::Settings;
use runtime::mpsc;
use tracing::warn;
use types::Error;

/// The file name under the config directory.
pub const SETTINGS_FILE: &str = "rick.toml";

/// Reads the settings file; a missing file means the defaults.
///
/// # Errors
///
/// The I/O error, or `Error::Io` with `InvalidData` when the TOML does not
/// parse.
pub fn load(path: &Path) -> Result<Settings, Error> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Settings::default());
        }
        Err(error) => return Err(Error::from_io(error)),
    };
    toml::from_str(&text).map_err(|error| {
        Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            error.to_string(),
        ))
    })
}

/// A writer that saves every settings snapshot sent to it, in order, off
/// the caller's thread: two changes in a row can never land reversed.
#[must_use]
pub fn saver(path: PathBuf) -> mpsc::UnboundedSender<Settings> {
    let (tx, mut rx) = mpsc::unbounded_channel::<Settings>();
    runtime::handle().spawn(async move {
        while let Some(mut settings) = rx.recv().await {
            // Only the latest of a burst needs writing.
            while let Ok(newer) = rx.try_recv() {
                settings = newer;
            }
            let path = path.clone();
            let written = runtime::handle()
                .spawn_blocking(move || save(&path, &settings))
                .await;
            match written {
                Ok(Ok(())) => {}
                Ok(Err(error)) => warn!(%error, "settings not saved"),
                Err(error) => warn!(%error, "settings writer stopped"),
            }
        }
    });
    tx
}

/// Writes the settings file, creating its directory, through a temporary
/// file renamed over it: the watcher, which reloads the file on any change
/// in the directory, then never reads a truncated file as the defaults and
/// writes the pins away.
///
/// # Errors
///
/// The I/O error.
pub fn save(path: &Path, settings: &Settings) -> Result<(), Error> {
    let text = toml::to_string_pretty(settings)
        .map_err(|error| Error::Io(std::io::Error::other(error.to_string())))?;
    crate::session::write_atomically(path, &text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::{Column, SortDir, Theme};

    #[test]
    fn round_trip_and_defaults() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let path = dir.path().join("sub").join(SETTINGS_FILE);
        assert_eq!(load(&path).ok(), Some(Settings::default()));

        let mut settings = Settings {
            show_hidden: true,
            sort_by: Column::Size,
            sort_dir: SortDir::Descending,
            theme: Theme::Dark,
            pinned: vec![types::Location::new(
                vfs_local::PROVIDER_ID,
                platform::paths::from_path(&dir.path().join("pinned here")),
            )],
            ..Settings::default()
        };
        settings.session.start = "Review".to_owned();
        settings.preview.text.wrap = true;
        settings.preview.text.line_numbers = false;
        settings.preview.pinned.insert("preview-image".into(), true);
        save(&path, &settings).unwrap_or_else(|e| panic!("{e}"));
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{e}"));
        assert!(text.contains("[preview.text]"), "{text}");
        assert!(text.contains("wrap = true"), "{text}");
        assert!(text.contains("line_numbers = false"), "{text}");
        assert!(text.contains("[preview.pinned]"), "{text}");
        assert!(text.contains("[session]"), "{text}");
        assert!(text.contains("start = \"Review\""), "{text}");
        assert_eq!(load(&path).ok(), Some(settings));
        let leftovers: Vec<_> = std::fs::read_dir(dir.path().join("sub"))
            .unwrap_or_else(|e| panic!("{e}"))
            .filter_map(Result::ok)
            .map(|e| e.file_name())
            .collect();
        assert_eq!(leftovers, [SETTINGS_FILE], "written in place, nothing left");
    }

    #[test]
    fn a_reader_never_sees_a_half_written_file() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let path = dir.path().join(SETTINGS_FILE);
        let settings = Settings {
            pinned: (0..40)
                .map(|i| types::Location::new(vfs_local::PROVIDER_ID, format!("/pinned/{i}")))
                .collect(),
            ..Settings::default()
        };
        save(&path, &settings).unwrap_or_else(|e| panic!("{e}"));
        let reader = {
            let path = path.clone();
            let expected = settings.clone();
            std::thread::spawn(move || {
                let mut reads = 0u32;
                let mut bad = 0u32;
                while reads < 2000 {
                    reads += 1;
                    if load(&path).ok() != Some(expected.clone()) {
                        bad += 1;
                    }
                }
                bad
            })
        };
        for _ in 0..200 {
            save(&path, &settings).unwrap_or_else(|e| panic!("{e}"));
        }
        let bad = reader.join().unwrap_or_else(|_| panic!("reader"));
        assert_eq!(bad, 0, "{bad} reads saw something else than the file");
    }

    #[test]
    fn partial_files_fill_in_defaults_and_bad_ones_fail() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let path = dir.path().join(SETTINGS_FILE);
        std::fs::write(&path, "font_size = 16\n").unwrap_or_else(|e| panic!("{e}"));
        let loaded = load(&path).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(loaded.font_size, 16);
        assert!(loaded.folders_first);

        std::fs::write(&path, "font_size = \n").unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(load(&path), Err(Error::Io(_))));
    }
}
