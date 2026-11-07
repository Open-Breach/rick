//! The Places list's own file: `places.toml` in the config directory.
//!
//! The first start seeds the list from the operating system's standard
//! folders in the spec's order — Home, Documents, Pictures, Music, Video,
//! Downloads, Favorites, Desktop, Recent, Trash — leaving out a folder the
//! system does not have, and writes the file; from then on the file is the
//! list, edited through the side panel.

use std::path::Path;

use model::Place;
use platform::{StandardFolder, StandardFolders};
use serde::{Deserialize, Serialize};
use types::{Error, Location};
use vfs_rick::Virtual;

/// The file name under the config directory.
pub const PLACES_FILE: &str = "places.toml";

/// The whole file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
struct File {
    place: Vec<Place>,
}

/// The Phosphor icon of a standard folder's default place.
fn folder_icon(folder: StandardFolder) -> &'static str {
    match folder {
        StandardFolder::Home => "HOUSE",
        StandardFolder::Documents => "FILE_TEXT",
        StandardFolder::Pictures => "IMAGE",
        StandardFolder::Music => "MUSIC_NOTES",
        StandardFolder::Video => "FILM_STRIP",
        StandardFolder::Downloads => "DOWNLOAD_SIMPLE",
        StandardFolder::Desktop => "DESKTOP",
    }
}

/// The Phosphor icon of a virtual location's default place.
#[must_use]
pub fn virtual_icon(which: Virtual) -> &'static str {
    match which {
        Virtual::Favorites => "STAR",
        Virtual::Recent => "CLOCK_COUNTER_CLOCKWISE",
        Virtual::Trash => "TRASH",
        Virtual::Machine => "DESKTOP_TOWER",
        Virtual::Network => "NETWORK",
        Virtual::Cloud => "CLOUD",
    }
}

/// The spec's default list for this machine: the standard folders that
/// exist, with Favorites after Downloads and Recent and Trash at the end.
#[must_use]
pub fn defaults(folders: &dyn StandardFolders) -> Vec<Place> {
    let folder = |which: StandardFolder| {
        folders.standard_folder(which).map(|path| {
            Place::new(
                which.title(),
                Location::new(vfs_local::PROVIDER_ID, platform::paths::from_path(&path)),
            )
            .with_icon(folder_icon(which))
        })
    };
    let virtual_place =
        |which: Virtual| Place::new(which.title(), which.location()).with_icon(virtual_icon(which));
    [
        folder(StandardFolder::Home),
        folder(StandardFolder::Documents),
        folder(StandardFolder::Pictures),
        folder(StandardFolder::Music),
        folder(StandardFolder::Video),
        folder(StandardFolder::Downloads),
        Some(virtual_place(Virtual::Favorites)),
        folder(StandardFolder::Desktop),
        Some(virtual_place(Virtual::Recent)),
        Some(virtual_place(Virtual::Trash)),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// Reads the file; `Ok(None)` when there is none yet.
///
/// # Errors
///
/// The I/O error, or `InvalidData` when the file does not parse.
pub fn load(path: &Path) -> Result<Option<Vec<Place>>, Error> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(Error::from_io(error)),
    };
    let file: File = toml::from_str(&text).map_err(|error| {
        Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            error.to_string(),
        ))
    })?;
    Ok(Some(file.place))
}

/// Writes the list through a temporary file and a rename.
///
/// # Errors
///
/// The I/O error.
pub fn save(path: &Path, places: &[Place]) -> Result<(), Error> {
    let file = File {
        place: places.to_vec(),
    };
    let text = toml::to_string_pretty(&file)
        .map_err(|error| Error::Io(std::io::Error::other(error.to_string())))?;
    crate::session::write_atomically(path, &text)
}

#[cfg(test)]
mod tests {
    use platform::Fake;

    use super::*;

    #[test]
    fn the_defaults_follow_the_spec_order_and_skip_missing_folders() {
        let fake = Fake::new().unwrap_or_else(|e| panic!("{e}"));
        let places = defaults(&fake);
        let titles: Vec<&str> = places.iter().map(|p| p.title.as_str()).collect();
        // The fake has Documents, Downloads and Pictures under its home.
        assert_eq!(
            titles,
            [
                "Home",
                "Documents",
                "Pictures",
                "Downloads",
                "Favorites",
                "Recent",
                "Trash"
            ]
        );
        assert_eq!(places[0].icon, "HOUSE");
        assert_eq!(places[4].location, Virtual::Favorites.location());
        assert_eq!(places[6].icon, "TRASH");
    }

    #[test]
    fn the_file_round_trips_and_a_missing_one_is_none() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let path = dir.path().join("cfg").join(PLACES_FILE);
        assert!(load(&path).is_ok_and(|p| p.is_none()));
        let places = vec![
            Place::new("Work", Location::new(vfs_local::PROVIDER_ID, "/srv/work")),
            Place::new("Trash", Virtual::Trash.location()).with_icon("TRASH"),
        ];
        save(&path, &places).unwrap_or_else(|e| panic!("{e}"));
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(text.contains("[[place]]"), "{text}");
        assert!(text.contains("location = \"rick:trash\""), "{text}");
        assert_eq!(load(&path).ok().flatten(), Some(places));
        std::fs::write(&path, "place = 3").unwrap_or_else(|e| panic!("{e}"));
        assert!(load(&path).is_err());
    }
}
