//! Copy name, Copy path, Copy path and name, Open with the system default,
//! Open in the system file manager.

use std::sync::Arc;

use ext::{
    Action, ActionCtx, ActionGroup, ActionId, ActionOutcome, Availability, Key, Modifiers,
    Shortcut, UiHost,
};
use runtime::JobRunner;
use types::Error;

use crate::{State, is_trash, local_path, location_of, needs_entries};

/// The clipboard text for a selection: one line per item, `f` picking what
/// each item contributes; duplicates dropped when `dedup`.
fn lines(ctx: &ActionCtx, dedup: bool, f: impl Fn(&types::Entry) -> String) -> String {
    let mut out: Vec<String> = ctx.selection.iter().map(f).collect();
    if dedup {
        let mut seen = std::collections::BTreeSet::new();
        out.retain(|line| seen.insert(line.clone()));
    }
    out.join("\n")
}

/// The OS form of an entry's path.
fn full_path(entry: &types::Entry) -> String {
    let loc = location_of(entry);
    local_path(&loc).map_or_else(
        |_| loc.path.to_string_lossy().into_owned(),
        |path| path.display().to_string(),
    )
}

/// The OS form of the folder holding an entry.
fn folder_path(entry: &types::Entry) -> String {
    let loc = location_of(entry);
    if let Ok(path) = local_path(&loc) {
        return path
            .parent()
            .map_or_else(|| path.display().to_string(), |p| p.display().to_string());
    }
    let text = loc.path.to_string_lossy();
    text.rfind(['/', '\\'])
        .map_or_else(|| text.to_string(), |i| text[..i.max(1)].to_owned())
}

/// Copies the file name of each selected item, one per line (Mod+Alt+C).
pub(crate) struct CopyName(pub(crate) Arc<State>);

impl Action for CopyName {
    fn id(&self) -> ActionId {
        ActionId("core.copy_name")
    }
    fn label(&self) -> &'static str {
        "Copy name"
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        Some(Shortcut {
            modifiers: Modifiers::MOD | Modifiers::ALT,
            key: Key::Char('c'),
        })
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        needs_entries(ctx)
    }
    fn run(
        &self,
        ctx: ActionCtx,
        _jobs: &JobRunner,
        _ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        let text = lines(&ctx, false, |entry| entry.name.clone());
        self.0.platform.set_text(&text)?;
        Ok(ActionOutcome::Done)
    }
}

/// Copies the folder path of each selected item, without the name, one per
/// line with duplicates removed (Mod+Alt+Shift+C); with nothing selected,
/// the current folder's path.
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
            modifiers: Modifiers::MOD | Modifiers::ALT | Modifiers::SHIFT,
            key: Key::Char('c'),
        })
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        if is_trash(ctx) {
            Availability::Hidden
        } else {
            Availability::Enabled
        }
    }
    fn run(
        &self,
        ctx: ActionCtx,
        _jobs: &JobRunner,
        _ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        let text = if ctx.selection.is_empty() {
            local_path(&ctx.location).map_or_else(
                |_| ctx.location.path.to_string_lossy().into_owned(),
                |path| path.display().to_string(),
            )
        } else {
            lines(&ctx, true, folder_path)
        };
        self.0.platform.set_text(&text)?;
        Ok(ActionOutcome::Done)
    }
}

/// Copies the full path of each selected item, one per line (Mod+Shift+C).
pub(crate) struct CopyPathAndName(pub(crate) Arc<State>);

impl Action for CopyPathAndName {
    fn id(&self) -> ActionId {
        ActionId("core.copy_path_and_name")
    }
    fn label(&self) -> &'static str {
        "Copy path and name"
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
        needs_entries(ctx)
    }
    fn run(
        &self,
        ctx: ActionCtx,
        _jobs: &JobRunner,
        _ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        let text = lines(&ctx, false, full_path);
        self.0.platform.set_text(&text)?;
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
        needs_entries(ctx)
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

/// Shows the current location in the system file manager, with the first
/// selected entry selected when there is one.
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
        if crate::is_virtual(ctx) && ctx.selection.is_empty() || is_trash(ctx) {
            Availability::Hidden
        } else {
            Availability::Enabled
        }
    }
    fn run(
        &self,
        ctx: ActionCtx,
        _jobs: &JobRunner,
        _ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        match ctx.selection.first() {
            Some(entry) => self.0.platform.reveal(&local_path(&location_of(entry))?)?,
            None => self.0.platform.open(&local_path(&ctx.location)?)?,
        }
        Ok(ActionOutcome::Done)
    }
}
