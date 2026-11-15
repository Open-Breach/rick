//! *Set as desktop background* (SPEC.md, under Functions): the desktop of
//! every screen is pointed at the one selected image, where it lies.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ext::{
    Action, ActionCtx, ActionGroup, ActionId, ActionOutcome, Availability, Shortcut, UiHost, t,
};
use runtime::JobRunner;
use types::{EntryKind, Error};

use crate::{State, local_path, location_of};

/// Set as desktop background: for exactly one selected image file on the
/// local file system, in a format this desktop takes. Hidden otherwise,
/// and on a desktop whose background Rick cannot set, which takes none.
pub(crate) struct SetDesktopBackground(pub(crate) Arc<State>);

impl SetDesktopBackground {
    /// The file the action is for, when the selection is one.
    fn image(&self, ctx: &ActionCtx) -> Option<PathBuf> {
        let [entry] = ctx.selection.as_slice() else {
            return None;
        };
        if entry.kind != EntryKind::File {
            return None;
        }
        let extension = Path::new(&entry.name).extension()?.to_str()?.to_lowercase();
        self.0
            .platform
            .background_formats()
            .contains(&extension.as_str())
            .then(|| local_path(&location_of(entry)).ok())
            .flatten()
    }
}

impl Action for SetDesktopBackground {
    fn id(&self) -> ActionId {
        ActionId("core.set_desktop_background")
    }
    fn label(&self) -> String {
        t!("action-set-desktop-background")
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        if self.image(ctx).is_some() {
            Availability::Enabled
        } else {
            Availability::Hidden
        }
    }
    /// The system's call is short and macOS wants it on the main thread,
    /// which the application's thread is: it is made here, not in a job.
    fn run(
        &self,
        ctx: ActionCtx,
        _jobs: &JobRunner,
        _ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        let image = self
            .image(&ctx)
            .ok_or(Error::Unsupported("not an image the desktop takes"))?;
        self.0.platform.set_desktop_background(&image)?;
        Ok(ActionOutcome::Done)
    }
}
