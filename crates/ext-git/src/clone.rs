//! *Git clone…*: the one action of the group that is offered outside a
//! work tree. On the empty area of a pane it clones into the folder the
//! pane shows, on a folder into that folder. It asks for the repository's
//! URL in a form, prefilled from the clipboard when that holds a Git URL
//! or the path of a local repository, and runs `git clone <url>` in the
//! target folder as a job: git names the new folder as it always does,
//! its progress lines are the job's progress, and cancelling the job stops
//! git.

use std::collections::VecDeque;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use ext::{
    Action, ActionCtx, ActionGroup, ActionId, ActionOutcome, Availability, Field, Form, Header,
    HeaderIcon, JobRunner, Level, PreviewRow, ProgressSink, Shortcut, UiHost, Values, t,
};
use types::{CancelToken, EntryKind, Error, Location, Progress};

use crate::actions::{GROUP, Shared};
use crate::repo::{GitError, to_error};

/// The form's field keys.
const URL: &str = "url";
const OPTIONS: &str = "options";
const BRANCH: &str = "branch";
const DEPTH: &str = "depth";
const SUBMODULES: &str = "submodules";

/// The header's icon, Phosphor's *code*. The dialogue draws a header glyph
/// as the text it is given, so this is the glyph itself, not its name.
const CODE_ICON: &str = "\u{E1BC}";

/// How often the job looks at its cancel token while git says nothing.
const POLL: Duration = Duration::from_millis(100);
/// The lines of git's output kept for the error it ends with.
const KEPT_LINES: usize = 12;
/// How long a stopped git's helpers get to end before the clone's folder
/// is left where it is.
const STOP_WAIT: Duration = Duration::from_secs(5);
/// How often the removal of a stopped clone's folder is tried: a file may
/// stay locked a moment longer than its process lives.
const REMOVE_TRIES: usize = 10;
/// The characters of the *Creates* line the dialogue's card has room for.
const CREATES_WIDTH: usize = 44;
/// The characters of a depth that is not one, shown before what is wrong.
const DEPTH_WIDTH: usize = 6;
/// The characters of a text that is no repository, shown before that.
const REFUSED_WIDTH: usize = 12;

/// The local folder `text` names when it is an absolute path: a rooted
/// one, or on Windows one with its drive or share.
fn absolute(text: &str) -> Option<&Path> {
    let path = Path::new(text);
    path.is_absolute().then_some(path)
}

/// Whether the folder at `path` is a Git repository, a work tree or a bare
/// one: what `git clone <path>` clones from.
fn is_repository(path: &Path) -> bool {
    path.is_dir() && gix::open(path).is_ok()
}

/// Whether `text` is something the dialogue clones from. That is a Git
/// URL — `https://…`, `ssh://…` or `git://…` with a host and a path,
/// `file://…` with a path, or the scp-like `user@host:path` — or a local
/// repository by its absolute path, and in each case one whose last
/// segment names the folder to create ([`repo_name`]).
///
/// A URL is taken by its shape. A local path is taken when a repository is
/// there: paths are on the clipboard all the time, and the field opens
/// with the clipboard's text only when this holds. Nothing else is taken:
/// not `http://`, not a relative path, not another transport.
#[must_use]
pub fn is_git_url(text: &str) -> bool {
    if text.is_empty() || text.chars().any(char::is_control) {
        return false;
    }
    if let Some(folder) = absolute(text) {
        return repo_name(text).is_some() && is_repository(folder);
    }
    if text.chars().any(char::is_whitespace) {
        return false;
    }
    let host = |host: &str| !host.is_empty() && !host.starts_with('-');
    let path = |path: &str| !path.trim_matches('/').is_empty();
    let is = |scheme: &str, known: &[&str]| known.iter().any(|k| scheme.eq_ignore_ascii_case(k));
    let shaped = match text.split_once("://") {
        Some((scheme, rest)) => rest.split_once('/').is_some_and(|(authority, rest)| {
            // A `file://` URL needs no host before its path.
            let named = if is(scheme, &["file"]) {
                !authority.starts_with('-')
            } else {
                is(scheme, &["https", "ssh", "git"]) && host(authority)
            };
            named && path(rest)
        }),
        None => text.split_once(':').is_some_and(|(login, rest)| {
            !login.contains('/')
                && !login.starts_with('-')
                && login
                    .split_once('@')
                    .is_some_and(|(user, at)| !user.is_empty() && host(at))
                && path(rest)
        }),
    };
    shaped && repo_name(text).is_some()
}

/// The folder `git clone <url>` creates: the last segment of the URL's
/// path, or of the local path, without a trailing `.git`, as git derives
/// it. `None` when there is no such segment.
#[must_use]
pub fn repo_name(url: &str) -> Option<String> {
    if let Some(folder) = absolute(url) {
        // `…/rick/.git` is the repository `rick`.
        let named = if folder.file_name().is_some_and(|name| name == ".git") {
            folder.parent()?
        } else {
            folder
        };
        let name = named.file_name()?.to_str()?;
        let name = name.strip_suffix(".git").unwrap_or(name);
        return (!name.is_empty()).then(|| name.to_owned());
    }
    let mut rest = url.trim_end_matches('/');
    if let Some(stem) = rest.strip_suffix("/.git") {
        rest = stem.trim_end_matches('/');
    }
    let path = match rest.split_once("://") {
        Some((_, after)) => after.split_once('/')?.1,
        None => rest.split_once(':').map_or(rest, |(_, path)| path),
    };
    let last = path.rsplit(['/', ':']).next().unwrap_or(path);
    let name = last.strip_suffix(".git").unwrap_or(last);
    (!name.is_empty() && name != "." && name != ".." && !name.contains('\\'))
        .then(|| name.to_owned())
}

/// The dialogue's options, as far as they were set.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Options {
    /// Clone this branch instead of the remote's default: `--branch`.
    pub branch: Option<String>,
    /// Only the last so many commits: `--depth`.
    pub depth: Option<u32>,
    /// Clone the submodules too: `--recurse-submodules`.
    pub submodules: bool,
}

impl Options {
    /// The options a form's values hold; `None` when the depth is not a
    /// number of commits.
    fn from_values(values: &Values) -> Option<Self> {
        let branch = values.text(BRANCH).trim();
        Some(Self {
            branch: (!branch.is_empty()).then(|| branch.to_owned()),
            depth: depth_of(values.text(DEPTH)).ok()?,
            submodules: values.check(SUBMODULES),
        })
    }
}

/// The depth typed into its field: `Ok(None)` while the field is empty,
/// `Err` for anything but a whole number above zero.
fn depth_of(text: &str) -> Result<Option<u32>, ()> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    match text.parse::<u32>() {
        Ok(depth) if depth > 0 => Ok(Some(depth)),
        _ => Err(()),
    }
}

/// The arguments of `git` for a clone of `url` with `options`. An option
/// that is not set adds nothing, so git's defaults apply; `--progress`
/// only makes git report its progress although no terminal is attached.
#[must_use]
pub fn clone_args(url: &str, options: &Options) -> Vec<String> {
    let mut args = vec!["clone".to_owned(), "--progress".to_owned()];
    if let Some(branch) = &options.branch {
        args.push(format!("--branch={branch}"));
    }
    if let Some(depth) = options.depth {
        args.push(format!("--depth={depth}"));
    }
    if options.submodules {
        args.push("--recurse-submodules".to_owned());
    }
    args.push("--".to_owned());
    args.push(url.to_owned());
    args
}

/// Git's output as [`lines_of`] hands it on: a line, and whether it is a
/// progress update.
type Lines = mpsc::Receiver<(String, bool)>;

/// Git's stderr cut into lines, each with whether it is a progress update
/// the next line overwrites (git ends those with `\r`, a line with `\n`).
/// An update is passed on as soon as it is read, not when the next one
/// arrives. The channel closes when git and every process it started have
/// closed the stream, which they do when they end.
fn lines_of(stderr: Option<ChildStderr>) -> Lines {
    let (tx, rx) = mpsc::channel();
    let Some(mut stderr) = stderr else {
        return rx;
    };
    std::thread::spawn(move || {
        let mut chunk = [0_u8; 4096];
        let mut line = Vec::new();
        // A `\r` was read and the byte after it decides what it ended.
        let mut after_cr = false;
        let send = |line: &mut Vec<u8>, update: bool| {
            let text = String::from_utf8_lossy(line).into_owned();
            line.clear();
            text.trim().is_empty() || tx.send((text, update)).is_ok()
        };
        loop {
            let read = match stderr.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(read) => read,
            };
            for &byte in &chunk[..read] {
                let ended = match (after_cr, byte) {
                    // `\r\n` ends a line like `\n` alone.
                    (_, b'\n') => Some(false),
                    (true, _) => Some(true),
                    (false, _) => None,
                };
                after_cr = false;
                if let Some(update) = ended
                    && !send(&mut line, update)
                {
                    return;
                }
                match byte {
                    b'\r' => after_cr = true,
                    b'\n' => {}
                    other => line.push(other),
                }
            }
            // What was read ends in `\r`: an update, shown now.
            if after_cr && !send(&mut line, true) {
                return;
            }
        }
        send(&mut line, after_cr);
    });
    rx
}

/// Takes one line of git's output: it becomes the job's progress, and a
/// line that is not a progress count is kept for the error.
fn note(line: &str, update: bool, said: &mut VecDeque<String>, progress: &ProgressSink) {
    let text = line.split_whitespace().collect::<Vec<_>>().join(" ");
    // A progress count reads `45% (450/1000)` in every language.
    if !update && !text.contains("% (") {
        if said.len() == KEPT_LINES {
            said.pop_front();
        }
        said.push_back(text.clone());
    }
    progress.report(Progress::Message(text));
}

/// Removes what a stopped clone left of the folder it had created: a
/// folder that was not there before git started and that holds git's
/// `.git` now. Anything else under that name is not the clone's and stays.
pub(crate) fn remove_partial(created: &Path) {
    if created.join(".git").symlink_metadata().is_err() {
        return;
    }
    for attempt in 1..=REMOVE_TRIES {
        // Gone, or not a folder: nothing of the clone's is left.
        if !created.symlink_metadata().is_ok_and(|meta| meta.is_dir()) {
            return;
        }
        match std::fs::remove_dir_all(created) {
            Ok(()) => return,
            Err(error) if attempt == REMOVE_TRIES => {
                tracing::warn!(folder = %created.display(), %error, "a cancelled clone's folder was not removed");
            }
            Err(_) => std::thread::sleep(POLL),
        }
    }
}

/// Stops git and removes the folder its clone had created, as git does
/// itself when it is interrupted. The folder is removed once git's output
/// is closed, which is when no process of the clone is left to write into
/// it; where one outlives git (a transfer helper that waits for the
/// network, or the git behind a launcher that was the one stopped) the
/// folder is left alone.
fn stop(child: &mut Child, lines: &Lines, created: Option<&Path>) -> Result<(), Error> {
    // An error means git ended by itself in this moment.
    let _ = child.kill();
    let status = child.wait().map_err(|e| to_error(GitError::Io(e)))?;
    if status.success() {
        return Ok(());
    }
    let deadline = Instant::now() + STOP_WAIT;
    let closed = loop {
        match lines.recv_timeout(POLL) {
            Err(RecvTimeoutError::Disconnected) => break true,
            Ok(_) | Err(RecvTimeoutError::Timeout) if Instant::now() >= deadline => break false,
            Ok(_) | Err(RecvTimeoutError::Timeout) => {}
        }
    };
    if !closed {
        tracing::warn!(
            "git was stopped, but a process it started still runs; the clone's folder is left"
        );
    } else if let Some(created) = created {
        remove_partial(created);
    }
    Err(Error::Cancelled)
}

/// Runs `git clone` for `url` in `target` the way the other actions run
/// git, and waits for it: every line git writes goes to `progress`, and
/// when `cancel` is cancelled git is stopped and the folder the clone had
/// created is removed.
///
/// # Errors
///
/// `Error::Cancelled` when cancelled; `GitError::Command` with what git
/// said when it failed; `GitError::Io` when git could not be run.
pub(crate) fn run(
    target: &Path,
    url: &str,
    options: &Options,
    cancel: &CancelToken,
    progress: &ProgressSink,
) -> Result<(), Error> {
    let io = |error| to_error(GitError::Io(error));
    // Only a folder that is not there yet is the clone's to remove.
    let created = repo_name(url)
        .map(|name| target.join(name))
        .filter(|folder| folder.symlink_metadata().is_err());
    let mut child = Command::new("git")
        .args(clone_args(url, options))
        .current_dir(target)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(io)?;
    let lines = lines_of(child.stderr.take());
    let mut said = VecDeque::new();
    let status = loop {
        if cancel.is_cancelled() {
            return stop(&mut child, &lines, created.as_deref());
        }
        match lines.recv_timeout(POLL) {
            Ok((line, update)) => note(&line, update, &mut said, progress),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => std::thread::sleep(POLL),
        }
        if let Some(status) = child.try_wait().map_err(io)? {
            break status;
        }
    };
    // Git has ended; what it said last is on its way.
    while let Ok((line, update)) = lines.recv_timeout(POLL) {
        note(&line, update, &mut said, progress);
    }
    if status.success() {
        Ok(())
    } else {
        Err(to_error(GitError::Command {
            program: "git clone".to_owned(),
            stderr: Vec::from(said).join("\n"),
        }))
    }
}

/// Starts the clone of `url` into `target` as a job named after the
/// repository.
pub(crate) fn spawn(
    shared: &Shared,
    jobs: &JobRunner,
    target: PathBuf,
    url: String,
    options: Options,
) -> ActionOutcome {
    let name = match repo_name(&url) {
        Some(repo) => format!("Git clone {repo}"),
        None => "Git clone".to_owned(),
    };
    let repos = Arc::clone(&shared.repos);
    let handle = jobs.spawn_named(name.clone(), move |job| async move {
        job.progress.report(Progress::Message(name));
        ext::blocking(move || {
            let result = run(&target, &url, &options, &job.cancel, &job.progress);
            // Inside a work tree the clone is new content there.
            repos.invalidate(&target);
            result
        })
        .await
    });
    ActionOutcome::Started(handle.id)
}

/// `path` with its middle cut out when it is longer than `width`
/// characters, so its end — the new folder's name — stays in view where
/// the card would cut it off.
fn elided(path: &str, width: usize) -> String {
    let chars: Vec<char> = path.chars().collect();
    if chars.len() <= width {
        return path.to_owned();
    }
    let tail = width * 2 / 3;
    let head = width - tail - 1;
    chars[..head]
        .iter()
        .chain(std::iter::once(&'…'))
        .chain(&chars[chars.len() - tail..])
        .collect()
}

/// The card under the URL: the folder the clone creates, following the
/// URL as it is typed, and what keeps the clone from starting.
fn preview(target: &Path, values: &Values) -> Vec<PreviewRow> {
    let creates = |name: &str| elided(&target.join(name).display().to_string(), CREATES_WIDTH);
    let url = values.text(URL).trim();
    let mut rows = vec![match repo_name(url).filter(|_| is_git_url(url)) {
        Some(name) => PreviewRow {
            from: t!("git-clone-creates"),
            to: creates(&name),
            problem: None,
        },
        // Nothing typed yet: the field's own warning asks for the URL.
        None if url.is_empty() => PreviewRow {
            from: t!("git-clone-creates"),
            to: creates("…"),
            problem: None,
        },
        None => PreviewRow {
            from: t!("git-clone-creates"),
            to: elided(url, REFUSED_WIDTH),
            problem: Some(t!("git-clone-not-a-git-url")),
        },
    }];
    if depth_of(values.text(DEPTH)).is_err() {
        rows.push(PreviewRow {
            from: t!("git-clone-depth"),
            to: elided(values.text(DEPTH).trim(), DEPTH_WIDTH),
            problem: Some(t!("git-clone-depth-not-a-number")),
        });
    }
    rows
}

/// The dialogue: the URL, opening with `url` when the clipboard held one,
/// the folder the clone creates, and the options folded away.
pub(crate) fn form(target: &Path, url: Option<String>) -> Form {
    let folder = target.to_path_buf();
    Form::new(t!("git-clone-repository"), t!("git-clone-confirm"))
        .header(
            Header::new(HeaderIcon::Glyph(CODE_ICON))
                .subtitle(t!("git-clone-into", folder = target.display().to_string())),
        )
        .field(
            Field::text(
                URL,
                t!("git-clone-url"),
                url.unwrap_or_default(),
                t!("git-clone-url-hint"),
            )
            .mono()
            .required(t!("git-clone-url-missing")),
        )
        .field(Field::preview_here())
        .field(Field::disclosure(
            OPTIONS,
            t!("git-clone-options"),
            t!("git-clone-options-summary"),
            false,
        ))
        .field(Field::text(
            BRANCH,
            t!("git-clone-branch"),
            "",
            t!("git-clone-branch-hint"),
        ))
        .field(Field::text(
            DEPTH,
            t!("git-clone-depth"),
            "",
            t!("git-clone-depth-hint"),
        ))
        .field(Field::check(SUBMODULES, t!("git-clone-submodules"), false))
        .note(t!("git-clone-runs-as-a-job"))
        .preview_title(t!("git-clone-destination"))
        .preview(Arc::new(move |values| preview(&folder, values)))
}

/// Where a clone would land for a context.
enum Target {
    /// In this folder: the pane's, or the one selected folder.
    Folder(PathBuf),
    /// The selection is not one folder.
    NotAFolder,
    /// The pane shows nothing a clone can land in.
    Elsewhere,
}

fn target_of(ctx: &ActionCtx) -> Target {
    let Some(shown) = crate::local_path(&ctx.location) else {
        return Target::Elsewhere;
    };
    match ctx.selection.as_slice() {
        [] => Target::Folder(shown),
        [one] if one.kind == EntryKind::Dir => {
            crate::local_path(&Location::new(ctx.location.provider, one.path.clone()))
                .map_or(Target::NotAFolder, Target::Folder)
        }
        _ => Target::NotAFolder,
    }
}

/// *Git clone…*: clones a repository into the pane's folder, or into the
/// one folder selected.
pub struct CloneRepository(pub Arc<Shared>);

impl CloneRepository {
    /// The clipboard's text when it is a Git URL or a local repository's
    /// path: what the field opens with.
    fn clipboard_url(&self) -> Option<String> {
        let text = self.0.platform.as_ref()?.text().ok().flatten()?;
        let text = text.trim();
        is_git_url(text).then(|| text.to_owned())
    }
}

impl Action for CloneRepository {
    fn id(&self) -> ActionId {
        ActionId("git.clone")
    }

    fn label(&self) -> String {
        t!("git-clone")
    }

    fn group(&self) -> ActionGroup {
        GROUP
    }

    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }

    fn availability(&self, ctx: &ActionCtx) -> Availability {
        match target_of(ctx) {
            Target::Folder(_) => Availability::Enabled,
            Target::NotAFolder => Availability::Disabled(t!("git-clone-select-one-folder")),
            Target::Elsewhere => Availability::Hidden,
        }
    }

    fn run(
        &self,
        ctx: ActionCtx,
        jobs: &JobRunner,
        ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        let Target::Folder(target) = target_of(&ctx) else {
            return Ok(ActionOutcome::Done);
        };
        let Some(values) = ui.form(form(&target, self.clipboard_url())) else {
            return Ok(ActionOutcome::Done);
        };
        let url = values.text(URL).trim();
        if url.is_empty() {
            return Ok(ActionOutcome::Done);
        }
        // Enter confirms a form whatever its button says, so what the
        // dialogue holds the button back for is looked at again here.
        let refuse = |why: String| {
            ui.notify(Level::Warning, &format!("{}: {why}", self.label()));
            Ok(ActionOutcome::Done)
        };
        if !is_git_url(url) {
            return refuse(t!("git-clone-not-a-git-url"));
        }
        let Some(options) = Options::from_values(&values) else {
            return refuse(t!("git-clone-depth-not-a-number"));
        };
        Ok(spawn(&self.0, jobs, target, url.to_owned(), options))
    }
}
