//! The "SVN" action group: each action runs the `svn` command line in the
//! working copy as a job, then drops the cached status so the badges
//! follow.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ext::{
    Action, ActionCtx, ActionGroup, ActionId, ActionOutcome, Availability, JobRunner, Shortcut,
    UiHost,
};
use types::{Entry, Error, Progress};

use crate::wc::{FileStatus, SvnError, WorkingCopies, svn, to_error};

/// The group every action belongs to.
pub const GROUP: ActionGroup = ActionGroup::Extension("SVN");

/// What every SVN action shares.
pub struct Shared {
    pub copies: Arc<WorkingCopies>,
}

/// The working copy of the context's location, if it is in one.
fn working_copy(copies: &WorkingCopies, ctx: &ActionCtx) -> Option<PathBuf> {
    let dir = crate::local_path(&ctx.location)?;
    copies.working_copy(&dir)
}

/// Enabled inside a working copy, hidden outside.
fn inside(copies: &WorkingCopies, ctx: &ActionCtx) -> Availability {
    if working_copy(copies, ctx).is_some() {
        Availability::Enabled
    } else {
        Availability::Hidden
    }
}

/// Needs a selection; hidden outside a working copy.
fn needs_selection(copies: &WorkingCopies, ctx: &ActionCtx) -> Availability {
    match inside(copies, ctx) {
        Availability::Enabled if ctx.selection.is_empty() => {
            Availability::Disabled("select the files first")
        }
        other => other,
    }
}

/// Paths of the selected entries relative to the working copy, as `svn`
/// takes them.
fn rela_paths(root: &Path, entries: &[Entry]) -> Vec<String> {
    entries
        .iter()
        .filter_map(|e| {
            let path = crate::local_path(&types::Location::new(
                types::ProviderId("local"),
                e.path.clone(),
            ))?;
            let rela = path.strip_prefix(root).ok()?;
            Some(rela.to_string_lossy().replace('\\', "/"))
        })
        .collect()
}

/// Spawns `svn args` as a job named `name`; the cached status of `root` is
/// dropped when it finishes.
fn spawn_svn(
    shared: &Shared,
    jobs: &JobRunner,
    name: String,
    root: PathBuf,
    args: Vec<String>,
) -> ActionOutcome {
    let copies = Arc::clone(&shared.copies);
    let handle = jobs.spawn_named(name.clone(), move |job| async move {
        job.progress.report(Progress::Message(name));
        let result = {
            let root = root.clone();
            ext::blocking(move || svn(&root, &args).map_err(to_error)).await
        };
        copies.invalidate(&root);
        result.map(|_| ())
    });
    ActionOutcome::Started(handle.id)
}

fn strings(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| (*p).to_owned()).collect()
}

/// The selected paths that are not under version control yet, by the
/// cached status; what `svn add` must take before a commit of them.
fn unversioned(shared: &Shared, root: &Path, paths: &[String]) -> Vec<String> {
    let Some(snapshot) = shared.copies.snapshot(root) else {
        return Vec::new();
    };
    paths
        .iter()
        .filter(|p| snapshot.status_of(p) == Some(FileStatus::Unversioned))
        .cloned()
        .collect()
}

macro_rules! svn_action {
    ($name:ident, $id:literal, $label:literal) => {
        /// An SVN action.
        pub struct $name(pub Arc<Shared>);

        impl $name {
            fn id_label() -> (ActionId, &'static str) {
                (ActionId($id), $label)
            }
        }
    };
}

svn_action!(Update, "svn.update", "Update");
svn_action!(Commit, "svn.commit", "Commit");
svn_action!(Revert, "svn.revert", "Revert");
svn_action!(Log, "svn.log", "Log");
svn_action!(Add, "svn.add", "Add");

/// The common parts of the trait, with the per-action pieces as closures.
macro_rules! impl_action {
    ($name:ident, availability: $avail:expr, run: $run:expr) => {
        impl Action for $name {
            fn id(&self) -> ActionId {
                Self::id_label().0
            }
            fn label(&self) -> &'static str {
                Self::id_label().1
            }
            fn group(&self) -> ActionGroup {
                GROUP
            }
            fn default_shortcut(&self) -> Option<Shortcut> {
                None
            }
            fn availability(&self, ctx: &ActionCtx) -> Availability {
                let avail: fn(&WorkingCopies, &ActionCtx) -> Availability = $avail;
                avail(&self.0.copies, ctx)
            }
            fn run(
                &self,
                ctx: ActionCtx,
                jobs: &JobRunner,
                ui: &dyn UiHost,
            ) -> Result<ActionOutcome, Error> {
                let Some(root) = working_copy(&self.0.copies, &ctx) else {
                    return Err(to_error(SvnError::NotAWorkingCopy));
                };
                let run: fn(
                    &Shared,
                    &JobRunner,
                    &dyn UiHost,
                    PathBuf,
                    &ActionCtx,
                ) -> Result<ActionOutcome, Error> = $run;
                run(&self.0, jobs, ui, root, &ctx)
            }
        }
    };
}

impl_action!(Update,
    availability: inside,
    run: |shared, jobs, _ui, root, ctx| {
        // The selection when there is one, the whole working copy otherwise.
        let mut args = strings(&["update"]);
        let paths = rela_paths(&root, &ctx.selection);
        if !paths.is_empty() {
            args.push("--".to_owned());
            args.extend(paths);
        }
        Ok(spawn_svn(shared, jobs, "SVN update".to_owned(), root, args))
    }
);

impl_action!(Commit,
    availability: inside,
    run: |shared, jobs, ui, root, ctx| {
        let Some(message) = ui.prompt("Commit message", "") else {
            return Ok(ActionOutcome::Done);
        };
        if message.trim().is_empty() {
            return Ok(ActionOutcome::Done);
        }
        let paths = rela_paths(&root, &ctx.selection);
        let mut args = strings(&["commit", "-m", &message]);
        if !paths.is_empty() {
            // What was selected but never added is added first, then only
            // the selection is committed.
            let new = unversioned(shared, &root, &paths);
            if !new.is_empty() {
                let mut add = strings(&["add", "--parents", "--"]);
                add.extend(new);
                svn(&root, &add).map_err(to_error)?;
            }
            args.push("--".to_owned());
            args.extend(paths);
        }
        Ok(spawn_svn(shared, jobs, "SVN commit".to_owned(), root, args))
    }
);

impl_action!(Revert,
    availability: needs_selection,
    run: |shared, jobs, ui, root, ctx| {
        let paths = rela_paths(&root, &ctx.selection);
        let what = if paths.len() == 1 { paths[0].clone() } else { format!("{} items", paths.len()) };
        if !ui.confirm("Revert", &format!("Revert the changes to {what}? This cannot be undone.")) {
            return Ok(ActionOutcome::Done);
        }
        let mut args = strings(&["revert", "-R", "--"]);
        args.extend(paths);
        Ok(spawn_svn(shared, jobs, "SVN revert".to_owned(), root, args))
    }
);

impl_action!(Log,
    availability: inside,
    run: |_shared, _jobs, ui, root, ctx| {
        let mut args = strings(&["log", "-l", "100"]);
        let paths = rela_paths(&root, &ctx.selection);
        if !paths.is_empty() {
            args.push("--".to_owned());
            args.extend(paths);
        }
        let log = svn(&root, &args).map_err(to_error)?;
        let log = log.trim();
        ui.show("SVN log", if log.is_empty() || log.chars().all(|c| c == '-') { "No commits yet." } else { log });
        Ok(ActionOutcome::Done)
    }
);

impl_action!(Add,
    availability: needs_selection,
    run: |shared, jobs, _ui, root, ctx| {
        // `--force` lets a versioned folder in the selection add what is
        // new inside it instead of refusing.
        let mut args = strings(&["add", "--parents", "--force", "--"]);
        args.extend(rela_paths(&root, &ctx.selection));
        Ok(spawn_svn(shared, jobs, "SVN add".to_owned(), root, args))
    }
);

/// Every action of the group, in menu order.
#[must_use]
pub fn all(shared: &Arc<Shared>) -> Vec<Arc<dyn Action>> {
    vec![
        Arc::new(Update(Arc::clone(shared))),
        Arc::new(Commit(Arc::clone(shared))),
        Arc::new(Revert(Arc::clone(shared))),
        Arc::new(Log(Arc::clone(shared))),
        Arc::new(Add(Arc::clone(shared))),
    ]
}
