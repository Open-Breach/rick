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
    let text = fs::read_to_string(h.app.settings_path()).unwrap_or_else(|e| panic!("{e}"));
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

#[test]
fn subtree_search_streams_matches_as_a_job_and_a_new_text_supersedes_it() {
    let mut h = harness();
    h.settle();
    let pane = h.app.model().focused;
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
    assert!(
        h.app
            .model()
            .jobs
            .get(&conflict.job)
            .is_some_and(|j| matches!(&j.state, model::JobState::Running(types::Progress::Message(m)) if m.contains("exists")))
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
fn recent_follows_the_home_watch_and_survives_a_restart() {
    let mut h = harness();
    h.settle();
    let home = h.fake.home_dir();
    fs::write(home.join("older.txt"), "o").unwrap_or_else(|e| panic!("{e}"));
    assert!(
        wait_until(&mut h, Duration::from_millis(500), |h| {
            h.app
                .model()
                .recent
                .entries()
                .first()
                .is_some_and(|e| e.name == "older.txt")
        }),
        "the touched file is the first row within 500 ms"
    );
    std::thread::sleep(Duration::from_millis(30));
    fs::write(home.join("newer.txt"), "n").unwrap_or_else(|e| panic!("{e}"));
    assert!(wait_until(&mut h, Duration::from_millis(500), |h| {
        h.app
            .model()
            .recent
            .entries()
            .first()
            .is_some_and(|e| e.name == "newer.txt")
    }));
    let rows: Vec<String> = h
        .app
        .model()
        .recent
        .entries()
        .iter()
        .map(|e| e.name.clone())
        .collect();
    assert_eq!(rows, ["newer.txt", "older.txt"]);
    h.settle();
    let file = h.app.settings_path().with_file_name(super::RECENT_FILE);
    assert!(
        wait_until(&mut h, Duration::from_secs(2), |_| {
            fs::read_to_string(&file).is_ok_and(|t| t.contains("newer.txt"))
        }),
        "the list is written"
    );

    // A second app from the same config directory has the rows.
    let ui = Arc::new(Scripted::default());
    let again = App::new(Config {
        platform: h.fake.clone(),
        ui,
        extensions: Vec::new(),
        config_dir: Some(h.fake.config_dir()),
        start: Some(local(h.root.path())),
    });
    let mut h2 = Harness {
        root: tempfile::tempdir().unwrap_or_else(|e| panic!("{e}")),
        fake: h.fake.clone(),
        ui: h.ui.clone(),
        app: again,
        events: Vec::new(),
    };
    assert!(wait_until(&mut h2, Duration::from_secs(5), |h| {
        h.app.model().recent.entries().len() == 2
    }));
    let rows: Vec<String> = h2
        .app
        .model()
        .recent
        .entries()
        .iter()
        .map(|e| e.name.clone())
        .collect();
    assert_eq!(rows, ["newer.txt", "older.txt"]);
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
        total: Some(100),
        free: Some(50),
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
