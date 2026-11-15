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

/// A shell with the Git extension on `start`, in a window of `size`.
fn git_harness(start: &Path, size: egui::Vec2) -> Harness<'static, RickUi> {
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: vec![Arc::new(ext_git::GitExtension::new())],
        config_dir: Some(fake.config_dir()),
        start: Some(local(start)),
    });
    let mut harness = Harness::builder()
        .with_size(size)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    harness
}

/// A Git working tree with one file, `notes.md`.
fn repository() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let output = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(dir.path())
        .output()
        .unwrap_or_else(|e| panic!("git: {e}"));
    assert!(
        output.status.success(),
        "git init: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::write(dir.path().join("notes.md"), "# Day one\n").unwrap_or_else(|e| panic!("{e}"));
    dir
}

/// The room a menu card has: the window, inside the frame Rick draws
/// around it, less 8 px at its top and at its bottom.
fn room(harness: &Harness<'_, RickUi>) -> egui::Rect {
    harness
        .state()
        .framed_content()
        .unwrap_or_else(|| harness.ctx.content_rect())
        .shrink2(egui::vec2(0.0, 8.0))
}

/// The rectangle of the one entry named `label`, wherever its card has
/// scrolled it to.
fn entry(harness: &Harness<'_, RickUi>, label: &str) -> egui::Rect {
    harness.get_by_label(label).rect()
}

/// Whether the entry named `label` is shown whole in `room`.
fn shown(harness: &Harness<'_, RickUi>, label: &str, room: egui::Rect) -> bool {
    let rect = entry(harness, label);
    rect.top() >= room.top() && rect.bottom() <= room.bottom()
}

/// The entries of the submenu open beside the entry at `row`: the menu
/// entries that start just right of its card.
fn submenu_entries(harness: &Harness<'_, RickUi>, row: egui::Rect) -> Vec<egui::Rect> {
    harness
        .query_all_by_role(egui::accesskit::Role::Button)
        .map(|n| n.rect())
        .filter(|r| {
            r.left() > row.right()
                && r.left() < row.right() + 40.0
                && (r.height() - 30.0).abs() < 1.0
        })
        .collect()
}

/// Rolls the wheel by `dy` points with the pointer at `at`.
fn wheel(harness: &mut Harness<'_, RickUi>, at: egui::Pos2, dy: f32) {
    harness.event(egui::Event::PointerMoved(at));
    harness.step();
    harness.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, dy),
        modifiers: egui::Modifiers::default(),
        phase: egui::TouchPhase::Move,
    });
    for _ in 0..12 {
        harness.step();
    }
}

/// The entries of a file's menu in a Git folder, top to bottom, as far
/// as this test names them.
const ENTRIES: [&str; 9] = [
    "Open",
    "Cut",
    "Copy",
    "Copy as ⏵",
    "Rename",
    "Move to trash",
    "Delete permanently",
    "Git ⏵",
    "Properties",
];

/// Acceptance (issue 219, item 4; SPEC.md *A menu that does not fit*): in
/// a window 400 px tall the context menu of a file in a Git folder has
/// more entries than room. Its card is as tall as fits, 8 px clear of the
/// window's top and bottom edges, and the entries scroll: every one of
/// them can be brought into the card. The *Copy as* submenu is shown
/// whole, in the same room.
#[test]
fn a_context_menu_taller_than_the_window_scrolls_8_px_clear_of_its_edges() {
    let dir = repository();
    let mut harness = git_harness(dir.path(), egui::vec2(900.0, 400.0));
    let room = room(&harness);
    let row = harness.get_by_label("notes.md").rect().center();
    click_at(&mut harness, row, egui::PointerButton::Secondary);
    harness.step();

    // More entries than fit: the first is shown, the last is not.
    let ys: Vec<f32> = ENTRIES.iter().map(|l| y_of(&harness, l)).collect();
    assert!(ys.windows(2).all(|w| w[0] < w[1]), "{ys:?}");
    let first = entry(&harness, "Open");
    let last = entry(&harness, "Properties");
    assert!(
        last.bottom() - first.top() > room.height(),
        "the menu is taller than its room: {first:?} to {last:?} in {room:?}"
    );
    assert!(shown(&harness, "Open", room));
    assert!(!shown(&harness, "Properties", room), "{last:?} in {room:?}");

    // The card is as tall as fits: its first entry sits under the card's
    // padding and border, 8 px below the window's top edge.
    assert!(
        (first.top() - room.top() - 8.0).abs() < 1.5,
        "the card starts 8 px under the window's edge: {first:?} in {room:?}"
    );

    // The wheel brings the last entry in, and the first leaves; the card
    // ends 8 px over the window's bottom edge.
    let over = egui::pos2(first.center().x, room.center().y);
    wheel(&mut harness, over, -2000.0);
    assert!(shown(&harness, "Properties", room), "scrolled to the end");
    assert!(!shown(&harness, "Open", room), "the first entry left");
    let last = entry(&harness, "Properties");
    assert!(
        (room.bottom() - last.bottom() - 8.0).abs() < 1.5,
        "the card ends 8 px over the window's edge: {last:?} in {room:?}"
    );
    // Every entry between was passed on the way: each can be shown.
    for label in ENTRIES {
        wheel(&mut harness, over, 2000.0);
        let mut found = shown(&harness, label, room);
        for _ in 0..40 {
            if found {
                break;
            }
            wheel(&mut harness, over, -30.0);
            found = shown(&harness, label, room);
        }
        assert!(found, "{label} can be scrolled into the card");
    }

    // The submenu of *Copy as*, the entry shown: all of it in the room.
    let copy_as = entry(&harness, "Copy as ⏵");
    harness.get_by_label("Copy as ⏵").hover();
    for _ in 0..4 {
        harness.step();
    }
    harness.get_by_label_contains("Path and name");
    let entries = submenu_entries(&harness, copy_as);
    assert_eq!(entries.len(), 3, "Name, Path, Path and name: {entries:?}");
    for rect in entries {
        assert!(
            rect.top() >= room.top() && rect.bottom() <= room.bottom(),
            "an entry of Copy as is shown: {rect:?} in {room:?}"
        );
    }

    // Escape closes the menu.
    harness.key_press(egui::Key::Escape);
    harness.step();
    harness.step();
    assert!(harness.query_by_label("Delete permanently").is_none());
}

/// Issue 219, item 4: a submenu taller than its room scrolls as its menu
/// does and keeps the same 8 px, and a menu that fits is where it was —
/// under the pointer, with no scrolling.
#[test]
fn a_submenu_scrolls_too_and_a_menu_that_fits_is_placed_as_before() {
    let dir = repository();

    // A window with room for the whole menu: the card opens at the
    // pointer, as it always did.
    let mut tall = git_harness(dir.path(), egui::vec2(900.0, 900.0));
    let row = tall.get_by_label("notes.md").rect().center();
    click_at(&mut tall, row, egui::PointerButton::Secondary);
    tall.step();
    let first = entry(&tall, "Open");
    assert!(
        (first.top() - row.y - 8.0).abs() < 1.5 && (first.left() - row.x - 8.0).abs() < 1.5,
        "the card hangs from the pointer: {first:?} at {row:?}"
    );
    let whole = room(&tall);
    for label in ENTRIES {
        assert!(shown(&tall, label, whole), "{label} without scrolling");
    }

    // A low window: the Git submenu has more entries than room.
    let mut low = git_harness(dir.path(), egui::vec2(900.0, 260.0));
    let room = room(&low);
    let row = low.get_by_label("notes.md").rect().center();
    click_at(&mut low, row, egui::PointerButton::Secondary);
    low.step();
    let over = egui::pos2(entry(&low, "Open").center().x, room.center().y);
    let mut found = shown(&low, "Git ⏵", room);
    for _ in 0..60 {
        if found {
            break;
        }
        wheel(&mut low, over, -30.0);
        found = shown(&low, "Git ⏵", room);
    }
    assert!(found, "Git can be scrolled into the card");
    let git = entry(&low, "Git ⏵");
    low.get_by_label("Git ⏵").hover();
    for _ in 0..4 {
        low.step();
    }
    let entries = submenu_entries(&low, git);
    assert!(entries.len() > 3, "the Git submenu is open: {entries:?}");
    let top = entries
        .iter()
        .map(egui::Rect::top)
        .fold(f32::INFINITY, f32::min);
    let bottom = entries
        .iter()
        .map(egui::Rect::bottom)
        .fold(f32::NEG_INFINITY, f32::max);
    assert!(
        bottom - top > room.height(),
        "the submenu is taller than its room: {top} to {bottom} in {room:?}"
    );
    assert!(
        (top - room.top() - 8.0).abs() < 1.5,
        "the submenu's card starts 8 px under the window's edge: {top} in {room:?}"
    );
    // The wheel over the submenu brings its last entry in.
    let inside = egui::pos2(entries[0].center().x, room.center().y);
    wheel(&mut low, inside, -2000.0);
    let bottom = submenu_entries(&low, git)
        .iter()
        .map(egui::Rect::bottom)
        .fold(f32::NEG_INFINITY, f32::max);
    assert!(
        (room.bottom() - bottom - 8.0).abs() < 1.5,
        "the submenu's card ends 8 px over the window's edge: {bottom} in {room:?}"
    );
}

/// A shell with the plugins that add entries to a file's menu — Git,
/// Compress, Compare, Rename and the terminal — on `start`, in a window
/// of `size`: a menu with more entries than the Git one alone has.
fn busy_harness(start: &Path, size: egui::Vec2) -> Harness<'static, RickUi> {
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: vec![
            Arc::new(ext_git::GitExtension::new()),
            Arc::new(ext_action_compress::CompressExtension::new()),
            Arc::new(ext_action_compare::CompareExtension::new()),
            Arc::new(ext_action_rename::RenameExtension::new()),
            Arc::new(ext_action_terminal::TerminalExtension::new()),
        ],
        config_dir: Some(fake.config_dir()),
        start: Some(local(start)),
    });
    let mut harness = Harness::builder()
        .with_size(size)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    harness
}

/// A menu takes the room the window has (SPEC.md *A menu that does not
/// fit*: "a menu card taller than the room it has is as tall as fits").
/// With five plugins a file's menu is some 680 px tall. Its card was held
/// to the 400 px an area is offered before its content is known: it
/// scrolled in a window that had room for all of it, and in a lower
/// window it was smaller than the room it had.
#[test]
fn a_menu_with_many_entries_takes_the_room_the_window_has() {
    let dir = repository();

    // Room for the whole menu: all of it is shown, hanging from the
    // pointer, and nothing scrolls.
    let mut tall = busy_harness(dir.path(), egui::vec2(900.0, 1000.0));
    let whole = room(&tall);
    let row = tall.get_by_label("notes.md").rect().center();
    click_at(&mut tall, row, egui::PointerButton::Secondary);
    tall.step();
    let first = entry(&tall, "Open");
    let last = entry(&tall, "Properties");
    assert!(
        last.bottom() - first.top() > 420.0,
        "a menu taller than 400 px: {first:?} to {last:?}"
    );
    assert!(
        (first.top() - row.y - 8.0).abs() < 1.5,
        "the card hangs from the pointer: {first:?} at {row:?}"
    );
    assert!(shown(&tall, "Open", whole) && shown(&tall, "Properties", whole));
    wheel(
        &mut tall,
        egui::pos2(first.center().x, first.bottom() + 60.0),
        -2000.0,
    );
    assert_eq!(
        entry(&tall, "Open"),
        first,
        "the whole menu is in its card: the wheel moves nothing"
    );

    // Less room than the menu needs, and more than 400 px of it: the
    // card takes all of it, 8 px clear of the window's edges.
    let mut low = busy_harness(dir.path(), egui::vec2(900.0, 600.0));
    let room = room(&low);
    assert!(room.height() > 500.0, "{room:?}");
    let row = low.get_by_label("notes.md").rect().center();
    click_at(&mut low, row, egui::PointerButton::Secondary);
    low.step();
    let first = entry(&low, "Open");
    assert!(
        (first.top() - room.top() - 8.0).abs() < 1.5,
        "the card starts 8 px under the window's edge: {first:?} in {room:?}"
    );
    wheel(
        &mut low,
        egui::pos2(first.center().x, room.center().y),
        -2000.0,
    );
    let last = entry(&low, "Properties");
    assert!(
        (room.bottom() - last.bottom() - 8.0).abs() < 1.5,
        "the card ends 8 px over the window's edge: {last:?} in {room:?}"
    );
}

/// Issue 219, item 4: the application menu is a card of the same kind. In
/// a window too low for its entries it is as tall as fits, 8 px clear of
/// the window's edges, and scrolls to *About Rick*.
#[test]
fn the_application_menu_scrolls_in_a_low_window() {
    let dir = tree();
    let mut harness = git_harness(dir.path(), egui::vec2(900.0, 200.0));
    let room = room(&harness);
    harness.get_by_label("Menu").click();
    for _ in 0..4 {
        harness.step();
    }
    let first = entry(&harness, "New window");
    let last = entry(&harness, "About Rick");
    assert!(
        last.bottom() - first.top() > room.height(),
        "the menu is taller than its room: {first:?} to {last:?} in {room:?}"
    );
    assert!(
        (first.top() - room.top() - 7.0).abs() < 1.5,
        "the card starts 8 px under the window's edge: {first:?} in {room:?}"
    );
    assert!(!shown(&harness, "About Rick", room));
    wheel(
        &mut harness,
        egui::pos2(first.center().x, room.center().y),
        -2000.0,
    );
    let last = entry(&harness, "About Rick");
    assert!(shown(&harness, "About Rick", room), "{last:?} in {room:?}");
    assert!(
        (room.bottom() - last.bottom() - 7.0).abs() < 1.5,
        "the card ends 8 px over the window's edge: {last:?} in {room:?}"
    );
}
