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
                version: Version::new(0, 1, 0),
                api_version: API_VERSION,
                requires: Capabilities::empty(),
                points: Points::SEARCH_INDEX,
                platforms: Platforms::ALL,
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
            Availability::Disabled("service gone")
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

#[test]
fn without_a_subtree_index_the_toggle_is_absent_and_a_search_ends_with_a_notice() {
    let mut h = harness();
    h.settle();
    assert!(!h.app.model().subtree_search);
    assert_eq!(h.app.model().search_indexes, Vec::<ExtId>::new());
    let pane = h.app.model().focused;
    h.send(Command::Subtree { pane, on: true });
    h.send(Command::Filter {
        pane,
        text: "foo".into(),
    });
    h.settle();
    let events = h.take_events();
    assert!(
        events.iter().any(|e| matches!(
            e,
            Event::Notify { level: Level::Error, text } if text.contains("No subdirectory search")
        )),
        "{events:?}"
    );
    assert!(!h.app.is_busy());
    assert_eq!(h.names(), Vec::<String>::new());
}

#[test]
fn subtree_search_streams_matches_as_a_job_and_a_new_text_supersedes_it() {
    let mut h = harness_with(vec![IndexExtension::around(
        "fake-walk",
        Arc::new(FakeWalk),
    )]);
    h.settle();
    assert!(h.app.model().subtree_search);
    assert!(
        h.app.model().search_indexes.is_empty(),
        "a walk is not a platform index"
    );
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
    });
    h.fake.add_volume(platform::Volume {
        name: "data".into(),
        mount_point: h.root.path().to_path_buf(),
        kind: platform::VolumeKind::Fixed,
        fs_type: "ext4".into(),
        device: None,
        total: Some(100),
        free: Some(50),
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

    pub(super) const ID: ProviderId = ProviderId("sftp");

    #[derive(Default)]
    pub(super) struct Stub {
        pub(super) connections: Mutex<Vec<Connection>>,
        pub(super) connected: Mutex<Vec<String>>,
        schema: Option<ConnectionSchema>,
    }

    fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
        m.lock().unwrap_or_else(PoisonError::into_inner)
    }

    impl Stub {
        pub(super) fn new() -> Arc<Self> {
            Arc::new(Self {
                schema: Some(ConnectionSchema {
                    title: "Stub",
                    fields: vec![
                        ConnectionField {
                            key: "host",
                            label: "Host",
                            kind: ConnectionFieldKind::Text {
                                default: String::new(),
                            },
                            required: true,
                        },
                        ConnectionField {
                            key: "port",
                            label: "Port",
                            kind: ConnectionFieldKind::Integer {
                                default: 22,
                                min: 1,
                                max: 65535,
                            },
                            required: false,
                        },
                        ConnectionField {
                            key: "password",
                            label: "Password",
                            kind: ConnectionFieldKind::Secret,
                            required: false,
                        },
                    ],
                }),
                ..Self::default()
            })
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
                return stream::once(async { Err(Error::NotFound) }).boxed();
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
                    version: Version::new(0, 1, 0),
                    api_version: API_VERSION,
                    requires: Capabilities::empty(),
                    points: Points::PROVIDER,
                    platforms: Platforms::ALL,
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
    let text = fs::read_to_string(&settings_path).unwrap_or_else(|e| panic!("{e}"));
    assert!(text.contains("[[connections]]"), "{text}");
    assert!(text.contains("provider = \"sftp\""), "{text}");
    assert!(text.contains("name = \"nas\""), "{text}");
    assert!(text.contains("host = \"10.0.0.2\""), "{text}");
    assert!(
        !text.contains("hunter2"),
        "the secret is not in the file: {text}"
    );
    assert!(!text.contains("stray"), "{text}");
    assert_eq!(
        h.fake
            .secrets()
            .get("sftp/nas/password")
            .map(String::as_str),
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
    let text = fs::read_to_string(&settings_path).unwrap_or_else(|e| panic!("{e}"));
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
