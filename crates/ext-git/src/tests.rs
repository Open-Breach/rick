use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use ext::{
    Action, ActionCtx, ActionId, ActionOutcome, Availability, CellValue, DecorateCtx, Invocation,
    Level, PaneId, Provider, Registry, UiHost,
};
use runtime::JobRunner;
use types::{CancelToken, Entry, Location};

use super::*;

/// Runs `git` in `dir` for test setup.
fn git(dir: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|e| panic!("git: {e}"));
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// A repository with one commit holding `tracked.txt`, an identity, and a
/// bare origin it pushed to.
fn repo() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let work = dir.path().join("work");
    let origin = dir.path().join("origin.git");
    fs::create_dir(&work).unwrap_or_else(|e| panic!("{e}"));
    git(
        dir.path(),
        &["init", "-q", "--bare", "-b", "main", "origin.git"],
    );
    git(&work, &["init", "-q", "-b", "main"]);
    git(&work, &["config", "user.name", "Rick Test"]);
    git(&work, &["config", "user.email", "rick@example.com"]);
    git(&work, &["config", "commit.gpgsign", "false"]);
    // Line endings as written, so the tests read back what they wrote on
    // Windows too.
    git(&work, &["config", "core.autocrlf", "false"]);
    fs::write(work.join("tracked.txt"), "one\n").unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir(work.join("sub")).unwrap_or_else(|e| panic!("{e}"));
    fs::write(work.join("sub/inner.txt"), "inner\n").unwrap_or_else(|e| panic!("{e}"));
    git(&work, &["add", "-A"]);
    git(&work, &["commit", "-q", "-m", "first commit"]);
    git(
        &work,
        &[
            "remote",
            "add",
            "origin",
            origin.to_str().unwrap_or_default(),
        ],
    );
    git(&work, &["push", "-q", "-u", "origin", "main"]);
    (dir, work)
}

#[derive(Default)]
struct Scripted {
    prompt: Mutex<Option<String>>,
    confirm: Mutex<bool>,
    shown: Mutex<Vec<(String, String)>>,
    /// What a form is answered with, over its defaults.
    typed: Mutex<Vec<(&'static str, ext::Value)>>,
    /// A form is dismissed instead of answered.
    dismiss: Mutex<bool>,
    /// The forms the actions showed.
    forms: Mutex<Vec<ext::Form>>,
    notices: Mutex<Vec<(Level, String)>>,
}

impl Scripted {
    /// Types `text` into the form's field `key`.
    fn type_text(&self, key: &'static str, text: &str) {
        self.typed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((key, ext::Value::Text(text.to_owned())));
    }

    /// The form shown last.
    fn last_form(&self) -> ext::Form {
        self.forms
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .last()
            .cloned()
            .unwrap_or_else(|| panic!("no form was shown"))
    }
}

impl UiHost for Scripted {
    fn notify(&self, level: Level, text: &str) {
        self.notices
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((level, text.to_owned()));
    }
    fn confirm(&self, _: &str, _: &str) -> bool {
        *self.confirm.lock().unwrap_or_else(PoisonError::into_inner)
    }
    fn prompt(&self, _: &str, _: &str) -> Option<String> {
        self.prompt
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
    fn show(&self, title: &str, text: &str) {
        self.shown
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((title.to_owned(), text.to_owned()));
    }
    fn form(&self, form: ext::Form) -> Option<ext::Values> {
        let mut values = form.defaults();
        self.forms
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(form);
        if *self.dismiss.lock().unwrap_or_else(PoisonError::into_inner) {
            return None;
        }
        for (key, value) in self
            .typed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
        {
            values.set(key, value.clone());
        }
        Some(values)
    }
}

fn loc(path: &Path) -> Location {
    Location::new(vfs_local::PROVIDER_ID, platform::paths::from_path(path))
}

fn entries(paths: &[&Path]) -> Vec<Entry> {
    let fake = Arc::new(platform::Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let provider = vfs_local::LocalProvider::new(fake);
    runtime::block_on(async {
        let mut out = Vec::new();
        for path in paths {
            out.push(
                provider
                    .stat(&loc(path))
                    .await
                    .unwrap_or_else(|e| panic!("{e}")),
            );
        }
        out
    })
}

fn ctx(location: &Path, selection: &[&Path]) -> ActionCtx {
    ActionCtx {
        pane: PaneId(1),
        location: loc(location),
        selection: entries(selection),
        invoked_from: Invocation::ContextMenu,
        other: None,
    }
}

struct Harness {
    registry: Registry,
    ext: GitExtension,
    jobs: JobRunner,
    events: Mutex<tokio::sync::mpsc::UnboundedReceiver<runtime::JobEvent>>,
    ui: Scripted,
}

fn harness() -> Harness {
    harness_on(Registry::new())
}

/// A harness whose extension is lent `platform`, for its clipboard.
fn harness_with(platform: Arc<platform::Fake>) -> Harness {
    harness_on(Registry::with_platform(platform))
}

fn harness_on(mut registry: Registry) -> Harness {
    let ext = GitExtension::new();
    registry.load(&ext).unwrap_or_else(|e| panic!("{e}"));
    let (jobs, events) = JobRunner::with_events();
    Harness {
        registry,
        ext,
        jobs,
        events: Mutex::new(events),
        ui: Scripted::default(),
    }
}

impl Harness {
    fn action(&self, id: &'static str) -> Arc<dyn Action> {
        self.registry
            .actions()
            .iter()
            .find(|a| a.id() == ActionId(id))
            .cloned()
            .unwrap_or_else(|| panic!("no action {id}"))
    }

    fn run(&self, id: &'static str, ctx: ActionCtx) -> ActionOutcome {
        let action = self.action(id);
        assert_eq!(action.availability(&ctx), Availability::Enabled, "{id}");
        let outcome = action
            .run(ctx, &self.jobs, &self.ui)
            .unwrap_or_else(|e| panic!("{id}: {e}"));
        self.settle();
        outcome
    }

    /// Waits for every job and fails on the first one that failed.
    fn settle(&self) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !self.jobs.running().is_empty() {
            assert!(Instant::now() < deadline, "jobs still running");
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut events = self.events.lock().unwrap_or_else(PoisonError::into_inner);
        while let Ok(event) = events.try_recv() {
            if let runtime::JobEvent::Finished {
                result: Err(error),
                id,
            } = event
            {
                panic!("job {id:?} failed: {error}");
            }
        }
    }

    fn snapshot(&self, work: &Path) -> Arc<Snapshot> {
        self.ext
            .repos()
            .refresh(work)
            .unwrap_or_else(|e| panic!("{e}"))
    }

    /// Waits for every job and returns what the jobs published, failures
    /// included.
    fn finish(&self) -> Vec<runtime::JobEvent> {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !self.jobs.running().is_empty() {
            assert!(Instant::now() < deadline, "jobs still running");
            std::thread::sleep(Duration::from_millis(10));
        }
        // `Finished` is published a moment after the job leaves the list.
        std::thread::sleep(Duration::from_millis(50));
        let mut events = self.events.lock().unwrap_or_else(PoisonError::into_inner);
        std::iter::from_fn(|| events.try_recv().ok()).collect()
    }

    /// What the actions share, for a clone started without the dialogue.
    fn shared(&self) -> actions::Shared {
        actions::Shared {
            repos: Arc::clone(self.ext.repos()),
            platform: None,
        }
    }
}

/// `path` as a `file://` URL, which git clones like a remote one.
fn file_url(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    if text.starts_with('/') {
        format!("file://{text}")
    } else {
        format!("file:///{text}")
    }
}

/// How the one job of `events` ended.
fn result_of(events: &[runtime::JobEvent]) -> &Result<(), types::Error> {
    events
        .iter()
        .find_map(|event| match event {
            runtime::JobEvent::Finished { result, .. } => Some(result),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no job finished: {events:?}"))
}

/// Waits until `done` holds.
fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !done() {
        assert!(Instant::now() < deadline, "{what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn the_snapshot_reads_branch_upstream_commit_and_statuses() {
    let (_dir, work) = repo();
    fs::write(work.join("tracked.txt"), "two\n").unwrap_or_else(|e| panic!("{e}"));
    fs::write(work.join("new.txt"), "new\n").unwrap_or_else(|e| panic!("{e}"));
    fs::write(work.join("staged.txt"), "staged\n").unwrap_or_else(|e| panic!("{e}"));
    git(&work, &["add", "staged.txt"]);
    fs::write(work.join("sub/inner.txt"), "changed\n").unwrap_or_else(|e| panic!("{e}"));
    fs::remove_file(work.join("sub/inner.txt")).unwrap_or_else(|e| panic!("{e}"));

    let snapshot = Snapshot::read(&work).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(snapshot.branch.as_deref(), Some("main"));
    assert!(!snapshot.detached);
    assert_eq!(snapshot.upstream.as_deref(), Some("origin/main"));
    assert_eq!((snapshot.ahead, snapshot.behind), (0, 0));
    let commit = snapshot
        .last_commit
        .as_ref()
        .unwrap_or_else(|| panic!("commit"));
    assert_eq!(commit.summary, "first commit");
    assert_eq!(commit.author, "Rick Test");
    assert_eq!(commit.date.len(), 10, "{}", commit.date);
    assert_eq!(
        snapshot.status_of("tracked.txt"),
        Some(FileStatus::Modified)
    );
    assert_eq!(snapshot.status_of("new.txt"), Some(FileStatus::Untracked));
    assert_eq!(snapshot.status_of("staged.txt"), Some(FileStatus::Added));
    assert_eq!(
        snapshot.status_of("sub/inner.txt"),
        Some(FileStatus::Deleted)
    );
    assert_eq!(snapshot.status_of("sub"), Some(FileStatus::Contains));
    assert_eq!(snapshot.status_of("sub\\"), Some(FileStatus::Contains));
    assert_eq!(snapshot.status_of("other.txt"), None);

    // A local commit puts the branch ahead.
    git(&work, &["commit", "-q", "-am", "second"]);
    let snapshot = Snapshot::read(&work).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!((snapshot.ahead, snapshot.behind), (1, 0));
    assert_eq!(snapshot.upstream_text(), "1 ahead");
}

#[test]
fn work_trees_are_found_and_cached_until_invalidated() {
    let (dir, work) = repo();
    let repos = Repos::default();
    assert_eq!(repos.work_tree(&work.join("sub")), Some(work.clone()));
    assert_eq!(repos.work_tree(dir.path()), None);
    assert!(repos.snapshot(&work).is_none());
    repos.refresh(&work).unwrap_or_else(|e| panic!("{e}"));
    assert!(repos.snapshot(&work).is_some());
    repos.invalidate(&work.join("sub/inner.txt"));
    assert!(
        repos.snapshot(&work).is_none(),
        "a change inside drops the snapshot"
    );
    repos.refresh(&work).unwrap_or_else(|e| panic!("{e}"));
    repos.invalidate(&work.join(".git"));
    assert!(
        repos.snapshot(&work).is_none(),
        "a .git change drops the snapshot"
    );
}

#[test]
fn the_decorator_answers_pending_first_then_publishes_and_then_from_cache() {
    let (dir, work) = repo();
    fs::write(work.join("tracked.txt"), "two\n").unwrap_or_else(|e| panic!("{e}"));
    let h = harness();
    let decorator = h
        .registry
        .decorators()
        .first()
        .cloned()
        .unwrap_or_else(|| panic!("decorator"));
    assert!(decorator.applies_to(&loc(&work)));
    assert!(!decorator.applies_to(&loc(dir.path())));
    assert_eq!(decorator.columns().len(), 1);
    assert_eq!(
        decorator.invalidates_on(&loc(&work.join("sub"))),
        [loc(&work.join(".git"))]
    );

    let published: Arc<Mutex<Vec<(types::EntryId, ext::Decoration)>>> = Arc::default();
    let sink = Arc::clone(&published);
    let ctx = DecorateCtx::new(
        loc(&work),
        CancelToken::new(),
        h.jobs.clone(),
        move |items| {
            sink.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend(items);
        },
    );
    let batch = entries(&[&work.join("tracked.txt"), &work.join("sub")]);
    let started = Instant::now();
    let first = decorator.decorate(&batch, &ctx);
    assert!(
        started.elapsed() < Duration::from_millis(16),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(first.len(), 2);
    assert!(
        first
            .iter()
            .all(|(_, d)| d.cells.iter().any(|(_, c)| *c == CellValue::Pending))
    );
    h.settle();
    let published = published
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    assert_eq!(published.len(), 2);
    let tracked = &published[0].1;
    assert_eq!(tracked.badge.map(|b| b.glyph), Some("M"));
    assert!(tracked.tint.is_some());
    assert!(
        tracked
            .cells
            .iter()
            .any(|(_, c)| *c == CellValue::Text("Modified".into()))
    );
    let sub = &published[1].1;
    assert_eq!(
        sub.badge.map(|b| b.glyph),
        None,
        "sub has no changes inside"
    );

    let second = decorator.decorate(&batch, &ctx);
    assert_eq!(second, published, "the cached snapshot answers inline");

    let panel = decorator
        .panel(&batch[0])
        .unwrap_or_else(|| panic!("panel"));
    assert_eq!(panel.title, ext::t!("git-git"));
    assert!(
        panel.rows.iter().any(|(k, v)| k == "Branch" && v == "main"),
        "{panel:?}"
    );
    assert!(
        panel
            .rows
            .iter()
            .any(|(k, v)| k == "Status" && v == "Modified"),
        "{panel:?}"
    );
    assert!(decorator.panel(&entries(&[dir.path()])[0]).is_none());
}

#[test]
fn actions_are_hidden_outside_a_work_tree() {
    let (dir, work) = repo();
    let h = harness();
    for action in h.registry.actions() {
        assert_eq!(action.group(), actions::GROUP);
        // Git clone… is the one action that needs no work tree.
        if action.id() == ActionId("git.clone") {
            continue;
        }
        assert_eq!(
            action.availability(&ctx(dir.path(), &[])),
            Availability::Hidden,
            "{}",
            action.id().0
        );
        assert_ne!(
            action.availability(&ctx(&work, &[])),
            Availability::Hidden,
            "{}",
            action.id().0
        );
    }
    assert_eq!(h.registry.actions().len(), 10);
}

#[test]
fn commit_stash_and_discard_change_the_work_tree() {
    let (_dir, work) = repo();
    let h = harness();
    let tracked = work.join("tracked.txt");

    fs::write(&tracked, "two\n").unwrap_or_else(|e| panic!("{e}"));
    *h.ui.prompt.lock().unwrap_or_else(PoisonError::into_inner) = Some("second".into());
    assert!(matches!(
        h.run("git.commit", ctx(&work, &[&tracked])),
        ActionOutcome::Started(_)
    ));
    assert_eq!(git(&work, &["log", "--oneline"]).lines().count(), 2);
    assert_eq!(h.snapshot(&work).status_of("tracked.txt"), None);

    fs::write(&tracked, "three\n").unwrap_or_else(|e| panic!("{e}"));
    h.run("git.stash", ctx(&work, &[]));
    assert_eq!(fs::read_to_string(&tracked).ok().as_deref(), Some("two\n"));
    h.run("git.stash_pop", ctx(&work, &[]));
    assert_eq!(
        fs::read_to_string(&tracked).ok().as_deref(),
        Some("three\n")
    );

    fs::write(work.join("junk.txt"), "x").unwrap_or_else(|e| panic!("{e}"));
    let discard = h.action("git.discard");
    assert!(matches!(
        discard.availability(&ctx(&work, &[])),
        Availability::Disabled(_)
    ));
    *h.ui.confirm.lock().unwrap_or_else(PoisonError::into_inner) = false;
    assert!(matches!(
        h.run("git.discard", ctx(&work, &[&tracked])),
        ActionOutcome::Done
    ));
    assert_eq!(
        fs::read_to_string(&tracked).ok().as_deref(),
        Some("three\n"),
        "declined"
    );
    *h.ui.confirm.lock().unwrap_or_else(PoisonError::into_inner) = true;
    h.run(
        "git.discard",
        ctx(&work, &[&tracked, &work.join("junk.txt")]),
    );
    assert_eq!(fs::read_to_string(&tracked).ok().as_deref(), Some("two\n"));
    assert!(!work.join("junk.txt").exists());
}

#[test]
fn push_pull_and_checkout_talk_to_the_origin() {
    let (dir, work) = repo();
    let h = harness();
    fs::write(work.join("tracked.txt"), "pushed\n").unwrap_or_else(|e| panic!("{e}"));
    git(&work, &["commit", "-q", "-am", "to push"]);
    assert_eq!(h.snapshot(&work).ahead, 1);
    h.run("git.push", ctx(&work, &[]));
    assert_eq!(h.snapshot(&work).ahead, 0);

    // A second clone commits and pushes; the first pulls it.
    let other = dir.path().join("other");
    git(dir.path(), &["clone", "-q", "origin.git", "other"]);
    git(&other, &["config", "user.name", "Other"]);
    git(&other, &["config", "user.email", "other@example.com"]);
    fs::write(other.join("from-other.txt"), "hi\n").unwrap_or_else(|e| panic!("{e}"));
    git(&other, &["add", "-A"]);
    git(&other, &["commit", "-q", "-m", "from other"]);
    git(&other, &["push", "-q"]);
    h.run("git.pull", ctx(&work, &[]));
    assert!(work.join("from-other.txt").exists());

    git(&work, &["branch", "feature"]);
    *h.ui.prompt.lock().unwrap_or_else(PoisonError::into_inner) = Some("feature".into());
    h.run("git.checkout", ctx(&work, &[]));
    assert_eq!(h.snapshot(&work).branch.as_deref(), Some("feature"));
    assert_eq!(h.snapshot(&work).upstream_text(), "no upstream");
}

#[test]
fn log_and_blame_show_text() {
    let (_dir, work) = repo();
    let h = harness();
    let tracked = work.join("tracked.txt");
    let blame = h.action("git.blame");
    assert!(matches!(
        blame.availability(&ctx(&work, &[])),
        Availability::Disabled(_)
    ));
    assert!(matches!(
        blame.availability(&ctx(&work, &[&work.join("sub")])),
        Availability::Disabled(_)
    ));
    assert!(matches!(
        h.run("git.log", ctx(&work, &[])),
        ActionOutcome::Done
    ));
    assert!(matches!(
        h.run("git.blame", ctx(&work, &[&tracked])),
        ActionOutcome::Done
    ));
    let shown =
        h.ui.shown
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
    assert_eq!(shown.len(), 2);
    assert_eq!(shown[0].0, "Git log");
    assert!(shown[0].1.contains("first commit"), "{}", shown[0].1);
    assert!(shown[1].0.starts_with("Blame"));
    assert!(shown[1].1.contains("Rick Test"), "{}", shown[1].1);
}

#[test]
fn a_git_url_is_one_of_the_four_forms_and_nothing_else() {
    for url in [
        "https://github.com/Open-Breach/rick.git",
        "https://github.com/Open-Breach/rick",
        "HTTPS://example.com/team/repo.git/",
        "ssh://git@example.com:2222/team/repo.git",
        "git://example.com/repo.git",
        "git@github.com:Open-Breach/rick.git",
        "deploy@build.example.com:repo",
    ] {
        assert!(clone::is_git_url(url), "{url}");
    }
    for text in [
        "",
        "rick",
        "https://github.com",
        "https://github.com/",
        "https:///rick.git",
        "http://example.com/repo.git",
        "file:///srv/repo.git",
        "ext::sh -c id",
        "ext::sh://x/y",
        "/srv/git/repo.git",
        "C:/work/repo",
        "serge@example.com",
        "git@github.com:",
        "-oProxyCommand=id@host:repo",
        "ssh://-oProxyCommand=id/repo",
        "see https://github.com/Open-Breach/rick.git",
        "https://github.com/Open-Breach/rick.git\nand more",
        "git clone https://github.com/Open-Breach/rick.git",
    ] {
        assert!(!clone::is_git_url(text), "{text:?}");
    }
}

#[test]
fn the_new_folder_is_named_as_git_names_it() {
    for (url, name) in [
        ("https://github.com/Open-Breach/rick.git", "rick"),
        ("https://github.com/Open-Breach/rick", "rick"),
        ("https://example.com/team/repo.git/", "repo"),
        ("https://example.com/team/repo/.git", "repo"),
        ("ssh://git@example.com:2222/team/repo.git", "repo"),
        ("git@github.com:Open-Breach/rick.git", "rick"),
        ("git@example.com:repo.git", "repo"),
        ("git://example.com/a.b.git", "a.b"),
    ] {
        assert_eq!(clone::repo_name(url).as_deref(), Some(name), "{url}");
    }
    for url in [
        "https://github.com",
        "https://example.com/.git",
        "git@host:..",
    ] {
        assert_eq!(clone::repo_name(url), None, "{url}");
    }
}

#[test]
fn the_options_become_gits_flags_and_an_unset_one_adds_nothing() {
    let url = "https://example.com/team/repo.git";
    assert_eq!(
        clone::clone_args(url, &clone::Options::default()),
        ["clone", "--progress", "--", url]
    );
    let all = clone::Options {
        branch: Some("release/1.2".to_owned()),
        depth: Some(5),
        submodules: true,
    };
    assert_eq!(
        clone::clone_args(url, &all),
        [
            "clone",
            "--progress",
            "--branch=release/1.2",
            "--depth=5",
            "--recurse-submodules",
            "--",
            url
        ]
    );
    let depth_only = clone::Options {
        depth: Some(1),
        ..clone::Options::default()
    };
    assert_eq!(
        clone::clone_args(url, &depth_only),
        ["clone", "--progress", "--depth=1", "--", url]
    );
}

#[test]
fn clone_is_offered_on_the_empty_area_and_on_one_folder_only() {
    let (dir, work) = repo();
    let h = harness();
    let clone = h.action("git.clone");
    let file = work.join("tracked.txt");
    let sub = work.join("sub");
    // The empty area and a folder, outside a work tree and inside one.
    for context in [
        ctx(dir.path(), &[]),
        ctx(dir.path(), &[&work]),
        ctx(&work, &[]),
        ctx(&work, &[&sub]),
    ] {
        assert_eq!(clone.availability(&context), Availability::Enabled);
    }
    // A file, and a selection of several: not in the context menu, greyed
    // with the reason in the command palette.
    for context in [ctx(&work, &[&file]), ctx(dir.path(), &[&work, &work])] {
        assert!(
            matches!(clone.availability(&context), Availability::Disabled(_)),
            "{:?}",
            context.selection
        );
    }
    // A location that is not a local folder.
    let mut remote = ctx(dir.path(), &[]);
    remote.location = Location::new(types::ProviderId("ssh"), "/srv");
    assert_eq!(clone.availability(&remote), Availability::Hidden);
}

/// Runs *Git clone…* with the dialogue dismissed and returns the form it
/// showed.
fn clone_form(h: &Harness, context: ActionCtx) -> ext::Form {
    *h.ui.dismiss.lock().unwrap_or_else(PoisonError::into_inner) = true;
    assert!(matches!(h.run("git.clone", context), ActionOutcome::Done));
    h.ui.last_form()
}

#[test]
fn the_dialogue_opens_with_a_git_url_from_the_clipboard_and_blank_otherwise() {
    use platform::Clipboard as _;
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(platform::Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let h = harness_with(Arc::clone(&fake));
    // Nothing on the clipboard.
    assert_eq!(
        clone_form(&h, ctx(dir.path(), &[])).defaults().text("url"),
        ""
    );
    for (clipboard, field) in [
        (
            "https://github.com/Open-Breach/rick.git",
            "https://github.com/Open-Breach/rick.git",
        ),
        // Copied with the line's end.
        (
            "  git@github.com:Open-Breach/rick.git\n",
            "git@github.com:Open-Breach/rick.git",
        ),
        ("just some text", ""),
        ("/home/serge/notes.txt", ""),
        ("git clone https://github.com/Open-Breach/rick.git", ""),
    ] {
        fake.set_text(clipboard).unwrap_or_else(|e| panic!("{e}"));
        let form = clone_form(&h, ctx(dir.path(), &[]));
        assert_eq!(form.defaults().text("url"), field, "{clipboard:?}");
    }
    // Without platform services there is no clipboard to read.
    let bare = harness();
    assert_eq!(
        clone_form(&bare, ctx(dir.path(), &[]))
            .defaults()
            .text("url"),
        ""
    );
}

#[test]
fn the_dialogue_names_the_target_and_the_folder_the_clone_creates() {
    let (dir, work) = repo();
    let h = harness();
    // On a folder the clone lands inside it; on the empty area in the
    // pane's folder.
    for (context, target) in [
        (ctx(dir.path(), &[&work]), work.clone()),
        (ctx(dir.path(), &[]), dir.path().to_path_buf()),
    ] {
        let form = clone_form(&h, context);
        assert_eq!(form.title, "Clone repository");
        assert_eq!(form.ok, "Clone");
        assert_eq!(
            form.header.as_ref().and_then(|h| h.subtitle.clone()),
            Some(format!("Into {}", target.display()))
        );
        assert_eq!(
            form.note.as_deref(),
            Some("Runs as a job — progress in Operations")
        );
        // The options are folded away and all unset.
        let options = form
            .fields
            .iter()
            .find(|f| f.key == "options")
            .unwrap_or_else(|| panic!("no options"));
        assert_eq!(options.label, "Options");
        assert_eq!(
            options.kind,
            ext::FieldKind::Disclosure {
                summary: "Branch, depth, submodules".to_owned(),
                open: false,
                counted: false,
            }
        );
        let mut values = form.defaults();
        assert_eq!(values.text("branch"), "");
        assert_eq!(values.text("depth"), "");
        assert!(!values.check("submodules"));

        // The *Creates* line follows the URL as it is typed.
        let creates = |values: &ext::Values| form.preview_rows(values);
        values.set(
            "url",
            ext::Value::Text("https://example.com/team/tool.git".into()),
        );
        let rows = creates(&values);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].from, "Creates");
        assert_eq!(rows[0].problem, None);
        let made = target.join("tool").display().to_string();
        // A long path keeps its end: the new folder's name.
        assert!(rows[0].to.ends_with("tool"), "{}", rows[0].to);
        assert!(
            rows[0].to == made || rows[0].to.contains('…'),
            "{}",
            rows[0].to
        );
        values.set("url", ext::Value::Text("git@example.com:other".into()));
        assert!(creates(&values)[0].to.ends_with("other"));

        // What is not a Git URL, and a depth that is not a number, hold
        // the Clone button back: a row with a problem does.
        values.set("url", ext::Value::Text("not a url".into()));
        assert!(creates(&values)[0].problem.is_some());
        values.set("url", ext::Value::Text("git@example.com:other".into()));
        for depth in ["0", "-1", "abc", "1.5"] {
            values.set("depth", ext::Value::Text(depth.into()));
            let rows = creates(&values);
            assert_eq!(rows.len(), 2, "{depth}");
            assert!(rows[1].problem.is_some(), "{depth}");
        }
        values.set("depth", ext::Value::Text(" 3 ".into()));
        assert!(creates(&values).iter().all(|row| row.problem.is_none()));
        // An empty URL is the field's own warning, not a problem row.
        values.set("url", ext::Value::Text(String::new()));
        assert!(creates(&values).iter().all(|row| row.problem.is_none()));
        let url = &form.fields[0];
        assert_eq!((url.key, url.mono), ("url", true));
        assert!(url.required.is_some());
    }
}

#[test]
fn what_the_dialogue_would_not_confirm_starts_no_clone() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    for (url, depth, warned) in [
        ("", "", false),
        ("not a url", "", true),
        ("ext::sh -c id", "", true),
        ("https://example.com/team/tool.git", "many", true),
    ] {
        let h = harness();
        h.ui.type_text("url", url);
        h.ui.type_text("depth", depth);
        assert!(matches!(
            h.run("git.clone", ctx(dir.path(), &[])),
            ActionOutcome::Done
        ));
        assert_eq!(h.finish().len(), 0, "{url:?}: no job");
        let notices = h.ui.notices.lock().unwrap_or_else(PoisonError::into_inner);
        assert_eq!(notices.len(), usize::from(warned), "{url:?}");
        assert!(notices.iter().all(|(level, _)| *level == Level::Warning));
    }
    assert_eq!(
        fs::read_dir(dir.path()).map(Iterator::count).ok(),
        Some(0),
        "nothing was created"
    );
}

#[test]
fn a_clone_creates_the_repositorys_folder_in_the_target_as_git_clone_does() {
    let (dir, _work) = repo();
    let url = file_url(&dir.path().join("origin.git"));
    let h = harness();
    let into = dir.path().join("into");
    fs::create_dir(&into).unwrap_or_else(|e| panic!("{e}"));
    let outcome = clone::spawn(
        &h.shared(),
        &h.jobs,
        into.clone(),
        url.clone(),
        clone::Options::default(),
    );
    assert!(matches!(outcome, ActionOutcome::Started(_)));
    let events = h.finish();
    assert!(result_of(&events).is_ok(), "{events:?}");
    // The job is named after the repository and passes on what git says.
    assert!(events.iter().any(|event| matches!(
        event,
        runtime::JobEvent::Started { name, .. } if name == "Git clone origin"
    )));
    assert!(
        events.iter().any(|event| matches!(
            event,
            runtime::JobEvent::Progress { progress: types::Progress::Message(line), .. }
                if line.contains("% (")
        )),
        "git's progress: {events:?}"
    );
    // The repository's folder, inside the target, with its content.
    let cloned = into.join("origin");
    assert_eq!(
        fs::read_to_string(cloned.join("tracked.txt"))
            .ok()
            .as_deref(),
        Some("one\n")
    );
    assert!(cloned.join(".git").is_dir());
    assert_eq!(fs::read_dir(&into).map(Iterator::count).ok(), Some(1));

    // Plain `git clone <url>` in a folder gives the same.
    let plain = dir.path().join("plain");
    fs::create_dir(&plain).unwrap_or_else(|e| panic!("{e}"));
    git(&plain, &["clone", "-q", &url]);
    let names = |folder: &Path| -> Vec<std::ffi::OsString> {
        fs::read_dir(folder)
            .unwrap_or_else(|e| panic!("{e}"))
            .filter_map(|entry| entry.ok().map(|e| e.file_name()))
            .collect()
    };
    assert_eq!(names(&into), names(&plain));
    assert_eq!(
        git(&cloned, &["rev-parse", "HEAD"]),
        git(&plain.join("origin"), &["rev-parse", "HEAD"])
    );
    assert_eq!(
        git(&cloned, &["remote", "get-url", "origin"]),
        git(&plain.join("origin"), &["remote", "get-url", "origin"])
    );
}

#[test]
fn branch_and_depth_reach_git_and_unset_they_change_nothing() {
    let (dir, work) = repo();
    // Two commits on main and two more on a branch of its own.
    for (branch, text) in [
        ("main", "two\n"),
        ("feature", "three\n"),
        ("feature", "four\n"),
    ] {
        if branch == "feature" && text == "three\n" {
            git(&work, &["checkout", "-q", "-b", "feature"]);
        }
        fs::write(work.join("tracked.txt"), text).unwrap_or_else(|e| panic!("{e}"));
        git(&work, &["commit", "-q", "-am", text.trim()]);
        git(&work, &["push", "-q", "-u", "origin", branch]);
    }
    let url = file_url(&dir.path().join("origin.git"));
    let h = harness();
    let clone_into = |name: &str, options: clone::Options| {
        let into = dir.path().join(name);
        fs::create_dir(&into).unwrap_or_else(|e| panic!("{e}"));
        clone::spawn(&h.shared(), &h.jobs, into.clone(), url.clone(), options);
        let events = h.finish();
        assert!(result_of(&events).is_ok(), "{name}: {events:?}");
        into.join("origin")
    };

    // Nothing set: the remote's default branch with its whole history.
    let plain = clone_into("plain", clone::Options::default());
    assert_eq!(
        git(&plain, &["rev-parse", "--abbrev-ref", "HEAD"]).trim(),
        "main"
    );
    assert_eq!(git(&plain, &["rev-list", "--count", "HEAD"]).trim(), "2");
    assert_eq!(
        git(&plain, &["rev-parse", "--is-shallow-repository"]).trim(),
        "false"
    );

    // Branch and depth: that branch, its last commit only.
    let shallow = clone_into(
        "shallow",
        clone::Options {
            branch: Some("feature".to_owned()),
            depth: Some(1),
            // No submodules in this repository: git takes the flag.
            submodules: true,
        },
    );
    assert_eq!(
        git(&shallow, &["rev-parse", "--abbrev-ref", "HEAD"]).trim(),
        "feature"
    );
    assert_eq!(git(&shallow, &["rev-list", "--count", "HEAD"]).trim(), "1");
    assert_eq!(
        fs::read_to_string(shallow.join("tracked.txt"))
            .ok()
            .as_deref(),
        Some("four\n")
    );

    // A branch the remote does not have fails in the job, as git fails.
    let into = dir.path().join("missing");
    fs::create_dir(&into).unwrap_or_else(|e| panic!("{e}"));
    clone::spawn(
        &h.shared(),
        &h.jobs,
        into.clone(),
        url.clone(),
        clone::Options {
            branch: Some("no-such-branch".to_owned()),
            ..clone::Options::default()
        },
    );
    let events = h.finish();
    let error = result_of(&events).as_ref().err().map_or_else(
        || panic!("cloned a branch that is not there"),
        ToString::to_string,
    );
    assert!(error.contains("no-such-branch"), "{error}");
    assert_eq!(fs::read_dir(&into).map(Iterator::count).ok(), Some(0));
}

#[test]
fn an_existing_folder_fails_the_job_as_git_fails_and_is_not_touched() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let taken = dir.path().join("tool");
    fs::create_dir(&taken).unwrap_or_else(|e| panic!("{e}"));
    fs::write(taken.join("mine.txt"), "mine").unwrap_or_else(|e| panic!("{e}"));
    let h = harness();
    // Git refuses the folder before it looks for the host.
    h.ui.type_text("url", "https://example.invalid/team/tool.git");
    let action = h.action("git.clone");
    let outcome = action
        .run(ctx(dir.path(), &[]), &h.jobs, &h.ui)
        .unwrap_or_else(|e| panic!("{e}"));
    // The failure is the job's, not the action's.
    assert!(matches!(outcome, ActionOutcome::Started(_)));
    let events = h.finish();
    let error = result_of(&events)
        .as_ref()
        .err()
        .unwrap_or_else(|| panic!("cloned into a folder that was there"));
    assert!(matches!(error, types::Error::Extension { .. }), "{error:?}");
    assert!(error.to_string().contains("tool"), "{error}");
    assert_eq!(
        fs::read_to_string(taken.join("mine.txt")).ok().as_deref(),
        Some("mine")
    );
    assert_eq!(fs::read_dir(&taken).map(Iterator::count).ok(), Some(1));
}

#[test]
fn cancelling_the_job_stops_git_and_removes_the_folder_it_had_created() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    // A server that takes the connection and never answers: the clone
    // waits for it until it is cancelled.
    let silent = std::net::TcpListener::bind("127.0.0.1:0").unwrap_or_else(|e| panic!("{e}"));
    let port = silent.local_addr().unwrap_or_else(|e| panic!("{e}")).port();
    let h = harness();
    h.ui.type_text("url", &format!("git://127.0.0.1:{port}/team/slow.git"));
    let action = h.action("git.clone");
    let outcome = action
        .run(ctx(dir.path(), &[]), &h.jobs, &h.ui)
        .unwrap_or_else(|e| panic!("{e}"));
    let ActionOutcome::Started(id) = outcome else {
        panic!("no job: {outcome:?}");
    };
    let created = dir.path().join("slow");
    wait_for("git creates the folder", || created.is_dir());
    assert_eq!(h.jobs.running(), vec![id], "the clone waits for the server");

    assert!(h.jobs.cancel(id));
    let events = h.finish();
    assert!(
        matches!(result_of(&events), Err(types::Error::Cancelled)),
        "{events:?}"
    );
    // Where `git` is a launcher (Git for Windows), the git it started is
    // not the process that was stopped: it ends when its server goes away
    // and removes its folder itself.
    let deadline = Instant::now() + Duration::from_secs(10);
    while created.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    if created.exists() {
        eprintln!("note: git outlived the process that was stopped");
    }
    drop(silent);
    wait_for("the folder is removed", || !created.exists());
    assert_eq!(fs::read_dir(dir.path()).map(Iterator::count).ok(), Some(0));
}
