//! Rename, New file and New folder.

use std::sync::Arc;

use ext::{
    Action, ActionCtx, ActionGroup, ActionId, ActionOutcome, Availability, Key, Modifiers,
    RemoveMode, Shortcut, UiHost, UndoCommand, WriteOptions,
};
use runtime::JobRunner;
use tokio::io::AsyncWriteExt;
use types::{Error, Location};
use vfs::Mounts;

use crate::edit::parent;
use crate::{State, inside, location_of, needs_writable_location};

/// Renames the one selected entry.
pub(crate) struct Rename(pub(crate) Arc<State>);

impl Action for Rename {
    fn id(&self) -> ActionId {
        ActionId("core.rename")
    }
    fn label(&self) -> &'static str {
        "Rename"
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        Some(Shortcut {
            modifiers: Modifiers::empty(),
            key: Key::F(2),
        })
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        if needs_writable_location(ctx) == Availability::Hidden {
            return Availability::Hidden;
        }
        match ctx.selection.len() {
            1 => Availability::Enabled,
            0 => Availability::Disabled("select an entry first"),
            _ => Availability::Disabled("select one entry to rename"),
        }
    }
    fn run(
        &self,
        ctx: ActionCtx,
        jobs: &JobRunner,
        ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        let Some(entry) = ctx.selection.first() else {
            return Ok(ActionOutcome::Done);
        };
        let Some(new_name) = ui.prompt("Rename", &entry.name) else {
            return Ok(ActionOutcome::Done);
        };
        if new_name.is_empty() || new_name == entry.name {
            return Ok(ActionOutcome::Done);
        }
        let from = location_of(entry);
        let dir = Location::new(from.provider, parent(&from.path));
        let command = RenameCommand {
            mounts: Arc::clone(&self.0.mounts),
            to: inside(&dir, new_name.as_bytes()),
            label: format!("Rename {} to {new_name}", entry.name),
            from,
        };
        command.redo(jobs)?;
        Ok(ActionOutcome::Undoable(Box::new(command)))
    }
}

struct RenameCommand {
    mounts: Arc<Mounts>,
    from: Location,
    to: Location,
    label: String,
}

fn spawn_rename(jobs: &JobRunner, mounts: Arc<Mounts>, name: String, from: Location, to: Location) {
    jobs.spawn_named(name, move |_job| async move {
        mounts.resolve(&from)?.rename(&from, &to).await?;
        Ok(())
    });
}

impl UndoCommand for RenameCommand {
    fn undo(&self, jobs: &JobRunner) -> Result<(), Error> {
        spawn_rename(
            jobs,
            Arc::clone(&self.mounts),
            format!("Undo {}", self.label),
            self.to.clone(),
            self.from.clone(),
        );
        Ok(())
    }

    fn redo(&self, jobs: &JobRunner) -> Result<(), Error> {
        spawn_rename(
            jobs,
            Arc::clone(&self.mounts),
            self.label.clone(),
            self.from.clone(),
            self.to.clone(),
        );
        Ok(())
    }

    fn label(&self) -> String {
        self.label.clone()
    }
}

/// What New file and New folder create.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Created {
    File,
    Folder,
}

/// Creates an empty entry; undo removes it.
struct CreateCommand {
    mounts: Arc<Mounts>,
    at: Location,
    kind: Created,
    label: String,
}

impl UndoCommand for CreateCommand {
    fn undo(&self, jobs: &JobRunner) -> Result<(), Error> {
        let (mounts, at) = (Arc::clone(&self.mounts), self.at.clone());
        jobs.spawn_named(format!("Undo {}", self.label), move |_job| async move {
            mounts
                .resolve(&at)?
                .remove(&at, RemoveMode::Permanent)
                .await
        });
        Ok(())
    }

    fn redo(&self, jobs: &JobRunner) -> Result<(), Error> {
        let (mounts, at, kind) = (Arc::clone(&self.mounts), self.at.clone(), self.kind);
        jobs.spawn_named(self.label.clone(), move |_job| async move {
            let provider = mounts.resolve(&at)?;
            match kind {
                Created::Folder => {
                    provider.create_dir(&at).await?;
                }
                Created::File => {
                    let mut writer = provider
                        .open_write(&at, WriteOptions { overwrite: false })
                        .await?;
                    writer.shutdown().await.map_err(Error::from_io)?;
                }
            }
            Ok(())
        });
        Ok(())
    }

    fn label(&self) -> String {
        self.label.clone()
    }
}

fn create(
    state: &State,
    ctx: &ActionCtx,
    ui: &dyn UiHost,
    jobs: &JobRunner,
    kind: Created,
    title: &str,
    suggested: &str,
) -> Result<ActionOutcome, Error> {
    let Some(name) = ui.prompt(title, suggested) else {
        return Ok(ActionOutcome::Done);
    };
    if name.is_empty() {
        return Ok(ActionOutcome::Done);
    }
    let command = CreateCommand {
        mounts: Arc::clone(&state.mounts),
        at: inside(&ctx.location, name.as_bytes()),
        kind,
        label: format!("{title} {name}"),
    };
    command.redo(jobs)?;
    Ok(ActionOutcome::Undoable(Box::new(command)))
}

/// Creates an empty file in the current location.
pub(crate) struct NewFile(pub(crate) Arc<State>);

impl Action for NewFile {
    fn id(&self) -> ActionId {
        ActionId("core.new_file")
    }
    fn label(&self) -> &'static str {
        "New file"
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    // Mod+N opens a new window (SPEC.md "Windows"; QUESTIONS.md 66), so New
    // file has no default binding until the spec names one.
    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        needs_writable_location(ctx)
    }
    fn run(
        &self,
        ctx: ActionCtx,
        jobs: &JobRunner,
        ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        create(
            &self.0,
            &ctx,
            ui,
            jobs,
            Created::File,
            "New file",
            "New file.txt",
        )
    }
}

/// Creates a folder in the current location.
pub(crate) struct NewFolder(pub(crate) Arc<State>);

impl Action for NewFolder {
    fn id(&self) -> ActionId {
        ActionId("core.new_folder")
    }
    fn label(&self) -> &'static str {
        "New folder"
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        Some(Shortcut {
            modifiers: Modifiers::MOD | Modifiers::SHIFT,
            key: Key::Char('n'),
        })
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        needs_writable_location(ctx)
    }
    fn run(
        &self,
        ctx: ActionCtx,
        jobs: &JobRunner,
        ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        create(
            &self.0,
            &ctx,
            ui,
            jobs,
            Created::Folder,
            "New folder",
            "New folder",
        )
    }
}
