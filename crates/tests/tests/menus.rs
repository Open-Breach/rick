//! Acceptance (issue 60, context menu): the file menu's groups in the
//! spec's order with Open, Edit, Change and Info, the *Copy as* submenu,
//! the empty area's menu with Select all, Refresh and Folder properties,
//! and the 260 px card.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use platform::{ConfigDir, Fake};
use tests::local;
use ui_egui::{EguiHost, RickUi};

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
        .with_size(egui::vec2(1200.0, 700.0))
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

/// Steps frames until `done` holds, within `limit`.
fn step_until(
    harness: &mut Harness<'_, RickUi>,
    limit: Duration,
    mut done: impl FnMut(&Harness<'_, RickUi>) -> bool,
) -> bool {
    let deadline = Instant::now() + limit;
    while !done(harness) {
        if Instant::now() > deadline {
            return false;
        }
        harness.step();
        std::thread::sleep(Duration::from_millis(5));
    }
    true
}

fn click_at(harness: &mut Harness<'_, RickUi>, at: egui::Pos2, button: egui::PointerButton) {
    harness.event(egui::Event::PointerMoved(at));
    harness.step();
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos: at,
            button,
            pressed,
            modifiers: egui::Modifiers::default(),
        });
        harness.step();
    }
    harness.step();
    harness.step();
}

fn tree() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("notes.md"), "# Day one\n").unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("plan.txt"), "plan").unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir(dir.path().join("sub")).unwrap_or_else(|e| panic!("{e}"));
    dir
}

/// The vertical centre of the one widget named `label`.
fn y_of(harness: &Harness<'_, RickUi>, label: &str) -> f32 {
    harness.get_by_label(label).rect().center().y
}

#[test]
fn a_files_menu_has_the_groups_in_the_specs_order_on_a_260_px_card() {
    let dir = tree();
    let mut harness = harness(dir.path());
    let row = harness.get_by_label("notes.md").rect().center();
    click_at(&mut harness, row, egui::PointerButton::Secondary);
    // Open, Edit with its Copy as submenu, Change with the destructive
    // pair, Info: top to bottom.
    let order = [
        "Open",
        "Cut",
        "Copy",
        "Copy as ⏵",
        "Rename",
        "Move to trash",
        "Delete permanently",
        "Properties",
    ];
    let ys: Vec<f32> = order.iter().map(|l| y_of(&harness, l)).collect();
    assert!(ys.windows(2).all(|w| w[0] < w[1]), "{ys:?}");
    // 30 px entries on a card 260 px wide (the entries fill it inside its
    // 7 px padding).
    let properties = harness.get_by_label("Properties").rect();
    assert!(
        (properties.height() - 30.0).abs() < 1.0,
        "{}",
        properties.height()
    );
    assert!(
        (properties.width() - 246.0).abs() < 2.0,
        "{}",
        properties.width()
    );
    // The entries' shortcuts are there, read with the key map's spelling.
    harness.get_by_label("Copy as ⏵").hover();
    harness.step();
    harness.step();
    harness.get_by_label_contains("Path and name");
    // Escape closes the menu.
    harness.key_press(egui::Key::Escape);
    harness.step();
    harness.step();
    assert!(harness.query_by_label("Delete permanently").is_none());
}

#[test]
fn the_empty_areas_menu_selects_all_refreshes_and_opens_the_folders_properties() {
    let dir = tree();
    let mut harness = harness(dir.path());
    let pane = harness.state().app().model().focused;
    let tab = harness
        .state()
        .app()
        .model()
        .active_tab(pane)
        .map_or_else(|| panic!("no tab"), |(id, _)| id);
    let toolbar = harness
        .state()
        .toolbar_row(tab)
        .unwrap_or_else(|| panic!("no toolbar"));
    let last = harness.get_by_label("sub").rect();
    let empty = egui::pos2(toolbar.center().x, last.bottom() + 120.0);
    click_at(&mut harness, empty, egui::PointerButton::Secondary);
    for label in [
        "New folder",
        "New file",
        "Select all",
        "Refresh",
        "Folder properties",
    ] {
        harness.get_by_label(label);
    }
    assert!(harness.query_by_label("Rename").is_none(), "a file's entry");
    let select_all = harness.get_by_label("Select all").rect().center();
    click_at(&mut harness, select_all, egui::PointerButton::Primary);
    settle(&mut harness);
    assert_eq!(
        harness
            .state()
            .app()
            .model()
            .active_tab(pane)
            .map(|(_, t)| t.selection.len()),
        Some(3)
    );
    // Folder properties: the dialogue of the folder itself.
    click_at(&mut harness, empty, egui::PointerButton::Secondary);
    let properties = harness.get_by_label("Folder properties").rect().center();
    click_at(&mut harness, properties, egui::PointerButton::Primary);
    let name = dir
        .path()
        .file_name()
        .map_or_default(|n| n.to_string_lossy().into_owned());
    assert!(
        step_until(&mut harness, Duration::from_secs(5), |h| {
            h.state()
                .properties()
                .info()
                .is_some_and(|info| info.entry.name == name)
        }),
        "the folder's Properties dialogue"
    );
}
