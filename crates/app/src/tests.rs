use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use ext::{ActionId, Invocation, Level, UiHost};
use model::{Column, SelectionChange, SettingsChange, SortDir};
use platform::{ConfigDir, Fake};
use types::Location;

use super::{App, Command, Config, Event, SETTINGS_FILE};

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
}

fn local(path: &Path) -> Location {
    Location::new(vfs_local::PROVIDER_ID, platform::paths::from_path(path))
}

struct Harness {
    root: tempfile::TempDir,
    fake: Arc<Fake>,
    ui: Arc<Scripted>,
    app: App,
    events: Vec<Event>,
}

fn harness() -> Harness {
    let root = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    for name in ["b.txt", "a.txt", "c.txt"] {
        fs::write(root.path().join(name), name).unwrap_or_else(|e| panic!("{e}"));
    }
    fs::create_dir(root.path().join("sub")).unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let ui = Arc::new(Scripted::default());
    let app = App::new(Config {
        platform: fake.clone(),
        ui: ui.clone(),
        extensions: Vec::new(),
        config_dir: Some(fake.config_dir()),
        start: local(root.path()),
    });
    Harness {
        root,
        fake,
        ui,
        app,
        events: Vec::new(),
    }
}

impl Harness {
    fn send(&mut self, command: Command) {
        self.app.handle(command, &mut self.events);
    }

    /// Runs the loop until nothing has arrived for a while and nothing runs.
    fn settle(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        runtime::block_on(async {
            loop {
                let quiet = tokio::time::timeout(
                    Duration::from_millis(80),
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

    fn names(&self) -> Vec<String> {
        let pane = self.app.model().focused;
        let (_, tab) = self
            .app
            .model()
            .active_tab(pane)
            .unwrap_or_else(|| panic!("tab"));
        tab.listing.rows().map(|e| e.name.clone()).collect()
    }

    fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }
}

#[test]
fn start_lists_the_start_location_and_reports_done() {
    let mut h = harness();
    h.settle();
    assert_eq!(h.names(), ["sub", "a.txt", "b.txt", "c.txt"]);
    let events = h.take_events();
    let listed: Vec<(usize, bool)> = events
        .iter()
        .filter_map(|e| match e {
            Event::Listed { batch, done, .. } => Some((batch.len(), *done)),
            _ => None,
        })
        .collect();
    assert_eq!(listed, [(4, false), (0, true)]);
    assert!(
        h.app.model().jobs.is_empty(),
        "listings stay out of the job table"
    );
    assert!(!h.app.is_busy());
}

#[test]
fn navigation_relists_and_clears_the_selection() {
    let mut h = harness();
    h.settle();
    h.take_events();
    let pane = h.app.model().focused;
    h.send(Command::Select {
        pane,
        change: SelectionChange::All,
    });
    assert!(
        matches!(h.take_events().as_slice(), [Event::SelectionChanged { ids, .. }] if ids.len() == 4)
    );

    let sub = h.root.path().join("sub");
    fs::write(sub.join("inner.txt"), "x").unwrap_or_else(|e| panic!("{e}"));
    h.send(Command::Navigate {
        pane,
        to: local(&sub),
    });
    assert!(
        matches!(h.events.first(), Some(Event::SelectionChanged { ids, .. }) if ids.is_empty())
    );
    h.settle();
    assert_eq!(h.names(), ["inner.txt"]);

    h.send(Command::Back(pane));
    h.settle();
    assert_eq!(h.names(), ["sub", "a.txt", "b.txt", "c.txt"]);
    h.send(Command::Forward(pane));
    h.settle();
    assert_eq!(h.names(), ["inner.txt"]);
}

#[test]
fn refresh_swaps_the_listing_without_emptying_it() {
    let mut h = harness();
    h.settle();
    let pane = h.app.model().focused;
    h.take_events();
    fs::write(h.root.path().join("d.txt"), "d").unwrap_or_else(|e| panic!("{e}"));
    h.send(Command::Refresh(pane));
    // The old rows stay until the new listing is complete.
    assert_eq!(h.names().len(), 4);
    h.settle();
    assert_eq!(h.names(), ["sub", "a.txt", "b.txt", "c.txt", "d.txt"]);
    let full = h
        .take_events()
        .into_iter()
        .any(|e| matches!(e, Event::Listed { batch, done: true, .. } if batch.len() == 5));
    assert!(full, "a refresh delivers the whole listing at once");
}

#[test]
fn tabs_and_splits_keep_their_listings_live() {
    let mut h = harness();
    h.settle();
    let pane = h.app.model().focused;
    let sub = h.root.path().join("sub");

    h.send(Command::NewTab(local(&sub)));
    h.settle();
    assert_eq!(h.names(), [] as [String; 0]);
    let (second, _) = h
        .app
        .model()
        .active_tab(pane)
        .unwrap_or_else(|| panic!("tab"));
    let first = h
        .app
        .model()
        .pane(pane)
        .unwrap_or_else(|| panic!("pane"))
        .tabs()[0];

    h.send(Command::ActivateTab(first));
    h.settle();
    assert_eq!(h.names().len(), 4);

    h.send(Command::Split {
        pane,
        dir: model::SplitDir::Right,
    });
    h.settle();
    assert_eq!(h.app.model().dock.panes().len(), 2);
    let new_pane = h.app.model().dock.panes()[1];
    let (_, tab) = h
        .app
        .model()
        .active_tab(new_pane)
        .unwrap_or_else(|| panic!("tab"));
    assert_eq!(
        tab.listing.len(),
        4,
        "the split pane lists the same location"
    );

    h.send(Command::CloseTab(second));
    h.settle();
    assert_eq!(
        h.app
            .model()
            .pane(pane)
            .unwrap_or_else(|| panic!("pane"))
            .tabs(),
        [first]
    );
    assert!(!h.app.is_busy());
}

#[test]
fn settings_changes_are_saved_and_disk_edits_are_picked_up() {
    let mut h = harness();
    h.settle();
    let path = h.app.settings_path().to_path_buf();
    assert_eq!(
        path.file_name().and_then(|n| n.to_str()),
        Some(SETTINGS_FILE)
    );
    assert!(path.starts_with(h.fake.config_dir()));

    h.send(Command::Settings(SettingsChange::Sort(
        Column::Size,
        SortDir::Descending,
    )));
    h.settle();
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{e}"));
    assert!(text.contains("sort_by = \"size\""), "{text}");
    assert_eq!(h.app.model().settings.sort_by, Column::Size);

    fs::write(&path, "show_system = true\nfont_size = 15\n").unwrap_or_else(|e| panic!("{e}"));
    let started = Instant::now();
    while h.app.model().settings.font_size != 15 {
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "settings not reloaded"
        );
        runtime::block_on(async {
            let _ =
                tokio::time::timeout(Duration::from_millis(20), h.app.tick(&mut h.events)).await;
        });
    }
    assert!(h.app.model().settings.show_system);
    h.settle();
    assert!(!h.events.iter().any(|e| matches!(
        e,
        Event::Notify {
            level: Level::Warning,
            ..
        }
    )));

    fs::write(&path, "font_size = \n").unwrap_or_else(|e| panic!("{e}"));
    h.settle();
    assert!(h.events.iter().any(|e| matches!(
        e,
        Event::Notify {
            level: Level::Warning,
            ..
        }
    )));
    assert_eq!(
        h.app.model().settings.font_size,
        15,
        "a broken file keeps the last good settings"
    );
}

#[test]
fn actions_run_through_the_registry_and_refresh_the_pane() {
    let mut h = harness();
    h.settle();
    let pane = h.app.model().focused;
    h.take_events();

    h.send(Command::Run {
        action: ActionId("nope"),
        ctx: h
            .app
            .action_ctx(pane, Invocation::Palette)
            .unwrap_or_else(|| panic!("ctx")),
    });
    assert!(matches!(
        h.take_events().as_slice(),
        [Event::Notify {
            level: Level::Error,
            ..
        }]
    ));

    h.send(Command::Run {
        action: ActionId("core.rename"),
        ctx: h
            .app
            .action_ctx(pane, Invocation::Palette)
            .unwrap_or_else(|| panic!("ctx")),
    });
    assert!(matches!(
        h.take_events().as_slice(),
        [Event::Notify {
            level: Level::Warning,
            ..
        }]
    ));

    *h.ui.prompt.lock().unwrap_or_else(PoisonError::into_inner) = Some("Stuff".into());
    h.send(Command::Run {
        action: ActionId("core.new_folder"),
        ctx: h
            .app
            .action_ctx(pane, Invocation::Shortcut)
            .unwrap_or_else(|| panic!("ctx")),
    });
    h.settle();
    assert!(h.root.path().join("Stuff").is_dir());
    assert!(h.names().contains(&"Stuff".to_owned()));
    assert_eq!(h.app.undo_label().as_deref(), Some("New folder Stuff"));
    let events = h.take_events();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::JobDone { result: Ok(()), .. }))
    );
    assert_eq!(h.app.model().jobs.len(), 1, "{:?}", h.app.model().jobs);

    h.send(Command::Undo);
    h.settle();
    assert!(!h.root.path().join("Stuff").exists());
    assert!(!h.names().contains(&"Stuff".to_owned()));
    assert_eq!(h.app.redo_label().as_deref(), Some("New folder Stuff"));
    h.send(Command::Redo);
    h.settle();
    assert!(h.root.path().join("Stuff").is_dir());
    assert_eq!(h.app.model().jobs.len(), 3);
    let id = *h
        .app
        .model()
        .jobs
        .keys()
        .next()
        .unwrap_or_else(|| panic!("job"));
    h.send(Command::DismissJob(id));
    assert_eq!(h.app.model().jobs.len(), 2);
}
