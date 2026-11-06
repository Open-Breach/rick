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
}

impl UiHost for Scripted {
    fn notify(&self, _: Level, _: &str) {}
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
    let ext = GitExtension::new();
    let mut registry = Registry::new();
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
    assert_eq!(panel.title, "Git");
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
    assert_eq!(h.registry.actions().len(), 9);
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
