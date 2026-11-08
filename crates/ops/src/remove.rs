//! Move to trash and Delete permanently.

use ext::t;
use std::sync::{Arc, Mutex, PoisonError};

use ext::{
    Action, ActionCtx, ActionGroup, ActionId, ActionOutcome, Availability, Capabilities, Key,
    Modifiers, RemoveMode, Shortcut, UiHost, UndoCommand,
};
use platform::TrashedItem;
use runtime::JobRunner;
use types::{Error, Location, Progress};

use crate::{State, describe, is_virtual, local_path, location_of, needs_entries};

/// Sends the selection to the platform trash; hidden where the provider
/// has no trash (an archive, the Trash view).
pub(crate) struct Trash(pub(crate) Arc<State>);

impl Action for Trash {
    fn id(&self) -> ActionId {
        ActionId("core.trash")
    }
    fn label(&self) -> String {
        t!("action-move-to-trash")
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        Some(Shortcut {
            modifiers: Modifiers::empty(),
            key: Key::Delete,
        })
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        // In a virtual view the entries are real files elsewhere: Favorites
        // and Recent trash them, the Trash view hides the action.
        if is_virtual(ctx) {
            return needs_entries(ctx);
        }
        match self.0.needs(&ctx.location, Capabilities::TRASH) {
            Availability::Enabled => needs_entries(ctx),
            hidden => hidden,
        }
    }
    fn run(
        &self,
        ctx: ActionCtx,
        jobs: &JobRunner,
        _ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        let command = TrashCommand {
            state: Arc::clone(&self.0),
            sources: ctx.selection.iter().map(location_of).collect(),
            trashed: Arc::new(Mutex::new(Vec::new())),
            label: format!("Move to trash {}", describe(&ctx.selection)),
        };
        command.redo(jobs)?;
        Ok(ActionOutcome::Undoable(Box::new(command)))
    }
}

/// Trashes `sources`; undo restores what the trash handed back.
struct TrashCommand {
    state: Arc<State>,
    sources: Vec<Location>,
    /// Filled by the trash job, drained by the restore job.
    trashed: Arc<Mutex<Vec<TrashedItem>>>,
    label: String,
}

impl UndoCommand for TrashCommand {
    fn undo(&self, jobs: &JobRunner) -> Result<(), Error> {
        let state = Arc::clone(&self.state);
        let trashed = Arc::clone(&self.trashed);
        jobs.spawn_named(format!("Undo {}", self.label), move |job| async move {
            let items: Vec<TrashedItem> = {
                let mut guard = trashed.lock().unwrap_or_else(PoisonError::into_inner);
                guard.drain(..).rev().collect()
            };
            let total = items.len() as u64;
            for (done, item) in items.iter().enumerate() {
                job.progress.report(Progress::Items {
                    done: done as u64,
                    total: Some(total),
                });
                let platform = Arc::clone(&state.platform);
                let item = item.clone();
                runtime::handle()
                    .spawn_blocking(move || platform.restore(&item))
                    .await
                    .map_err(|join| Error::Io(std::io::Error::other(join)))??;
            }
            Ok(())
        });
        Ok(())
    }

    fn redo(&self, jobs: &JobRunner) -> Result<(), Error> {
        let state = Arc::clone(&self.state);
        let sources = self.sources.clone();
        let trashed = Arc::clone(&self.trashed);
        jobs.spawn_named(self.label.clone(), move |job| async move {
            let total = sources.len() as u64;
            for (done, source) in sources.iter().enumerate() {
                job.progress.report(Progress::Items {
                    done: done as u64,
                    total: Some(total),
                });
                if job.cancel.is_cancelled() {
                    return Err(Error::Cancelled);
                }
                match local_path(source) {
                    // Through the platform directly, so the item can be
                    // restored; the provider's trash hides that handle.
                    Ok(path) => {
                        let platform = Arc::clone(&state.platform);
                        let item = runtime::handle()
                            .spawn_blocking(move || platform.trash(&path))
                            .await
                            .map_err(|join| Error::Io(std::io::Error::other(join)))??;
                        trashed
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .push(item);
                    }
                    Err(_) => {
                        state
                            .mounts
                            .resolve(source)?
                            .remove(source, RemoveMode::Trash)
                            .await?;
                    }
                }
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

/// Removes the selection for good, after confirmation.
pub(crate) struct Delete(pub(crate) Arc<State>);

impl Action for Delete {
    fn id(&self) -> ActionId {
        ActionId("core.delete")
    }
    fn label(&self) -> String {
        t!("action-delete-permanently")
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        Some(Shortcut {
            modifiers: Modifiers::SHIFT,
            key: Key::Delete,
        })
    }
    /// With a selection, in the Trash view too: a trashed item is deleted
    /// for good through the view's own `remove`.
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        if ctx.selection.is_empty() {
            Availability::Disabled(t!("reason-select-something-first"))
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
        let what = describe(&ctx.selection);
        if !ui.confirm(
            "Delete permanently",
            &format!("Delete {what}? This cannot be undone."),
        ) {
            return Ok(ActionOutcome::Done);
        }
        let mounts = Arc::clone(&self.0.mounts);
        let sources: Vec<Location> = ctx.selection.iter().map(location_of).collect();
        let handle = jobs.spawn_named(format!("Delete {what}"), move |job| async move {
            let total = sources.len() as u64;
            for (done, source) in sources.iter().enumerate() {
                job.progress.report(Progress::Items {
                    done: done as u64,
                    total: Some(total),
                });
                if job.cancel.is_cancelled() {
                    return Err(Error::Cancelled);
                }
                mounts
                    .resolve(source)?
                    .remove(source, RemoveMode::Permanent)
                    .await?;
            }
            job.progress.report(Progress::Items {
                done: total,
                total: Some(total),
            });
            Ok(())
        });
        Ok(ActionOutcome::Started(handle.id))
    }
}
