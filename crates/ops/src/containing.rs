//! Open containing folder, in this tab or a new one.
//!
//! The action navigates the pane and selects the entry, which an
//! [`ActionOutcome`] cannot express; the UI recognises the two ids and does
//! the navigation itself (QUESTIONS.md 19), so `run` here only confirms the
//! action was reachable. Registering them keeps them in the menus, the
//! palette and the shortcut table like every other operation.

use ext::t;
use ext::{
    Action, ActionCtx, ActionGroup, ActionId, ActionOutcome, Availability, Shortcut, UiHost,
};
use runtime::JobRunner;
use types::Error;

use crate::edit::parent;

/// Available when something is selected whose folder is not the pane's
/// location: search results, the flattened view, Recent.
fn elsewhere(ctx: &ActionCtx) -> Availability {
    if ctx.selection.is_empty() {
        return Availability::Disabled(t!("reason-select-an-entry-first"));
    }
    let here = ctx
        .selection
        .iter()
        .all(|entry| parent(&entry.path) == ctx.location.path);
    if here {
        Availability::Disabled(t!("reason-selection-in-this-folder"))
    } else {
        Availability::Enabled
    }
}

/// Navigates the pane to the folder holding the selected entry and selects
/// it there.
pub(crate) struct OpenContaining;

impl Action for OpenContaining {
    fn id(&self) -> ActionId {
        ActionId("core.open_containing")
    }
    fn label(&self) -> String {
        t!("action-open-containing-folder")
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        elsewhere(ctx)
    }
    fn run(
        &self,
        _ctx: ActionCtx,
        _jobs: &JobRunner,
        _ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        Ok(ActionOutcome::Done)
    }
}

/// The same, in a new tab.
pub(crate) struct OpenContainingInTab;

impl Action for OpenContainingInTab {
    fn id(&self) -> ActionId {
        ActionId("core.open_containing_tab")
    }
    fn label(&self) -> String {
        t!("action-open-containing-folder-in-new-tab")
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        elsewhere(ctx)
    }
    fn run(
        &self,
        _ctx: ActionCtx,
        _jobs: &JobRunner,
        _ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        Ok(ActionOutcome::Done)
    }
}
