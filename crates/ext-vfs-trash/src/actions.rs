//! Restore, Restore trash items and Empty trash: the "Trash" action group,
//! available in the view only. Delete permanently is the core action, which
//! goes through the provider's `remove`.

use std::sync::{Arc, Mutex, PoisonError};

use ext::platform::{Trash, TrashedItem};
use ext::{
    Action, ActionCtx, ActionGroup, ActionId, ActionOutcome, Availability, JobRunner, Shortcut,
    UiHost, UndoCommand,
};
use types::{Error, Progress};

use crate::TrashProvider;

/// Puts one trashed item back where it came from.
pub const RESTORE: ActionId = ActionId("trash.restore");
/// Puts the selected trashed items back where they came from.
pub const RESTORE_ITEMS: ActionId = ActionId("trash.restore_items");
/// Deletes everything in the trash for good, after confirmation.
pub const EMPTY: ActionId = ActionId("trash.empty");

const GROUP: ActionGroup = ActionGroup::Extension("Trash");

/// The three actions over `provider`.
pub fn all(provider: &Arc<TrashProvider>) -> Vec<Arc<dyn Action>> {
    vec![
        Arc::new(Restore(Arc::clone(provider))),
        Arc::new(RestoreItems(Arc::clone(provider))),
        Arc::new(Empty(Arc::clone(provider))),
    ]
}

/// Whether the context is the view itself: the actions are hidden anywhere
/// else, inside a trashed folder included.
fn in_view(ctx: &ActionCtx) -> bool {
    crate::is_view(&ctx.location)
}

/// The name an item had where it came from.
fn original_name(item: &TrashedItem) -> String {
    item.original.file_name().map_or_else(
        || item.original.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

fn describe(items: &[TrashedItem]) -> String {
    match items {
        [one] => original_name(one),
        many => format!("{} items", many.len()),
    }
}

/// Runs a platform call on the blocking pool.
async fn on_platform<T: Send + 'static>(
    trash: &Arc<dyn Trash>,
    call: impl FnOnce(&dyn Trash) -> Result<T, Error> + Send + 'static,
) -> Result<T, Error> {
    let trash = Arc::clone(trash);
    ext::blocking(move || call(&*trash)).await
}

/// Puts trashed items back; undo trashes them again. One command serves
/// Restore and Restore trash items.
struct RestoreCommand {
    trash: Arc<dyn Trash>,
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
        let trash = Arc::clone(&self.trash);
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
                let original = item.original.clone();
                again.push(on_platform(&trash, move |t| t.trash(&original)).await?);
            }
            *slot.lock().unwrap_or_else(PoisonError::into_inner) = again;
            Ok(())
        });
        Ok(())
    }

    fn redo(&self, jobs: &JobRunner) -> Result<(), Error> {
        let trash = Arc::clone(&self.trash);
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
                let item = item.clone();
                on_platform(&trash, move |t| t.restore(&item)).await?;
            }
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

/// Starts restoring `items` and returns the undoable command.
fn restore(
    provider: &TrashProvider,
    jobs: &JobRunner,
    ctx: &ActionCtx,
) -> Result<ActionOutcome, Error> {
    let items = provider.items_of(&ctx.selection);
    if items.is_empty() {
        return Err(Error::Unsupported("nothing to restore"));
    }
    let command = RestoreCommand {
        trash: Arc::clone(provider.trash()),
        label: format!("Restore {}", describe(&items)),
        items: Arc::new(Mutex::new(items)),
    };
    command.redo(jobs)?;
    Ok(ActionOutcome::Undoable(Box::new(command)))
}

/// Puts one trashed item back where it came from; the single-item case of
/// Restore trash items.
struct Restore(Arc<TrashProvider>);

impl Action for Restore {
    fn id(&self) -> ActionId {
        RESTORE
    }
    fn label(&self) -> &'static str {
        "Restore"
    }
    fn group(&self) -> ActionGroup {
        GROUP
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        if !in_view(ctx) {
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
        restore(&self.0, jobs, &ctx)
    }
}

/// Puts the selected trashed items back where they came from.
struct RestoreItems(Arc<TrashProvider>);

impl Action for RestoreItems {
    fn id(&self) -> ActionId {
        RESTORE_ITEMS
    }
    fn label(&self) -> &'static str {
        "Restore trash items"
    }
    fn group(&self) -> ActionGroup {
        GROUP
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        if !in_view(ctx) {
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
        restore(&self.0, jobs, &ctx)
    }
}

/// Deletes everything in the trash for good, after confirmation.
struct Empty(Arc<TrashProvider>);

impl Action for Empty {
    fn id(&self) -> ActionId {
        EMPTY
    }
    fn label(&self) -> &'static str {
        "Empty trash"
    }
    fn group(&self) -> ActionGroup {
        GROUP
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        if !in_view(ctx) {
            Availability::Hidden
        } else if self.0.is_empty() {
            Availability::Disabled("the trash is empty")
        } else {
            Availability::Enabled
        }
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
        let trash = Arc::clone(self.0.trash());
        let handle = jobs.spawn_named("Empty trash", move |job| async move {
            job.progress
                .report(Progress::Message("Emptying the trash".into()));
            on_platform(&trash, |t| t.empty()).await
        });
        Ok(ActionOutcome::Started(handle.id))
    }
}
