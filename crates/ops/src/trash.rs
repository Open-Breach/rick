//! Restore, Restore trash items, Empty trash and Delete permanently for the
//! trash: the actions behind the side panel's Trash section.
//!
//! Trash items are not entries of any provider in 0.1, so the section hands
//! the actions an [`ActionCtx`] whose location is [`trash_location`] and
//! whose selection is one [`trash_entry`] per item; the actions find the
//! [`TrashedItem`]s again through the [`TrashStore`] the application keeps,
//! which also holds Rick's own record of what it trashed where the platform
//! cannot list its trash (macOS).

use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use ext::{
    Action, ActionCtx, ActionGroup, ActionId, ActionOutcome, Availability, Shortcut, UiHost,
    UndoCommand,
};
use platform::TrashedItem;
use runtime::JobRunner;
use types::{Attrs, Entry, EntryId, EntryKind, Error, Location, Progress, ProviderId, VPath};

use crate::State;

/// The provider id of the trash's pseudo-location.
pub const TRASH_PROVIDER: ProviderId = ProviderId("trash");

/// The location an [`ActionCtx`] carries when it stands for the trash.
#[must_use]
pub fn trash_location() -> Location {
    Location::new(TRASH_PROVIDER, "/")
}

/// Whether a context stands for the trash.
#[must_use]
pub fn is_trash(ctx: &ActionCtx) -> bool {
    ctx.location.provider == TRASH_PROVIDER
}

/// The entry that stands for a trashed item in an [`ActionCtx`]: the
/// original name, the trashed path, the deletion time as `modified`.
#[must_use]
pub fn trash_entry(item: &TrashedItem) -> Entry {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    item.trashed.hash(&mut hasher);
    let key = u128::from(hasher.finish());
    let is_dir = std::fs::symlink_metadata(&item.trashed).is_ok_and(|m| m.is_dir());
    Entry {
        id: EntryId::new(TRASH_PROVIDER, key),
        name: original_name(item),
        path: VPath::from_os_str(item.trashed.as_os_str()),
        kind: if is_dir {
            EntryKind::Dir
        } else {
            EntryKind::File
        },
        size: None,
        created: None,
        modified: Some(item.deleted_at),
        accessed: None,
        attrs: Attrs::empty(),
        mime: None,
    }
}

/// The trash as the actions see it: what the platform lists plus Rick's own
/// record. The application owns and persists it.
pub trait TrashStore: Send + Sync {
    /// Every item known, newest first.
    fn items(&self) -> Vec<TrashedItem>;

    /// Rick trashed these; remember them.
    fn record(&self, items: &[TrashedItem]);

    /// These left the trash (restored, deleted, emptied); forget them.
    fn forget(&self, items: &[TrashedItem]);
}

/// The items a trash context names, in selection order.
fn items_of(store: &dyn TrashStore, ctx: &ActionCtx) -> Vec<TrashedItem> {
    let known = store.items();
    ctx.selection
        .iter()
        .filter_map(|entry| {
            let trashed: PathBuf = platform::paths::to_path(&entry.path).ok()?;
            known.iter().find(|item| item.trashed == trashed).cloned()
        })
        .collect()
}

/// The name a trashed item had where it came from.
fn original_name(item: &TrashedItem) -> String {
    item.original.file_name().map_or_else(
        || item.original.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

fn describe_items(items: &[TrashedItem]) -> String {
    match items {
        [one] => original_name(one),
        many => format!("{} items", many.len()),
    }
}

/// Puts trashed items back; undo trashes them again. One command serves
/// Restore and Restore trash items.
pub(crate) struct RestoreCommand {
    state: Arc<State>,
    /// The items as they are in the trash now; an undo that trashes them
    /// anew writes their new trash paths back here for the next redo.
    items: Arc<Mutex<Vec<TrashedItem>>>,
    label: String,
}

impl RestoreCommand {
    fn current(&self) -> Vec<TrashedItem> {
        self.items
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl UndoCommand for RestoreCommand {
    /// Trashes the restored entries again and remembers where they went.
    fn undo(&self, jobs: &JobRunner) -> Result<(), Error> {
        let state = Arc::clone(&self.state);
        let items = self.current();
        let slot = Arc::clone(&self.items);
        jobs.spawn_named(format!("Undo {}", self.label), move |job| async move {
            let total = items.len() as u64;
            let mut again = Vec::with_capacity(items.len());
            for (done, item) in items.iter().enumerate() {
                job.progress.report(Progress::Items {
                    done: done as u64,
                    total: Some(total),
                });
                let platform = Arc::clone(&state.platform);
                let original = item.original.clone();
                let trashed = runtime::handle()
                    .spawn_blocking(move || platform.trash(&original))
                    .await
                    .map_err(|join| Error::Io(std::io::Error::other(join)))??;
                again.push(trashed);
            }
            state.trash.record(&again);
            *slot.lock().unwrap_or_else(PoisonError::into_inner) = again;
            Ok(())
        });
        Ok(())
    }

    fn redo(&self, jobs: &JobRunner) -> Result<(), Error> {
        let state = Arc::clone(&self.state);
        let items = self.current();
        jobs.spawn_named(self.label.clone(), move |job| async move {
            let total = items.len() as u64;
            for (done, item) in items.iter().enumerate() {
                job.progress.report(Progress::Items {
                    done: done as u64,
                    total: Some(total),
                });
                if job.cancel.is_cancelled() {
                    return Err(Error::Cancelled);
                }
                let platform = Arc::clone(&state.platform);
                let item = item.clone();
                runtime::handle()
                    .spawn_blocking(move || platform.restore(&item))
                    .await
                    .map_err(|join| Error::Io(std::io::Error::other(join)))??;
            }
            state.trash.forget(&items);
            job.progress.report(Progress::Items {
                done: total,
                total: Some(total),
            });
            Ok(())
        });
        Ok(())
    }

    fn label(&self) -> String {
        self.label.clone()
    }
}

/// Starts restoring `items` and returns the undoable command; the single
/// Restore and Restore trash items both come here.
///
/// # Errors
///
/// `Error::Unsupported` for an empty set.
pub(crate) fn restore(
    state: &Arc<State>,
    jobs: &JobRunner,
    items: Vec<TrashedItem>,
) -> Result<ActionOutcome, Error> {
    if items.is_empty() {
        return Err(Error::Unsupported("nothing to restore"));
    }
    let command = RestoreCommand {
        state: Arc::clone(state),
        label: format!("Restore {}", describe_items(&items)),
        items: Arc::new(Mutex::new(items)),
    };
    command.redo(jobs)?;
    Ok(ActionOutcome::Undoable(Box::new(command)))
}

/// Puts one trashed item back where it came from; the single-item case of
/// Restore trash items.
pub(crate) struct Restore(pub(crate) Arc<State>);

impl Action for Restore {
    fn id(&self) -> ActionId {
        ActionId("core.restore")
    }
    fn label(&self) -> &'static str {
        "Restore"
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        if !is_trash(ctx) {
            return Availability::Hidden;
        }
        match ctx.selection.len() {
            1 => Availability::Enabled,
            0 => Availability::Disabled("select a trashed item first"),
            _ => Availability::Disabled("Restore trash items puts several back"),
        }
    }
    fn run(
        &self,
        ctx: ActionCtx,
        jobs: &JobRunner,
        _ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        let items = items_of(&*self.0.trash, &ctx);
        restore(&self.0, jobs, items)
    }
}

/// Puts the selected trashed items back where they came from.
pub(crate) struct RestoreTrash(pub(crate) Arc<State>);

impl Action for RestoreTrash {
    fn id(&self) -> ActionId {
        ActionId("core.restore_trash")
    }
    fn label(&self) -> &'static str {
        "Restore trash items"
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        if !is_trash(ctx) {
            Availability::Hidden
        } else if ctx.selection.is_empty() {
            Availability::Disabled("select trashed items first")
        } else {
            Availability::Enabled
        }
    }
    fn run(
        &self,
        ctx: ActionCtx,
        jobs: &JobRunner,
        _ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        let items = items_of(&*self.0.trash, &ctx);
        restore(&self.0, jobs, items)
    }
}

/// Deletes everything in the trash for good, after confirmation.
pub(crate) struct EmptyTrash(pub(crate) Arc<State>);

impl Action for EmptyTrash {
    fn id(&self) -> ActionId {
        ActionId("core.empty_trash")
    }
    fn label(&self) -> &'static str {
        "Empty trash"
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }
    fn availability(&self, _ctx: &ActionCtx) -> Availability {
        Availability::Enabled
    }
    fn run(
        &self,
        _ctx: ActionCtx,
        jobs: &JobRunner,
        ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        if !ui.confirm(
            "Empty trash",
            "Delete everything in the trash? This cannot be undone.",
        ) {
            return Ok(ActionOutcome::Done);
        }
        let state = Arc::clone(&self.0);
        let handle = jobs.spawn_named("Empty trash", move |job| async move {
            job.progress
                .report(Progress::Message("Emptying the trash".into()));
            let platform = Arc::clone(&state.platform);
            let known = state.trash.items();
            runtime::handle()
                .spawn_blocking(move || platform.empty())
                .await
                .map_err(|join| Error::Io(std::io::Error::other(join)))??;
            state.trash.forget(&known);
            Ok(())
        });
        Ok(ActionOutcome::Started(handle.id))
    }
}

/// Deletes the selected trashed items for good, after confirmation.
pub(crate) struct PurgeTrash(pub(crate) Arc<State>);

impl Action for PurgeTrash {
    fn id(&self) -> ActionId {
        ActionId("core.purge_trash")
    }
    fn label(&self) -> &'static str {
        "Delete permanently"
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        if !is_trash(ctx) {
            Availability::Hidden
        } else if ctx.selection.is_empty() {
            Availability::Disabled("select trashed items first")
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
        let items = items_of(&*self.0.trash, &ctx);
        if items.is_empty() {
            return Err(Error::Unsupported("nothing to delete"));
        }
        let what = describe_items(&items);
        if !ui.confirm(
            "Delete permanently",
            &format!("Delete {what} from the trash? This cannot be undone."),
        ) {
            return Ok(ActionOutcome::Done);
        }
        let state = Arc::clone(&self.0);
        let handle = jobs.spawn_named(format!("Delete {what}"), move |job| async move {
            let total = items.len() as u64;
            for (done, item) in items.iter().enumerate() {
                job.progress.report(Progress::Items {
                    done: done as u64,
                    total: Some(total),
                });
                if job.cancel.is_cancelled() {
                    return Err(Error::Cancelled);
                }
                let platform = Arc::clone(&state.platform);
                let item = item.clone();
                runtime::handle()
                    .spawn_blocking(move || platform.remove(&item))
                    .await
                    .map_err(|join| Error::Io(std::io::Error::other(join)))??;
            }
            state.trash.forget(&items);
            Ok(())
        });
        Ok(ActionOutcome::Started(handle.id))
    }
}
