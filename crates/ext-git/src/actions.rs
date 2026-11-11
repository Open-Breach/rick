//! The "Git" action group: each action runs the `git` command line in the
//! work tree as a job, then drops the cached status so the badges follow.
//! *Git clone…* ([`crate::clone`]) is the one that needs no work tree.

use ext::t;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use ext::{
    Action, ActionCtx, ActionGroup, ActionId, ActionOutcome, Availability, JobRunner, Shortcut,
    UiHost,
};
use types::{Entry, Error, Progress};

use crate::repo::{GitError, Repos, to_error};

/// The group every action belongs to.
pub const GROUP: ActionGroup = ActionGroup::Extension("Git");

/// What every Git action shares.
pub struct Shared {
    pub repos: Arc<Repos>,
    /// The platform services the host lent, when it lent any: the
    /// clipboard *Git clone…* prefills its URL from.
    pub platform: Option<Arc<dyn ext::platform::Platform>>,
}

/// The work tree of the context's location, if it is in one.
fn work_tree(repos: &Repos, ctx: &ActionCtx) -> Option<PathBuf> {
    let dir = crate::local_path(&ctx.location)?;
    repos.work_tree(&dir)
}

/// Enabled inside a work tree, hidden outside.
fn inside(repos: &Repos, ctx: &ActionCtx) -> Availability {
    if work_tree(repos, ctx).is_some() {
        Availability::Enabled
    } else {
        Availability::Hidden
    }
}

/// Paths of the selected entries relative to the work tree, as `git` takes
/// them.
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

/// Runs `git args` in `root` and returns its stdout.
///
/// # Errors
///
/// `GitError::Command` with git's stderr when it exits non-zero, `Io` when
/// it cannot start.
pub fn git(root: &Path, args: &[String]) -> Result<String, GitError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        Err(GitError::Command {
            program: format!("git {}", args.first().map_or("", String::as_str)),
            stderr: if stderr.is_empty() { stdout } else { stderr },
        })
    }
}

/// Spawns `git args` as a job named `name`; the cached status of `root` is
/// dropped when it finishes.
fn spawn_git(
    shared: &Shared,
    jobs: &JobRunner,
    name: String,
    root: PathBuf,
    args: Vec<String>,
) -> ActionOutcome {
    let repos = Arc::clone(&shared.repos);
    let handle = jobs.spawn_named(name.clone(), move |job| async move {
        job.progress.report(Progress::Message(name));
        let result = {
            let root = root.clone();
            ext::blocking(move || git(&root, &args).map_err(to_error)).await
        };
        repos.invalidate(&root);
        result.map(|_| ())
    });
    ActionOutcome::Started(handle.id)
}

fn strings(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| (*p).to_owned()).collect()
}

/// Tracked content under `paths` goes back to HEAD, untracked content is
/// removed. `git restore` refuses a pathspec nothing tracked matches, so the
/// tracked paths are found first.
fn discard(root: &Path, paths: &[String]) -> Result<(), GitError> {
    let mut list = strings(&["ls-files", "-z", "--"]);
    list.extend(paths.iter().cloned());
    let tracked = git(root, &list)?;
    let tracked: Vec<&str> = tracked.split('\0').filter(|t| !t.is_empty()).collect();
    let restore: Vec<String> = paths
        .iter()
        .filter(|path| {
            tracked.iter().any(|t| {
                t == path
                    || t.strip_prefix(path.as_str())
                        .is_some_and(|rest| rest.starts_with('/'))
            })
        })
        .cloned()
        .collect();
    if !restore.is_empty() {
        let mut args = strings(&[
            "restore",
            "--staged",
            "--worktree",
            "--source=HEAD",
            "--ignore-unmerged",
            "--",
        ]);
        args.extend(restore);
        git(root, &args)?;
    }
    let mut clean = strings(&["clean", "-fd", "--"]);
    clean.extend(paths.iter().cloned());
    git(root, &clean)?;
    Ok(())
}

macro_rules! git_action {
    ($name:ident, $id:literal, $label:literal, $shortcut:expr) => {
        /// A Git action.
        pub struct $name(pub Arc<Shared>);

        impl $name {
            fn id_label() -> (ActionId, &'static str) {
                (ActionId($id), $label)
            }
        }
    };
}

git_action!(Commit, "git.commit", "git-commit", None);
git_action!(Stash, "git.stash", "git-stash", None);
git_action!(StashPop, "git.stash_pop", "git-stash-pop", None);
git_action!(Pull, "git.pull", "git-pull", None);
git_action!(Push, "git.push", "git-push", None);
git_action!(CheckoutBranch, "git.checkout", "git-checkout-branch", None);
git_action!(DiscardChanges, "git.discard", "git-discard-changes", None);
git_action!(ShowLog, "git.log", "git-show-log", None);
git_action!(Blame, "git.blame", "git-blame", None);

/// The common parts of the trait, with the per-action pieces as closures.
macro_rules! impl_action {
    ($name:ident, availability: $avail:expr, run: $run:expr) => {
        impl Action for $name {
            fn id(&self) -> ActionId {
                Self::id_label().0
            }
            fn label(&self) -> String {
                ext::t!(Self::id_label().1)
            }
            fn group(&self) -> ActionGroup {
                GROUP
            }
            fn default_shortcut(&self) -> Option<Shortcut> {
                None
            }
            fn availability(&self, ctx: &ActionCtx) -> Availability {
                let avail: fn(&Repos, &ActionCtx) -> Availability = $avail;
                avail(&self.0.repos, ctx)
            }
            fn run(
                &self,
                ctx: ActionCtx,
                jobs: &JobRunner,
                ui: &dyn UiHost,
            ) -> Result<ActionOutcome, Error> {
                let Some(root) = work_tree(&self.0.repos, &ctx) else {
                    return Err(to_error(GitError::NotARepository));
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

/// Needs a selection; hidden outside a work tree.
fn needs_selection(repos: &Repos, ctx: &ActionCtx) -> Availability {
    match inside(repos, ctx) {
        Availability::Enabled if ctx.selection.is_empty() => {
            Availability::Disabled(t!("git-select-the-files-first"))
        }
        other => other,
    }
}

/// Needs exactly one selected file; hidden outside a work tree.
fn needs_one_file(repos: &Repos, ctx: &ActionCtx) -> Availability {
    match inside(repos, ctx) {
        Availability::Enabled => match ctx.selection.as_slice() {
            [one] if one.kind == types::EntryKind::File => Availability::Enabled,
            [_] => Availability::Disabled(t!("git-blame-works-on-a-file")),
            [] => Availability::Disabled(t!("git-select-a-file-first")),
            _ => Availability::Disabled(t!("git-select-one-file")),
        },
        other => other,
    }
}

impl_action!(Commit,
    availability: inside,
    run: |shared, jobs, ui, root, ctx| {
        let Some(message) = ui.prompt(&t!("git-commit-message"), "") else {
            return Ok(ActionOutcome::Done);
        };
        if message.trim().is_empty() {
            return Ok(ActionOutcome::Done);
        }
        let paths = rela_paths(&root, &ctx.selection);
        let mut args = strings(&["commit", "-m", &message]);
        if paths.is_empty() {
            args.push("-a".to_owned());
        } else {
            // Stage what was selected first, then commit only that.
            let mut add = strings(&["add", "--"]);
            add.extend(paths.iter().cloned());
            git(&root, &add).map_err(to_error)?;
            args.push("--".to_owned());
            args.extend(paths);
        }
        Ok(spawn_git(shared, jobs, "Git commit".to_owned(), root, args))
    }
);

impl_action!(Stash,
    availability: inside,
    run: |shared, jobs, _ui, root, ctx| {
        let mut args = strings(&["stash", "push", "--include-untracked"]);
        let paths = rela_paths(&root, &ctx.selection);
        if !paths.is_empty() {
            args.push("--".to_owned());
            args.extend(paths);
        }
        Ok(spawn_git(shared, jobs, "Git stash".to_owned(), root, args))
    }
);

impl_action!(StashPop,
    availability: inside,
    run: |shared, jobs, _ui, root, _ctx| {
        Ok(spawn_git(shared, jobs, "Git stash pop".to_owned(), root, strings(&["stash", "pop"])))
    }
);

impl_action!(Pull,
    availability: inside,
    run: |shared, jobs, _ui, root, _ctx| {
        Ok(spawn_git(shared, jobs, "Git pull".to_owned(), root, strings(&["pull", "--no-rebase"])))
    }
);

impl_action!(Push,
    availability: inside,
    run: |shared, jobs, _ui, root, _ctx| {
        Ok(spawn_git(shared, jobs, "Git push".to_owned(), root, strings(&["push"])))
    }
);

impl_action!(CheckoutBranch,
    availability: inside,
    run: |shared, jobs, ui, root, _ctx| {
        let current = shared
            .repos
            .snapshot(&root)
            .and_then(|s| s.branch.clone())
            .unwrap_or_default();
        let Some(branch) = ui.prompt(&t!("git-checkout-branch"), &current) else {
            return Ok(ActionOutcome::Done);
        };
        let branch = branch.trim().to_owned();
        if branch.is_empty() || branch == current {
            return Ok(ActionOutcome::Done);
        }
        Ok(spawn_git(shared, jobs, format!("Git checkout {branch}"), root, strings(&["checkout", &branch])))
    }
);

impl_action!(DiscardChanges,
    availability: needs_selection,
    run: |shared, jobs, ui, root, ctx| {
        let paths = rela_paths(&root, &ctx.selection);
        let what = if paths.len() == 1 { paths[0].clone() } else { format!("{} items", paths.len()) };
        if !ui.confirm(&t!("git-discard-changes"), &format!("Discard the changes to {what}? This cannot be undone.")) {
            return Ok(ActionOutcome::Done);
        }
        let repos = Arc::clone(&shared.repos);
        let handle = jobs.spawn_named("Git discard changes".to_owned(), move |_job| async move {
            let result = {
                let root = root.clone();
                ext::blocking(move || discard(&root, &paths).map_err(to_error)).await
            };
            repos.invalidate(&root);
            result
        });
        Ok(ActionOutcome::Started(handle.id))
    }
);

impl_action!(ShowLog,
    availability: inside,
    run: |_shared, _jobs, ui, root, ctx| {
        let mut args = strings(&["log", "--date=short", "--pretty=format:%h %ad %an  %s", "-n", "100"]);
        let paths = rela_paths(&root, &ctx.selection);
        if !paths.is_empty() {
            args.push("--".to_owned());
            args.extend(paths);
        }
        let log = git(&root, &args).map_err(to_error)?;
        ui.show(&t!("git-git-log"), if log.is_empty() { "No commits yet." } else { &log });
        Ok(ActionOutcome::Done)
    }
);

impl_action!(Blame,
    availability: needs_one_file,
    run: |_shared, _jobs, ui, root, ctx| {
        let paths = rela_paths(&root, &ctx.selection);
        let Some(path) = paths.first() else {
            return Ok(ActionOutcome::Done);
        };
        let mut args = strings(&["blame", "--date=short", "--"]);
        args.push(path.clone());
        let blame = git(&root, &args).map_err(to_error)?;
        ui.show(&format!("Blame {path}"), &blame);
        Ok(ActionOutcome::Done)
    }
);

/// Every action of the group, in menu order.
#[must_use]
pub fn all(shared: &Arc<Shared>) -> Vec<Arc<dyn Action>> {
    vec![
        Arc::new(Commit(Arc::clone(shared))),
        Arc::new(Stash(Arc::clone(shared))),
        Arc::new(StashPop(Arc::clone(shared))),
        Arc::new(Pull(Arc::clone(shared))),
        Arc::new(Push(Arc::clone(shared))),
        Arc::new(CheckoutBranch(Arc::clone(shared))),
        Arc::new(DiscardChanges(Arc::clone(shared))),
        Arc::new(ShowLog(Arc::clone(shared))),
        Arc::new(Blame(Arc::clone(shared))),
        Arc::new(crate::clone::CloneRepository(Arc::clone(shared))),
    ]
}
