//! Acceptance (0.2 milestone 5, issue 22): the Subversion extension against
//! generated working copies, the shape of `git.rs`. Every action of the
//! "SVN" group runs headlessly through the app on a temporary working copy;
//! a change in the working copy re-decorates the listing; the context menu
//! shows the group only inside a working copy. Where the `svn` command line
//! is missing the tests print a note and pass, unless `RICK_REQUIRE_SVN`
//! is set.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use app::{App, Command, Config, Event};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use ext::{ActionId, CellValue, Invocation, Level, UiHost};
use ext_svn::{STATUS_COLUMN, SvnExtension};
use model::PaneId;
use platform::{ConfigDir, Fake};
use tests::local;
use types::EntryId;
use ui_egui::{EguiHost, RickUi};

/// Whether `svn` and `svnadmin` are there; without them the tests skip
/// with a note, unless `RICK_REQUIRE_SVN` is set.
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

/// Runs a Subversion tool in `dir` for the set-up.
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

fn svn(dir: &Path, args: &[&str]) -> String {
    let mut full = vec!["--non-interactive"];
    full.extend_from_slice(args);
    run("svn", dir, &full)
}

/// A repository in `dir/repo` with `tracked.txt` and `sub/inner.txt`
/// committed, checked out into `dir/work` and updated to that revision.
fn working_copy(dir: &Path) -> PathBuf {
    run("svnadmin", dir, &["create", "repo"]);
    let url = platform::paths::file_uri(&dir.join("repo"));
    svn(dir, &["checkout", "-q", &url, "work"]);
    let work = dir.join("work");
    fs::write(work.join("tracked.txt"), "one\n").unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir(work.join("sub")).unwrap_or_else(|e| panic!("{e}"));
    fs::write(work.join("sub/inner.txt"), "inner\n").unwrap_or_else(|e| panic!("{e}"));
    svn(&work, &["add", "-q", "tracked.txt", "sub"]);
    svn(&work, &["commit", "-q", "-m", "first commit"]);
    svn(&work, &["update", "-q"]);
    work
}

/// Another checkout of the same repository at `dir/other`.
fn other_checkout(dir: &Path) -> PathBuf {
    let url = platform::paths::file_uri(&dir.join("repo"));
    svn(dir, &["checkout", "-q", &url, "other"]);
    dir.join("other")
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

/// The app with the SVN extension loaded, driven headlessly.
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
            extensions: vec![Arc::new(SvnExtension::new())],
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

    /// The SVN column's text for `name`.
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

/// Every action of the group runs through the app on a temporary working
/// copy and leaves it as `svn` would.
#[test]
fn every_svn_action_runs_headlessly() {
    if !svn_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let work = working_copy(dir.path());
    let mut d = Driver::new(&work);
    assert_eq!(d.status_text("tracked.txt").as_deref(), Some(""));

    // Commit: a change becomes a revision and the badge goes away; an
    // unversioned file in the selection is added on the way.
    fs::write(work.join("tracked.txt"), "two\n").unwrap_or_else(|e| panic!("{e}"));
    fs::write(work.join("new.txt"), "new\n").unwrap_or_else(|e| panic!("{e}"));
    d.step(Command::Refresh(d.pane));
    assert_eq!(d.status_text("tracked.txt").as_deref(), Some("Modified"));
    assert_eq!(d.status_text("new.txt").as_deref(), Some("Unversioned"));
    *d.ui.prompt.lock().unwrap_or_else(PoisonError::into_inner) = Some("second".into());
    d.run("svn.commit", &["tracked.txt", "new.txt"]);
    let revisions = svn(&work, &["log", "-q", "-r", "HEAD:1"]);
    assert_eq!(
        revisions
            .lines()
            .filter(|line| line.starts_with('r'))
            .count(),
        2,
        "{revisions}"
    );
    assert_eq!(d.status_text("tracked.txt").as_deref(), Some(""));
    assert_eq!(d.status_text("new.txt").as_deref(), Some(""));

    // Add: an unversioned file is scheduled for addition.
    fs::write(work.join("later.txt"), "x").unwrap_or_else(|e| panic!("{e}"));
    d.step(Command::Refresh(d.pane));
    d.run("svn.add", &["later.txt"]);
    assert_eq!(d.status_text("later.txt").as_deref(), Some("Added"));

    // Revert, after confirmation: the change and the addition go back.
    fs::write(work.join("tracked.txt"), "three\n").unwrap_or_else(|e| panic!("{e}"));
    d.step(Command::Refresh(d.pane));
    assert_eq!(d.status_text("tracked.txt").as_deref(), Some("Modified"));
    *d.ui.confirm.lock().unwrap_or_else(PoisonError::into_inner) = true;
    d.run("svn.revert", &["tracked.txt", "later.txt"]);
    assert_eq!(
        fs::read_to_string(work.join("tracked.txt")).ok().as_deref(),
        Some("two\n")
    );
    assert_eq!(d.status_text("tracked.txt").as_deref(), Some(""));
    assert_eq!(d.status_text("later.txt").as_deref(), Some("Unversioned"));

    // Update brings in what another working copy committed.
    let other = other_checkout(dir.path());
    fs::write(other.join("from-other.txt"), "hi\n").unwrap_or_else(|e| panic!("{e}"));
    svn(&other, &["add", "-q", "from-other.txt"]);
    svn(&other, &["commit", "-q", "-m", "from other"]);
    d.run("svn.update", &[]);
    assert!(work.join("from-other.txt").exists());
    assert_eq!(d.status_text("from-other.txt").as_deref(), Some(""));

    // Log opens text.
    d.run("svn.log", &[]);
    let shown =
        d.ui.shown
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
    assert_eq!(shown.len(), 1, "{shown:?}");
    assert_eq!(shown[0].0, "SVN log");
    assert!(shown[0].1.contains("from other"), "{}", shown[0].1);
    assert!(shown[0].1.contains("first commit"), "{}", shown[0].1);
}

/// A change in the working copy re-decorates the listing without a
/// refresh, and so does a commit from outside, which touches `.svn`.
#[test]
fn a_change_in_the_working_copy_updates_the_badges() {
    if !svn_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let work = working_copy(dir.path());
    let mut d = Driver::new(&work);
    assert_eq!(d.status_text("tracked.txt").as_deref(), Some(""));
    fs::write(work.join("tracked.txt"), "edited\n").unwrap_or_else(|e| panic!("{e}"));
    let deadline = Instant::now() + Duration::from_secs(10);
    while d.status_text("tracked.txt").as_deref() != Some("Modified") {
        assert!(Instant::now() < deadline, "badge did not follow the change");
        d.settle();
    }
    svn(&work, &["commit", "-q", "-m", "outside"]);
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
        extensions: vec![Arc::new(SvnExtension::new())],
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

/// The context menu has the "SVN" group inside the working copy and not
/// outside; the SVN column follows the same rule.
#[test]
fn the_context_menu_shows_the_svn_group_only_inside_the_working_copy() {
    if !svn_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let plain = dir.path().join("plain");
    fs::create_dir(&plain).unwrap_or_else(|e| panic!("{e}"));
    fs::write(plain.join("file.txt"), "x").unwrap_or_else(|e| panic!("{e}"));
    let work = working_copy(dir.path());
    fs::write(work.join("tracked.txt"), "edited\n").unwrap_or_else(|e| panic!("{e}"));

    let mut harness = harness(&plain);
    assert_eq!(
        harness.query_all_by_label_contains("SVN").count(),
        0,
        "no SVN column outside"
    );
    harness.get_by_label("file.txt").click_secondary();
    harness.step();
    harness.get_by_label_contains("Properties");
    assert_eq!(
        harness.query_all_by_label_contains("SVN").count(),
        0,
        "no SVN group outside"
    );
    harness.key_press(egui::Key::Escape);
    harness.step();
    harness.step();

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
        harness.query_all_by_label_contains("SVN").count(),
        1,
        "the SVN column"
    );
    harness.get_by_label("Modified");
    harness.get_by_label("tracked.txt").click_secondary();
    // Two frames: the menu lays itself out in the first and, taller than
    // the room below the row, moves up to fit the window in the second.
    harness.step();
    harness.step();
    harness.get_by_label_contains("Properties");
    assert_eq!(
        harness.query_all_by_label_contains("SVN").count(),
        2,
        "column and group"
    );
    harness.get_by_label_contains("SVN ⏵").hover();
    harness.step();
    harness.step();
    harness.get_by_label("Update");
    harness.get_by_label("Revert");
    assert!(
        harness.query_by_label("Commit").is_some(),
        "Commit is in the group"
    );
}
