//! Work trees and their status, read through gix and cached until the work
//! tree or its `.git` directory changes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use ext::Color;
use gix::bstr::ByteSlice as _;
use types::Error;

/// What Git says about one path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    /// Changed since HEAD, staged or not.
    Modified,
    /// New and staged.
    Added,
    /// Removed.
    Deleted,
    /// Renamed from somewhere else.
    Renamed,
    /// File became a link or the other way round.
    TypeChange,
    /// Not tracked and not ignored.
    Untracked,
    /// Unmerged.
    Conflict,
    /// A directory with changes somewhere inside.
    Contains,
}

impl FileStatus {
    /// The badge glyph.
    #[must_use]
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Modified => "M",
            Self::Added => "A",
            Self::Deleted => "D",
            Self::Renamed => "R",
            Self::TypeChange => "T",
            Self::Untracked => "?",
            Self::Conflict => "!",
            Self::Contains => "•",
        }
    }

    /// The column text.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Modified => "Modified",
            Self::Added => "Added",
            Self::Deleted => "Deleted",
            Self::Renamed => "Renamed",
            Self::TypeChange => "Type changed",
            Self::Untracked => "Untracked",
            Self::Conflict => "Conflict",
            Self::Contains => "Changes inside",
        }
    }

    /// The badge colour.
    #[must_use]
    pub fn color(self) -> Color {
        match self {
            Self::Modified | Self::Contains | Self::TypeChange => Color::rgb(0xE6, 0x7E, 0x22),
            Self::Added | Self::Untracked => Color::rgb(0x2E, 0x9E, 0x4F),
            Self::Deleted | Self::Conflict => Color::rgb(0xD3, 0x2F, 0x2F),
            Self::Renamed => Color::rgb(0x1E, 0x6F, 0xD9),
        }
    }
}

/// The last commit of a work tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    /// Short hash.
    pub id: String,
    /// First line of the message.
    pub summary: String,
    /// Author name.
    pub author: String,
    /// Author date, `YYYY-MM-DD`.
    pub date: String,
}

/// Everything the decorator shows for one work tree.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    /// The work tree.
    pub root: PathBuf,
    /// Branch name, or the short commit hash when detached.
    pub branch: Option<String>,
    /// HEAD points at a commit, not a branch.
    pub detached: bool,
    /// The remote tracking branch, short form.
    pub upstream: Option<String>,
    /// Commits on the branch the upstream lacks.
    pub ahead: usize,
    /// Commits on the upstream the branch lacks.
    pub behind: usize,
    /// HEAD's commit.
    pub last_commit: Option<Commit>,
    /// Status by path relative to the root, `/`-separated; directories with
    /// changes inside carry [`FileStatus::Contains`].
    pub files: HashMap<String, FileStatus>,
}

/// Why Git could not answer.
#[derive(Debug, thiserror::Error)]
pub enum GitError {
    /// The location is not inside a work tree.
    #[error("not inside a Git work tree")]
    NotARepository,
    /// gix refused.
    #[error("{0}")]
    Gix(String),
    /// The `git` command exited non-zero.
    #[error("{program}: {stderr}")]
    Command {
        /// What ran.
        program: String,
        /// What it said.
        stderr: String,
    },
    /// The `git` command could not be run.
    #[error("git could not be run: {0}")]
    Io(#[from] std::io::Error),
}

/// Wraps a Git error as the kernel's extension error.
#[must_use]
pub fn to_error(error: GitError) -> Error {
    Error::Extension {
        ext: crate::ID,
        source: Box::new(error),
    }
}

fn gix_error(error: impl std::fmt::Display) -> GitError {
    GitError::Gix(error.to_string())
}

impl Snapshot {
    /// Reads branch, upstream counts, last commit and statuses of `root`.
    ///
    /// # Errors
    ///
    /// `GitError::Gix` when the repository cannot be opened or its status
    /// read.
    pub fn read(root: &Path) -> Result<Self, GitError> {
        let repo = gix::open(root).map_err(gix_error)?;
        let mut snapshot = Self {
            root: root.to_path_buf(),
            ..Self::default()
        };
        snapshot.read_head(&repo);
        snapshot.read_status(&repo)?;
        Ok(snapshot)
    }

    fn read_head(&mut self, repo: &gix::Repository) {
        match repo.head_ref() {
            Ok(Some(reference)) => {
                self.branch = Some(reference.name().shorten().to_string());
                if let Some(Ok(upstream)) =
                    reference.remote_tracking_ref_name(gix::remote::Direction::Fetch)
                {
                    self.upstream = Some(upstream.shorten().to_string());
                    if let Ok(mut remote) = repo.find_reference(upstream.as_ref())
                        && let Ok(remote_id) = remote.peel_to_id()
                        && let Ok(local_id) = repo.head_id()
                    {
                        let (remote_id, local_id) = (remote_id.detach(), local_id.detach());
                        self.ahead = count_only(repo, local_id, remote_id);
                        self.behind = count_only(repo, remote_id, local_id);
                    }
                }
            }
            Ok(None) => {
                self.detached = true;
                self.branch = repo.head_id().ok().map(|id| {
                    id.shorten()
                        .map_or_else(|_| id.to_string(), |prefix| prefix.to_string())
                });
            }
            Err(_) => {}
        }
        if let Ok(commit) = repo.head_commit() {
            let summary = commit
                .message()
                .map(|m| m.summary().to_string())
                .unwrap_or_default();
            let author = commit
                .author()
                .map(|a| a.name.to_string())
                .unwrap_or_default();
            let date = commit
                .time()
                .map(|t| t.format_or_unix(gix::date::time::format::SHORT))
                .unwrap_or_default();
            let id = commit
                .id()
                .shorten()
                .map_or_else(|_| commit.id().to_string(), |p| p.to_string());
            self.last_commit = Some(Commit {
                id,
                summary,
                author,
                date,
            });
        }
    }

    fn read_status(&mut self, repo: &gix::Repository) -> Result<(), GitError> {
        use gix::status::index_worktree::Item as Worktree;
        use gix::status::plumbing::index_as_worktree::{Change, EntryStatus};

        let platform = repo
            .status(gix::progress::Discard)
            .map_err(gix_error)?
            .untracked_files(gix::status::UntrackedFiles::Files)
            .dirwalk_options(|options| {
                options.emit_untracked(gix::dir::walk::EmissionMode::Matching)
            });
        let items = platform
            .into_iter(Vec::<gix::bstr::BString>::new())
            .map_err(gix_error)?;
        let mut staged: HashMap<String, FileStatus> = HashMap::new();
        let mut worktree: HashMap<String, FileStatus> = HashMap::new();
        for item in items {
            let item = item.map_err(gix_error)?;
            match item {
                gix::status::Item::TreeIndex(change) => {
                    let status = match &change {
                        gix::diff::index::ChangeRef::Addition { .. } => FileStatus::Added,
                        gix::diff::index::ChangeRef::Deletion { .. } => FileStatus::Deleted,
                        gix::diff::index::ChangeRef::Modification { .. } => FileStatus::Modified,
                        gix::diff::index::ChangeRef::Rewrite { .. } => FileStatus::Renamed,
                    };
                    staged.insert(change.location().to_str_lossy().into_owned(), status);
                }
                gix::status::Item::IndexWorktree(Worktree::Modification {
                    rela_path,
                    status,
                    ..
                }) => {
                    let status = match status {
                        EntryStatus::Conflict { .. } => Some(FileStatus::Conflict),
                        EntryStatus::Change(Change::Removed) => Some(FileStatus::Deleted),
                        EntryStatus::Change(Change::Type { .. }) => Some(FileStatus::TypeChange),
                        EntryStatus::Change(
                            Change::Modification { .. } | Change::SubmoduleModification(_),
                        ) => Some(FileStatus::Modified),
                        EntryStatus::IntentToAdd => Some(FileStatus::Added),
                        EntryStatus::NeedsUpdate(_) => None,
                    };
                    if let Some(status) = status {
                        worktree.insert(rela_path.to_str_lossy().into_owned(), status);
                    }
                }
                gix::status::Item::IndexWorktree(Worktree::DirectoryContents { entry, .. }) => {
                    if entry.status == gix::dir::entry::Status::Untracked {
                        worktree.insert(
                            entry.rela_path.to_str_lossy().into_owned(),
                            FileStatus::Untracked,
                        );
                    }
                }
                gix::status::Item::IndexWorktree(Worktree::Rewrite { dirwalk_entry, .. }) => {
                    worktree.insert(
                        dirwalk_entry.rela_path.to_str_lossy().into_owned(),
                        FileStatus::Renamed,
                    );
                }
            }
        }
        // What the work tree says wins over what is merely staged.
        let mut files = staged;
        files.extend(worktree);
        // Directories carry a mark when something inside changed.
        let dirs: Vec<String> = files
            .keys()
            .flat_map(|path| {
                let mut out = Vec::new();
                let mut end = path.len();
                while let Some(slash) = path[..end].rfind('/') {
                    out.push(path[..slash].to_owned());
                    end = slash;
                }
                out
            })
            .collect();
        for dir in dirs {
            files.entry(dir).or_insert(FileStatus::Contains);
        }
        self.files = files;
        Ok(())
    }

    /// The status of a path relative to the root, with either separator.
    #[must_use]
    pub fn status_of(&self, rela: &str) -> Option<FileStatus> {
        let key = rela.replace('\\', "/");
        self.files.get(key.trim_end_matches('/')).copied()
    }

    /// "2 ahead, 1 behind", "up to date", or "no upstream".
    #[must_use]
    pub fn upstream_text(&self) -> String {
        match (&self.upstream, self.ahead, self.behind) {
            (None, _, _) => "no upstream".to_owned(),
            (Some(_), 0, 0) => "up to date".to_owned(),
            (Some(_), ahead, 0) => format!("{ahead} ahead"),
            (Some(_), 0, behind) => format!("{behind} behind"),
            (Some(_), ahead, behind) => format!("{ahead} ahead, {behind} behind"),
        }
    }
}

/// Commits reachable from `tip` but not from `hidden`.
fn count_only(repo: &gix::Repository, tip: gix::ObjectId, hidden: gix::ObjectId) -> usize {
    repo.rev_walk([tip])
        .with_hidden([hidden])
        .all()
        .map_or(0, Iterator::count)
}

/// Work trees under the directories seen so far, and their snapshots.
#[derive(Default)]
pub struct Repos {
    roots: Mutex<HashMap<PathBuf, Option<PathBuf>>>,
    snapshots: Mutex<HashMap<PathBuf, Arc<Snapshot>>>,
}

impl std::fmt::Debug for Repos {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Repos")
            .field("roots", &lock(&self.roots).len())
            .field("snapshots", &lock(&self.snapshots).len())
            .finish()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Plain maps; a panic elsewhere leaves them usable.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The work tree holding `dir`: the nearest ancestor with a `.git` entry.
#[must_use]
pub fn find_work_tree(dir: &Path) -> Option<PathBuf> {
    let mut current = Some(dir);
    while let Some(candidate) = current {
        if candidate.join(".git").exists() {
            return Some(candidate.to_path_buf());
        }
        current = candidate.parent();
    }
    None
}

impl Repos {
    /// The work tree holding `dir`, cached.
    #[must_use]
    pub fn work_tree(&self, dir: &Path) -> Option<PathBuf> {
        if let Some(known) = lock(&self.roots).get(dir) {
            return known.clone();
        }
        let found = find_work_tree(dir);
        lock(&self.roots).insert(dir.to_path_buf(), found.clone());
        found
    }

    /// The cached snapshot of `root`, if any.
    #[must_use]
    pub fn snapshot(&self, root: &Path) -> Option<Arc<Snapshot>> {
        lock(&self.snapshots).get(root).cloned()
    }

    /// Reads `root` again and caches the result.
    ///
    /// # Errors
    ///
    /// See [`Snapshot::read`].
    pub fn refresh(&self, root: &Path) -> Result<Arc<Snapshot>, GitError> {
        let snapshot = Arc::new(Snapshot::read(root)?);
        lock(&self.snapshots).insert(root.to_path_buf(), Arc::clone(&snapshot));
        Ok(snapshot)
    }

    /// Forgets what is known about `path`: the snapshot of the work tree
    /// holding it, and which work tree directories at or below it belong to.
    pub fn invalidate(&self, path: &Path) {
        lock(&self.snapshots).retain(|root, _| !(path.starts_with(root) || root.starts_with(path)));
        lock(&self.roots).retain(|dir, _| !dir.starts_with(path));
    }
}
