//! Copy path, Open with the system default, Open in the system file manager.

use std::sync::Arc;

use ext::{
    Action, ActionCtx, ActionGroup, ActionId, ActionOutcome, Availability, Key, Modifiers,
    Shortcut, UiHost,
};
use runtime::JobRunner;
use types::Error;

use crate::{State, local_path, location_of};

fn needs_selection(ctx: &ActionCtx) -> Availability {
    if ctx.selection.is_empty() {
        Availability::Disabled("select something first")
    } else {
        Availability::Enabled
    }
}

/// Puts the selection's full paths on the system clipboard, one per line.
pub(crate) struct CopyPath(pub(crate) Arc<State>);

impl Action for CopyPath {
    fn id(&self) -> ActionId {
        ActionId("core.copy_path")
    }
    fn label(&self) -> &'static str {
        "Copy path"
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        Some(Shortcut {
            modifiers: Modifiers::MOD | Modifiers::SHIFT,
            key: Key::Char('c'),
        })
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        needs_selection(ctx)
    }
    fn run(
        &self,
        ctx: ActionCtx,
        _jobs: &JobRunner,
        _ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        let lines: Vec<String> = ctx
            .selection
            .iter()
            .map(|entry| {
                let loc = location_of(entry);
                local_path(&loc).map_or_else(
                    |_| loc.path.to_string_lossy().into_owned(),
                    |path| path.display().to_string(),
                )
            })
            .collect();
        self.0.platform.set_text(&lines.join("\n"))?;
        Ok(ActionOutcome::Done)
    }
}

/// Opens each selected entry with the system default application.
pub(crate) struct OpenDefault(pub(crate) Arc<State>);

impl Action for OpenDefault {
    fn id(&self) -> ActionId {
        ActionId("core.open_default")
    }
    fn label(&self) -> &'static str {
        "Open with system default"
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        Some(Shortcut {
            modifiers: Modifiers::MOD,
            key: Key::Enter,
        })
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        needs_selection(ctx)
    }
    fn run(
        &self,
        ctx: ActionCtx,
        _jobs: &JobRunner,
        _ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        for entry in &ctx.selection {
            let path = local_path(&location_of(entry))?;
            self.0.platform.open(&path)?;
        }
        Ok(ActionOutcome::Done)
    }
}

/// Shows the first selected entry in the system file manager.
pub(crate) struct Reveal(pub(crate) Arc<State>);

impl Action for Reveal {
    fn id(&self) -> ActionId {
        ActionId("core.reveal")
    }
    fn label(&self) -> &'static str {
        "Open in system file manager"
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        Some(Shortcut {
            modifiers: Modifiers::MOD | Modifiers::SHIFT,
            key: Key::Char('o'),
        })
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        needs_selection(ctx)
    }
    fn run(
        &self,
        ctx: ActionCtx,
        _jobs: &JobRunner,
        _ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        let Some(entry) = ctx.selection.first() else {
            return Ok(ActionOutcome::Done);
        };
        let path = local_path(&location_of(entry))?;
        self.0.platform.reveal(&path)?;
        Ok(ActionOutcome::Done)
    }
}
