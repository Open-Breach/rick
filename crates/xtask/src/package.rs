//! `cargo xtask package`: the plugin packages of a bundle
//! (specs/plugin-distribution.md §4).
//!
//! A separate plugin is one whose crate says so in its `Cargo.toml`, under
//! `[package.metadata.rick]`: `separate = true`, or `separate = ["macos",
//! "windows"]` for a plugin that is separate on some platforms only. For
//! each of them that the bundle holds, [`write()`] makes one package,
//! `<id>-<version>-<platform>.rkp`: a zip archive of the plugin's folder
//! `<id>/` exactly as the bundle lays it out under `plugins/`, which is
//! what *Install from file…* reads. Beside the packages it writes
//! `plugins-<platform>.json`, the manifest the registry's publisher reads
//! (`docs/spec.md` 7.4 in Open-Breach/trebuchet), with `meta.points` and
//! `meta.triggers` from each `plugin.toml`.
//!
//! The same bundle gives the same bytes: the entries of an archive are
//! written in the order of their names, with one fixed timestamp and one
//! fixed mode, and the manifest's keys and entries have a fixed order.

use std::collections::BTreeSet;
use std::fs::File;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};

use crate::Result;

/// The name of the application in the manifest.
const APP: &str = "rick";
/// The manifest file of a plugin's folder.
const MANIFEST_FILE: &str = "plugin.toml";
/// The mode every entry of a package is written with. A plugin's folder
/// holds libraries and data, which need no more; a fixed one keeps the
/// archive the same wherever the bundle was made.
const ENTRY_MODE: u32 = 0o644;

/// The release's name for the platform a build of `os` and `arch` (as
/// `std::env::consts` spell them) is for: `linux-x64`, `linux-arm64`,
/// `macos-arm64`, `windows-x64`, the names of the release archives.
#[must_use]
pub fn platform_name(os: &str, arch: &str) -> String {
    let arch = match arch {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    };
    format!("{os}-{arch}")
}

/// Whether a crate whose `[package.metadata.rick]` holds `separate` as
/// `mark` is a separate plugin on `os`: `true` everywhere, a list on the
/// platforms it names, and not at all when the crate says nothing.
///
/// # Errors
///
/// A `separate` that is neither a boolean nor a list of platform names.
pub fn is_separate(mark: &serde_json::Value, os: &str) -> Result<bool> {
    match mark {
        serde_json::Value::Null => Ok(false),
        serde_json::Value::Bool(separate) => Ok(*separate),
        serde_json::Value::Array(platforms) => {
            let mut here = false;
            for platform in platforms {
                match platform.as_str() {
                    Some(name @ ("linux" | "macos" | "windows")) => here |= name == os,
                    _ => return Err(format!("`separate` names {platform}, not a platform").into()),
                }
            }
            Ok(here)
        }
        other => Err(format!("`separate` is {other}, neither true, false nor a list").into()),
    }
}

/// One package as the manifest lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    /// The plugin's id, also the folder the archive holds.
    pub id: String,
    /// The file's name in the output directory.
    pub file: String,
    /// Its size in bytes.
    pub size: u64,
}

/// What a `plugin.toml` tells the task.
struct Plugin {
    id: String,
    name: String,
    description: String,
    version: String,
    api_version: i64,
    rick: String,
    /// The library's crate, as its file name spells it: `ext_git`.
    stem: String,
    points: Vec<String>,
    previews: Vec<String>,
    markers: Vec<String>,
    /// Section and title of each kind of connection.
    connections: Vec<(String, String)>,
}

/// The text of `table[key]`.
fn text(table: &toml::Table, key: &str, file: &Path) -> Result<String> {
    table
        .get(key)
        .and_then(toml::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("{}: no `{key}`", file.display()).into())
}

/// The texts of the list `table[key]`; empty when the key is absent.
fn texts(table: &toml::Table, key: &str) -> Vec<String> {
    table
        .get(key)
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str().map(str::to_owned))
        .collect()
}

/// Reads the manifest of the plugin folder `dir` for a bundle of `os`.
fn read_plugin(dir: &Path, os: &str) -> Result<Plugin> {
    let file = dir.join(MANIFEST_FILE);
    let source = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    let table: toml::Table = source
        .parse()
        .map_err(|e| format!("{}: {e}", file.display()))?;
    let library = table
        .get("library")
        .and_then(|library| library.get(os))
        .and_then(toml::Value::as_str)
        .ok_or_else(|| format!("{}: no library named for {os}", file.display()))?;
    let stem = library
        .strip_prefix(std::env::consts::DLL_PREFIX)
        .unwrap_or(library);
    let stem = stem
        .strip_suffix(std::env::consts::DLL_SUFFIX)
        .unwrap_or(stem);
    let connections = table
        .get("connections")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|connection| {
            let field = |key| connection.get(key).and_then(toml::Value::as_str);
            Some((field("section")?.to_owned(), field("title")?.to_owned()))
        })
        .collect();
    Ok(Plugin {
        id: text(&table, "id", &file)?,
        name: text(&table, "name", &file)?,
        description: table
            .get("description")
            .and_then(toml::Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        version: text(&table, "version", &file)?,
        api_version: table
            .get("api_version")
            .and_then(toml::Value::as_integer)
            .ok_or_else(|| format!("{}: no `api_version`", file.display()))?,
        rick: text(&table, "rick", &file)?,
        stem: stem.to_owned(),
        points: texts(&table, "points"),
        previews: texts(&table, "previews"),
        markers: texts(&table, "markers"),
        connections,
    })
}

/// The files under `dir`, as their path below it with `/` between the
/// names, in the order of those paths. A symbolic link is refused: an
/// archive without it would not be the folder the bundle has.
fn files(dir: &Path) -> Result<Vec<(String, PathBuf)>> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, PathBuf)>) -> Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name();
            let name = name
                .to_str()
                .ok_or_else(|| format!("{}: not a UTF-8 name", path.display()))?;
            let inside = format!("{prefix}{name}");
            let kind = std::fs::symlink_metadata(&path)?.file_type();
            if kind.is_symlink() {
                return Err(
                    format!("{}: a symbolic link in a plugin folder", path.display()).into(),
                );
            } else if kind.is_dir() {
                walk(&path, &format!("{inside}/"), out)?;
            } else {
                out.push((inside, path));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(dir, "", &mut out)?;
    out.sort();
    Ok(out)
}

/// Writes the folder `dir` of the plugin `id` as the archive `target`, and
/// returns the archive's size and SHA-256.
fn pack(dir: &Path, id: &str, target: &Path) -> Result<(u64, String)> {
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::DEFAULT)
        .unix_permissions(ENTRY_MODE);
    let mut archive = zip::ZipWriter::new(File::create(target)?);
    for (inside, path) in files(dir)? {
        let large = std::fs::metadata(&path)?.len() >= u64::from(u32::MAX);
        archive.start_file(format!("{id}/{inside}"), options.large_file(large))?;
        std::io::copy(&mut File::open(&path)?, &mut archive)?;
    }
    archive.finish()?.flush()?;

    let mut hash = Sha256::new();
    let mut size = 0_u64;
    let mut file = File::open(target)?;
    let mut buffer = vec![0_u8; 1 << 16];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
        size += read as u64;
    }
    Ok((size, hex(&hash.finalize())))
}

/// `bytes` in lower-case hexadecimal.
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut text, byte| {
            // Writing into a String cannot fail.
            let _ = write!(text, "{byte:02x}");
            text
        })
}

/// The plugin's entry of the manifest, in the key order of the publisher's
/// contract. The triggers hold the lists that are not empty.
fn entry(plugin: &Plugin, file: &str, size: u64, sha256: &str) -> serde_json::Value {
    let mut triggers = serde_json::Map::new();
    if !plugin.previews.is_empty() {
        triggers.insert("previews".to_owned(), plugin.previews.clone().into());
    }
    if !plugin.markers.is_empty() {
        triggers.insert("markers".to_owned(), plugin.markers.clone().into());
    }
    if !plugin.connections.is_empty() {
        let connections: Vec<serde_json::Value> = plugin
            .connections
            .iter()
            .map(|(section, title)| serde_json::json!({ "section": section, "title": title }))
            .collect();
        triggers.insert("connections".to_owned(), connections.into());
    }
    serde_json::json!({
        "id": plugin.id,
        "name": plugin.name,
        "description": plugin.description,
        "version": plugin.version,
        "api_version": plugin.api_version,
        "meta": { "points": plugin.points, "triggers": triggers },
        "file": file,
        "size": size,
        "sha256": sha256,
    })
}

/// Writes into `out` (created) one package per plugin folder of
/// `plugins_dir` whose library is one of `separate` (crate names as a
/// library's file name spells them, `ext_git`), and the manifest
/// `plugins-<platform>.json` beside them. `os` is the bundle's operating
/// system and `rick` the version of Rick it must have been built for.
/// Returns the packages in the manifest's order, by id.
///
/// # Errors
///
/// A plugin folder whose manifest cannot be read, names another id than
/// its folder or was built for another Rick than `rick`; a symbolic link
/// in a plugin's folder; any I/O error.
pub fn write(
    plugins_dir: &Path,
    out: &Path,
    platform: &str,
    os: &str,
    rick: &str,
    separate: &BTreeSet<String>,
) -> Result<Vec<Package>> {
    std::fs::create_dir_all(out)?;
    let mut folders: Vec<PathBuf> = std::fs::read_dir(plugins_dir)
        .map_err(|e| format!("{}: {e}", plugins_dir.display()))?
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    folders.sort();

    let mut packages = Vec::new();
    let mut entries = Vec::new();
    for folder in folders {
        let plugin = read_plugin(&folder, os)?;
        if !separate.contains(&plugin.stem) {
            continue;
        }
        if folder.file_name().and_then(|name| name.to_str()) != Some(plugin.id.as_str()) {
            return Err(format!("{} holds the plugin `{}`", folder.display(), plugin.id).into());
        }
        if plugin.rick != rick {
            return Err(format!(
                "{} was built for Rick {}, and this is {rick}: run `cargo xtask bundle` again",
                folder.display(),
                plugin.rick
            )
            .into());
        }
        let file = format!("{}-{}-{platform}.rkp", plugin.id, plugin.version);
        let (size, sha256) = pack(&folder, &plugin.id, &out.join(&file))?;
        entries.push(entry(&plugin, &file, size, &sha256));
        packages.push(Package {
            id: plugin.id,
            file,
            size,
        });
    }

    let manifest = serde_json::json!({
        "app": APP,
        "app_version": rick,
        "platform": platform,
        "plugins": entries,
    });
    let mut text = serde_json::to_string_pretty(&manifest)?;
    text.push('\n');
    std::fs::write(out.join(manifest_name(platform)), text)?;
    Ok(packages)
}

/// The manifest's file name for `platform`.
#[must_use]
pub fn manifest_name(platform: &str) -> String {
    format!("plugins-{platform}.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory of its own under the system's temporary one, removed
    /// when it goes out of scope.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("rick-xtask-package-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap_or_else(|e| panic!("{e}"));
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const OS: &str = "linux";

    /// A plugin folder as the bundler lays it out; `more` goes into the
    /// manifest after the scalar keys.
    fn plugin(plugins: &Path, id: &str, stem: &str, rick: &str, more: &str) -> PathBuf {
        let dir = plugins.join(id);
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        let library = format!(
            "{}{stem}{}",
            std::env::consts::DLL_PREFIX,
            std::env::consts::DLL_SUFFIX
        );
        let manifest = format!(
            "id = \"{id}\"\nname = \"The {id} plugin\"\ndescription = \"What {id} does.\"\n\
             version = \"1.2.3\"\napi_version = 23\npoints = [\"decorator\", \"action\"]\n\
             platforms = [\"linux\", \"macos\", \"windows\"]\nrick = \"{rick}\"\n\
             toolchain = \"rustc\"\n{more}\n[library]\n{OS} = \"{library}\"\n"
        );
        std::fs::write(dir.join(MANIFEST_FILE), manifest).unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(
            dir.join(library),
            format!("the library of {id}").repeat(200),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        dir
    }

    /// A bundle's plugins folder: Git with a file in a folder of its own,
    /// S3, and the trash, which is not separate.
    fn bundle(root: &Path) -> PathBuf {
        let plugins = root.join("plugins");
        let git = plugin(&plugins, "git", "ext_git", "9.9.9", "markers = [\".git\"]");
        std::fs::create_dir_all(git.join("share").join("deep")).unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(git.join("share").join("deep").join("notes.txt"), "notes")
            .unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(git.join("LICENSE"), "licence").unwrap_or_else(|e| panic!("{e}"));
        let s3 = plugin(
            &plugins,
            "vfs-s3",
            "ext_vfs_s3",
            "9.9.9",
            "previews = []\nmarkers = []",
        );
        let manifest = s3.join(MANIFEST_FILE);
        let text = std::fs::read_to_string(&manifest).unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(
            &manifest,
            format!("{text}\n[[connections]]\nsection = \"cloud\"\ntitle = \"S3\"\n"),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        plugin(&plugins, "vfs-trash", "ext_vfs_trash", "9.9.9", "");
        plugins
    }

    fn separate() -> BTreeSet<String> {
        ["ext_git", "ext_vfs_s3", "ext_zfs"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    fn sha256(bytes: &[u8]) -> String {
        hex(&Sha256::digest(bytes))
    }

    fn keys(value: &serde_json::Value) -> Vec<&str> {
        value
            .as_object()
            .unwrap_or_else(|| panic!("not an object: {value}"))
            .keys()
            .map(String::as_str)
            .collect()
    }

    #[test]
    fn the_platforms_are_named_as_the_release_names_its_archives() {
        assert_eq!(platform_name("linux", "x86_64"), "linux-x64");
        assert_eq!(platform_name("linux", "aarch64"), "linux-arm64");
        assert_eq!(platform_name("macos", "aarch64"), "macos-arm64");
        assert_eq!(platform_name("windows", "x86_64"), "windows-x64");
    }

    #[test]
    fn a_crate_is_separate_everywhere_on_the_platforms_it_names_or_not_at_all() {
        let mark = |text: &str| serde_json::from_str(text).unwrap_or_else(|e| panic!("{e}"));
        for os in ["linux", "macos", "windows"] {
            assert_eq!(is_separate(&mark("true"), os).ok(), Some(true), "{os}");
            assert_eq!(is_separate(&mark("false"), os).ok(), Some(false), "{os}");
            assert_eq!(is_separate(&mark("null"), os).ok(), Some(false), "{os}");
        }
        let media = mark(r#"["macos", "windows"]"#);
        assert_eq!(is_separate(&media, "linux").ok(), Some(false));
        assert_eq!(is_separate(&media, "macos").ok(), Some(true));
        assert_eq!(is_separate(&media, "windows").ok(), Some(true));
        // A misspelt platform or another kind of value is an error, not a
        // plugin that silently stays in the archive.
        assert!(is_separate(&mark(r#"["macosx"]"#), "macos").is_err());
        assert!(is_separate(&mark(r#""yes""#), "linux").is_err());
    }

    #[test]
    fn the_same_bundle_gives_the_same_bytes_and_one_package_per_separate_plugin() {
        let scratch = Scratch::new("same-bytes");
        let plugins = bundle(&scratch.0);
        let first = scratch.0.join("first");
        let second = scratch.0.join("second");
        let made = write(&plugins, &first, "linux-x64", OS, "9.9.9", &separate())
            .unwrap_or_else(|e| panic!("{e}"));
        let again = write(&plugins, &second, "linux-x64", OS, "9.9.9", &separate())
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(made, again);
        let names: Vec<&str> = made.iter().map(|package| package.file.as_str()).collect();
        // Git and S3; the trash is in the default set, and ZFS is separate
        // but not in this bundle.
        assert_eq!(
            names,
            ["git-1.2.3-linux-x64.rkp", "vfs-s3-1.2.3-linux-x64.rkp"]
        );
        for name in names.into_iter().chain(["plugins-linux-x64.json"]) {
            let one = std::fs::read(first.join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
            let two = std::fs::read(second.join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(one == two, "{name} differs between two runs");
        }
        let written = std::fs::read_dir(&first)
            .unwrap_or_else(|e| panic!("{e}"))
            .count();
        assert_eq!(written, 3, "two packages and the manifest");
    }

    #[test]
    fn a_package_holds_the_plugin_s_folder_as_the_bundle_has_it() {
        let scratch = Scratch::new("folder");
        let plugins = bundle(&scratch.0);
        let out = scratch.0.join("packages");
        write(&plugins, &out, "linux-x64", OS, "9.9.9", &separate())
            .unwrap_or_else(|e| panic!("{e}"));
        let file =
            File::open(out.join("git-1.2.3-linux-x64.rkp")).unwrap_or_else(|e| panic!("{e}"));
        let mut archive = zip::ZipArchive::new(file).unwrap_or_else(|e| panic!("{e}"));
        let library = format!(
            "git/{}ext_git{}",
            std::env::consts::DLL_PREFIX,
            std::env::consts::DLL_SUFFIX
        );
        let mut expected = vec![
            "git/LICENSE".to_owned(),
            library.clone(),
            "git/plugin.toml".to_owned(),
            "git/share/deep/notes.txt".to_owned(),
        ];
        expected.sort();
        let names: Vec<String> = (0..archive.len())
            .map(|index| {
                let entry = archive.by_index(index).unwrap_or_else(|e| panic!("{e}"));
                let name = entry.name().unwrap_or_else(|e| panic!("{e}")).into_owned();
                // One timestamp and one mode for every entry.
                assert_eq!(
                    entry.last_modified(),
                    Some(zip::DateTime::DEFAULT),
                    "{name}"
                );
                assert_eq!(
                    entry.unix_mode().map(|mode| mode & 0o777),
                    Some(ENTRY_MODE),
                    "{name}"
                );
                name
            })
            .collect();
        assert_eq!(names, expected, "every file, in the order of the names");
        // The content is the bundle's.
        let mut packed = Vec::new();
        archive
            .by_name(&library)
            .unwrap_or_else(|e| panic!("{e}"))
            .read_to_end(&mut packed)
            .unwrap_or_else(|e| panic!("{e}"));
        let on_disk = std::fs::read(plugins.join(&library)).unwrap_or_else(|e| panic!("{e}"));
        assert!(packed == on_disk, "the library is not the bundle's");
    }

    #[test]
    fn the_manifest_names_every_package_with_its_size_and_hash_in_the_publisher_s_shape() {
        let scratch = Scratch::new("manifest");
        let plugins = bundle(&scratch.0);
        let out = scratch.0.join("packages");
        let made = write(&plugins, &out, "linux-x64", OS, "9.9.9", &separate())
            .unwrap_or_else(|e| panic!("{e}"));
        let text = std::fs::read_to_string(out.join(manifest_name("linux-x64")))
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(text.ends_with("}\n"), "{text}");
        let manifest: serde_json::Value =
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}"));

        // The shape of the publisher's contract, key for key and in its
        // order (Open-Breach/trebuchet, docs/spec.md 7.4).
        assert_eq!(
            keys(&manifest),
            ["app", "app_version", "platform", "plugins"]
        );
        assert_eq!(manifest["app"], "rick");
        assert_eq!(manifest["app_version"], "9.9.9");
        assert_eq!(manifest["platform"], "linux-x64");
        let listed = manifest["plugins"]
            .as_array()
            .unwrap_or_else(|| panic!("{text}"));
        assert_eq!(listed.len(), made.len());
        for (entry, package) in listed.iter().zip(&made) {
            assert_eq!(
                keys(entry),
                [
                    "id",
                    "name",
                    "description",
                    "version",
                    "api_version",
                    "meta",
                    "file",
                    "size",
                    "sha256"
                ]
            );
            assert_eq!(keys(&entry["meta"]), ["points", "triggers"]);
            assert_eq!(entry["id"], package.id.as_str());
            assert_eq!(entry["version"], "1.2.3");
            assert_eq!(entry["api_version"], 23);
            // The file is there, as large as the entry says and with its
            // hash.
            let file = entry["file"].as_str().unwrap_or_else(|| panic!("{entry}"));
            assert_eq!(file, package.file);
            let bytes = std::fs::read(out.join(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
            assert_eq!(entry["size"], bytes.len());
            assert_eq!(entry["size"], package.size);
            assert_eq!(entry["sha256"], sha256(&bytes).as_str());
        }

        let git = &listed[0];
        assert_eq!(git["name"], "The git plugin");
        assert_eq!(git["description"], "What git does.");
        assert_eq!(
            git["meta"],
            serde_json::json!({
                "points": ["decorator", "action"],
                "triggers": { "markers": [".git"] },
            })
        );
        assert_eq!(
            listed[1]["meta"]["triggers"],
            serde_json::json!({ "connections": [{ "section": "cloud", "title": "S3" }] })
        );
    }

    #[test]
    fn a_bundle_of_another_rick_or_a_misnamed_folder_is_refused() {
        let scratch = Scratch::new("refused");
        let plugins = scratch.0.join("plugins");
        plugin(&plugins, "git", "ext_git", "0.4.1", "");
        let out = scratch.0.join("packages");
        let error = write(&plugins, &out, "linux-x64", OS, "9.9.9", &separate())
            .err()
            .map(|e| e.to_string());
        assert!(
            error
                .as_deref()
                .is_some_and(|e| e.contains("was built for Rick 0.4.1, and this is 9.9.9")),
            "{error:?}"
        );

        let scratch = Scratch::new("misnamed");
        let plugins = scratch.0.join("plugins");
        let git = plugin(&plugins, "git", "ext_git", "9.9.9", "");
        std::fs::rename(&git, plugins.join("gitt")).unwrap_or_else(|e| panic!("{e}"));
        let error = write(&plugins, &out, "linux-x64", OS, "9.9.9", &separate())
            .err()
            .map(|e| e.to_string());
        assert!(
            error
                .as_deref()
                .is_some_and(|e| e.contains("holds the plugin `git`")),
            "{error:?}"
        );
    }
}
