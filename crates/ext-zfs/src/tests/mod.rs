//! Tests over a fixture tree that mimics `<mountpoint>/.zfs/snapshot/<s>/…`
//! in a temporary directory and a scripted `zfs`.

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
