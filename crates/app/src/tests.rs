use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use ext::{
    API_VERSION, ActionId, Availability, Capabilities, Extension, Filter, Invocation, Level,
    Manifest, Platforms, Points, Query, Registry, SearchHit, SearchIndex, SearchScope, UiHost,
    Version,
};
use futures::StreamExt;
use futures::stream;
use model::{Column, SelectionChange, SettingsChange, SortDir};
use platform::{ConfigDir, Fake};
use types::{Attrs, CancelToken, Entry, EntryId, EntryKind, Error, ExtId, Location, VPath};

use super::{App, Command, Config, Contents, Event, SETTINGS_FILE};

/// The text of the file at `path` once `done` holds for it, or as it is
/// after ten seconds: the settings saver writes on a task of the runtime
/// after the command, which `settle` does not wait for, so a test reads the
/// file this way rather than once (the Linux arm64 runner read it early).
fn read_until(path: &Path, done: impl Fn(&str) -> bool) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let text = fs::read_to_string(path).unwrap_or_default();
        if done(&text) || Instant::now() >= deadline {
            return text;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

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
    harness_with(Vec::new())
}

/// A harness with `extensions` loaded besides the core.
fn harness_with(extensions: Vec<Arc<dyn Extension>>) -> Harness {
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
        extensions,
        config_dir: Some(fake.config_dir()),
        start: Some(local(root.path())),
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

    /// The pane's active tab.
    fn tab(&self, pane: model::PaneId) -> &model::Tab {
        self.app
            .model()
            .active_tab(pane)
            .map_or_else(|| panic!("no tab"), |(_, tab)| tab)
    }

    /// Runs an action on the pane's location and selection.
    fn run(&mut self, pane: model::PaneId, action: &'static str) {
        let ctx = self
            .app
            .action_ctx(pane, Invocation::Shortcut)
            .unwrap_or_else(|| panic!("ctx"));
        self.send(Command::Run {
            action: ActionId(action),
            ctx,
        });
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
    let window = h.app.model().focused_window;
    let sub = h.root.path().join("sub");
    let (first, _) = h
        .app
        .model()
        .active_tab(pane)
        .unwrap_or_else(|| panic!("tab"));

    // A new tab is a workspace of its own, shown and listed.
    h.send(Command::NewTab(local(&sub)));
    h.settle();
    assert_eq!(h.app.model().workspaces(window).len(), 2);
    let second_pane = h.app.model().focused;
    assert_ne!(second_pane, pane);
    assert_eq!(h.names(), [] as [String; 0], "an empty folder, listed");
    let (second, _) = h
        .app
        .model()
        .active_tab(second_pane)
        .unwrap_or_else(|| panic!("tab"));

    // Activating a tab shows its workspace again, its listing kept.
    h.send(Command::ActivateTab(first));
    h.settle();
    assert_eq!(h.app.model().focused, pane);
    assert_eq!(h.names().len(), 4);

    h.send(Command::Split {
        pane,
        dir: model::SplitDir::Right,
    });
    h.settle();
    assert_eq!(h.app.model().dock().panes().len(), 2);
    let new_pane = h.app.model().dock().panes()[1];
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

    // Closing the one tab of the other workspace closes the workspace.
    h.send(Command::CloseTab(second));
    h.settle();
    assert_eq!(h.app.model().workspaces(window).len(), 1);
    assert!(h.app.model().pane(second_pane).is_none());
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

/// A change that is still on its way to the settings file is not undone
/// by the file as it was. The watcher reloads the file on any removal or
/// rescan in the configuration directory, and another file written there
/// through its temporary file is enough for one; until the saver has
/// written, the file holds what it held at the start, which is no edit to
/// pick up. The Windows runner of tier 2 lost a plugin's *disabled* this
/// way (full.yml run 42962612644, `extensions.rs`).
#[test]
fn the_settings_file_as_it_was_at_the_start_does_not_undo_a_change_on_its_way_to_it() {
    let mut h = harness();
    let path = h.app.settings_path().to_path_buf();
    let as_it_was = super::settings::load(&path).unwrap_or_else(|e| panic!("{e}"));
    assert_ne!(as_it_was.sort_by, Column::Size);

    h.send(Command::Settings(SettingsChange::Sort(
        Column::Size,
        SortDir::Descending,
    )));
    // What the watcher's reload reports when it read the file before the
    // saver wrote it.
    h.app
        .on_msg(super::Msg::Settings(Ok(as_it_was)), &mut h.events);
    assert_eq!(
        h.app.model().settings.sort_by,
        Column::Size,
        "the file as it was undid the change"
    );

    // The change reaches the file, and an edit to the file is still
    // picked up afterwards.
    h.settle();
    let text = read_until(&path, |t| t.contains("sort_by = \"size\""));
    assert!(text.contains("sort_by = \"size\""), "{text}");
    fs::write(&path, "font_size = 15\n").unwrap_or_else(|e| panic!("{e}"));
    let started = Instant::now();
    while h.app.model().settings.font_size != 15 {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "settings not reloaded"
        );
        runtime::block_on(async {
            let _ =
                tokio::time::timeout(Duration::from_millis(20), h.app.tick(&mut h.events)).await;
        });
    }
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
    let text = read_until(&path, |t| t.contains("sort_by = \"size\""));
    assert!(text.contains("sort_by = \"size\""), "{text}");
    assert_eq!(h.app.model().settings.sort_by, Column::Size);

    fs::write(&path, "show_system = true\nfont_size = 15\n").unwrap_or_else(|e| panic!("{e}"));
    // The watcher's latency is the platform's, and a loaded CI runner on
    // Windows has taken over half a second to report the write.
    let started = Instant::now();
    while h.app.model().settings.font_size != 15 {
        assert!(
            started.elapsed() < Duration::from_secs(10),
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
    // The watcher reports this write when the platform gets to it, as it
    // did the one above, and `settle` returns once nothing has arrived for
    // 80 ms: the warning is waited for, as the reload was (issue 263).
    let warned = wait_until(&mut h, Duration::from_secs(10), |h| {
        h.events.iter().any(|e| {
            matches!(
                e,
                Event::Notify {
                    level: Level::Warning,
                    ..
                }
            )
        })
    });
    assert!(warned, "no warning for the broken file: {:?}", h.events);
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

fn id_of(h: &Harness, name: &str) -> types::EntryId {
    let pane = h.app.model().focused;
    h.app
        .model()
        .active_tab(pane)
        .and_then(|(_, tab)| tab.listing.rows().find(|e| e.name == name).map(|e| e.id))
        .unwrap_or_else(|| panic!("no entry {name}"))
}

#[test]
fn transfer_moves_with_undo_and_copies_with_a_notice() {
    let mut h = harness();
    h.settle();
    let pane = h.app.model().focused;
    let sub = h.root.path().join("sub");
    h.take_events();

    let a = id_of(&h, "a.txt");
    h.send(Command::Transfer {
        items: vec![a],
        from: pane,
        to: local(&sub),
        mode: super::TransferMode::Move,
    });
    h.settle();
    assert!(sub.join("a.txt").exists());
    assert!(!h.root.path().join("a.txt").exists());
    assert_eq!(h.names(), ["sub", "b.txt", "c.txt"], "the pane refreshed");
    assert_eq!(h.app.undo_label().as_deref(), Some("Move a.txt to sub"));
    h.send(Command::Undo);
    h.settle();
    assert!(h.root.path().join("a.txt").exists());
    assert!(!sub.join("a.txt").exists());

    let b = id_of(&h, "b.txt");
    h.take_events();
    h.send(Command::Transfer {
        items: vec![b],
        from: pane,
        to: local(&sub),
        mode: super::TransferMode::Copy,
    });
    h.settle();
    assert!(sub.join("b.txt").exists());
    assert!(
        h.root.path().join("b.txt").exists(),
        "a copy keeps the source"
    );
    assert_eq!(h.app.undo_label(), None, "a copy is not undoable");
    assert_eq!(
        h.app.redo_label().as_deref(),
        Some("Move a.txt to sub"),
        "and does not clear the redo log"
    );
    let said_so = h.take_events().into_iter().any(|e| {
        matches!(e, Event::Notify { level: Level::Info, text } if text.contains("cannot be undone"))
    });
    assert!(said_so, "the copy says it is not undoable");

    // No-ops: into its own folder, a folder onto itself, into its own subtree.
    let c = id_of(&h, "c.txt");
    let sub_id = id_of(&h, "sub");
    for (items, to) in [
        (vec![c], local(h.root.path())),
        (vec![sub_id], local(&sub)),
        (vec![sub_id], local(&sub.join("deeper"))),
    ] {
        h.take_events();
        h.send(Command::Transfer {
            items,
            from: pane,
            to,
            mode: super::TransferMode::Move,
        });
        h.settle();
        assert!(h.take_events().is_empty(), "nothing happens, silently");
    }
    assert!(h.root.path().join("c.txt").exists());

    // Ask is the UI's to answer; here it is only reported.
    h.send(Command::Transfer {
        items: vec![c],
        from: pane,
        to: local(&sub),
        mode: super::TransferMode::Ask,
    });
    assert!(h.take_events().iter().any(|e| matches!(
        e,
        Event::Notify {
            level: Level::Warning,
            ..
        }
    )));
    assert!(h.root.path().join("c.txt").exists());

    // From outside the process: locations instead of ids.
    let outside = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    fs::write(outside.path().join("ext.txt"), "e").unwrap_or_else(|e| panic!("{e}"));
    h.send(Command::Receive {
        items: vec![local(&outside.path().join("ext.txt"))],
        to: local(h.root.path()),
        mode: super::TransferMode::Copy,
    });
    h.settle();
    assert!(h.root.path().join("ext.txt").exists());
    assert!(outside.path().join("ext.txt").exists());
    assert!(h.names().contains(&"ext.txt".to_owned()));
}

#[test]
fn spring_loading_pushes_one_step_and_back_restores_the_selection() {
    let mut h = harness();
    h.settle();
    let pane = h.app.model().focused;
    let sub = h.root.path().join("sub");
    fs::create_dir(sub.join("deeper")).unwrap_or_else(|e| panic!("{e}"));
    let a = id_of(&h, "a.txt");
    h.send(Command::Select {
        pane,
        change: SelectionChange::Set(vec![a]),
    });
    h.take_events();

    h.send(Command::SpringLoad {
        pane,
        to: local(&sub),
        first: true,
    });
    h.settle();
    assert_eq!(h.names(), ["deeper"]);
    h.send(Command::SpringLoad {
        pane,
        to: local(&sub.join("deeper")),
        first: false,
    });
    h.settle();
    let tab = h
        .app
        .model()
        .active_tab(pane)
        .map_or_else(|| panic!("tab"), |(_, t)| t.clone());
    assert_eq!(tab.location, local(&sub.join("deeper")));
    assert_eq!(tab.history.back.len(), 1, "one step for the whole drag");

    h.take_events();
    h.send(Command::Back(pane));
    h.settle();
    assert_eq!(h.names(), ["sub", "a.txt", "b.txt", "c.txt"]);
    let tab = h
        .app
        .model()
        .active_tab(pane)
        .map_or_else(|| panic!("tab"), |(_, t)| t.clone());
    assert!(tab.selection.contains(&a), "the selection came back");
    assert_eq!(tab.history.back, [] as [model::HistoryEntry; 0]);
    assert!(h.take_events().iter().any(|e| matches!(
        e,
        Event::SelectionChanged { ids, .. } if ids == &[a]
    )));
}

#[test]
fn pin_and_unpin_edit_the_settings_and_undo() {
    let mut h = harness();
    h.settle();
    let pane = h.app.model().focused;
    let (a, sub) = (h.root.path().join("a.txt"), h.root.path().join("sub"));

    // Through the command, at a position.
    h.send(Command::Pin {
        items: vec![local(&a)],
        at: None,
    });
    h.send(Command::Pin {
        items: vec![local(&sub)],
        at: Some(0),
    });
    assert_eq!(h.app.model().settings.pinned, [local(&sub), local(&a)]);
    assert_eq!(h.app.undo_label().as_deref(), Some("Pin sub"));
    h.send(Command::Undo);
    assert_eq!(h.app.model().settings.pinned, [local(&a)]);
    h.send(Command::Redo);
    assert_eq!(h.app.model().settings.pinned, [local(&sub), local(&a)]);
    h.send(Command::Unpin(local(&sub)));
    assert_eq!(h.app.model().settings.pinned, [local(&a)]);
    assert_eq!(h.app.undo_label().as_deref(), Some("Unpin sub"));
    h.send(Command::Undo);
    assert_eq!(h.app.model().settings.pinned, [local(&sub), local(&a)]);
    h.settle();
    let settings_path = h.app.settings_path();
    let text = read_until(Path::new(&settings_path), |t| t.contains("local:"));
    assert!(
        text.contains("local:"),
        "pins are saved as locations: {text}"
    );

    // Through the actions: Unpin shows for a pinned selection, Pin otherwise.
    h.send(Command::Select {
        pane,
        change: SelectionChange::Set(vec![id_of(&h, "a.txt")]),
    });
    let ctx = h
        .app
        .action_ctx(pane, Invocation::ContextMenu)
        .unwrap_or_else(|| panic!("ctx"));
    let availability = |h: &Harness, id: &'static str, ctx: &ext::ActionCtx| {
        h.app
            .registry()
            .actions()
            .iter()
            .find(|a| a.id() == ActionId(id))
            .map(|a| a.availability(ctx))
    };
    assert_eq!(
        availability(&h, "core.unpin", &ctx),
        Some(ext::Availability::Enabled)
    );
    assert_eq!(
        availability(&h, "core.pin", &ctx),
        Some(ext::Availability::Hidden)
    );
    h.send(Command::Run {
        action: ActionId("core.unpin"),
        ctx: ctx.clone(),
    });
    assert_eq!(h.app.model().settings.pinned, [local(&sub)]);
    assert_eq!(
        availability(&h, "core.pin", &ctx),
        Some(ext::Availability::Enabled),
        "the label flips back"
    );
    h.send(Command::Run {
        action: ActionId("core.pin"),
        ctx,
    });
    assert_eq!(h.app.model().settings.pinned, [local(&sub), local(&a)]);
    h.send(Command::Undo);
    assert_eq!(h.app.model().settings.pinned, [local(&sub)]);
}

#[test]
fn transfer_noops_are_the_spec_cases() {
    let l = |p: &str| Location::new(vfs_local::PROVIDER_ID, p);
    assert!(super::transfer_is_noop(&l("/a/b.txt"), &l("/a")));
    assert!(super::transfer_is_noop(&l("/a/b"), &l("/a/b")));
    assert!(super::transfer_is_noop(&l("/a/b"), &l("/a/b/c/d")));
    assert!(!super::transfer_is_noop(&l("/a/b"), &l("/a/bc")));
    assert!(!super::transfer_is_noop(&l("/a/b.txt"), &l("/x")));
    assert!(!super::transfer_is_noop(
        &l("/a/b.txt"),
        &Location::new(types::ProviderId("other"), "/a")
    ));
}

/// Runs the loop until `done` holds or `limit` passes; returns whether it
/// held.
fn wait_until(h: &mut Harness, limit: Duration, mut done: impl FnMut(&Harness) -> bool) -> bool {
    let deadline = Instant::now() + limit;
    runtime::block_on(async {
        loop {
            if done(h) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            let _ =
                tokio::time::timeout(Duration::from_millis(20), h.app.tick(&mut h.events)).await;
        }
    })
}

#[test]
fn tree_view_lists_children_lazily_and_keeps_them_across_a_refresh() {
    let mut h = harness();
    h.settle();
    let pane = h.app.model().focused;
    let sub = h.root.path().join("sub");
    fs::write(sub.join("inner.txt"), "i").unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir(sub.join("deeper")).unwrap_or_else(|e| panic!("{e}"));
    h.send(Command::SetView {
        pane,
        mode: model::ViewMode::Tree,
    });
    let rows = |h: &Harness| -> Vec<(String, u16)> {
        let (_, tab) = h
            .app
            .model()
            .active_tab(pane)
            .unwrap_or_else(|| panic!("tab"));
        (0..tab.row_count())
            .filter_map(|i| tab.row(i).map(|e| (e.name.clone(), tab.depth(i))))
            .collect()
    };
    assert_eq!(rows(&h).len(), 4, "the listing alone");
    let sub_id = id_of(&h, "sub");
    h.send(Command::TreeExpand { pane, id: sub_id });
    h.settle();
    assert_eq!(
        rows(&h),
        [
            ("sub".to_owned(), 0),
            ("deeper".to_owned(), 1),
            ("inner.txt".to_owned(), 1),
            ("a.txt".to_owned(), 0),
            ("b.txt".to_owned(), 0),
            ("c.txt".to_owned(), 0)
        ]
    );
    assert!(
        h.app.model().jobs.is_empty(),
        "child listings are not jobs the user sees"
    );

    // A refresh keeps the expanded folder and its children.
    fs::write(sub.join("later.txt"), "l").unwrap_or_else(|e| panic!("{e}"));
    h.send(Command::Refresh(pane));
    h.settle();
    let names: Vec<String> = rows(&h).into_iter().map(|(n, _)| n).collect();
    assert!(names.contains(&"later.txt".to_owned()), "{names:?}");
    assert!(names.contains(&"inner.txt".to_owned()), "{names:?}");
    let (_, tab) = h
        .app
        .model()
        .active_tab(pane)
        .unwrap_or_else(|| panic!("tab"));
    assert!(tab.tree.expanded.contains(&sub_id));

    h.send(Command::TreeCollapse { pane, id: sub_id });
    assert_eq!(rows(&h).len(), 4);
    h.send(Command::Navigate {
        pane,
        to: local(&sub),
    });
    h.settle();
    let (_, tab) = h
        .app
        .model()
        .active_tab(pane)
        .unwrap_or_else(|| panic!("tab"));
    assert!(tab.tree.expanded.is_empty());
}

/// A search index that stands in for a plugin: a manifest around it and
/// the extension that registers it.
struct IndexExtension {
    manifest: Manifest,
    index: Arc<dyn SearchIndex>,
}

impl IndexExtension {
    /// The extension around `index`, ready for a [`Config`].
    fn around(id: &'static str, index: Arc<dyn SearchIndex>) -> Arc<dyn Extension> {
        Arc::new(Self {
            manifest: Manifest {
                id: ExtId(id),
                name: "Fake index",
                description: "",
                version: Version::new(0, 1, 0),
                api_version: API_VERSION,
                requires: Capabilities::empty(),
                points: Points::SEARCH_INDEX,
                platforms: Platforms::ALL,
                previews: &[],
                markers: &[],
                connections: &[],
                commands: &[],
            },
            index,
        })
    }
}

impl Extension for IndexExtension {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        reg.search_index(Arc::clone(&self.index));
        Ok(())
    }
}

/// The subdirectory walk as the `ext-search-walk` plugin provides it,
/// small enough for this crate's tests: `Under` the local root, one batch
/// per directory, honouring the hidden and skip filters.
struct FakeWalk;

impl SearchIndex for FakeWalk {
    fn id(&self) -> ExtId {
        ExtId("fake-walk")
    }
    fn availability(&self) -> Availability {
        Availability::Enabled
    }
    fn scope(&self) -> SearchScope {
        SearchScope::Under(Location::new(vfs_local::PROVIDER_ID, VPath::new()))
    }
    fn query(
        &self,
        q: &Query,
        cancel: CancelToken,
    ) -> ext::BoxStream<'static, Result<Vec<SearchHit>, Error>> {
        let Some(root) = q.root.clone() else {
            return stream::once(async { Err(Error::Unsupported("no root")) }).boxed();
        };
        let text = q.text.to_lowercase();
        let hidden = !q.filters.contains(&Filter::NoHidden);
        let skip: Vec<String> = q
            .filters
            .iter()
            .filter_map(|f| match f {
                Filter::SkipNames(names) => Some(names.clone()),
                _ => None,
            })
            .flatten()
            .collect();
        let (tx, rx) = runtime::mpsc::channel(8);
        runtime::handle().spawn_blocking(move || {
            let start = platform::paths::to_path(&root.path).unwrap_or_default();
            let mut dirs = vec![start];
            while let Some(dir) = dirs.pop() {
                if cancel.is_cancelled() {
                    return;
                }
                let mut batch = Vec::new();
                for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if (!hidden && name.starts_with('.')) || skip.contains(&name) {
                        continue;
                    }
                    if entry.file_type().is_ok_and(|t| t.is_dir()) {
                        dirs.push(entry.path());
                    }
                    if name.to_lowercase().contains(&text)
                        && let Ok(found) = vfs_local::entry_at(
                            &entry.path(),
                            &platform::paths::from_path(&entry.path()),
                        )
                    {
                        batch.push(SearchHit {
                            entry: found,
                            snippet: None,
                        });
                    }
                }
                if !batch.is_empty() && tx.blocking_send(Ok(batch)).is_err() {
                    return;
                }
            }
        });
        stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|b| (b, rx)) }).boxed()
    }
}

/// A platform index: a fixed list of hits, a switch for its availability
/// and a record of the queries it was asked.
struct FakeIndex {
    id: &'static str,
    available: AtomicBool,
    hits: Vec<Entry>,
    queries: Mutex<Vec<Query>>,
}

impl FakeIndex {
    fn new(id: &'static str, hits: Vec<Entry>) -> Arc<Self> {
        Arc::new(Self {
            id,
            available: AtomicBool::new(true),
            hits,
            queries: Mutex::new(Vec::new()),
        })
    }

    fn queries(&self) -> Vec<String> {
        self.queries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|q| q.text.clone())
            .collect()
    }
}

impl SearchIndex for FakeIndex {
    fn id(&self) -> ExtId {
        ExtId(self.id)
    }
    fn availability(&self) -> Availability {
        if self.available.load(Ordering::SeqCst) {
            Availability::Enabled
        } else {
            Availability::Disabled("service gone".to_owned())
        }
    }
    fn scope(&self) -> SearchScope {
        SearchScope::Everywhere
    }
    fn query(
        &self,
        q: &Query,
        _cancel: CancelToken,
    ) -> ext::BoxStream<'static, Result<Vec<SearchHit>, Error>> {
        self.queries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(q.clone());
        if q.text == "fail" {
            return stream::once(async { Err(Error::Unsupported("the index refused")) }).boxed();
        }
        let hits: Vec<SearchHit> = self
            .hits
            .iter()
            .filter(|e| e.name.contains(&q.text))
            .map(|e| SearchHit {
                entry: e.clone(),
                snippet: None,
            })
            .collect();
        stream::iter(vec![Ok(hits)]).boxed()
    }
}

/// An entry of a fictional file for the fake indexes.
fn hit(id: u128, path: &str) -> Entry {
    let name = path.rsplit('/').next().unwrap_or(path).to_owned();
    Entry {
        id: EntryId::new(vfs_local::PROVIDER_ID, id),
        name,
        path: VPath::from(path),
        kind: EntryKind::File,
        size: None,
        created: None,
        modified: None,
        accessed: None,
        attrs: Attrs::empty(),
        mime: None,
    }
}

/// Issue 149: where nothing searches beneath a location — no search
/// index under it and no `SEARCH_SUBTREE` on its provider — the toggle
/// cannot be switched on, the filter narrows the folder as always, and no
/// error is shown anywhere. The "search-walk plugin is not loaded" notice
/// is gone.
#[test]
fn where_nothing_searches_beneath_the_toggle_stays_off_and_no_error_is_shown() {
    let mut h = harness();
    h.settle();
    let pane = h.app.model().focused;
    let location = h.tab(pane).location.clone();
    assert_eq!(
        h.app.subtree_search(&location),
        Availability::Disabled("Not available in this location".to_owned())
    );
    assert_eq!(h.app.model().search_indexes, Vec::<ExtId>::new());
    h.take_events();
    h.send(Command::Subtree { pane, on: true });
    h.send(Command::Filter {
        pane,
        text: "a.t".into(),
    });
    h.settle();
    let events = h.take_events();
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Event::Notify { level, .. } if *level != Level::Info)),
        "{events:?}"
    );
    assert!(!h.tab(pane).subtree, "the toggle stays off");
    assert!(!h.tab(pane).is_searching());
    assert!(!h.app.is_busy());
    assert_eq!(h.names(), ["a.txt"], "the filter narrows the folder itself");
}

#[test]
fn subtree_search_streams_matches_as_a_job_and_a_new_text_supersedes_it() {
    let mut h = harness_with(vec![IndexExtension::around(
        "fake-walk",
        Arc::new(FakeWalk),
    )]);
    h.settle();
    let pane = h.app.model().focused;
    assert_eq!(
        h.app.subtree_search(&h.tab(pane).location),
        Availability::Enabled,
        "the walk searches beneath a local folder"
    );
    assert!(
        h.app.model().search_indexes.is_empty(),
        "a walk is not a platform index"
    );
    let mut dir = h.root.path().join("sub");
    for level in 0..20 {
        dir.push(format!("l{level}"));
        fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        fs::write(dir.join(format!("foo{level}.txt")), "x").unwrap_or_else(|e| panic!("{e}"));
    }
    h.send(Command::Subtree { pane, on: true });
    assert_eq!(h.names().len(), 4, "a toggle without text changes nothing");
    h.send(Command::Filter {
        pane,
        text: "foo".into(),
    });
    let started = Instant::now();
    assert!(
        wait_until(&mut h, Duration::from_secs(5), |h| !h.names().is_empty()),
        "no match arrived"
    );
    let first = started.elapsed();
    h.settle();
    let names = h.names();
    assert_eq!(names.len(), 20, "{names:?}");
    assert!(names.iter().all(|n| n.starts_with("foo")));
    let (_, tab) = h
        .app
        .model()
        .active_tab(pane)
        .unwrap_or_else(|| panic!("tab"));
    assert!(tab.is_searching());
    let deep = tab
        .listing
        .rows()
        .find(|e| e.name == "foo19.txt")
        .unwrap_or_else(|| panic!("deep match"));
    assert!(deep.path.to_string_lossy().contains("l19"), "full paths");
    println!("first search batch after {first:?}");
    let search_jobs: Vec<&model::JobInfo> = h
        .app
        .model()
        .jobs
        .values()
        .filter(|j| j.name.starts_with("Search"))
        .collect();
    assert_eq!(search_jobs.len(), 1, "the walk is a job the user sees");
    assert_eq!(search_jobs[0].state, model::JobState::Done);

    // A new text supersedes the walk: the first job is cancelled if it is
    // still running (the acceptance test shows that on the 200k fixture) or
    // has finished; either way nothing stays running and only the last
    // text's matches show.
    h.send(Command::Filter {
        pane,
        text: "fo".into(),
    });
    h.send(Command::Filter {
        pane,
        text: "foo1".into(),
    });
    h.settle();
    let states: Vec<model::JobState> = h
        .app
        .model()
        .jobs
        .values()
        .filter(|j| j.name.starts_with("Search"))
        .map(|j| j.state.clone())
        .collect();
    assert_eq!(states.len(), 3, "{states:?}");
    assert!(
        states
            .iter()
            .all(|s| matches!(s, model::JobState::Done | model::JobState::Cancelled)),
        "a superseded walk ends: {states:?}"
    );
    let names = h.names();
    assert!(names.iter().all(|n| n.starts_with("foo1")), "{names:?}");
    // Off again: the location's own entries are back.
    h.send(Command::Subtree { pane, on: false });
    h.settle();
    assert_eq!(
        h.names().len(),
        0,
        "the plain filter hides what does not match"
    );
    h.send(Command::Filter {
        pane,
        text: String::new(),
    });
    assert_eq!(h.names().len(), 4);
}

/// A walk whose stream holds one of its polls open, as the real walk's
/// set-up holds its first: asked for `held`, the stream answers `before`
/// batches that are ready at once, then says on `entered` that it is inside
/// a poll and waits there, on the thread that polls it, for `release`. Only
/// then does it look at its token, and it ends as the spec has a stream end:
/// quietly when the token is cancelled, after one more batch otherwise. Any
/// other text is answered at once.
///
/// The wait is on the polling thread on purpose. A stream that returned
/// `Pending` would hand the poll back to the job runner, which looks at the
/// token before it polls the job again; held open, the poll is one in which
/// the runner has already looked.
struct GatedWalk {
    held: &'static str,
    before: u64,
    entered: std::sync::mpsc::Sender<()>,
    release: Arc<Mutex<std::sync::mpsc::Receiver<()>>>,
}

impl SearchIndex for GatedWalk {
    fn id(&self) -> ExtId {
        ExtId("gated-walk")
    }
    fn availability(&self) -> Availability {
        Availability::Enabled
    }
    fn scope(&self) -> SearchScope {
        SearchScope::Under(Location::new(vfs_local::PROVIDER_ID, VPath::new()))
    }
    fn query(
        &self,
        q: &Query,
        cancel: CancelToken,
    ) -> ext::BoxStream<'static, Result<Vec<SearchHit>, Error>> {
        let text = q.text.clone();
        let held = text == self.held;
        let batch = move |n: u64| {
            let id = u128::from(n) + if held { 100 } else { 1 };
            Ok(vec![SearchHit {
                entry: hit(id, &format!("/{text}-{n}.txt")),
                snippet: None,
            }])
        };
        if !held {
            return stream::iter(vec![batch(0)]).boxed();
        }
        let before = self.before;
        let entered = self.entered.clone();
        let release = Arc::clone(&self.release);
        stream::unfold(0u64, move |sent| {
            let entered = entered.clone();
            let release = Arc::clone(&release);
            let cancel = cancel.clone();
            let batch = batch.clone();
            async move {
                if sent < before {
                    return Some((batch(sent), sent + 1));
                }
                if sent > before {
                    return None;
                }
                let _ = entered.send(());
                let _ = release
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .recv_timeout(Duration::from_secs(10));
                if cancel.is_cancelled() {
                    return None;
                }
                Some((batch(sent), sent + 1))
            }
        })
        .boxed()
    }
}

/// The states of the search jobs in the job table, oldest first.
fn search_states(h: &Harness) -> Vec<model::JobState> {
    h.app
        .model()
        .jobs
        .values()
        .filter(|j| j.name.starts_with("Search"))
        .map(|j| j.state.clone())
        .collect()
}

/// Searches for `held` on a [`GatedWalk`] that answers `before` batches
/// first and returns once the search job is inside the poll the walk holds
/// open, with what lets the walk go. The batches before that poll are in
/// the listing by then. Nothing in a test that goes on from here depends on
/// how fast anything runs: it cancels after the poll began, and the poll
/// ends after it let the walk go.
fn search_held_inside_a_poll(before: u64) -> (Harness, std::sync::mpsc::Sender<()>) {
    let (entered, has_entered) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel();
    let mut h = harness_with(vec![IndexExtension::around(
        "gated-walk",
        Arc::new(GatedWalk {
            held: "held",
            before,
            entered,
            release: Arc::new(Mutex::new(released)),
        }),
    )]);
    h.settle();
    let pane = h.app.model().focused;
    h.send(Command::Subtree { pane, on: true });
    h.send(Command::Filter {
        pane,
        text: "held".into(),
    });
    assert!(
        has_entered.recv_timeout(Duration::from_secs(10)).is_ok(),
        "the search never reached the poll the walk holds open"
    );
    let shown = usize::try_from(before).unwrap_or(usize::MAX);
    let all_shown = wait_until(&mut h, Duration::from_secs(10), |h| {
        h.names().len() == shown
    });
    assert!(
        all_shown,
        "the batches before the held poll never showed: {:?}",
        h.names()
    );
    (h, release)
}

/// A second text cancels the search held inside a poll, and only then is
/// the walk let go.
fn supersede_inside_a_poll(before: u64) -> Harness {
    let (mut h, release) = search_held_inside_a_poll(before);
    let pane = h.app.model().focused;
    h.send(Command::Filter {
        pane,
        text: "next".into(),
    });
    assert!(release.send(()).is_ok(), "the walk is gone");
    h.settle();
    h
}

/// The cancel lands inside the first poll of the search job, after the job
/// runner looked at the token and before the walk's stream does: the stream
/// ends quietly, and the job must not take that for the end of the search
/// (the comment of 2026-11-19 on issue 263). The search that replaced it
/// really finishes and stays Done.
#[test]
fn a_search_cancelled_inside_its_first_poll_ends_cancelled() {
    let h = supersede_inside_a_poll(0);
    assert_eq!(
        search_states(&h),
        [model::JobState::Cancelled, model::JobState::Done],
        "the first search was cancelled, the second ran through"
    );
    assert_eq!(h.names(), ["next-0.txt"], "only the second text's match");
}

/// The same for a cancel that lands after the first batch: the search job
/// works through a batch that was ready and asks for the next in the same
/// poll, and the cancel lands there.
#[test]
fn a_search_cancelled_inside_a_poll_after_its_first_batch_ends_cancelled() {
    let h = supersede_inside_a_poll(1);
    assert_eq!(
        search_states(&h),
        [model::JobState::Cancelled, model::JobState::Done],
        "the first search was cancelled, the second ran through"
    );
    assert_eq!(h.names(), ["next-0.txt"], "only the second text's match");
}

/// `ListDone` carries the job's result: a search cancelled from the
/// operations area, the cancel landing inside a poll, reaches its pane as
/// cancelled and not as complete. The pane keeps the matches it has and
/// says that the listing did not end, as it does for any listing that
/// failed.
#[test]
fn a_search_cancelled_from_the_operations_area_inside_a_poll_is_not_complete_for_its_pane() {
    let (mut h, release) = search_held_inside_a_poll(1);
    let pane = h.app.model().focused;
    let search = h
        .app
        .model()
        .jobs
        .iter()
        .find(|(_, job)| job.name.starts_with("Search"))
        .map_or_else(|| panic!("the search is in the job table"), |(id, _)| *id);
    h.send(Command::CancelJob(search));
    assert!(release.send(()).is_ok(), "the walk is gone");
    h.settle();
    assert_eq!(search_states(&h), [model::JobState::Cancelled]);
    assert_eq!(h.names(), ["held-0.txt"], "the match it had stays");
    let failure = h.tab(pane).failure.clone().unwrap_or_default();
    assert!(
        failure.ends_with("could not be listed: cancelled"),
        "the pane says the search did not end: {failure:?}"
    );
}

/// A folder's listing in Tree view, cancelled inside a poll.
mod held_folder {
    use std::sync::mpsc::{Receiver, Sender, channel};

    use ext::{
        ChangeStream, CopyOptions, EntryStream, LinkKind, ListOptions, Provider, ReadHandle,
        RemoveMode, WriteHandle, WriteOptions, async_trait,
    };
    use runtime::ProgressSink;
    use types::ProviderId;

    use super::*;

    const ID: ProviderId = ProviderId("held");

    /// A provider of one folder, `/folder`, whose listing holds a poll open
    /// as [`GatedWalk`]'s stream does: it says on `entered` that it is
    /// inside a poll and waits there for `release`, then ends quietly when
    /// its token is cancelled and after one file otherwise.
    struct Held {
        entered: Sender<()>,
        release: Arc<Mutex<Receiver<()>>>,
    }

    fn entry(key: u128, path: &str, kind: EntryKind) -> Entry {
        Entry {
            id: EntryId::new(ID, key),
            kind,
            ..hit(key, path)
        }
    }

    #[async_trait]
    impl Provider for Held {
        fn id(&self) -> ProviderId {
            ID
        }
        fn capabilities(&self, _loc: &Location) -> Capabilities {
            Capabilities::empty()
        }
        fn list(&self, loc: &Location, _opts: ListOptions, cancel: CancelToken) -> EntryStream {
            if loc.path.as_bytes() == b"/" {
                return stream::once(async { Ok(vec![entry(1, "/folder", EntryKind::Dir)]) })
                    .boxed();
            }
            let entered = self.entered.clone();
            let release = Arc::clone(&self.release);
            stream::unfold(false, move |answered| {
                let entered = entered.clone();
                let release = Arc::clone(&release);
                let cancel = cancel.clone();
                async move {
                    if answered {
                        return None;
                    }
                    let _ = entered.send(());
                    let _ = release
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .recv_timeout(Duration::from_secs(10));
                    if cancel.is_cancelled() {
                        return None;
                    }
                    let file = entry(2, "/folder/file.txt", EntryKind::File);
                    Some((Ok(vec![file]), true))
                }
            })
            .boxed()
        }
        async fn stat(&self, _loc: &Location) -> Result<Entry, Error> {
            Err(Error::NotFound)
        }
        async fn open_read(&self, _loc: &Location) -> Result<ReadHandle, Error> {
            Err(Error::Unsupported("held"))
        }
        async fn open_write(&self, _l: &Location, _o: WriteOptions) -> Result<WriteHandle, Error> {
            Err(Error::Unsupported("held"))
        }
        async fn create_dir(&self, _loc: &Location) -> Result<Entry, Error> {
            Err(Error::Unsupported("held"))
        }
        async fn rename(&self, _f: &Location, _t: &Location) -> Result<Entry, Error> {
            Err(Error::Unsupported("held"))
        }
        async fn remove(&self, _loc: &Location, _mode: RemoveMode) -> Result<(), Error> {
            Err(Error::Unsupported("held"))
        }
        async fn link(&self, _t: &Location, _a: &Location, _k: LinkKind) -> Result<Entry, Error> {
            Err(Error::Unsupported("held"))
        }
        async fn copy(
            &self,
            _f: &Location,
            _t: &Location,
            _o: CopyOptions,
            _p: ProgressSink,
            _c: CancelToken,
        ) -> Result<(), Error> {
            Err(Error::Unsupported("held"))
        }
        fn watch(&self, _loc: &Location) -> Result<ChangeStream, Error> {
            Err(Error::Unsupported("held"))
        }
    }

    struct HeldExtension {
        manifest: Manifest,
        provider: Arc<Held>,
    }

    impl Extension for HeldExtension {
        fn manifest(&self) -> &Manifest {
            &self.manifest
        }
        fn register(&self, reg: &mut Registry) -> Result<(), Error> {
            reg.provider(Arc::clone(&self.provider) as Arc<dyn Provider>);
            Ok(())
        }
    }

    /// A folder collapsed while its children are being listed, the cancel
    /// landing inside a poll of that listing: the listing ends cancelled
    /// and tells the pane nothing, as when the runner sees the cancel
    /// first. Told that it was cancelled, the pane would warn *Listing
    /// folder: cancelled* about a folder the user merely collapsed. Expanded
    /// again, the folder lists.
    #[test]
    fn a_folder_collapsed_inside_a_poll_of_its_listing_says_nothing() {
        let (entered, has_entered) = channel();
        let (release, released) = channel();
        let mut h = harness_with(vec![Arc::new(HeldExtension {
            manifest: Manifest {
                id: ExtId("held"),
                name: "Held folder",
                description: "",
                version: Version::new(0, 1, 0),
                api_version: API_VERSION,
                requires: Capabilities::empty(),
                points: Points::PROVIDER,
                platforms: Platforms::ALL,
                previews: &[],
                markers: &[],
                connections: &[],
                commands: &[],
            },
            provider: Arc::new(Held {
                entered,
                release: Arc::new(Mutex::new(released)),
            }),
        })]);
        h.settle();
        let pane = h.app.model().focused;
        h.send(Command::Navigate {
            pane,
            to: Location::new(ID, "/"),
        });
        h.send(Command::SetView {
            pane,
            mode: model::ViewMode::Tree,
        });
        h.settle();
        let folder = id_of(&h, "folder");
        h.take_events();

        h.send(Command::TreeExpand { pane, id: folder });
        assert!(
            has_entered.recv_timeout(Duration::from_secs(10)).is_ok(),
            "the folder's listing never reached the poll it holds open"
        );
        h.send(Command::TreeCollapse { pane, id: folder });
        assert!(release.send(()).is_ok(), "the listing is gone");
        h.settle();
        let events = h.take_events();
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, Event::Notify { level, .. } if *level != Level::Info)),
            "a collapsed folder is no warning: {events:?}"
        );
        assert_eq!(h.names(), ["folder"]);

        h.send(Command::TreeExpand { pane, id: folder });
        assert!(has_entered.recv_timeout(Duration::from_secs(10)).is_ok());
        assert!(release.send(()).is_ok(), "the listing is gone");
        h.settle();
        let tab = h.tab(pane);
        let rows: Vec<String> = (0..tab.row_count())
            .filter_map(|i| tab.row(i).map(|e| e.name.clone()))
            .collect();
        assert_eq!(rows, ["folder", "file.txt"], "a listing that ends is told");
    }
}

#[test]
fn paste_conflicts_reach_the_model_and_the_answer_reaches_the_job() {
    let mut h = harness();
    h.settle();
    let pane = h.app.model().focused;
    let sub = h.root.path().join("sub");
    fs::write(sub.join("a.txt"), "old").unwrap_or_else(|e| panic!("{e}"));
    h.send(Command::Select {
        pane,
        change: SelectionChange::Set(vec![id_of(&h, "a.txt")]),
    });
    let ctx = h
        .app
        .action_ctx(pane, Invocation::Shortcut)
        .unwrap_or_else(|| panic!("ctx"));
    h.send(Command::Run {
        action: ActionId("core.copy"),
        ctx,
    });
    h.send(Command::Navigate {
        pane,
        to: local(&sub),
    });
    h.settle();
    let ctx = h
        .app
        .action_ctx(pane, Invocation::Shortcut)
        .unwrap_or_else(|| panic!("ctx"));
    h.send(Command::Run {
        action: ActionId("core.paste"),
        ctx,
    });
    assert!(
        wait_until(&mut h, Duration::from_secs(5), |h| !h
            .app
            .model()
            .conflicts
            .is_empty()),
        "the conflict reaches the model"
    );
    let conflict = h.app.model().conflicts[0].clone();
    assert_eq!(conflict.proposed, "a (2).txt");
    assert!(
        h.take_events()
            .iter()
            .any(|e| matches!(e, Event::Conflict(c) if c.id == conflict.id))
    );
    // The job's message travels on the progress channel, the conflict on
    // the mailbox: the message may be a tick behind the conflict.
    assert!(
        wait_until(&mut h, Duration::from_secs(5), |h| {
            h
            .app
            .model()
            .jobs
            .get(&conflict.job)
            .is_some_and(|j| matches!(&j.state, model::JobState::Running(types::Progress::Message(m)) if m.contains("exists")))
        }),
        "the job waits on the conflict: {:?}",
        h.app.model().jobs.get(&conflict.job)
    );
    h.send(Command::ResolveConflict {
        id: conflict.id,
        answer: types::ConflictAnswer::once(types::Resolution::Rename("fresh.txt".into())),
    });
    assert_eq!(h.app.model().conflicts.len(), 0, "no conflict is pending");
    h.settle();
    assert_eq!(
        fs::read_to_string(sub.join("fresh.txt")).ok(),
        Some("a.txt".into())
    );
    assert_eq!(
        fs::read_to_string(sub.join("a.txt")).ok(),
        Some("old".into())
    );
    assert!(
        h.names().contains(&"fresh.txt".to_owned()),
        "the pane refreshed"
    );
    assert_eq!(
        h.app
            .model()
            .jobs
            .get(&conflict.job)
            .map(|j| j.state.clone()),
        Some(model::JobState::Done)
    );
}

#[test]
fn places_are_seeded_from_the_standard_folders_and_edits_persist() {
    let mut h = harness();
    h.settle();
    let titles = |h: &Harness| -> Vec<String> {
        h.app
            .model()
            .places
            .rows()
            .iter()
            .map(|p| p.title.clone())
            .collect()
    };
    // The fake has Documents, Pictures and Downloads under its home.
    assert_eq!(
        titles(&h),
        [
            "Home",
            "Documents",
            "Pictures",
            "Downloads",
            "Favorites",
            "Recent",
            "Trash"
        ]
    );
    let file = h.app.settings_path().with_file_name(super::PLACES_FILE);
    assert!(
        wait_until(&mut h, Duration::from_secs(2), |_| {
            fs::read_to_string(&file).is_ok_and(|t| t.contains("Downloads"))
        }),
        "the seeded list is written"
    );
    let sub = local(&h.root.path().join("sub"));
    h.send(Command::PlaceAdd {
        place: model::Place::new("sub", sub.clone()),
        at: Some(1),
    });
    assert_eq!(h.app.model().places.rows()[1].title, "sub");
    h.send(Command::PlaceAdd {
        place: model::Place::new("again", sub),
        at: None,
    });
    assert_eq!(h.app.model().places.len(), 8, "a listed location stays");
    h.send(Command::PlaceMove { from: 1, to: 7 });
    assert_eq!(
        h.app.model().places.rows().last().map(|p| p.title.as_str()),
        Some("sub")
    );
    h.send(Command::PlaceEdit {
        index: 7,
        title: "Project".into(),
        icon: "STAR".into(),
    });
    h.send(Command::PlaceRemove(2));
    assert_eq!(
        titles(&h),
        [
            "Home",
            "Documents",
            "Downloads",
            "Favorites",
            "Recent",
            "Trash",
            "Project"
        ]
    );
    assert_eq!(h.app.model().places.rows()[6].icon, "STAR");
    assert!(wait_until(&mut h, Duration::from_secs(2), |_| {
        fs::read_to_string(&file).is_ok_and(|t| t.contains("Project") && !t.contains("Pictures"))
    }));

    // A second app from the same config directory has the edited list.
    let again = App::new(Config {
        platform: h.fake.clone(),
        ui: Arc::new(Scripted::default()),
        extensions: Vec::new(),
        config_dir: Some(h.fake.config_dir()),
        start: Some(local(h.root.path())),
    });
    assert_eq!(again.model().places.rows(), h.app.model().places.rows());
}

#[test]
fn the_recent_and_trash_views_list_through_the_rick_provider() {
    let mut h = harness();
    h.settle();
    let pane = h.app.model().focused;
    let at = |secs: u64| std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(secs);
    h.fake.set_recent(vec![
        platform::RecentItem {
            path: h.root.path().join("b.txt"),
            used_at: at(10),
        },
        platform::RecentItem {
            path: h.root.path().join("a.txt"),
            used_at: at(20),
        },
        platform::RecentItem {
            path: h.root.path().join("gone.txt"),
            used_at: at(30),
        },
    ]);
    h.send(Command::Navigate {
        pane,
        to: vfs_rick::Virtual::Recent.location(),
    });
    h.settle();
    assert_eq!(
        h.names(),
        ["a.txt", "b.txt"],
        "newest use first, what exists"
    );
    assert_eq!(h.tab(pane).sort.by, Column::Accessed);
    assert_eq!(h.tab(pane).sort.dir, SortDir::Descending);
    assert_eq!(
        h.tab(pane).listing.rows().next().map(|e| e.accessed),
        Some(Some(at(20)))
    );

    // Trash: an item trashed from a folder counts on the Places row; the
    // view itself is the trash plugin's (issue 20), so without the plugin
    // the location is served empty.
    h.send(Command::Back(pane));
    h.settle();
    h.send(Command::Select {
        pane,
        change: SelectionChange::Set(vec![id_of(&h, "c.txt")]),
    });
    h.run(pane, "core.trash");
    assert!(wait_until(&mut h, Duration::from_secs(5), |h| {
        h.app.model().trash.items.len() == 1
    }));
    h.send(Command::Navigate {
        pane,
        to: vfs_rick::Virtual::Trash.location(),
    });
    h.settle();
    assert_eq!(h.tab(pane).location, vfs_rick::Virtual::Trash.location());
    assert!(h.names().is_empty(), "{:?}", h.names());
    assert!(!h.root.path().join("c.txt").exists());
}

/// Two copies into one folder share its volume: the second waits for the
/// first, which a conflict holds open, and runs once it is answered.
#[test]
fn a_copy_on_a_busy_volume_waits_for_the_running_one() {
    let mut h = harness();
    h.settle();
    let pane = h.app.model().focused;
    let sub = h.root.path().join("sub");
    fs::write(sub.join("a.txt"), "old").unwrap_or_else(|e| panic!("{e}"));
    let paste = |h: &mut Harness, name: &str| {
        h.send(Command::Navigate {
            pane,
            to: local(h.root.path()),
        });
        // Not `settle`: a job may be running (or waiting) the whole time.
        let wanted = name.to_owned();
        assert!(wait_until(h, Duration::from_secs(5), |h| {
            h.tab(pane).listing.complete && h.names().contains(&wanted)
        }));
        h.send(Command::Select {
            pane,
            change: SelectionChange::Set(vec![id_of(h, name)]),
        });
        h.run(pane, "core.copy");
        h.send(Command::Navigate {
            pane,
            to: local(&sub),
        });
        h.run(pane, "core.paste");
    };
    paste(&mut h, "a.txt");
    assert!(
        wait_until(&mut h, Duration::from_secs(5), |h| {
            !h.app.model().conflicts.is_empty()
        }),
        "the first copy asks about a.txt"
    );
    let first = h.app.model().conflicts[0].job;
    paste(&mut h, "b.txt");
    assert!(
        wait_until(&mut h, Duration::from_secs(5), |h| {
            h.app
                .model()
                .jobs
                .values()
                .any(|j| j.state == model::JobState::Waiting("Copy a.txt".into()))
        }),
        "the second copy waits for the first: {:?}",
        h.app.model().jobs
    );
    assert!(!sub.join("b.txt").exists(), "nothing copied while waiting");
    let conflict = h.app.model().conflicts[0].id;
    h.send(Command::ResolveConflict {
        id: conflict,
        answer: types::ConflictAnswer::once(types::Resolution::Skip),
    });
    assert!(wait_until(&mut h, Duration::from_secs(5), |h| {
        h.app
            .model()
            .jobs
            .values()
            .all(|j| j.state == model::JobState::Done)
            && h.app.model().jobs.len() == 2
    }));
    assert!(
        sub.join("b.txt").exists(),
        "the second copy ran after the first"
    );
    assert_eq!(
        fs::read_to_string(sub.join("a.txt")).ok(),
        Some("old".into())
    );
    assert!(h.app.model().jobs.contains_key(&first));
}

#[test]
fn eject_goes_through_the_platform_and_the_list_follows() {
    let mut h = harness();
    h.settle();
    let sub = h.root.path().join("sub");
    h.fake.add_volume(platform::Volume {
        name: "STICK".into(),
        mount_point: sub.clone(),
        kind: platform::VolumeKind::Removable,
        fs_type: "vfat".into(),
        device: None,
        total: Some(100),
        free: Some(50),
        read_only: false,
    });
    h.fake.add_volume(platform::Volume {
        name: "data".into(),
        mount_point: h.root.path().to_path_buf(),
        kind: platform::VolumeKind::Fixed,
        fs_type: "ext4".into(),
        device: None,
        total: Some(100),
        free: Some(50),
        read_only: false,
    });
    h.send(Command::RefreshVolumes);
    assert!(wait_until(&mut h, Duration::from_secs(5), |h| {
        h.app.model().volumes.len() == 2
    }));
    h.send(Command::Eject(sub.clone()));
    assert!(wait_until(&mut h, Duration::from_secs(5), |h| {
        h.fake.ejected() == [sub.clone()] && h.app.model().volumes.len() == 1
    }));
    assert!(h.app.model().jobs.values().any(|j| j.name == "Eject STICK"));
    // A fixed disk cannot be ejected: the job fails and says so.
    h.send(Command::Eject(h.root.path().to_path_buf()));
    assert!(wait_until(&mut h, Duration::from_secs(5), |h| {
        h.app
            .model()
            .jobs
            .values()
            .any(|j| j.name == "Eject data" && matches!(j.state, model::JobState::Failed(_)))
    }));
    assert_eq!(h.fake.ejected().len(), 1);
}

/// Acceptance (issue 149, item 4): in a folder with hidden files that are
/// not shown, Select all selects the visible entries only and a notice
/// names how many hidden ones it left out; with them shown it selects
/// all and says nothing.
#[test]
fn select_all_selects_what_is_shown_and_counts_the_hidden_it_left_out() {
    let mut h = harness();
    for name in [".cache", ".secret"] {
        fs::write(h.root.path().join(name), name).unwrap_or_else(|e| panic!("{e}"));
    }
    // The start location is being listed by now: the two are in the
    // listing from its read of the folder, or from the watcher's report
    // when the read came first, and `settle` waits for the read only.
    let pane = h.app.model().focused;
    let listed = wait_until(&mut h, Duration::from_secs(10), |h| {
        let hidden = h.tab(pane).listing.entries().iter();
        hidden.filter(|e| e.name.starts_with('.')).count() == 2
    });
    assert!(listed, "the hidden files are not listed");
    h.settle();
    assert_eq!(h.names(), ["sub", "a.txt", "b.txt", "c.txt"]);
    h.take_events();
    let hidden_notices = |events: &[Event]| -> Vec<String> {
        events
            .iter()
            .filter_map(|event| match event {
                Event::Notify {
                    level: Level::Info,
                    text,
                } if text.contains("not selected") => Some(text.clone()),
                _ => None,
            })
            .collect()
    };

    h.send(Command::Select {
        pane,
        change: SelectionChange::All,
    });
    assert_eq!(h.tab(pane).selection.len(), 4, "the four that are shown");
    assert!(
        h.tab(pane)
            .selected()
            .all(|entry| !entry.name.starts_with('.'))
    );
    assert_eq!(
        hidden_notices(&h.take_events()),
        ["2 hidden items not selected"]
    );

    // One hidden item reads in the singular; the filter narrows what
    // counts, as it narrows what is shown.
    h.send(Command::Filter {
        pane,
        text: "a".into(),
    });
    h.send(Command::Select {
        pane,
        change: SelectionChange::All,
    });
    assert_eq!(h.names(), ["a.txt"]);
    assert_eq!(
        hidden_notices(&h.take_events()),
        ["1 hidden item not selected"],
        "of the hidden two only .cache has an a"
    );
    h.send(Command::Filter {
        pane,
        text: String::new(),
    });

    // With the hidden files shown, everything is selected and nothing is
    // said.
    h.send(Command::ToggleHidden(pane));
    h.settle();
    h.take_events();
    h.send(Command::Select {
        pane,
        change: SelectionChange::All,
    });
    assert_eq!(h.tab(pane).selection.len(), 6);
    assert_eq!(hidden_notices(&h.take_events()), [] as [String; 0]);
}

/// A stick mounted at `sub` of the harness's root, as the platform lists it.
fn stick(h: &Harness) -> platform::Volume {
    platform::Volume {
        name: "STICK".into(),
        mount_point: h.root.path().join("sub"),
        kind: platform::VolumeKind::Removable,
        fs_type: "vfat".into(),
        device: None,
        total: Some(100),
        free: Some(50),
        read_only: false,
    }
}

/// The texts of the notices in `events` that say something was removed.
fn removal_notices(events: &[Event]) -> Vec<(Level, String)> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::Notify { level, text } if text.contains("was removed") => {
                Some((*level, text.clone()))
            }
            _ => None,
        })
        .collect()
}

/// Acceptance (issue 149, item 3): with a tab three folders deep on a
/// stick, pulling the stick sends every pane on it Home at once, with one
/// notice that names the drive, and Back never returns to what is gone.
#[test]
fn pulling_a_stick_sends_every_pane_on_it_home_and_says_so_once() {
    let mut h = harness();
    h.settle();
    let sub = h.root.path().join("sub");
    let deep = sub.join("one").join("two").join("three");
    fs::create_dir_all(&deep).unwrap_or_else(|e| panic!("{e}"));
    h.fake.add_volume(stick(&h));
    h.send(Command::RefreshVolumes);
    assert!(wait_until(&mut h, Duration::from_secs(5), |h| {
        h.app.model().volumes.len() == 1
    }));
    // One pane walks down the stick, a second tab opens on its root.
    let first = h.app.model().focused;
    for folder in [
        sub.clone(),
        sub.join("one"),
        sub.join("one").join("two"),
        deep.clone(),
    ] {
        h.send(Command::Navigate {
            pane: first,
            to: local(&folder),
        });
        h.settle();
    }
    h.send(Command::NewTab(local(&sub)));
    h.settle();
    let second = h.app.model().focused;
    assert_ne!(second, first);
    h.take_events();

    h.fake.remove_volume(&sub);
    h.send(Command::RefreshVolumes);
    assert!(wait_until(&mut h, Duration::from_secs(5), |h| {
        h.app.model().volumes.is_empty()
    }));
    let home = local(&h.fake.home_dir());
    assert_eq!(explorer_at(&h, first), home);
    assert_eq!(explorer_at(&h, second), home);
    h.settle();
    let events = h.take_events();
    assert_eq!(
        removal_notices(&events),
        [(Level::Info, "STICK was removed".to_owned())]
    );
    assert!(
        !events.iter().any(|e| matches!(
            e,
            Event::Notify {
                level: Level::Error | Level::Warning,
                ..
            }
        )),
        "a removed drive is no error: {events:?}"
    );
    assert_eq!(h.tab(first).failure, None);

    // The steps on the stick left the history with it.
    h.send(Command::Back(first));
    h.settle();
    assert_eq!(explorer_at(&h, first), local(h.root.path()));
}

/// Eject sends the panes Home first and unmounts afterwards, so no pane
/// points at a drive that is gone; the list that follows the unmount says
/// nothing a second time.
#[test]
fn eject_sends_the_panes_home_before_it_unmounts() {
    let mut h = harness();
    h.settle();
    let sub = h.root.path().join("sub");
    h.fake.add_volume(stick(&h));
    h.send(Command::RefreshVolumes);
    assert!(wait_until(&mut h, Duration::from_secs(5), |h| {
        h.app.model().volumes.len() == 1
    }));
    let pane = h.app.model().focused;
    h.send(Command::Navigate {
        pane,
        to: local(&sub),
    });
    h.settle();
    h.take_events();

    h.send(Command::Eject(sub.clone()));
    // Before the loop has run once more, and so before the unmount is
    // reported: the pane is Home already.
    let home = local(&h.fake.home_dir());
    assert_eq!(explorer_at(&h, pane), home);
    assert_eq!(
        removal_notices(&h.events),
        [(Level::Info, "STICK was removed".to_owned())]
    );
    assert!(wait_until(&mut h, Duration::from_secs(5), |h| {
        h.fake.ejected() == [sub.clone()] && h.app.model().volumes.is_empty()
    }));
    h.settle();
    assert_eq!(removal_notices(&h.take_events()).len(), 1);
    assert_eq!(explorer_at(&h, pane), home);
}

/// The drive list is read again on the platform's mount event, not only at
/// its interval: a partition that mounts a moment after the first is there
/// well before the next interval.
#[test]
fn a_mount_event_reads_the_drive_list_again_at_once() {
    let mut h = harness();
    h.settle();
    // The first partition, seen by the interval: a read has just happened,
    // and the next one by the clock is a whole interval away.
    h.fake.add_volume(stick(&h));
    assert!(wait_until(&mut h, super::VOLUMES_EVERY * 3, |h| {
        h.app.model().volumes.len() == 1
    }));
    h.fake.add_volume(platform::Volume {
        name: "SECOND".into(),
        mount_point: h.root.path().to_path_buf(),
        ..stick(&h)
    });
    h.fake.mounts_changed();
    assert!(
        wait_until(&mut h, super::VOLUMES_EVERY / 2, |h| {
            h.app.model().volumes.len() == 2
        }),
        "the second partition waits for the interval"
    );
}

/// A harness with the stub network provider and its one host `nas`, the
/// focused pane on it.
fn harness_on_a_host() -> (Harness, Arc<net::Stub>, model::PaneId, Location) {
    let stub = net::Stub::new();
    let mut h = harness_with(vec![Arc::new(net::StubExtension::new(Arc::clone(&stub)))]);
    h.settle();
    h.send(Command::AddConnection {
        provider: net::ID,
        name: "nas".into(),
        values: vec![("host".into(), "10.0.0.2".into())],
    });
    h.settle();
    let pane = h.app.model().focused;
    let host = Location::new(net::ID, "/nas/share");
    h.send(Command::Navigate {
        pane,
        to: host.clone(),
    });
    h.settle();
    assert_eq!(explorer_at(&h, pane), host);
    assert_eq!(h.tab(pane).failure, None);
    h.take_events();
    (h, stub, pane, host)
}

/// Removing a host from the side panel sends the panes on it Home with
/// `<host> was removed`.
#[test]
fn removing_a_connection_sends_the_panes_on_it_home() {
    let (mut h, _stub, pane, _host) = harness_on_a_host();
    h.send(Command::Connection {
        provider: net::ID,
        name: "nas".into(),
        op: super::ConnectionOp::Remove,
    });
    assert_eq!(explorer_at(&h, pane), local(&h.fake.home_dir()));
    h.settle();
    assert_eq!(
        removal_notices(&h.take_events()),
        [(Level::Info, "nas was removed".to_owned())]
    );
    assert_eq!(explorer_at(&h, pane), local(&h.fake.home_dir()));
}

/// Acceptance (issue 149, item 3): a connection that drops keeps the pane
/// where it is and shows *Connection lost to <host>* with the provider's
/// message; Rick tries again by itself, and the listing is back once the
/// host answers. A missing path is not a lost connection.
#[test]
fn a_lost_connection_keeps_the_pane_and_is_tried_again_until_it_answers() {
    let (mut h, stub, pane, host) = harness_on_a_host();
    stub.set_down(Some("connection timed out"));
    h.send(Command::Refresh(pane));
    assert!(wait_until(&mut h, Duration::from_secs(5), |h| {
        h.tab(pane).lost.is_some()
    }));
    let lost = h.tab(pane).lost.clone().unwrap_or_else(|| panic!("lost"));
    assert_eq!(lost.source, "nas");
    assert_eq!(lost.provider, "Stub");
    assert_eq!(lost.message, "connection timed out");
    assert_eq!(explorer_at(&h, pane), host, "the pane stays where it is");
    assert_eq!(h.tab(pane).failure, None, "the card, not the banner");

    // Rick tries again by itself: the first time after two seconds.
    assert!(wait_until(
        &mut h,
        super::RETRY_LOST_AFTER[0] + Duration::from_secs(3),
        |h| h.tab(pane).lost.as_ref().is_some_and(|l| l.retries == 1)
    ));
    assert_eq!(explorer_at(&h, pane), host);
    // Retry, the card's button, is a refresh: the host still does not
    // answer, and the card stays.
    h.send(Command::Refresh(pane));
    h.settle();
    assert!(h.tab(pane).lost.is_some());

    // The host answers again: the next try shows the listing where the
    // pane was, with nothing asked of the user.
    stub.set_down(None);
    assert!(wait_until(
        &mut h,
        super::RETRY_LOST_AFTER[1] + Duration::from_secs(3),
        |h| h.tab(pane).lost.is_none()
    ));
    assert_eq!(explorer_at(&h, pane), host);
    assert_eq!(h.tab(pane).failure, None);
    let events = h.take_events();
    assert!(
        !events.iter().any(|e| matches!(
            e,
            Event::Notify {
                level: Level::Error,
                ..
            }
        )),
        "a lost connection is shown in the pane, not as a notice: {events:?}"
    );

    // A folder that is not there is a failed listing, as it was.
    h.send(Command::Navigate {
        pane,
        to: Location::new(net::ID, "/gone"),
    });
    h.settle();
    assert_eq!(h.tab(pane).lost, None);
    assert!(h.tab(pane).failure.is_some());
}

/// Format… opens the system's tool on the drive, and says so where the
/// system has none.
#[test]
fn format_opens_the_systems_tool_on_the_drive() {
    let mut h = harness();
    h.settle();
    let sub = h.root.path().join("sub");
    h.fake.add_volume(stick(&h));
    h.send(Command::RefreshVolumes);
    assert!(wait_until(&mut h, Duration::from_secs(5), |h| {
        h.app.model().volumes.len() == 1
    }));
    assert!(h.app.model().format_tool);
    h.send(Command::Format(sub.clone()));
    assert!(wait_until(&mut h, Duration::from_secs(5), |h| {
        h.fake.format_opened() == [sub.clone()]
    }));

    h.fake.set_format_tool(false);
    h.take_events();
    h.send(Command::Format(sub.clone()));
    h.settle();
    assert_eq!(h.fake.format_opened().len(), 1);
    assert!(h.take_events().iter().any(|e| matches!(
        e,
        Event::Notify { level: Level::Warning, text } if text == "No disk utility is installed"
    )));
}

#[test]
fn a_virtual_location_comes_back_with_the_session() {
    let mut h = harness();
    h.settle();
    let pane = h.app.model().focused;
    h.send(Command::Navigate {
        pane,
        to: vfs_rick::Virtual::Favorites.location(),
    });
    h.settle();
    h.app.save_session().unwrap_or_else(|e| panic!("{e}"));
    let again = App::new(Config {
        platform: h.fake.clone(),
        ui: Arc::new(Scripted::default()),
        extensions: Vec::new(),
        config_dir: Some(h.fake.config_dir()),
        start: None,
    });
    let (_, tab) = again
        .model()
        .active_tab(again.model().focused)
        .unwrap_or_else(|| panic!("tab"));
    assert_eq!(tab.location, vfs_rick::Virtual::Favorites.location());
    assert!(tab.note.is_none(), "{:?}", tab.note);
}

#[test]
fn the_volumes_reach_the_model() {
    let mut h = harness();
    h.settle();
    // Volumes come from the platform.
    assert_eq!(h.app.model().volumes.len(), 0, "no volume is listed");
    h.fake.add_volume(platform::Volume {
        name: "USB".into(),
        mount_point: h.root.path().join("sub"),
        kind: platform::VolumeKind::Removable,
        fs_type: String::new(),
        device: None,
        total: Some(100),
        free: Some(50),
        read_only: false,
    });
    h.send(Command::RefreshVolumes);
    assert!(wait_until(&mut h, Duration::from_secs(5), |h| {
        h.app.model().volumes.len() == 1
    }));
    assert_eq!(h.app.model().volumes[0].name, "USB");
}

/// Open navigates into a folder; with no archive provider mounted a file
/// goes straight to the system default.
#[test]
fn open_navigates_into_a_folder_and_opens_a_file_with_the_default() {
    let mut h = harness();
    h.settle();
    let pane = h.app.model().focused;
    let file = id_of(&h, "a.txt");
    h.send(Command::Open { pane, id: file });
    h.settle();
    assert_eq!(h.fake.opened(), [h.root.path().join("a.txt")]);
    assert_eq!(h.names(), ["sub", "a.txt", "b.txt", "c.txt"], "still here");

    let sub = id_of(&h, "sub");
    h.send(Command::Open { pane, id: sub });
    h.settle();
    let location = h
        .app
        .model()
        .active_tab(pane)
        .map_or_else(|| panic!("tab"), |(_, tab)| tab.location.clone());
    assert_eq!(location, local(&h.root.path().join("sub")));
    assert_eq!(h.fake.opened().len(), 1, "a folder opens nothing outside");
}

/// A network provider stub for the connection tests: its root lists the
/// connections its connector was handed, everything else is unsupported.
mod net {
    use ext::Text;
    use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

    use ext::{
        API_VERSION, Capabilities, ChangeStream, Connection, ConnectionField, ConnectionFieldKind,
        ConnectionSchema, Connector, CopyOptions, EntryStream, Extension, LinkKind, ListOptions,
        Manifest, Platforms, Points, Provider, ReadHandle, Registry, RemoveMode, Version,
        WriteHandle, WriteOptions, async_trait,
    };
    use futures::StreamExt as _;
    use futures::stream;
    use runtime::ProgressSink;
    use types::{
        Attrs, CancelToken, Entry, EntryId, EntryKind, Error, ExtId, Location, ProviderId,
    };

    pub(super) const ID: ProviderId = ProviderId("ssh");

    #[derive(Default)]
    pub(super) struct Stub {
        pub(super) connections: Mutex<Vec<Connection>>,
        pub(super) connected: Mutex<Vec<String>>,
        /// What the hosts answer with while they cannot be reached.
        down: Mutex<Option<&'static str>>,
        schema: Option<ConnectionSchema>,
    }

    fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
        m.lock().unwrap_or_else(PoisonError::into_inner)
    }

    impl Stub {
        pub(super) fn new() -> Arc<Self> {
            Arc::new(Self {
                schema: Some(ConnectionSchema {
                    title: Text("Stub"),
                    fields: vec![
                        ConnectionField {
                            key: "host",
                            label: Text("Host"),
                            kind: ConnectionFieldKind::Text {
                                default: String::new(),
                            },
                            required: true,
                        },
                        ConnectionField {
                            key: "port",
                            label: Text("Port"),
                            kind: ConnectionFieldKind::Integer {
                                default: 22,
                                min: 1,
                                max: 65535,
                            },
                            required: false,
                        },
                        ConnectionField {
                            key: "password",
                            label: Text("Password"),
                            kind: ConnectionFieldKind::Secret,
                            required: false,
                        },
                    ],
                }),
                ..Self::default()
            })
        }
    }

    impl Stub {
        /// From now on the hosts cannot be reached and say `why`, or
        /// answer again with `None`.
        pub(super) fn set_down(&self, why: Option<&'static str>) {
            *lock(&self.down) = why;
        }
    }

    #[async_trait]
    impl Connector for Stub {
        fn provider(&self) -> ProviderId {
            ID
        }
        fn schema(&self) -> ConnectionSchema {
            self.schema.clone().unwrap_or_else(|| unreachable!())
        }
        fn update(&self, connections: Vec<Connection>) {
            let names: Vec<String> = connections.iter().map(|c| c.name.clone()).collect();
            lock(&self.connected).retain(|n| names.contains(n));
            *lock(&self.connections) = connections;
        }
        async fn connect(&self, name: &str) -> Result<(), Error> {
            if !lock(&self.connections).iter().any(|c| c.name == name) {
                return Err(Error::NotFound);
            }
            lock(&self.connected).push(name.to_owned());
            Ok(())
        }
        fn disconnect(&self, name: &str) {
            lock(&self.connected).retain(|n| n != name);
        }
        fn is_connected(&self, name: &str) -> bool {
            lock(&self.connected).iter().any(|n| n == name)
        }
    }

    #[async_trait]
    impl Provider for Stub {
        fn id(&self) -> ProviderId {
            ID
        }
        fn capabilities(&self, _loc: &Location) -> Capabilities {
            Capabilities::REMOTE
        }
        fn list(&self, loc: &Location, _opts: ListOptions, _cancel: CancelToken) -> EntryStream {
            if loc.path.as_bytes() != b"/" {
                // A host's own folder is empty, while the host answers.
                let host = loc.path.as_bytes()[1..].split(|b| *b == b'/').next();
                let known = lock(&self.connections)
                    .iter()
                    .any(|c| Some(c.name.as_bytes()) == host);
                let answer = match *lock(&self.down) {
                    _ if !known => Err(Error::NotFound),
                    Some(why) => Err(Error::unreachable(why)),
                    None => Ok(Vec::new()),
                };
                return stream::once(async { answer }).boxed();
            }
            let entries: Vec<Entry> = lock(&self.connections)
                .iter()
                .map(|c| Entry {
                    id: EntryId::new(ID, u128::from(c.name.len() as u64)),
                    name: c.name.clone(),
                    path: format!("/{}", c.name).into(),
                    kind: EntryKind::Dir,
                    size: None,
                    created: None,
                    modified: None,
                    accessed: None,
                    attrs: Attrs::empty(),
                    mime: None,
                })
                .collect();
            stream::once(async move { Ok(entries) }).boxed()
        }
        async fn stat(&self, _loc: &Location) -> Result<Entry, Error> {
            Err(Error::NotFound)
        }
        async fn open_read(&self, _loc: &Location) -> Result<ReadHandle, Error> {
            Err(Error::Unsupported("stub"))
        }
        async fn open_write(&self, _l: &Location, _o: WriteOptions) -> Result<WriteHandle, Error> {
            Err(Error::Unsupported("stub"))
        }
        async fn create_dir(&self, _loc: &Location) -> Result<Entry, Error> {
            Err(Error::Unsupported("stub"))
        }
        async fn rename(&self, _f: &Location, _t: &Location) -> Result<Entry, Error> {
            Err(Error::Unsupported("stub"))
        }
        async fn remove(&self, _loc: &Location, _mode: RemoveMode) -> Result<(), Error> {
            Err(Error::Unsupported("stub"))
        }
        async fn link(&self, _t: &Location, _a: &Location, _k: LinkKind) -> Result<Entry, Error> {
            Err(Error::Unsupported("stub"))
        }
        async fn copy(
            &self,
            _f: &Location,
            _t: &Location,
            _o: CopyOptions,
            _p: ProgressSink,
            _c: CancelToken,
        ) -> Result<(), Error> {
            Err(Error::Unsupported("stub"))
        }
        fn watch(&self, _loc: &Location) -> Result<ChangeStream, Error> {
            Err(Error::Unsupported("stub"))
        }
    }

    pub(super) struct StubExtension {
        manifest: Manifest,
        pub(super) stub: Arc<Stub>,
    }

    impl StubExtension {
        pub(super) fn new(stub: Arc<Stub>) -> Self {
            Self {
                manifest: Manifest {
                    id: ExtId("stub-net"),
                    name: "Stub network provider",
                    description: "",
                    version: Version::new(0, 1, 0),
                    api_version: API_VERSION,
                    requires: Capabilities::empty(),
                    points: Points::PROVIDER,
                    platforms: Platforms::ALL,
                    previews: &[],
                    markers: &[],
                    connections: &[],
                    commands: &[],
                },
                stub,
            }
        }
    }

    impl Extension for StubExtension {
        fn manifest(&self) -> &Manifest {
            &self.manifest
        }
        fn register(&self, reg: &mut Registry) -> Result<(), Error> {
            reg.provider(Arc::clone(&self.stub) as Arc<dyn Provider>);
            reg.connector(Arc::clone(&self.stub) as Arc<dyn Connector>);
            Ok(())
        }
    }
}

/// Acceptance (issue 21): a connection's secret goes to the platform
/// keychain and never into `rick.toml`, its other fields into
/// `[[connections]]`; the connector is handed the list; the Network rows
/// follow; a second connection of the same name is refused; Connect marks
/// it connected and Remove forgets the entry and its secret.
#[test]
// One scenario, start to finish: add, refuse twice, connect, disconnect, remove.
#[allow(clippy::too_many_lines)]
fn a_connection_keeps_its_secret_in_the_keychain_and_the_rest_in_the_settings() {
    let root = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let stub = net::Stub::new();
    let app = App::new(Config {
        platform: fake.clone(),
        ui: Arc::new(Scripted::default()),
        extensions: vec![Arc::new(net::StubExtension::new(Arc::clone(&stub)))],
        config_dir: Some(fake.config_dir()),
        start: Some(local(root.path())),
    });
    let mut h = Harness {
        root,
        fake,
        ui: Arc::new(Scripted::default()),
        app,
        events: Vec::new(),
    };
    h.settle();
    assert_eq!(h.app.model().network_providers, [net::ID]);
    assert_eq!(h.app.model().network, Vec::<Entry>::new());

    h.send(Command::AddConnection {
        provider: net::ID,
        name: "nas".into(),
        values: vec![
            ("host".into(), "10.0.0.2".into()),
            ("port".into(), String::new()),
            ("password".into(), "hunter2".into()),
            ("stray".into(), "dropped".into()),
        ],
    });
    h.settle();
    let settings_path = h.fake.config_dir().join(SETTINGS_FILE);
    let text = read_until(&settings_path, |t| t.contains("name = \"nas\""));
    assert!(text.contains("[[connections]]"), "{text}");
    assert!(text.contains("provider = \"ssh\""), "{text}");
    assert!(text.contains("name = \"nas\""), "{text}");
    assert!(text.contains("host = \"10.0.0.2\""), "{text}");
    assert!(
        !text.contains("hunter2"),
        "the secret is not in the file: {text}"
    );
    assert!(!text.contains("stray"), "{text}");
    assert_eq!(
        h.fake.secrets().get("ssh/nas/password").map(String::as_str),
        Some("hunter2")
    );
    {
        let handed = stub
            .connections
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        assert_eq!(handed.len(), 1);
        assert_eq!(handed[0].name, "nas");
        assert_eq!(handed[0].get("host"), Some("10.0.0.2"));
        assert_eq!(
            handed[0].get("password"),
            None,
            "the connector gets no secret"
        );
    }
    let rows: Vec<String> = h
        .app
        .model()
        .network
        .iter()
        .map(|e| e.name.clone())
        .collect();
    assert_eq!(rows, ["nas"]);

    // The same name again is refused with a notice and changes nothing.
    h.take_events();
    h.send(Command::AddConnection {
        provider: net::ID,
        name: "nas".into(),
        values: vec![("host".into(), "other".into())],
    });
    h.settle();
    assert!(
        h.take_events()
            .iter()
            .any(|e| matches!(e, Event::Notify { level: Level::Error, text } if text.contains("exists already"))),
    );
    assert_eq!(h.app.model().settings.connections.len(), 1);

    // A required field left empty is refused too.
    h.send(Command::AddConnection {
        provider: net::ID,
        name: "second".into(),
        values: vec![("password".into(), "x".into())],
    });
    h.settle();
    assert_eq!(h.app.model().settings.connections.len(), 1);
    assert_eq!(h.fake.secrets().len(), 1);

    // Connect runs as a job and marks the connection connected.
    h.send(Command::Connection {
        provider: net::ID,
        name: "nas".into(),
        op: super::ConnectionOp::Connect,
    });
    h.settle();
    assert!(ext::Connector::is_connected(&*stub, "nas"));
    h.send(Command::Connection {
        provider: net::ID,
        name: "nas".into(),
        op: super::ConnectionOp::Disconnect,
    });
    h.settle();
    assert!(!ext::Connector::is_connected(&*stub, "nas"));

    // Remove forgets the entry, the secret and the row.
    h.send(Command::Connection {
        provider: net::ID,
        name: "nas".into(),
        op: super::ConnectionOp::Remove,
    });
    h.settle();
    let text = read_until(&settings_path, |t| !t.contains("nas"));
    assert!(!text.contains("nas"), "{text}");
    assert!(h.fake.secrets().is_empty());
    assert!(
        stub.connections
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    );
    assert_eq!(h.app.model().network, Vec::<Entry>::new());
    drop(h.ui);
}

/// The title-row search box asks every available index once the text has
/// stood for a moment, lists each hit once, follows the availability of
/// the indexes, and an empty text ends the search.
#[test]
fn the_title_row_search_asks_every_available_index_and_lists_each_hit_once() {
    let first = FakeIndex::new(
        "fake-one",
        vec![hit(1, "/one/foo.txt"), hit(2, "/shared/foobar.md")],
    );
    let second = FakeIndex::new(
        "fake-two",
        vec![hit(2, "/shared/foobar.md"), hit(3, "/two/foo.rs")],
    );
    let mut h = harness_with(vec![
        IndexExtension::around("fake-one", first.clone()),
        IndexExtension::around("fake-two", second.clone()),
    ]);
    h.settle();
    assert_eq!(
        h.app.model().search_indexes,
        [ExtId("fake-one"), ExtId("fake-two")]
    );
    assert!(h.app.model().index_search.done, "nothing asked yet");

    h.send(Command::IndexSearch { text: "foo".into() });
    assert_eq!(h.app.model().index_search.text, "foo");
    assert!(!h.app.model().index_search.done);
    assert!(
        first.queries().is_empty(),
        "the indexes wait for the pause after the keystroke"
    );
    assert!(wait_until(&mut h, Duration::from_secs(5), |h| {
        h.app.model().index_search.done
    }));
    let search = &h.app.model().index_search;
    let mut names: Vec<&str> = search.hits.iter().map(|e| e.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["foo.rs", "foo.txt", "foobar.md"], "{search:?}");
    assert_eq!(search.errors, Vec::<String>::new());
    assert_eq!(first.queries(), ["foo"]);
    assert_eq!(second.queries(), ["foo"]);
    let events = h.take_events();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::IndexSearched { done: true, .. })),
        "{events:?}"
    );
    assert!(
        h.app
            .model()
            .jobs
            .values()
            .all(|j| !j.name.starts_with("Search")),
        "index queries stay out of the operations area"
    );

    // Two texts within the pause: only the last one is asked.
    h.send(Command::IndexSearch { text: "fo".into() });
    h.send(Command::IndexSearch {
        text: " foobar ".into(),
    });
    assert!(wait_until(&mut h, Duration::from_secs(5), |h| {
        h.app.model().index_search.done
    }));
    assert_eq!(first.queries(), ["foo", "foobar"]);
    assert_eq!(h.app.model().index_search.hits.len(), 1);

    // An index that fails is reported beside the others' hits.
    h.send(Command::IndexSearch {
        text: "fail".into(),
    });
    assert!(wait_until(&mut h, Duration::from_secs(5), |h| {
        h.app.model().index_search.done
    }));
    let errors = &h.app.model().index_search.errors;
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert!(
        errors.iter().any(|e| e.starts_with("fake-one: ")),
        "{errors:?}"
    );

    // An empty text ends the search.
    h.send(Command::IndexSearch {
        text: String::new(),
    });
    let search = &h.app.model().index_search;
    assert!(search.done && search.text.is_empty() && search.hits.is_empty());

    // The service behind an index goes away: the box follows on the next
    // poll, and with no index left a search is done at once, unasked.
    first.available.store(false, Ordering::SeqCst);
    h.app.poll(&mut h.events);
    assert_eq!(h.app.model().search_indexes, [ExtId("fake-two")]);
    second.available.store(false, Ordering::SeqCst);
    h.app.poll(&mut h.events);
    assert_eq!(h.app.model().search_indexes, Vec::<ExtId>::new());
    h.send(Command::IndexSearch { text: "foo".into() });
    assert!(h.app.model().index_search.done);
    h.settle();
    assert_eq!(first.queries().len(), 3);
    assert_eq!(second.queries().len(), 3);
}

/// Issue 83: the preview pane's *Copy* puts the inflated text of a
/// gzip-compressed file (an SVGZ) on the clipboard, and the decoded text of
/// any other file as before.
#[test]
fn copy_inflates_a_gzip_compressed_file_before_decoding_it() {
    use std::io::Write as _;

    let xml = "<svg xmlns=\"http://www.w3.org/2000/svg\"/>\n";
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(xml.as_bytes())
        .unwrap_or_else(|e| panic!("{e}"));
    let svgz = gz.finish().unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        super::copied_text(&svgz).unwrap_or_else(|e| panic!("{e}")),
        xml
    );
    assert_eq!(
        super::copied_text(xml.as_bytes()).unwrap_or_else(|e| panic!("{e}")),
        xml
    );
    // A byte order mark still decides the encoding.
    let mut utf16 = vec![0xFF, 0xFE];
    utf16.extend("hi".encode_utf16().flat_map(u16::to_le_bytes));
    assert_eq!(
        super::copied_text(&utf16).unwrap_or_else(|e| panic!("{e}")),
        "hi"
    );
    // Gzip that does not inflate is an error, never a panic.
    assert!(super::copied_text(&[0x1f, 0x8b, 8, 0, 1, 2, 3]).is_err());
}

/// `Command::Reveal` (specs/system-integration.md §3): the files' folder
/// opens as a tab of the focused window and the files are selected once
/// listed; with `new_window` the first folder's tab is a window of its
/// own; `Command::Activate` asks the UI to raise the focused window with
/// the launcher's token.
#[test]
fn reveal_opens_the_folder_in_a_tab_and_selects_the_files_once_listed() {
    let mut h = harness();
    h.settle();
    let windows = h.app.model().windows().count();
    let root = h.root.path().to_path_buf();
    h.send(Command::Reveal {
        items: vec![local(&root.join("a.txt")), local(&root.join("c.txt"))],
        new_window: false,
        properties: false,
    });
    h.settle();
    let focused = h.app.model().focused;
    let (_, tab) = h
        .app
        .model()
        .active_tab(focused)
        .unwrap_or_else(|| panic!("a focused tab"));
    assert_eq!(tab.location, local(&root));
    let mut names: Vec<String> = tab.selected().map(|e| e.name.clone()).collect();
    names.sort();
    assert_eq!(names, ["a.txt", "c.txt"]);
    assert_eq!(h.app.model().windows().count(), windows);
    assert!(
        h.events
            .iter()
            .any(|e| matches!(e, Event::SelectionChanged { pane, ids } if *pane == focused && ids.len() == 2)),
        "{:?}",
        h.events
    );

    // A window of its own for the first folder; a name that is not there
    // selects nothing once the listing is complete.
    h.send(Command::Reveal {
        items: vec![local(&root.join("sub")), local(&root.join("nowhere"))],
        new_window: true,
        properties: false,
    });
    h.settle();
    assert_eq!(h.app.model().windows().count(), windows + 1);
    let focused = h.app.model().focused;
    let (_, tab) = h
        .app
        .model()
        .active_tab(focused)
        .unwrap_or_else(|| panic!("a focused tab"));
    assert_eq!(tab.location, local(&root));
    let names: Vec<String> = tab.selected().map(|e| e.name.clone()).collect();
    assert_eq!(names, ["sub"]);

    h.send(Command::Activate {
        token: Some("token-1".to_owned()),
    });
    assert!(
        h.events.iter().any(|e| matches!(
            e,
            Event::Activate { window, token } if *window == h.app.model().focused_window && token.as_deref() == Some("token-1")
        )),
        "{:?}",
        h.events
    );
}

/// The Properties dialogue on a folder (SPEC.md *Dialogues*): the entry
/// arrives with its contents still being counted, the count follows as an
/// event of its own and is no job of the operations drawer; asked again
/// for the same folder the application answers with the count it has; a
/// file is not counted.
#[test]
fn properties_count_a_folder_s_contents_in_the_background() {
    let mut h = harness();
    h.settle();
    let sub = h.root.path().join("sub");
    fs::create_dir(sub.join("deeper")).unwrap_or_else(|e| panic!("{e}"));
    fs::write(sub.join("one.bin"), vec![0u8; 1000]).unwrap_or_else(|e| panic!("{e}"));
    fs::write(sub.join("deeper/two.bin"), vec![0u8; 500]).unwrap_or_else(|e| panic!("{e}"));
    let pane = h.app.model().focused;
    let select = |h: &mut Harness, name: &str| {
        let id = h
            .tab(pane)
            .listing
            .rows()
            .find(|e| e.name == name)
            .map_or_else(|| panic!("{name} is listed"), |e| e.id);
        h.send(Command::Select {
            pane,
            change: SelectionChange::Set(vec![id]),
        });
    };
    let shown = |events: &[Event]| {
        events.iter().rev().find_map(|e| match e {
            Event::Properties(info) => Some((info.entry.name.clone(), info.contents)),
            _ => None,
        })
    };
    let counted = |events: &[Event]| {
        events.iter().find_map(|e| match e {
            Event::PropertiesCounted { path, contents, .. } => Some((path.clone(), *contents)),
            _ => None,
        })
    };

    select(&mut h, "sub");
    h.events.clear();
    h.send(Command::ShowProperties(pane));
    h.settle();
    let (path, contents) = counted(&h.events).unwrap_or_else(|| panic!("{:?}", h.events));
    assert_eq!(path, sub);
    assert_eq!(contents.items, 3, "two files and the folder between them");
    assert_eq!(contents.bytes, 1500);
    // The entry came with the count running, or already with its result.
    let (name, first) = shown(&h.events).unwrap_or_else(|| panic!("{:?}", h.events));
    assert_eq!(name, "sub");
    assert!(
        first == Contents::Counting || first == Contents::Counted(contents),
        "{first:?}"
    );
    assert!(
        h.app.model().jobs.is_empty(),
        "the count is not in the drawer: {:?}",
        h.app.model().jobs
    );

    // Asked again while the dialogue shows the folder: the count it has.
    h.events.clear();
    h.send(Command::ShowProperties(pane));
    h.settle();
    assert_eq!(
        shown(&h.events),
        Some(("sub".to_owned(), Contents::Counted(contents)))
    );
    assert!(counted(&h.events).is_none(), "not counted twice");

    // Closed, the count is dropped: the next dialogue counts again.
    h.send(Command::PropertiesClosed);
    h.events.clear();
    h.send(Command::ShowProperties(pane));
    h.settle();
    assert!(counted(&h.events).is_some());

    // A file has nothing to count.
    h.send(Command::PropertiesClosed);
    select(&mut h, "c.txt");
    h.events.clear();
    h.send(Command::ShowProperties(pane));
    h.settle();
    assert_eq!(shown(&h.events), Some(("c.txt".to_owned(), Contents::None)));
    assert!(counted(&h.events).is_none());
}

/// The Properties dialogue's entry is read off the loop, and the
/// application is busy until that read is answered: whoever waits for the
/// application to be idle has the entry by then. On a slow machine the
/// answer used to come after the wait (issue 139, the macOS runner).
#[test]
fn a_properties_read_under_way_counts_as_busy() {
    let mut h = harness();
    h.settle();
    let pane = h.app.model().focused;
    let shown = |events: &[Event]| {
        events.iter().find_map(|e| match e {
            Event::Properties(info) => Some(info.entry.name.clone()),
            _ => None,
        })
    };

    // A file: nothing is counted, so no job stands in for the read.
    let id = h
        .tab(pane)
        .listing
        .rows()
        .find(|e| e.name == "c.txt")
        .map_or_else(|| panic!("c.txt is listed"), |e| e.id);
    h.send(Command::Select {
        pane,
        change: SelectionChange::Set(vec![id]),
    });
    h.settle();
    assert!(!h.app.is_busy());
    h.events.clear();
    h.send(Command::ShowProperties(pane));
    assert!(h.app.model().jobs.is_empty(), "no job runs for a file");
    assert!(h.app.is_busy(), "the entry is being read");
    h.settle();
    assert_eq!(shown(&h.events).as_deref(), Some("c.txt"), "{:?}", h.events);
    assert!(!h.app.is_busy(), "the read was answered");

    // The folder's own properties, with nothing selected, are read through
    // its provider first; the wait covers that too.
    h.send(Command::PropertiesClosed);
    h.send(Command::Select {
        pane,
        change: SelectionChange::Clear,
    });
    h.settle();
    h.events.clear();
    h.send(Command::ShowProperties(pane));
    assert!(h.app.is_busy());
    h.settle();
    assert!(shown(&h.events).is_some(), "{:?}", h.events);
    assert!(!h.app.is_busy());
}

/// Issue 178: Properties that is called off while its entry is still being
/// read does not open when the read is answered, and neither does a
/// request a later one overtook.
#[test]
fn properties_called_off_or_overtaken_do_not_open() {
    let mut h = harness();
    h.settle();
    let pane = h.app.model().focused;
    let id_of = |h: &Harness, name: &str| {
        h.tab(pane)
            .listing
            .rows()
            .find(|e| e.name == name)
            .map_or_else(|| panic!("{name} is listed"), |e| e.id)
    };
    let shown = |events: &[Event]| -> Vec<String> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::Properties(info) => Some(info.entry.name.clone()),
                _ => None,
            })
            .collect()
    };
    let (c, a) = (id_of(&h, "c.txt"), id_of(&h, "a.txt"));
    h.send(Command::Select {
        pane,
        change: SelectionChange::Set(vec![c]),
    });
    h.settle();

    // The read is held back, as a slow volume would hold it.
    h.fake.hold_permissions();
    h.events.clear();
    h.send(Command::ShowProperties(pane));
    assert!(h.app.properties_pending(), "asked for, not open yet");
    // Called off, as Escape does.
    h.send(Command::PropertiesClosed);
    assert!(!h.app.properties_pending());
    assert!(h.app.is_busy(), "the read itself still runs");
    h.fake.release_permissions();
    h.settle();
    assert_eq!(shown(&h.events), Vec::<String>::new(), "it does not open");
    assert!(!h.app.is_busy());

    // Asked again it opens.
    h.events.clear();
    h.send(Command::ShowProperties(pane));
    h.settle();
    assert_eq!(shown(&h.events), ["c.txt"]);
    assert!(!h.app.properties_pending());
    h.send(Command::PropertiesClosed);

    // Overtaken: asked for one entry, then for another before the first
    // was read. Only the one last asked for opens.
    h.fake.hold_permissions();
    h.events.clear();
    h.send(Command::ShowProperties(pane));
    h.send(Command::Select {
        pane,
        change: SelectionChange::Set(vec![a]),
    });
    h.send(Command::ShowProperties(pane));
    h.fake.release_permissions();
    h.settle();
    assert_eq!(shown(&h.events), ["a.txt"]);
}

/// Issue 178, on a provider that is not the local one — where the read is
/// slowest, and where the entry has no local path to tell a stale answer
/// by: Properties called off before its answer is in does not open.
#[test]
fn properties_called_off_on_a_remote_entry_do_not_open() {
    let stub = net::Stub::new();
    let mut h = harness_with(vec![Arc::new(net::StubExtension::new(Arc::clone(&stub)))]);
    h.settle();
    h.send(Command::AddConnection {
        provider: net::ID,
        name: "nas".into(),
        values: vec![("host".into(), "10.0.0.2".into())],
    });
    h.settle();
    let pane = h.app.model().focused;
    h.send(Command::Navigate {
        pane,
        to: Location::new(net::ID, "/"),
    });
    h.settle();
    let id = h
        .tab(pane)
        .listing
        .rows()
        .find(|e| e.name == "nas")
        .map_or_else(|| panic!("nas is listed: {:?}", h.names()), |e| e.id);
    h.send(Command::Select {
        pane,
        change: SelectionChange::Set(vec![id]),
    });
    h.settle();
    let shown = |events: &[Event]| -> Vec<String> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::Properties(info) => Some(info.entry.name.clone()),
                _ => None,
            })
            .collect()
    };

    // Asked for and called off before the answer is taken in.
    h.events.clear();
    h.send(Command::ShowProperties(pane));
    assert!(h.app.properties_pending());
    h.send(Command::PropertiesClosed);
    h.settle();
    assert_eq!(shown(&h.events), Vec::<String>::new(), "{:?}", h.events);

    // Asked for and left alone, it opens.
    h.events.clear();
    h.send(Command::ShowProperties(pane));
    h.settle();
    assert_eq!(shown(&h.events), ["nas"]);
}

/// What the Properties dialogue reads through links: these need a
/// symbolic link to be made, which only Unix lets a test do freely.
#[cfg(unix)]
mod links {
    use std::fs;
    use std::path::{Path, PathBuf};

    use types::{CancelToken, EntryKind, VPath};

    use crate::{count_tree, lexical, link_target};

    /// What a link resolves to (SPEC.md *Dialogues*, Properties): the entry at
    /// the end of the link, under the path the link's text names, a relative
    /// text taken from the link's folder; nothing for a broken link.
    #[test]
    fn a_link_s_target_is_the_entry_it_names_from_its_own_folder() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let bin = dir.path().join("bin");
        let links = dir.path().join("links");
        fs::create_dir(&bin).unwrap_or_else(|e| panic!("{e}"));
        fs::create_dir(&links).unwrap_or_else(|e| panic!("{e}"));
        fs::write(bin.join("tool"), b"abc").unwrap_or_else(|e| panic!("{e}"));
        let link = links.join("tool-link");
        std::os::unix::fs::symlink("../bin/./tool", &link).unwrap_or_else(|e| panic!("{e}"));
        let target = link_target(&link, &VPath::from("../bin/./tool"))
            .unwrap_or_else(|| panic!("the link resolves"));
        assert_eq!(target.name, "tool");
        assert_eq!(target.kind, EntryKind::File);
        assert_eq!(target.size, Some(3));
        assert_eq!(target.path, platform::paths::from_path(&bin.join("tool")));

        let broken = links.join("gone");
        std::os::unix::fs::symlink("nowhere", &broken).unwrap_or_else(|e| panic!("{e}"));
        assert!(link_target(&broken, &VPath::from("nowhere")).is_none());

        assert_eq!(lexical(Path::new("/a/b/../c/./d")), PathBuf::from("/a/c/d"));
    }

    /// The count walks everything below the folder, never through a link, and
    /// stops when it is cancelled.
    #[test]
    fn the_count_follows_no_link_and_stops_when_cancelled() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let inside = dir.path().join("inside");
        let outside = dir.path().join("outside");
        fs::create_dir_all(inside.join("sub")).unwrap_or_else(|e| panic!("{e}"));
        fs::create_dir(&outside).unwrap_or_else(|e| panic!("{e}"));
        fs::write(inside.join("a"), vec![0u8; 10]).unwrap_or_else(|e| panic!("{e}"));
        fs::write(inside.join("sub/b"), vec![0u8; 20]).unwrap_or_else(|e| panic!("{e}"));
        fs::write(outside.join("big"), vec![0u8; 5000]).unwrap_or_else(|e| panic!("{e}"));
        std::os::unix::fs::symlink(&outside, inside.join("out")).unwrap_or_else(|e| panic!("{e}"));
        let contents =
            count_tree(&inside, &CancelToken::new()).unwrap_or_else(|| panic!("not cancelled"));
        assert_eq!(contents.items, 4, "a, sub, sub/b and the link itself");
        let link = fs::symlink_metadata(inside.join("out"))
            .unwrap_or_else(|e| panic!("{e}"))
            .len();
        assert_eq!(contents.bytes, 30 + link, "nothing from behind the link");

        let cancelled = CancelToken::new();
        cancelled.cancel();
        assert!(count_tree(&inside, &cancelled).is_none());
    }
}

/// `Command::Reveal` with `properties` (specs/system-integration.md §4,
/// `ShowItemProperties`): the Properties dialogue opens on the file once
/// it is listed and selected, and not before.
#[test]
fn reveal_with_properties_opens_properties_on_the_selected_file() {
    let mut h = harness();
    h.settle();
    let root = h.root.path().to_path_buf();
    h.send(Command::Reveal {
        items: vec![local(&root.join("c.txt"))],
        new_window: false,
        properties: true,
    });
    assert!(
        !h.events.iter().any(|e| matches!(e, Event::Properties(_))),
        "nothing is listed yet"
    );
    h.settle();
    let focused = h.app.model().focused;
    let deadline = Instant::now() + Duration::from_secs(10);
    while !h.events.iter().any(|e| {
        matches!(e, Event::Properties(info) if info.pane == focused && info.entry.name == "c.txt")
    }) {
        assert!(
            Instant::now() < deadline,
            "no Properties: {:?}",
            h.events
        );
        h.settle();
    }
    let (_, tab) = h
        .app
        .model()
        .active_tab(focused)
        .unwrap_or_else(|| panic!("a focused tab"));
    assert_eq!(tab.location, local(&root));
    assert_eq!(
        tab.selected().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        ["c.txt"]
    );

    // A name that is not there selects nothing and opens nothing.
    h.events.clear();
    h.send(Command::Reveal {
        items: vec![local(&root.join("sub").join("nowhere"))],
        new_window: false,
        properties: true,
    });
    h.settle();
    h.settle();
    assert!(
        !h.events.iter().any(|e| matches!(e, Event::Properties(_))),
        "{:?}",
        h.events
    );
}

/// A console whose working directory and foreground job the test sets,
/// and which records what is typed into it.
#[derive(Default)]
struct FakeConsole {
    cwd: Mutex<Option<std::path::PathBuf>>,
    job: Mutex<Option<bool>>,
    typed: Mutex<Vec<u8>>,
}

impl ext::Console for FakeConsole {
    fn title(&self) -> String {
        "fake".to_owned()
    }
    fn screen(&self, _: usize) -> ext::Screen {
        ext::Screen {
            cols: 80,
            rows: 24,
            cells: Vec::new(),
            cursor: None,
            scrollback: 0,
            application_cursor: false,
        }
    }
    fn write(&self, bytes: &[u8]) {
        self.typed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(bytes);
    }
    fn resize(&self, _: u16, _: u16) {}
    fn exit_status(&self) -> Option<i32> {
        None
    }
    fn set_waker(&self, _: ext::ConsoleWaker) {}
    fn generation(&self) -> u64 {
        0
    }
    fn cwd(&self) -> Option<std::path::PathBuf> {
        self.cwd
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
    fn foreground_job(&self) -> Option<bool> {
        *self.job.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// An extension whose one action opens the fake console as a view.
struct ConsoleOpener {
    manifest: Manifest,
    console: Arc<FakeConsole>,
    /// The id its action goes by.
    action: ActionId,
}

const OPEN_FAKE_CONSOLE: ActionId = ActionId("fake.console");

struct OpenFakeConsole(Arc<FakeConsole>, ActionId);

impl ext::Action for OpenFakeConsole {
    fn id(&self) -> ActionId {
        self.1
    }
    fn label(&self) -> String {
        "Fake console".to_owned()
    }
    fn group(&self) -> ext::ActionGroup {
        ext::ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<ext::Shortcut> {
        None
    }
    fn availability(&self, _: &ext::ActionCtx) -> Availability {
        Availability::Enabled
    }
    fn run(
        &self,
        _: ext::ActionCtx,
        _: &ext::JobRunner,
        _: &dyn UiHost,
    ) -> Result<ext::ActionOutcome, Error> {
        Ok(ext::ActionOutcome::View(ext::View::Console(
            Arc::clone(&self.0) as Arc<dyn ext::Console>,
        )))
    }
}

impl Extension for ConsoleOpener {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        reg.action(Arc::new(OpenFakeConsole(
            Arc::clone(&self.console),
            self.action,
        )));
        Ok(())
    }
}

/// Writes `bytes` to `name` in the harness's root and sets its execute
/// bit, or leaves it off.
#[cfg(unix)]
fn program(h: &Harness, name: &str, bytes: &[u8], executable: bool) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let path = h.root.path().join(name);
    fs::write(&path, bytes).unwrap_or_else(|e| panic!("{e}"));
    let mode = if executable { 0o755 } else { 0o644 };
    fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap_or_else(|e| panic!("{e}"));
    path
}

/// Opens the entry named `name` in the focused pane, as Enter does.
#[cfg(unix)]
fn open_named(h: &mut Harness, name: &str) {
    let pane = h.app.model().focused;
    let id = id_of(h, name);
    h.send(Command::Open { pane, id });
}

/// The question that is open, once one is.
#[cfg(unix)]
fn question(h: &mut Harness) -> types::Question {
    assert!(
        wait_until(h, Duration::from_secs(5), |h| {
            !h.app.model().questions.is_empty()
        }),
        "no question was asked"
    );
    h.app.model().questions[0].clone()
}

/// Acceptance (issue 149, item 6): a binary with the execute bit starts
/// on Enter, detached, in its folder and without a question; nothing is
/// handed to the system.
#[cfg(unix)]
#[test]
fn a_binary_with_the_execute_bit_runs_detached_in_its_folder() {
    let mut h = harness();
    let tool = program(&h, "tool", b"\x7fELF\x02\x01\x01\0\0\0\0\0\0\0\0\0", true);
    h.settle();
    open_named(&mut h, "tool");
    assert!(wait_until(&mut h, Duration::from_secs(5), |h| {
        !h.fake.ran().is_empty()
    }));
    assert_eq!(
        h.fake.ran(),
        [(tool, Vec::new(), h.root.path().to_path_buf())]
    );
    h.settle();
    assert!(h.app.model().questions.is_empty(), "no question");
    assert!(h.fake.opened().is_empty(), "not handed to the system");
}

/// An executable script asks first, *Run script?* with Cancel, Edit and
/// Run, Run being the one Enter picks. Run starts it in the system's
/// terminal in its folder where no embedded console is loaded; Edit opens
/// it in the editor; Cancel does nothing.
#[cfg(unix)]
#[test]
fn an_executable_script_asks_to_run_edit_or_cancel() {
    let mut h = harness();
    let script = program(&h, "build.sh", b"#!/bin/sh\necho built\n", true);
    h.settle();
    let answer = |h: &mut Harness, choice: usize| {
        open_named(h, "build.sh");
        let asked = question(h);
        assert_eq!(asked.title, "Run script?");
        assert_eq!(asked.text, "build.sh");
        assert_eq!(
            asked.kind,
            types::QuestionKind::Choice {
                options: vec!["Cancel".into(), "Edit".into(), "Run".into()],
                default: 2,
            }
        );
        h.send(Command::AnswerQuestion {
            id: asked.id,
            answer: types::Answer::Choice(choice),
        });
        h.settle();
        assert!(h.app.model().questions.is_empty(), "answered");
    };

    // Cancel: nothing runs, nothing opens.
    answer(&mut h, 0);
    assert!(h.fake.ran().is_empty() && h.fake.terminals().is_empty());
    assert!(h.fake.edited().is_empty() && h.fake.opened().is_empty());

    // Run: a terminal in the script's folder, running it.
    answer(&mut h, 2);
    assert_eq!(
        h.fake.terminals(),
        [(
            h.root.path().to_path_buf(),
            Some(vec![script.to_string_lossy().into_owned()])
        )]
    );
    assert!(h.fake.ran().is_empty(), "a script is not started blind");

    // Edit: the editor.
    answer(&mut h, 1);
    assert_eq!(h.fake.edited(), [vec![script.clone()]]);
    assert_eq!(h.fake.terminals().len(), 1);

    // Escape, or closing the dialogue: as Cancel.
    open_named(&mut h, "build.sh");
    let asked = question(&mut h);
    h.send(Command::AnswerQuestion {
        id: asked.id,
        answer: types::Answer::Cancelled,
    });
    h.settle();
    assert_eq!(h.fake.terminals().len(), 1);
    assert_eq!(h.fake.edited().len(), 1);
}

/// With the embedded console loaded, Run opens a console in the script's
/// folder and types the script at its prompt.
#[cfg(unix)]
#[test]
fn run_starts_a_script_in_the_embedded_console_where_it_is_loaded() {
    let console = Arc::new(FakeConsole::default());
    let mut h = harness_with(vec![console_opener(&console, ActionId("terminal.console"))]);
    program(&h, "it's a build.sh", b"#!/bin/sh\necho built\n", true);
    h.settle();
    // The shell is busy starting: nothing is typed yet.
    *console.job.lock().unwrap_or_else(PoisonError::into_inner) = Some(true);
    open_named(&mut h, "it's a build.sh");
    let asked = question(&mut h);
    h.send(Command::AnswerQuestion {
        id: asked.id,
        answer: types::Answer::Choice(2),
    });
    h.settle();
    let typed = |console: &FakeConsole| {
        String::from_utf8_lossy(&console.typed.lock().unwrap_or_else(PoisonError::into_inner))
            .into_owned()
    };
    assert!(
        h.events
            .iter()
            .any(|e| matches!(e, Event::ViewOpened { .. })),
        "the console is opened: {:?}",
        h.events
    );
    assert_eq!(typed(&console), "");
    // At its prompt: the script, quoted, and Enter.
    *console.job.lock().unwrap_or_else(PoisonError::into_inner) = Some(false);
    assert!(wait_until(&mut h, Duration::from_secs(5), |_| {
        !typed(&console).is_empty()
    }));
    assert_eq!(typed(&console), "'./it'\\''s a build.sh'\r");
    assert!(h.fake.terminals().is_empty(), "not the system's terminal");
}

/// A script without the execute bit is a text file like any other; and
/// where the execute bit makes no program, as on Windows, every file is
/// handed to the system as before.
#[cfg(unix)]
#[test]
fn without_the_execute_bit_a_file_opens_with_the_system_default() {
    let mut h = harness();
    let plain = program(&h, "notes.sh", b"#!/bin/sh\necho hi\n", false);
    let tool = program(&h, "tool", b"\x7fELF\0\0\0\0", true);
    h.settle();
    open_named(&mut h, "notes.sh");
    assert!(wait_until(&mut h, Duration::from_secs(5), |h| {
        h.fake.opened() == [plain.clone()]
    }));
    assert!(h.app.model().questions.is_empty(), "nothing is asked");

    h.fake.set_execute_bit(false);
    open_named(&mut h, "tool");
    assert!(wait_until(&mut h, Duration::from_secs(5), |h| {
        h.fake.opened() == [plain.clone(), tool.clone()]
    }));
    assert!(h.fake.ran().is_empty(), "nothing is started");
}

/// An extension standing in for `ext-installer`: its one action goes by
/// the id of *Run* on an AppImage, applies to a file named `*.AppImage`
/// and records the files it was run on.
struct AppImageRunner {
    manifest: Manifest,
    ran: Arc<Mutex<Vec<String>>>,
}

struct FakeAppImageRun(Arc<Mutex<Vec<String>>>);

impl ext::Action for FakeAppImageRun {
    fn id(&self) -> ActionId {
        ActionId("installer.run")
    }
    fn label(&self) -> String {
        "Run".to_owned()
    }
    fn group(&self) -> ext::ActionGroup {
        ext::ActionGroup::Extension("Installer")
    }
    fn default_shortcut(&self) -> Option<ext::Shortcut> {
        None
    }
    fn availability(&self, ctx: &ext::ActionCtx) -> Availability {
        match ctx.selection.as_slice() {
            [entry] if entry.name.ends_with(".AppImage") => Availability::Enabled,
            _ => Availability::Hidden,
        }
    }
    fn run(
        &self,
        ctx: ext::ActionCtx,
        _: &ext::JobRunner,
        _: &dyn UiHost,
    ) -> Result<ext::ActionOutcome, Error> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend(ctx.selection.iter().map(|entry| entry.name.clone()));
        Ok(ext::ActionOutcome::Done)
    }
}

impl Extension for AppImageRunner {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        reg.action(Arc::new(FakeAppImageRun(Arc::clone(&self.ran))));
        Ok(())
    }
}

/// Enter and a double-click on an AppImage do what *Run* in its context
/// menu does, with the execute bit and without it (issue 223, decided
/// 2026-11-15): the installer's action is run on the file, which sets the
/// bit and makes its checks, and Rick neither starts the file itself nor
/// hands it to the system. What the action does not take opens as before.
#[test]
fn enter_on_an_appimage_runs_the_installers_run() {
    let ran = Arc::new(Mutex::new(Vec::new()));
    let mut h = harness_with(vec![Arc::new(AppImageRunner {
        manifest: Manifest {
            id: ExtId("fake-installer"),
            name: "Fake installer",
            description: "",
            version: Version::new(0, 1, 0),
            api_version: API_VERSION,
            requires: Capabilities::empty(),
            points: Points::ACTION,
            platforms: Platforms::ALL,
            previews: &[],
            markers: &[],
            connections: &[],
            commands: &[],
        },
        ran: Arc::clone(&ran),
    })]);
    let elf = b"\x7fELF\x02\x01\x01\0AI\x02\0\0\0\0\0";
    for (name, mode) in [("Downloaded.AppImage", 0o644), ("Marked.AppImage", 0o755)] {
        let path = h.root.path().join(name);
        fs::write(&path, elf).unwrap_or_else(|e| panic!("{e}"));
        platform::fs::set_unix_mode(&path, mode).unwrap_or_else(|e| panic!("{e}"));
    }
    // Listed by the start location's read of the folder, or by the
    // watcher's report when the read came first: waited for either way.
    let listed = wait_until(&mut h, Duration::from_secs(10), |h| {
        let names = h.names();
        ["Downloaded.AppImage", "Marked.AppImage"]
            .iter()
            .all(|name| names.iter().any(|listed| listed == name))
    });
    assert!(listed, "the two files are not listed: {:?}", h.names());
    h.settle();
    let seen =
        |ran: &Mutex<Vec<String>>| ran.lock().unwrap_or_else(PoisonError::into_inner).clone();
    let open = |h: &mut Harness, name: &str| {
        let pane = h.app.model().focused;
        let id = id_of(h, name);
        h.send(Command::Open { pane, id });
        h.settle();
    };

    open(&mut h, "Downloaded.AppImage");
    assert_eq!(seen(&ran), ["Downloaded.AppImage"]);
    open(&mut h, "Marked.AppImage");
    assert_eq!(seen(&ran), ["Downloaded.AppImage", "Marked.AppImage"]);
    assert!(h.fake.ran().is_empty(), "the action starts it, not Rick");
    assert!(h.fake.opened().is_empty(), "not handed to the system");

    // What the action does not take is opened as before.
    open(&mut h, "a.txt");
    assert!(wait_until(&mut h, Duration::from_secs(5), |h| {
        h.fake.opened() == [h.root.path().join("a.txt")]
    }));
    assert_eq!(seen(&ran).len(), 2);
}

/// Issue 221, item 3 (SPEC.md *Icons view for a folder of media* and
/// *View mode*), through real listings. A folder of eight photos, a video
/// and a text file opens in Icons, and going back to its parent shows the
/// parent in the view it had; a folder of seven photos and three text
/// files stays as it was; Details chosen inside the media folder stays,
/// a refresh included, until the tab leaves it. Choosing Large is written
/// to the settings file, and an application started on that file opens
/// Icons in Large, by the rule too.
#[test]
fn a_folder_of_media_opens_in_icons_in_the_size_chosen_last() {
    use model::{IconSize, ViewMode};
    let mut h = harness();
    h.settle();
    let pane = h.app.model().focused;
    let photos = h.root.path().join("photos");
    let mixed = h.root.path().join("mixed");
    for dir in [&photos, &mixed] {
        fs::create_dir(dir).unwrap_or_else(|e| panic!("{e}"));
    }
    let put = |dir: &Path, name: &str| {
        fs::write(dir.join(name), "x").unwrap_or_else(|e| panic!("{e}"));
    };
    for i in 0..8 {
        put(&photos, &format!("photo-{i}.jpg"));
    }
    put(&photos, "clip.mp4");
    put(&photos, "notes.txt");
    for i in 0..7 {
        put(&mixed, &format!("photo-{i}.jpg"));
    }
    for name in ["a.txt", "b.txt", "c.txt"] {
        put(&mixed, name);
    }
    let go = |h: &mut Harness, to: &Path| {
        h.send(Command::Navigate {
            pane,
            to: local(to),
        });
        h.settle();
    };
    assert_eq!(h.tab(pane).view, ViewMode::Details);

    // Nine media files in ten: Icons, and the parent as it was.
    go(&mut h, &photos);
    assert_eq!(h.tab(pane).view, ViewMode::Icons(IconSize::Medium));
    h.send(Command::Back(pane));
    h.settle();
    assert_eq!(h.tab(pane).view, ViewMode::Details, "the parent's view");

    // Seven in ten: as it was.
    go(&mut h, &mixed);
    assert_eq!(h.tab(pane).view, ViewMode::Details);

    // Details by hand inside the media folder stays, a refresh included.
    go(&mut h, &photos);
    assert_eq!(h.tab(pane).view, ViewMode::Icons(IconSize::Medium));
    h.send(Command::SetView {
        pane,
        mode: ViewMode::Details,
    });
    h.send(Command::Refresh(pane));
    h.settle();
    assert_eq!(h.tab(pane).view, ViewMode::Details, "nothing flips back");
    assert_eq!(h.app.model().settings.icon_size, IconSize::Medium);

    // A size chosen by hand is kept in the settings file.
    h.send(Command::SetView {
        pane,
        mode: ViewMode::Icons(IconSize::Large),
    });
    assert_eq!(h.app.model().settings.icon_size, IconSize::Large);
    let file = h.fake.config_dir().join(SETTINGS_FILE);
    assert!(
        wait_until(&mut h, Duration::from_secs(2), |_| {
            fs::read_to_string(&file).is_ok_and(|t| t.contains("icon_size = \"large\""))
        }),
        "the size is in the settings file"
    );
    // Leaving puts back the view the tab had before the folder all the
    // same: what was picked there belonged to the stay.
    go(&mut h, &mixed);
    assert_eq!(h.tab(pane).view, ViewMode::Details);

    // Started again on the same configuration, in the media folder.
    h.app = App::new(Config {
        platform: h.fake.clone(),
        ui: h.ui.clone(),
        extensions: Vec::new(),
        config_dir: Some(h.fake.config_dir()),
        start: Some(local(&photos)),
    });
    h.settle();
    let pane = h.app.model().focused;
    assert_eq!(h.app.model().settings.icon_size, IconSize::Large);
    assert_eq!(
        h.tab(pane).view,
        ViewMode::Icons(IconSize::Large),
        "the rule opens Icons in the size chosen last"
    );
}

/// Acceptance (issue 149, item 6): Open in editor opens the selected
/// files in the system's default text editor, or with the command the
/// setting names, split at spaces outside double quotes, the files
/// appended; it is not offered for a folder.
#[test]
fn open_in_editor_uses_the_systems_editor_or_the_command_the_setting_names() {
    let mut h = harness();
    h.settle();
    let pane = h.app.model().focused;
    let (a, b) = (h.root.path().join("a.txt"), h.root.path().join("b.txt"));
    let select = |h: &mut Harness, names: &[&str]| {
        let ids = names.iter().map(|name| id_of(h, name)).collect();
        h.send(Command::Select {
            pane,
            change: SelectionChange::Set(ids),
        });
    };
    select(&mut h, &["a.txt"]);
    h.run(pane, "core.open_in_editor");
    h.settle();
    assert_eq!(h.fake.edited(), [vec![a.clone()]]);
    assert!(h.fake.ran().is_empty(), "the editor is no program");

    h.send(Command::Settings(SettingsChange::Editor(
        "code --wait \"--user-data-dir=/tmp/my code\"".into(),
    )));
    select(&mut h, &["a.txt", "b.txt"]);
    h.run(pane, "core.open_in_editor");
    h.settle();
    assert_eq!(
        h.fake.ran(),
        [(
            std::path::PathBuf::from("code"),
            vec![
                "--wait".into(),
                "--user-data-dir=/tmp/my code".into(),
                a.clone().into_os_string(),
                b.into_os_string(),
            ],
            h.root.path().to_path_buf(),
        )]
    );
    assert_eq!(
        h.fake.edited().len(),
        1,
        "the setting's editor, not the system's"
    );

    // A folder is not offered the editor: nothing happens, nothing is said.
    select(&mut h, &["sub"]);
    h.take_events();
    h.run(pane, "core.open_in_editor");
    h.settle();
    assert_eq!(h.fake.ran().len(), 1);
    assert!(
        !h.take_events()
            .iter()
            .any(|e| matches!(e, Event::Notify { .. }))
    );
    // F4 is its key.
    let action = h
        .app
        .registry()
        .actions()
        .iter()
        .find(|action| action.id() == ActionId("core.open_in_editor"))
        .cloned()
        .unwrap_or_else(|| panic!("the action"));
    assert_eq!(
        action.default_shortcut().map(|s| s.to_string()),
        Some("F4".to_owned())
    );
    assert_eq!(action.label(), "Open in editor");
}

/// The extension whose action, known as `action`, opens `console`.
fn console_opener(console: &Arc<FakeConsole>, action: ActionId) -> Arc<dyn Extension> {
    Arc::new(ConsoleOpener {
        manifest: Manifest {
            id: ExtId("fake-console"),
            name: "Fake console",
            description: "",
            version: Version::new(0, 1, 0),
            api_version: API_VERSION,
            requires: Capabilities::empty(),
            points: Points::ACTION,
            platforms: Platforms::ALL,
            previews: &[],
            markers: &[],
            connections: &[],
            commands: &[],
        },
        console: Arc::clone(console),
        action,
    })
}

/// A harness with the fake console opened from its explorer tab: the
/// harness, the console and the pane both are in.
fn with_fake_console() -> (Harness, Arc<FakeConsole>, model::PaneId) {
    let console = Arc::new(FakeConsole::default());
    let opener = console_opener(&console, OPEN_FAKE_CONSOLE);
    let mut h = harness_with(vec![opener]);
    h.settle();
    let pane = h.app.model().focused;
    let ctx = h
        .app
        .action_ctx(pane, Invocation::Palette)
        .unwrap_or_else(|| panic!("a context"));
    h.send(Command::Run {
        action: OPEN_FAKE_CONSOLE,
        ctx,
    });
    h.settle();
    assert!(
        h.events
            .iter()
            .any(|e| matches!(e, Event::ViewOpened { pane: p, .. } if *p == pane)),
        "{:?}",
        h.events
    );
    (h, console, pane)
}

/// The location of the explorer tab in `pane`.
fn explorer_at(h: &Harness, pane: model::PaneId) -> Location {
    h.app.model().active_tab(pane).map_or_else(
        || panic!("the explorer tab"),
        |(_, tab)| tab.location.clone(),
    )
}

/// The console follows the tab and the tab follows the console
/// (specs/system-integration.md §4): the shell's working directory moves
/// the explorer tab the console was opened from; the explorer's
/// navigation types `cd` into the console at the prompt, and nothing
/// while a job has the terminal or the session cannot tell; a report of
/// the directory the tab is at already types nothing back.
#[test]
fn a_console_follows_its_tab_and_the_tab_follows_the_console() {
    let (mut h, console, pane) = with_fake_console();
    let root = h
        .root
        .path()
        .canonicalize()
        .unwrap_or_else(|e| panic!("{e}"));
    let sub = root.join("sub");
    let explorer = |h: &Harness| explorer_at(h, pane);
    assert_eq!(explorer(&h), local(h.root.path()));

    // The shell changes directory: the tab follows on the next poll (the
    // UI polls every frame, and the console's waker asks for one), and
    // nothing is typed back for a move the console itself made.
    *console.cwd.lock().unwrap_or_else(PoisonError::into_inner) = Some(sub.clone());
    h.app.poll(&mut h.events);
    h.settle();
    assert_eq!(explorer(&h), local(&sub));
    assert!(
        console
            .typed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    );

    // The tab navigates while the shell is at its prompt: `cd` is typed.
    *console.job.lock().unwrap_or_else(PoisonError::into_inner) = Some(false);
    h.send(Command::Navigate {
        pane,
        to: local(&root),
    });
    h.settle();
    let typed =
        String::from_utf8_lossy(&console.typed.lock().unwrap_or_else(PoisonError::into_inner))
            .into_owned();
    assert!(
        typed.contains("cd ")
            && typed.contains(&root.to_string_lossy().into_owned())
            && typed.ends_with('\r'),
        "{typed:?}"
    );
    let before = typed.len();

    // A job has the terminal, or the session cannot tell: nothing is
    // typed into it.
    *console.job.lock().unwrap_or_else(PoisonError::into_inner) = Some(true);
    h.send(Command::Navigate {
        pane,
        to: local(&sub),
    });
    h.settle();
    *console.job.lock().unwrap_or_else(PoisonError::into_inner) = None;
    h.send(Command::Navigate {
        pane,
        to: local(&root),
    });
    h.settle();
    assert_eq!(
        console
            .typed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len(),
        before,
        "nothing typed while a job runs or the state is unknown"
    );
}

/// *Shell integration* switched off in Settings → Integration
/// (specs/system-integration.md §6.1): the console and the tab go their
/// own ways in both directions, and switching it on again catches up on
/// nothing — the next move is followed.
#[test]
fn shell_integration_switched_off_unties_the_console_and_the_tab() {
    let (mut h, console, pane) = with_fake_console();
    let root = h
        .root
        .path()
        .canonicalize()
        .unwrap_or_else(|e| panic!("{e}"));
    let sub = root.join("sub");
    let typed = |console: &FakeConsole| {
        console
            .typed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    };
    h.send(Command::Settings(SettingsChange::ShellIntegration(false)));
    *console.job.lock().unwrap_or_else(PoisonError::into_inner) = Some(false);
    h.send(Command::Navigate {
        pane,
        to: local(&sub),
    });
    h.settle();
    assert_eq!(typed(&console), 0, "no cd typed with shell integration off");
    *console.cwd.lock().unwrap_or_else(PoisonError::into_inner) = Some(root.clone());
    h.app.poll(&mut h.events);
    h.settle();
    assert_eq!(explorer_at(&h, pane), local(&sub), "the tab stays put");
    // Switched on again, nothing is caught up on: the next move follows.
    h.send(Command::Settings(SettingsChange::ShellIntegration(true)));
    h.app.poll(&mut h.events);
    h.settle();
    assert_eq!(explorer_at(&h, pane), local(&sub));
    assert_eq!(typed(&console), 0);
    let elsewhere = root.join("elsewhere");
    fs::create_dir(&elsewhere).unwrap_or_else(|e| panic!("{e}"));
    *console.cwd.lock().unwrap_or_else(PoisonError::into_inner) = Some(elsewhere.clone());
    h.app.poll(&mut h.events);
    h.settle();
    assert_eq!(
        explorer_at(&h, pane),
        local(&elsewhere),
        "the tab follows the console again"
    );
}

mod integration {
    //! The integration check and apply (specs/system-integration.md §6.1)
    //! against a scripted platform: nothing here touches a real system.

    use model::{ItemState, ItemStatus};
    use platform::FakeIntegration;
    use platform::integration::{Actual, Call, Check, Desktop, Item, Reason, Scope};

    use super::*;
    use crate::IntegrationCommand;

    /// A harness on a GNOME desktop with three items: the launcher entry
    /// missing, the `rick` command set up, the dock favourite missing.
    fn scripted() -> (Harness, FakeIntegration) {
        let h = harness();
        let fake = FakeIntegration::new("Fedora Linux 43 · GNOME", Some(Desktop::Gnome));
        fake.set(Item::LauncherEntry, Check::missing());
        fake.set(Item::RickCommand, Check::set_up());
        fake.set(Item::DockFavourite, Check::missing());
        h.fake.set_integration(Arc::new(fake.clone()));
        (h, fake)
    }

    fn check(h: &mut Harness, startup: bool) {
        h.send(Command::Integration(IntegrationCommand::Check { startup }));
        h.settle();
    }

    fn status(h: &Harness, item: Item) -> Option<ItemStatus> {
        h.app
            .model()
            .integration
            .get(item)
            .map(|row| row.status.clone())
    }

    fn state(h: &Harness, item: Item) -> Option<ItemState> {
        h.app
            .model()
            .settings
            .integration
            .record(item)
            .map(|record| record.state)
    }

    /// Whether the events hold the check's answer, and what it says about
    /// asking.
    fn asked(events: &[Event]) -> Option<bool> {
        events.iter().find_map(|event| match event {
            Event::IntegrationChecked { ask } => Some(*ask),
            _ => None,
        })
    }

    #[test]
    fn the_startup_check_reads_every_item_and_asks_about_the_missing_ones() {
        let (mut h, fake) = scripted();
        h.settle();
        assert!(
            h.app.model().integration.items.is_empty(),
            "before the check"
        );
        check(&mut h, true);
        let integration = &h.app.model().integration;
        assert_eq!(integration.system, "Fedora Linux 43 · GNOME");
        assert_eq!(integration.desktop, Some(Desktop::Gnome));
        assert!(integration.checked_at.is_some() && integration.at_startup);
        assert!(!integration.checking);
        assert_eq!(status(&h, Item::LauncherEntry), Some(ItemStatus::Missing));
        assert_eq!(status(&h, Item::RickCommand), Some(ItemStatus::SetUp));
        assert_eq!(status(&h, Item::DockFavourite), Some(ItemStatus::Missing));
        assert_eq!(asked(&h.take_events()), Some(true), "something is missing");
        // The check read and changed nothing.
        assert_eq!(
            fake.calls(),
            [
                Call::Check(Item::LauncherEntry),
                Call::Check(Item::RickCommand),
                Call::Check(Item::DockFavourite)
            ]
        );
        assert_eq!(fake.actual(Item::LauncherEntry), Some(Actual::Missing));
        // The records are in rick.toml, each with the day of the check.
        assert_eq!(state(&h, Item::RickCommand), Some(ItemState::SetUp));
        let text = read_until(h.app.settings_path(), |text| text.contains("rick-command"));
        assert!(text.contains("[integration.items.rick-command]"), "{text}");
        assert!(text.contains("state = \"set-up\""), "{text}");
        assert!(text.contains("state = \"missing\""), "{text}");
        assert!(text.contains("checked = 20"), "{text}");
    }

    #[test]
    fn the_startup_check_can_be_switched_off_and_the_dialog_silenced() {
        let (mut h, fake) = scripted();
        h.settle();
        // *Don't ask again*: the check still runs, the dialog stays shut.
        h.send(Command::Settings(SettingsChange::IntegrationAsk(false)));
        check(&mut h, true);
        assert_eq!(asked(&h.take_events()), Some(false));
        assert_eq!(status(&h, Item::LauncherEntry), Some(ItemStatus::Missing));
        // *Check integration at startup* off: no check at startup at all,
        // while *Check again* still checks.
        h.send(Command::Settings(
            SettingsChange::IntegrationCheckAtStartup(false),
        ));
        let before = fake.calls().len();
        check(&mut h, true);
        assert_eq!(asked(&h.take_events()), None);
        assert_eq!(fake.calls().len(), before);
        check(&mut h, false);
        assert_eq!(
            asked(&h.take_events()),
            Some(false),
            "not the startup check"
        );
        assert!(fake.calls().len() > before);
        assert!(!h.app.model().integration.at_startup);
    }

    #[test]
    fn apply_performs_the_rows_on_declines_the_rows_off_and_records_both() {
        let (mut h, fake) = scripted();
        h.settle();
        check(&mut h, true);
        h.take_events();
        h.send(Command::Integration(IntegrationCommand::Apply {
            scope: Scope::User,
            perform: vec![Item::LauncherEntry],
            decline: vec![Item::DockFavourite],
            replace: Vec::new(),
        }));
        h.settle();
        let events = h.take_events();
        assert!(
            events.iter().any(
                |e| matches!(e, Event::IntegrationApplied { failed, .. } if failed.is_empty())
            ),
            "{events:?}"
        );
        assert_eq!(status(&h, Item::LauncherEntry), Some(ItemStatus::SetUp));
        assert_eq!(status(&h, Item::DockFavourite), Some(ItemStatus::Declined));
        assert_eq!(h.app.model().integration.applying, None);
        // What the perform wrote is on record, for the undo.
        let record = h
            .app
            .model()
            .settings
            .integration
            .record(Item::LauncherEntry)
            .cloned()
            .unwrap_or_else(|| panic!("a record"));
        assert_eq!(record.state, ItemState::SetUp);
        assert_eq!(record.files, ["fake/launcher-entry"]);
        assert_eq!(state(&h, Item::DockFavourite), Some(ItemState::Declined));
        // The declined row was neither performed nor checked again.
        let calls = fake.calls();
        assert!(
            calls
                .iter()
                .any(|c| matches!(c, Call::Perform(Item::LauncherEntry, _)))
        );
        assert!(
            !calls
                .iter()
                .any(|c| matches!(c, Call::Perform(Item::DockFavourite, _)))
        );
        assert_eq!(
            calls
                .iter()
                .filter(|c| **c == Call::Check(Item::DockFavourite))
                .count(),
            1,
            "only the first check read the dock"
        );
        // At the next start nothing is asked: declined counts as done.
        check(&mut h, true);
        assert_eq!(asked(&h.take_events()), Some(false));
        let text = read_until(h.app.settings_path(), |text| text.contains("declined"));
        assert!(
            text.contains("[integration.items.dock-favourite]\nstate = \"declined\""),
            "{text}"
        );
    }

    #[test]
    fn add_sets_a_declined_item_up_and_reset_makes_the_dialog_ask_again() {
        let (mut h, fake) = scripted();
        h.settle();
        check(&mut h, true);
        h.send(Command::Integration(IntegrationCommand::Apply {
            scope: Scope::User,
            perform: Vec::new(),
            decline: vec![Item::LauncherEntry, Item::DockFavourite],
            replace: Vec::new(),
        }));
        h.settle();
        assert_eq!(status(&h, Item::LauncherEntry), Some(ItemStatus::Declined));
        // *Add* in Settings: performed, and set-up from then on.
        h.send(Command::Integration(IntegrationCommand::Add {
            item: Item::DockFavourite,
            replace: false,
        }));
        h.settle();
        assert_eq!(status(&h, Item::DockFavourite), Some(ItemStatus::SetUp));
        assert_eq!(state(&h, Item::DockFavourite), Some(ItemState::SetUp));
        assert_eq!(fake.actual(Item::DockFavourite), Some(Actual::SetUp));
        // *Reset choices*: the declined mark goes, the dialog asks again,
        // and a *Don't ask again* is taken back with it.
        h.send(Command::Settings(SettingsChange::IntegrationAsk(false)));
        h.send(Command::Integration(IntegrationCommand::Reset));
        h.settle();
        assert_eq!(status(&h, Item::LauncherEntry), Some(ItemStatus::Missing));
        assert!(h.app.model().settings.integration.ask);
        h.take_events();
        check(&mut h, true);
        assert_eq!(asked(&h.take_events()), Some(true));
    }

    #[test]
    fn an_item_that_was_set_up_and_is_gone_is_drifted_until_restored() {
        let (mut h, fake) = scripted();
        h.settle();
        check(&mut h, true);
        assert_eq!(status(&h, Item::RickCommand), Some(ItemStatus::SetUp));
        // Removed behind Rick's back.
        fake.set(Item::RickCommand, Check::missing());
        h.take_events();
        check(&mut h, true);
        assert_eq!(status(&h, Item::RickCommand), Some(ItemStatus::Drifted));
        assert_eq!(
            state(&h, Item::RickCommand),
            Some(ItemState::SetUp),
            "the record keeps what was wanted"
        );
        assert_eq!(
            asked(&h.take_events()),
            Some(true),
            "drift opens the dialog"
        );
        h.send(Command::Integration(IntegrationCommand::Restore {
            item: Item::RickCommand,
            replace: false,
        }));
        h.settle();
        assert_eq!(status(&h, Item::RickCommand), Some(ItemStatus::SetUp));
        // A declined item is not watched for drift, whatever happens to it.
        h.send(Command::Integration(IntegrationCommand::Apply {
            scope: Scope::User,
            perform: Vec::new(),
            decline: vec![Item::RickCommand],
            replace: Vec::new(),
        }));
        h.settle();
        fake.set(Item::RickCommand, Check::missing());
        check(&mut h, true);
        assert_eq!(status(&h, Item::RickCommand), Some(ItemStatus::Declined));
    }

    #[test]
    fn a_row_that_fails_is_reported_and_does_not_stop_the_others() {
        let (mut h, fake) = scripted();
        fake.set(
            Item::ShowInFolder,
            Check::unsupported(Reason::Program("dbus".to_owned())),
        );
        fake.fail(Item::DockFavourite);
        h.settle();
        check(&mut h, true);
        assert_eq!(
            status(&h, Item::ShowInFolder),
            Some(ItemStatus::Unsupported(Reason::Program("dbus".to_owned())))
        );
        assert_eq!(state(&h, Item::ShowInFolder), Some(ItemState::Unsupported));
        h.take_events();
        // Given out of order: performed in the order the rows are listed.
        h.send(Command::Integration(IntegrationCommand::Apply {
            scope: Scope::User,
            perform: vec![Item::DockFavourite, Item::LauncherEntry],
            decline: Vec::new(),
            replace: Vec::new(),
        }));
        h.settle();
        let events = h.take_events();
        assert!(
            events.iter().any(|e| matches!(
                e,
                Event::IntegrationApplied { failed, .. } if failed == &[Item::DockFavourite]
            )),
            "{events:?}"
        );
        // Best effort: a warning, naming the row.
        assert!(
            events.iter().any(|e| matches!(
                e,
                Event::Notify { level: Level::Warning, text } if text.contains("Dock favourite")
            )),
            "{events:?}"
        );
        assert_eq!(status(&h, Item::LauncherEntry), Some(ItemStatus::SetUp));
        assert_eq!(status(&h, Item::DockFavourite), Some(ItemStatus::Missing));
        let performed: Vec<Item> = fake
            .calls()
            .iter()
            .filter_map(|call| match call {
                Call::Perform(item, _) => Some(*item),
                _ => None,
            })
            .collect();
        assert_eq!(performed, [Item::LauncherEntry, Item::DockFavourite]);
    }

    #[test]
    fn a_file_of_another_origin_is_replaced_only_when_the_user_agreed() {
        let (mut h, fake) = scripted();
        fake.set(
            Item::LauncherEntry,
            Check {
                foreign: vec![std::path::PathBuf::from("/home/u/rick.desktop")],
                ..Check::missing()
            },
        );
        h.settle();
        check(&mut h, true);
        let row = h.app.model().integration.get(Item::LauncherEntry).cloned();
        assert_eq!(
            row.map(|row| row.check.foreign.len()),
            Some(1),
            "the row knows what is in its way"
        );
        h.take_events();
        h.send(Command::Integration(IntegrationCommand::Apply {
            scope: Scope::User,
            perform: vec![Item::LauncherEntry],
            decline: Vec::new(),
            replace: Vec::new(),
        }));
        h.settle();
        assert_eq!(status(&h, Item::LauncherEntry), Some(ItemStatus::Missing));
        assert!(h.take_events().iter().any(|e| matches!(
            e,
            Event::IntegrationApplied { failed, .. } if failed == &[Item::LauncherEntry]
        )));
        h.send(Command::Integration(IntegrationCommand::Apply {
            scope: Scope::User,
            perform: vec![Item::LauncherEntry],
            decline: Vec::new(),
            replace: vec![Item::LauncherEntry],
        }));
        h.settle();
        assert_eq!(status(&h, Item::LauncherEntry), Some(ItemStatus::SetUp));
        assert!(fake.calls().iter().any(|call| matches!(
            call,
            Call::Perform(Item::LauncherEntry, request) if request.replace
        )));
    }

    #[test]
    fn a_platform_without_items_checks_to_an_empty_list_and_never_asks() {
        // The fake's own integration lists nothing, as Windows and macOS do.
        let mut h = harness();
        h.settle();
        check(&mut h, true);
        assert_eq!(asked(&h.take_events()), Some(false));
        let integration = &h.app.model().integration;
        assert_eq!(integration.items, []);
        assert!(integration.checked_at.is_some());
        assert_ne!(
            integration.system, "",
            "the status line still names the system"
        );
    }

    /// A stand-in for the installer plugin: its `install-rick` command
    /// records what it was asked and starts a job that succeeds or fails.
    struct FakeInstaller {
        manifest: Manifest,
        asked: Arc<Mutex<Vec<Vec<String>>>>,
        fail: bool,
    }

    struct InstallRick {
        asked: Arc<Mutex<Vec<Vec<String>>>>,
        fail: bool,
    }

    impl ext::CliCommand for InstallRick {
        fn name(&self) -> &'static str {
            crate::INSTALL_RICK_COMMAND
        }

        fn run(
            &self,
            ctx: ext::CliCtx,
            jobs: &ext::JobRunner,
            _ui: &dyn UiHost,
        ) -> Result<ext::CliOutcome, Error> {
            self.asked
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(ctx.args);
            let fail = self.fail;
            let handle = jobs.spawn_named("Install Rick", move |_job| async move {
                if fail {
                    Err(Error::PermissionDenied)
                } else {
                    Ok(())
                }
            });
            Ok(ext::CliOutcome::Outcome(ext::ActionOutcome::Started(
                handle.id,
            )))
        }
    }

    impl Extension for FakeInstaller {
        fn manifest(&self) -> &Manifest {
            &self.manifest
        }

        fn register(&self, reg: &mut Registry) -> Result<(), Error> {
            reg.command(Arc::new(InstallRick {
                asked: Arc::clone(&self.asked),
                fail: self.fail,
            }));
            Ok(())
        }
    }

    const INSTALL_COMMANDS: &[ext::CommandSpec] = &[ext::CommandSpec {
        name: crate::INSTALL_RICK_COMMAND,
        help: "Install Rick",
    }];

    /// A harness with the stand-in installer loaded and an unpacked Rick:
    /// the install location and the launcher entry missing.
    fn unpacked(fail: bool) -> (Harness, FakeIntegration, Arc<Mutex<Vec<Vec<String>>>>) {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let installer: Arc<dyn Extension> = Arc::new(FakeInstaller {
            manifest: Manifest {
                id: ExtId("installer"),
                name: "Installer",
                description: "",
                version: Version::new(0, 1, 0),
                api_version: API_VERSION,
                requires: Capabilities::empty(),
                points: Points::empty(),
                platforms: Platforms::ALL,
                previews: &[],
                markers: &[],
                connections: &[],
                commands: INSTALL_COMMANDS,
            },
            asked: Arc::clone(&asked),
            fail,
        });
        let h = harness_with(vec![installer]);
        let fake = FakeIntegration::new("Fedora Linux 43 · GNOME", Some(Desktop::Gnome));
        fake.set(Item::InstallLocation, unpacked_check());
        fake.set(Item::LauncherEntry, Check::missing());
        h.fake.set_integration(Arc::new(fake.clone()));
        (h, fake, asked)
    }

    /// What the platform finds of a Rick unpacked into Downloads.
    fn unpacked_check() -> Check {
        Check {
            place: Some(std::path::PathBuf::from("/home/u/.local/opt/rick")),
            target: Some(std::path::PathBuf::from("/home/u/Downloads/rick-0.3.0")),
            ..Check::missing()
        }
    }

    fn applied(events: &[Event]) -> Option<(Vec<Item>, Option<std::path::PathBuf>)> {
        events.iter().find_map(|event| match event {
            Event::IntegrationApplied { failed, relaunch } => {
                Some((failed.clone(), relaunch.clone()))
            }
            _ => None,
        })
    }

    #[test]
    fn the_install_location_is_the_plugins_and_the_other_rows_act_on_the_installed_copy() {
        let (mut h, fake, asked) = unpacked(false);
        h.settle();
        check(&mut h, true);
        assert_eq!(status(&h, Item::InstallLocation), Some(ItemStatus::Missing));
        h.take_events();
        h.send(Command::Integration(IntegrationCommand::Apply {
            scope: Scope::User,
            perform: vec![Item::LauncherEntry, Item::InstallLocation],
            decline: Vec::new(),
            replace: Vec::new(),
        }));
        h.settle();
        // The plugin's command got the folder Rick runs from and Rick's
        // version; the platform was never asked to install.
        assert_eq!(
            *asked.lock().unwrap_or_else(PoisonError::into_inner),
            [vec![
                "/home/u/Downloads/rick-0.3.0".to_owned(),
                env!("CARGO_PKG_VERSION").to_owned()
            ]]
        );
        let installed = std::path::PathBuf::from("/home/u/.local/opt/rick/current/rick");
        let performed: Vec<(Item, Option<std::path::PathBuf>)> = fake
            .calls()
            .into_iter()
            .filter_map(|call| match call {
                Call::Perform(item, request) => Some((item, request.binary)),
                _ => None,
            })
            .collect();
        assert_eq!(
            performed,
            [(Item::LauncherEntry, Some(installed.clone()))],
            "after the install, on the installed copy"
        );
        // The apply ends naming the copy to continue from.
        let events = h.take_events();
        assert_eq!(applied(&events), Some((Vec::new(), Some(installed))));
        assert_eq!(state(&h, Item::InstallLocation), Some(ItemState::SetUp));
        assert_eq!(status(&h, Item::InstallLocation), Some(ItemStatus::SetUp));
        assert_eq!(status(&h, Item::LauncherEntry), Some(ItemStatus::SetUp));
        assert!(
            h.app
                .model()
                .settings
                .integration
                .receipt(Item::InstallLocation)
                .is_empty(),
            "the install's record is the installer's own"
        );
    }

    #[test]
    fn a_failed_install_leaves_the_rows_that_waited_for_it_alone() {
        let (mut h, fake, _asked) = unpacked(true);
        h.settle();
        check(&mut h, true);
        h.take_events();
        h.send(Command::Integration(IntegrationCommand::Apply {
            scope: Scope::User,
            perform: vec![Item::InstallLocation, Item::LauncherEntry],
            decline: Vec::new(),
            replace: Vec::new(),
        }));
        h.settle();
        let events = h.take_events();
        assert_eq!(
            applied(&events),
            Some((vec![Item::InstallLocation], None)),
            "{events:?}"
        );
        // The job's failure was shown under its own name.
        assert!(
            events.iter().any(|e| matches!(
                e,
                Event::Notify { level: Level::Error, text } if text.contains("Install Rick")
            )),
            "{events:?}"
        );
        assert!(
            !fake
                .calls()
                .iter()
                .any(|call| matches!(call, Call::Perform(..))),
            "nothing else was set up"
        );
        assert_eq!(status(&h, Item::InstallLocation), Some(ItemStatus::Missing));
        assert_eq!(h.app.model().integration.applying, None);
    }

    #[test]
    fn without_the_installer_plugin_the_install_row_is_unsupported_and_the_rest_works() {
        let mut h = harness();
        let fake = FakeIntegration::new("Fedora Linux 43 · GNOME", Some(Desktop::Gnome));
        fake.set(Item::InstallLocation, unpacked_check());
        fake.set(Item::LauncherEntry, Check::missing());
        h.fake.set_integration(Arc::new(fake.clone()));
        h.settle();
        check(&mut h, true);
        assert_eq!(
            status(&h, Item::InstallLocation),
            Some(ItemStatus::Unsupported(Reason::Plugin(
                "Installer".to_owned()
            )))
        );
        let asked: Vec<Item> = h
            .app
            .model()
            .integration
            .asked()
            .map(|row| row.item)
            .collect();
        assert_eq!(asked, [Item::LauncherEntry], "the row is not offered");
        h.take_events();
        h.send(Command::Integration(IntegrationCommand::Apply {
            scope: Scope::User,
            perform: vec![Item::LauncherEntry],
            decline: Vec::new(),
            replace: Vec::new(),
        }));
        h.settle();
        assert_eq!(applied(&h.take_events()), Some((Vec::new(), None)));
        // Against the folder Rick runs from: no other binary is named.
        assert!(fake.calls().iter().any(|call| matches!(
            call,
            Call::Perform(Item::LauncherEntry, request) if request.binary.is_none()
        )));
        assert_eq!(status(&h, Item::LauncherEntry), Some(ItemStatus::SetUp));
    }
}

/// The time slider: reading off the UI thread and
/// only while shown, merging sources, one history step per run, and the
/// nearest parent where a point lacks the folder.
mod timeline {
    use std::path::PathBuf;
    use std::sync::atomic::AtomicUsize;
    use std::time::SystemTime;

    use ext::{ColumnDef, DecorateCtx, Decoration, Decorator, PanelSection, TimePoint, Timeline};
    use model::TimelineState;

    use super::*;

    /// A timeline source over the harness's folder: its points are
    /// folders `.snaps/<label>` that mirror the tree, so the folder `rel`
    /// at point `label` is `.snaps/<label>/<rel>`.
    struct Source {
        id: &'static str,
        name: &'static str,
        root: PathBuf,
        /// Day and label of each point.
        points: Vec<(u64, &'static str)>,
        /// How long an answer takes.
        delay: Duration,
        /// How often `timeline` was asked.
        calls: AtomicUsize,
    }

    impl Source {
        fn new(
            id: &'static str,
            name: &'static str,
            root: &Path,
            points: &[(u64, &'static str)],
        ) -> Arc<Self> {
            Arc::new(Self {
                id,
                name,
                root: root.to_path_buf(),
                points: points.to_vec(),
                delay: Duration::ZERO,
                calls: AtomicUsize::new(0),
            })
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }

        /// The folder `loc` shows, relative to the root, wherever it is.
        fn rel(&self, loc: &Location) -> Option<PathBuf> {
            let path = platform::paths::to_path(&loc.path).ok()?;
            let rel = path.strip_prefix(&self.root).ok()?.to_path_buf();
            let mut parts = rel.components();
            if parts.next().is_some_and(|c| c.as_os_str() == ".snaps") {
                parts.next()?;
                return Some(parts.as_path().to_path_buf());
            }
            Some(rel)
        }
    }

    impl Decorator for Source {
        fn id(&self) -> ExtId {
            ExtId(self.id)
        }
        fn applies_to(&self, loc: &Location) -> bool {
            self.rel(loc).is_some()
        }
        fn columns(&self) -> &[ColumnDef] {
            &[]
        }
        fn decorate(&self, _: &[Entry], _: &DecorateCtx) -> Vec<(EntryId, Decoration)> {
            Vec::new()
        }
        fn panel(&self, _: &Entry) -> Option<PanelSection> {
            None
        }
        fn invalidates_on(&self, _: &Location) -> Vec<Location> {
            Vec::new()
        }
        fn invalidate(&self, _: &Location) {}
        fn offers_timelines(&self) -> bool {
            true
        }
        fn timeline(&self, loc: &Location) -> Option<Timeline> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(self.delay);
            let rel = self.rel(loc)?;
            let points: Vec<TimePoint> = self
                .points
                .iter()
                .map(|(day, label)| TimePoint {
                    time: SystemTime::UNIX_EPOCH + Duration::from_secs(day * 86_400),
                    label: (*label).into(),
                    source: self.name.into(),
                    location: local(&self.root.join(".snaps").join(label).join(&rel)),
                })
                .collect();
            // The folder asked about, as the ZFS source answers:
            // `join` of an empty path would end it with a separator.
            let live = if rel.as_os_str().is_empty() {
                local(&self.root)
            } else {
                local(&self.root.join(&rel))
            };
            (!points.is_empty()).then_some(Timeline { live, points })
        }
    }

    struct SourceExtension {
        manifest: Manifest,
        sources: Vec<Arc<Source>>,
    }

    impl Extension for SourceExtension {
        fn manifest(&self) -> &Manifest {
            &self.manifest
        }
        fn register(&self, reg: &mut Registry) -> Result<(), Error> {
            for source in &self.sources {
                reg.decorator(Arc::clone(source) as Arc<dyn Decorator>);
            }
            reg.action(Arc::new(TakeSnapshot));
            Ok(())
        }
    }

    /// An action as *Create snapshot* is: it changes the timeline in a
    /// job and answers "later", for a question that may follow.
    struct TakeSnapshot;

    impl ext::Action for TakeSnapshot {
        fn id(&self) -> ActionId {
            ActionId("sources.snapshot")
        }
        fn label(&self) -> String {
            "Take snapshot".to_owned()
        }
        fn group(&self) -> ext::ActionGroup {
            ext::ActionGroup::Core
        }
        fn default_shortcut(&self) -> Option<ext::Shortcut> {
            None
        }
        fn availability(&self, _: &ext::ActionCtx) -> Availability {
            Availability::Enabled
        }
        fn run(
            &self,
            _: ext::ActionCtx,
            jobs: &ext::JobRunner,
            _: &dyn UiHost,
        ) -> Result<ext::ActionOutcome, Error> {
            let (sender, later) = ext::FormLater::channel();
            jobs.spawn_named("Take snapshot", move |_job| async move {
                drop(sender);
                Ok(())
            });
            Ok(ext::ActionOutcome::Later(later))
        }
    }

    /// A harness whose folder has the points `mon` and `thu` (ZFS) and
    /// `tue` (Backup); `sub` is missing at `thu`.
    fn with_sources(delay: Duration) -> (Harness, Arc<Source>, Arc<Source>) {
        let mut h = harness();
        let root = h.root.path().to_path_buf();
        for label in ["mon", "tue", "thu"] {
            fs::create_dir_all(root.join(".snaps").join(label)).unwrap_or_else(|e| panic!("{e}"));
            fs::write(
                root.join(".snaps").join(label).join(format!("{label}.txt")),
                label,
            )
            .unwrap_or_else(|e| panic!("{e}"));
        }
        fs::create_dir(root.join(".snaps/mon/sub")).unwrap_or_else(|e| panic!("{e}"));
        let mut zfs = Source::new("zfs", "ZFS", &root, &[(1, "mon"), (4, "thu")]);
        if let Some(source) = Arc::get_mut(&mut zfs) {
            source.delay = delay;
        }
        let backup = Source::new("backup", "Backup", &root, &[(2, "tue")]);
        let extension: Arc<dyn Extension> = Arc::new(SourceExtension {
            manifest: Manifest {
                id: ExtId("sources"),
                name: "Timeline sources",
                description: "",
                version: Version::new(0, 1, 0),
                api_version: API_VERSION,
                requires: Capabilities::empty(),
                points: Points::DECORATOR.union(Points::ACTION),
                platforms: Platforms::ALL,
                previews: &[],
                markers: &[],
                connections: &[],
                commands: &[],
            },
            sources: vec![Arc::clone(&zfs), Arc::clone(&backup)],
        });
        drop(h.app);
        h.app = App::new(Config {
            platform: h.fake.clone(),
            ui: h.ui.clone(),
            extensions: vec![extension],
            config_dir: Some(h.fake.config_dir()),
            start: Some(local(&root)),
        });
        h.settle();
        (h, zfs, backup)
    }

    fn labels(h: &Harness, pane: model::PaneId) -> Vec<(String, String)> {
        h.app
            .model()
            .timeline_points(pane)
            .map(|(t, _)| {
                t.points
                    .iter()
                    .map(|p| (p.label.clone(), p.source.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn a_hidden_bar_asks_no_source() {
        let (mut h, zfs, backup) = with_sources(Duration::ZERO);
        let pane = h.app.model().focused;
        let sub = h.root.path().join("sub");
        h.send(Command::Navigate {
            pane,
            to: local(&sub),
        });
        h.settle();
        h.send(Command::Back(pane));
        h.settle();
        assert_eq!(zfs.calls() + backup.calls(), 0);
        assert_eq!(
            *h.app.model().timeline(pane),
            TimelineState::Unread,
            "a source applies"
        );
        assert!(!h.app.model().timeline_visible(pane));
    }

    #[test]
    fn two_sources_merge_and_each_location_is_read_once() {
        let (mut h, zfs, backup) = with_sources(Duration::ZERO);
        let pane = h.app.model().focused;
        h.take_events();
        h.send(Command::ToggleTimeline(pane));
        h.settle();
        assert!(
            h.take_events()
                .iter()
                .any(|e| matches!(e, Event::Timeline { .. }))
        );
        assert_eq!(
            labels(&h, pane),
            [
                ("mon".into(), "ZFS".into()),
                ("tue".into(), "Backup".into()),
                ("thu".into(), "ZFS".into())
            ]
        );
        assert!(h.app.model().timeline_visible(pane));
        assert_eq!((zfs.calls(), backup.calls()), (1, 1));

        let sub = h.root.path().join("sub");
        h.send(Command::Navigate {
            pane,
            to: local(&sub),
        });
        h.settle();
        h.send(Command::Back(pane));
        h.settle();
        assert_eq!(
            (zfs.calls(), backup.calls()),
            (2, 2),
            "the folder again is cached"
        );
        assert_eq!(labels(&h, pane).len(), 3);
    }

    #[test]
    fn a_slow_source_leaves_the_pane_listing() {
        let (mut h, zfs, _) = with_sources(Duration::from_millis(1500));
        let pane = h.app.model().focused;
        h.send(Command::ToggleTimeline(pane));
        assert_eq!(*h.app.model().timeline(pane), TimelineState::Reading);
        assert!(h.app.model().timeline_visible(pane), "the reading state");
        let sub = h.root.path().join("sub");
        let started = Instant::now();
        h.send(Command::Navigate {
            pane,
            to: local(&sub),
        });
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "handle did not wait"
        );
        h.settle();
        assert_eq!(h.names(), [] as [String; 0], "sub is listed (and empty)");
        assert_eq!(labels(&h, pane).len(), 3);
        assert!(zfs.calls() >= 1);
    }

    #[test]
    fn switching_off_keeps_no_bar_at_now() {
        let (mut h, _, _) = with_sources(Duration::ZERO);
        let pane = h.app.model().focused;
        h.send(Command::ToggleTimeline(pane));
        h.settle();
        h.send(Command::ToggleTimeline(pane));
        h.settle();
        assert!(!h.app.model().timeline_visible(pane));
        assert!(!h.app.model().timeline_shown(pane));
    }

    #[test]
    fn scrubbing_is_one_history_step_and_asks_nobody() {
        let (mut h, zfs, backup) = with_sources(Duration::ZERO);
        let pane = h.app.model().focused;
        let live = local(h.root.path());
        h.send(Command::ToggleTimeline(pane));
        h.settle();
        let asked = zfs.calls() + backup.calls();
        let points: Vec<Location> = h
            .app
            .model()
            .timeline_points(pane)
            .map(|(t, _)| t.points.iter().map(|p| p.location.clone()).collect())
            .unwrap_or_default();
        for (i, point) in points.iter().enumerate() {
            h.send(Command::TimeTravel {
                pane,
                to: point.clone(),
                first: i == 0,
            });
            h.settle();
        }
        assert_eq!(h.names(), ["thu.txt"]);
        assert!(h.app.model().in_the_past(pane));
        assert_eq!(
            h.app.model().timeline_points(pane).map(|(_, at)| at),
            Some(Some(2))
        );
        assert_eq!(
            zfs.calls() + backup.calls(),
            asked,
            "moving along asks nobody"
        );

        h.send(Command::Back(pane));
        h.settle();
        assert_eq!(h.tab(pane).location, live);
        assert!(
            h.tab(pane).history.back.is_empty(),
            "one step for the whole run"
        );
        assert!(!h.app.model().in_the_past(pane));
    }

    #[test]
    fn a_point_without_the_folder_shows_its_parent() {
        let (mut h, _, _) = with_sources(Duration::ZERO);
        let pane = h.app.model().focused;
        let sub = h.root.path().join("sub");
        h.send(Command::Navigate {
            pane,
            to: local(&sub),
        });
        h.send(Command::ToggleTimeline(pane));
        h.settle();
        let thu = local(&h.root.path().join(".snaps/thu/sub"));
        h.take_events();
        h.send(Command::TimeTravel {
            pane,
            to: thu,
            first: true,
        });
        h.settle();
        assert_eq!(
            h.tab(pane).location,
            local(&h.root.path().join(".snaps/thu"))
        );
        assert_eq!(h.names(), ["thu.txt"]);
        let notices = h
            .take_events()
            .into_iter()
            .filter(|e| matches!(e, Event::Notify { .. }))
            .count();
        assert_eq!(notices, 1, "said once");
        h.send(Command::Back(pane));
        h.settle();
        assert_eq!(h.tab(pane).location, local(&sub));
    }

    #[test]
    fn an_empty_answer_shows_no_bar() {
        let mut h = harness();
        let root = h.root.path().to_path_buf();
        let none = Source::new("none", "None", &root, &[]);
        let extension: Arc<dyn Extension> = Arc::new(SourceExtension {
            manifest: Manifest {
                id: ExtId("sources"),
                name: "Timeline sources",
                description: "",
                version: Version::new(0, 1, 0),
                api_version: API_VERSION,
                requires: Capabilities::empty(),
                points: Points::DECORATOR.union(Points::ACTION),
                platforms: Platforms::ALL,
                previews: &[],
                markers: &[],
                connections: &[],
                commands: &[],
            },
            sources: vec![Arc::clone(&none)],
        });
        drop(h.app);
        h.app = App::new(Config {
            platform: h.fake.clone(),
            ui: h.ui.clone(),
            extensions: vec![extension],
            config_dir: Some(h.fake.config_dir()),
            start: Some(local(&root)),
        });
        h.settle();
        let pane = h.app.model().focused;
        h.send(Command::ToggleTimeline(pane));
        h.settle();
        assert_eq!(*h.app.model().timeline(pane), TimelineState::Empty);
        assert!(!h.app.model().timeline_visible(pane), "never an empty bar");
    }

    /// A refresh reads the timeline anew, so a snapshot taken meanwhile
    /// shows on the bar at once, not only after the 30 seconds.
    #[test]
    fn a_refresh_reads_the_timeline_again() {
        let (mut h, zfs, backup) = with_sources(Duration::ZERO);
        let pane = h.app.model().focused;
        h.send(Command::ToggleTimeline(pane));
        h.settle();
        assert_eq!((zfs.calls(), backup.calls()), (1, 1));
        h.send(Command::Refresh(pane));
        h.settle();
        assert_eq!((zfs.calls(), backup.calls()), (2, 2));
        assert_eq!(labels(&h, pane).len(), 3, "the bar keeps its points");
    }

    /// A snapshot the user takes in Rick shows on the bar when its job
    /// ends, without a refresh: the action's job is the pane's.
    #[test]
    fn an_actions_job_reads_the_timeline_again_when_it_ends() {
        let (mut h, zfs, _) = with_sources(Duration::ZERO);
        let pane = h.app.model().focused;
        h.send(Command::ToggleTimeline(pane));
        h.settle();
        assert_eq!(zfs.calls(), 1);
        h.run(pane, "sources.snapshot");
        assert!(
            wait_until(&mut h, Duration::from_secs(5), |_| zfs.calls() == 2),
            "asked {} times",
            zfs.calls()
        );
    }

    #[test]
    fn no_source_no_timeline() {
        let mut h = harness();
        h.settle();
        let pane = h.app.model().focused;
        h.send(Command::ToggleTimeline(pane));
        h.settle();
        assert_eq!(*h.app.model().timeline(pane), TimelineState::None);
        assert!(!h.app.model().timeline_visible(pane));
    }
}

/// The plugin registry (specs/plugin-distribution.md §6), against a
/// registry of the test's own: a server on this machine and a key made
/// here, named by `[registry] url` and `public_key`.
mod registry {
    use std::time::SystemTime;

    use plugins_registry::fixtures::{INDEX_PATH, Server, Signer};
    use serde_json::json;

    use super::*;
    use crate::{CACHE_DIR, RegistryCommand, RegistryProblem};

    /// The Rick the tests ask as.
    const RICK: &str = "0.6.0";

    /// An index with Git for [`RICK`] on this platform, in version
    /// `version`.
    fn index(schema: u32, version: &str) -> serde_json::Value {
        json!({
            "schema": schema,
            "generated": "2026-11-19T02:00:00Z",
            "apps": { "rick": { "plugins": [{
                "id": "git",
                "name": "Git",
                "description": "Status badges and the Git actions of the context menu.",
                "publisher": "Open-Breach",
                "meta": { "points": ["decorator", "action"], "triggers": { "markers": [".git"] } },
                "builds": [{
                    "version": version,
                    "app_version": RICK,
                    "api_version": API_VERSION,
                    "platform": plugins_registry::platform(),
                    "url": "https://example.invalid/git.rkp",
                    "size": 1024,
                    "sha256": "aa".repeat(32),
                }],
            }] } },
        })
    }

    /// The `[registry]` table that names `server` and `signer`, with the
    /// switch as `check` has it.
    fn table(server: &Server, signer: &Signer, check: bool) -> String {
        format!(
            "[registry]\ncheck = {check}\nurl = \"{}\"\npublic_key = \"{}\"\n",
            server.index_url(),
            signer.public_key()
        )
    }

    /// A Rick of version [`RICK`] started on `fake`'s config directory,
    /// whose `rick.toml` is `settings`.
    fn start(fake: &Arc<Fake>, settings: &str) -> Harness {
        let config = fake.config_dir();
        fs::create_dir_all(&config).unwrap_or_else(|e| panic!("{e}"));
        fs::write(config.join(SETTINGS_FILE), settings).unwrap_or_else(|e| panic!("{e}"));
        let root = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let ui = Arc::new(Scripted::default());
        let mut app = App::new(Config {
            platform: fake.clone(),
            ui: ui.clone(),
            extensions: Vec::new(),
            config_dir: Some(config),
            start: Some(local(root.path())),
        });
        app.set_rick_version(RICK);
        let mut harness = Harness {
            root,
            fake: fake.clone(),
            ui,
            app,
            events: Vec::new(),
        };
        harness.settle();
        harness
    }

    fn fake() -> Arc<Fake> {
        Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")))
    }

    impl Harness {
        /// Sends the registry `command` and waits for what comes of it.
        fn registry(&mut self, command: RegistryCommand) {
            self.send(Command::Registry(command));
            self.settle();
        }

        /// A start as time brings it: the first frame, which reads the
        /// cache, and then the background check, which the timer sends
        /// a minute later.
        fn begin(&mut self) {
            self.registry(RegistryCommand::Start);
            self.registry(RegistryCommand::Daily);
        }

        /// The version of Git in the index the application holds.
        fn git(&self) -> Option<String> {
            let index = self.app.plugin_registry().index.as_ref()?;
            Some(index.plugin("git")?.builds.first()?.version.clone())
        }
    }

    /// Runs the loop until `server` has been asked `count` times and says
    /// when that was. It was never asked more often on the way, and half
    /// a minute is as long as this waits.
    fn until_asked(h: &mut Harness, server: &Server, count: usize) -> Instant {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            h.settle();
            let seen = server.requests().len();
            assert!(seen <= count, "asked {seen} times, {count} were expected");
            if seen == count {
                return Instant::now();
            }
            assert!(Instant::now() < deadline, "asked {seen} times only");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Runs the loop until `until`, in which `server` is asked nothing
    /// new. What is seen after `until` proves nothing and is not held
    /// against the application: a machine that is slow enough to be past
    /// it already checks nothing here.
    fn left_alone(h: &mut Harness, server: &Server, until: Instant) {
        let asked = server.requests().len();
        loop {
            h.settle();
            // The count first: what it holds happened before the clock
            // is read.
            let seen = server.requests().len();
            if Instant::now() >= until {
                break;
            }
            assert_eq!(seen, asked, "asked in the meantime");
        }
    }

    /// What the clock of a file and the clock of a test may differ by:
    /// a file's time is the system's coarse one, a tick or so behind.
    const SLACK: Duration = Duration::from_millis(100);

    #[test]
    fn the_start_asks_nobody_and_the_background_check_comes_a_while_after_it() {
        let (server, signer, fake) = (Server::start(), Signer::new(1), fake());
        server.publish(&signer, &index(1, "0.6.0"));
        let mut h = start(&fake, &table(&server, &signer, true));
        // Rick's own schedule is a minute and a day; the test's is the
        // same in small.
        assert_eq!(crate::REGISTRY_AFTER_START, Duration::from_mins(1));
        let after_start = Duration::from_millis(600);
        h.app
            .set_registry_schedule(after_start, Duration::from_hours(24));

        // The first frame: the cache is read, which is empty, and nobody
        // is asked, although there is no list at all.
        let started = Instant::now();
        h.registry(RegistryCommand::Start);
        assert_eq!(h.git(), None);
        let asked = server.requests();
        if started.elapsed() < after_start {
            assert_eq!(asked, [], "the start asks nobody");
        }

        // The check comes by itself, and not before its time.
        let asked = until_asked(&mut h, &server, 2);
        assert!(
            asked.duration_since(started) >= after_start,
            "asked after {:?}",
            asked.duration_since(started)
        );
        h.settle();
        assert_eq!(h.git().as_deref(), Some("0.6.0"));
        assert_eq!(h.app.plugin_registry().problem, None);
        // And then the list is good for a day.
        left_alone(&mut h, &server, Instant::now() + 3 * after_start);
    }

    #[test]
    fn a_rick_that_stays_open_fetches_the_list_again_when_it_is_a_day_old() {
        let (server, signer, fake) = (Server::start(), Signer::new(1), fake());
        server.publish(&signer, &index(1, "0.6.0"));
        let mut h = start(&fake, &table(&server, &signer, true));
        // A day of a second and a half.
        let (after_start, day) = (Duration::from_millis(100), Duration::from_millis(1500));
        h.app.set_registry_schedule(after_start, day);
        let started = Instant::now();
        h.registry(RegistryCommand::Start);
        let first = until_asked(&mut h, &server, 2);
        assert_eq!(h.git().as_deref(), Some("0.6.0"));

        // A new list is published meanwhile. Rick does not look before
        // the one it has is a day old, and then it does, by itself.
        server.publish(&signer, &index(1, "0.6.1"));
        let second = until_asked(&mut h, &server, 4);
        // The list arrived after the start, so a day after the start is
        // the earliest.
        assert!(
            second.duration_since(started) + SLACK >= day,
            "fetched again after {:?}",
            second.duration_since(started)
        );
        assert!(second > first);
        h.settle();
        assert_eq!(h.git().as_deref(), Some("0.6.1"));

        // And again a day after that.
        server.publish(&signer, &index(1, "0.6.2"));
        let third = until_asked(&mut h, &server, 6);
        assert!(
            third.duration_since(started) + 2 * SLACK >= 2 * day,
            "fetched a third time after {:?}",
            third.duration_since(started)
        );
        h.settle();
        assert_eq!(h.git().as_deref(), Some("0.6.2"));
    }

    #[test]
    fn a_list_younger_than_a_day_is_left_until_it_is_one_and_one_the_user_fetched_counts() {
        let (server, signer, fake) = (Server::start(), Signer::new(1), fake());
        server.publish(&signer, &index(1, "0.6.0"));
        let settings = table(&server, &signer, true);
        let (after_start, day) = (Duration::from_millis(100), Duration::from_millis(3000));

        // A Rick that fetched the list, and was closed.
        let fetched = Instant::now();
        let mut before = start(&fake, &settings);
        before.begin();
        assert_eq!(server.requests().len(), 2);
        drop(before);

        // The next one finds a list that is younger than a day: its
        // check, a moment after the start, asks nobody. It waits for the
        // list to be a day old, not for a day of its own.
        let mut h = start(&fake, &settings);
        h.app.set_registry_schedule(after_start, day);
        h.registry(RegistryCommand::Start);
        assert_eq!(h.git().as_deref(), Some("0.6.0"));
        // Well past its own first look, and well before the list's day.
        left_alone(&mut h, &server, fetched + day / 2);
        let again = until_asked(&mut h, &server, 4);
        assert!(
            again.duration_since(fetched) + SLACK >= day,
            "fetched again after {:?}",
            again.duration_since(fetched)
        );

        // The user asks for the list: the day starts over from there.
        let asked = Instant::now();
        h.registry(RegistryCommand::CheckNow);
        assert_eq!(server.requests().len(), 6);
        let later = until_asked(&mut h, &server, 8);
        assert!(
            later.duration_since(asked) + SLACK >= day,
            "fetched again after {:?}",
            later.duration_since(asked)
        );
    }

    #[test]
    fn a_background_check_that_fails_is_tried_again_a_day_later_and_not_before() {
        let (server, signer, fake) = (Server::start(), Signer::new(1), fake());
        // Nothing is published: the index is not there.
        let mut h = start(&fake, &table(&server, &signer, true));
        let (after_start, day) = (Duration::from_millis(100), Duration::from_millis(1500));
        h.app.set_registry_schedule(after_start, day);
        let started = Instant::now();
        h.registry(RegistryCommand::Start);
        until_asked(&mut h, &server, 1);
        h.settle();
        assert_eq!(
            h.app.plugin_registry().problem,
            Some(RegistryProblem::Offline)
        );

        // No second try for a day, however soon the list would be there.
        // The first try was not before its time, so a day after that is
        // the earliest for the second.
        server.publish(&signer, &index(1, "0.6.0"));
        left_alone(&mut h, &server, started + day);
        let second = until_asked(&mut h, &server, 3);
        assert!(
            second.duration_since(started) >= after_start + day,
            "tried again after {:?}",
            second.duration_since(started)
        );
        h.settle();
        assert_eq!(h.git().as_deref(), Some("0.6.0"));
        assert_eq!(h.app.plugin_registry().problem, None);
    }

    #[test]
    fn with_the_switch_off_the_background_check_asks_nobody() {
        let (server, signer, fake) = (Server::start(), Signer::new(1), fake());
        server.publish(&signer, &index(1, "0.6.0"));
        let mut h = start(&fake, &table(&server, &signer, false));
        let (after_start, day) = (Duration::from_millis(50), Duration::from_millis(400));
        h.app.set_registry_schedule(after_start, day);
        h.registry(RegistryCommand::Start);
        // A day and more: not at its start, and not when it comes round.
        left_alone(&mut h, &server, Instant::now() + 3 * day);
        assert_eq!(server.requests(), []);
        // Check now still asks, whatever the switch says.
        h.registry(RegistryCommand::CheckNow);
        assert_eq!(server.requests().len(), 2);
    }

    /// [`index`] with `latest` as the newest release of Rick.
    fn released(latest: &serde_json::Value) -> serde_json::Value {
        let mut index = index(1, "0.6.0");
        index["apps"]["rick"]["latest"] = latest.clone();
        index
    }

    /// The releases of Rick `events` say are available, in their order.
    fn told(events: &[Event]) -> Vec<String> {
        events
            .iter()
            .filter_map(|event| match event {
                Event::RickUpdate { version } => Some(version.clone()),
                _ => None,
            })
            .collect()
    }

    const PAGE: &str = "https://github.com/Open-Breach/rick/releases/tag/v0.7.0";

    #[test]
    fn a_newer_rick_in_the_list_is_said_once_for_a_release_and_remembered() {
        let (server, signer, fake) = (Server::start(), Signer::new(1), fake());
        server.publish(
            &signer,
            &released(&json!({ "version": "0.7.0", "url": PAGE })),
        );
        let settings = table(&server, &signer, true);
        let mut h = start(&fake, &settings);
        assert_eq!(h.app.rick_update(), None, "nothing is known before a list");

        // The list arrives with the background check: it names a newer
        // Rick, and that is said. No request was made for it.
        h.begin();
        assert_eq!(told(&h.take_events()), ["0.7.0"]);
        assert_eq!(server.requests().len(), 2, "the two files of the list");
        let update = crate::RickUpdate {
            version: "0.7.0".to_owned(),
            page: Some(PAGE.to_owned()),
            packaged: false,
        };
        assert_eq!(h.app.rick_update(), Some(update.clone()));

        // Once: the list fetched again says nothing new.
        h.registry(RegistryCommand::CheckNow);
        h.registry(RegistryCommand::Opened);
        assert_eq!(told(&h.take_events()), [] as [&str; 0]);
        assert_eq!(h.app.rick_update(), Some(update.clone()));
        drop(h);

        // And remembered: the next start reads the same list from the
        // cache and knows of the release, without a word.
        let mut again = start(&fake, &settings);
        again.begin();
        assert_eq!(told(&again.take_events()), [] as [&str; 0]);
        assert_eq!(again.app.rick_update(), Some(update));
        let remembered = fake
            .config_dir()
            .join(CACHE_DIR)
            .join(plugins_registry::CACHE_DIR)
            .join("release-announced");
        assert_eq!(
            fs::read_to_string(&remembered).ok().as_deref(),
            Some("0.7.0")
        );

        // A release after that one is new again.
        let next = "https://github.com/Open-Breach/rick/releases/tag/v0.7.1";
        server.publish(
            &signer,
            &released(&json!({ "version": "0.7.1", "url": next })),
        );
        again.registry(RegistryCommand::CheckNow);
        assert_eq!(told(&again.take_events()), ["0.7.1"]);
        again.registry(RegistryCommand::CheckNow);
        assert_eq!(told(&again.take_events()), [] as [&str; 0]);
        again.settle();
        assert_eq!(
            fs::read_to_string(&remembered).ok().as_deref(),
            Some("0.7.1")
        );
        drop(again);

        // Until Rick is that release or a newer one: then there is no
        // update, and nothing to say.
        for rick in ["0.7.1", "0.8.0"] {
            let mut updated = start(&fake, &settings);
            updated.app.set_rick_version(rick);
            updated.begin();
            assert_eq!(updated.app.rick_update(), None, "{rick}");
            assert_eq!(told(&updated.take_events()), [] as [&str; 0], "{rick}");
        }
    }

    #[test]
    fn the_update_is_said_whenever_the_list_first_names_it() {
        let (server, signer, fake) = (Server::start(), Signer::new(1), fake());
        server.publish(
            &signer,
            &released(&json!({ "version": "0.7.0", "url": PAGE })),
        );
        let mut h = start(&fake, &table(&server, &signer, true));
        // The first frame: nothing is cached, and nobody is asked.
        h.registry(RegistryCommand::Start);
        assert_eq!(told(&h.take_events()), [] as [&str; 0]);
        // The Available list is opened before the background check came.
        h.registry(RegistryCommand::Opened);
        assert_eq!(told(&h.take_events()), ["0.7.0"]);
        h.registry(RegistryCommand::Daily);
        assert_eq!(told(&h.take_events()), [] as [&str; 0]);
    }

    #[test]
    fn no_update_is_said_with_the_switch_off_for_a_pre_release_or_without_a_newer_one() {
        let (server, signer) = (Server::start(), Signer::new(1));
        let newer = json!({ "version": "0.7.0", "url": PAGE });
        server.publish(&signer, &released(&newer));

        // The switch off: the list Check now fetched names the release,
        // and Rick does not look at that.
        let mut off = start(&fake(), &table(&server, &signer, false));
        off.begin();
        off.registry(RegistryCommand::CheckNow);
        assert_eq!(off.git().as_deref(), Some("0.6.0"), "the list is there");
        assert_eq!(off.app.rick_update(), None);
        assert_eq!(told(&off.take_events()), [] as [&str; 0]);

        // A pre-release Rick asks for nothing and is told of nothing.
        let mut dev = start(&fake(), &table(&server, &signer, true));
        dev.app.set_rick_version("0.7.0-dev");
        dev.begin();
        dev.registry(RegistryCommand::CheckNow);
        assert_eq!(dev.app.rick_update(), None);
        assert_eq!(told(&dev.take_events()), [] as [&str; 0]);

        // The same release, an older one, a pre-release, no entry and an
        // entry that is none: nothing is newer.
        for latest in [
            json!({ "version": "0.6.0", "url": PAGE }),
            json!({ "version": "0.5.0", "url": PAGE }),
            json!({ "version": "0.7.0-rc1", "url": PAGE }),
            json!(null),
            json!("0.7.0"),
        ] {
            server.publish(&signer, &released(&latest));
            let mut h = start(&fake(), &table(&server, &signer, true));
            h.begin();
            assert_eq!(h.git().as_deref(), Some("0.6.0"), "{latest}: the list");
            assert_eq!(h.app.rick_update(), None, "{latest}");
            assert_eq!(told(&h.take_events()), [] as [&str; 0], "{latest}");
        }

        // A release whose address is not one to open is a release all the
        // same, without a page.
        for url in ["http://example.org/v0.7.0", "file:///tmp/v0.7.0", ""] {
            server.publish(
                &signer,
                &released(&json!({ "version": "0.7.0", "url": url })),
            );
            let mut h = start(&fake(), &table(&server, &signer, true));
            h.begin();
            let update = h.app.rick_update();
            assert_eq!(update.as_ref().map(|u| u.version.as_str()), Some("0.7.0"));
            assert_eq!(update.and_then(|u| u.page), None, "{url}");
            assert_eq!(told(&h.take_events()), ["0.7.0"], "{url}");
        }
    }

    #[test]
    fn a_packaged_rick_is_updated_by_its_package_manager_and_has_no_page() {
        use crate::plugins::Packaging;
        let (server, signer) = (Server::start(), Signer::new(1));
        server.publish(
            &signer,
            &released(&json!({ "version": "0.7.0", "url": PAGE })),
        );
        for packaging in [Packaging::Deb, Packaging::Rpm] {
            let mut h = start(&fake(), &table(&server, &signer, true));
            h.app.set_packaging(Some(packaging));
            h.begin();
            assert_eq!(told(&h.take_events()), ["0.7.0"], "{packaging:?}");
            assert_eq!(
                h.app.rick_update(),
                Some(crate::RickUpdate {
                    version: "0.7.0".to_owned(),
                    page: None,
                    packaged: true,
                }),
                "{packaging:?}"
            );
        }
    }

    #[test]
    fn the_start_fetches_the_list_and_the_next_start_reads_it_from_the_cache() {
        let (server, signer, fake) = (Server::start(), Signer::new(1), fake());
        server.publish(&signer, &index(1, "0.6.0"));
        let settings = table(&server, &signer, true);

        let mut h = start(&fake, &settings);
        let registry = h.app.plugin_registry().clone();
        assert!(registry.set_up && !registry.checking);
        assert_eq!(registry.rick, RICK);
        assert_eq!(registry.custom, Some(server.index_url()));
        assert_eq!(
            registry.index, None,
            "nothing is read before the first frame"
        );
        assert_eq!(server.requests(), [], "and nothing is asked");

        h.begin();
        let requests = server.requests();
        let paths: Vec<&str> = requests.iter().map(|r| r.path.as_str()).collect();
        assert_eq!(paths, [INDEX_PATH, "registry/index.json.minisig"]);
        assert_eq!(requests[0].header("user-agent"), Some("rick/0.6.0"));
        let registry = h.app.plugin_registry();
        assert_eq!(h.git().as_deref(), Some("0.6.0"));
        assert!(registry.fetched.is_some() && !registry.checking);
        assert_eq!(registry.problem, None);
        // The cache is in the config directory of a Rick that was given
        // one of its own.
        let cached = fake
            .config_dir()
            .join(CACHE_DIR)
            .join(plugins_registry::CACHE_DIR);
        assert!(cached.join(plugins_registry::INDEX_FILE).is_file());
        assert!(cached.join(plugins_registry::SIGNATURE_FILE).is_file());

        // The next start, the same day: the list is there and nobody is
        // asked.
        let mut again = start(&fake, &settings);
        again.begin();
        assert_eq!(again.git().as_deref(), Some("0.6.0"));
        assert_eq!(again.app.plugin_registry().fetched, registry.fetched);
        assert_eq!(server.requests().len(), 2, "at most once a day");

        // A day later the start asks again.
        let file = fs::File::options()
            .write(true)
            .open(cached.join(plugins_registry::INDEX_FILE))
            .unwrap_or_else(|e| panic!("{e}"));
        file.set_modified(SystemTime::now() - Duration::from_hours(25))
            .unwrap_or_else(|e| panic!("{e}"));
        drop(file);
        server.publish(&signer, &index(1, "0.6.1"));
        let mut later = start(&fake, &settings);
        later.begin();
        assert_eq!(later.git().as_deref(), Some("0.6.1"));
        assert_eq!(server.requests().len(), 4);
    }

    #[test]
    fn with_the_switch_off_nothing_is_fetched_by_itself() {
        let (server, signer, fake) = (Server::start(), Signer::new(1), fake());
        server.publish(&signer, &index(1, "0.6.0"));
        let mut h = start(&fake, &table(&server, &signer, false));

        // The start, the view opened, a selection: not one request.
        h.begin();
        h.registry(RegistryCommand::Opened);
        let pane = h.app.model().focused;
        h.send(Command::Select {
            pane,
            change: SelectionChange::All,
        });
        h.settle();
        let registry = h.app.plugin_registry();
        assert!(registry.set_up && !registry.checking);
        assert_eq!((&registry.index, registry.problem), (&None, None));
        assert_eq!(server.requests(), [], "nothing is fetched by itself");

        // *Check now* is the user asking.
        h.registry(RegistryCommand::CheckNow);
        assert_eq!(server.requests().len(), 2);
        assert_eq!(h.git().as_deref(), Some("0.6.0"));

        // What was cached is shown by the next start, which asks nobody.
        let mut again = start(&fake, &table(&server, &signer, false));
        again.begin();
        assert_eq!(again.git().as_deref(), Some("0.6.0"));
        assert_eq!(server.requests().len(), 2);
    }

    #[test]
    fn opening_the_view_fetches_while_the_switch_is_on() {
        let (server, signer, fake) = (Server::start(), Signer::new(1), fake());
        server.publish(&signer, &index(1, "0.6.0"));
        let mut h = start(&fake, &table(&server, &signer, true));
        h.begin();
        assert_eq!(server.requests().len(), 2);
        server.publish(&signer, &index(1, "0.6.1"));
        h.registry(RegistryCommand::Opened);
        assert_eq!(server.requests().len(), 4);
        assert_eq!(h.git().as_deref(), Some("0.6.1"));
    }

    #[test]
    fn an_index_that_does_not_verify_is_not_used_and_the_last_good_one_is() {
        let (server, signer, fake) = (Server::start(), Signer::new(1), fake());
        server.publish(&signer, &index(1, "0.6.0"));
        let settings = table(&server, &signer, true);
        let mut h = start(&fake, &settings);
        h.begin();
        let fetched = h.app.plugin_registry().fetched;

        // The registry now serves an index somebody else signed.
        server.publish(&Signer::new(2), &index(1, "6.6.6"));
        h.registry(RegistryCommand::CheckNow);
        let registry = h.app.plugin_registry();
        assert_eq!(registry.problem, Some(RegistryProblem::Unverified));
        assert_eq!(h.git().as_deref(), Some("0.6.0"), "the last good one");
        assert_eq!(registry.fetched, fetched);
        assert!(!registry.checking);

        // It was never cached: the next start has the good one, and a
        // fetch of a good index takes the problem away.
        let mut again = start(&fake, &settings);
        again.begin();
        assert_eq!(again.git().as_deref(), Some("0.6.0"));
        assert_eq!(again.app.plugin_registry().problem, None);
        again.registry(RegistryCommand::CheckNow);
        assert_eq!(
            again.app.plugin_registry().problem,
            Some(RegistryProblem::Unverified)
        );
        server.publish(&signer, &index(1, "0.6.1"));
        again.registry(RegistryCommand::CheckNow);
        assert_eq!(again.app.plugin_registry().problem, None);
        assert_eq!(again.git().as_deref(), Some("0.6.1"));
    }

    #[test]
    fn a_cached_index_that_does_not_verify_is_fetched_again_at_the_start() {
        let (server, signer, fake) = (Server::start(), Signer::new(1), fake());
        server.publish(&signer, &index(1, "0.6.0"));
        let settings = table(&server, &signer, true);
        let mut h = start(&fake, &settings);
        h.begin();
        assert_eq!(server.requests().len(), 2);

        // Somebody changes the cached file. It is fresh, and no reason to
        // wait a day: the start drops it and asks.
        let cached = fake
            .config_dir()
            .join(CACHE_DIR)
            .join(plugins_registry::CACHE_DIR)
            .join(plugins_registry::INDEX_FILE);
        fs::write(&cached, index(1, "6.6.6").to_string()).unwrap_or_else(|e| panic!("{e}"));
        let mut again = start(&fake, &settings);
        again.begin();
        assert_eq!(server.requests().len(), 4);
        assert_eq!(again.git().as_deref(), Some("0.6.0"));
        assert_eq!(again.app.plugin_registry().problem, None);

        // With the switch off it is dropped all the same, and said.
        fs::write(&cached, index(1, "6.6.6").to_string()).unwrap_or_else(|e| panic!("{e}"));
        let mut off = start(&fake, &table(&server, &signer, false));
        off.begin();
        assert_eq!(server.requests().len(), 4);
        assert_eq!(off.git(), None);
        assert_eq!(
            off.app.plugin_registry().problem,
            Some(RegistryProblem::Unverified)
        );
    }

    #[test]
    fn a_newer_schema_and_a_lost_network_say_so_and_change_nothing() {
        let (server, signer, fake) = (Server::start(), Signer::new(1), fake());
        server.publish(&signer, &index(1, "0.6.0"));
        let mut h = start(&fake, &table(&server, &signer, true));
        h.begin();

        server.publish(&signer, &index(2, "0.9.0"));
        h.registry(RegistryCommand::CheckNow);
        assert_eq!(
            h.app.plugin_registry().problem,
            Some(RegistryProblem::NewerRick)
        );
        assert_eq!(h.git().as_deref(), Some("0.6.0"));

        runtime::block_on(server.stop());
        h.registry(RegistryCommand::CheckNow);
        assert_eq!(
            h.app.plugin_registry().problem,
            Some(RegistryProblem::Offline)
        );
        assert_eq!(h.git().as_deref(), Some("0.6.0"));
        assert!(!h.app.plugin_registry().checking);
    }

    #[test]
    fn without_a_key_and_as_a_pre_release_nothing_is_asked() {
        let (server, signer, fake) = (Server::start(), Signer::new(1), fake());
        server.publish(&signer, &index(1, "0.6.0"));
        let occasions = [
            RegistryCommand::Start,
            RegistryCommand::Opened,
            RegistryCommand::CheckNow,
        ];

        // A key that is none: the registry is off.
        let unkeyed = format!(
            "[registry]\nurl = \"{}\"\npublic_key = \"not a key\"\n",
            server.index_url()
        );
        let mut h = start(&fake, &unkeyed);
        assert!(!h.app.plugin_registry().set_up);
        for occasion in occasions {
            h.registry(occasion);
        }
        assert_eq!(h.app.plugin_registry().index, None);

        // An application nobody turned the build's registry on for has
        // none without a key of its own, whatever key the build carries:
        // that is every test, and none of them asks the real registry.
        let addressed = format!("[registry]\nurl = \"{}\"\n", server.index_url());
        let mut h = start(&self::fake(), &addressed);
        assert!(!h.app.plugin_registry().set_up);
        for occasion in occasions {
            h.registry(occasion);
        }
        assert!(!harness().app.plugin_registry().set_up);

        // The binary turns it on. While the committed key file is empty,
        // the registry is off all the same, however the address is set.
        let built_with_a_key = plugins_registry::Source::new(None, None).key.is_some();
        if !built_with_a_key {
            let mut h = start(&self::fake(), &addressed);
            h.app.use_build_registry();
            assert!(!h.app.plugin_registry().set_up);
            for occasion in occasions {
                h.registry(occasion);
            }
        }

        // A pre-release fits nothing the registry has, and does not ask.
        let mut dev = start(&self::fake(), &table(&server, &signer, true));
        dev.app.set_rick_version("0.6.0-dev");
        for occasion in occasions {
            dev.registry(occasion);
        }
        assert!(dev.app.plugin_registry().set_up);
        assert_eq!(dev.app.plugin_registry().index, None);

        assert_eq!(server.requests(), [], "not one request");
    }
}

/// *Install* from the registry and the record of what was installed
/// (specs/plugin-distribution.md §7), against a registry of the test's
/// own.
mod install {
    use std::io::Write as _;

    use model::JobState;
    use plugins_registry::fixtures::{Server, Signer};
    use serde_json::json;

    use super::*;
    use crate::plugins::{Origin, RECORD_FILE, Record};
    use crate::{Offering, PluginStatus, RegistryCommand};

    /// The Rick the tests ask as.
    const RICK: &str = "0.6.0";
    /// Where the test registry serves the package.
    const PACKAGE: &str = "plugins/rick/0.6.0/hello.rkp";

    /// The bytes of a plugin package: a zip archive of the folder `hello/`
    /// with a manifest that names `version`.
    fn package(version: &str) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        let manifest = format!(
            "id = \"hello\"\nname = \"Hello\"\ndescription = \"Says hello.\"\n\
             version = \"{version}\"\npoints = [\"action\"]\n"
        );
        for (name, content) in [
            ("hello/plugin.toml", manifest.as_str()),
            ("hello/libhello.so", "library"),
        ] {
            zip.start_file(name, options)
                .unwrap_or_else(|e| panic!("{e}"));
            zip.write_all(content.as_bytes())
                .unwrap_or_else(|e| panic!("{e}"));
        }
        zip.finish().unwrap_or_else(|e| panic!("{e}")).into_inner()
    }

    fn sha256(bytes: &[u8]) -> String {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let file = dir.path().join("package");
        fs::write(&file, bytes).unwrap_or_else(|e| panic!("{e}"));
        crate::plugins::sha256_of(&file).unwrap_or_else(|e| panic!("{e}"))
    }

    /// An index that offers Hello for [`RICK`] on this platform as the
    /// package `described`, served at [`PACKAGE`].
    fn index(server: &Server, described: &[u8]) -> serde_json::Value {
        index_for(server, described, RICK)
    }

    /// [`index`] with the build made for the release `rick` of Rick.
    fn index_for(server: &Server, described: &[u8], rick: &str) -> serde_json::Value {
        json!({
            "schema": 1,
            "generated": "2026-11-19T02:00:00Z",
            "apps": { "rick": { "plugins": [{
                "id": "hello",
                "name": "Hello",
                "description": "Says hello.",
                "publisher": "Open-Breach",
                "meta": { "points": ["action"], "triggers": { "markers": [".hello"] } },
                "builds": [{
                    "version": "1.2.0",
                    "app_version": rick,
                    "api_version": API_VERSION,
                    "platform": plugins_registry::platform(),
                    "url": server.url(PACKAGE),
                    "size": described.len(),
                    "sha256": sha256(described),
                }],
            }] } },
        })
    }

    /// A Rick of version [`RICK`] on a fresh config directory that reads
    /// `server`'s list, which `signer` signed, and has read it.
    fn start(server: &Server, signer: &Signer) -> Harness {
        let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
        start_on(&fake, server, signer)
    }

    fn start_on(fake: &Arc<Fake>, server: &Server, signer: &Signer) -> Harness {
        let config = fake.config_dir();
        fs::create_dir_all(&config).unwrap_or_else(|e| panic!("{e}"));
        let settings = format!(
            "[registry]\nurl = \"{}\"\npublic_key = \"{}\"\n",
            server.index_url(),
            signer.public_key()
        );
        fs::write(config.join(SETTINGS_FILE), settings).unwrap_or_else(|e| panic!("{e}"));
        let root = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let ui = Arc::new(Scripted::default());
        let mut app = App::new(Config {
            platform: fake.clone(),
            ui: ui.clone(),
            extensions: Vec::new(),
            config_dir: Some(config),
            start: Some(local(root.path())),
        });
        app.set_rick_version(RICK);
        let mut harness = Harness {
            root,
            fake: fake.clone(),
            ui,
            app,
            events: Vec::new(),
        };
        harness.send(Command::Registry(RegistryCommand::Start));
        harness.settle();
        harness.send(Command::Registry(RegistryCommand::Daily));
        harness.settle();
        harness
    }

    /// The ids on offer, or why there is no list.
    fn offered(app: &App) -> Vec<String> {
        match app.offering() {
            Offering::Plugins(plugins) => plugins.into_iter().map(|p| p.id).collect(),
            other => panic!("no list: {other:?}"),
        }
    }

    /// The jobs of the job table, by name, with how they ended.
    fn jobs(app: &App) -> Vec<(String, JobState)> {
        app.model()
            .jobs
            .values()
            .map(|job| (job.name.clone(), job.state.clone()))
            .collect()
    }

    #[test]
    fn install_puts_an_available_plugin_into_the_per_user_folder_and_notes_it() {
        let (server, signer) = (Server::start(), Signer::new(1));
        let bytes = package("1.2.0");
        server.put(PACKAGE, bytes.clone());
        server.publish(&signer, &index(&server, &bytes));
        let mut h = start(&server, &signer);

        // On offer, with what the list says of it.
        let Offering::Plugins(plugins) = h.app.offering() else {
            panic!("a list");
        };
        let [hello] = plugins.as_slice() else {
            panic!("one plugin: {plugins:?}");
        };
        assert_eq!(
            (
                hello.id.as_str(),
                hello.name.as_str(),
                hello.version.as_str()
            ),
            ("hello", "Hello", "1.2.0")
        );
        assert_eq!(hello.points, Points::ACTION);
        assert_eq!(hello.markers, [".hello"]);
        assert_eq!(hello.size, bytes.len() as u64);
        assert!(!hello.installing);

        h.take_events();
        h.send(Command::InstallAvailable("hello".to_owned()));
        assert!(
            matches!(h.app.offering(), Offering::Plugins(p) if p[0].installing),
            "the install is running"
        );
        h.settle();

        // In the per-user plugins folder, whole, and nowhere else.
        let plugins_dir = h.app.plugins_dir();
        let folder = plugins_dir.join("hello");
        assert_eq!(
            fs::read_to_string(folder.join("libhello.so"))
                .ok()
                .as_deref(),
            Some("library")
        );
        let downloads = h.fake.config_dir().join(crate::CACHE_DIR).join("downloads");
        assert_eq!(
            fs::read_dir(&downloads).map(Iterator::count).ok(),
            Some(0),
            "the package is gone once it is unpacked"
        );
        // The row is under Installed, waiting for the next start, and the
        // plugin is no longer on offer.
        let row = h
            .app
            .plugins()
            .iter()
            .find(|row| row.id == "hello")
            .unwrap_or_else(|| panic!("a row"));
        assert_eq!(row.status, PluginStatus::Installed);
        assert_eq!(
            (row.path.as_path(), row.built_in),
            (folder.as_path(), false)
        );
        assert_eq!(offered(&h.app), [] as [String; 0]);
        let events = h.take_events();
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::PluginInstalled { path } if *path == folder)),
            "{events:?}"
        );
        assert_eq!(
            jobs(&h.app),
            [("Install Hello".to_owned(), JobState::Done)],
            "a job of the operations area"
        );

        // The record says where it came from, in memory and in the file.
        let noted = h.app.installed().get(&folder).cloned();
        let noted = noted.unwrap_or_else(|| panic!("an entry"));
        assert_eq!(noted.source, Origin::Registry);
        assert_eq!(noted.version, "1.2.0");
        assert_eq!(noted.sha256, sha256(&bytes));
        assert_eq!(noted.url, None);
        let text = fs::read_to_string(plugins_dir.join(RECORD_FILE)).unwrap_or_default();
        assert!(
            text.contains("[hello]\nsource = \"registry\"\nversion = \"1.2.0\"\nsha256 = \""),
            "{text}"
        );
        assert_eq!(&Record::read(&plugins_dir), h.app.installed());

        // The package was asked for once, as Rick.
        let asked: Vec<_> = server
            .requests()
            .into_iter()
            .filter(|request| request.path == PACKAGE)
            .collect();
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].header("user-agent"), Some("rick/0.6.0"));
    }

    #[test]
    fn a_package_that_differs_from_the_list_is_deleted_and_the_job_fails() {
        let (server, signer) = (Server::start(), Signer::new(1));
        let described = package("1.2.0");
        server.publish(&signer, &index(&server, &described));
        // The registry serves another package than the one it describes:
        // one byte of it changed, then one of another size.
        let mut changed = described.clone();
        let middle = changed.len() / 2;
        changed[middle] ^= 0xff;
        for served in [changed, package("6.6.6-and-a-longer-version")] {
            server.put(PACKAGE, served);
            let mut h = start(&server, &signer);
            h.take_events();
            h.send(Command::InstallAvailable("hello".to_owned()));
            h.settle();

            let said = "The download did not match the plugin list";
            assert_eq!(
                jobs(&h.app),
                [(
                    "Install Hello".to_owned(),
                    JobState::Failed(said.to_owned())
                )]
            );
            let events = h.take_events();
            let notice = format!("Install Hello: {said}");
            assert!(
                events.iter().any(|e| matches!(
                    e,
                    Event::Notify { level: Level::Error, text } if *text == notice
                )),
                "{events:?}"
            );
            // Nothing landed in the plugins folder, and the download is
            // deleted.
            let plugins_dir = h.app.plugins_dir();
            assert!(!plugins_dir.join("hello").exists());
            assert!(!plugins_dir.join(RECORD_FILE).exists());
            let downloads = h.fake.config_dir().join(crate::CACHE_DIR).join("downloads");
            assert_eq!(fs::read_dir(&downloads).map(Iterator::count).ok(), Some(0));
            assert_eq!(h.app.plugins(), []);
            // It is still on offer, and can be asked for again.
            assert!(matches!(
                h.app.offering(),
                Offering::Plugins(p) if p.len() == 1 && !p[0].installing
            ));
        }
    }

    #[test]
    fn a_package_that_does_not_arrive_fails_the_job_in_plain_words() {
        let (server, signer) = (Server::start(), Signer::new(1));
        // Described, and not served.
        server.publish(&signer, &index(&server, &package("1.2.0")));
        let mut h = start(&server, &signer);
        h.send(Command::InstallAvailable("hello".to_owned()));
        h.settle();
        let ended = jobs(&h.app);
        let [(name, JobState::Failed(said))] = ended.as_slice() else {
            panic!("the job failed: {ended:?}");
        };
        assert_eq!(name, "Install Hello");
        assert!(
            said.starts_with("The plugin could not be downloaded: ") && said.contains("404"),
            "{said}"
        );
        assert!(!h.app.plugins_dir().join("hello").exists());
        // A plugin the list does not have is no job at all.
        h.send(Command::InstallAvailable("nobody".to_owned()));
        h.settle();
        assert_eq!(jobs(&h.app).len(), 1);
    }

    #[test]
    fn remove_drops_the_record_s_entry_and_the_plugin_is_available_again() {
        let (server, signer) = (Server::start(), Signer::new(1));
        let bytes = package("1.2.0");
        server.put(PACKAGE, bytes.clone());
        server.publish(&signer, &index(&server, &bytes));
        let mut h = start(&server, &signer);
        h.send(Command::InstallAvailable("hello".to_owned()));
        h.settle();
        let plugins_dir = h.app.plugins_dir();
        let folder = plugins_dir.join("hello");
        assert!(h.app.installed().get(&folder).is_some());
        assert_eq!(offered(&h.app), [] as [String; 0]);

        h.send(Command::RemovePlugin(folder.clone()));
        h.settle();
        assert!(!folder.exists());
        assert_eq!(h.app.installed().get(&folder), None);
        assert_eq!(Record::read(&plugins_dir), Record::default());
        assert_eq!(offered(&h.app), ["hello"]);
    }

    #[test]
    fn install_from_file_notes_the_file_and_a_start_drops_what_is_gone() {
        let (server, signer) = (Server::start(), Signer::new(1));
        server.publish(&signer, &index(&server, &package("1.2.0")));
        let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
        let mut h = start_on(&fake, &server, &signer);
        let bytes = package("0.9.0");
        let file = h.root.path().join("hello.rkp");
        fs::write(&file, &bytes).unwrap_or_else(|e| panic!("{e}"));
        h.send(Command::InstallPlugin(file));
        h.settle();
        let plugins_dir = h.app.plugins_dir();
        let folder = plugins_dir.join("hello");
        let noted = h.app.installed().get(&folder).cloned();
        let noted = noted.unwrap_or_else(|| panic!("an entry"));
        assert_eq!(noted.source, Origin::File);
        assert_eq!(noted.version, "0.9.0");
        assert_eq!(noted.sha256, sha256(&bytes));
        // In the folder, so not on offer, whatever its origin.
        assert_eq!(offered(&h.app), [] as [String; 0]);

        // A plugin put there by hand has no entry, and the entry of a
        // folder somebody deleted is dropped at the next start.
        fs::create_dir_all(plugins_dir.join("by-hand")).unwrap_or_else(|e| panic!("{e}"));
        fs::remove_dir_all(&folder).unwrap_or_else(|e| panic!("{e}"));
        assert!(Record::read(&plugins_dir).get(&folder).is_some());
        let again = start_on(&fake, &server, &signer);
        assert_eq!(again.app.installed(), &Record::default());
        assert_eq!(Record::read(&plugins_dir), Record::default());
        assert!(plugins_dir.join("by-hand").is_dir());
    }

    /// A config directory whose per-user plugins folder holds Hello in
    /// version `version`, noted in the record as coming from `source`.
    fn installed_before(fake: &Arc<Fake>, version: &str, source: &str) -> std::path::PathBuf {
        let plugins_dir = fake.config_dir().join(crate::plugins::PLUGINS_DIR);
        let folder = plugins_dir.join("hello");
        fs::create_dir_all(&folder).unwrap_or_else(|e| panic!("{e}"));
        fs::write(folder.join("old.txt"), "the build before").unwrap_or_else(|e| panic!("{e}"));
        fs::write(
            plugins_dir.join(RECORD_FILE),
            format!("[hello]\nsource = \"{source}\"\nversion = \"{version}\"\nsha256 = \"00\"\n"),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        folder
    }

    /// Hello's row as the binary hands it over: in the per-user folder,
    /// in version `version`, with what the host made of it.
    fn row(folder: &Path, version: &str, status: PluginStatus) -> crate::Plugin {
        crate::Plugin {
            id: "hello".to_owned(),
            name: "Hello".to_owned(),
            description: String::new(),
            version: version.to_owned(),
            points: Points::ACTION,
            previews: Vec::new(),
            status,
            path: folder.to_owned(),
            built_in: false,
            removed: false,
        }
    }

    /// A Rick of version `rick` on `fake`'s config directory with `rows`
    /// from its plugin host, after its start read the plugin list.
    fn start_with(
        fake: &Arc<Fake>,
        server: &Server,
        signer: &Signer,
        rick: &str,
        rows: Vec<crate::Plugin>,
    ) -> Harness {
        let config = fake.config_dir();
        fs::create_dir_all(&config).unwrap_or_else(|e| panic!("{e}"));
        let settings = format!(
            "[registry]\nurl = \"{}\"\npublic_key = \"{}\"\n",
            server.index_url(),
            signer.public_key()
        );
        fs::write(config.join(SETTINGS_FILE), settings).unwrap_or_else(|e| panic!("{e}"));
        let root = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let ui = Arc::new(Scripted::default());
        let mut app = App::new(Config {
            platform: fake.clone(),
            ui: ui.clone(),
            extensions: Vec::new(),
            config_dir: Some(config),
            start: Some(local(root.path())),
        });
        app.set_rick_version(rick);
        app.set_plugins(rows);
        let mut harness = Harness {
            root,
            fake: fake.clone(),
            ui,
            app,
            events: Vec::new(),
        };
        harness.settle();
        harness.take_events();
        harness.send(Command::Registry(RegistryCommand::Start));
        harness.settle();
        harness.send(Command::Registry(RegistryCommand::Daily));
        harness.settle();
        harness
    }

    /// How many plugins the start said have an update, when it said so.
    fn announced(events: &[Event]) -> Option<usize> {
        events.iter().find_map(|event| match event {
            Event::PluginUpdates { count } => Some(*count),
            _ => None,
        })
    }

    #[test]
    fn a_registry_plugin_built_for_the_rick_before_is_an_update_and_update_installs_it() {
        let (server, signer) = (Server::start(), Signer::new(1));
        let bytes = package("1.2.0");
        server.put(PACKAGE, bytes.clone());
        server.publish(&signer, &index(&server, &bytes));
        let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
        let folder = installed_before(&fake, "1.1.0", "registry");
        let refused = || PluginStatus::Refused("Built for Rick 0.5.0".to_owned());

        let mut h = start_with(
            &fake,
            &server,
            &signer,
            RICK,
            vec![row(&folder, "1.1.0", refused())],
        );
        let updates = h.app.updates();
        let [update] = updates.as_slice() else {
            panic!("one update: {updates:?}");
        };
        assert_eq!(
            (update.id.as_str(), update.version.as_str()),
            ("hello", "1.2.0")
        );
        assert_eq!(update.path, folder);
        assert!(update.refused && !update.updating);
        // In the folder, so not on offer; and the start says so, once.
        assert_eq!(offered(&h.app), [] as [String; 0]);
        assert_eq!(announced(&h.take_events()), Some(1));

        // The next start of the same release has the update still, and
        // says nothing again.
        let mut again = start_with(
            &fake,
            &server,
            &signer,
            RICK,
            vec![row(&folder, "1.1.0", refused())],
        );
        assert_eq!(again.app.updates().len(), 1);
        assert_eq!(announced(&again.take_events()), None);
        drop(again);

        // Update all: an install over the old folder.
        h.send(Command::UpdateAll);
        assert!(
            h.app
                .updates()
                .first()
                .is_some_and(|update| update.updating)
        );
        h.settle();
        assert_eq!(jobs(&h.app), [("Install Hello".to_owned(), JobState::Done)]);
        assert_eq!(
            fs::read_to_string(folder.join("libhello.so"))
                .ok()
                .as_deref(),
            Some("library")
        );
        assert!(!folder.join("old.txt").exists(), "the old build is gone");
        let noted = h.app.installed().get(&folder).cloned();
        let noted = noted.unwrap_or_else(|| panic!("an entry"));
        assert_eq!(
            (noted.source, noted.version.as_str()),
            (Origin::Registry, "1.2.0")
        );
        assert_eq!(noted.sha256, sha256(&bytes));
        let rows = h.app.plugins();
        assert_eq!(rows.len(), 1, "the row is the same one: {rows:?}");
        assert_eq!(rows[0].status, PluginStatus::Installed);
        assert_eq!(
            h.app.updates(),
            [],
            "it waits for the next start, and is no update"
        );
    }

    #[test]
    fn only_a_plugin_the_registry_installed_and_has_a_build_for_is_an_update() {
        let (server, signer) = (Server::start(), Signer::new(1));
        let bytes = package("1.2.0");
        server.publish(&signer, &index(&server, &bytes));
        let refused = || PluginStatus::Refused("Built for Rick 0.5.0".to_owned());
        let fake = || Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));

        // Installed from a file: nobody's to update, and nothing is said.
        let own = fake();
        let folder = installed_before(&own, "1.1.0", "file");
        let mut h = start_with(
            &own,
            &server,
            &signer,
            RICK,
            vec![row(&folder, "1.1.0", refused())],
        );
        assert_eq!(h.app.updates(), []);
        assert_eq!(announced(&h.take_events()), None);

        // Put there by hand: no entry at all.
        let own = fake();
        let folder = installed_before(&own, "1.1.0", "registry");
        fs::remove_file(own.config_dir().join("plugins").join(RECORD_FILE))
            .unwrap_or_else(|e| panic!("{e}"));
        let h = start_with(
            &own,
            &server,
            &signer,
            RICK,
            vec![row(&folder, "1.1.0", refused())],
        );
        assert_eq!(h.app.updates(), []);

        // The registry's, and the list has no build for this Rick.
        let own = fake();
        let folder = installed_before(&own, "1.1.0", "registry");
        let h = start_with(
            &own,
            &server,
            &signer,
            "0.7.0",
            vec![row(&folder, "1.1.0", refused())],
        );
        assert_eq!(h.app.updates(), []);

        // The registry's and loaded: an update only for a higher version.
        let own = fake();
        let folder = installed_before(&own, "1.2.0", "registry");
        let h = start_with(
            &own,
            &server,
            &signer,
            RICK,
            vec![row(&folder, "1.2.0", PluginStatus::Loaded)],
        );
        assert_eq!(h.app.updates(), [], "the same version");
        let own = fake();
        let folder = installed_before(&own, "1.1.9", "registry");
        let mut h = start_with(
            &own,
            &server,
            &signer,
            RICK,
            vec![row(&folder, "1.1.9", PluginStatus::Loaded)],
        );
        let updates = h.app.updates();
        assert_eq!(updates.len(), 1, "{updates:?}");
        assert!(!updates[0].refused);
        assert_eq!(announced(&h.take_events()), Some(1));
    }

    #[test]
    fn the_updates_of_another_release_of_rick_are_said_again() {
        let (server, signer) = (Server::start(), Signer::new(1));
        let bytes = package("1.2.0");
        let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
        let folder = installed_before(&fake, "1.1.0", "registry");
        let refused = || PluginStatus::Refused("Built for Rick 0.5.0".to_owned());

        server.publish(&signer, &index_for(&server, &bytes, "0.6.0"));
        let mut h = start_with(
            &fake,
            &server,
            &signer,
            "0.6.0",
            vec![row(&folder, "1.1.0", refused())],
        );
        assert_eq!(announced(&h.take_events()), Some(1));
        drop(h);
        // Rick is updated again before the plugin was: the new release's
        // builds are another occasion.
        server.publish(&signer, &index_for(&server, &bytes, "0.6.1"));
        let cached = fake
            .config_dir()
            .join(crate::CACHE_DIR)
            .join(plugins_registry::CACHE_DIR)
            .join(plugins_registry::INDEX_FILE);
        fs::remove_file(cached).unwrap_or_else(|e| panic!("{e}"));
        let mut h = start_with(
            &fake,
            &server,
            &signer,
            "0.6.1",
            vec![row(&folder, "1.1.0", refused())],
        );
        assert_eq!(announced(&h.take_events()), Some(1));
        drop(h);
        let mut h = start_with(
            &fake,
            &server,
            &signer,
            "0.6.1",
            vec![row(&folder, "1.1.0", refused())],
        );
        assert_eq!(announced(&h.take_events()), None);
    }

    #[test]
    fn install_from_an_address_is_handled_as_a_file_and_notes_the_address() {
        let (server, signer) = (Server::start(), Signer::new(1));
        server.publish(&signer, &index(&server, &package("1.2.0")));
        // A package of the user's own, somewhere else on the same server.
        let bytes = package("3.0.0");
        server.put("downloads/hello.rkp", bytes.clone());
        let address = server.url("downloads/hello.rkp");
        let mut h = start(&server, &signer);
        h.take_events();
        h.send(Command::InstallPluginFrom(format!("  {address} ")));
        h.settle();

        assert_eq!(
            jobs(&h.app),
            [("Install hello.rkp".to_owned(), JobState::Done)]
        );
        let plugins_dir = h.app.plugins_dir();
        let folder = plugins_dir.join("hello");
        assert!(folder.join("plugin.toml").is_file());
        let noted = h.app.installed().get(&folder).cloned();
        let noted = noted.unwrap_or_else(|| panic!("an entry"));
        assert_eq!(noted.source, Origin::Url);
        assert_eq!(noted.version, "3.0.0");
        assert_eq!(
            noted.sha256,
            sha256(&bytes),
            "noted, and checked against nothing"
        );
        assert_eq!(noted.url.as_deref(), Some(address.as_str()));
        let text = fs::read_to_string(plugins_dir.join(RECORD_FILE)).unwrap_or_default();
        assert!(text.contains("source = \"url\""), "{text}");
        assert!(text.contains(&format!("url = \"{address}\"")), "{text}");
        let row = &h.app.plugins()[0];
        assert_eq!(
            (row.id.as_str(), &row.status),
            ("hello", &PluginStatus::Installed)
        );
        let downloads = h.fake.config_dir().join(crate::CACHE_DIR).join("downloads");
        assert_eq!(fs::read_dir(&downloads).map(Iterator::count).ok(), Some(0));
        // Installed from an address, it is nobody's to update.
        assert_eq!(h.app.updates(), []);
    }

    #[test]
    fn an_address_that_is_not_https_is_refused_and_a_download_that_fails_says_so() {
        let (server, signer) = (Server::start(), Signer::new(1));
        server.publish(&signer, &index(&server, &package("1.2.0")));
        let mut h = start(&server, &signer);
        h.take_events();

        // Not https, and not this machine: no job and no request, a
        // notice that says what is needed.
        let asked_before = server.requests().len();
        for refused in [
            "http://example.org/hello.rkp",
            "http://192.168.1.10/hello.rkp",
            "http://localhost.example.org/hello.rkp",
            "hello.rkp",
            "ftp://example.org/h.rkp",
        ] {
            h.send(Command::InstallPluginFrom(refused.to_owned()));
            h.settle();
            let events = h.take_events();
            let notice =
                format!("{refused} was not requested: a plugin package needs an https:// address");
            assert!(
                events.iter().any(|e| matches!(
                    e,
                    Event::Notify { level: Level::Error, text } if *text == notice
                )),
                "{refused}: {events:?}"
            );
        }
        assert_eq!(jobs(&h.app), []);
        assert_eq!(server.requests().len(), asked_before);

        // Nothing at the address.
        h.send(Command::InstallPluginFrom(server.url("downloads/none.rkp")));
        h.settle();
        let ended = jobs(&h.app);
        let [(name, JobState::Failed(said))] = ended.as_slice() else {
            panic!("the job failed: {ended:?}");
        };
        assert_eq!(name, "Install none.rkp");
        assert!(
            said.starts_with("The plugin could not be downloaded: "),
            "{said}"
        );

        // An address of this machine that sends the download on to plain
        // HTTP elsewhere: the job fails with the same words, and the
        // other machine is not asked.
        server.redirect("downloads/moved.rkp", "http://plugins.example/hello.rkp");
        h.send(Command::InstallPluginFrom(
            server.url("downloads/moved.rkp"),
        ));
        h.settle();
        let ended = jobs(&h.app);
        let Some((name, JobState::Failed(said))) = ended.last() else {
            panic!("the job failed: {ended:?}");
        };
        assert_eq!(name, "Install moved.rkp");
        assert_eq!(
            said,
            "The download was sent on to http://plugins.example/hello.rkp, which was not \
             requested: a plugin package needs an https:// address"
        );
        assert_eq!(server.requests().len(), asked_before + 2);
        assert_eq!(h.app.plugins(), [], "nothing was installed");

        // Something that is no package: it is handled as such a file is.
        server.put("downloads/page.rkp", "<html>not a package</html>");
        h.send(Command::InstallPluginFrom(server.url("downloads/page.rkp")));
        h.settle();
        let ended = jobs(&h.app);
        let Some((_, JobState::Failed(said))) = ended.last() else {
            panic!("the job failed: {ended:?}");
        };
        assert!(said.contains("is not a plugin package"), "{said}");
        assert_eq!(h.app.plugins(), [], "nothing was installed");
        let downloads = h.fake.config_dir().join(crate::CACHE_DIR).join("downloads");
        assert_eq!(fs::read_dir(&downloads).map(Iterator::count).ok(), Some(0));
    }

    #[test]
    fn a_packaged_rick_downloads_no_plugin_and_has_no_update_to_offer() {
        use crate::plugins::Packaging;
        let (server, signer) = (Server::start(), Signer::new(1));
        let bytes = package("1.2.0");
        server.put(PACKAGE, bytes.clone());
        server.publish(&signer, &index(&server, &bytes));
        let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
        let folder = installed_before(&fake, "1.1.0", "registry");
        let refused = PluginStatus::Refused("Built for Rick 0.5.0".to_owned());
        let mut h = start_with(
            &fake,
            &server,
            &signer,
            RICK,
            vec![row(&folder, "1.1.0", refused)],
        );
        assert_eq!(h.app.packaging(), None);
        assert_eq!(h.app.updates().len(), 1);

        h.app.set_packaging(Some(Packaging::Deb));
        assert_eq!(h.app.packaging(), Some(Packaging::Deb));
        assert_eq!(h.app.updates(), [], "the package manager moves its plugins");
        h.send(Command::UpdateAll);
        h.send(Command::InstallAvailable("hello".to_owned()));
        h.settle();
        assert_eq!(jobs(&h.app), []);
        assert!(
            folder.join("old.txt").is_file(),
            "nothing was installed over it"
        );
        assert!(
            server
                .requests()
                .iter()
                .all(|request| request.path != PACKAGE),
            "nothing was downloaded"
        );
    }
}
