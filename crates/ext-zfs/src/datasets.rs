//! Which dataset a local path lies in, and the snapshots of each dataset.
//!
//! The mounted file systems come from one `zfs list` call, made the first
//! time anyone asks; a live path maps to the dataset with the longest mount
//! point that is a prefix of it on whole path components. The snapshot
//! lists are read per dataset and kept for [`TTL`]; a management job drops
//! its dataset's list when it finishes. Without `zfs`, or when the list
//! fails, there are no datasets: that is logged once, at `debug`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use types::Error;

use crate::parse::{self, FileSystem, Snapshot};
use crate::zfs::{Zfs, ZfsError, classify, to_error};

/// How long a snapshot list is trusted before it is read again.
pub const TTL: Duration = Duration::from_secs(30);

/// A snapshot list and when it was read.
#[derive(Debug, Clone)]
struct Read {
    at: Instant,
    /// Oldest first, as `zfs list -s creation` prints them.
    snapshots: Arc<Vec<Snapshot>>,
}

/// The datasets of this machine and their snapshots, as far as `zfs`
/// tells.
#[derive(Debug)]
pub struct Datasets {
    zfs: Arc<dyn Zfs>,
    ttl: Duration,
    filesystems: OnceLock<Vec<FileSystem>>,
    snapshots: Mutex<HashMap<String, Read>>,
    told: AtomicBool,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Datasets {
    /// The datasets `zfs` reports, read when first needed.
    #[must_use]
    pub fn new(zfs: Arc<dyn Zfs>) -> Self {
        Self::with_ttl(zfs, TTL)
    }

    /// The same with another time a snapshot list is trusted, for tests.
    #[must_use]
    pub fn with_ttl(zfs: Arc<dyn Zfs>, ttl: Duration) -> Self {
        Self {
            zfs,
            ttl,
            filesystems: OnceLock::new(),
            snapshots: Mutex::default(),
            told: AtomicBool::new(false),
        }
    }

    /// The runner every call goes through.
    #[must_use]
    pub fn zfs(&self) -> &Arc<dyn Zfs> {
        &self.zfs
    }

    /// Logs, once per process, why there are no datasets.
    fn tell_once(&self, why: &dyn std::fmt::Display) {
        if !self.told.swap(true, Ordering::Relaxed) {
            tracing::debug!(%why, "no ZFS datasets: the ZFS extension stays out of sight");
        }
    }

    /// Runs `zfs args…` and classifies the result.
    fn run(&self, args: &[String]) -> Result<String, Error> {
        let output = self.zfs.run(args)?;
        classify(output).map_err(to_error)
    }

    /// The mounted file systems, read on the first call.
    #[must_use]
    pub fn filesystems(&self) -> &[FileSystem] {
        self.filesystems.get_or_init(|| {
            match self.run(&parse::list_filesystems_args()) {
                Ok(out) => {
                    let read = parse::filesystems(&out);
                    if read.is_empty() {
                        self.tell_once(&"zfs lists no mounted file system");
                    }
                    read
                }
                Err(error) => {
                    self.tell_once(&error);
                    Vec::new()
                }
            }
        })
    }

    /// The file system `path` lies in: the one with the longest mount
    /// point that is a prefix of `path` on whole components.
    #[must_use]
    pub fn dataset_of(&self, path: &Path) -> Option<&FileSystem> {
        if !path.is_absolute() {
            return None;
        }
        self.filesystems()
            .iter()
            .filter(|fs| path.starts_with(&fs.mountpoint))
            .max_by_key(|fs| fs.mountpoint.components().count())
    }

    /// The mounted file system named `name`.
    #[must_use]
    pub fn named(&self, name: &str) -> Option<&FileSystem> {
        self.filesystems().iter().find(|fs| fs.name == name)
    }

    /// The path of `path` relative to its dataset's mount point, with the
    /// dataset.
    #[must_use]
    pub fn split(&self, path: &Path) -> Option<(&FileSystem, PathBuf)> {
        let fs = self.dataset_of(path)?;
        let rel = path.strip_prefix(&fs.mountpoint).ok()?.to_path_buf();
        Some((fs, rel))
    }

    /// The snapshots of `dataset`, newest first, from the cache when it is
    /// younger than the TTL, read again otherwise.
    ///
    /// # Errors
    ///
    /// `Error::Unsupported` without `zfs`; ZFS's error when the list fails.
    pub fn snapshots(&self, dataset: &str) -> Result<Vec<Snapshot>, Error> {
        if let Some(fresh) = self.fresh(dataset) {
            return Ok(newest_first(&fresh));
        }
        let out = self.run(&parse::list_snapshots_args(dataset))?;
        let read: Vec<Snapshot> = parse::snapshots(&out)
            .into_iter()
            .filter(|s| s.dataset == dataset)
            .collect();
        let read = Arc::new(read);
        lock(&self.snapshots).insert(
            dataset.to_owned(),
            Read {
                at: Instant::now(),
                snapshots: Arc::clone(&read),
            },
        );
        Ok(newest_first(&read))
    }

    /// The cached snapshot list of `dataset`, newest first, when it is
    /// younger than the TTL; nothing is read.
    #[must_use]
    pub fn cached(&self, dataset: &str) -> Option<Vec<Snapshot>> {
        self.fresh(dataset).map(|read| newest_first(&read))
    }

    fn fresh(&self, dataset: &str) -> Option<Arc<Vec<Snapshot>>> {
        lock(&self.snapshots)
            .get(dataset)
            .filter(|read| read.at.elapsed() < self.ttl)
            .map(|read| Arc::clone(&read.snapshots))
    }

    /// Forgets the snapshot list of `dataset`: the next request reads it.
    pub fn invalidate(&self, dataset: &str) {
        lock(&self.snapshots).remove(dataset);
    }

    /// The mounted datasets that have snapshots, with them, newest first,
    /// sorted by dataset name. A dataset whose list fails is left out.
    #[must_use]
    pub fn with_snapshots(&self) -> Vec<(FileSystem, Vec<Snapshot>)> {
        let mut out: Vec<(FileSystem, Vec<Snapshot>)> = self
            .filesystems()
            .iter()
            .filter_map(|fs| match self.snapshots(&fs.name) {
                Ok(list) if !list.is_empty() => Some((fs.clone(), list)),
                Ok(_) => None,
                Err(error) => {
                    tracing::debug!(dataset = %fs.name, %error, "snapshots not listed");
                    None
                }
            })
            .collect();
        out.sort_by(|a, b| a.0.name.cmp(&b.0.name));
        out
    }

    /// Where the snapshot `snapshot` of `fs` shows its files.
    #[must_use]
    pub fn snapshot_dir(fs: &FileSystem, snapshot: &str) -> PathBuf {
        fs.mountpoint.join(".zfs").join("snapshot").join(snapshot)
    }
}

fn newest_first(list: &[Snapshot]) -> Vec<Snapshot> {
    list.iter().rev().cloned().collect()
}

/// Whether a ZFS error is the one a missing dataset gives.
#[must_use]
pub fn is_missing(error: &ZfsError) -> bool {
    error.message().contains("does not exist")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zfs::{Output, ScriptedZfs};

    const LIST: &str = "tank\t/tank\tyes\ntank/home\t/tank/home\tyes\ntank/homework\t/tank/homework\tyes\ntank/off\tnone\tno\n";

    fn scripted() -> Arc<ScriptedZfs> {
        let zfs = Arc::new(ScriptedZfs::new());
        zfs.on(
            &[
                "list",
                "-H",
                "-p",
                "-o",
                "name,mountpoint,mounted",
                "-t",
                "filesystem",
            ],
            Output::ok(LIST),
        );
        zfs
    }

    fn snapshot_args(dataset: &str) -> Vec<String> {
        parse::list_snapshots_args(dataset)
    }

    fn on_snapshots(zfs: &ScriptedZfs, dataset: &str, out: &str) {
        let args = snapshot_args(dataset);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        zfs.on(&args, Output::ok(out));
    }

    #[test]
    fn nested_datasets_pick_the_longest_mount_point() {
        let zfs = scripted();
        let sets = Datasets::new(zfs.clone());
        let name = |p: &str| sets.dataset_of(Path::new(p)).map(|fs| fs.name.clone());
        assert_eq!(name("/tank/home/ann/docs"), Some("tank/home".to_owned()));
        assert_eq!(name("/tank/home"), Some("tank/home".to_owned()));
        // A component prefix, not a string prefix.
        assert_eq!(name("/tank/homework/x"), Some("tank/homework".to_owned()));
        assert_eq!(name("/tank/homes"), Some("tank".to_owned()));
        assert_eq!(name("/tank"), Some("tank".to_owned()));
        let (fs, rel) = sets.split(Path::new("/tank/home/ann/docs")).unwrap_or_else(|| {
            panic!("no split")
        });
        assert_eq!(fs.name, "tank/home");
        assert_eq!(rel, PathBuf::from("ann/docs"));
        // One file-system list serves every question.
        assert_eq!(zfs.calls().len(), 1);
    }

    #[test]
    fn a_path_outside_every_dataset_has_none() {
        let sets = Datasets::new(scripted());
        assert!(sets.dataset_of(Path::new("/home/ann")).is_none());
        assert!(sets.dataset_of(Path::new("relative/tank")).is_none());
        assert!(sets.named("tank/off").is_none());
        assert!(sets.named("tank/home").is_some());
    }

    #[test]
    fn snapshot_lists_are_cached_newest_first_until_the_ttl() {
        let zfs = scripted();
        on_snapshots(&zfs, "tank/home", "tank/home@a\t100\t1\ntank/home@b\t200\t2\n");
        let sets = Datasets::new(zfs.clone());
        assert!(sets.cached("tank/home").is_none());
        let names = |list: Vec<Snapshot>| list.into_iter().map(|s| s.name).collect::<Vec<_>>();
        assert_eq!(names(sets.snapshots("tank/home").unwrap_or_default()), ["b", "a"]);
        assert_eq!(names(sets.snapshots("tank/home").unwrap_or_default()), ["b", "a"]);
        assert_eq!(names(sets.cached("tank/home").unwrap_or_default()), ["b", "a"]);
        assert_eq!(zfs.calls().len(), 1, "read once: {:?}", zfs.calls());

        let expired = Datasets::with_ttl(zfs.clone(), Duration::ZERO);
        let _ = expired.snapshots("tank/home");
        let _ = expired.snapshots("tank/home");
        assert!(expired.cached("tank/home").is_none());
        assert_eq!(zfs.calls().len(), 3);
    }

    #[test]
    fn invalidation_drops_one_list() {
        let zfs = scripted();
        on_snapshots(&zfs, "tank/home", "tank/home@a\t100\t1\n");
        let sets = Datasets::new(zfs.clone());
        let _ = sets.snapshots("tank/home");
        on_snapshots(&zfs, "tank/home", "tank/home@a\t100\t1\ntank/home@c\t300\t3\n");
        assert_eq!(sets.snapshots("tank/home").unwrap_or_default().len(), 1);
        sets.invalidate("tank/home");
        assert_eq!(sets.snapshots("tank/home").unwrap_or_default().len(), 2);
    }

    #[test]
    fn only_datasets_with_snapshots_are_offered() {
        let zfs = scripted();
        on_snapshots(&zfs, "tank", "");
        on_snapshots(&zfs, "tank/home", "tank/home@a\t100\t1\n");
        // tank/homework is not scripted: its list fails and it is left out.
        let sets = Datasets::new(zfs);
        let offered: Vec<String> = sets
            .with_snapshots()
            .into_iter()
            .map(|(fs, _)| fs.name)
            .collect();
        assert_eq!(offered, ["tank/home"]);
    }

    #[test]
    fn without_zfs_there_are_no_datasets() {
        let sets = Datasets::new(Arc::new(ScriptedZfs::missing()));
        assert!(sets.filesystems().is_empty());
        assert!(sets.dataset_of(Path::new("/tank/home")).is_none());
        assert!(sets.with_snapshots().is_empty());
        assert!(sets.told.load(Ordering::Relaxed));
    }

    #[test]
    fn a_failed_list_means_no_datasets() {
        let zfs = Arc::new(ScriptedZfs::new());
        zfs.on(
            &[
                "list",
                "-H",
                "-p",
                "-o",
                "name,mountpoint,mounted",
                "-t",
                "filesystem",
            ],
            Output::failed(1, "The ZFS modules cannot be auto-loaded.\n"),
        );
        let sets = Datasets::new(zfs.clone());
        assert!(sets.dataset_of(Path::new("/tank")).is_none());
        assert!(sets.filesystems().is_empty());
        assert_eq!(zfs.calls().len(), 1, "the failure is remembered");
        assert!(sets.told.load(Ordering::Relaxed));
    }

    #[test]
    fn the_snapshot_directory_is_under_the_mount_point() {
        let fs = FileSystem {
            name: "tank/home".to_owned(),
            mountpoint: PathBuf::from("/tank/home"),
        };
        assert_eq!(
            Datasets::snapshot_dir(&fs, "b"),
            PathBuf::from("/tank/home/.zfs/snapshot/b")
        );
    }
}
