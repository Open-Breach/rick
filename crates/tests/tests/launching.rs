//! Acceptance (issue 149, item 6, launching and opening files), in the
//! running window (SPEC.md *Executables and scripts*, *Open in editor*,
//! *Path bar*).
//!
//! A double-click on a binary starts it; on an executable script the
//! question with its three buttons appears, and *Run* starts it; F4 on a
//! text file opens the editor the setting names. *Open in editor* is in a
//! file's context menu and not in a folder's, and the editor is a field of
//! the Behaviour page. `~/.local/bin` typed into the path bar navigates,
//! and so does a path with a variable, typed or pasted.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use model::SelectionChange;
use platform::{ConfigDir, Fake};
use tests::local;
use ui_egui::{EguiHost, RickUi};

struct Window {
    harness: Harness<'static, RickUi>,
    fake: Arc<Fake>,
}

/// A window on `start`.
fn window(start: &Path) -> Window {
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
        .with_size(egui::vec2(1200.0, 700.0))
        .with_pixels_per_point(1.0)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    Window { harness, fake }
}

fn pane(harness: &Harness<'_, RickUi>) -> model::PaneId {
    harness.state().app().model().focused
}

fn tab<'a>(harness: &'a Harness<'_, RickUi>) -> (model::TabId, &'a model::Tab) {
    harness
        .state()
        .app()
        .model()
        .active_tab(pane(harness))
        .unwrap_or_else(|| panic!("no tab"))
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

/// Runs frames until `done` holds.
fn step_until(
    harness: &mut Harness<'_, RickUi>,
    what: &str,
    mut done: impl FnMut(&Harness<'_, RickUi>) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !done(harness) {
        harness.step();
        assert!(Instant::now() < deadline, "{what}");
        std::thread::sleep(Duration::from_millis(5));
    }
    for _ in 0..3 {
        harness.step();
    }
}

/// The centre of the list row named `name`.
fn row(harness: &Harness<'_, RickUi>, name: &str) -> egui::Pos2 {
    let pane = pane(harness);
    let rect = harness
        .state()
        .pane_rects()
        .into_iter()
        .find(|(p, _)| *p == pane)
        .map_or_else(|| panic!("no pane rect"), |(_, r)| r);
    harness
        .query_all_by_label(name)
        .map(|n| n.rect())
        .find(|r| rect.contains(r.center()))
        .map_or_else(|| panic!("no row {name}"), |r| r.center())
}

fn press_at(harness: &mut Harness<'_, RickUi>, at: egui::Pos2, button: egui::PointerButton) {
    for pressed in [true, false] {
        harness.input_mut().events.push(egui::Event::PointerButton {
            pos: at,
            button,
            pressed,
            modifiers: egui::Modifiers::default(),
        });
    }
}

fn double_click(harness: &mut Harness<'_, RickUi>, at: egui::Pos2) {
    harness.event(egui::Event::PointerMoved(at));
    harness.step();
    press_at(harness, at, egui::PointerButton::Primary);
    press_at(harness, at, egui::PointerButton::Primary);
    harness.step();
    harness.step();
}

fn right_click(harness: &mut Harness<'_, RickUi>, at: egui::Pos2) {
    harness.event(egui::Event::PointerMoved(at));
    harness.step();
    press_at(harness, at, egui::PointerButton::Secondary);
    for _ in 0..4 {
        harness.step();
    }
}

fn select(harness: &mut Harness<'_, RickUi>, name: &str) {
    let id = tab(harness)
        .1
        .rows()
        .find(|e| e.name == name)
        .map_or_else(|| panic!("no entry {name}"), |e| e.id);
    let pane = pane(harness);
    harness.state_mut().app_mut().handle(
        app::Command::Select {
            pane,
            change: SelectionChange::Set(vec![id]),
        },
        &mut Vec::new(),
    );
    harness.step();
}

/// A file in `dir` with the execute bit.
fn executable(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, bytes).unwrap_or_else(|e| panic!("{e}"));
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap_or_else(|e| panic!("{e}"));
    path
}

/// A double-click on a binary starts it, in its folder and without a
/// question; on an executable script the question appears with Cancel,
/// Edit and Run, and Run starts it in a terminal.
#[test]
fn a_double_click_starts_a_binary_and_asks_about_a_script() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let bin = dir.path().join(".local").join("bin");
    fs::create_dir_all(&bin).unwrap_or_else(|e| panic!("{e}"));
    let tool = executable(&bin, "tool", b"\x7fELF\x02\x01\x01\0\0\0\0\0");
    let script = executable(&bin, "deploy.sh", b"#!/bin/sh\necho deployed\n");
    let mut w = window(&bin);
    let fake = Arc::clone(&w.fake);
    let harness = &mut w.harness;

    let at = row(harness, "tool");
    double_click(harness, at);
    step_until(harness, "the binary is started", |_| !fake.ran().is_empty());
    assert_eq!(fake.ran(), [(tool, Vec::new(), bin.clone())]);
    assert!(harness.query_by_label("Run script?").is_none());
    assert!(fake.opened().is_empty(), "not handed to the system");

    let at = row(harness, "deploy.sh");
    double_click(harness, at);
    step_until(harness, "the question", |h| {
        h.query_by_label("Run script?").is_some()
    });
    for button in ["Cancel", "Edit", "Run"] {
        harness.get_by_role_and_label(Role::Button, button);
    }
    assert_eq!(fake.ran().len(), 1, "nothing runs before the answer");
    harness.get_by_role_and_label(Role::Button, "Run").click();
    step_until(harness, "the script runs in a terminal", |_| {
        !fake.terminals().is_empty()
    });
    assert_eq!(
        fake.terminals(),
        [(bin, Some(vec![script.to_string_lossy().into_owned()]))]
    );
    assert!(harness.query_by_label("Run script?").is_none());
}

/// F4 on a text file opens the editor the setting names; *Open in editor*
/// is in a file's context menu and not in a folder's; and the editor is a
/// field of the Behaviour page that shows what an empty one means.
#[test]
fn f4_opens_the_configured_editor_and_the_menu_offers_it_for_files() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("notes.txt"), "notes").unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir(dir.path().join("folder")).unwrap_or_else(|e| panic!("{e}"));
    let notes = dir.path().join("notes.txt");
    let mut w = window(dir.path());
    let fake = Arc::clone(&w.fake);
    let harness = &mut w.harness;

    // Nothing named: the system's default text editor.
    select(harness, "notes.txt");
    harness.key_press(egui::Key::F4);
    step_until(harness, "the system's editor", |_| {
        !fake.edited().is_empty()
    });
    assert_eq!(fake.edited(), [vec![notes.clone()]]);

    // The setting is a field of the Behaviour page.
    harness.state_mut().open_settings();
    for _ in 0..4 {
        harness.step();
    }
    harness
        .get_by_role_and_label(Role::RadioButton, "Behaviour")
        .click();
    for _ in 0..4 {
        harness.step();
    }
    harness.get_by_label("Opening");
    harness.get_by_label("Editor");
    // Its field is the text field of the Editor row; what is typed there
    // is the setting.
    assert_eq!(harness.state().app().model().settings.editor, "");
    let field = |harness: &Harness<'_, RickUi>| {
        let label = harness.get_by_label("Editor").rect();
        harness
            .get_all_by_role(Role::TextInput)
            .map(|node| node.rect())
            .find(|rect| {
                rect.left() > label.right() && (rect.center().y - label.center().y).abs() < 24.0
            })
            .map_or_else(
                || panic!("no field in the Editor row"),
                |rect| rect.center(),
            )
    };
    let at = field(harness);
    harness.event(egui::Event::PointerMoved(at));
    harness.step();
    press_at(harness, at, egui::PointerButton::Primary);
    for _ in 0..3 {
        harness.step();
    }
    harness.event(egui::Event::Text("code --reuse-window".into()));
    for _ in 0..3 {
        harness.step();
    }
    assert_eq!(
        harness.state().app().model().settings.editor,
        "code --reuse-window"
    );
    harness.get_by_role_and_label(Role::Button, "Close").click();
    for _ in 0..4 {
        harness.step();
    }
    assert!(!harness.state().settings_open());
    settle(harness);

    select(harness, "notes.txt");
    harness.key_press(egui::Key::F4);
    step_until(harness, "the configured editor", |_| !fake.ran().is_empty());
    assert_eq!(
        fake.ran(),
        [(
            PathBuf::from("code"),
            vec!["--reuse-window".into(), notes.into_os_string()],
            dir.path().to_path_buf(),
        )]
    );

    // In the menu of a file, and not of a folder.
    let at = row(harness, "notes.txt");
    right_click(harness, at);
    harness.get_by_label("Open in editor");
    harness.key_press(egui::Key::Escape);
    harness.step();
    harness.step();
    let at = row(harness, "folder");
    right_click(harness, at);
    harness.get_by_label("Open in new tab");
    assert!(harness.query_by_label("Open in editor").is_none());
}

/// `~/.local/bin` typed into the path bar navigates to it; a path with a
/// variable does too, typed with braces or pasted, and pasted text goes
/// into the field expanded.
#[test]
fn the_path_bar_expands_the_home_folder_and_variables() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let mut w = window(dir.path());
    let home = w.fake.home_dir();
    let bin = home.join(".local").join("bin");
    fs::create_dir_all(&bin).unwrap_or_else(|e| panic!("{e}"));
    let harness = &mut w.harness;
    let location = |h: &Harness<'_, RickUi>| tab(h).1.location.clone();
    let edit = |harness: &mut Harness<'static, RickUi>| {
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::L);
        harness.step();
        harness.step();
        assert!(harness.state().path_text(tab(harness).0).is_some());
    };

    edit(harness);
    harness.event(egui::Event::Text("~/.local/bin".into()));
    harness.step();
    harness.key_press(egui::Key::Enter);
    settle(harness);
    assert_eq!(location(harness), local(&bin));

    // A variable of this process: where the test crate is, which Cargo
    // names for every test.
    let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") else {
        panic!("CARGO_MANIFEST_DIR is not set");
    };
    let manifest = PathBuf::from(manifest);
    edit(harness);
    harness.event(egui::Event::Text("${CARGO_MANIFEST_DIR}/src".into()));
    harness.step();
    harness.key_press(egui::Key::Enter);
    settle(harness);
    assert_eq!(location(harness), local(&manifest.join("src")));

    // Pasted: the field shows the path expanded, and Enter goes there.
    edit(harness);
    harness.event(egui::Event::Paste("$CARGO_MANIFEST_DIR/tests".into()));
    harness.step();
    harness.step();
    let shown = harness
        .state()
        .path_text(tab(harness).0)
        .map_or_default(str::to_owned);
    assert_eq!(shown, manifest.join("tests").to_string_lossy());
    harness.key_press(egui::Key::Enter);
    settle(harness);
    assert_eq!(location(harness), local(&manifest.join("tests")));
}
