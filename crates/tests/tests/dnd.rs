//! Acceptance (drag and drop): every internal drop target, the move/copy
//! rule with its modifiers, the ghost with its indicator, spring-loading
//! with the stay-or-return setting, the no-op cases, Escape, drops from
//! outside the process, and Pin/Unpin from the context menu.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use model::{AfterDrag, SettingsChange};
use platform::{ConfigDir, Fake, Trash as _};
use tests::local;
use ui_egui::dnd::{Indicator, Target};
use ui_egui::{EguiHost, RickUi, icons};

/// The spring delay the tests set, at the harness's 50 ms steps: twenty
/// frames of hovering, well past what a drag passing over a row spends.
const SPRING_MS: u64 = 1000;
/// Frames that hover a target past the spring delay.
const SPRING_STEPS: usize = 24;

fn harness(start: &Path) -> (Harness<'static, RickUi>, Arc<Fake>) {
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: Vec::new(),
        config_dir: Some(fake.config_dir()),
        start: local(start),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 800.0))
        .with_step_dt(0.05)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    command(
        &mut harness,
        app::Command::Settings(SettingsChange::SpringDelay(SPRING_MS)),
    );
    (harness, fake)
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

fn command(harness: &mut Harness<'_, RickUi>, command: app::Command) {
    harness
        .state_mut()
        .app_mut()
        .handle(command, &mut Vec::new());
    settle(harness);
}

fn location(harness: &Harness<'_, RickUi>, pane: model::PaneId) -> PathBuf {
    harness
        .state()
        .app()
        .model()
        .active_tab(pane)
        .and_then(|(_, t)| platform::paths::to_path(&t.location.path).ok())
        .unwrap_or_else(|| panic!("no location"))
}

fn tab(harness: &Harness<'_, RickUi>, pane: model::PaneId) -> (model::TabId, model::Tab) {
    harness
        .state()
        .app()
        .model()
        .active_tab(pane)
        .map_or_else(|| panic!("no tab"), |(id, t)| (id, t.clone()))
}

fn selected_names(harness: &Harness<'_, RickUi>, pane: model::PaneId) -> Vec<String> {
    tab(harness, pane)
        .1
        .selected()
        .map(|e| e.name.clone())
        .collect()
}

/// The centre of the one widget labelled `label`.
fn centre_of(harness: &Harness<'_, RickUi>, label: &str) -> egui::Pos2 {
    harness.get_by_label(label).rect().center()
}

/// The centre of the widget labelled `label` inside `pane`, when the label
/// shows in more than one place.
fn centre_in(harness: &Harness<'_, RickUi>, pane: model::PaneId, label: &str) -> egui::Pos2 {
    let rect = rect_of(harness, pane);
    harness
        .query_all_by_label(label)
        .map(|n| n.rect())
        .find(|r| rect.contains(r.center()))
        .map_or_else(|| panic!("no {label} in {pane:?}"), |r| r.center())
}

/// Holds the pointer where it is for the spring delay and lets the app
/// list what it drilled into.
fn hover_for_spring(harness: &mut Harness<'_, RickUi>) {
    for _ in 0..SPRING_STEPS {
        harness.step();
    }
    settle(harness);
}

/// Holds `modifiers` from now on.
fn hold(harness: &mut Harness<'_, RickUi>, modifiers: egui::Modifiers) {
    harness.event(egui::Event::ModifiersChanged(modifiers));
    harness.step();
}

fn press(harness: &mut Harness<'_, RickUi>, at: egui::Pos2, button: egui::PointerButton) {
    harness.event(egui::Event::PointerMoved(at));
    harness.step();
    harness.event(egui::Event::PointerButton {
        pos: at,
        button,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
}

/// Moves the pointer to `to` in small steps, so egui decides it is a drag.
fn move_to(harness: &mut Harness<'_, RickUi>, from: egui::Pos2, to: egui::Pos2) {
    for i in 1..=8u8 {
        let t = f32::from(i) / 8.0;
        harness.event(egui::Event::PointerMoved(from + (to - from) * t));
        harness.step();
    }
    harness.step();
}

fn release(harness: &mut Harness<'_, RickUi>, at: egui::Pos2, button: egui::PointerButton) {
    harness.event(egui::Event::PointerButton {
        pos: at,
        button,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
    harness.step();
    settle(harness);
}

/// A whole drag of the row labelled `label` to `to`, with `modifiers` held.
fn drag(
    harness: &mut Harness<'_, RickUi>,
    label: &str,
    to: egui::Pos2,
    modifiers: egui::Modifiers,
) -> Option<ui_egui::dnd::DragView> {
    let from = centre_of(harness, label);
    hold(harness, modifiers);
    press(harness, from, egui::PointerButton::Primary);
    move_to(harness, from, to);
    let view = harness.state().drag();
    release(harness, to, egui::PointerButton::Primary);
    hold(harness, egui::Modifiers::NONE);
    settle(harness);
    view
}

/// A folder with three files and a `sub` folder holding `child`.
fn tree() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    for name in ["a.txt", "b.txt", "c.txt"] {
        fs::write(dir.path().join(name), name).unwrap_or_else(|e| panic!("{e}"));
    }
    fs::create_dir_all(dir.path().join("sub").join("child")).unwrap_or_else(|e| panic!("{e}"));
    dir
}

/// Two panes side by side, the second showing `right`.
fn two_panes(harness: &mut Harness<'_, RickUi>, right: &Path) -> (model::PaneId, model::PaneId) {
    let pane_a = harness.state().app().model().focused;
    command(
        harness,
        app::Command::Split {
            pane: pane_a,
            dir: model::SplitDir::Right,
        },
    );
    let pane_b = harness
        .state()
        .app()
        .model()
        .panes()
        .map(|(p, _)| p)
        .find(|p| *p != pane_a)
        .unwrap_or_else(|| panic!("no second pane"));
    command(
        harness,
        app::Command::Navigate {
            pane: pane_b,
            to: local(right),
        },
    );
    command(harness, app::Command::FocusPane(pane_a));
    (pane_a, pane_b)
}

fn rect_of(harness: &Harness<'_, RickUi>, pane: model::PaneId) -> egui::Rect {
    harness
        .state()
        .pane_rects()
        .iter()
        .find(|(p, _)| *p == pane)
        .map_or_else(|| panic!("no rect for {pane:?}"), |(_, r)| *r)
}

/// A point in the lower half of a pane's body: empty space when the
/// folder is small.
fn empty_space(harness: &Harness<'_, RickUi>, pane: model::PaneId) -> egui::Pos2 {
    let rect = rect_of(harness, pane);
    egui::pos2(rect.center().x, rect.bottom() - 80.0)
}

#[test]
fn a_file_dragged_onto_a_folder_row_moves_and_the_modifiers_force_the_mode() {
    let dir = tree();
    let (mut harness, _fake) = harness(dir.path());
    let pane = harness.state().app().model().focused;

    // Same volume: a move.
    let to = centre_of(&harness, "sub");
    let view = drag(&mut harness, "a.txt", to, egui::Modifiers::NONE);
    assert_eq!(
        view.as_ref().and_then(|v| v.indicator),
        Some(Indicator::Move),
        "{view:?}"
    );
    assert!(dir.path().join("sub/a.txt").exists());
    assert!(!dir.path().join("a.txt").exists());
    assert_eq!(
        harness.state().app().undo_label().as_deref(),
        Some("Move a.txt to sub")
    );

    // Mod forces a copy.
    let to = centre_of(&harness, "sub");
    let view = drag(&mut harness, "b.txt", to, egui::Modifiers::COMMAND);
    assert_eq!(
        view.as_ref().and_then(|v| v.indicator),
        Some(Indicator::Copy)
    );
    assert!(dir.path().join("sub/b.txt").exists());
    assert!(dir.path().join("b.txt").exists(), "a copy keeps the source");

    // Shift forces a move.
    let to = centre_of(&harness, "sub");
    let view = drag(&mut harness, "c.txt", to, egui::Modifiers::SHIFT);
    assert_eq!(
        view.as_ref().and_then(|v| v.indicator),
        Some(Indicator::Move)
    );
    assert!(dir.path().join("sub/c.txt").exists());
    assert!(!dir.path().join("c.txt").exists());

    // Alt asks: the prompt is up and nothing moved until Move is chosen.
    let to = centre_of(&harness, "sub");
    drag(&mut harness, "b.txt", to, egui::Modifiers::ALT);
    assert!(harness.state().drop_prompt_open(), "the move/copy prompt");
    assert!(
        dir.path().join("b.txt").exists(),
        "no job before the answer"
    );
    assert!(!dir.path().join("sub/b.txt").is_file() || dir.path().join("b.txt").exists());
    harness
        .get_by_label(&format!("{} Copy", icons::PLUS))
        .click();
    harness.step();
    settle(&mut harness);
    assert!(!harness.state().drop_prompt_open());
    assert!(dir.path().join("b.txt").exists());
    assert_eq!(
        fs::read_to_string(dir.path().join("sub/b.txt")).ok(),
        Some("b.txt".into())
    );
    assert_eq!(location(&harness, pane), dir.path(), "the pane stayed");
}

/// Across volumes the default is a copy. The test needs a second volume;
/// `/dev/shm` is one on Linux runners, and the test says so when it finds
/// none rather than assert on the same device twice.
#[test]
fn a_file_dragged_onto_another_volume_copies() {
    let dir = tree();
    let Some(other) = other_volume() else {
        eprintln!("no second volume to test with; skipped");
        return;
    };
    let (mut harness, _fake) = harness(dir.path());
    let (pane_a, pane_b) = two_panes(&mut harness, other.path());
    let device = |harness: &Harness<'_, RickUi>, pane| {
        tab(harness, pane)
            .1
            .listing
            .entries()
            .first()
            .map(|e| e.id.key >> 64)
    };
    fs::write(other.path().join("marker.txt"), "m").unwrap_or_else(|e| panic!("{e}"));
    command(&mut harness, app::Command::Refresh(pane_b));
    if device(&harness, pane_a) == device(&harness, pane_b) {
        eprintln!("the second volume reports the same device; skipped");
        return;
    }
    let to = empty_space(&harness, pane_b);
    let view = drag(&mut harness, "a.txt", to, egui::Modifiers::NONE);
    assert_eq!(
        view.as_ref().and_then(|v| v.indicator),
        Some(Indicator::Copy)
    );
    assert!(other.path().join("a.txt").exists());
    assert!(dir.path().join("a.txt").exists(), "copied, not moved");
    // Shift still forces a move across volumes.
    let to = empty_space(&harness, pane_b);
    drag(&mut harness, "b.txt", to, egui::Modifiers::SHIFT);
    assert!(other.path().join("b.txt").exists());
    assert!(!dir.path().join("b.txt").exists());
}

/// A writable folder on another volume than the system temp dir, if there
/// is one.
fn other_volume() -> Option<tempfile::TempDir> {
    for candidate in ["/dev/shm", "/run/user"] {
        if let Ok(dir) = tempfile::Builder::new()
            .prefix("rick-dnd-")
            .tempdir_in(candidate)
        {
            return Some(dir);
        }
    }
    None
}

#[test]
fn drops_onto_a_breadcrumb_a_tab_header_and_another_panes_space_move_the_file() {
    let dir = tree();
    let other = dir.path().join("other");
    fs::create_dir(&other).unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("sub/child/deep.txt"), "d").unwrap_or_else(|e| panic!("{e}"));
    let (mut harness, _fake) = harness(dir.path());
    let pane_a = harness.state().app().model().focused;

    // Breadcrumb: from sub/child onto the `sub` segment.
    command(
        &mut harness,
        app::Command::Navigate {
            pane: pane_a,
            to: local(&dir.path().join("sub/child")),
        },
    );
    let to = centre_of(&harness, "sub");
    drag(&mut harness, "deep.txt", to, egui::Modifiers::NONE);
    assert!(
        dir.path().join("sub/deep.txt").exists(),
        "moved to the ancestor"
    );
    assert!(!dir.path().join("sub/child/deep.txt").exists());

    // Tab header: a second tab at `other`, a file dropped on its header.
    command(
        &mut harness,
        app::Command::Navigate {
            pane: pane_a,
            to: local(dir.path()),
        },
    );
    let (first_tab, _) = tab(&harness, pane_a);
    command(&mut harness, app::Command::NewTab(local(&other)));
    let (other_tab, _) = tab(&harness, pane_a);
    command(&mut harness, app::Command::ActivateTab(first_tab));
    let header = harness
        .state()
        .tab_header_rect(other_tab)
        .unwrap_or_else(|| panic!("no header for the other tab"));
    drag(
        &mut harness,
        "a.txt",
        header.center(),
        egui::Modifiers::NONE,
    );
    assert!(other.join("a.txt").exists(), "moved into the tab's folder");
    assert!(!dir.path().join("a.txt").exists());
    assert_eq!(
        tab(&harness, pane_a).0,
        first_tab,
        "a drop on a header does not switch tabs"
    );

    // Another pane's empty space.
    let (_, pane_b) = two_panes(&mut harness, &other);
    let to = empty_space(&harness, pane_b);
    drag(&mut harness, "b.txt", to, egui::Modifiers::NONE);
    assert!(other.join("b.txt").exists());
    assert!(!dir.path().join("b.txt").exists());
}

#[test]
fn drops_onto_pinned_pin_and_a_pinned_file_opens_its_folder_selected() {
    let dir = tree();
    let (mut harness, _fake) = harness(dir.path());
    let pane = harness.state().app().model().focused;
    let pinned = |harness: &Harness<'_, RickUi>| -> Vec<String> {
        harness
            .state()
            .app()
            .model()
            .settings
            .pinned
            .iter()
            .map(|l| l.path.to_string())
            .collect()
    };

    // The header pins at the end; nothing moves.
    let to = centre_of(&harness, "Pinned");
    let view = drag(&mut harness, "a.txt", to, egui::Modifiers::NONE);
    assert_eq!(
        view.as_ref().and_then(|v| v.indicator),
        Some(Indicator::Pin)
    );
    assert_eq!(
        pinned(&harness),
        [dir.path().join("a.txt").display().to_string()]
    );
    assert!(dir.path().join("a.txt").exists());
    assert_eq!(
        harness.state().app().undo_label().as_deref(),
        Some("Pin a.txt"),
        "a pin, not a transfer"
    );
    harness.get_by_label(&format!("{} a.txt", icons::PIN));

    // A folder pins too.
    let to = centre_of(&harness, "Pinned");
    drag(&mut harness, "sub", to, egui::Modifiers::NONE);
    assert_eq!(pinned(&harness).len(), 2);
    assert!(dir.path().join("sub").is_dir());
    assert!(dir.path().join("a.txt").exists());

    // Onto an existing entry: inserted before it.
    let to = centre_of(&harness, &format!("{} a.txt", icons::PIN));
    drag(&mut harness, "b.txt", to, egui::Modifiers::NONE);
    assert_eq!(
        pinned(&harness),
        [
            dir.path().join("b.txt").display().to_string(),
            dir.path().join("a.txt").display().to_string(),
            dir.path().join("sub").display().to_string(),
        ]
    );
    assert!(dir.path().join("b.txt").exists());

    // Clicking the pinned file from elsewhere: its folder, the file selected.
    command(
        &mut harness,
        app::Command::Navigate {
            pane,
            to: local(&dir.path().join("sub")),
        },
    );
    harness
        .get_by_label(&format!("{} a.txt", icons::PIN))
        .click();
    harness.step();
    settle(&mut harness);
    harness.step();
    settle(&mut harness);
    assert_eq!(location(&harness, pane), dir.path());
    assert_eq!(selected_names(&harness, pane), ["a.txt"]);

    // The pinned folder opens in the pane.
    harness.get_by_label(&format!("{} sub", icons::PIN)).click();
    harness.step();
    settle(&mut harness);
    assert_eq!(location(&harness, pane), dir.path().join("sub"));
}

#[test]
fn a_drop_onto_trash_trashes_and_undo_restores() {
    let dir = tree();
    let (mut harness, fake) = harness(dir.path());
    let to = centre_of(&harness, "Trash");
    let view = drag(&mut harness, "a.txt", to, egui::Modifiers::NONE);
    assert_eq!(
        view.as_ref().and_then(|v| v.indicator),
        Some(Indicator::Trash)
    );
    assert!(!dir.path().join("a.txt").exists());
    assert_eq!(fake.list().map(|l| l.len()).ok(), Some(1));
    assert_eq!(
        harness.state().app().undo_label().as_deref(),
        Some("Move to trash a.txt")
    );
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.step();
    settle(&mut harness);
    assert!(dir.path().join("a.txt").exists(), "undo restored it");
    assert_eq!(fake.list().map(|l| l.len()).ok(), Some(0));
}

#[test]
fn no_op_drops_do_nothing_and_do_not_highlight() {
    let dir = tree();
    let (mut harness, _fake) = harness(dir.path());
    let (pane_a, pane_b) = two_panes(&mut harness, &dir.path().join("sub"));
    let before: Vec<String> = fs::read_dir(dir.path())
        .unwrap_or_else(|e| panic!("{e}"))
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();

    // Onto the source folder's own empty space, and a file onto a file.
    let from = centre_of(&harness, "a.txt");
    let space = empty_space(&harness, pane_a);
    press(&mut harness, from, egui::PointerButton::Primary);
    move_to(&mut harness, from, space);
    let view = harness.state().drag().unwrap_or_else(|| panic!("a drag"));
    assert_eq!(view.target, Some(Target::NoOp), "{view:?}");
    assert_eq!(view.indicator, None);
    let b = centre_of(&harness, "b.txt");
    move_to(&mut harness, space, b);
    let view = harness.state().drag().unwrap_or_else(|| panic!("a drag"));
    assert_eq!(view.target, Some(Target::NoOp), "file onto file: {view:?}");
    release(&mut harness, b, egui::PointerButton::Primary);

    // A folder onto itself.
    let from = centre_in(&harness, pane_a, "sub");
    press(&mut harness, from, egui::PointerButton::Primary);
    move_to(&mut harness, from, from + egui::vec2(30.0, 0.0));
    let view = harness.state().drag().unwrap_or_else(|| panic!("a drag"));
    assert_eq!(view.target, Some(Target::NoOp), "onto itself: {view:?}");
    // And into its own child, shown in the other pane.
    let child = centre_in(&harness, pane_b, "child");
    move_to(&mut harness, from + egui::vec2(30.0, 0.0), child);
    let view = harness.state().drag().unwrap_or_else(|| panic!("a drag"));
    assert_eq!(view.target, Some(Target::NoOp), "into its child: {view:?}");
    release(&mut harness, child, egui::PointerButton::Primary);

    let after: Vec<String> = fs::read_dir(dir.path())
        .unwrap_or_else(|e| panic!("{e}"))
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(before, after, "nothing moved");
    assert!(dir.path().join("sub/child").is_dir());
    assert_eq!(
        harness.state().app().undo_label(),
        None,
        "no command was sent"
    );
    assert!(harness.state().toasts().is_empty(), "and no error");
}

#[test]
fn the_ghost_counts_the_items_and_the_indicator_follows_target_and_keys() {
    let dir = tree();
    let (mut harness, _fake) = harness(dir.path());
    let pane = harness.state().app().model().focused;
    let ids: Vec<types::EntryId> = tab(&harness, pane)
        .1
        .listing
        .rows()
        .filter(|e| e.kind == types::EntryKind::File)
        .map(|e| e.id)
        .collect();
    command(
        &mut harness,
        app::Command::Select {
            pane,
            change: model::SelectionChange::Set(ids),
        },
    );
    let from = centre_of(&harness, "a.txt");
    press(&mut harness, from, egui::PointerButton::Primary);
    let sub = centre_of(&harness, "sub");
    move_to(&mut harness, from, sub);
    let view = harness.state().drag().unwrap_or_else(|| panic!("a drag"));
    assert_eq!(view.count, 3, "the badge says three");
    assert_eq!(view.indicator, Some(Indicator::Move));

    hold(&mut harness, egui::Modifiers::COMMAND);
    harness.step();
    let view = harness.state().drag().unwrap_or_else(|| panic!("a drag"));
    assert_eq!(view.indicator, Some(Indicator::Copy), "Mod mid-drag");
    hold(&mut harness, egui::Modifiers::NONE);

    let pinned = centre_of(&harness, "Pinned");
    move_to(&mut harness, sub, pinned);
    let view = harness.state().drag().unwrap_or_else(|| panic!("a drag"));
    assert_eq!(view.indicator, Some(Indicator::Pin), "over Pinned");

    // Escape: the ghost goes and the model is unchanged.
    harness.state_mut().clear_dispatched();
    harness.key_press(egui::Key::Escape);
    harness.step();
    assert!(harness.state().drag().is_none(), "Escape cancels");
    assert!(
        harness.state().dispatched().is_empty(),
        "the drag took the key: {:?}",
        harness.state().dispatched()
    );
    release(&mut harness, pinned, egui::PointerButton::Primary);
    assert_eq!(
        harness.state().app().model().settings.pinned,
        [] as [types::Location; 0]
    );
    assert!(dir.path().join("a.txt").exists());
    assert_eq!(
        selected_names(&harness, pane).len(),
        3,
        "the selection stays"
    );
}

/// Sixty files and a folder that sorts last, so the pane scrolls and the
/// folder is reachable at the bottom.
fn long_tree() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let tree = dir.path().join("tree");
    fs::create_dir_all(tree.join("zz-sub").join("child")).unwrap_or_else(|e| panic!("{e}"));
    for i in 0..60 {
        fs::write(tree.join(format!("f{i:03}.txt")), "x").unwrap_or_else(|e| panic!("{e}"));
    }
    fs::write(dir.path().join("payload.txt"), "p").unwrap_or_else(|e| panic!("{e}"));
    dir
}

fn scroll(harness: &mut Harness<'_, RickUi>, at: egui::Pos2, dy: f32) {
    harness.event(egui::Event::PointerMoved(at));
    harness.step();
    harness.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, dy),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
    harness.step();
}

#[test]
fn spring_loading_drills_in_and_one_back_restores_scroll_and_selection() {
    let dir = long_tree();
    let tree = dir.path().join("tree");
    let (mut harness, _fake) = harness(dir.path());
    command(
        &mut harness,
        app::Command::Settings(SettingsChange::FoldersFirst(false)),
    );
    let (pane_a, pane_b) = two_panes(&mut harness, &tree);
    let (tab_b, _) = tab(&harness, pane_b);

    // Pane B: scrolled to the bottom, a file selected.
    let rect_b = rect_of(&harness, pane_b);
    scroll(&mut harness, rect_b.center(), -5000.0);
    let offset = harness.state().scroll_offset(tab_b);
    assert!(offset > 100.0, "pane B scrolled: {offset}");
    let f059 = tab(&harness, pane_b)
        .1
        .listing
        .rows()
        .find(|e| e.name == "f059.txt")
        .map_or_else(|| panic!("f059"), |e| e.id);
    command(
        &mut harness,
        app::Command::Select {
            pane: pane_b,
            change: model::SelectionChange::Set(vec![f059]),
        },
    );
    command(&mut harness, app::Command::FocusPane(pane_a));
    harness.step();
    let steps_before = tab(&harness, pane_b).1.history.back.len();

    // Hold payload.txt over zz-sub for the delay: B drills in, first step.
    let from = centre_of(&harness, "payload.txt");
    press(&mut harness, from, egui::PointerButton::Primary);
    let sub = centre_of(&harness, "zz-sub");
    move_to(&mut harness, from, sub);
    hover_for_spring(&mut harness);
    assert_eq!(location(&harness, pane_b), tree.join("zz-sub"));
    let (_, state) = tab(&harness, pane_b);
    assert_eq!(
        state.history.back.len(),
        steps_before + 1,
        "one step for the drag"
    );
    let step = state.history.back.last().unwrap_or_else(|| panic!("step"));
    assert_eq!(
        step.selection.iter().map(|e| e.id).collect::<Vec<_>>(),
        [f059]
    );
    assert_eq!(harness.state().drag().map(|d| d.sprung), Some(vec![pane_b]));

    // Hold over child: drills again, no second step.
    let child = centre_in(&harness, pane_b, "child");
    harness.event(egui::Event::PointerMoved(child));
    harness.step();
    hover_for_spring(&mut harness);
    assert_eq!(location(&harness, pane_b), tree.join("zz-sub/child"));
    assert_eq!(tab(&harness, pane_b).1.history.back.len(), steps_before + 1);

    // Drop on the child's empty space: moved there, the pane stays.
    let to = empty_space(&harness, pane_b);
    harness.event(egui::Event::PointerMoved(to));
    harness.step();
    release(&mut harness, to, egui::PointerButton::Primary);
    assert!(tree.join("zz-sub/child/payload.txt").exists());
    assert!(!dir.path().join("payload.txt").exists());
    assert_eq!(
        location(&harness, pane_b),
        tree.join("zz-sub/child"),
        "stay"
    );

    // One Back: the start, with the selection and the scroll offset.
    command(&mut harness, app::Command::Back(pane_b));
    for _ in 0..4 {
        harness.step();
    }
    settle(&mut harness);
    assert_eq!(location(&harness, pane_b), tree);
    assert_eq!(selected_names(&harness, pane_b), ["f059.txt"]);
    let restored = harness.state().scroll_offset(tab_b);
    assert!(
        (restored - offset).abs() < 1.0,
        "scroll offset {restored} vs {offset}"
    );
    assert_eq!(tab(&harness, pane_b).1.history.back.len(), steps_before);
}

#[test]
fn with_return_the_pane_is_back_at_the_start_after_the_drop() {
    let dir = tree();
    let (mut harness, _fake) = harness(dir.path());
    command(
        &mut harness,
        app::Command::Settings(SettingsChange::AfterDrag(AfterDrag::Return)),
    );
    let pane = harness.state().app().model().focused;
    let from = centre_of(&harness, "a.txt");
    press(&mut harness, from, egui::PointerButton::Primary);
    let sub = centre_of(&harness, "sub");
    move_to(&mut harness, from, sub);
    hover_for_spring(&mut harness);
    assert_eq!(location(&harness, pane), dir.path().join("sub"));
    let to = empty_space(&harness, pane);
    harness.event(egui::Event::PointerMoved(to));
    harness.step();
    release(&mut harness, to, egui::PointerButton::Primary);
    assert!(dir.path().join("sub/a.txt").exists());
    assert_eq!(location(&harness, pane), dir.path(), "returned");
    assert_eq!(
        tab(&harness, pane).1.history.back,
        [] as [model::HistoryEntry; 0]
    );
}

/// A file dropped from outside the process, as the backend reports it.
#[derive(Debug)]
struct Dropped(PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &Path {
        &self.0
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        fs::read(&self.0).map_err(|e| e.to_string())
    }
}

#[test]
fn an_external_drop_copies_and_shift_moves() {
    let dir = tree();
    let outside = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    fs::write(outside.path().join("ext.txt"), "e").unwrap_or_else(|e| panic!("{e}"));
    fs::write(outside.path().join("ext2.txt"), "e2").unwrap_or_else(|e| panic!("{e}"));
    let (mut harness, _fake) = harness(dir.path());
    let pane = harness.state().app().model().focused;

    // Hovering from outside shows the indicator: a copy, whatever the volume.
    let at = empty_space(&harness, pane);
    harness.event(egui::Event::PointerMoved(at));
    harness.input_mut().hovered_files.push(egui::HoveredFile {
        path: Some(outside.path().join("ext.txt")),
        mime: String::new(),
    });
    harness.step();
    harness.step();
    let view = harness
        .state()
        .drag()
        .unwrap_or_else(|| panic!("an external drag"));
    assert!(view.external);
    assert_eq!(view.indicator, Some(Indicator::Copy));

    harness.input_mut().hovered_files.clear();
    harness
        .input_mut()
        .dropped_files
        .push(Arc::new(Dropped(outside.path().join("ext.txt"))));
    harness.step();
    harness.step();
    settle(&mut harness);
    assert!(dir.path().join("ext.txt").exists());
    assert!(outside.path().join("ext.txt").exists(), "copied");
    assert!(harness.state().drag().is_none());

    // Shift forces a move.
    hold(&mut harness, egui::Modifiers::SHIFT);
    harness.event(egui::Event::PointerMoved(at));
    harness
        .input_mut()
        .dropped_files
        .push(Arc::new(Dropped(outside.path().join("ext2.txt"))));
    harness.step();
    harness.step();
    settle(&mut harness);
    hold(&mut harness, egui::Modifiers::NONE);
    assert!(dir.path().join("ext2.txt").exists());
    assert!(!outside.path().join("ext2.txt").exists(), "moved");
}

#[test]
fn leaving_the_window_hands_the_drag_to_the_platform() {
    let dir = tree();
    let (mut harness, fake) = harness(dir.path());
    let from = centre_of(&harness, "a.txt");
    press(&mut harness, from, egui::PointerButton::Primary);
    move_to(&mut harness, from, egui::pos2(-40.0, from.y));
    harness.step();
    let dragged = fake.dragged();
    assert_eq!(dragged.len(), 1, "{dragged:?}");
    assert!(dragged[0][0].starts_with("file://"), "{dragged:?}");
    assert!(dragged[0][0].ends_with("/a.txt"), "{dragged:?}");
    release(
        &mut harness,
        egui::pos2(-40.0, from.y),
        egui::PointerButton::Primary,
    );
    assert!(dir.path().join("a.txt").exists());
}

#[test]
fn pin_and_unpin_from_the_context_menu_flip_and_undo() {
    let dir = tree();
    let (mut harness, _fake) = harness(dir.path());
    let pinned =
        |harness: &Harness<'_, RickUi>| harness.state().app().model().settings.pinned.len();

    for label in ["a.txt", "sub"] {
        harness.get_by_label(label).click_secondary();
        harness.step();
        assert!(harness.query_by_label("Unpin").is_none());
        harness.get_by_label("Pin to side panel").click();
        harness.step();
        harness.step();
        harness.get_by_label(&format!("{} {label}", icons::PIN));

        harness.get_by_label(label).click_secondary();
        harness.step();
        assert!(
            harness.query_by_label("Pin to side panel").is_none(),
            "the label flipped"
        );
        harness.get_by_label("Unpin").click();
        harness.step();
        harness.step();
        assert_eq!(pinned(&harness), 0);

        // Undo brings the pin back, undo again removes it.
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
        harness.step();
        assert_eq!(pinned(&harness), 1, "undo of the unpin");
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
        harness.step();
        assert_eq!(pinned(&harness), 0, "undo of the pin");
        harness.key_press(egui::Key::Escape);
        harness.step();
    }

    // The Pinned entry's own menu unpins too.
    harness.get_by_label("sub").click_secondary();
    harness.step();
    harness.get_by_label("Pin to side panel").click();
    harness.step();
    harness.step();
    harness
        .get_by_label(&format!("{} sub", icons::PIN))
        .click_secondary();
    harness.step();
    harness.get_by_label("Unpin").click();
    harness.step();
    harness.step();
    assert_eq!(pinned(&harness), 0);
}
