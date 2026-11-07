//! The file operations of version 0.1 as [`Action`](ext::Action)s.
//!
//! [`Core`] is the built-in [`Extension`] that registers them: copy, cut and
//! paste, duplicate, move to trash, delete permanently, rename, new file, new
//! folder, copy path, open with the system default, open in the system file
//! manager, open containing folder, restore, restore trash items, empty
//! trash, pin to side panel and unpin. Navigation, selection and view
//! commands are `Command`s the application handles directly, as is a drag
//! and drop's [`transfer`], which is not an action but runs the same jobs.
//!
//! Every operation that touches the filesystem runs as a job on the
//! [`JobRunner`](runtime::JobRunner) the action receives, so `run` returns at
//! once. Reversible operations (move, rename, trash, restore, new file, new
//! folder) return [`ActionOutcome::Undoable`](ext::ActionOutcome::Undoable)
//! with a command whose `undo` and `redo` spawn the reverse and forward jobs;
//! copies and permanent deletes are not undoable, as the spec says, and
//! return [`ActionOutcome::Started`](ext::ActionOutcome::Started).
//!
//! A copy or move that would land on an existing name asks through
//! [`Conflicts`] — skip, overwrite, rename, apply to all, cancel — and waits
//! for the answer; the application owns the queue and the UI shows the
//! dialogue.

#![forbid(unsafe_code)]

mod clipboard;
mod conflict;
mod containing;
mod create;
mod edit;
mod open;
mod pin;
mod remove;
mod transfer;
mod trash;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use ext::{API_VERSION, Capabilities, Extension, Manifest, Platforms, Points, Registry, Version};
use platform::Platform;
use types::{Entry, Error, ExtId, Location};
use vfs::Mounts;

pub use clipboard::{ClipMode, Clipboard};
pub use conflict::{ConflictWaker, Conflicts};
pub use pin::{PinStore, pin_command, unpin_command};
pub use transfer::{TransferKind, transfer};
pub use trash::{TRASH_PROVIDER, TrashStore, is_trash, trash_entry, trash_location};

/// The built-in extension holding the core operations.
pub struct Core {
    manifest: Manifest,
    state: Arc<State>,
}

/// What every action shares.
struct State {
    mounts: Arc<Mounts>,
    platform: Arc<dyn Platform>,
    pins: Arc<dyn PinStore>,
    trash: Arc<dyn TrashStore>,
    conflicts: Arc<Conflicts>,
    // Actions run on the app thread one at a time; the lock is for the
    // `Send + Sync` bound, not for contention.
    clipboard: Mutex<Option<Clipboard>>,
}

impl State {
    fn clipboard(&self) -> MutexGuard<'_, Option<Clipboard>> {
        self.clipboard
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

impl Core {
    /// The core operations over `mounts`, trashing and opening through
    /// `platform`, pinning into `pins`, remembering trashed items in
    /// `trash`, asking about name conflicts through `conflicts`.
    #[must_use]
    pub fn new(
        mounts: Arc<Mounts>,
        platform: Arc<dyn Platform>,
        pins: Arc<dyn PinStore>,
        trash: Arc<dyn TrashStore>,
        conflicts: Arc<Conflicts>,
    ) -> Self {
        Self {
            manifest: Manifest {
                id: ExtId("core"),
                name: "Core operations",
                version: Version::new(0, 1, 0),
                api_version: API_VERSION,
                requires: Capabilities::WRITE,
                points: Points::ACTION,
                platforms: Platforms::ALL,
            },
            state: Arc::new(State {
                mounts,
                platform,
                pins,
                trash,
                conflicts,
                clipboard: Mutex::new(None),
            }),
        }
    }

    /// What Copy or Cut put on the clipboard, if anything.
    #[must_use]
    pub fn clipboard(&self) -> Option<Clipboard> {
        self.state.clipboard().clone()
    }
}

impl std::fmt::Debug for Core {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Core")
            .field("manifest", &self.manifest)
            .finish_non_exhaustive()
    }
}

impl Extension for Core {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        let s = &self.state;
        reg.action(Arc::new(edit::Copy(Arc::clone(s))));
        reg.action(Arc::new(edit::Cut(Arc::clone(s))));
        reg.action(Arc::new(edit::Paste(Arc::clone(s))));
        reg.action(Arc::new(edit::Duplicate(Arc::clone(s))));
        reg.action(Arc::new(remove::Trash(Arc::clone(s))));
        reg.action(Arc::new(remove::Delete(Arc::clone(s))));
        reg.action(Arc::new(create::Rename(Arc::clone(s))));
        reg.action(Arc::new(create::NewFile(Arc::clone(s))));
        reg.action(Arc::new(create::NewFolder(Arc::clone(s))));
        reg.action(Arc::new(open::CopyName(Arc::clone(s))));
        reg.action(Arc::new(open::CopyPath(Arc::clone(s))));
        reg.action(Arc::new(open::CopyPathAndName(Arc::clone(s))));
        reg.action(Arc::new(open::OpenDefault(Arc::clone(s))));
        reg.action(Arc::new(open::Reveal(Arc::clone(s))));
        reg.action(Arc::new(containing::OpenContaining));
        reg.action(Arc::new(containing::OpenContainingInTab));
        reg.action(Arc::new(trash::Restore(Arc::clone(s))));
        reg.action(Arc::new(trash::RestoreTrash(Arc::clone(s))));
        reg.action(Arc::new(trash::EmptyTrash(Arc::clone(s))));
        reg.action(Arc::new(trash::PurgeTrash(Arc::clone(s))));
        reg.action(Arc::new(pin::Pin(Arc::clone(s))));
        reg.action(Arc::new(pin::Unpin(Arc::clone(s))));
        Ok(())
    }
}

/// The location of an entry.
fn location_of(entry: &Entry) -> Location {
    Location::new(entry.id.provider, entry.path.clone())
}

/// The last path component of an entry, falling back to its display name.
fn file_name(entry: &Entry) -> Vec<u8> {
    entry
        .path
        .file_name()
        .map_or_else(|| entry.name.as_bytes().to_vec(), <[u8]>::to_vec)
}

/// `name` inside `dir`.
fn inside(dir: &Location, name: &[u8]) -> Location {
    Location::new(dir.provider, dir.path.join(name))
}

/// The OS path of a local location; other providers have none.
fn local_path(loc: &Location) -> Result<PathBuf, Error> {
    if loc.provider != vfs_local::PROVIDER_ID {
        return Err(Error::Unsupported("only local entries can be used here"));
    }
    platform::paths::to_path(&loc.path)
}

/// "3 items" or the one entry's name.
fn describe(entries: &[Entry]) -> String {
    match entries {
        [one] => one.name.clone(),
        many => format!("{} items", many.len()),
    }
}

#[cfg(test)]
mod tests;
