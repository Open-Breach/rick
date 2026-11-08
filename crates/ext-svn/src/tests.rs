use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use ext::{
    Action, ActionCtx, ActionId, ActionOutcome, Availability, CellValue, DecorateCtx, Invocation,
    Level, PaneId, Provider, Registry, UiHost,
};
use runtime::JobRunner;
use types::{CancelToken, Entry, Location};

use super::*;

/// Whether the `svn` and `svnadmin` command lines are there. Without them
/// the tests print a note and pass, unless `RICK_REQUIRE_SVN` is set, in
/// which case they fail: CI installs Subversion and sets it.
fn svn_available() -> bool {
    let present = ["svn", "svnadmin"].iter().all(|tool| {
        std::process::Command::new(tool)
            .args(["--version", "--quiet"])
            .output()
            .is_ok_and(|o| o.status.success())
    });
    if !present {
        assert!(
            std::env::var_os("RICK_REQUIRE_SVN").is_none(),
            "RICK_REQUIRE_SVN is set but svn or svnadmin is missing"
        );
        println!("svn or svnadmin not installed; test skipped");
    }
    present
}

/// Runs `svn` in `dir` for test setup.
fn run(program: &str, dir: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new(program)
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|e| panic!("{program}: {e}"));
    assert!(
        output.status.success(),
        "{program} {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn svn_cmd(dir: &Path, args: &[&str]) -> String {
    let mut full = vec!["--non-interactive"];
    full.extend_from_slice(args);
    run("svn", dir, &full)
}

/// The `file://` URL of a repository directory, as `svn` takes it.
fn repo_url(path: &Path) -> String {
    platform::paths::file_uri(path)
}

/// A repository with one revision holding `tracked.txt` and
/// `sub/inner.txt`, checked out into `work` and updated to it.
fn repo() -> Option<(tempfile::TempDir, PathBuf)> {
    if !svn_available() {
        return None;
    }
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let work = dir.path().join("work");
    run("svnadmin", dir.path(), &["create", "repo"]);
    let url = repo_url(&dir.path().join("repo"));
    svn_cmd(dir.path(), &["checkout", "-q", &url, "work"]);
    fs::write(work.join("tracked.txt"), "one\n").unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir(work.join("sub")).unwrap_or_else(|e| panic!("{e}"));
    fs::write(work.join("sub/inner.txt"), "inner\n").unwrap_or_else(|e| panic!("{e}"));
    svn_cmd(&work, &["add", "-q", "tracked.txt", "sub"]);
    svn_cmd(
        &work,
        &["commit", "-q", "-m", "first commit", "--username", "rick"],
    );
    svn_cmd(&work, &["update", "-q"]);
    Some((dir, work))
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
    ext: SvnExtension,
    jobs: JobRunner,
    events: Mutex<tokio::sync::mpsc::UnboundedReceiver<runtime::JobEvent>>,
    ui: Scripted,
}

fn harness() -> Harness {
    let ext = SvnExtension::new();
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
            .copies()
            .refresh(work)
            .unwrap_or_else(|e| panic!("{e}"))
    }
}

#[test]
fn the_snapshot_reads_revision_urls_commit_and_statuses() {
    let Some((_dir, work)) = repo() else {
        return;
    };
    fs::write(work.join("tracked.txt"), "two\n").unwrap_or_else(|e| panic!("{e}"));
    fs::write(work.join("new.txt"), "new\n").unwrap_or_else(|e| panic!("{e}"));
    fs::write(work.join("added.txt"), "added\n").unwrap_or_else(|e| panic!("{e}"));
    svn_cmd(&work, &["add", "-q", "added.txt"]);
    svn_cmd(&work, &["rm", "-q", "sub/inner.txt"]);

    let snapshot = Snapshot::read(&work).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(snapshot.revision, Some(1));
    assert_eq!(snapshot.revision_text(), "r1");
    assert_eq!(snapshot.url.as_deref(), Some("^/"));
    assert!(
        snapshot
            .repository
            .as_deref()
            .is_some_and(|r| r.starts_with("file://")),
        "{snapshot:?}"
    );
    let commit = snapshot
        .last_commit
        .as_ref()
        .unwrap_or_else(|| panic!("commit"));
    assert_eq!(commit.revision, 1);
    assert!(!commit.author.is_empty());
    assert_eq!(commit.date.len(), 10, "{}", commit.date);
    assert_eq!(
        snapshot.status_of("tracked.txt"),
        Some(FileStatus::Modified)
    );
    assert_eq!(snapshot.status_of("new.txt"), Some(FileStatus::Unversioned));
    assert_eq!(snapshot.status_of("added.txt"), Some(FileStatus::Added));
    assert_eq!(
        snapshot.status_of("sub/inner.txt"),
        Some(FileStatus::Deleted)
    );
    assert_eq!(snapshot.status_of("sub"), Some(FileStatus::Contains));
    assert_eq!(snapshot.status_of("sub\\"), Some(FileStatus::Contains));
    assert_eq!(snapshot.status_of("other.txt"), None);

    // Missing: a versioned file removed from the disk only.
    svn_cmd(&work, &["revert", "-q", "-R", "."]);
    fs::remove_file(work.join("tracked.txt")).unwrap_or_else(|e| panic!("{e}"));
    let snapshot = Snapshot::read(&work).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(snapshot.status_of("tracked.txt"), Some(FileStatus::Missing));
    assert_eq!(snapshot.status_of("sub"), None, "reverted");
}

#[test]
fn the_xml_readers_cover_every_status_without_svn() {
    let mut snapshot = Snapshot::default();
    snapshot
        .read_info(
            r#"<?xml version="1.0"?><info><entry kind="dir" path="." revision="7">
<url>file:///r/trunk</url><relative-url>^/trunk</relative-url>
<repository><root>file:///r</root><uuid>x</uuid></repository>
<commit revision="6"><author>ann</author><date>2026-11-08T03:19:36.568967Z</date></commit>
</entry></info>"#,
        )
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(snapshot.revision, Some(7));
    assert_eq!(snapshot.url.as_deref(), Some("^/trunk"));
    assert_eq!(snapshot.repository.as_deref(), Some("file:///r"));
    assert_eq!(
        snapshot.last_commit,
        Some(Commit {
            revision: 6,
            author: "ann".into(),
            date: "2026-11-08".into()
        })
    );
    snapshot
        .read_status(
            r#"<?xml version="1.0"?><status><target path=".">
<entry path="a"><wc-status item="modified" props="none"/></entry>
<entry path="b"><wc-status item="normal" props="modified"/></entry>
<entry path="c"><wc-status item="replaced" props="none"/></entry>
<entry path="d"><wc-status item="conflicted" props="none"/></entry>
<entry path="e"><wc-status item="missing" props="none"/></entry>
<entry path="f"><wc-status item="ignored" props="none"/></entry>
<entry path="g"><wc-status item="external" props="none"/></entry>
<entry path="deep\dir\h"><wc-status item="obstructed" props="none"/></entry>
<entry path="nothing"><other/></entry>
</target></status>"#,
        )
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(snapshot.status_of("a"), Some(FileStatus::Modified));
    assert_eq!(snapshot.status_of("b"), Some(FileStatus::Modified), "props");
    assert_eq!(snapshot.status_of("c"), Some(FileStatus::Replaced));
    assert_eq!(snapshot.status_of("d"), Some(FileStatus::Conflict));
    assert_eq!(snapshot.status_of("e"), Some(FileStatus::Missing));
    assert_eq!(snapshot.status_of("f"), None, "ignored shows nothing");
    assert_eq!(snapshot.status_of("g"), None, "externals show nothing");
    assert_eq!(snapshot.status_of("deep/dir/h"), Some(FileStatus::Conflict));
    assert_eq!(snapshot.status_of("deep/dir"), Some(FileStatus::Contains));
    assert_eq!(snapshot.status_of("deep"), Some(FileStatus::Contains));
    assert_eq!(snapshot.status_of("nothing"), None);
    assert!(matches!(
        snapshot.read_status("<status><entry"),
        Err(SvnError::Parse(_))
    ));
    for status in [
        FileStatus::Modified,
        FileStatus::Added,
        FileStatus::Deleted,
        FileStatus::Replaced,
        FileStatus::Conflict,
        FileStatus::Missing,
        FileStatus::Unversioned,
        FileStatus::Contains,
    ] {
        assert!(!status.glyph().is_empty());
        assert!(!status.label().is_empty());
    }
}

#[test]
fn working_copies_are_found_and_cached_until_invalidated() {
    let Some((dir, work)) = repo() else {
        return;
    };
    let copies = WorkingCopies::default();
    assert_eq!(copies.working_copy(&work.join("sub")), Some(work.clone()));
    assert_eq!(copies.working_copy(dir.path()), None);
    assert!(copies.snapshot(&work).is_none());
    copies.refresh(&work).unwrap_or_else(|e| panic!("{e}"));
    assert!(copies.snapshot(&work).is_some());
    copies.invalidate(&work.join("sub/inner.txt"));
    assert!(
        copies.snapshot(&work).is_none(),
        "a change inside drops the snapshot"
    );
    copies.refresh(&work).unwrap_or_else(|e| panic!("{e}"));
    copies.invalidate(&work.join(".svn"));
    assert!(
        copies.snapshot(&work).is_none(),
        "a .svn change drops the snapshot"
    );
    assert!(matches!(
        copies.refresh(dir.path()),
        Err(SvnError::Command { .. })
    ));
}

#[test]
fn the_decorator_answers_pending_first_then_publishes_and_then_from_cache() {
    let Some((dir, work)) = repo() else {
        return;
    };
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
    assert_eq!(decorator.columns()[0].label, "SVN");
    assert_eq!(
        decorator.invalidates_on(&loc(&work.join("sub"))),
        [loc(&work.join(".svn"))]
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
    assert_eq!(panel.title, "SVN");
    assert!(
        panel.rows.iter().any(|(k, v)| k == "Revision" && v == "r1"),
        "{panel:?}"
    );
    assert!(
        panel.rows.iter().any(|(k, v)| k == "URL" && v == "^/"),
        "{panel:?}"
    );
    assert!(
        panel
            .rows
            .iter()
            .any(|(k, v)| k == "Last commit" && v.starts_with("r1 by ")),
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

    // A folder with a change inside rolls it up.
    fs::write(work.join("sub/inner.txt"), "changed\n").unwrap_or_else(|e| panic!("{e}"));
    decorator.invalidate(&loc(&work.join(".svn")));
    h.snapshot(&work);
    let again = decorator.decorate(&batch, &ctx);
    assert_eq!(again[1].1.badge.map(|b| b.glyph), Some("•"));
    assert!(
        again[1].1.tint.is_none(),
        "a roll-up keeps the name's colour"
    );
}

#[test]
fn actions_are_hidden_outside_a_working_copy() {
    let Some((dir, work)) = repo() else {
        return;
    };
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
    assert_eq!(h.registry.actions().len(), 5);
    let ids: Vec<&str> = h.registry.actions().iter().map(|a| a.id().0).collect();
    assert_eq!(
        ids,
        [
            "svn.update",
            "svn.commit",
            "svn.revert",
            "svn.log",
            "svn.add"
        ]
    );
}

#[test]
fn commit_add_and_revert_change_the_working_copy() {
    let Some((_dir, work)) = repo() else {
        return;
    };
    let h = harness();
    let tracked = work.join("tracked.txt");

    // Commit of a selection, with an unversioned file added on the way.
    fs::write(&tracked, "two\n").unwrap_or_else(|e| panic!("{e}"));
    fs::write(work.join("new.txt"), "new\n").unwrap_or_else(|e| panic!("{e}"));
    h.snapshot(&work);
    *h.ui.prompt.lock().unwrap_or_else(PoisonError::into_inner) = Some("second".into());
    assert!(matches!(
        h.run("svn.commit", ctx(&work, &[&tracked, &work.join("new.txt")])),
        ActionOutcome::Started(_)
    ));
    svn_cmd(&work, &["update", "-q"]);
    let snapshot = h.snapshot(&work);
    assert_eq!(snapshot.revision, Some(2));
    assert_eq!(snapshot.status_of("tracked.txt"), None);
    assert_eq!(snapshot.status_of("new.txt"), None);

    // An empty message commits nothing.
    fs::write(&tracked, "three\n").unwrap_or_else(|e| panic!("{e}"));
    *h.ui.prompt.lock().unwrap_or_else(PoisonError::into_inner) = Some("  ".into());
    assert!(matches!(
        h.run("svn.commit", ctx(&work, &[])),
        ActionOutcome::Done
    ));
    assert_eq!(
        h.snapshot(&work).status_of("tracked.txt"),
        Some(FileStatus::Modified)
    );

    // Add: the unversioned file is scheduled for addition.
    fs::write(work.join("later.txt"), "x").unwrap_or_else(|e| panic!("{e}"));
    let add = h.action("svn.add");
    assert!(matches!(
        add.availability(&ctx(&work, &[])),
        Availability::Disabled(_)
    ));
    h.run("svn.add", ctx(&work, &[&work.join("later.txt")]));
    assert_eq!(
        h.snapshot(&work).status_of("later.txt"),
        Some(FileStatus::Added)
    );

    // Revert: declined leaves the change, accepted takes it back.
    let revert = h.action("svn.revert");
    assert!(matches!(
        revert.availability(&ctx(&work, &[])),
        Availability::Disabled(_)
    ));
    *h.ui.confirm.lock().unwrap_or_else(PoisonError::into_inner) = false;
    assert!(matches!(
        h.run("svn.revert", ctx(&work, &[&tracked])),
        ActionOutcome::Done
    ));
    assert_eq!(
        fs::read_to_string(&tracked).ok().as_deref(),
        Some("three\n"),
        "declined"
    );
    *h.ui.confirm.lock().unwrap_or_else(PoisonError::into_inner) = true;
    h.run(
        "svn.revert",
        ctx(&work, &[&tracked, &work.join("later.txt")]),
    );
    assert_eq!(fs::read_to_string(&tracked).ok().as_deref(), Some("two\n"));
    assert_eq!(
        h.snapshot(&work).status_of("later.txt"),
        Some(FileStatus::Unversioned),
        "reverting an addition leaves the file"
    );
}

#[test]
fn update_brings_in_what_another_working_copy_committed_and_log_shows_text() {
    let Some((dir, work)) = repo() else {
        return;
    };
    let h = harness();
    let url = repo_url(&dir.path().join("repo"));
    svn_cmd(dir.path(), &["checkout", "-q", &url, "other"]);
    let other = dir.path().join("other");
    fs::write(other.join("from-other.txt"), "hi\n").unwrap_or_else(|e| panic!("{e}"));
    svn_cmd(&other, &["add", "-q", "from-other.txt"]);
    svn_cmd(&other, &["commit", "-q", "-m", "from other"]);
    assert_eq!(h.snapshot(&work).revision, Some(1));
    h.run("svn.update", ctx(&work, &[]));
    assert!(work.join("from-other.txt").exists());
    assert_eq!(h.snapshot(&work).revision, Some(2));

    assert!(matches!(
        h.run("svn.log", ctx(&work, &[])),
        ActionOutcome::Done
    ));
    h.run("svn.log", ctx(&work, &[&work.join("tracked.txt")]));
    let shown =
        h.ui.shown
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
    assert_eq!(shown.len(), 2);
    assert_eq!(shown[0].0, "SVN log");
    assert!(shown[0].1.contains("from other"), "{}", shown[0].1);
    assert!(shown[0].1.contains("first commit"), "{}", shown[0].1);
    assert!(
        !shown[1].1.contains("from other"),
        "the log of one file: {}",
        shown[1].1
    );
}
