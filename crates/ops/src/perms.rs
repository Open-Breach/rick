//! Permission changes as jobs: the owner, group, mode bits and ACL the
//! Properties dialogue sets on an entry, and on everything inside a folder
//! when asked, with a separate say over files and folders. Every entry's
//! state before the change is recorded, so the whole change is one step on
//! the undo stack, however many entries it touched. An entry the current
//! user may not change is not an error yet: the entries that were refused
//! go to the privileged helper in one call through
//! [`Elevate`](platform::Elevate), so the user sees one prompt, and the
//! helper reports what they had before the same way.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ext::UndoCommand;
use platform::perms::helper;
use platform::{FilePermissions, PermissionChange, Platform};
use runtime::{JobHandle, JobRunner};
use types::{CancelToken, Error, Progress};

/// Which enclosed entries a recursive change applies to, besides the
/// folder itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Recursion {
    /// The files inside.
    pub files: bool,
    /// The folders inside, which are walked either way.
    pub folders: bool,
}

/// What the Properties dialogue asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionRequest {
    /// The entries the dialogue was opened on.
    pub targets: Vec<PathBuf>,
    /// What to change on each.
    pub change: PermissionChange,
    /// Apply inside the target folders too, to these kinds of entries.
    pub recursive: Option<Recursion>,
}

/// What a permission job did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Applied {
    /// Every entry changed, with what it had before, in the order changed.
    pub previous: Vec<(PathBuf, FilePermissions)>,
    /// The entries that could not be changed, with why.
    pub failed: Vec<(PathBuf, String)>,
    /// Whether the privileged helper was needed.
    pub elevated: bool,
}

/// The label of the job and the undo step for `request`.
#[must_use]
pub fn label(request: &PermissionRequest) -> String {
    let what = match request.targets.as_slice() {
        [one] => one.file_name().map_or_else(
            || one.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        ),
        many => format!("{} items", many.len()),
    };
    let mut parts = Vec::new();
    if request.change.owner.is_some() || request.change.group.is_some() {
        parts.push("owner");
    }
    if request.change.mode.is_some() {
        parts.push("permissions");
    }
    if request.change.acl.is_some() {
        parts.push("access list");
    }
    let changed = if parts.is_empty() {
        "permissions".to_owned()
    } else {
        parts.join(" and ")
    };
    format!("Change {changed} of {what}")
}

/// Starts the job for `request`: the targets, then, for a recursive
/// request, everything inside the target folders of the kinds asked for,
/// the change recorded and applied entry by entry with progress.
pub fn spawn(
    jobs: &JobRunner,
    platform: Arc<dyn Platform>,
    name: String,
    request: PermissionRequest,
) -> JobHandle<Applied> {
    jobs.spawn_named(name, move |job| async move {
        let cancel = job.cancel.clone();
        let progress = job.progress.clone();
        runtime::handle()
            .spawn_blocking(move || {
                progress.report(Progress::Message("Collecting entries".to_owned()));
                let items = expand(&request, &cancel)?;
                apply(&*platform, items, &cancel, &progress)
            })
            .await
            .unwrap_or_else(|join| Err(Error::Io(std::io::Error::other(join))))
    })
}

/// Starts a job that applies `items` as they are: what undo and redo run.
pub fn spawn_items(
    jobs: &JobRunner,
    platform: Arc<dyn Platform>,
    name: String,
    items: Vec<(PathBuf, PermissionChange)>,
) -> JobHandle<Applied> {
    jobs.spawn_named(name, move |job| async move {
        let cancel = job.cancel.clone();
        let progress = job.progress.clone();
        runtime::handle()
            .spawn_blocking(move || apply(&*platform, items, &cancel, &progress))
            .await
            .unwrap_or_else(|join| Err(Error::Io(std::io::Error::other(join))))
    })
}

/// The entries a request applies to, each with its change: the targets,
/// and inside a target folder every entry of the kinds asked for, folders
/// walked depth first without following links.
fn expand(
    request: &PermissionRequest,
    cancel: &CancelToken,
) -> Result<Vec<(PathBuf, PermissionChange)>, Error> {
    let mut items = Vec::new();
    for target in &request.targets {
        items.push((target.clone(), request.change.clone()));
        let Some(recursion) = request.recursive else {
            continue;
        };
        let meta = std::fs::symlink_metadata(target).map_err(Error::from_io)?;
        if meta.is_dir() {
            walk(target, recursion, &request.change, cancel, &mut items)?;
        }
    }
    Ok(items)
}

fn walk(
    dir: &Path,
    recursion: Recursion,
    change: &PermissionChange,
    cancel: &CancelToken,
    items: &mut Vec<(PathBuf, PermissionChange)>,
) -> Result<(), Error> {
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        // A folder that cannot be read is reported when it is changed.
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries {
            let entry = entry.map_err(Error::from_io)?;
            let path = entry.path();
            let kind = entry.file_type().map_err(Error::from_io)?;
            if kind.is_dir() {
                if recursion.folders {
                    items.push((path.clone(), change.clone()));
                }
                pending.push(path);
            } else if recursion.files {
                items.push((path, change.clone()));
            }
        }
    }
    Ok(())
}

/// Applies `items` one by one, recording each entry's state first; the
/// entries the user may not change go to the privileged helper together.
fn apply(
    platform: &dyn Platform,
    items: Vec<(PathBuf, PermissionChange)>,
    cancel: &CancelToken,
    progress: &runtime::ProgressSink,
) -> Result<Applied, Error> {
    let total = u64::try_from(items.len()).unwrap_or(u64::MAX);
    let mut applied = Applied::default();
    let mut refused: Vec<helper::Item> = Vec::new();
    for (done, (path, change)) in items.into_iter().enumerate() {
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        progress.report(Progress::Items {
            done: u64::try_from(done).unwrap_or(u64::MAX),
            total: Some(total),
        });
        if change.is_empty() {
            continue;
        }
        let previous = match platform.permissions(&path) {
            Ok(previous) => previous,
            Err(Error::PermissionDenied) => {
                refused.push(helper::Item { path, change });
                continue;
            }
            Err(error) => {
                applied.failed.push((path, error.to_string()));
                continue;
            }
        };
        match platform.set_permissions(&path, &change) {
            Ok(()) => applied.previous.push((path, previous)),
            Err(Error::PermissionDenied) => refused.push(helper::Item { path, change }),
            Err(error) => applied.failed.push((path, error.to_string())),
        }
    }
    progress.report(Progress::Items {
        done: total,
        total: Some(total),
    });
    if refused.is_empty() {
        return Ok(applied);
    }
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    progress.report(Progress::Message(format!(
        "{} entries need the administrator",
        refused.len()
    )));
    applied.elevated = true;
    let elevate: &dyn platform::Elevate = platform;
    match platform::perms::apply_elevated(elevate, &refused) {
        Ok(reply) => {
            applied
                .previous
                .extend(reply.done.into_iter().map(|d| (d.path, d.previous)));
            applied
                .failed
                .extend(reply.failed.into_iter().map(|f| (f.path, f.error)));
            Ok(applied)
        }
        Err(error) => {
            // Nothing elevated ran; what was changed before stays changed
            // and is reported, the refused entries each with the reason.
            let reason = error.to_string();
            applied
                .failed
                .extend(refused.into_iter().map(|item| (item.path, reason.clone())));
            Ok(applied)
        }
    }
}

/// The undo step of a permission change: what every entry had before, and
/// the change itself for redo.
pub struct PermissionsCommand {
    platform: Arc<dyn Platform>,
    label: String,
    /// The change as applied, entry by entry.
    forward: Vec<(PathBuf, PermissionChange)>,
    /// What the entries had before, in the order changed.
    previous: Vec<(PathBuf, FilePermissions)>,
}

impl PermissionsCommand {
    /// The step for a job that applied `request` and reported `applied`.
    #[must_use]
    pub fn new(
        platform: Arc<dyn Platform>,
        label: String,
        request: &PermissionRequest,
        applied: &Applied,
    ) -> Self {
        let forward = applied
            .previous
            .iter()
            .map(|(path, _)| (path.clone(), request.change.clone()))
            .collect();
        Self {
            platform,
            label,
            forward,
            previous: applied.previous.clone(),
        }
    }

    /// How many entries the step covers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.previous.len()
    }

    /// Whether the step covers no entry at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.previous.is_empty()
    }
}

impl std::fmt::Debug for PermissionsCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PermissionsCommand")
            .field("label", &self.label)
            .field("entries", &self.previous.len())
            .finish_non_exhaustive()
    }
}

impl UndoCommand for PermissionsCommand {
    fn undo(&self, jobs: &JobRunner) -> Result<(), Error> {
        let items = self
            .previous
            .iter()
            .rev()
            .map(|(path, previous)| (path.clone(), previous.restore()))
            .collect();
        spawn_items(
            jobs,
            Arc::clone(&self.platform),
            format!("Undo {}", self.label),
            items,
        );
        Ok(())
    }

    fn redo(&self, jobs: &JobRunner) -> Result<(), Error> {
        spawn_items(
            jobs,
            Arc::clone(&self.platform),
            self.label.clone(),
            self.forward.clone(),
        );
        Ok(())
    }

    fn label(&self) -> String {
        self.label.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use platform::Fake;

    fn request(
        targets: Vec<PathBuf>,
        mode: u32,
        recursive: Option<Recursion>,
    ) -> PermissionRequest {
        PermissionRequest {
            targets,
            change: PermissionChange {
                mode: Some(mode),
                ..PermissionChange::default()
            },
            recursive,
        }
    }

    #[test]
    fn labels_name_what_changes() {
        let r = request(vec![PathBuf::from("/a/notes.txt")], 0o644, None);
        assert_eq!(label(&r), "Change permissions of notes.txt");
        let mut r = request(vec![PathBuf::from("/a"), PathBuf::from("/b")], 0o644, None);
        r.change.owner = Some(platform::Principal::id("0"));
        assert_eq!(label(&r), "Change owner and permissions of 2 items");
    }

    #[cfg(unix)]
    #[test]
    fn a_recursive_change_touches_the_kinds_asked_for_and_undoes_in_one_step() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let root = dir.path().join("tree");
        std::fs::create_dir_all(root.join("sub")).unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(root.join("a.txt"), b"a").unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(root.join("sub").join("b.txt"), b"b").unwrap_or_else(|e| panic!("{e}"));
        for (path, mode) in [
            (root.clone(), 0o755),
            (root.join("sub"), 0o755),
            (root.join("a.txt"), 0o644),
            (root.join("sub").join("b.txt"), 0o644),
        ] {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))
                .unwrap_or_else(|e| panic!("{e}"));
        }
        let mode_of = |path: &Path| {
            std::fs::symlink_metadata(path)
                .unwrap_or_else(|e| panic!("{e}"))
                .permissions()
                .mode()
                & 0o7777
        };
        let fake: Arc<dyn Platform> = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
        let jobs = JobRunner::new();

        // Files only: the folders keep their mode.
        let r = request(
            vec![root.clone()],
            0o600,
            Some(Recursion {
                files: true,
                folders: false,
            }),
        );
        let handle = spawn(&jobs, Arc::clone(&fake), label(&r), r.clone());
        let applied = runtime::block_on(handle.result)
            .unwrap_or_else(|e| panic!("{e}"))
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(applied.failed.is_empty(), "{:?}", applied.failed);
        assert!(!applied.elevated);
        // The target itself and the two files; `sub` was walked, not changed.
        assert_eq!(applied.previous.len(), 3);
        assert_eq!(mode_of(&root), 0o600);
        assert_eq!(mode_of(&root.join("a.txt")), 0o600);
        assert_eq!(mode_of(&root.join("sub").join("b.txt")), 0o600);
        assert_eq!(mode_of(&root.join("sub")), 0o755);

        // One undo puts all three back.
        let command = PermissionsCommand::new(Arc::clone(&fake), label(&r), &r, &applied);
        assert_eq!(command.len(), 3);
        command.undo(&jobs).unwrap_or_else(|e| panic!("{e}"));
        wait_idle(&jobs);
        assert_eq!(mode_of(&root), 0o755);
        assert_eq!(mode_of(&root.join("a.txt")), 0o644);
        assert_eq!(mode_of(&root.join("sub").join("b.txt")), 0o644);

        // Redo applies the recorded change again.
        command.redo(&jobs).unwrap_or_else(|e| panic!("{e}"));
        wait_idle(&jobs);
        assert_eq!(mode_of(&root.join("a.txt")), 0o600);
        assert_eq!(mode_of(&root), 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn a_missing_entry_is_reported_and_the_rest_goes_on() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let file = dir.path().join("f");
        std::fs::write(&file, b"x").unwrap_or_else(|e| panic!("{e}"));
        let fake: Arc<dyn Platform> = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
        let jobs = JobRunner::new();
        let r = request(vec![dir.path().join("gone"), file.clone()], 0o600, None);
        let handle = spawn(&jobs, Arc::clone(&fake), label(&r), r);
        let applied = runtime::block_on(handle.result)
            .unwrap_or_else(|e| panic!("{e}"))
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(applied.previous.len(), 1);
        assert_eq!(applied.failed.len(), 1);
        assert_eq!(applied.failed[0].1, "not found");
    }

    /// Waits for every job of `jobs` to finish.
    fn wait_idle(jobs: &JobRunner) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !jobs.running().is_empty() {
            assert!(std::time::Instant::now() < deadline, "jobs still running");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}
