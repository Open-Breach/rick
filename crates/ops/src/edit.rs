//! Copy, Cut, Paste and Duplicate.

use std::sync::Arc;

use ext::{
    Action, ActionCtx, ActionGroup, ActionId, ActionOutcome, Availability, Capabilities,
    CopyOptions, Key, Modifiers, RemoveMode, Shortcut, UiHost, UndoCommand,
};
use runtime::{JobCtx, JobRunner};
use types::{EntryKind, Error, Location, Progress};
use vfs::Mounts;

use crate::clipboard::{ClipMode, Clipboard};
use crate::conflict::{Conflicts, Landing, Policy};
use crate::schedule::{self, Scheduler};
use crate::{State, describe, file_name, inside, location_of, needs_entries};

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
        needs_entries(ctx)
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
        needs_entries(ctx)
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
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        match self.0.needs(&ctx.location, Capabilities::WRITE) {
            Availability::Enabled => {}
            hidden => return hidden,
        }
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
                let handle = spawn_copies(
                    jobs,
                    Arc::clone(&self.0.mounts),
                    Arc::clone(&self.0.conflicts),
                    Arc::clone(&self.0.scheduler),
                    format!("Copy {what}"),
                    pairs,
                );
                Ok(ActionOutcome::Started(handle.id))
            }
            ClipMode::Cut => {
                *self.0.clipboard() = None;
                let command = MoveCommand {
                    mounts: Arc::clone(&self.0.mounts),
                    conflicts: Arc::clone(&self.0.conflicts),
                    scheduler: Arc::clone(&self.0.scheduler),
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
pub(crate) struct MoveCommand {
    pub(crate) mounts: Arc<Mounts>,
    pub(crate) conflicts: Arc<Conflicts>,
    pub(crate) scheduler: Arc<Scheduler>,
    pub(crate) pairs: Vec<(Location, Location)>,
    pub(crate) label: String,
}

/// Copies `from` to `to`; with `overwrite`, a target of another kind (a
/// file where a folder goes, or the reverse) is removed first, a folder
/// over a folder merges and a file over a file is replaced.
async fn copy_one(
    mounts: &Mounts,
    job: &JobCtx,
    from: &Location,
    to: &Location,
    overwrite: bool,
) -> Result<(), Error> {
    if overwrite {
        clear_mismatched_target(mounts, from, to).await?;
    }
    vfs::copy(
        mounts,
        from,
        to,
        CopyOptions { overwrite },
        job.progress.clone(),
        job.cancel.clone(),
        &job.pause,
    )
    .await
}

/// Moves `from` to `to`; with `overwrite`, a folder over a folder merges
/// (copy, then remove the source) and anything else replaces the target.
async fn move_one(
    mounts: &Mounts,
    job: &JobCtx,
    from: &Location,
    to: &Location,
    overwrite: bool,
) -> Result<(), Error> {
    if overwrite {
        let dst = mounts.resolve(to)?;
        let src = mounts.resolve(from)?;
        let both_dirs = matches!(src.stat(from).await, Ok(e) if e.kind == EntryKind::Dir)
            && matches!(dst.stat(to).await, Ok(e) if e.kind == EntryKind::Dir);
        if both_dirs {
            vfs::copy(
                mounts,
                from,
                to,
                CopyOptions { overwrite: true },
                job.progress.clone(),
                job.cancel.clone(),
                &job.pause,
            )
            .await?;
            if job.cancel.is_cancelled() {
                return Err(Error::Cancelled);
            }
            return src.remove(from, RemoveMode::Permanent).await;
        }
        match dst.remove(to, RemoveMode::Permanent).await {
            Ok(()) | Err(Error::NotFound) => {}
            Err(e) => return Err(e),
        }
    }
    vfs::move_entry(
        mounts,
        from,
        to,
        CopyOptions { overwrite },
        job.progress.clone(),
        job.cancel.clone(),
        &job.pause,
    )
    .await
}

/// Removes `to` when it is of another kind than `from`, so a folder can
/// take a file's place and the reverse.
async fn clear_mismatched_target(
    mounts: &Mounts,
    from: &Location,
    to: &Location,
) -> Result<(), Error> {
    let src = mounts.resolve(from)?;
    let dst = mounts.resolve(to)?;
    let (Ok(source), Ok(target)) = (src.stat(from).await, dst.stat(to).await) else {
        return Ok(());
    };
    if (source.kind == EntryKind::Dir) != (target.kind == EntryKind::Dir) {
        dst.remove(to, RemoveMode::Permanent).await?;
    }
    Ok(())
}

/// Spawns a job that copies every pair in order, through the provider's
/// fast path, asking about existing names, after waiting its turn on the
/// volumes it touches, and returns its handle.
pub(crate) fn spawn_copies(
    jobs: &JobRunner,
    mounts: Arc<Mounts>,
    conflicts: Arc<Conflicts>,
    scheduler: Arc<Scheduler>,
    name: String,
    pairs: Vec<(Location, Location)>,
) -> runtime::JobHandle<()> {
    jobs.spawn_pausable(name.clone(), move |job| async move {
        let keys = schedule::volume_keys(&mounts, &pairs).await;
        let _lease = scheduler.acquire(&job, &name, keys).await;
        let total = pairs.len() as u64;
        let mut policy = Policy::default();
        for (done, (from, to)) in pairs.iter().enumerate() {
            job.pause.wait().await;
            job.progress.report(Progress::Items {
                done: done as u64,
                total: Some(total),
            });
            let remaining = total - done as u64 - 1;
            match policy
                .land(&conflicts, &job, &mounts, from, to, remaining)
                .await?
            {
                Landing::Skip => {}
                Landing::At { to, overwrite } => {
                    copy_one(&mounts, &job, from, &to, overwrite).await?;
                }
            }
        }
        job.progress.report(Progress::Items {
            done: total,
            total: Some(total),
        });
        Ok(())
    })
}

/// Spawns a job that moves every pair in order, asking about existing
/// names, after waiting its turn on the volumes it touches.
pub(crate) fn spawn_moves(
    jobs: &JobRunner,
    mounts: Arc<Mounts>,
    conflicts: Arc<Conflicts>,
    scheduler: Arc<Scheduler>,
    name: String,
    pairs: Vec<(Location, Location)>,
) {
    jobs.spawn_pausable(name.clone(), move |job| async move {
        let keys = schedule::volume_keys(&mounts, &pairs).await;
        let _lease = scheduler.acquire(&job, &name, keys).await;
        let total = pairs.len() as u64;
        let mut policy = Policy::default();
        for (done, (from, to)) in pairs.iter().enumerate() {
            job.pause.wait().await;
            job.progress.report(Progress::Items {
                done: done as u64,
                total: Some(total),
            });
            let remaining = total - done as u64 - 1;
            match policy
                .land(&conflicts, &job, &mounts, from, to, remaining)
                .await?
            {
                Landing::Skip => {}
                Landing::At { to, overwrite } => {
                    move_one(&mounts, &job, from, &to, overwrite).await?;
                }
            }
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
            Arc::clone(&self.conflicts),
            Arc::clone(&self.scheduler),
            format!("Undo {}", self.label),
            back,
        );
        Ok(())
    }

    fn redo(&self, jobs: &JobRunner) -> Result<(), Error> {
        spawn_moves(
            jobs,
            Arc::clone(&self.mounts),
            Arc::clone(&self.conflicts),
            Arc::clone(&self.scheduler),
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
        match self.0.needs(&ctx.location, Capabilities::WRITE) {
            Availability::Enabled => needs_entries(ctx),
            hidden => hidden,
        }
    }
    fn run(
        &self,
        ctx: ActionCtx,
        jobs: &JobRunner,
        _ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        let mounts = Arc::clone(&self.0.mounts);
        let scheduler = Arc::clone(&self.0.scheduler);
        let what = describe(&ctx.selection);
        let name = format!("Duplicate {what}");
        let sources: Vec<Location> = ctx.selection.iter().map(location_of).collect();
        let handle = jobs.spawn_named(name.clone(), move |job| async move {
            let pairs: Vec<(Location, Location)> =
                sources.iter().map(|s| (s.clone(), s.clone())).collect();
            let keys = schedule::volume_keys(&mounts, &pairs).await;
            let _lease = scheduler.acquire(&job, &name, keys).await;
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
                    &job.pause,
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
