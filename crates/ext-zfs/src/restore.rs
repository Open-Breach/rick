//! *Restore*: put entries from a snapshot back at their live location.
//!
//! The plugin cannot use `ops` (it sits above the plugins), so Restore is
//! its own job: for each selected entry it computes the live path (mount
//! point plus the path below the snapshot), asks once to replace the live
//! items that exist, moves those to the trash through the platform's
//! [`Trash`], creates missing parents and copies the entry back: regular
//! files, folders and symbolic links, with their permissions and
//! modification times. The cancel token is checked between files. An item
//! whose live version cannot be trashed is skipped and reported. Not
//! undoable: the replaced version is in the trash, the restored one still
//! in the snapshot.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ext::platform::Trash;
use ext::platform::fs as pfs;
use ext::{
    Action, ActionCtx, ActionGroup, ActionId, ActionOutcome, Availability, JobRunner, Shortcut,
    UiHost, t,
};
use types::{CancelToken, Error, Location, Progress};

use crate::actions::{GROUP, Shared, live_path};
use crate::provider::{PROVIDER_ID, Target, target};

/// *Restore*.
pub const RESTORE: ActionId = ActionId("zfs.restore");

/// One entry to put back: where it is in the snapshot and where it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// The entry's OS path under `.zfs/snapshot/`.
    pub from: PathBuf,
    /// The live path it is restored to.
    pub to: PathBuf,
}

/// Why some items were not restored.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct NotRestored(pub String);

/// The items for the entries at `locations`, which must lie inside a
/// snapshot below its root.
///
/// # Errors
///
/// `Error::Unsupported` for a location that is not inside a snapshot or is
/// a snapshot's root; `Error::NotFound` when its dataset is not mounted.
pub fn items(shared: &Shared, locations: &[Location]) -> Result<Vec<Item>, Error> {
    locations
        .iter()
        .map(|loc| {
            let Ok(Target::Snapshot {
                dataset,
                snapshot,
                rel,
            }) = target(loc)
            else {
                return Err(Error::Unsupported("only entries inside a snapshot restore"));
            };
            if rel.as_os_str().is_empty() {
                return Err(Error::Unsupported(
                    "a whole snapshot is not restored over its dataset",
                ));
            }
            let fs = shared.datasets.named(&dataset).ok_or(Error::NotFound)?;
            let from = crate::datasets::Datasets::snapshot_dir(fs, &snapshot).join(&rel);
            let to = live_path(&shared.datasets, loc).ok_or(Error::NotFound)?;
            Ok(Item { from, to })
        })
        .collect()
}

/// Restores `items`: trashes each live item in the way, then copies.
///
/// # Errors
///
/// `Error::Cancelled` when the token fired; an extension error naming the
/// items that were skipped because their live version could not be
/// trashed or the copy failed.
pub fn restore(
    items: &[Item],
    trash: &dyn Trash,
    cancel: &CancelToken,
    progress: &mut dyn FnMut(usize),
) -> Result<(), Error> {
    let mut skipped = String::new();
    for (done, item) in items.iter().enumerate() {
        progress(done);
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        if item.to.symlink_metadata().is_ok()
            && let Err(error) = trash.trash(&item.to)
        {
            tracing::warn!(path = %item.to.display(), %error, "live item not trashed; not restored");
            let _ = writeln!(skipped, "{}: {error}", item.to.display());
            continue;
        }
        if let Err(error) = put_back(&item.from, &item.to, cancel) {
            if matches!(error, Error::Cancelled) {
                return Err(error);
            }
            let _ = writeln!(skipped, "{}: {error}", item.to.display());
        }
    }
    progress(items.len());
    if skipped.is_empty() {
        Ok(())
    } else {
        Err(Error::Extension {
            ext: crate::ID,
            source: Box::new(NotRestored(t!(
                "zfs-not-restored",
                items = skipped.trim_end().to_owned()
            ))),
        })
    }
}

/// Creates the missing parents of `to` and copies `from` there.
fn put_back(from: &Path, to: &Path, cancel: &CancelToken) -> Result<(), Error> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(Error::from_io)?;
    }
    copy_tree(from, to, cancel)
}

/// Copies a file, a folder with everything in it, or a symbolic link as a
/// link, keeping permissions and modification times.
fn copy_tree(from: &Path, to: &Path, cancel: &CancelToken) -> Result<(), Error> {
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    let meta = from.symlink_metadata().map_err(Error::from_io)?;
    if meta.file_type().is_symlink() {
        let target = std::fs::read_link(from).map_err(Error::from_io)?;
        return pfs::symlink(&target, to).map_err(Error::from_io);
    }
    if meta.is_dir() {
        std::fs::create_dir(to).map_err(Error::from_io)?;
        for child in std::fs::read_dir(from).map_err(Error::from_io)? {
            let child = child.map_err(Error::from_io)?;
            copy_tree(&child.path(), &to.join(child.file_name()), cancel)?;
        }
        keep_mode(from, to);
        if let Ok(modified) = meta.modified()
            && let Err(error) = std::fs::File::open(to).and_then(|dir| dir.set_modified(modified))
        {
            tracing::debug!(path = %to.display(), %error, "folder date not kept");
        }
        return Ok(());
    }
    pfs::copy_file(from, to, false, &mut |_, _| !cancel.is_cancelled()).map_err(|e| {
        if e.kind() == std::io::ErrorKind::Interrupted {
            Error::Cancelled
        } else {
            Error::from_io(e)
        }
    })?;
    keep_mode(from, to);
    Ok(())
}

/// Gives `to` the Unix permission bits of `from`, where there are any.
fn keep_mode(from: &Path, to: &Path) {
    if let Ok(Some(mode)) = pfs::unix_mode(from)
        && let Err(error) = pfs::set_unix_mode(to, mode)
    {
        tracing::debug!(path = %to.display(), %error, "permissions not kept");
    }
}

/// Puts the selected entries of a snapshot or a versions view back.
#[derive(Debug)]
pub struct Restore(pub Arc<Shared>);

impl Restore {
    fn applies(ctx: &ActionCtx) -> bool {
        ctx.location.provider == PROVIDER_ID
            && matches!(
                target(&ctx.location),
                Ok(Target::Snapshot { .. } | Target::Versions(_))
            )
    }
}

impl Action for Restore {
    fn id(&self) -> ActionId {
        RESTORE
    }

    fn label(&self) -> String {
        t!("zfs-restore")
    }

    fn group(&self) -> ActionGroup {
        GROUP
    }

    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }

    fn availability(&self, ctx: &ActionCtx) -> Availability {
        if !Self::applies(ctx) || self.0.platform.is_none() {
            Availability::Hidden
        } else if ctx.selection.is_empty() {
            Availability::Disabled(t!("zfs-select-items-to-restore"))
        } else {
            Availability::Enabled
        }
    }

    fn run(
        &self,
        ctx: ActionCtx,
        jobs: &JobRunner,
        ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        let platform = self
            .0
            .platform
            .clone()
            .ok_or(Error::Unsupported("restoring needs the platform's trash"))?;
        let locations: Vec<Location> = ctx
            .selection
            .iter()
            .map(|e| Location::new(ctx.location.provider, e.path.clone()))
            .collect();
        let items = items(&self.0, &locations)?;
        let replaced: Vec<String> = items
            .iter()
            .filter(|i| i.to.symlink_metadata().is_ok())
            .map(|i| i.to.display().to_string())
            .collect();
        if !replaced.is_empty()
            && !ui.confirm(
                &t!("zfs-restore"),
                &t!("zfs-restore-replaces", items = replaced.join("\n")),
            )
        {
            return Ok(ActionOutcome::Done);
        }
        let name = t!("zfs-restoring", count = items.len());
        let handle = jobs.spawn_named(name, move |job| async move {
            let total = items.len() as u64;
            let progress = job.progress.clone();
            let cancel = job.cancel.clone();
            ext::blocking(move || {
                restore(&items, &*platform, &cancel, &mut |done| {
                    progress.report(Progress::Items {
                        done: done as u64,
                        total: Some(total),
                    });
                })
            })
            .await
        });
        Ok(ActionOutcome::Started(handle.id))
    }
}
