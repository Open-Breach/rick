//! Move or copy a set of entries into a folder: what a drag and drop runs,
//! and later Move to and Copy to.
//!
//! One job per transfer. A move goes through [`vfs::move_entry`], which is
//! the provider's `rename` within one provider and a copy followed by a
//! permanent removal across providers or devices; a copy goes through
//! [`vfs::copy`], the provider's fast path. A move is undoable, a copy is
//! not, as for Cut and Copy with Paste.

use std::sync::Arc;

use ext::ActionOutcome;
use runtime::JobRunner;
use types::{Error, Location};
use vfs::Mounts;

use crate::conflict::Conflicts;
use crate::edit::{MoveCommand, spawn_copies};
use crate::inside;
use crate::schedule::Scheduler;

/// Move or copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferKind {
    /// Rename within a provider, copy and remove across providers.
    Move,
    /// The provider's copy.
    Copy,
}

/// "3 items" or the one location's name.
fn describe(items: &[Location]) -> String {
    match items {
        [one] => one.path.file_name().map_or_else(
            || one.path.to_string(),
            |n| String::from_utf8_lossy(n).into_owned(),
        ),
        many => format!("{} items", many.len()),
    }
}

/// The last component of a folder's path, or the whole path at a root.
fn folder_name(folder: &Location) -> String {
    folder.path.file_name().map_or_else(
        || folder.path.to_string(),
        |n| String::from_utf8_lossy(n).into_owned(),
    )
}

/// Starts the job that moves or copies `sources` into `to`, each under its
/// own name, asking about existing names through `conflicts` and waiting
/// its turn on its volumes through `scheduler`. A move returns the undoable
/// command, already running; a copy returns the job it started.
///
/// # Errors
///
/// `Error::Unsupported` for an empty set of sources, or a source without a
/// name.
pub fn transfer(
    jobs: &JobRunner,
    mounts: Arc<Mounts>,
    conflicts: Arc<Conflicts>,
    scheduler: Arc<Scheduler>,
    sources: &[Location],
    to: &Location,
    kind: TransferKind,
) -> Result<ActionOutcome, Error> {
    if sources.is_empty() {
        return Err(Error::Unsupported("nothing to transfer"));
    }
    let mut pairs = Vec::with_capacity(sources.len());
    for source in sources {
        let name = source
            .path
            .file_name()
            .ok_or(Error::Unsupported("a root cannot be moved or copied"))?;
        pairs.push((source.clone(), inside(to, name)));
    }
    let what = describe(sources);
    let folder = folder_name(to);
    match kind {
        TransferKind::Move => {
            let command = MoveCommand {
                mounts,
                conflicts,
                scheduler,
                pairs,
                label: format!("Move {what} to {folder}"),
            };
            ext::UndoCommand::redo(&command, jobs)?;
            Ok(ActionOutcome::Undoable(Box::new(command)))
        }
        TransferKind::Copy => {
            let handle = spawn_copies(
                jobs,
                mounts,
                conflicts,
                scheduler,
                format!("Copy {what} to {folder}"),
                pairs,
            );
            Ok(ActionOutcome::Started(handle.id))
        }
    }
}
