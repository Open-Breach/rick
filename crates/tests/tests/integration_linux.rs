//! Acceptance (issue 120, specs/system-integration.md §6.1 and §7.1): the
//! Linux integration items against a home of the test's own. Nothing here
//! touches the real system: every path is under a temporary directory
//! with its own data and config homes, `gsettings` is a map in memory,
//! and nothing runs `pkexec`.

#![cfg(target_os = "linux")]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use platform::integration::{Actual, Item, Reason, Request};
use platform::{Gsettings, Integration as _, LinuxEnv, LinuxIntegration};
use types::Error;

/// `gsettings` in memory: the keys a test set.
#[derive(Debug, Default)]
struct Settings(Mutex<BTreeMap<String, String>>);

impl Gsettings for Settings {
    fn get(&self, schema: &str, key: &str) -> Result<String, Error> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&format!("{schema} {key}"))
            .cloned()
            .ok_or(Error::Unsupported("no such key"))
    }

    fn set(&self, schema: &str, key: &str, value: &str) -> Result<(), Error> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(format!("{schema} {key}"), value.to_owned());
        Ok(())
    }
}

/// A scratch folder with a Rick unpacked into it, as the release archive
/// unpacks, and an empty home beside it.
struct Scratch {
    dir: tempfile::TempDir,
    env: LinuxEnv,
}

impl Scratch {
    fn new(desktop: &str) -> Self {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let home = dir.path().join("home");
        let unpacked = dir.path().join("Downloads/rick-0.3.0");
        fs::create_dir_all(&home).unwrap_or_else(|e| panic!("{e}"));
        fs::create_dir_all(unpacked.join("plugins")).unwrap_or_else(|e| panic!("{e}"));
        fs::write(unpacked.join("rick"), b"#!/bin/sh\n").unwrap_or_else(|e| panic!("{e}"));
        let env = LinuxEnv::under(&home, &unpacked.join("rick"), desktop);
        Self { dir, env }
    }

    fn integration(&self) -> LinuxIntegration {
        LinuxIntegration::new(self.env.clone(), Arc::new(Settings::default()))
    }
}

/// Every path under `dir`, with what a file holds or a link points at.
fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
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
                out.push((
                    path.clone(),
                    fs::read(&path).unwrap_or_else(|e| panic!("{e}")),
                ));
            }
        }
    }
    out.sort();
    out
}

/// A Rick unpacked into a scratch folder has nothing set up: every item
/// the desktop supports is missing, the install location names where
/// Rick is and where it would go, and the check wrote nothing.
#[test]
fn the_check_of_an_unpacked_rick_finds_everything_missing_and_changes_nothing() {
    let scratch = Scratch::new("GNOME");
    let integration = scratch.integration();
    assert_eq!(integration.integration_items().items, Item::ALL);
    let before = snapshot(scratch.dir.path());
    for item in [
        Item::InstallLocation,
        Item::LauncherEntry,
        Item::DefaultFileManager,
        Item::ShowInFolder,
        Item::RickCommand,
    ] {
        let check = integration.check_integration(item, &Request::default());
        assert_eq!(check.actual, Actual::Missing, "{item:?}");
        assert!(check.foreign.is_empty(), "{item:?}");
    }
    let location = integration.check_integration(Item::InstallLocation, &Request::default());
    assert_eq!(
        location.place,
        Some(scratch.env.home.join(".local/opt/rick"))
    );
    assert_eq!(
        location.target,
        Some(scratch.dir.path().join("Downloads/rick-0.3.0"))
    );
    assert_eq!(
        snapshot(scratch.dir.path()),
        before,
        "the check is read-only"
    );
}

/// On a desktop outside the four the dock row is *Unsupported*; the other
/// rows are what they are anywhere.
#[test]
fn on_a_desktop_outside_the_four_the_dock_row_is_unsupported() {
    let scratch = Scratch::new("XFCE");
    let integration = scratch.integration();
    assert_eq!(integration.integration_items().desktop, None);
    assert_eq!(
        integration
            .check_integration(Item::DockFavourite, &Request::default())
            .actual,
        Actual::Unsupported(Reason::Desktop)
    );
    assert_eq!(
        integration
            .check_integration(Item::LauncherEntry, &Request::default())
            .actual,
        Actual::Missing
    );
}
