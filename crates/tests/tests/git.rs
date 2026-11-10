//! Acceptance (issue 6): the Git extension against generated repositories.
//! Decorating a visible batch of a 10 000-file repository returns within
//! 16 ms and completes within 500 ms; every action runs headlessly through
//! the app on a temporary repository; the context menu shows the "Git"
//! group only inside a work tree.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use app::{App, Command, Config, Event};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use ext::{ActionId, CellValue, DecorateCtx, Invocation, Level, UiHost};
use ext_git::{GitExtension, STATUS_COLUMN};
use model::PaneId;
use platform::{ConfigDir, Fake};
use runtime::JobRunner;
use tests::local;
use types::{CancelToken, EntryId};
use ui_egui::{EguiHost, RickUi};

/// Runs `git` in `dir` for the set-up.
fn git(dir: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .unwrap_or_else(|e| panic!("git: {e}"));
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Turns `dir` into a repository with one commit of everything in it.
fn init_repo(dir: &Path) {
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "user.name", "Rick Test"]);
    git(dir, &["config", "user.email", "rick@example.com"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
    // Line endings as written, so the tests read back what they wrote on
    // Windows too.
    git(dir, &["config", "core.autocrlf", "false"]);
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "first commit"]);
}

/// A repository `work` with `tracked.txt` and `sub/inner.txt` committed and
/// pushed to a bare `origin.git` next to it.
fn repo_with_origin(dir: &Path) -> PathBuf {
    let work = dir.join("work");
    fs::create_dir(&work).unwrap_or_else(|e| panic!("{e}"));
    fs::write(work.join("tracked.txt"), "one\n").unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir(work.join("sub")).unwrap_or_else(|e| panic!("{e}"));
    fs::write(work.join("sub/inner.txt"), "inner\n").unwrap_or_else(|e| panic!("{e}"));
    init_repo(&work);
    git(dir, &["init", "-q", "--bare", "-b", "main", "origin.git"]);
    let origin = dir.join("origin.git");
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
    work
}

/// A UI whose answers the test sets in advance and that keeps what it was
/// shown.
#[derive(Default)]
struct Scripted {
    prompt: Mutex<Option<String>>,
    confirm: Mutex<bool>,
    shown: Mutex<Vec<(String, String)>>,
    notices: Mutex<Vec<(Level, String)>>,
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
        Some(form.defaults())
    }
}

/// The app with the Git extension loaded, driven headlessly.
struct Driver {
    app: App,
    ui: Arc<Scripted>,
    events: Vec<Event>,
    pane: PaneId,
}

impl Driver {
    fn new(start: &Path) -> Self {
        let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
        let ui = Arc::new(Scripted::default());
        let app = App::new(Config {
            platform: fake.clone(),
            ui: ui.clone(),
            extensions: vec![Arc::new(GitExtension::new())],
            config_dir: Some(fake.config_dir()),
            start: Some(local(start)),
        });
        let pane = app.model().focused;
        let mut driver = Self {
            app,
            ui,
            events: Vec::new(),
            pane,
        };
        driver.settle();
        driver
    }

    /// Sends one command and lets everything it started finish.
    fn step(&mut self, command: Command) {
        self.app.handle(command, &mut self.events);
        self.settle();
    }

    /// Waits until nothing has arrived for a while and nothing runs.
    fn settle(&mut self) {
        let deadline = Instant::now() + Duration::from_mins(1);
        runtime::block_on(async {
            loop {
                let quiet = tokio::time::timeout(
                    Duration::from_millis(200),
                    self.app.tick(&mut self.events),
                )
                .await
                .is_err();
                if quiet && !self.app.is_busy() {
                    break;
                }
                assert!(Instant::now() < deadline, "did not settle: {:?}", self.app);
            }
        });
    }

    fn tab(&self) -> &model::Tab {
        self.app
            .model()
            .active_tab(self.pane)
            .map_or_else(|| panic!("no active tab"), |(_, tab)| tab)
    }

    fn id_of(&self, name: &str) -> EntryId {
        self.tab()
            .listing
            .entries()
            .iter()
            .find(|e| e.name == name)
            .map_or_else(|| panic!("no entry {name}"), |e| e.id)
    }

    /// Selects `names` and runs `action` on the selection.
    fn run(&mut self, action: &'static str, names: &[&str]) {
        let ids: Vec<EntryId> = names.iter().map(|n| self.id_of(n)).collect();
        self.step(Command::Select {
            pane: self.pane,
            change: model::SelectionChange::Set(ids),
        });
        let ctx = self
            .app
            .action_ctx(self.pane, Invocation::ContextMenu)
            .unwrap_or_else(|| panic!("no context"));
        self.step(Command::Run {
            action: ActionId(action),
            ctx,
        });
        let errors: Vec<(Level, String)> = self
            .ui
            .notices
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|(level, _)| *level == Level::Error)
            .cloned()
            .collect();
        assert!(errors.is_empty(), "{action}: {errors:?}");
    }

    /// The Git column's text for `name`.
    fn status_text(&self, name: &str) -> Option<String> {
        let id = self.id_of(name);
        self.tab()
            .decorations
            .get(&id)
            .and_then(|d| d.cells.iter().find(|(c, _)| *c == STATUS_COLUMN))
            .map(|(_, v)| match v {
                CellValue::Text(text) => text.clone(),
                CellValue::Number(n) => n.to_string(),
                CellValue::Pending => "…".to_owned(),
            })
    }
}

/// Decorating a visible batch of a 10 000-file repository returns within
/// 16 ms both with a cached status and without one, and in the latter case
/// the real decorations are published within 500 ms.
#[test]
#[ignore = "generates and commits 10 000 files; run with --ignored in release"]
fn decorating_a_visible_batch_of_a_10k_repository_is_fast() {
    let fixture = tests::fixture::Config {
        entries: 10_000,
        max_depth: 2,
        entries_per_dir: 100,
        sizes: tests::fixture::SizeDistribution::Empty,
        seed: 6,
    }
    .generate()
    .unwrap_or_else(|e| panic!("{e}"));
    let root = fixture.root();
    init_repo(root);
    let mut names: Vec<String> = fs::read_dir(root)
        .unwrap_or_else(|e| panic!("{e}"))
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    let changed = names.first().cloned().unwrap_or_else(|| panic!("no files"));
    fs::write(root.join(&changed), "changed\n").unwrap_or_else(|e| panic!("{e}"));

    // Through the app: the listing lands decorated.
    let driver = Driver::new(root);
    assert_eq!(driver.status_text(&changed).as_deref(), Some("Modified"));

    let decorator = driver
        .app
        .registry()
        .decorators()
        .first()
        .cloned()
        .unwrap_or_else(|| panic!("no decorator"));
    let location = driver.tab().location.clone();
    let batch: Vec<types::Entry> = driver.tab().listing.rows().take(100).cloned().collect();
    assert_eq!(batch.len(), 100);
    let published: Arc<Mutex<Vec<(EntryId, ext::Decoration)>>> = Arc::default();
    let sink = Arc::clone(&published);
    let ctx = DecorateCtx::new(
        location.clone(),
        CancelToken::new(),
        JobRunner::new(),
        move |items| {
            sink.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend(items);
        },
    );

    // Cached: answered inline.
    let mut cached_tooks = Vec::new();
    let mut cached = Vec::new();
    for _ in 0..tests::bench::runs() {
        let started = Instant::now();
        cached = decorator.decorate(&batch, &ctx);
        cached_tooks.push(started.elapsed());
        assert_eq!(cached.len(), 100);
        assert!(
            cached
                .iter()
                .all(|(_, d)| !d.cells.iter().any(|(_, c)| *c == CellValue::Pending))
        );
    }

    // Not cached: Pending now, the real thing within the target; the sink
    // keeps every run's decorations.
    let mut pending_tooks = Vec::new();
    let mut completions = Vec::new();
    for run in 0..tests::bench::runs() {
        decorator.invalidate(&location);
        let started = Instant::now();
        let pending = decorator.decorate(&batch, &ctx);
        pending_tooks.push(started.elapsed());
        assert!(
            pending
                .iter()
                .all(|(_, d)| d.cells.iter().any(|(_, c)| *c == CellValue::Pending))
        );
        let expected = 100 * (run + 1);
        let deadline = started + Duration::from_secs(10);
        loop {
            let count = published
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .len();
            if count >= expected {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "only {count} of {expected} decorations published within 10 s"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        completions.push(started.elapsed());
    }
    println!(
        "10k repository: cached batch {cached_tooks:?}, pending batch {pending_tooks:?}, completion {completions:?}"
    );
    let published = published.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(published[published.len() - 100..], cached[..]);
    tests::bench::record("git-batch-cached", &cached_tooks);
    tests::bench::record("git-batch-pending", &pending_tooks);
    tests::bench::record("git-complete", &completions);
}

/// Every action of the group runs through the app on a temporary
/// repository and leaves the work tree as `git` would.
#[test]
fn every_git_action_runs_headlessly() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let work = repo_with_origin(dir.path());
    let mut d = Driver::new(&work);
    assert_eq!(d.status_text("tracked.txt").as_deref(), Some(""));

    // Commit: a change becomes a commit and the badge goes away.
    fs::write(work.join("tracked.txt"), "two\n").unwrap_or_else(|e| panic!("{e}"));
    d.step(Command::Refresh(d.pane));
    assert_eq!(d.status_text("tracked.txt").as_deref(), Some("Modified"));
    *d.ui.prompt.lock().unwrap_or_else(PoisonError::into_inner) = Some("second".into());
    d.run("git.commit", &["tracked.txt"]);
    assert_eq!(git(&work, &["log", "--oneline"]).lines().count(), 2);
    assert_eq!(d.status_text("tracked.txt").as_deref(), Some(""));

    // Stash and stash pop.
    fs::write(work.join("tracked.txt"), "three\n").unwrap_or_else(|e| panic!("{e}"));
    d.run("git.stash", &[]);
    assert_eq!(
        fs::read_to_string(work.join("tracked.txt")).ok().as_deref(),
        Some("two\n")
    );
    d.run("git.stash_pop", &[]);
    assert_eq!(
        fs::read_to_string(work.join("tracked.txt")).ok().as_deref(),
        Some("three\n")
    );

    // Discard changes, after confirmation.
    fs::write(work.join("junk.txt"), "x").unwrap_or_else(|e| panic!("{e}"));
    d.step(Command::Refresh(d.pane));
    assert_eq!(d.status_text("junk.txt").as_deref(), Some("Untracked"));
    *d.ui.confirm.lock().unwrap_or_else(PoisonError::into_inner) = true;
    d.run("git.discard", &["tracked.txt", "junk.txt"]);
    assert_eq!(
        fs::read_to_string(work.join("tracked.txt")).ok().as_deref(),
        Some("two\n")
    );
    assert!(!work.join("junk.txt").exists());

    // Push, then pull what another clone pushed.
    fs::write(work.join("tracked.txt"), "pushed\n").unwrap_or_else(|e| panic!("{e}"));
    git(&work, &["commit", "-q", "-am", "to push"]);
    d.run("git.push", &[]);
    assert_eq!(
        git(
            dir.path().join("origin.git").as_path(),
            &["log", "--oneline", "main"]
        )
        .lines()
        .count(),
        3
    );
    let other = dir.path().join("other");
    git(dir.path(), &["clone", "-q", "origin.git", "other"]);
    git(&other, &["config", "user.name", "Other"]);
    git(&other, &["config", "user.email", "other@example.com"]);
    fs::write(other.join("from-other.txt"), "hi\n").unwrap_or_else(|e| panic!("{e}"));
    git(&other, &["add", "-A"]);
    git(&other, &["commit", "-q", "-m", "from other"]);
    git(&other, &["push", "-q"]);
    d.run("git.pull", &[]);
    assert!(work.join("from-other.txt").exists());
    assert_eq!(d.status_text("from-other.txt").as_deref(), Some(""));

    // Checkout branch.
    git(&work, &["branch", "feature"]);
    *d.ui.prompt.lock().unwrap_or_else(PoisonError::into_inner) = Some("feature".into());
    d.run("git.checkout", &[]);
    assert_eq!(git(&work, &["branch", "--show-current"]).trim(), "feature");

    // Show log and blame open text.
    d.run("git.log", &[]);
    d.run("git.blame", &["tracked.txt"]);
    let shown =
        d.ui.shown
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
    assert_eq!(shown.len(), 2, "{shown:?}");
    assert_eq!(shown[0].0, "Git log");
    assert!(shown[0].1.contains("from other"), "{}", shown[0].1);
    assert_eq!(shown[1].0, "Blame tracked.txt");
    assert!(shown[1].1.contains("pushed"), "{}", shown[1].1);
}

/// A change in the work tree re-decorates the listing without a refresh.
#[test]
fn a_change_in_the_work_tree_updates_the_badges() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let work = repo_with_origin(dir.path());
    let mut d = Driver::new(&work);
    assert_eq!(d.status_text("tracked.txt").as_deref(), Some(""));
    fs::write(work.join("tracked.txt"), "edited\n").unwrap_or_else(|e| panic!("{e}"));
    let deadline = Instant::now() + Duration::from_secs(10);
    while d.status_text("tracked.txt").as_deref() != Some("Modified") {
        assert!(Instant::now() < deadline, "badge did not follow the change");
        d.settle();
    }
    // A commit from outside touches .git; the badge goes away again.
    git(&work, &["commit", "-q", "-am", "outside"]);
    let deadline = Instant::now() + Duration::from_secs(10);
    while d.status_text("tracked.txt").as_deref() != Some("") {
        assert!(Instant::now() < deadline, "badge did not follow the commit");
        d.settle();
    }
}

fn harness(start: &Path) -> Harness<'static, RickUi> {
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = App::new(Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: vec![Arc::new(GitExtension::new())],
        config_dir: Some(fake.config_dir()),
        start: Some(local(start)),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 720.0))
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    harness
}

/// Runs frames until the app has nothing in flight and a quiet moment
/// passed.
fn settle(harness: &mut Harness<'_, RickUi>) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        for _ in 0..3 {
            harness.step();
        }
        if !harness.state().app().is_busy() {
            std::thread::sleep(Duration::from_millis(50));
            harness.step();
            if !harness.state().app().is_busy() {
                break;
            }
        }
        assert!(Instant::now() < deadline, "did not settle");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The context menu has the "Git" group inside the repository and not
/// outside; the Git column follows the same rule.
#[test]
fn the_context_menu_shows_the_git_group_only_inside_the_repository() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let plain = dir.path().join("plain");
    fs::create_dir(&plain).unwrap_or_else(|e| panic!("{e}"));
    fs::write(plain.join("file.txt"), "x").unwrap_or_else(|e| panic!("{e}"));
    let work = repo_with_origin(dir.path());
    fs::write(work.join("tracked.txt"), "edited\n").unwrap_or_else(|e| panic!("{e}"));

    let mut harness = harness(&plain);
    assert_eq!(
        harness.query_all_by_label_contains("Git").count(),
        0,
        "no Git column outside"
    );
    harness.get_by_label("file.txt").click_secondary();
    harness.step();
    harness.get_by_label_contains("Properties");
    assert_eq!(
        harness.query_all_by_label_contains("Git").count(),
        0,
        "no Git group outside"
    );
    // Escape closes the menu instead of reaching the shell's shortcut.
    harness.key_press(egui::Key::Escape);
    harness.step();
    harness.step();
    assert!(
        harness.query_by_label_contains("Properties").is_none(),
        "the menu closes on Escape"
    );

    let pane = harness.state().app().model().focused;
    harness.state_mut().app_mut().handle(
        Command::Navigate {
            pane,
            to: local(&work),
        },
        &mut Vec::new(),
    );
    settle(&mut harness);
    assert_eq!(
        harness.query_all_by_label_contains("Git").count(),
        1,
        "the Git column"
    );
    // The status cell; the Modified column header carries the same name.
    harness.get_by_role_and_label(egui::accesskit::Role::Label, "Modified");
    harness.get_by_label("tracked.txt").click_secondary();
    // Two frames: the menu lays itself out in the first and, taller than
    // the room below the row, moves up to fit the window in the second.
    harness.step();
    harness.step();
    harness.get_by_label_contains("Properties");
    assert_eq!(
        harness.query_all_by_label_contains("Git").count(),
        2,
        "column and group"
    );
    // The group holds the actions; it opens on hover like every submenu.
    harness.get_by_label_contains("Git ⏵").hover();
    harness.step();
    harness.step();
    harness.get_by_label("Show log");
    harness.get_by_label("Blame");
    assert!(
        harness.query_by_label("Git commit").is_some(),
        "Git commit is in the group"
    );
}
