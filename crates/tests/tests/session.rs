//! Acceptance (session, layouts and windows): the session comes back field
//! by field after an exit and after a crash, a vanished folder opens at its
//! parent with a note, layouts save, switch, start, rename and delete, a
//! second window shares the jobs and the undo log, closing the last window
//! exits with the session saved, and a big session starts about as fast as
//! a small one. The headless tests drive `app` with `Command`s; the kittest
//! ones go through the window.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use app::{App, Command, Config, Event};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use ext::{ActionId, Invocation, Level, UiHost};
use model::{
    Column, Dock, Geometry, PanelSide, Section, SelectionChange, SettingsChange, SidePanelChange,
    SortDir, SplitDir, ViewMode,
};
use platform::Fake;
use tests::local;
use ui_egui::{EguiHost, RickUi};

/// A UI whose answers the script sets in advance.
#[derive(Default)]
struct Scripted {
    prompt: Mutex<Option<String>>,
}

impl UiHost for Scripted {
    fn notify(&self, _level: Level, _text: &str) {}
    fn confirm(&self, _title: &str, _text: &str) -> bool {
        true
    }
    fn prompt(&self, _title: &str, _default: &str) -> Option<String> {
        self.prompt
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
    fn show(&self, _title: &str, _text: &str) {}
    fn form(&self, form: ext::Form) -> Option<ext::Values> {
        Some(form.defaults())
    }
}

struct Driver {
    app: App,
    ui: Arc<Scripted>,
    _fake: Arc<Fake>,
    events: Vec<Event>,
}

impl Driver {
    /// An app over `config`, opening `start` in a tab when given.
    fn new(config: &Path, start: Option<&Path>) -> Self {
        let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
        let ui = Arc::new(Scripted::default());
        let app = App::new(Config {
            platform: fake.clone(),
            ui: ui.clone(),
            extensions: Vec::new(),
            config_dir: Some(config.to_path_buf()),
            start: start.map(local),
        });
        let mut driver = Self {
            app,
            ui,
            _fake: fake,
            events: Vec::new(),
        };
        driver.settle();
        driver
    }

    fn step(&mut self, command: Command) -> Vec<Event> {
        self.events.clear();
        self.app.handle(command, &mut self.events);
        self.settle();
        std::mem::take(&mut self.events)
    }

    fn run(&mut self, pane: model::PaneId, action: &'static str) -> Vec<Event> {
        let ctx = self
            .app
            .action_ctx(pane, Invocation::Shortcut)
            .unwrap_or_else(|| panic!("no active tab"));
        self.step(Command::Run {
            action: ActionId(action),
            ctx,
        })
    }

    /// Waits until nothing has arrived for a while and nothing runs.
    fn settle(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(30);
        runtime::block_on(async {
            loop {
                let quiet = tokio::time::timeout(
                    Duration::from_millis(100),
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

    fn tab(&self, pane: model::PaneId) -> &model::Tab {
        self.app
            .model()
            .active_tab(pane)
            .map_or_else(|| panic!("no active tab"), |(_, tab)| tab)
    }

    fn location(&self, pane: model::PaneId) -> PathBuf {
        platform::paths::to_path(&self.tab(pane).location.path).unwrap_or_else(|e| panic!("{e}"))
    }

    fn names(&self, pane: model::PaneId) -> Vec<String> {
        self.tab(pane)
            .listing
            .rows()
            .map(|e| e.name.clone())
            .collect()
    }

    fn select(&mut self, pane: model::PaneId, names: &[&str]) {
        let ids: Vec<_> = self
            .tab(pane)
            .listing
            .rows()
            .filter(|e| names.contains(&e.name.as_str()))
            .map(|e| e.id)
            .collect();
        assert_eq!(
            ids.len(),
            names.len(),
            "{names:?} in {:?}",
            self.names(pane)
        );
        self.step(Command::Select {
            pane,
            change: SelectionChange::Set(ids),
        });
    }

    /// Waits until the continuous writer has written at least `n` times.
    fn wait_for_writes(&mut self, n: u64) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.app.session_writes() < n {
            assert!(Instant::now() < deadline, "no {n}th session write");
            self.settle();
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// A tree with two folders and a few files.
fn tree(root: &Path) {
    for dir in ["a", "b", "b/deeper"] {
        fs::create_dir_all(root.join(dir)).unwrap_or_else(|e| panic!("{e}"));
    }
    for (i, name) in ["one.txt", "two.txt", "three.txt", "four.txt"]
        .iter()
        .enumerate()
    {
        fs::write(root.join("a").join(name), vec![b'x'; (i + 1) * 100])
            .unwrap_or_else(|e| panic!("{e}"));
        fs::write(root.join(name), name).unwrap_or_else(|e| panic!("{e}"));
    }
}

// Every detail set up, then every detail checked: one long script.
#[allow(clippy::too_many_lines)]
#[test]
fn restore_brings_every_detail_back_and_writes_at_most_once_per_second() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let root = dir.path().join("root");
    tree(&root);
    let config = dir.path().join("config");

    let started = Instant::now();
    let mut d = Driver::new(&config, Some(&root));
    let window = d.app.model().focused_window;
    let pane = d.app.model().focused;
    let (first_tab, _) = d
        .app
        .model()
        .active_tab(pane)
        .unwrap_or_else(|| panic!("tab"));
    d.step(Command::Scroll {
        tab: first_tab,
        offset: 12.5,
    });

    // A second workspace tab with a pane in folder a, sorted by size,
    // filtered, grouped, hidden files on, icons, with a selection and a
    // scroll offset.
    d.step(Command::NewTab(local(&root.join("a"))));
    let second = d.app.model().focused;
    assert_ne!(
        second, pane,
        "a new tab is a new workspace with its own pane"
    );
    assert_eq!(d.app.model().workspaces(window).len(), 2);
    let (second_tab, _) = d
        .app
        .model()
        .active_tab(second)
        .unwrap_or_else(|| panic!("tab"));
    d.step(Command::Sort {
        pane: second,
        by: Column::Size,
        dir: SortDir::Descending,
    });
    d.step(Command::Filter {
        pane: second,
        text: "t".into(),
    });
    d.step(Command::GroupBy {
        pane: second,
        by: Some(Column::Type),
    });
    d.step(Command::ToggleHidden(second));
    d.step(Command::SetView {
        pane: second,
        mode: ViewMode::Icons(model::IconSize::Large),
    });
    d.select(second, &["two.txt", "three.txt"]);
    let selected = d.tab(second).selection.clone();
    d.step(Command::Scroll {
        tab: second_tab,
        offset: 80.0,
    });

    // Back in the first workspace: split right, with a divider the user
    // moved; collapse and move the side panel; the window's place on the
    // screen.
    d.step(Command::Split {
        pane,
        dir: SplitDir::Right,
    });
    let right = d.app.model().focused;
    assert_ne!(right, pane);
    assert_eq!(d.app.model().dock().panes(), [pane, right]);
    let mut layout = d.app.model().dock().clone();
    if let Dock::Split { fraction, .. } = &mut layout {
        *fraction = 0.3;
    }
    d.step(Command::Layout {
        window,
        dock: layout,
    });
    for change in [
        SidePanelChange::Visible(false),
        SidePanelChange::Side(PanelSide::Right),
        SidePanelChange::Width(300),
        SidePanelChange::Fold {
            section: Section::Network,
            folded: true,
        },
    ] {
        d.step(Command::SidePanel { window, change });
    }
    d.step(Command::WindowGeometry {
        window,
        geometry: Geometry {
            x: 10.0,
            y: 20.0,
            width: 900.0,
            height: 600.0,
            maximised: false,
        },
    });
    d.step(Command::FocusPane(pane));

    // Debounced: at most one write per second since the start, plus the
    // first one.
    d.wait_for_writes(1);
    let elapsed = started.elapsed();
    let writes = d.app.session_writes();
    assert!(
        writes <= elapsed.as_secs() + 1,
        "{writes} writes in {elapsed:?}"
    );
    let saved = d.app.session();

    // Exit, then start again over the same config directory.
    d.app.save_session().unwrap_or_else(|e| panic!("{e}"));
    drop(d);
    let d = Driver::new(&config, None);
    assert_eq!(d.app.session(), saved, "the session is back whole");

    let model = d.app.model();
    assert_eq!(model.windows().count(), 1);
    let window = model.focused_window;
    let w = model.window(window).unwrap_or_else(|| panic!("window"));
    assert!(!w.side_panel.visible, "collapsed");
    assert_eq!(w.side_panel.side, PanelSide::Right);
    assert_eq!(w.side_panel.width, 300);
    assert!(w.side_panel.folded.contains(&Section::Network));
    assert_eq!(
        w.geometry.map(|g| (g.x, g.y, g.width, g.height)),
        Some((10.0, 20.0, 900.0, 600.0))
    );
    assert_eq!(w.workspaces.len(), 2, "both workspace tabs");
    assert_eq!(w.active, w.workspaces[0].id, "the first one shown");
    let panes = w.dock().panes();
    assert_eq!(panes.len(), 2, "split right");
    assert!(
        matches!(w.dock(), Dock::Split { dir: SplitDir::Right, fraction, .. } if (fraction - 0.3).abs() < 1e-6)
    );
    assert_eq!(model.focused, panes[0]);

    let (_, first) = model.active_tab(panes[0]).unwrap_or_else(|| panic!("tab"));
    assert_eq!(
        platform::paths::to_path(&first.location.path).ok(),
        Some(root.clone())
    );
    assert!((first.scroll - 12.5).abs() < f32::EPSILON);
    assert_eq!(first.sort.by, Column::Name);
    assert_eq!(d.location(panes[1]), root.clone(), "the split pane too");

    let second = w.workspaces[1].focused;
    let (_, tab) = model.active_tab(second).unwrap_or_else(|| panic!("tab"));
    assert_eq!(d.location(second), root.join("a"));
    assert_eq!(tab.sort.by, Column::Size);
    assert_eq!(tab.sort.dir, SortDir::Descending);
    assert_eq!(tab.filter, "t");
    assert_eq!(tab.group, Some(Column::Type));
    assert!(tab.show_hidden);
    assert_eq!(tab.view, ViewMode::Icons(model::IconSize::Large));
    assert_eq!(tab.selection, selected);
    assert!((tab.scroll - 80.0).abs() < f32::EPSILON);
    assert!(tab.note.is_none());

    // The selected entries are rows again once the listing is in.
    let rows: Vec<String> = d.names(second);
    assert!(rows.contains(&"two.txt".to_owned()), "{rows:?}");
    assert_eq!(d.tab(second).selected().count(), 2);
}

#[test]
fn a_crash_keeps_the_last_written_session() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let root = dir.path().join("root");
    tree(&root);
    let config = dir.path().join("config");
    {
        let mut d = Driver::new(&config, Some(&root));
        let pane = d.app.model().focused;
        d.step(Command::Navigate {
            pane,
            to: local(&root.join("b")),
        });
        d.step(Command::NewTab(local(&root.join("a"))));
        let before = d.app.session_writes();
        d.wait_for_writes(before + 1);
        // No exit hook: the process is gone.
        drop(d);
    }
    let mut d = Driver::new(&config, None);
    let events = std::mem::take(&mut d.events);
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Event::Notify { text, .. } if text.contains("not read"))),
        "{events:?}"
    );
    let pane = d.app.model().focused;
    assert_eq!(
        d.app.model().workspaces(d.app.model().focused_window).len(),
        2,
        "both workspace tabs are back"
    );
    assert_eq!(d.location(pane), root.join("a"));
}

#[test]
fn a_deleted_folder_opens_at_its_parent_with_a_note() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let root = dir.path().join("root");
    tree(&root);
    let config = dir.path().join("config");
    let gone = root.join("b").join("deeper");
    {
        let mut d = Driver::new(&config, Some(&gone));
        d.app.save_session().unwrap_or_else(|e| panic!("{e}"));
    }
    fs::remove_dir_all(root.join("b")).unwrap_or_else(|e| panic!("{e}"));
    let d = Driver::new(&config, None);
    let pane = d.app.model().focused;
    assert_eq!(d.location(pane), root, "the nearest existing ancestor");
    let note = d.tab(pane).note.clone().unwrap_or_default();
    assert!(note.contains("no longer exists"), "{note}");
    assert!(note.contains("deeper"), "{note}");
}

#[test]
fn layouts_save_switch_start_rename_and_delete() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let root = dir.path().join("root");
    tree(&root);
    let config = dir.path().join("config");

    // Run 1: save "Review", change everything, switch back.
    let mut d = Driver::new(&config, Some(&root));
    let pane = d.app.model().focused;
    let window = d.app.model().focused_window;
    d.step(Command::NewTab(local(&root.join("a"))));
    d.step(Command::SaveLayout("Review".into()));
    let review = d.app.session();
    assert_eq!(d.app.layouts(), ["Review"]);
    assert!(config.join("layouts").join("Review.toml").is_file());

    d.step(Command::NewTab(local(&root.join("b"))));
    d.step(Command::Split {
        pane,
        dir: SplitDir::Down,
    });
    d.step(Command::SidePanel {
        window,
        change: SidePanelChange::Visible(false),
    });
    d.step(Command::Sort {
        pane,
        by: Column::Size,
        dir: SortDir::Descending,
    });
    assert_ne!(d.app.session(), review);
    d.step(Command::SwitchLayout(Some("Review".into())));
    assert_eq!(d.app.session(), review, "the model equals the saved layout");
    assert_eq!(d.app.model().windows().count(), 1);
    let pane = d.app.model().focused;
    assert_eq!(d.location(pane), root.join("a"), "and its tabs list");
    assert_eq!(d.names(pane).len(), 4);

    // Set it as the start layout, then leave with something else on screen.
    d.step(Command::Settings(SettingsChange::StartLayout(Some(
        "Review".into(),
    ))));
    assert_eq!(
        d.app.model().settings.session.start_layout(),
        Some("Review")
    );
    d.step(Command::NewTab(local(&root.join("b"))));
    let last = d.app.session();
    assert_ne!(last, review);
    d.app.save_session().unwrap_or_else(|e| panic!("{e}"));
    drop(d);

    // Run 2: the start layout opens; the last session stays reachable.
    let mut d = Driver::new(&config, None);
    assert_eq!(d.app.session(), review, "restart opens the start layout");
    d.step(Command::SwitchLayout(None));
    assert_eq!(d.app.session(), last, "Continue where I left off");
    d.step(Command::SwitchLayout(Some("Review".into())));
    d.step(Command::NewTab(local(&root.join("b").join("deeper"))));
    let changed = d.app.session();
    d.step(Command::Settings(SettingsChange::StartLayout(None)));
    d.app.save_session().unwrap_or_else(|e| panic!("{e}"));
    drop(d);

    // Run 3: start = last opens what run 2 left.
    let mut d = Driver::new(&config, None);
    assert_eq!(
        d.app.session(),
        changed,
        "restart opens the previous session"
    );
    let text = fs::read_to_string(config.join("rick.toml")).unwrap_or_default();
    assert!(text.contains("start = \"last\""), "{text}");

    // Rename follows into the settings; delete clears the start layout.
    d.step(Command::Settings(SettingsChange::StartLayout(Some(
        "Review".into(),
    ))));
    d.step(Command::RenameLayout {
        from: "Review".into(),
        to: "Final".into(),
    });
    assert_eq!(d.app.layouts(), ["Final"]);
    assert_eq!(d.app.model().settings.session.start_layout(), Some("Final"));
    d.step(Command::SwitchLayout(Some("Final".into())));
    assert_eq!(d.app.session(), review);
    d.step(Command::DeleteLayout("Final".into()));
    assert_eq!(d.app.layouts(), Vec::<String>::new());
    assert_eq!(d.app.model().settings.session.start_layout(), None);
    assert!(!config.join("layouts").join("Final.toml").exists());
    let events = d.step(Command::SaveLayout("last".into()));
    assert!(
        events.iter().any(|e| matches!(
            e,
            Event::Notify {
                level: Level::Warning,
                ..
            }
        )),
        "\"last\" is not a layout name: {events:?}"
    );
}

#[test]
fn a_second_window_shares_the_jobs_and_the_undo_log() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let root = dir.path().join("root");
    tree(&root);
    let config = dir.path().join("config");
    let mut d = Driver::new(&config, Some(&root));
    let first = d.app.model().focused_window;
    let pane_1 = d.app.model().focused;

    // With no start layout the new window opens where the focused tab is.
    d.step(Command::NewWindow);
    assert_eq!(d.app.model().windows().count(), 2);
    let second = d.app.model().focused_window;
    assert_ne!(second, first);
    let pane_2 = d.app.model().focused;
    assert_eq!(d.app.model().window_of(pane_2), Some(second));
    assert_eq!(d.location(pane_2), root);
    assert_eq!(d.names(pane_2), d.names(pane_1), "it lists");

    // With a start layout the new window is that layout's window.
    d.step(Command::NewTab(local(&root.join("a"))));
    d.step(Command::SaveLayout("Two tabs".into()));
    d.step(Command::Settings(SettingsChange::StartLayout(Some(
        "Two tabs".into(),
    ))));
    d.step(Command::NewWindow);
    assert_eq!(d.app.model().windows().count(), 3);
    let third = d.app.model().focused_window;
    let w = d
        .app
        .model()
        .window(third)
        .unwrap_or_else(|| panic!("window"));
    assert_eq!(w.workspaces.len(), 2, "the layout's two tabs");
    assert!(d.app.model().pane(w.focused()).is_some());
    d.step(Command::CloseWindow(third));
    assert_eq!(d.app.model().windows().count(), 2);

    // A rename in window 1 shows in the job table every window reads, and
    // Undo from window 2 reverses it.
    d.step(Command::FocusWindow(first));
    assert_eq!(d.app.model().focused, pane_1);
    *d.ui.prompt.lock().unwrap_or_else(PoisonError::into_inner) = Some("renamed.txt".into());
    d.select(pane_1, &["one.txt"]);
    let events = d.run(pane_1, "core.rename");
    assert!(root.join("renamed.txt").exists(), "{events:?}");
    assert!(
        !d.app.model().jobs.is_empty(),
        "the job is in the shared table"
    );
    d.step(Command::FocusWindow(second));
    assert_eq!(d.app.model().focused_window, second);
    // The window shows the workspace opened last, the one at `a`; the
    // first workspace's pane keeps listing behind it.
    let shown = d.app.model().focused;
    assert_ne!(shown, pane_2);
    assert_eq!(d.app.model().window_of(shown), Some(second));
    assert_eq!(d.location(shown), root.join("a"));
    d.step(Command::Undo);
    assert!(root.join("one.txt").exists());
    assert!(!root.join("renamed.txt").exists());
    assert!(d.names(pane_2).contains(&"one.txt".to_owned()));
}

#[test]
fn closing_the_last_window_exits_with_the_session_saved() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let root = dir.path().join("root");
    tree(&root);
    let config = dir.path().join("config");
    let mut d = Driver::new(&config, Some(&root));
    let first = d.app.model().focused_window;
    d.step(Command::NewWindow);
    let second = d.app.model().focused_window;
    d.step(Command::CloseWindow(second));
    assert_eq!(d.app.model().windows().count(), 1);
    assert!(!d.app.exit_requested(), "windows close independently");

    d.step(Command::CloseWindow(first));
    assert!(d.app.exit_requested(), "the last one exits");
    assert_eq!(
        d.app.model().windows().count(),
        1,
        "and stays in the session"
    );
    d.app.save_session().unwrap_or_else(|e| panic!("{e}"));
    let text = fs::read_to_string(d.app.session_path()).unwrap_or_default();
    assert!(text.contains("[[windows]]"), "{text}");
    drop(d);
    let d = Driver::new(&config, None);
    assert_eq!(d.app.model().windows().count(), 1);
    assert_eq!(d.location(d.app.model().focused), root);
}

#[test]
fn a_big_session_starts_about_as_fast_as_a_small_one() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let root = dir.path().join("root");
    tree(&root);

    // Three windows, twenty tabs, written by one run.
    let big = dir.path().join("big");
    {
        let mut d = Driver::new(&big, Some(&root));
        for window in 0..3 {
            if window > 0 {
                d.step(Command::NewWindow);
            }
            for i in 0..6 {
                let sub = if i % 2 == 0 { "a" } else { "b" };
                d.step(Command::NewTab(local(&root.join(sub))));
            }
        }
        assert_eq!(d.app.model().windows().count(), 3);
        let tabs: usize = d.app.model().panes().map(|(_, p)| p.tabs().len()).sum();
        assert!(tabs >= 20, "{tabs} tabs");
        d.app.save_session().unwrap_or_else(|e| panic!("{e}"));
    }
    let small = dir.path().join("small");
    {
        let mut d = Driver::new(&small, Some(&root));
        d.app.save_session().unwrap_or_else(|e| panic!("{e}"));
    }

    let time = |config: &Path| {
        let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
        let ui = Arc::new(Scripted::default());
        let started = Instant::now();
        let app = App::new(Config {
            platform: fake.clone(),
            ui,
            extensions: Vec::new(),
            config_dir: Some(config.to_path_buf()),
            start: None,
        });
        let took = started.elapsed();
        (took, app)
    };
    // Warm the caches once, then measure.
    drop(time(&small));
    let (small_took, small_app) = time(&small);
    assert_eq!(small_app.model().windows().count(), 1);
    let mut bigs = Vec::new();
    for _ in 0..tests::bench::runs() {
        let (big_took, big_app) = time(&big);
        assert_eq!(big_app.model().windows().count(), 3);
        println!("session start: small {small_took:?}, big {big_took:?}");
        assert!(
            big_took <= small_took + Duration::from_millis(300),
            "big {big_took:?} vs small {small_took:?}"
        );
        bigs.push(big_took);
    }
    tests::bench::record("session-start", &bigs);
}

/// A kittest harness whose config lives in `config`, so a second one can
/// start from what the first one saved.
fn harness(start: Option<&Path>, config: &Path) -> Harness<'static, RickUi> {
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = App::new(Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: Vec::new(),
        config_dir: Some(config.to_path_buf()),
        start: start.map(local),
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

#[test]
fn through_the_window_the_layout_scroll_and_side_panel_come_back() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let root = dir.path().join("root");
    tree(&root);
    let many = root.join("many");
    fs::create_dir(&many).unwrap_or_else(|e| panic!("{e}"));
    for i in 0..200 {
        fs::write(many.join(format!("file-{i:03}.txt")), "x").unwrap_or_else(|e| panic!("{e}"));
    }
    let config = dir.path().join("config");
    let session = {
        let mut harness = harness(Some(&many), &config);
        let pane = harness.state().app().model().focused;
        let (tab, _) = harness
            .state()
            .app()
            .model()
            .active_tab(pane)
            .unwrap_or_else(|| panic!("tab"));
        // Scroll the list with the wheel, then split right and collapse the
        // side panel with the keyboard.
        let list = harness
            .state()
            .pane_rects()
            .first()
            .map_or_else(|| panic!("pane rect"), |(_, r)| r.center());
        harness.event(egui::Event::PointerMoved(list));
        for _ in 0..5 {
            harness.event(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -120.0),
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::NONE,
            });
            harness.step();
        }
        harness.step();
        harness.step();
        let scrolled = harness.state().scroll_offset(tab);
        assert!(scrolled > 0.0, "the wheel scrolled the list");
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Backslash);
        harness.step();
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::B);
        harness.step();
        settle(&mut harness);
        let model = harness.state().app().model();
        assert_eq!(model.dock().panes().len(), 2, "split right");
        let window = model.focused_window;
        assert!(
            !model.window(window).is_some_and(|w| w.side_panel.visible),
            "collapsed"
        );
        assert!(
            (model.tab(tab).map_or(0.0, |t| t.scroll) - scrolled).abs() < 0.5,
            "the scroll offset reached the model"
        );
        let session = harness.state().app().session();
        harness
            .state_mut()
            .app_mut()
            .save_session()
            .unwrap_or_else(|e| panic!("{e}"));
        session
    };
    let mut harness = harness(None, &config);
    assert_eq!(harness.state().app().session(), session);
    settle(&mut harness);
    // The scrolled tab is in the left pane; the split's new pane has the
    // focus, as it had before the exit.
    let left = harness.state().app().model().dock().panes()[0];
    let (tab, _) = harness
        .state()
        .app()
        .model()
        .active_tab(left)
        .unwrap_or_else(|| panic!("tab"));
    let wanted = session.windows[0].workspaces[0].panes[0].tabs[0].scroll;
    assert!(wanted > 0.0);
    let deadline = Instant::now() + Duration::from_secs(5);
    while (harness.state().scroll_offset(tab) - wanted).abs() > 0.5 {
        assert!(
            Instant::now() < deadline,
            "the list scrolled back to {wanted}: {}",
            harness.state().scroll_offset(tab)
        );
        harness.step();
    }
    assert!(
        harness.query_by_label("Pinned").is_none(),
        "panel still hidden"
    );
}

#[test]
fn mod_n_opens_a_second_window_and_the_menu_lists_layouts() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let root = dir.path().join("root");
    tree(&root);
    let config = dir.path().join("config");
    let mut harness = harness(Some(&root), &config);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::N);
    harness.step();
    harness.step();
    assert_eq!(harness.state().app().model().windows().count(), 2);
    assert_eq!(harness.state().window_ids().len(), 2);
    settle(&mut harness);

    // Save layout as… asks for a name; the palette lists the layout.
    harness
        .state_mut()
        .app_mut()
        .handle(Command::SaveLayout("Review".into()), &mut Vec::new());
    harness.step();
    harness.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::P,
    );
    harness.step();
    harness.event(egui::Event::Text("Review".to_owned()));
    harness.step();
    harness.get_by_label_contains("Layout: Review");
    harness.key_press(egui::Key::Escape);
    harness.step();
    harness.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::P,
    );
    harness.step();
    harness.event(egui::Event::Text("left off".to_owned()));
    harness.step();
    harness.get_by_label_contains("Continue where I left off");
}
