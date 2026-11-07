//! Pin to side panel and Unpin.
//!
//! The pinned list itself belongs to the application's settings; the actions
//! see it through [`PinStore`], which the application implements. Both
//! actions are undoable: undoing a pin removes what it added, undoing an
//! unpin puts the entry back where it was.

use std::sync::Arc;

use ext::{
    Action, ActionCtx, ActionGroup, ActionId, ActionOutcome, Availability, Shortcut, UiHost,
    UndoCommand,
};
use runtime::JobRunner;
use types::{Error, Location};

use crate::{State, describe, is_trash, location_of};

/// The side panel's Pinned list, as the pin actions see it. The application
/// owns the list and persists it; it hands the actions this view.
pub trait PinStore: Send + Sync {
    /// The pinned locations in display order.
    fn pinned(&self) -> Vec<Location>;

    /// Inserts `items` at `at`, or at the end; locations already pinned
    /// stay where they are.
    fn pin(&self, items: Vec<Location>, at: Option<usize>);

    /// Removes a location.
    fn unpin(&self, item: &Location);
}

/// The last component of a location's path, for labels.
fn name_of(loc: &Location) -> String {
    loc.path.file_name().map_or_else(
        || loc.path.to_string(),
        |n| String::from_utf8_lossy(n).into_owned(),
    )
}

/// "3 items" or the one location's name.
fn describe_locations(items: &[Location]) -> String {
    match items {
        [one] => name_of(one),
        many => format!("{} items", many.len()),
    }
}

/// Pins `items`, or redoes it; undo removes them again. Built by the Pin
/// action and by a drop onto the Pinned section.
#[must_use]
pub fn pin_command(
    store: Arc<dyn PinStore>,
    items: Vec<Location>,
    at: Option<usize>,
) -> Box<dyn UndoCommand> {
    let already = store.pinned();
    let items: Vec<Location> = items
        .into_iter()
        .filter(|item| !already.contains(item))
        .collect();
    Box::new(PinCommand {
        label: format!("Pin {}", describe_locations(&items)),
        store,
        items,
        at,
    })
}

/// Unpins `item`, or redoes it; undo pins it again at its old index. Built
/// by the Unpin action and by the Pinned entry's context menu.
#[must_use]
pub fn unpin_command(store: Arc<dyn PinStore>, item: Location) -> Box<dyn UndoCommand> {
    let index = store.pinned().iter().position(|p| *p == item);
    Box::new(UnpinCommand {
        label: format!("Unpin {}", name_of(&item)),
        store,
        item,
        index,
    })
}

struct PinCommand {
    store: Arc<dyn PinStore>,
    items: Vec<Location>,
    at: Option<usize>,
    label: String,
}

impl UndoCommand for PinCommand {
    fn undo(&self, _jobs: &JobRunner) -> Result<(), Error> {
        for item in &self.items {
            self.store.unpin(item);
        }
        Ok(())
    }

    fn redo(&self, _jobs: &JobRunner) -> Result<(), Error> {
        self.store.pin(self.items.clone(), self.at);
        Ok(())
    }

    fn label(&self) -> String {
        self.label.clone()
    }
}

struct UnpinCommand {
    store: Arc<dyn PinStore>,
    item: Location,
    index: Option<usize>,
    label: String,
}

impl UndoCommand for UnpinCommand {
    fn undo(&self, _jobs: &JobRunner) -> Result<(), Error> {
        self.store.pin(vec![self.item.clone()], self.index);
        Ok(())
    }

    fn redo(&self, _jobs: &JobRunner) -> Result<(), Error> {
        self.store.unpin(&self.item);
        Ok(())
    }

    fn label(&self) -> String {
        self.label.clone()
    }
}

/// Whether every selected entry is pinned already; `false` for an empty
/// selection.
fn all_pinned(state: &State, ctx: &ActionCtx) -> bool {
    if ctx.selection.is_empty() {
        return false;
    }
    let pinned = state.pins.pinned();
    ctx.selection
        .iter()
        .all(|entry| pinned.contains(&location_of(entry)))
}

/// Adds the selection to the Pinned section, at the end.
pub(crate) struct Pin(pub(crate) Arc<State>);

impl Action for Pin {
    fn id(&self) -> ActionId {
        ActionId("core.pin")
    }
    fn label(&self) -> &'static str {
        "Pin to side panel"
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        if is_trash(ctx) {
            Availability::Hidden
        } else if ctx.selection.is_empty() {
            Availability::Disabled("select something first")
        } else if all_pinned(&self.0, ctx) {
            // Unpin takes its place.
            Availability::Hidden
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
        let items: Vec<Location> = ctx.selection.iter().map(location_of).collect();
        let command = pin_command(Arc::clone(&self.0.pins), items, None);
        command.redo(jobs)?;
        Ok(ActionOutcome::Undoable(command))
    }
}

/// Removes the selection from the Pinned section; shown in place of Pin when
/// everything selected is pinned already.
pub(crate) struct Unpin(pub(crate) Arc<State>);

impl Action for Unpin {
    fn id(&self) -> ActionId {
        ActionId("core.unpin")
    }
    fn label(&self) -> &'static str {
        "Unpin"
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        if !is_trash(ctx) && all_pinned(&self.0, ctx) {
            Availability::Enabled
        } else {
            Availability::Hidden
        }
    }
    fn run(
        &self,
        ctx: ActionCtx,
        jobs: &JobRunner,
        _ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        let items: Vec<Location> = ctx.selection.iter().map(location_of).collect();
        let command: Box<dyn UndoCommand> = match items.as_slice() {
            [one] => unpin_command(Arc::clone(&self.0.pins), one.clone()),
            many => Box::new(UnpinManyCommand {
                label: format!("Unpin {}", describe(&ctx.selection)),
                commands: many
                    .iter()
                    .map(|item| unpin_command(Arc::clone(&self.0.pins), item.clone()))
                    .collect(),
            }),
        };
        command.redo(jobs)?;
        Ok(ActionOutcome::Undoable(command))
    }
}

/// Several unpins as one undo step; undone last to first so the indices
/// the single commands remembered still hold.
struct UnpinManyCommand {
    commands: Vec<Box<dyn UndoCommand>>,
    label: String,
}

impl UndoCommand for UnpinManyCommand {
    fn undo(&self, jobs: &JobRunner) -> Result<(), Error> {
        for command in self.commands.iter().rev() {
            command.undo(jobs)?;
        }
        Ok(())
    }

    fn redo(&self, jobs: &JobRunner) -> Result<(), Error> {
        for command in &self.commands {
            command.redo(jobs)?;
        }
        Ok(())
    }

    fn label(&self) -> String {
        self.label.clone()
    }
}
