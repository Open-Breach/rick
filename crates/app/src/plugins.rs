//! Installing and removing plugins in the per-user plugins folder
//! (SPEC.md *Extensions dialogue*).
//!
//! A plugin package, an `.rkp` file, is a zip archive of a plugin's
//! folder. [`install`] unpacks it into the plugins folder as it is: it
//! checks nothing and asks nothing, a plugin already there under the same
//! name is replaced, and whether the result is a plugin is found out by
//! the loader at the next start, as for a folder put there by hand. One
//! thing holds whatever the archive says: nothing is written outside the
//! plugins folder. [`remove`] deletes a plugin's folder. [`describe`]
//! reads what a folder's `plugin.toml` says, for the row a plugin
//! installed in this session gets before the host has seen it.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use ext::Points;
use serde::Deserialize;

use crate::{Plugin, PluginStatus};

/// The folder in the config directory whose subfolders are the user's
/// plugins.
pub const PLUGINS_DIR: &str = "plugins";

/// The file name extension of a plugin package.
pub const PACKAGE_EXTENSION: &str = "rkp";

/// What the file dialogue of *Install from file…* looks for.
pub const PACKAGE_FILTER: platform::DialogFilter = platform::DialogFilter {
    name: "Rick plugin package",
    extensions: &[PACKAGE_EXTENSION],
};

/// The manifest file in a plugin's folder.
const MANIFEST_FILE: &str = "plugin.toml";

/// Why a package was not installed or a plugin not removed.
#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    /// The package could not be read as a zip archive.
    #[error("{0} is not a plugin package: {1}")]
    Package(PathBuf, String),
    /// Neither the archive nor the file's name gives the folder a name.
    #[error("{0} names no plugin folder")]
    Unnamed(PathBuf),
    /// The folder is not one of the per-user plugins folder.
    #[error("{0} is not in the plugins folder")]
    Outside(PathBuf),
    /// Reading or writing failed.
    #[error("{0}: {1}")]
    Io(PathBuf, #[source] io::Error),
}

/// A path of an archive entry as the list of its plain names; `None` for
/// one that is absolute, climbs with `..` or is empty, which is never
/// written.
fn plain(path: &Path) -> Option<Vec<String>> {
    let mut names = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(name) => names.push(name.to_str()?.to_owned()),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!names.is_empty()).then_some(names)
}

/// Whether `name` can be a plugin folder's name: one plain name that does
/// not start with a dot.
fn is_folder_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && Path::new(name).components().count() == 1
        && matches!(
            Path::new(name).components().next(),
            Some(Component::Normal(_))
        )
}

/// Where a folder stands on its way in or out: beside the plugins folder,
/// on the same file system, so that nothing half done is ever taken for a
/// plugin and a folder changes hands with one rename.
fn staging(plugins: &Path) -> PathBuf {
    plugins.with_file_name(".plugins-staging")
}

/// An empty staging folder; what an earlier run left behind goes first.
fn fresh_staging(plugins: &Path) -> Result<PathBuf, PluginError> {
    let dir = staging(plugins);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).map_err(|error| PluginError::Io(dir.clone(), error))?;
    Ok(dir)
}

/// Unpacks the package into `plugins`, the per-user plugins folder, and
/// returns the plugin folder it made.
///
/// The folder is the one the archive holds when everything in it sits in
/// one top-level folder, and is otherwise named after the package file,
/// whose entries are then the folder's contents. A plugin folder of that
/// name is replaced. The archive is unpacked beside the plugins folder
/// first and moved in when it is whole, so a package that cannot be read
/// leaves the plugin that was there as it was. Symbolic links in the
/// archive are not written.
///
/// # Errors
///
/// [`PluginError::Package`] when the file is no zip archive,
/// [`PluginError::Unnamed`] when no folder name can be had, and the I/O
/// error of the first write that failed.
pub fn install(package: &Path, plugins: &Path) -> Result<PathBuf, PluginError> {
    let bad =
        |error: zip::result::ZipError| PluginError::Package(package.to_owned(), error.to_string());
    let file =
        fs::File::open(package).map_err(|error| PluginError::Io(package.to_owned(), error))?;
    let mut archive = zip::ZipArchive::new(io::BufReader::new(file)).map_err(bad)?;

    // What the archive holds, and whether one folder holds all of it.
    let mut entries = Vec::new();
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(bad)?;
        let Some(names) = entry.enclosed_name().as_deref().and_then(plain) else {
            tracing::warn!(entry = ?entry.name_raw(), "entry outside the plugin folder skipped");
            continue;
        };
        if entry.is_symlink() {
            tracing::warn!(entry = ?names, "symbolic link skipped");
            continue;
        }
        entries.push((index, names, entry.is_dir()));
    }
    let top = entries.first().map(|(_, names, _)| names[0].clone());
    let wrapped = top.as_ref().is_some_and(|top| {
        entries
            .iter()
            .all(|(_, names, dir)| names[0] == *top && (names.len() > 1 || *dir))
    });
    let name = if wrapped {
        top.unwrap_or_default()
    } else {
        package
            .file_stem()
            .map_or_default(|stem| stem.to_string_lossy().into_owned())
    };
    if !is_folder_name(&name) {
        return Err(PluginError::Unnamed(package.to_owned()));
    }

    let at = |path: &Path| {
        let path = path.to_owned();
        move |error| PluginError::Io(path, error)
    };
    let staging = fresh_staging(plugins)?;
    let part = staging.join(&name);
    let old = staging.join(format!("{name}.old"));
    let target = plugins.join(&name);
    let unpacked = (|| {
        fs::create_dir_all(&part).map_err(at(&part))?;
        for (index, names, dir) in &entries {
            let inside = &names[usize::from(wrapped)..];
            let path = inside
                .iter()
                .fold(part.clone(), |path, name| path.join(name));
            if *dir {
                fs::create_dir_all(&path).map_err(at(&path))?;
                continue;
            }
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(at(parent))?;
            }
            let mut entry = archive.by_index(*index).map_err(bad)?;
            let mut out = fs::File::create(&path).map_err(at(&path))?;
            io::copy(&mut entry, &mut out).map_err(at(&path))?;
        }
        fs::create_dir_all(plugins).map_err(at(plugins))?;
        // The plugin that was there steps aside, whole, and comes back if
        // the new one cannot take its place.
        let replaced = target.exists();
        if replaced {
            fs::rename(&target, &old).map_err(at(&target))?;
        }
        fs::rename(&part, &target).map_err(|error| {
            if replaced {
                let _ = fs::rename(&old, &target);
            }
            PluginError::Io(target.clone(), error)
        })
    })();
    let _ = fs::remove_dir_all(&staging);
    unpacked.map(|()| target)
}

/// Deletes the plugin folder `dir`, which must be a folder directly in
/// `plugins`, the per-user plugins folder. The folder leaves the plugins
/// folder whole, with one rename, before it is deleted: a folder that
/// cannot go stays as it was.
///
/// # Errors
///
/// [`PluginError::Outside`] for any other path, otherwise the I/O error.
pub fn remove(dir: &Path, plugins: &Path) -> Result<(), PluginError> {
    let name = dir
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| is_folder_name(name));
    let (Some(name), true) = (name, dir.parent() == Some(plugins)) else {
        return Err(PluginError::Outside(dir.to_owned()));
    };
    let staging = fresh_staging(plugins)?;
    let moved =
        fs::rename(dir, staging.join(name)).map_err(|error| PluginError::Io(dir.to_owned(), error));
    let _ = fs::remove_dir_all(&staging);
    moved
}

/// What the row of a plugin folder needs from its `plugin.toml`; every
/// key may be missing, since nothing was checked.
#[derive(Debug, Default, Deserialize)]
struct Described {
    id: Option<String>,
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    points: Vec<String>,
    #[serde(default)]
    previews: Vec<String>,
}

/// The row of the plugin folder `dir`, unpacked in this session: what its
/// `plugin.toml` says about it, or the folder's name alone when it has
/// none that can be read. The next start decides whether it loads.
#[must_use]
pub fn describe(dir: &Path) -> Plugin {
    let folder = dir
        .file_name()
        .map_or_default(|name| name.to_string_lossy().into_owned());
    let said: Described = fs::read_to_string(dir.join(MANIFEST_FILE))
        .ok()
        .and_then(|text| toml::from_str(&text).ok())
        .unwrap_or_default();
    Plugin {
        id: said.id.unwrap_or(folder),
        name: said.name,
        description: said.description,
        version: said.version,
        points: said
            .points
            .iter()
            .filter_map(|name| Points::named(name))
            .fold(Points::empty(), |all, point| all | point),
        previews: said.previews,
        status: PluginStatus::Installed,
        path: dir.to_owned(),
        built_in: false,
        removed: false,
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::*;

    /// Writes a zip archive of `files` (name, content; a name ending in
    /// `/` is a folder) to `path`.
    fn package(path: &Path, files: &[(&str, &str)]) {
        let file = fs::File::create(path).unwrap_or_else(|e| panic!("{e}"));
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        for (name, content) in files {
            if name.ends_with('/') {
                zip.add_directory(*name, options)
                    .unwrap_or_else(|e| panic!("{e}"));
            } else {
                zip.start_file(*name, options)
                    .unwrap_or_else(|e| panic!("{e}"));
                zip.write_all(content.as_bytes())
                    .unwrap_or_else(|e| panic!("{e}"));
            }
        }
        zip.finish().unwrap_or_else(|e| panic!("{e}"));
    }

    const MANIFEST: &str = "id = \"hello\"\nname = \"Hello\"\ndescription = \"Says hello.\"\n\
                            version = \"1.2.0\"\npoints = [\"action\", \"previewer\"]\n\
                            previews = [\"hi\"]\n";

    #[test]
    fn a_package_of_a_folder_is_unpacked_under_the_folder_s_name() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let plugins = dir.path().join("plugins");
        let rkp = dir.path().join("download.rkp");
        package(
            &rkp,
            &[
                ("hello/", ""),
                ("hello/plugin.toml", MANIFEST),
                ("hello/lib/libhello.so", "library"),
            ],
        );
        let made = install(&rkp, &plugins).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(made, plugins.join("hello"));
        assert_eq!(
            fs::read_to_string(made.join("lib/libhello.so")).unwrap_or_default(),
            "library"
        );
        let row = describe(&made);
        assert_eq!(
            (row.id.as_str(), row.name.as_str(), row.version.as_str()),
            ("hello", "Hello", "1.2.0")
        );
        assert_eq!(row.description, "Says hello.");
        assert_eq!(row.points, Points::ACTION | Points::PREVIEWER);
        assert_eq!(row.previews, ["hi"]);
        assert_eq!(row.status, PluginStatus::Installed);
        assert!(!row.built_in && !row.removed);
        assert_eq!(fs::read_dir(&plugins).map(Iterator::count).ok(), Some(1));
    }

    #[test]
    fn a_package_of_a_folder_s_contents_is_unpacked_under_the_package_s_name() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let plugins = dir.path().join("plugins");
        let rkp = dir.path().join("hello.rkp");
        package(&rkp, &[("plugin.toml", MANIFEST), ("libhello.so", "new")]);
        // A plugin already there under that name is replaced, whole.
        fs::create_dir_all(plugins.join("hello")).unwrap_or_else(|e| panic!("{e}"));
        fs::write(plugins.join("hello/old.txt"), "old").unwrap_or_else(|e| panic!("{e}"));
        let made = install(&rkp, &plugins).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(made, plugins.join("hello"));
        assert!(made.join("plugin.toml").is_file());
        assert!(!made.join("old.txt").exists(), "the old plugin is gone");
    }

    #[test]
    fn nothing_is_written_outside_the_plugins_folder() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let plugins = dir.path().join("config/plugins");
        let rkp = dir.path().join("sly.rkp");
        package(
            &rkp,
            &[
                ("plugin.toml", MANIFEST),
                ("../escaped.txt", "out"),
                ("../../escaped.txt", "out"),
                ("/tmp/rick-escaped.txt", "out"),
            ],
        );
        let made = install(&rkp, &plugins).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(made, plugins.join("sly"));
        assert!(made.join("plugin.toml").is_file());
        assert!(!plugins.join("escaped.txt").exists());
        assert!(!dir.path().join("config/escaped.txt").exists());
        assert!(!dir.path().join("escaped.txt").exists());
        // Only the plugin's folder is in the plugins folder.
        assert_eq!(fs::read_dir(&plugins).map(Iterator::count).ok(), Some(1));
    }

    #[test]
    fn a_file_that_is_no_archive_leaves_the_plugin_that_was_there() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let plugins = dir.path().join("plugins");
        fs::create_dir_all(plugins.join("hello")).unwrap_or_else(|e| panic!("{e}"));
        fs::write(plugins.join("hello/plugin.toml"), MANIFEST).unwrap_or_else(|e| panic!("{e}"));
        let rkp = dir.path().join("hello.rkp");
        fs::write(&rkp, "not a zip").unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(
            install(&rkp, &plugins),
            Err(PluginError::Package(..))
        ));
        assert!(plugins.join("hello/plugin.toml").is_file());
        assert_eq!(fs::read_dir(&plugins).map(Iterator::count).ok(), Some(1));
    }

    #[test]
    fn remove_deletes_a_plugin_folder_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let plugins = dir.path().join("plugins");
        let hello = plugins.join("hello");
        fs::create_dir_all(&hello).unwrap_or_else(|e| panic!("{e}"));
        fs::write(hello.join("plugin.toml"), MANIFEST).unwrap_or_else(|e| panic!("{e}"));
        let elsewhere = dir.path().join("elsewhere");
        fs::create_dir_all(&elsewhere).unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(
            remove(&elsewhere, &plugins),
            Err(PluginError::Outside(_))
        ));
        assert!(matches!(
            remove(&plugins, &plugins),
            Err(PluginError::Outside(_))
        ));
        assert!(elsewhere.is_dir() && plugins.is_dir());
        remove(&hello, &plugins).unwrap_or_else(|e| panic!("{e}"));
        assert!(!hello.exists() && plugins.is_dir());
    }

    #[test]
    fn a_folder_without_a_manifest_is_a_row_under_its_name() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let folder = dir.path().join("mystery");
        fs::create_dir_all(&folder).unwrap_or_else(|e| panic!("{e}"));
        let row = describe(&folder);
        assert_eq!(row.id, "mystery");
        assert!(row.name.is_empty() && row.points.is_empty());
    }
}
