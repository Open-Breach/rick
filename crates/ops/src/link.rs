//! Paste as link (Mod+Shift+V) and Create link…: a link to each clipboard
//! item, or to each selected item in a chosen folder. A symbolic link
//! everywhere; where the provider offers them (Windows) a choice between a
//! symbolic link, a directory junction and a `.lnk` shortcut. Undo removes
//! the links.

use ext::t;
use std::sync::Arc;

use ext::{
    Action, ActionCtx, ActionGroup, ActionId, ActionOutcome, Availability, Capabilities, Field,
    Form, Key, LinkKind, Modifiers, Shortcut, UiHost, UndoCommand,
};
use runtime::JobRunner;
use types::{Entry, EntryKind, Error, Location, Progress};
use vfs::Mounts;

use crate::{State, describe, file_name, inside, is_trash, location_of, needs_entries};

/// The link kinds a provider offers, in the order the choice lists them.
fn kinds_for(caps: Capabilities) -> Vec<LinkKind> {
    let mut kinds = Vec::new();
    if caps.contains(Capabilities::SYMLINK) {
        kinds.push(LinkKind::Symbolic);
    }
    if caps.contains(Capabilities::JUNCTION) {
        kinds.push(LinkKind::Junction);
    }
    if caps.contains(Capabilities::SHORTCUT) {
        kinds.push(LinkKind::Shortcut);
    }
    kinds
}

/// Asks which kind of link when the provider offers more than one;
/// `None` when the form was dismissed.
fn choose_kind(ui: &dyn UiHost, kinds: &[LinkKind], title: &str) -> Option<LinkKind> {
    match kinds {
        [] => None,
        [one] => Some(*one),
        many => {
            let form = Form::new(title, "Create").field(Field::choice(
                "kind",
                "Kind of link",
                many.iter().map(|k| k.label()).collect(),
                0,
            ));
            let values = ui.form(form)?;
            many.get(values.choice("kind")).copied()
        }
    }
}

/// The name a link to `entry` takes in its folder: the entry's name, with
/// `.lnk` for a shortcut.
fn link_name(entry: &Entry, kind: LinkKind) -> Vec<u8> {
    let mut name = file_name(entry);
    if kind == LinkKind::Shortcut {
        name.extend_from_slice(b".lnk");
    }
    name
}

/// Plans one link per item into `folder`, under a free name each.
fn plan(
    mounts: &Mounts,
    items: &[Entry],
    folder: &Location,
    kind: LinkKind,
) -> Result<Vec<(Location, Location)>, Error> {
    let provider = mounts.resolve(folder)?;
    let mut links = Vec::with_capacity(items.len());
    runtime::block_on(async {
        for item in items {
            if kind == LinkKind::Junction && item.kind != EntryKind::Dir {
                return Err(Error::Unsupported("a junction points to a folder"));
            }
            let name = String::from_utf8_lossy(&link_name(item, kind)).into_owned();
            let mut at = inside(folder, name.as_bytes());
            let mut n = 1;
            while provider.stat(&at).await.is_ok() {
                n += 1;
                if n > 9_999 {
                    return Err(Error::Unsupported("too many links with that name"));
                }
                at = inside(folder, types::numbered_name(&name, n).as_bytes());
            }
            links.push((location_of(item), at));
        }
        Ok(())
    })?;
    Ok(links)
}

/// Creates links forward, removes them back.
struct LinkCommand {
    mounts: Arc<Mounts>,
    /// (target, link) pairs.
    links: Vec<(Location, Location)>,
    kind: LinkKind,
    label: String,
}

impl UndoCommand for LinkCommand {
    fn undo(&self, jobs: &JobRunner) -> Result<(), Error> {
        let mounts = Arc::clone(&self.mounts);
        let links = self.links.clone();
        jobs.spawn_named(format!("Undo {}", self.label), move |job| async move {
            let total = links.len() as u64;
            for (done, (_, at)) in links.iter().enumerate() {
                job.progress.report(Progress::Items {
                    done: done as u64,
                    total: Some(total),
                });
                match mounts
                    .resolve(at)?
                    .remove(at, ext::RemoveMode::Permanent)
                    .await
                {
                    Ok(()) | Err(Error::NotFound) => {}
                    // A directory link on Windows is removed as a folder.
                    Err(error) => {
                        let path = crate::local_path(at);
                        match path {
                            Ok(path) if std::fs::remove_dir(&path).is_ok() => {}
                            _ => return Err(error),
                        }
                    }
                }
            }
            Ok(())
        });
        Ok(())
    }

    fn redo(&self, jobs: &JobRunner) -> Result<(), Error> {
        let mounts = Arc::clone(&self.mounts);
        let links = self.links.clone();
        let kind = self.kind;
        jobs.spawn_named(self.label.clone(), move |job| async move {
            let total = links.len() as u64;
            for (done, (target, at)) in links.iter().enumerate() {
                job.progress.report(Progress::Items {
                    done: done as u64,
                    total: Some(total),
                });
                if job.cancel.is_cancelled() {
                    return Err(Error::Cancelled);
                }
                mounts.resolve(at)?.link(target, at, kind).await?;
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

/// Builds and starts the command for `items` into `folder`.
fn link_items(
    state: &State,
    jobs: &JobRunner,
    ui: &dyn UiHost,
    items: &[Entry],
    folder: &Location,
    title: &str,
) -> Result<ActionOutcome, Error> {
    let kinds = kinds_for(state.capabilities(folder));
    let Some(kind) = choose_kind(ui, &kinds, title) else {
        return Ok(ActionOutcome::Done);
    };
    let links = plan(&state.mounts, items, folder, kind)?;
    let command = LinkCommand {
        mounts: Arc::clone(&state.mounts),
        label: format!(
            "Link {} into {}",
            describe(items),
            crate::conflict::name_of(folder)
        ),
        links,
        kind,
    };
    command.redo(jobs)?;
    Ok(ActionOutcome::Undoable(Box::new(command)))
}

/// Links to the clipboard's items in the current folder.
pub(crate) struct PasteAsLink(pub(crate) Arc<State>);

impl Action for PasteAsLink {
    fn id(&self) -> ActionId {
        ActionId("core.paste_link")
    }
    fn label(&self) -> String {
        t!("action-paste-as-link")
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        Some(Shortcut {
            modifiers: Modifiers::MOD | Modifiers::SHIFT,
            key: Key::Char('v'),
        })
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        if crate::is_virtual(ctx) {
            return Availability::Hidden;
        }
        if kinds_for(self.0.capabilities(&ctx.location)).is_empty() {
            return Availability::Hidden;
        }
        if self.0.clipboard().is_some() {
            Availability::Enabled
        } else {
            Availability::Disabled(t!("reason-nothing-to-paste"))
        }
    }
    fn run(
        &self,
        ctx: ActionCtx,
        jobs: &JobRunner,
        ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        let Some(clipboard) = self.0.clipboard().clone() else {
            return Ok(ActionOutcome::Done);
        };
        link_items(
            &self.0,
            jobs,
            ui,
            &clipboard.items,
            &ctx.location,
            "Paste as link",
        )
    }
}

/// Links to the selection in a chosen folder.
pub(crate) struct CreateLink(pub(crate) Arc<State>);

impl Action for CreateLink {
    fn id(&self) -> ActionId {
        ActionId("core.create_link")
    }
    fn label(&self) -> String {
        t!("action-create-link")
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        if is_trash(ctx) || crate::is_virtual(ctx) {
            return Availability::Hidden;
        }
        if kinds_for(self.0.capabilities(&ctx.location)).is_empty() {
            return Availability::Hidden;
        }
        needs_entries(ctx)
    }
    fn run(
        &self,
        ctx: ActionCtx,
        jobs: &JobRunner,
        ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        if ctx.selection.is_empty() {
            return Ok(ActionOutcome::Done);
        }
        let kinds = kinds_for(self.0.capabilities(&ctx.location));
        let current = crate::local_path(&ctx.location).map_or_default(|p| p.display().to_string());
        let mut form = Form::new(
            format!("Create link to {}", describe(&ctx.selection)),
            "Create",
        )
        .field(Field::folder("folder", "Create the link in", current));
        if kinds.len() > 1 {
            form = form.field(Field::choice(
                "kind",
                "Kind of link",
                kinds.iter().map(|k| k.label()).collect(),
                0,
            ));
        }
        let Some(values) = ui.form(form) else {
            return Ok(ActionOutcome::Done);
        };
        let folder_text = values.text("folder").trim().to_owned();
        if folder_text.is_empty() {
            return Err(Error::Unsupported("name the folder for the link"));
        }
        let folder = Location::new(
            ctx.location.provider,
            platform::paths::from_path(std::path::Path::new(&folder_text)),
        );
        let kind = kinds
            .get(if kinds.len() > 1 {
                values.choice("kind")
            } else {
                0
            })
            .copied()
            .ok_or(Error::Unsupported("this location takes no links"))?;
        let links = plan(&self.0.mounts, &ctx.selection, &folder, kind)?;
        let command = LinkCommand {
            mounts: Arc::clone(&self.0.mounts),
            label: format!(
                "Link {} into {}",
                describe(&ctx.selection),
                crate::conflict::name_of(&folder)
            ),
            links,
            kind,
        };
        command.redo(jobs)?;
        Ok(ActionOutcome::Undoable(Box::new(command)))
    }
}
