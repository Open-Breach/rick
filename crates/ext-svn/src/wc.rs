//! Working copies and their status, read through `svn status --xml` and
//! `svn info --xml` and cached until the working copy or its `.svn`
//! directory changes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use ext::Color;
use types::Error;

/// What Subversion says about one path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    /// Content or properties changed.
    Modified,
    /// Scheduled for addition.
    Added,
    /// Scheduled for removal.
    Deleted,
    /// Removed and added again in one go.
    Replaced,
    /// In conflict, or in the way of the working copy (obstructed,
    /// incomplete).
    Conflict,
    /// Versioned but gone from the disk.
    Missing,
    /// Not under version control and not ignored.
    Unversioned,
    /// A directory with changes somewhere inside.
    Contains,
}

impl FileStatus {
    /// The badge glyph: Subversion's own status letters.
    #[must_use]
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Modified => "M",
            Self::Added => "A",
            Self::Deleted => "D",
            Self::Replaced => "R",
            Self::Conflict => "C",
            Self::Missing => "!",
            Self::Unversioned => "?",
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
            Self::Replaced => "Replaced",
            Self::Conflict => "Conflict",
            Self::Missing => "Missing",
            Self::Unversioned => "Unversioned",
            Self::Contains => "Changes inside",
        }
    }

    /// The badge colour.
    #[must_use]
    pub fn color(self) -> Color {
        match self {
            Self::Modified | Self::Contains => Color::rgb(0xE6, 0x7E, 0x22),
            Self::Added | Self::Unversioned => Color::rgb(0x2E, 0x9E, 0x4F),
            Self::Deleted | Self::Conflict | Self::Missing => Color::rgb(0xD3, 0x2F, 0x2F),
            Self::Replaced => Color::rgb(0x1E, 0x6F, 0xD9),
        }
    }

    /// The status for `svn status --xml`'s `item` attribute, with `props`
    /// deciding for an item whose content is unchanged; `None` for what
    /// shows no badge (normal, ignored, external, none).
    #[must_use]
    pub fn from_xml(item: &str, props: &str) -> Option<Self> {
        match item {
            "modified" => Some(Self::Modified),
            "added" => Some(Self::Added),
            "deleted" => Some(Self::Deleted),
            "replaced" => Some(Self::Replaced),
            "conflicted" | "obstructed" | "incomplete" => Some(Self::Conflict),
            "missing" => Some(Self::Missing),
            "unversioned" => Some(Self::Unversioned),
            _ => match props {
                "modified" => Some(Self::Modified),
                "conflicted" => Some(Self::Conflict),
                _ => None,
            },
        }
    }
}

/// The last commit of a working copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    /// The revision.
    pub revision: u64,
    /// Author name.
    pub author: String,
    /// Commit date, `YYYY-MM-DD`.
    pub date: String,
}

/// Everything the decorator shows for one working copy.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    /// The working copy root.
    pub root: PathBuf,
    /// The revision of the working copy root, as `svn info` reports it.
    pub revision: Option<u64>,
    /// The URL the root is checked out from.
    pub url: Option<String>,
    /// The repository root URL.
    pub repository: Option<String>,
    /// The last commit the root knows of.
    pub last_commit: Option<Commit>,
    /// Status by path relative to the root, `/`-separated; directories
    /// with changes inside carry [`FileStatus::Contains`].
    pub files: HashMap<String, FileStatus>,
}

/// Why Subversion could not answer.
#[derive(Debug, thiserror::Error)]
pub enum SvnError {
    /// The location is not inside a working copy.
    #[error("not inside a Subversion working copy")]
    NotAWorkingCopy,
    /// The `svn` command exited non-zero.
    #[error("{program}: {stderr}")]
    Command {
        /// What ran.
        program: String,
        /// What it said.
        stderr: String,
    },
    /// The `svn` command could not be run.
    #[error("svn could not be run: {0}")]
    Io(#[from] std::io::Error),
    /// `svn` printed XML that could not be read.
    #[error("svn output not understood: {0}")]
    Parse(String),
}

/// Wraps a Subversion error as the kernel's extension error.
#[must_use]
pub fn to_error(error: SvnError) -> Error {
    Error::Extension {
        ext: crate::ID,
        source: Box::new(error),
    }
}

/// Runs `svn args` in `dir` and returns its stdout.
///
/// # Errors
///
/// `SvnError::Command` with svn's stderr when it exits non-zero, `Io` when
/// it cannot start.
pub fn svn(dir: &Path, args: &[String]) -> Result<String, SvnError> {
    let output = Command::new("svn")
        .arg("--non-interactive")
        .args(args)
        .current_dir(dir)
        .output()?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        Err(SvnError::Command {
            program: format!("svn {}", args.first().map_or("", String::as_str)),
            stderr: if stderr.is_empty() { stdout } else { stderr },
        })
    }
}

fn strings(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| (*p).to_owned()).collect()
}

impl Snapshot {
    /// Reads the revision, URLs, last commit and statuses of `root`.
    ///
    /// # Errors
    ///
    /// `SvnError::Command` when `svn` refuses, `Parse` when its XML does
    /// not read.
    pub fn read(root: &Path) -> Result<Self, SvnError> {
        let mut snapshot = Self {
            root: root.to_path_buf(),
            ..Self::default()
        };
        let info = svn(root, &strings(&["info", "--xml"]))?;
        snapshot.read_info(&info)?;
        let status = svn(root, &strings(&["status", "--xml"]))?;
        snapshot.read_status(&status)?;
        Ok(snapshot)
    }

    /// Fills the revision, URLs and last commit from `svn info --xml`.
    ///
    /// # Errors
    ///
    /// `SvnError::Parse` when the XML does not read.
    pub fn read_info(&mut self, xml: &str) -> Result<(), SvnError> {
        let doc = roxmltree::Document::parse(xml).map_err(|e| SvnError::Parse(e.to_string()))?;
        let Some(entry) = doc.descendants().find(|n| n.has_tag_name("entry")) else {
            return Ok(());
        };
        self.revision = entry.attribute("revision").and_then(|r| r.parse().ok());
        self.url = entry
            .children()
            .find(|n| n.has_tag_name("relative-url"))
            .or_else(|| entry.children().find(|n| n.has_tag_name("url")))
            .and_then(|n| n.text())
            .map(str::to_owned);
        self.repository = entry
            .children()
            .find(|n| n.has_tag_name("repository"))
            .and_then(|r| r.children().find(|n| n.has_tag_name("root")))
            .and_then(|n| n.text())
            .map(str::to_owned);
        if let Some(commit) = entry.children().find(|n| n.has_tag_name("commit")) {
            let text = |tag: &str| {
                commit
                    .children()
                    .find(|n| n.has_tag_name(tag))
                    .and_then(|n| n.text())
                    .map_or_default(str::to_owned)
            };
            self.last_commit = commit
                .attribute("revision")
                .and_then(|r| r.parse().ok())
                .map(|revision| Commit {
                    revision,
                    author: text("author"),
                    date: text("date").chars().take(10).collect(),
                });
        }
        Ok(())
    }

    /// Fills the statuses from `svn status --xml`.
    ///
    /// # Errors
    ///
    /// `SvnError::Parse` when the XML does not read.
    pub fn read_status(&mut self, xml: &str) -> Result<(), SvnError> {
        let doc = roxmltree::Document::parse(xml).map_err(|e| SvnError::Parse(e.to_string()))?;
        let mut files: HashMap<String, FileStatus> = HashMap::new();
        for entry in doc.descendants().filter(|n| n.has_tag_name("entry")) {
            let Some(path) = entry.attribute("path") else {
                continue;
            };
            let Some(status) = entry.children().find(|n| n.has_tag_name("wc-status")) else {
                continue;
            };
            let item = status.attribute("item").unwrap_or("none");
            let props = status.attribute("props").unwrap_or("none");
            if let Some(status) = FileStatus::from_xml(item, props) {
                files.insert(path.replace('\\', "/"), status);
            }
        }
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

    /// "r12", or "unknown" before `svn info` answered.
    #[must_use]
    pub fn revision_text(&self) -> String {
        self.revision
            .map_or_else(|| "unknown".to_owned(), |r| format!("r{r}"))
    }
}

/// Working copies under the directories seen so far, and their snapshots.
#[derive(Default)]
pub struct WorkingCopies {
    roots: Mutex<HashMap<PathBuf, Option<PathBuf>>>,
    snapshots: Mutex<HashMap<PathBuf, Arc<Snapshot>>>,
}

impl std::fmt::Debug for WorkingCopies {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkingCopies")
            .field("roots", &lock(&self.roots).len())
            .field("snapshots", &lock(&self.snapshots).len())
            .finish()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Plain maps; a panic elsewhere leaves them usable.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The working copy holding `dir`: the nearest ancestor with a `.svn`
/// directory (one per working copy since Subversion 1.7).
#[must_use]
pub fn find_working_copy(dir: &Path) -> Option<PathBuf> {
    let mut current = Some(dir);
    while let Some(candidate) = current {
        if candidate.join(".svn").is_dir() {
            return Some(candidate.to_path_buf());
        }
        current = candidate.parent();
    }
    None
}

impl WorkingCopies {
    /// The working copy holding `dir`, cached.
    #[must_use]
    pub fn working_copy(&self, dir: &Path) -> Option<PathBuf> {
        if let Some(known) = lock(&self.roots).get(dir) {
            return known.clone();
        }
        let found = find_working_copy(dir);
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
    pub fn refresh(&self, root: &Path) -> Result<Arc<Snapshot>, SvnError> {
        let snapshot = Arc::new(Snapshot::read(root)?);
        lock(&self.snapshots).insert(root.to_path_buf(), Arc::clone(&snapshot));
        Ok(snapshot)
    }

    /// Forgets what is known about `path`: the snapshot of the working copy
    /// holding it, and which working copy directories at or below it
    /// belong to.
    pub fn invalidate(&self, path: &Path) {
        lock(&self.snapshots).retain(|root, _| !(path.starts_with(root) || root.starts_with(path)));
        lock(&self.roots).retain(|dir, _| !dir.starts_with(path));
    }
}
