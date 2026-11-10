//! Tests over a fixture tree that mimics `<mountpoint>/.zfs/snapshot/<s>/…`
//! in a temporary directory and a scripted `zfs`.

mod actions;
mod decorator;
mod live;
mod manage;
mod restore;
mod provider;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use ext::{Depth, ListOptions, Provider};
use futures::StreamExt;
use types::{CancelToken, Entry, Location};

use crate::datasets::Datasets;
use crate::parse;
use crate::provider::ZfsProvider;
use crate::zfs::{Output, ScriptedZfs};

/// A machine with `tank` mounted at the temp dir and `tank/home` at
/// `<tmp>/home`; `tank/home` has snapshots `a` (older) and `b`.
///
/// ```text
/// home/ann/docs/report.txt           "live"     (live)
/// home/.zfs/snapshot/a/ann/docs/report.txt "same"
/// home/.zfs/snapshot/b/ann/docs/report.txt "same" (same size and mtime)
/// home/.zfs/snapshot/b/ann/.hidden
/// home/.zfs/snapshot/b/ann/link -> docs/report.txt
/// home/.zfs/snapshot/b/ann/old.txt   (not live any more)
/// ```
pub(crate) struct Fixture {
    pub dir: tempfile::TempDir,
    pub zfs: Arc<ScriptedZfs>,
    pub datasets: Arc<Datasets>,
}

pub(crate) fn write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap_or_else(|e| panic!("{e}"));
    }
    fs::write(path, text).unwrap_or_else(|e| panic!("{e}"));
}

pub(crate) fn set_mtime(path: &Path, secs: u64) {
    let file = fs::File::options()
        .write(true)
        .open(path)
        .unwrap_or_else(|e| panic!("{e}"));
    file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
        .unwrap_or_else(|e| panic!("{e}"));
}

pub(crate) fn fs_args() -> Vec<&'static str> {
    vec![
        "list",
        "-H",
        "-p",
        "-o",
        "name,mountpoint,mounted",
        "-t",
        "filesystem",
    ]
}

pub(crate) fn on_snapshots(zfs: &ScriptedZfs, dataset: &str, out: &str) {
    let args = parse::list_snapshots_args(dataset);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    zfs.on(&args, Output::ok(out));
}

impl Fixture {
    pub fn new() -> Self {
        // The messages the tests read, whatever ran before.
        ext::i18n::add_bundled("en", include_str!("../../locales/en.ftl"));
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let root = dir.path();
        let zfs = Arc::new(ScriptedZfs::new());
        zfs.on(
            &fs_args(),
            Output::ok(format!(
                "tank\t{}\tyes\ntank/home\t{}\tyes\n",
                root.display(),
                root.join("home").display()
            )),
        );
        on_snapshots(&zfs, "tank", "");
        on_snapshots(
            &zfs,
            "tank/home",
            "tank/home@a\t100\t10\ntank/home@b\t200\t20\n",
        );
        let home = root.join("home");
        write(&home.join("ann/docs/report.txt"), "live");
        for snap in ["a", "b"] {
            let file = home.join(format!(".zfs/snapshot/{snap}/ann/docs/report.txt"));
            write(&file, "same");
            set_mtime(&file, 1_000);
        }
        let b = home.join(".zfs/snapshot/b/ann");
        write(&b.join(".hidden"), "h");
        write(&b.join("old.txt"), "gone live");
        ext::platform::fs::symlink(Path::new("docs/report.txt"), &b.join("link"))
            .unwrap_or_else(|e| panic!("{e}"));
        let datasets = Arc::new(Datasets::new(zfs.clone()));
        Self { dir, zfs, datasets }
    }

    pub fn home(&self) -> PathBuf {
        self.dir.path().join("home")
    }

    pub fn snapshot_dir(&self, snap: &str) -> PathBuf {
        self.home().join(".zfs/snapshot").join(snap)
    }

    pub fn provider(&self) -> ZfsProvider {
        ZfsProvider::new(Arc::clone(&self.datasets))
    }
}

pub(crate) fn list_with(provider: &dyn Provider, loc: &Location, opts: ListOptions) -> Vec<Entry> {
    runtime::block_on(async {
        let mut stream = provider.list(loc, opts, CancelToken::new());
        let mut out = Vec::new();
        while let Some(batch) = stream.next().await {
            out.extend(batch.unwrap_or_else(|e| panic!("{loc}: {e}")));
        }
        out
    })
}

pub(crate) fn list(provider: &dyn Provider, loc: &Location) -> Vec<Entry> {
    list_with(
        provider,
        loc,
        ListOptions {
            hidden: false,
            system: false,
            depth: Depth::One,
        },
    )
}

pub(crate) fn names(entries: &[Entry]) -> Vec<String> {
    let mut names: Vec<String> = entries.iter().map(|e| e.name.clone()).collect();
    names.sort();
    names
}

/// A UI whose answers the test sets: the confirmation, and what a form
/// returns (its defaults with some fields replaced, or nothing).
#[derive(Default)]
pub(crate) struct Scripted {
    pub confirm: std::sync::Mutex<bool>,
    pub form: std::sync::Mutex<Option<Vec<(&'static str, ext::Value)>>>,
    pub dismiss_form: std::sync::Mutex<bool>,
    pub asked: std::sync::Mutex<Vec<String>>,
    pub forms: std::sync::Mutex<Vec<ext::Form>>,
}

fn guard<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Scripted {
    pub fn confirming(answer: bool) -> Self {
        let ui = Self::default();
        *guard(&ui.confirm) = answer;
        ui
    }

    pub fn asked(&self) -> Vec<String> {
        guard(&self.asked).clone()
    }

}

impl ext::UiHost for Scripted {
    fn notify(&self, _level: ext::Level, _text: &str) {}
    fn confirm(&self, title: &str, text: &str) -> bool {
        guard(&self.asked).push(format!("{title}: {text}"));
        *guard(&self.confirm)
    }
    fn prompt(&self, _title: &str, default: &str) -> Option<String> {
        Some(default.to_owned())
    }
    fn show(&self, _title: &str, _text: &str) {}
    fn form(&self, form: ext::Form) -> Option<ext::Values> {
        let mut values = form.defaults();
        guard(&self.forms).push(form);
        if *guard(&self.dismiss_form) {
            return None;
        }
        for (key, value) in guard(&self.form).clone().unwrap_or_default() {
            values.set(key, value);
        }
        Some(values)
    }
}

/// An action context for the pane at `location` with `selection`.
pub(crate) fn ctx(location: Location, selection: Vec<Entry>) -> ext::ActionCtx {
    ext::ActionCtx {
        pane: ext::PaneId(1),
        location,
        selection,
        invoked_from: ext::Invocation::Palette,
        other: None,
    }
}

/// The entry for a live path, as the local provider would list it.
pub(crate) fn live_entry(path: &Path) -> Entry {
    let raw = ext::platform::fs::stat(path).unwrap_or_else(|e| panic!("{e}"));
    let dir = path.parent().unwrap_or(path).to_path_buf();
    let mut entry = crate::provider::entry_from_raw(&dir, ext::platform::paths::from_path(path), raw);
    entry.id.provider = types::ProviderId("local");
    entry
}

/// The entry at a location of the provider.
pub(crate) fn zfs_entry(provider: &ZfsProvider, loc: &Location) -> Entry {
    runtime::block_on(provider.stat(loc)).unwrap_or_else(|e| panic!("{loc}: {e}"))
}
