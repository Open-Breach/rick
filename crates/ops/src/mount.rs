//! Mount: a disk image handed to the operating system's own mount
//! mechanism through [`platform::Mount`] (SPEC.md "Disk images and
//! Mount"). The action shows on a selection of disk-image files whose
//! formats this OS mounts natively and nowhere else; the mounted image
//! appears as a removable volume in the side panel's This Machine through
//! the live volume watch, and that row's *Eject* unmounts it, so there is
//! no Unmount action.

use ext::t;
use std::path::PathBuf;
use std::sync::Arc;

use ext::{
    Action, ActionCtx, ActionGroup, ActionId, ActionOutcome, Availability, Shortcut, UiHost,
};
use platform::ImageFormat;
use runtime::JobRunner;
use types::{EntryKind, Error, Progress};

use crate::{State, describe, is_trash, local_path, location_of};

/// Mounts the selected disk images through the OS, one job for the
/// selection.
pub(crate) struct MountImage(pub(crate) Arc<State>);

impl MountImage {
    /// Whether every selected entry is a local disk-image file of a
    /// format this OS mounts.
    fn all_mountable(&self, ctx: &ActionCtx) -> bool {
        let mountable = self.0.platform.mountable();
        !ctx.selection.is_empty()
            && ctx.selection.iter().all(|entry| {
                entry.kind == EntryKind::File
                    && entry.id.provider == vfs_local::PROVIDER_ID
                    && ImageFormat::for_name(&entry.name).is_some_and(|f| mountable.contains(&f))
            })
    }
}

impl Action for MountImage {
    fn id(&self) -> ActionId {
        ActionId("core.mount")
    }
    fn label(&self) -> String {
        t!("action-mount")
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }
    /// Shown only on disk images the current OS mounts; hidden, never
    /// greyed, on every other file and in the Trash view.
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        if !is_trash(ctx) && self.all_mountable(ctx) {
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
        let paths: Vec<PathBuf> = ctx
            .selection
            .iter()
            .map(|entry| local_path(&location_of(entry)))
            .collect::<Result<_, _>>()?;
        let state = Arc::clone(&self.0);
        let handle = jobs.spawn_named(
            format!("Mount {}", describe(&ctx.selection)),
            move |job| async move {
                let total = paths.len() as u64;
                for (done, path) in paths.into_iter().enumerate() {
                    if job.cancel.is_cancelled() {
                        return Err(Error::Cancelled);
                    }
                    job.progress.report(Progress::Items {
                        done: done as u64,
                        total: Some(total),
                    });
                    let platform = Arc::clone(&state.platform);
                    let volume = runtime::handle()
                        .spawn_blocking(move || platform.mount_image(&path))
                        .await
                        .map_err(|join| Error::Io(std::io::Error::other(join)))??;
                    job.progress.report(Progress::Message(format!(
                        "Mounted {} at {}",
                        volume.name,
                        volume.mount_point.display()
                    )));
                }
                Ok(())
            },
        );
        Ok(ActionOutcome::Started(handle.id))
    }
}
