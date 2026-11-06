//! Copy, Cut, Paste and Duplicate.

use std::sync::Arc;

use ext::{
    Action, ActionCtx, ActionGroup, ActionId, ActionOutcome, Availability, CopyOptions, Key,
    Modifiers, Shortcut, UiHost, UndoCommand,
};
use runtime::JobRunner;
use types::{Error, Location, Progress};
use vfs::Mounts;

use crate::clipboard::{ClipMode, Clipboard};
use crate::{State, describe, file_name, inside, location_of};

fn needs_selection(ctx: &ActionCtx) -> Availability {
    if ctx.selection.is_empty() {
        Availability::Disabled("select something first")
    } else {
        Availability::Enabled
    }
}

/// Puts the selection on the clipboard for copying.
pub(crate) struct Copy(pub(crate) Arc<State>);

impl Action for Copy {
    fn id(&self) -> ActionId {
        ActionId("core.copy")
    }
    fn label(&self) -> &'static str {
        "Copy"
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        Some(Shortcut {
            modifiers: Modifiers::MOD,
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
        *self.0.clipboard() = Some(Clipboard {
            items: ctx.selection,
            mode: ClipMode::Copy,
        });
        Ok(ActionOutcome::Done)
    }
}

/// Puts the selection on the clipboard for moving.
pub(crate) struct Cut(pub(crate) Arc<State>);

impl Action for Cut {
    fn id(&self) -> ActionId {
        ActionId("core.cut")
    }
    fn label(&self) -> &'static str {
        "Cut"
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        Some(Shortcut {
            modifiers: Modifiers::MOD,
            key: Key::Char('x'),
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
        *self.0.clipboard() = Some(Clipboard {
            items: ctx.selection,
            mode: ClipMode::Cut,
        });
        Ok(ActionOutcome::Done)
    }
}

/// Copies or moves the clipboard's items into the current location.
pub(crate) struct Paste(pub(crate) Arc<State>);

impl Action for Paste {
    fn id(&self) -> ActionId {
        ActionId("core.paste")
    }
    fn label(&self) -> &'static str {
        "Paste"
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        Some(Shortcut {
            modifiers: Modifiers::MOD,
            key: Key::Char('v'),
        })
    }
    fn availability(&self, _ctx: &ActionCtx) -> Availability {
        if self
            .0
            .clipboard()
            .as_ref()
            .is_some_and(|c| !c.items.is_empty())
        {
            Availability::Enabled
        } else {
            Availability::Disabled("nothing to paste")
        }
    }
    fn run(
        &self,
        ctx: ActionCtx,
        jobs: &JobRunner,
        _ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        let Some(clipboard) = self.0.clipboard().clone() else {
            return Ok(ActionOutcome::Done);
        };
        let pairs: Vec<(Location, Location)> = clipboard
            .items
            .iter()
            .map(|item| (location_of(item), inside(&ctx.location, &file_name(item))))
            .collect();
        let what = describe(&clipboard.items);
        match clipboard.mode {
            ClipMode::Copy => {
                let mounts = Arc::clone(&self.0.mounts);
                let handle = jobs.spawn_named(format!("Copy {what}"), move |job| async move {
                    let total = pairs.len() as u64;
                    for (done, (from, to)) in pairs.iter().enumerate() {
                        job.progress.report(Progress::Items {
                            done: done as u64,
                            total: Some(total),
                        });
                        vfs::copy(
                            &mounts,
                            from,
                            to,
                            CopyOptions::default(),
                            job.progress.clone(),
                            job.cancel.clone(),
                        )
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
            ClipMode::Cut => {
                *self.0.clipboard() = None;
                let command = MoveCommand {
                    mounts: Arc::clone(&self.0.mounts),
                    pairs,
                    label: format!("Move {what}"),
                };
                command.redo(jobs)?;
                Ok(ActionOutcome::Undoable(Box::new(command)))
            }
        }
    }
}

/// Moves `pairs` forward, or back again.
struct MoveCommand {
    mounts: Arc<Mounts>,
    pairs: Vec<(Location, Location)>,
    label: String,
}

/// Spawns a job that moves every pair in order.
pub(crate) fn spawn_moves(
    jobs: &JobRunner,
    mounts: Arc<Mounts>,
    name: String,
    pairs: Vec<(Location, Location)>,
) {
    jobs.spawn_named(name, move |job| async move {
        let total = pairs.len() as u64;
        for (done, (from, to)) in pairs.iter().enumerate() {
            job.progress.report(Progress::Items {
                done: done as u64,
                total: Some(total),
            });
            vfs::move_entry(
                &mounts,
                from,
                to,
                CopyOptions::default(),
                job.progress.clone(),
                job.cancel.clone(),
            )
            .await?;
        }
        job.progress.report(Progress::Items {
            done: total,
            total: Some(total),
        });
        Ok(())
    });
}

impl UndoCommand for MoveCommand {
    fn undo(&self, jobs: &JobRunner) -> Result<(), Error> {
        let back = self
            .pairs
            .iter()
            .rev()
            .map(|(from, to)| (to.clone(), from.clone()))
            .collect();
        spawn_moves(
            jobs,
            Arc::clone(&self.mounts),
            format!("Undo {}", self.label),
            back,
        );
        Ok(())
    }

    fn redo(&self, jobs: &JobRunner) -> Result<(), Error> {
        spawn_moves(
            jobs,
            Arc::clone(&self.mounts),
            self.label.clone(),
            self.pairs.clone(),
        );
        Ok(())
    }

    fn label(&self) -> String {
        self.label.clone()
    }
}

/// Copies the selection next to itself with a derived name.
pub(crate) struct Duplicate(pub(crate) Arc<State>);

/// `name (copy).ext`, then `name (copy 2).ext`, and so on.
fn derived_name(name: &[u8], attempt: u32) -> Vec<u8> {
    let (stem, ext) = match name.iter().rposition(|&b| b == b'.') {
        Some(dot) if dot > 0 => (&name[..dot], &name[dot..]),
        _ => (name, &name[name.len()..]),
    };
    let mut out = stem.to_vec();
    if attempt <= 1 {
        out.extend_from_slice(b" (copy)");
    } else {
        out.extend_from_slice(format!(" (copy {attempt})").as_bytes());
    }
    out.extend_from_slice(ext);
    out
}

impl Action for Duplicate {
    fn id(&self) -> ActionId {
        ActionId("core.duplicate")
    }
    fn label(&self) -> &'static str {
        "Duplicate"
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        Some(Shortcut {
            modifiers: Modifiers::MOD,
            key: Key::Char('d'),
        })
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        needs_selection(ctx)
    }
    fn run(
        &self,
        ctx: ActionCtx,
        jobs: &JobRunner,
        _ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        let mounts = Arc::clone(&self.0.mounts);
        let what = describe(&ctx.selection);
        let sources: Vec<Location> = ctx.selection.iter().map(location_of).collect();
        let handle = jobs.spawn_named(format!("Duplicate {what}"), move |job| async move {
            let total = sources.len() as u64;
            for (done, from) in sources.iter().enumerate() {
                job.progress.report(Progress::Items {
                    done: done as u64,
                    total: Some(total),
                });
                let provider = mounts.resolve(from)?;
                let dir = Location::new(from.provider, parent(&from.path));
                let name = from.path.file_name().unwrap_or_default().to_vec();
                let mut to = inside(&dir, &derived_name(&name, 1));
                let mut attempt = 1;
                while provider.stat(&to).await.is_ok() {
                    attempt += 1;
                    if attempt > 99 {
                        return Err(Error::Unsupported("too many copies with that name"));
                    }
                    to = inside(&dir, &derived_name(&name, attempt));
                }
                vfs::copy(
                    &mounts,
                    from,
                    &to,
                    CopyOptions::default(),
                    job.progress.clone(),
                    job.cancel.clone(),
                )
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

/// The path without its last component.
pub(crate) fn parent(path: &types::VPath) -> types::VPath {
    let bytes = path.as_bytes();
    let trimmed = match bytes.split_last() {
        Some((b'/' | b'\\', rest)) => rest,
        _ => bytes,
    };
    match trimmed.iter().rposition(|b| matches!(b, b'/' | b'\\')) {
        Some(0) => types::VPath::from_bytes(trimmed[..1].to_vec()),
        Some(i) => types::VPath::from_bytes(trimmed[..i].to_vec()),
        None => types::VPath::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_names_keep_the_extension() {
        assert_eq!(derived_name(b"a.txt", 1), b"a (copy).txt");
        assert_eq!(derived_name(b"a.txt", 2), b"a (copy 2).txt");
        assert_eq!(derived_name(b"Makefile", 1), b"Makefile (copy)");
        assert_eq!(derived_name(b".hidden", 1), b".hidden (copy)");
        assert_eq!(derived_name(b"x.tar.gz", 3), b"x.tar (copy 3).gz");
    }

    #[test]
    fn parent_strips_the_last_component() {
        assert_eq!(parent(&"/a/b/c".into()).to_string(), "/a/b");
        assert_eq!(parent(&"/a".into()).to_string(), "/");
        assert_eq!(parent(&"C:\\x\\y".into()).to_string(), "C:\\x");
        assert_eq!(parent(&"name".into()).to_string(), "");
    }
}
