//! Download: a remote selection brought down to the machine.
//!
//! A file on a location with `REMOTE` is not on the machine, so nothing is
//! handed to the system for it (SPEC.md, *Remote files*). Enter and a
//! double-click copy it into the Downloads folder instead, and *Download…*
//! does the same for the whole selection, folders included with
//! everything beneath them, into a folder that is asked for first. Both
//! run the copy job every other copy runs: progress in the operations
//! drawer, the paste-conflict dialogue for a name that is already there,
//! and nothing opened at the end.

use ext::t;
use std::path::PathBuf;
use std::sync::Arc;

use ext::{
    Action, ActionCtx, ActionGroup, ActionId, ActionOutcome, Availability, Capabilities, Shortcut,
    UiHost,
};
use platform::{Platform, StandardFolder};
use runtime::JobRunner;
use types::{Entry, Error, Location};

use crate::edit::spawn_copies;
use crate::{State, describe, file_name, inside, location_of, needs_entries};

/// Where a download lands unless the user says otherwise: the Downloads
/// folder the platform names, the home folder where it names none.
pub(crate) fn downloads(platform: &dyn Platform) -> Location {
    let folder = platform
        .standard_folder(StandardFolder::Downloads)
        .unwrap_or_else(|| platform.home_dir());
    Location::new(vfs_local::PROVIDER_ID, platform::paths::from_path(&folder))
}

/// Starts the job that copies `entries` into `to`, each under its own
/// name.
pub(crate) fn start(
    state: &State,
    jobs: &JobRunner,
    entries: &[Entry],
    to: &Location,
) -> ActionOutcome {
    if entries.is_empty() {
        return ActionOutcome::Done;
    }
    let pairs: Vec<(Location, Location)> = entries
        .iter()
        .map(|entry| (location_of(entry), inside(to, &file_name(entry))))
        .collect();
    let folder = to.path.file_name().map_or_else(
        || to.path.to_string(),
        |name| String::from_utf8_lossy(name).into_owned(),
    );
    let handle = spawn_copies(
        jobs,
        Arc::clone(&state.mounts),
        Arc::clone(&state.conflicts),
        Arc::clone(&state.scheduler),
        format!("Download {} to {folder}", describe(entries)),
        pairs,
    );
    ActionOutcome::Started(handle.id)
}

/// Brings the selected remote files and folders down to the machine, into
/// a folder asked for first.
pub(crate) struct Download(pub(crate) Arc<State>);

impl Action for Download {
    fn id(&self) -> ActionId {
        ActionId("core.download")
    }
    fn label(&self) -> String {
        t!("action-download")
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        // In the menu on remote locations only; there it follows the
        // location's `DOWNLOAD`.
        if !self
            .0
            .capabilities(&ctx.location)
            .contains(Capabilities::REMOTE)
        {
            return Availability::Hidden;
        }
        match self.0.offers(&ctx.location, Capabilities::DOWNLOAD) {
            Availability::Enabled => needs_entries(ctx),
            other => other,
        }
    }
    fn run(
        &self,
        ctx: ActionCtx,
        jobs: &JobRunner,
        ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        // The target folder is asked for first, Downloads by default. A
        // host with the system's folder dialogue answers the prompt with
        // the folder picked there.
        let suggested = downloads(&*self.0.platform);
        let suggested = platform::paths::to_path(&suggested.path)?;
        let Some(answer) = ui.prompt(&t!("download-to"), &suggested.to_string_lossy()) else {
            return Ok(ActionOutcome::Done);
        };
        let answer = answer.trim();
        if answer.is_empty() {
            return Ok(ActionOutcome::Done);
        }
        let to = Location::new(
            vfs_local::PROVIDER_ID,
            platform::paths::from_path(&PathBuf::from(answer)),
        );
        Ok(start(&self.0, jobs, &ctx.selection, &to))
    }
}
