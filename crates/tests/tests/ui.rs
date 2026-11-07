//! Acceptance (issue 4): the egui shell driven headlessly by `egui_kittest`.
//! Every shortcut of the spec's tables reaches its intent, and scrolling the
//! 200k fixture in details view stays within a 60 fps frame budget.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use ext::ActionId;
use model::Column;
use platform::{ConfigDir, Fake};
use tests::local;
use ui_egui::keys::{Direction, Intent, UI_INTENTS, to_egui_key, to_egui_modifiers};
use ui_egui::{EguiHost, RickUi};

/// Writes straight to the process's stderr, past libtest's capture, so CI
/// logs show progress and panics as they happen.
fn trace(text: &str) {
    use std::io::Write as _;
    let _ = writeln!(std::io::stderr(), "[ui test] {text}");
}

/// Reports panics as they happen and aborts a test that runs past `limit`,
/// so a stuck frame fails the job quickly with the last progress line
/// instead of waiting for the job timeout.
fn guard(name: &'static str, limit: Duration) -> impl Drop {
    use std::sync::atomic::{AtomicBool, Ordering};
    struct Disarm(Arc<AtomicBool>);
    impl Drop for Disarm {
        fn drop(&mut self) {
            self.0.store(false, Ordering::SeqCst);
        }
    }
    static HOOK: std::sync::Once = std::sync::Once::new();
    HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            trace(&format!("panic: {info}"));
            previous(info);
        }));
    });
    let armed = Arc::new(AtomicBool::new(true));
    let flag = Arc::clone(&armed);
    std::thread::spawn(move || {
        std::thread::sleep(limit);
        if flag.load(Ordering::SeqCst) {
            trace(&format!("{name}: still running after {limit:?}; aborting"));
            std::process::abort();
        }
    });
    Disarm(armed)
}

fn harness(start: &Path) -> Harness<'static, RickUi> {
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: Vec::new(),
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
/// passed, so listings and jobs have landed in the model.
fn settle(harness: &mut Harness<'_, RickUi>) {
    let started = Instant::now();
    let deadline = started + Duration::from_secs(30);
    let mut reported = 0;
    loop {
        let waited = started.elapsed().as_secs();
        if waited >= reported + 5 {
            reported = waited;
            trace(&format!("settling for {waited} s: {:?}", harness.state()));
        }
        for _ in 0..3 {
            harness.step();
        }
        if !harness.state().app().is_busy() {
            std::thread::sleep(Duration::from_millis(30));
            harness.step();
            if !harness.state().app().is_busy() {
                break;
            }
        }
        assert!(Instant::now() < deadline, "did not settle");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The spec's shortcut tables: every row and the intent it must reach.
fn spec_rows() -> Vec<(&'static str, Intent)> {
    let action = |id: &'static str| Intent::Action(ActionId(id));
    vec![
        // Navigation
        ("Enter", Intent::Open),
        ("Mod+Enter", action("core.open_default")),
        ("Backspace", Intent::Back),
        ("Alt+Left", Intent::Back),
        ("Alt+Right", Intent::Forward),
        ("Alt+Up", Intent::Parent),
        ("Mod+Shift+Home", Intent::Root),
        ("Mod+L", Intent::FocusPath),
        ("F5", Intent::Refresh),
        ("Mod+R", Intent::Refresh),
        ("Mod+Shift+P", Intent::Palette),
        ("Mod+P", Intent::QuickOpen),
        // Selection
        ("Mod+A", Intent::SelectAll),
        ("Mod+Shift+A", Intent::InvertSelection),
        ("Mod+Shift+S", Intent::SelectByPattern),
        ("Space", Intent::ToggleFocused),
        ("Escape", Intent::Escape),
        // File operations
        ("Mod+C", action("core.copy")),
        ("Mod+X", action("core.cut")),
        ("Mod+V", action("core.paste")),
        ("Mod+Shift+V", Intent::PasteAsLink),
        ("Mod+D", action("core.duplicate")),
        ("Delete", action("core.trash")),
        ("Shift+Delete", action("core.delete")),
        ("F2", action("core.rename")),
        ("Mod+F2", Intent::BulkRename),
        ("Mod+N", Intent::NewWindow),
        ("Mod+Shift+N", action("core.new_folder")),
        ("Mod+Shift+C", action("core.copy_path_and_name")),
        ("Mod+Alt+C", action("core.copy_name")),
        ("Mod+Alt+Shift+C", action("core.copy_path")),
        ("Mod+I", Intent::Inspect),
        ("Alt+Enter", Intent::Inspect),
        ("Mod+Z", Intent::Undo),
        ("Mod+Shift+Z", Intent::Redo),
        ("Mod+Y", Intent::Redo),
        ("Mod+O", Intent::OpenWith),
        ("Mod+Shift+O", action("core.reveal")),
        // Tabs and panes
        ("Mod+T", Intent::NewTab),
        ("Mod+W", Intent::CloseTab),
        ("Mod+Shift+T", Intent::ReopenTab),
        ("Mod+Shift+D", Intent::DuplicateTab),
        ("Mod+Tab", Intent::NextTab),
        ("Mod+PageDown", Intent::NextTab),
        ("Mod+Shift+Tab", Intent::PrevTab),
        ("Mod+PageUp", Intent::PrevTab),
        ("Mod+1", Intent::Tab(1)),
        ("Mod+5", Intent::Tab(5)),
        ("Mod+9", Intent::Tab(9)),
        ("Mod+Shift+PageUp", Intent::MoveTabLeft),
        ("Mod+Shift+PageDown", Intent::MoveTabRight),
        ("Mod+\\", Intent::SplitRight),
        ("Mod+Shift+\\", Intent::SplitDown),
        ("Mod+Alt+Left", Intent::FocusPane(Direction::Left)),
        ("Mod+Alt+Right", Intent::FocusPane(Direction::Right)),
        ("Mod+Alt+Up", Intent::FocusPane(Direction::Up)),
        ("Mod+Alt+Down", Intent::FocusPane(Direction::Down)),
        ("Mod+Shift+W", Intent::CloseOtherTabs),
        // View
        ("Mod+F", Intent::Filter),
        ("Mod+H", Intent::ToggleHidden),
        ("Mod+Shift+H", Intent::ToggleSystem),
        ("Mod+E", Intent::ToggleExtensions),
        ("Mod+Shift+F", Intent::ToggleFlatten),
        ("Mod+Shift+I", Intent::ToggleInspector),
        ("Mod+`", Intent::Console),
        ("Mod+,", Intent::Settings),
        ("Mod+Shift+1", Intent::ViewList),
        ("Mod+Shift+2", Intent::ViewDetails),
        ("Mod+Shift+3", Intent::ViewTree),
        ("Mod+Shift+4", Intent::ViewIcons),
        ("Mod+Plus", Intent::IconSizeUp),
        ("Mod+Minus", Intent::IconSizeDown),
        ("Mod+Shift+R", Intent::SizeColumns),
        ("Mod+Alt+N", Intent::Sort(Column::Name)),
        ("Mod+Alt+S", Intent::Sort(Column::Size)),
        ("Mod+Alt+M", Intent::Sort(Column::Modified)),
        ("Mod+Alt+T", Intent::Sort(Column::Type)),
        ("Mod+Alt+G", Intent::ClearGrouping),
        // Window rows of the first table
        ("F11", Intent::Fullscreen),
        ("Mod+Q", Intent::Quit),
        ("Mod+B", Intent::ToggleSidePanel),
    ]
}

#[test]
fn every_shortcut_in_the_spec_table_reaches_its_intent() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(dir.path().join("a.txt"), "a").unwrap_or_else(|e| panic!("{e}"));
    std::fs::create_dir(dir.path().join("sub")).unwrap_or_else(|e| panic!("{e}"));
    let _guard = guard(
        "every_shortcut_in_the_spec_table_reaches_its_intent",
        Duration::from_mins(10),
    );
    let mut harness = harness(dir.path());

    // Every intent in the UI table has a label and an id; nothing is lost.
    assert!(
        UI_INTENTS
            .iter()
            .all(|s| !s.label.is_empty() && !s.id.is_empty())
    );

    let mut missed = Vec::new();
    for (text, expected) in spec_rows() {
        trace(&format!("shortcut {text}"));
        let shortcut: ext::Shortcut = text.parse().unwrap_or_else(|e| panic!("{text}: {e}"));
        let key = to_egui_key(shortcut.key).unwrap_or_else(|| panic!("{text}: no egui key"));
        let modifiers = to_egui_modifiers(shortcut.modifiers);
        harness.state_mut().clear_dispatched();
        // Back in the sandbox with nothing selected and nothing focused, so
        // every row starts from the same place and the list has the keyboard.
        let pane = harness.state().app().model().focused;
        let mut events = Vec::new();
        harness.state_mut().app_mut().handle(
            app::Command::Navigate {
                pane,
                to: local(dir.path()),
            },
            &mut events,
        );
        harness.state_mut().app_mut().handle(
            app::Command::Select {
                pane,
                change: model::SelectionChange::Clear,
            },
            &mut events,
        );
        settle(&mut harness);
        harness.key_press_modifiers(modifiers, key);
        harness.step();
        harness.step();
        let got: Vec<Intent> = harness
            .state()
            .dispatched()
            .iter()
            .map(|d| d.intent)
            .collect();
        if !got.contains(&expected) {
            missed.push(format!("{text}: wanted {expected:?}, got {got:?}"));
        }
        // Leave dialogs, the palette and windows the shortcut may have opened.
        harness.key_press(egui::Key::Escape);
        harness.step();
        settle(&mut harness);
    }
    assert!(missed.is_empty(), "{}", missed.join("\n"));
}

#[test]
fn the_shell_shows_the_listing_and_navigates() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(dir.path().join("a.txt"), "a").unwrap_or_else(|e| panic!("{e}"));
    std::fs::create_dir(dir.path().join("sub")).unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(dir.path().join("sub/inner.txt"), "i").unwrap_or_else(|e| panic!("{e}"));
    let _guard = guard(
        "the_shell_shows_the_listing_and_navigates",
        Duration::from_mins(5),
    );
    let mut harness = harness(dir.path());
    let pane = harness.state().app().model().focused;
    assert_eq!(
        harness
            .state()
            .app()
            .model()
            .active_tab(pane)
            .map(|(_, t)| t.listing.len()),
        Some(2)
    );
    // The rows are drawn: their names are in the accessibility tree.
    harness.get_by_label("a.txt");
    harness.get_by_label("sub");

    // Keyboard: cursor down selects the first row (the folder), Enter opens it.
    harness.key_press(egui::Key::ArrowDown);
    harness.step();
    assert_eq!(
        harness
            .state()
            .app()
            .model()
            .active_tab(pane)
            .map(|(_, t)| t.selection.len()),
        Some(1)
    );
    harness.key_press(egui::Key::Enter);
    harness.step();
    settle(&mut harness);
    let location = harness
        .state()
        .app()
        .model()
        .active_tab(pane)
        .and_then(|(_, t)| platform::paths::to_path(&t.location.path).ok());
    assert_eq!(location, Some(dir.path().join("sub")));
    harness.get_by_label("inner.txt");

    // Backspace goes back; the palette opens on its shortcut.
    harness.key_press(egui::Key::Backspace);
    harness.step();
    settle(&mut harness);
    harness.get_by_label("a.txt");
    harness.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::P,
    );
    harness.step();
    assert!(harness.state().search_active());
    assert_eq!(harness.state().search_text(), ">");
    harness.key_press(egui::Key::Escape);
    harness.step();
    assert!(!harness.state().search_active());
}

/// A click anywhere on a row selects it, the name included. The name is a
/// label, and egui's text-selectable labels would otherwise take the click
/// before the row saw it.
#[test]
fn clicking_a_name_in_details_view_selects_the_row() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(dir.path().join("a.txt"), "a").unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(dir.path().join("b.txt"), "b").unwrap_or_else(|e| panic!("{e}"));
    let _guard = guard(
        "clicking_a_name_in_details_view_selects_the_row",
        Duration::from_mins(5),
    );
    let mut harness = harness(dir.path());
    let pane = harness.state().app().model().focused;
    let selected = |harness: &Harness<'_, RickUi>| -> Vec<String> {
        harness
            .state()
            .app()
            .model()
            .active_tab(pane)
            .map(|(_, t)| {
                t.listing
                    .rows()
                    .filter(|e| t.selection.contains(&e.id))
                    .map(|e| e.name.clone())
                    .collect()
            })
            .unwrap_or_default()
    };
    assert_eq!(selected(&harness), Vec::<String>::new());

    harness.get_by_label("b.txt").click();
    harness.step();
    settle(&mut harness);
    assert_eq!(selected(&harness), ["b.txt"]);

    harness.get_by_label("a.txt").click();
    harness.step();
    settle(&mut harness);
    assert_eq!(selected(&harness), ["a.txt"]);
}

/// The Decisions table asks for 60 fps while scrolling 200k entries; the
/// frame here is egui's whole frame (input, layout, tessellation) on the
/// CPU, which is all the UI code influences (QUESTIONS.md 24).
#[test]
#[ignore = "lists 200 000 entries; run with --ignored in release"]
fn scrolling_the_200k_fixture_in_details_view_holds_60_fps() {
    let fixture = tests::fixture::Config {
        entries: 200_000,
        max_depth: 0,
        sizes: tests::fixture::SizeDistribution::Empty,
        ..tests::fixture::Config::new(200_000, 11)
    }
    .generate()
    .unwrap_or_else(|e| panic!("{e}"));
    let _guard = guard(
        "scrolling_the_200k_fixture_in_details_view_holds_60_fps",
        Duration::from_mins(15),
    );
    let mut harness = harness(fixture.root());
    let pane = harness.state().app().model().focused;
    assert_eq!(
        harness
            .state()
            .app()
            .model()
            .active_tab(pane)
            .map(|(_, t)| t.listing.len()),
        Some(200_000)
    );
    // Warm up.
    for _ in 0..10 {
        harness.step();
    }
    let mut frame_times = Vec::with_capacity(240);
    for i in 0..240 {
        let delta = if i % 60 < 30 { -400.0 } else { 400.0 };
        harness.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, delta),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        });
        let started = Instant::now();
        harness.step();
        frame_times.push(started.elapsed());
    }
    frame_times.sort_unstable();
    let median = frame_times[frame_times.len() / 2];
    let p95 = frame_times[frame_times.len() * 95 / 100];
    let worst = frame_times[frame_times.len() - 1];
    println!("200k details view scrolling: median {median:?}, p95 {p95:?}, worst {worst:?}");
    assert!(
        p95 < Duration::from_micros(16_600),
        "p95 frame {p95:?} is over the 60 fps budget"
    );
}
