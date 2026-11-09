//! Acceptance (issue 60, workspace tabs): the title row holds one tab per
//! workspace, titled after the folder in its focused pane; the new-tab
//! button, a click, a middle click, Mod+Tab and a drag along the row open,
//! show, close, cycle and reorder them; a pane dropped on the row becomes
//! a workspace of its own; and every tab and close button carries a name
//! for a screen reader. Issue 80, rules 7 and 8: a dragged tab is its own
//! ghost, lifted with a shadow — riding the row, or off it anchored to the
//! grab point — under the open hand before the drag and the closed hand
//! during it. Rule 1: the neighbours of a tab dragged along the row slide
//! apart to open the gap where it will land, with no insertion line, and
//! the tab can land anywhere in the row. Rules 2 and 3: a tab holding
//! exactly one pane, dragged down, is offered the four side dock targets
//! and docks its pane there; a tab of several panes is offered none.
//! Rule 4: a dragged pane, or a dragged tab of one pane, resting on
//! another tab for the spring-loading delay switches the window to that
//! tab's workspace, where it can be docked. Rule 9: `Mod+Shift+PageUp` and
//! `Mod+Shift+PageDown` move the shown tab along the row, and Escape
//! cancels the drag of a pane or of a tab and leaves the layout exactly as
//! it was.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use platform::{ConfigDir, Fake};
use tests::local;
use ui_egui::{EguiHost, RickUi};

fn harness(start: &Path, size: egui::Vec2) -> Harness<'static, RickUi> {
    harness_at(start, size, 0.25).0
}

/// A window whose frames are `dt` seconds apart, with its platform: a
/// quarter of a second is the harness's own step, a sixtieth shows an
/// animation frame by frame.
fn harness_at(start: &Path, size: egui::Vec2, dt: f32) -> (Harness<'static, RickUi>, Arc<Fake>) {
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
        .with_size(size)
        .with_step_dt(dt)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
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

/// The titles of the focused window's workspace tabs, in row order.
fn titles(harness: &Harness<'_, RickUi>) -> Vec<String> {
    let model = harness.state().app().model();
    model
        .workspaces(model.focused_window)
        .iter()
        .map(|ws| {
            model
                .active_tab(ws.focused)
                .and_then(|(_, t)| platform::paths::to_path(&t.location.path).ok())
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_default()
        })
        .collect()
}

/// The shown workspace's position in the row.
fn shown(harness: &Harness<'_, RickUi>) -> usize {
    let model = harness.state().app().model();
    let w = model
        .window(model.focused_window)
        .unwrap_or_else(|| panic!("window"));
    w.position(w.active).unwrap_or_else(|| panic!("active"))
}

/// Presses `button` at `pos` and releases it.
fn click_at(harness: &mut Harness<'_, RickUi>, pos: egui::Pos2, button: egui::PointerButton) {
    harness.event(egui::Event::PointerMoved(pos));
    harness.step();
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: egui::Modifiers::default(),
        });
        harness.step();
    }
    harness.step();
}

/// Presses the primary button at `from`, moves to `to` in small steps so
/// a drag is recognised, releases, and lets the app settle.
fn drag(harness: &mut Harness<'_, RickUi>, from: egui::Pos2, to: egui::Pos2) {
    harness.event(egui::Event::PointerMoved(from));
    harness.step();
    harness.event(egui::Event::PointerButton {
        pos: from,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::default(),
    });
    harness.step();
    for i in 1..=12u8 {
        let t = f32::from(i) / 12.0;
        let pos = from + (to - from) * t;
        harness.event(egui::Event::PointerMoved(pos));
        harness.step();
    }
    harness.event(egui::Event::PointerButton {
        pos: to,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::default(),
    });
    harness.step();
    harness.step();
    settle(harness);
}

/// The centre of the workspace tab labelled `title` in the title row: the
/// row's tabs are the buttons with that name above the panes.
fn tab_centre(harness: &Harness<'_, RickUi>, title: &str) -> egui::Pos2 {
    let row = harness
        .state()
        .tab_row()
        .unwrap_or_else(|| panic!("no tab row"));
    harness
        .query_all_by_label(title)
        .map(|n| n.rect())
        .find(|r| row.contains(r.center()))
        .unwrap_or_else(|| panic!("no tab {title} in the row {row:?}"))
        .center()
}

#[test]
fn the_title_row_opens_shows_closes_cycles_and_reorders_workspace_tabs() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let alpha = dir.path().join("alpha-folder");
    let beta = dir.path().join("beta-folder");
    fs::create_dir(&alpha).unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir(&beta).unwrap_or_else(|e| panic!("{e}"));
    fs::write(alpha.join("a.txt"), "a").unwrap_or_else(|e| panic!("{e}"));
    let mut harness = harness(&alpha, egui::vec2(1400.0, 800.0));

    // One tab, titled after the folder, with a close button named for a
    // screen reader; the new-tab button after it.
    assert_eq!(titles(&harness), ["alpha-folder"]);
    tab_centre(&harness, "alpha-folder");
    assert_eq!(harness.query_all_by_label("Close tab").count(), 1);

    // The new-tab button opens a workspace at the focused pane's folder,
    // right after the shown one, and shows it.
    harness.get_by_label("New tab").click();
    harness.step();
    settle(&mut harness);
    assert_eq!(titles(&harness), ["alpha-folder", "alpha-folder"]);
    assert_eq!(shown(&harness), 1);
    assert_eq!(harness.query_all_by_label("Close tab").count(), 2);

    // The title follows the folder of the workspace's focused pane.
    let pane = harness.state().app().model().focused;
    command(
        &mut harness,
        app::Command::Navigate {
            pane,
            to: local(&beta),
        },
    );
    assert_eq!(titles(&harness), ["alpha-folder", "beta-folder"]);

    // A click shows a tab; Mod+Tab cycles.
    let alpha_tab = tab_centre(&harness, "alpha-folder");
    click_at(&mut harness, alpha_tab, egui::PointerButton::Primary);
    settle(&mut harness);
    assert_eq!(shown(&harness), 0);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Tab);
    harness.step();
    settle(&mut harness);
    assert_eq!(shown(&harness), 1, "Mod+Tab shows the next tab");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Tab);
    harness.step();
    settle(&mut harness);
    assert_eq!(shown(&harness), 0, "and wraps around");

    // A third tab, dragged to the front of the row.
    harness.get_by_label("New tab").click();
    harness.step();
    settle(&mut harness);
    let gamma = dir.path().join("gamma-folder");
    fs::create_dir(&gamma).unwrap_or_else(|e| panic!("{e}"));
    let pane = harness.state().app().model().focused;
    command(
        &mut harness,
        app::Command::Navigate {
            pane,
            to: local(&gamma),
        },
    );
    assert_eq!(
        titles(&harness),
        ["alpha-folder", "gamma-folder", "beta-folder"],
        "a new tab opens right after the shown one"
    );
    let from = tab_centre(&harness, "gamma-folder");
    let first = tab_centre(&harness, "alpha-folder");
    drag(&mut harness, from, egui::pos2(first.x - 60.0, first.y));
    assert_eq!(
        titles(&harness),
        ["gamma-folder", "alpha-folder", "beta-folder"],
        "dragged to the front"
    );
    assert_eq!(shown(&harness), 0, "the dragged tab stays shown");

    // A middle click closes a tab; the neighbour shows.
    let alpha_tab = tab_centre(&harness, "alpha-folder");
    click_at(&mut harness, alpha_tab, egui::PointerButton::Middle);
    settle(&mut harness);
    assert_eq!(titles(&harness), ["gamma-folder", "beta-folder"]);
    // The close button of the shown tab closes it too.
    let shown_before = shown(&harness);
    let row = harness
        .state()
        .tab_row()
        .unwrap_or_else(|| panic!("no tab row"));
    let close = harness
        .query_all_by_label("Close tab")
        .map(|n| n.rect())
        .filter(|r| row.contains(r.center()))
        .nth(shown_before)
        .unwrap_or_else(|| panic!("no close button"));
    click_at(&mut harness, close.center(), egui::PointerButton::Primary);
    settle(&mut harness);
    assert_eq!(titles(&harness), ["beta-folder"]);
    // Mod+W on the last tab of the last window leaves it.
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::W);
    harness.step();
    settle(&mut harness);
    assert_eq!(titles(&harness), ["beta-folder"]);
}

#[test]
fn a_pane_dropped_on_the_tab_row_becomes_a_workspace_of_its_own() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let left = dir.path().join("left-pane");
    let right = dir.path().join("right-pane");
    fs::create_dir(&left).unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir(&right).unwrap_or_else(|e| panic!("{e}"));
    let mut harness = harness(&left, egui::vec2(1400.0, 800.0));
    let pane_a = harness.state().app().model().focused;
    command(
        &mut harness,
        app::Command::Split {
            pane: pane_a,
            dir: model::SplitDir::Right,
        },
    );
    let pane_b = harness.state().app().model().focused;
    assert_ne!(pane_b, pane_a);
    command(
        &mut harness,
        app::Command::Navigate {
            pane: pane_b,
            to: local(&right),
        },
    );
    assert_eq!(titles(&harness), ["right-pane"], "one workspace, two panes");

    // Pane B, dragged by its handle onto the tab row after the one tab: a
    // second workspace holding pane B, shown.
    let rect_b = harness
        .state()
        .pane_rects()
        .into_iter()
        .find(|(p, _)| *p == pane_b)
        .map_or_else(|| panic!("no rect"), |(_, r)| r);
    let from = harness
        .query_all_by_label("Drag pane")
        .map(|n| n.rect())
        .find(|r| rect_b.contains(r.center()))
        .map_or_else(|| panic!("no handle in pane B"), |r| r.center());
    let row = harness
        .state()
        .tab_row()
        .unwrap_or_else(|| panic!("no tab row"));
    let to = egui::pos2(row.left() + 240.0, row.center().y);
    drag(&mut harness, from, to);
    let model = harness.state().app().model();
    let window = model.focused_window;
    assert_eq!(
        model.workspaces(window).len(),
        2,
        "{:?}",
        model.workspaces(window)
    );
    assert_eq!(model.workspaces(window)[0].dock, model::Dock::Pane(pane_a));
    assert_eq!(model.workspaces(window)[1].dock, model::Dock::Pane(pane_b));
    assert_eq!(shown(&harness), 1);
    assert_eq!(titles(&harness), ["left-pane", "right-pane"]);
}

/// The cursor the last frame asked for.
fn cursor(harness: &Harness<'_, RickUi>) -> egui::CursorIcon {
    harness.output().platform_output.cursor_icon
}

/// Moves the pointer to `to` and runs a frame.
fn move_to(harness: &mut Harness<'_, RickUi>, to: egui::Pos2) {
    harness.event(egui::Event::PointerMoved(to));
    harness.step();
}

/// Presses or releases the primary button at `pos` and runs a frame.
fn button(harness: &mut Harness<'_, RickUi>, pos: egui::Pos2, pressed: bool) {
    harness.event(egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    });
    harness.step();
}

/// Whether the last frame painted a soft shadow under `rect`, in view: a
/// blurred rectangle of its size, a few points lower, its middle not
/// clipped away.
fn shadow_under(harness: &Harness<'_, RickUi>, rect: egui::Rect) -> bool {
    harness
        .output()
        .shapes
        .iter()
        .any(|clipped| match &clipped.shape {
            egui::Shape::Rect(r) => {
                r.blur_width > 0.0
                    && (r.rect.width() - rect.width()).abs() < 0.01
                    && (r.rect.left() - rect.left()).abs() < 0.01
                    && r.rect.top() > rect.top()
                    && r.rect.top() - rect.top() < 12.0
                    && clipped.clip_rect.contains(r.rect.center())
            }
            _ => false,
        })
}

/// Whether the last frame painted the text `title` inside `rect`, in view.
fn titled(harness: &Harness<'_, RickUi>, rect: egui::Rect, title: &str) -> bool {
    harness
        .output()
        .shapes
        .iter()
        .any(|clipped| match &clipped.shape {
            egui::Shape::Text(text) => {
                let at = text.galley.rect.translate(text.pos.to_vec2());
                text.galley.job.text == title
                    && rect.contains_rect(at)
                    && clipped.clip_rect.contains(at.center())
            }
            _ => false,
        })
}

/// Issue 80, rules 7 and 8 (SPEC.md *Workspace tabs*, *Drag ghost*): a tab
/// shows the open hand over its body and not over its close button; a
/// dragged tab is its own ghost, the tab itself lifted with a shadow —
/// along the row it rides the row held where it was grabbed, off the row
/// it follows the cursor with the grab point 8 px down and right of it and
/// the row keeps no room for it — under the closed hand throughout; and
/// released off the row inside the window it returns to its place.
#[test]
// One gesture from the hover to the release.
#[allow(clippy::too_many_lines)]
fn a_dragged_tab_is_its_own_ghost_lifted_with_a_shadow() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let alpha = dir.path().join("alpha-folder");
    let beta = dir.path().join("beta-folder");
    fs::create_dir(&alpha).unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir(&beta).unwrap_or_else(|e| panic!("{e}"));
    fs::write(alpha.join("a.txt"), "a").unwrap_or_else(|e| panic!("{e}"));
    let mut harness = harness(&alpha, egui::vec2(1400.0, 800.0));
    harness.get_by_label("New tab").click();
    harness.step();
    settle(&mut harness);
    let pane = harness.state().app().model().focused;
    command(
        &mut harness,
        app::Command::Navigate {
            pane,
            to: local(&beta),
        },
    );
    let alpha_tab = tab_centre(&harness, "alpha-folder");
    click_at(&mut harness, alpha_tab, egui::PointerButton::Primary);
    settle(&mut harness);
    assert_eq!(titles(&harness), ["alpha-folder", "beta-folder"]);
    assert_eq!(shown(&harness), 0);
    let row = harness
        .state()
        .tab_row()
        .unwrap_or_else(|| panic!("no tab row"));
    let tab = harness
        .query_all_by_label("beta-folder")
        .map(|n| n.rect())
        .find(|r| row.contains(r.center()))
        .unwrap_or_else(|| panic!("no beta tab"));

    // Rule 8: the open hand over the tab's body, not over its close button.
    move_to(&mut harness, tab.center());
    harness.step();
    assert_eq!(cursor(&harness), egui::CursorIcon::Grab);
    let close = harness
        .query_all_by_label("Close tab")
        .map(|n| n.rect())
        .find(|r| tab.contains(r.center()))
        .unwrap_or_else(|| panic!("no close button on the beta tab"));
    move_to(&mut harness, close.center());
    harness.step();
    assert_ne!(cursor(&harness), egui::CursorIcon::Grab);
    assert_eq!(harness.state().drag_ghost(), None);

    // Grabbed right of its middle and pulled along the row: the tab rides
    // the row, the grab point under the cursor.
    let grab = tab.center() + egui::vec2(30.0, 4.0);
    move_to(&mut harness, grab);
    button(&mut harness, grab, true);
    for i in 1..=6u8 {
        move_to(&mut harness, grab + egui::vec2(-8.0, 0.0) * f32::from(i));
    }
    let at = grab + egui::vec2(-48.0, 0.0);
    harness.step();
    let ghost = harness
        .state()
        .drag_ghost()
        .unwrap_or_else(|| panic!("no ghost in the row"));
    assert_eq!(ghost.of, ui_egui::drag::Dragged::Tab);
    assert!(
        (ghost.rect.size() - tab.size()).length() < 0.01,
        "full size"
    );
    assert!(
        (ghost.rect.min - (tab.min + egui::vec2(-48.0, 0.0))).length() < 0.01,
        "{:?} rides the row from {tab:?}",
        ghost.rect
    );
    assert!(
        ((ghost.rect.min + (grab - tab.min)) - at).length() < 0.01,
        "held where it was grabbed"
    );
    assert!(shadow_under(&harness, ghost.rect), "lifted with a shadow");
    assert!(
        titled(&harness, ghost.rect, "beta-folder"),
        "the tab itself"
    );
    assert_eq!(cursor(&harness), egui::CursorIcon::Grabbing);
    harness.get_by_label("Dragging tab beta-folder");
    assert_eq!(shown(&harness), 0, "a drag does not activate the tab");

    // Off the row, over the panes: the tab follows the cursor, its grab
    // point 8 px down and right of it, and the row closes up behind it.
    for stop in [
        egui::pos2(at.x, row.bottom() + 40.0),
        egui::pos2(900.0, 420.0),
        egui::pos2(120.0, 600.0),
    ] {
        move_to(&mut harness, stop);
        harness.step();
        let ghost = harness
            .state()
            .drag_ghost()
            .unwrap_or_else(|| panic!("no ghost at {stop:?}"));
        assert_eq!(ghost.of, ui_egui::drag::Dragged::Tab);
        assert!(
            (ghost.rect.size() - tab.size()).length() < 0.01,
            "full size"
        );
        let held = ghost.rect.min + (grab - tab.min);
        assert!(
            (held - (stop + ui_egui::drag::GHOST_OFFSET)).length() < 0.01,
            "the grab point at {held:?}, the cursor at {stop:?}"
        );
        assert!(shadow_under(&harness, ghost.rect), "lifted with a shadow");
        assert!(
            titled(&harness, ghost.rect, "beta-folder"),
            "the tab itself at {stop:?}"
        );
        assert_eq!(cursor(&harness), egui::CursorIcon::Grabbing, "at {stop:?}");
        let plus = harness.get_by_label("New tab").rect();
        assert!(
            (plus.left() - tab.left()).abs() < 0.01,
            "the row keeps no room: the new-tab button at {plus:?}"
        );
    }

    // Released there, inside the window and off the row: back in its
    // place, nothing shown that was not.
    let end = egui::pos2(120.0, 600.0);
    button(&mut harness, end, false);
    settle(&mut harness);
    assert_eq!(titles(&harness), ["alpha-folder", "beta-folder"]);
    assert_eq!(shown(&harness), 0);
    assert_eq!(harness.state().drag_ghost(), None);
    assert_ne!(cursor(&harness), egui::CursorIcon::Grabbing);
    let back = harness
        .query_all_by_label("beta-folder")
        .map(|n| n.rect())
        .find(|r| row.contains(r.center()))
        .unwrap_or_else(|| panic!("no beta tab"));
    assert_eq!(back, tab, "the tab is where it was");
}

/// The rectangle of the workspace tab labelled `title` in the title row.
fn tab_rect(harness: &Harness<'_, RickUi>, title: &str) -> egui::Rect {
    let row = harness
        .state()
        .tab_row()
        .unwrap_or_else(|| panic!("no tab row"));
    harness
        .query_all_by_label(title)
        .map(|n| n.rect())
        .find(|r| row.contains(r.center()))
        .unwrap_or_else(|| panic!("no tab {title} in the row {row:?}"))
}

/// Whether the last frame painted anything in `colour` inside `area`: a
/// filled or outlined rectangle, or a line.
fn painted_in(harness: &Harness<'_, RickUi>, area: egui::Rect, colour: egui::Color32) -> bool {
    harness
        .output()
        .shapes
        .iter()
        .any(|clipped| match &clipped.shape {
            egui::Shape::Rect(r) => {
                area.intersects(r.rect.intersect(clipped.clip_rect))
                    && (r.fill == colour || (r.stroke.width > 0.0 && r.stroke.color == colour))
            }
            egui::Shape::LineSegment { points, stroke } => {
                stroke.color == colour && points.iter().any(|p| area.contains(*p))
            }
            _ => false,
        })
}

/// Issue 80, rule 1 (SPEC.md *Workspace tabs*, Dragging a tab): a tab
/// dragged along the row follows the cursor; the neighbour it passes
/// slides, frame by frame, into the place the tab came from, so the gap
/// opens where the tab will land; nothing in an accent colour marks the
/// place — no insertion line; the tab can be taken to the last place of
/// the row, and released it settles into the gap. Under the system's
/// reduce-motion setting the neighbours take their places at once.
#[test]
// One drag from the first place to the last, then one back.
#[allow(clippy::too_many_lines)]
fn the_neighbours_of_a_dragged_tab_slide_apart_and_it_lands_anywhere_in_the_row() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let folders = ["alpha-folder", "beta-folder", "gamma-folder"].map(|name| {
        let folder = dir.path().join(name);
        fs::create_dir(&folder).unwrap_or_else(|e| panic!("{e}"));
        folder
    });
    let (mut harness, fake) = harness_at(&folders[0], egui::vec2(1400.0, 800.0), 1.0 / 60.0);
    command(
        &mut harness,
        app::Command::Settings(model::SettingsChange::Theme(model::Theme::Light)),
    );
    for folder in &folders[1..] {
        command(&mut harness, app::Command::NewTab(local(folder)));
    }
    assert_eq!(
        titles(&harness),
        ["alpha-folder", "beta-folder", "gamma-folder"]
    );
    let row = harness
        .state()
        .tab_row()
        .unwrap_or_else(|| panic!("no tab row"));
    let title_row = row.expand2(egui::vec2(0.0, 8.0));
    let first = tab_rect(&harness, "alpha-folder");
    let pitch = tab_rect(&harness, "beta-folder").left() - first.left();
    let place = |n: f32| first.left() + n * pitch;
    assert!((tab_rect(&harness, "gamma-folder").left() - place(2.0)).abs() < 0.01);
    let scheme = ui_egui::theme::Scheme::LIGHT;
    // How many frames a slide takes at this pace, and one to spare.
    let slide = (1u32..=600)
        .find(|frames| f64::from(*frames) / 60.0 >= ui_egui::drag::TAB_SLIDE)
        .unwrap_or(10)
        + 1;

    // The first tab, taken by its middle and pulled right: until it is
    // half over its neighbour, the neighbour stays.
    let grab = first.center();
    harness.event(egui::Event::PointerMoved(grab));
    harness.step();
    harness.event(egui::Event::PointerButton {
        pos: grab,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::default(),
    });
    harness.step();
    let mut travelled = 0.0;
    while travelled < pitch / 2.0 - 10.0 {
        travelled += 6.0;
        harness.event(egui::Event::PointerMoved(grab + egui::vec2(travelled, 0.0)));
        harness.step();
    }
    for _ in 0..slide {
        harness.step();
    }
    let ghost = harness
        .state()
        .drag_ghost()
        .unwrap_or_else(|| panic!("no ghost"));
    assert!(
        (ghost.rect.left() - (first.left() + travelled)).abs() < 0.01,
        "the tab follows the cursor along the row: {:?}",
        ghost.rect
    );
    assert!(
        (tab_rect(&harness, "beta-folder").left() - place(1.0)).abs() < 0.01,
        "not yet passed"
    );

    // Past the neighbour's middle: the neighbour slides to the first
    // place, a little further every frame and never back, and no line or
    // slot in an accent colour marks where the tab will land.
    travelled = pitch / 2.0 + 8.0;
    harness.event(egui::Event::PointerMoved(grab + egui::vec2(travelled, 0.0)));
    harness.step();
    let mut seen = vec![tab_rect(&harness, "beta-folder").left()];
    for _ in 0..slide {
        harness.step();
        seen.push(tab_rect(&harness, "beta-folder").left());
        for accent in [scheme.accent_button, scheme.accent_focus] {
            assert!(
                !painted_in(&harness, title_row, accent),
                "no insertion line in the row"
            );
        }
    }
    assert!(
        seen.windows(2).all(|pair| pair[1] <= pair[0]),
        "it slides one way: {seen:?}"
    );
    let underway = seen
        .iter()
        .filter(|x| **x < place(1.0) - 1.0 && **x > place(0.0) + 1.0)
        .count();
    assert!(
        underway >= 4,
        "{underway} frames between the two places: {seen:?}"
    );
    assert!(
        (seen[seen.len() - 1] - place(0.0)).abs() < 0.01,
        "it arrives at the first place: {seen:?}"
    );
    assert!(
        (tab_rect(&harness, "gamma-folder").left() - place(2.0)).abs() < 0.01,
        "the tab beyond the gap stays"
    );

    // On to the end of the row, the last place of all; released a little
    // short of it, the tab settles into the gap.
    while travelled < 2.0 * pitch - 40.0 {
        travelled += 6.0;
        harness.event(egui::Event::PointerMoved(grab + egui::vec2(travelled, 0.0)));
        harness.step();
    }
    for _ in 0..slide {
        harness.step();
    }
    assert!(
        (tab_rect(&harness, "gamma-folder").left() - place(1.0)).abs() < 0.01,
        "the last tab made room too"
    );
    let end = grab + egui::vec2(travelled, 0.0);
    harness.event(egui::Event::PointerButton {
        pos: end,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::default(),
    });
    harness.step();
    harness.step();
    assert_eq!(
        titles(&harness),
        ["beta-folder", "gamma-folder", "alpha-folder"],
        "dragged from the first place to the last"
    );
    let settling = tab_rect(&harness, "alpha-folder").left();
    assert!(
        settling > first.left() + travelled - 0.01 && settling < place(2.0) - 1.0,
        "settling from where it was let go: {settling}"
    );
    for _ in 0..slide {
        harness.step();
    }
    assert!((tab_rect(&harness, "alpha-folder").left() - place(2.0)).abs() < 0.01);
    assert_eq!(harness.state().drag_ghost(), None);

    // With reduced motion the neighbour is in its new place in the frame
    // the tab passes its middle.
    fake.set_reduce_motion(true);
    let last = tab_rect(&harness, "alpha-folder");
    let grab = last.center();
    harness.event(egui::Event::PointerMoved(grab));
    harness.step();
    harness.event(egui::Event::PointerButton {
        pos: grab,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::default(),
    });
    harness.step();
    let mut travelled = 0.0;
    while travelled < pitch / 2.0 - 10.0 {
        travelled += 6.0;
        harness.event(egui::Event::PointerMoved(grab - egui::vec2(travelled, 0.0)));
        harness.step();
    }
    assert!((tab_rect(&harness, "gamma-folder").left() - place(1.0)).abs() < 0.01);
    travelled = pitch / 2.0 + 8.0;
    let over = grab - egui::vec2(travelled, 0.0);
    harness.event(egui::Event::PointerMoved(over));
    harness.step();
    assert!(
        (tab_rect(&harness, "gamma-folder").left() - place(2.0)).abs() < 0.01,
        "no slide under reduced motion"
    );
    harness.event(egui::Event::PointerButton {
        pos: over,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::default(),
    });
    harness.step();
    settle(&mut harness);
    assert_eq!(
        titles(&harness),
        ["beta-folder", "alpha-folder", "gamma-folder"]
    );
}

/// The centres of the dock tiles the last frame painted: the 40 px
/// rounded squares of the dock cross, lit or not.
fn dock_tiles(harness: &Harness<'_, RickUi>, scheme: &ui_egui::theme::Scheme) -> Vec<egui::Pos2> {
    harness
        .output()
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Rect(r)
                if (r.rect.width() - ui_egui::DOCK_TILE).abs() < 0.01
                    && (r.rect.height() - ui_egui::DOCK_TILE).abs() < 0.01
                    && (r.fill == scheme.bg_card || r.fill == scheme.accent_button) =>
            {
                Some(r.rect.center())
            }
            _ => None,
        })
        .collect()
}

/// Presses the primary button at `from` and moves to `to` in small steps,
/// the button still down.
fn hold(harness: &mut Harness<'_, RickUi>, from: egui::Pos2, to: egui::Pos2) {
    move_to(harness, from);
    button(harness, from, true);
    for i in 1..=12u8 {
        move_to(harness, from + (to - from) * (f32::from(i) / 12.0));
    }
    harness.step();
}

/// Issue 80, rules 2 and 3 (SPEC.md *Workspace tabs*, Dragging a tab): a
/// tab holding exactly one pane, dragged down into the shown workspace,
/// is offered the four side targets on the pane under the cursor — no
/// centre — with the preview of the half; dropped on one, its pane docks
/// on that side and the tab leaves the row. A tab holding several panes
/// is offered no target and returns to its place, and so does the shown
/// tab over its own pane.
#[test]
// One row of tabs, three drags.
#[allow(clippy::too_many_lines)]
fn a_tab_of_one_pane_docks_into_the_shown_layout_and_a_tab_of_several_is_offered_nothing() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let folders = [
        "alpha-folder",
        "beta-folder",
        "gamma-folder",
        "delta-folder",
    ]
    .map(|name| {
        let folder = dir.path().join(name);
        fs::create_dir(&folder).unwrap_or_else(|e| panic!("{e}"));
        folder
    });
    let mut harness = harness(&folders[0], egui::vec2(1400.0, 800.0));
    command(
        &mut harness,
        app::Command::Settings(model::SettingsChange::Theme(model::Theme::Light)),
    );
    let scheme = ui_egui::theme::Scheme::LIGHT;
    let pane_alpha = harness.state().app().model().focused;
    command(&mut harness, app::Command::NewTab(local(&folders[1])));
    let pane_beta = harness.state().app().model().focused;
    // A third workspace of two panes, gamma and delta.
    command(&mut harness, app::Command::NewTab(local(&folders[2])));
    let pane_gamma = harness.state().app().model().focused;
    command(
        &mut harness,
        app::Command::Split {
            pane: pane_gamma,
            dir: model::SplitDir::Right,
        },
    );
    let pane_delta = harness.state().app().model().focused;
    command(
        &mut harness,
        app::Command::Navigate {
            pane: pane_delta,
            to: local(&folders[3]),
        },
    );
    let model = harness.state().app().model();
    let window = model.focused_window;
    let ids: Vec<model::WorkspaceId> = model.workspaces(window).iter().map(|w| w.id).collect();
    assert_eq!(ids.len(), 3);
    assert_eq!(model.sole_pane(ids[1]), Some(pane_beta));
    assert_eq!(model.sole_pane(ids[2]), None, "two panes");
    command(&mut harness, app::Command::ActivateWorkspace(ids[0]));
    assert_eq!(
        titles(&harness),
        ["alpha-folder", "beta-folder", "delta-folder"]
    );
    assert_eq!(shown(&harness), 0);
    let rect_alpha = harness
        .state()
        .pane_rects()
        .into_iter()
        .find(|(p, _)| *p == pane_alpha)
        .map_or_else(|| panic!("no rect"), |(_, r)| r);
    let tile =
        |dx: f32, dy: f32| rect_alpha.center() + egui::vec2(dx, dy) * ui_egui::DOCK_TILE_PITCH;

    // Rule 3: the tab of two panes, dragged down over the shown pane, is
    // offered nothing; released where a target would be, it is back in its
    // place.
    let from = tab_centre(&harness, "delta-folder");
    hold(&mut harness, from, tile(-1.0, 0.0));
    assert!(
        harness.state().drag_ghost().is_some(),
        "it is being dragged"
    );
    assert_eq!(dock_tiles(&harness, &scheme), [], "no dock targets");
    button(&mut harness, tile(-1.0, 0.0), false);
    settle(&mut harness);
    let model = harness.state().app().model();
    assert_eq!(
        model
            .workspaces(window)
            .iter()
            .map(|w| w.id)
            .collect::<Vec<_>>(),
        ids
    );
    assert_eq!(*model.dock(), model::Dock::Pane(pane_alpha));

    // Rule 2: the tab of one pane, dragged down over the shown pane: the
    // four side targets and no centre.
    let from = tab_centre(&harness, "beta-folder");
    let beside = rect_alpha.center() + egui::vec2(140.0, 130.0);
    hold(&mut harness, from, beside);
    let mut tiles = dock_tiles(&harness, &scheme);
    tiles.sort_by(|a, b| {
        (a.x, a.y)
            .partial_cmp(&(b.x, b.y))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut wanted = vec![
        tile(-1.0, 0.0),
        tile(0.0, -1.0),
        tile(0.0, 1.0),
        tile(1.0, 0.0),
    ];
    wanted.sort_by(|a, b| {
        (a.x, a.y)
            .partial_cmp(&(b.x, b.y))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    assert_eq!(tiles.len(), 4, "left, right, top and bottom: {tiles:?}");
    for (got, want) in tiles.iter().zip(&wanted) {
        assert!(
            (*got - *want).length() < 0.01,
            "{tiles:?} against {wanted:?}"
        );
    }
    assert_eq!(shown(&harness), 0, "the window shows what it showed");
    // On the left target: it is lit, and the left half of the pane is
    // previewed.
    move_to(&mut harness, tile(-1.0, 0.0));
    harness.step();
    let lit = harness
        .output()
        .shapes
        .iter()
        .any(|clipped| match &clipped.shape {
            egui::Shape::Rect(r) => {
                r.fill == scheme.accent_button
                    && (r.rect.center() - tile(-1.0, 0.0)).length() < 0.01
            }
            _ => false,
        });
    assert!(lit, "the target under the cursor is filled accent/button");
    let half = egui::Rect::from_min_max(
        rect_alpha.min,
        egui::pos2(rect_alpha.center().x, rect_alpha.max.y),
    );
    let preview = harness
        .output()
        .shapes
        .iter()
        .any(|clipped| match &clipped.shape {
            egui::Shape::Rect(r) => {
                r.fill == scheme.accent_button.gamma_multiply(0.35)
                    && (r.rect.min - half.min).length() < 0.01
                    && (r.rect.max - half.max).length() < 0.01
            }
            _ => false,
        });
    assert!(preview, "the half the pane would take is previewed");
    // Dropped there: beta's pane is left of alpha's in the one layout, the
    // tab is gone from the row, and the docked pane has the keyboard.
    button(&mut harness, tile(-1.0, 0.0), false);
    settle(&mut harness);
    let model = harness.state().app().model();
    assert_eq!(
        model
            .workspaces(window)
            .iter()
            .map(|w| w.id)
            .collect::<Vec<_>>(),
        [ids[0], ids[2]],
        "the docked tab left the row"
    );
    assert_eq!(
        *model.dock(),
        model::Dock::split_of(
            model::SplitDir::Right,
            model::Dock::Pane(pane_beta),
            model::Dock::Pane(pane_alpha),
        ),
        "beta's pane is left of alpha's"
    );
    assert_eq!(model.focused, pane_beta);
    assert_eq!(shown(&harness), 0);
    assert_eq!(harness.state().drag_ghost(), None);
    assert_eq!(dock_tiles(&harness, &scheme), []);

    // The shown tab over its own pane: a tab of one pane, and nothing to
    // dock it on.
    let epsilon = dir.path().join("epsilon-folder");
    fs::create_dir(&epsilon).unwrap_or_else(|e| panic!("{e}"));
    command(&mut harness, app::Command::NewTab(local(&epsilon)));
    let pane_epsilon = harness.state().app().model().focused;
    let model = harness.state().app().model();
    let order: Vec<model::WorkspaceId> = model.workspaces(window).iter().map(|w| w.id).collect();
    let rect_epsilon = harness
        .state()
        .pane_rects()
        .into_iter()
        .find(|(p, _)| *p == pane_epsilon)
        .map_or_else(|| panic!("no rect"), |(_, r)| r);
    let own = rect_epsilon.center() + egui::vec2(-ui_egui::DOCK_TILE_PITCH, 0.0);
    let from = tab_centre(&harness, "epsilon-folder");
    hold(&mut harness, from, own);
    assert!(harness.state().drag_ghost().is_some());
    assert_eq!(
        dock_tiles(&harness, &scheme),
        [],
        "no target on its own pane"
    );
    button(&mut harness, own, false);
    settle(&mut harness);
    let model = harness.state().app().model();
    assert_eq!(
        model
            .workspaces(window)
            .iter()
            .map(|w| w.id)
            .collect::<Vec<_>>(),
        order
    );
    assert_eq!(*model.dock(), model::Dock::Pane(pane_epsilon));
}

/// The pane rectangle of `pane` in the last frame.
fn pane_rect(harness: &Harness<'_, RickUi>, pane: model::PaneId) -> egui::Rect {
    harness
        .state()
        .pane_rects()
        .into_iter()
        .find(|(p, _)| *p == pane)
        .map_or_else(|| panic!("no rect for {pane:?}"), |(_, r)| r)
}

/// Runs `frames` frames with the pointer where it is: at the harness's
/// quarter of a second a frame, four are a second.
fn rest(harness: &mut Harness<'_, RickUi>, frames: usize) {
    for _ in 0..frames {
        harness.step();
    }
}

/// Issue 80, rule 4 (SPEC.md *Workspace tabs*, Spring-loaded tabs), for a
/// dragged tab: a tab of one pane that leans on a neighbour — overlapping
/// it by a quarter of a tab or more, short of the half where they swap —
/// for the spring-loading delay switches the window to the neighbour's
/// workspace, and dragged down from there docks into its layout. Held
/// squarely in its gap it switches nothing, and neither does a tab of
/// several panes.
#[test]
// One row of tabs, three drags.
#[allow(clippy::too_many_lines)]
fn a_dragged_tab_of_one_pane_leaning_on_a_neighbour_springs_its_workspace_open() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let folders = [
        "alpha-folder",
        "beta-folder",
        "gamma-folder",
        "delta-folder",
    ]
    .map(|name| {
        let folder = dir.path().join(name);
        fs::create_dir(&folder).unwrap_or_else(|e| panic!("{e}"));
        folder
    });
    let mut harness = harness(&folders[0], egui::vec2(1400.0, 800.0));
    command(
        &mut harness,
        app::Command::Settings(model::SettingsChange::SpringDelay(500)),
    );
    // alpha, of two panes (alpha and delta); beta; gamma. Alpha is shown.
    let pane_alpha = harness.state().app().model().focused;
    command(
        &mut harness,
        app::Command::Split {
            pane: pane_alpha,
            dir: model::SplitDir::Right,
        },
    );
    let pane_delta = harness.state().app().model().focused;
    command(
        &mut harness,
        app::Command::Navigate {
            pane: pane_delta,
            to: local(&folders[3]),
        },
    );
    command(&mut harness, app::Command::NewTab(local(&folders[1])));
    let pane_beta = harness.state().app().model().focused;
    command(&mut harness, app::Command::NewTab(local(&folders[2])));
    let pane_gamma = harness.state().app().model().focused;
    let model = harness.state().app().model();
    let window = model.focused_window;
    let ids: Vec<model::WorkspaceId> = model.workspaces(window).iter().map(|w| w.id).collect();
    command(&mut harness, app::Command::ActivateWorkspace(ids[0]));
    assert_eq!(
        titles(&harness),
        ["delta-folder", "beta-folder", "gamma-folder"]
    );
    assert_eq!(shown(&harness), 0);
    let pitch =
        tab_rect(&harness, "gamma-folder").left() - tab_rect(&harness, "beta-folder").left();

    // The tab of two panes, leaning on its neighbour for well over the
    // delay: it is not one that can be docked, and nothing springs.
    let from = tab_centre(&harness, "delta-folder");
    hold(&mut harness, from, from + egui::vec2(pitch * 0.35, 0.0));
    rest(&mut harness, 6);
    assert_eq!(shown(&harness), 0, "a tab of several panes springs nothing");
    button(&mut harness, from + egui::vec2(pitch * 0.35, 0.0), false);
    settle(&mut harness);
    assert_eq!(
        titles(&harness),
        ["delta-folder", "beta-folder", "gamma-folder"]
    );

    // The tab of one pane, taken slowly all the way past gamma — over it
    // for seconds, but never still: a reorder, and nothing springs.
    let from = tab_centre(&harness, "beta-folder");
    move_to(&mut harness, from);
    button(&mut harness, from, true);
    for i in 1..=24u8 {
        move_to(
            &mut harness,
            from + egui::vec2(pitch * f32::from(i) / 24.0, 0.0),
        );
        assert_eq!(shown(&harness), 0, "on the move it rests on no one");
    }
    button(&mut harness, from + egui::vec2(pitch, 0.0), false);
    settle(&mut harness);
    assert_eq!(
        titles(&harness),
        ["delta-folder", "gamma-folder", "beta-folder"]
    );
    assert_eq!(shown(&harness), 0);
    // And back to where it was, as slowly.
    let from = tab_centre(&harness, "beta-folder");
    move_to(&mut harness, from);
    button(&mut harness, from, true);
    for i in 1..=24u8 {
        move_to(
            &mut harness,
            from - egui::vec2(pitch * f32::from(i) / 24.0, 0.0),
        );
    }
    button(&mut harness, from - egui::vec2(pitch, 0.0), false);
    settle(&mut harness);
    assert_eq!(
        titles(&harness),
        ["delta-folder", "beta-folder", "gamma-folder"]
    );
    assert_eq!(shown(&harness), 0);

    // The tab of one pane, held squarely in its gap: nothing springs.
    let from = tab_centre(&harness, "beta-folder");
    hold(&mut harness, from, from + egui::vec2(pitch * 0.1, 0.0));
    rest(&mut harness, 6);
    assert_eq!(shown(&harness), 0, "in its gap it rests on no one");
    // Leaning on gamma, a third of a tab over it: not at once, but after
    // the delay the window shows gamma.
    move_to(&mut harness, from + egui::vec2(pitch * 0.35, 0.0));
    assert_eq!(shown(&harness), 0, "not before the delay");
    rest(&mut harness, 4);
    assert_eq!(shown(&harness), 2, "the window shows the tab it leans on");
    assert!(harness.state().drag_ghost().is_some(), "still dragging");
    // Down into gamma's layout, onto the left target of its pane.
    let rect_gamma = pane_rect(&harness, pane_gamma);
    let tile = rect_gamma.center() + egui::vec2(-ui_egui::DOCK_TILE_PITCH, 0.0);
    for i in 1..=10u8 {
        let start = from + egui::vec2(pitch * 0.35, 0.0);
        move_to(&mut harness, start + (tile - start) * (f32::from(i) / 10.0));
    }
    button(&mut harness, tile, false);
    settle(&mut harness);
    let model = harness.state().app().model();
    assert_eq!(
        model
            .workspaces(window)
            .iter()
            .map(|w| w.id)
            .collect::<Vec<_>>(),
        [ids[0], ids[2]],
        "beta's tab left the row"
    );
    assert_eq!(
        *model.dock(),
        model::Dock::split_of(
            model::SplitDir::Right,
            model::Dock::Pane(pane_beta),
            model::Dock::Pane(pane_gamma),
        ),
        "beta's pane docked left of gamma's"
    );
    assert_eq!(model.window(window).map(|w| w.active), Some(ids[2]));
}

/// Issue 80, rule 4, for a dragged pane: resting on another workspace's
/// tab for the spring-loading delay switches the window to it, and the
/// pane docks into the layout that opened. Let go where it cannot land,
/// nothing moves and the window shows the workspace the drag began in.
#[test]
// One pane, dragged twice.
#[allow(clippy::too_many_lines)]
fn a_dragged_pane_resting_on_a_tab_springs_its_workspace_open_and_docks_there() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let folders = ["alpha-folder", "beta-folder", "gamma-folder"].map(|name| {
        let folder = dir.path().join(name);
        fs::create_dir(&folder).unwrap_or_else(|e| panic!("{e}"));
        folder
    });
    let mut harness = harness(&folders[0], egui::vec2(1400.0, 800.0));
    command(
        &mut harness,
        app::Command::Settings(model::SettingsChange::SpringDelay(500)),
    );
    // The first workspace: alpha and beta side by side. The second: gamma.
    let pane_alpha = harness.state().app().model().focused;
    command(
        &mut harness,
        app::Command::Split {
            pane: pane_alpha,
            dir: model::SplitDir::Right,
        },
    );
    let pane_beta = harness.state().app().model().focused;
    command(
        &mut harness,
        app::Command::Navigate {
            pane: pane_beta,
            to: local(&folders[1]),
        },
    );
    command(&mut harness, app::Command::NewTab(local(&folders[2])));
    let pane_gamma = harness.state().app().model().focused;
    let model = harness.state().app().model();
    let window = model.focused_window;
    let ids: Vec<model::WorkspaceId> = model.workspaces(window).iter().map(|w| w.id).collect();
    command(&mut harness, app::Command::ActivateWorkspace(ids[0]));
    assert_eq!(shown(&harness), 0);
    let first = harness.state().app().model().dock().clone();
    let grip_of = |harness: &Harness<'_, RickUi>, pane: model::PaneId| {
        let rect = pane_rect(harness, pane);
        harness
            .query_all_by_label("Drag pane")
            .map(|n| n.rect())
            .find(|r| rect.contains(r.center()))
            .map_or_else(|| panic!("no handle in {pane:?}"), |r| r.center())
    };
    // On the right half of gamma's tab, where the slot of a new tab opens
    // after it and the tab stays under the cursor.
    let on_tab = |harness: &Harness<'_, RickUi>| {
        tab_rect(harness, "gamma-folder").center() + egui::vec2(60.0, 0.0)
    };

    // Beta's pane, held on gamma's tab: after the delay the window shows
    // gamma; let go beside the dock targets there, nothing has moved and
    // the first workspace is shown again.
    let from = grip_of(&harness, pane_beta);
    let to = on_tab(&harness);
    hold(&mut harness, from, to);
    assert_eq!(shown(&harness), 0, "not before the delay");
    rest(&mut harness, 4);
    assert_eq!(shown(&harness), 1, "resting on the tab shows its workspace");
    assert!(harness.state().drag_ghost().is_some(), "still dragging");
    let rect_gamma = pane_rect(&harness, pane_gamma);
    let beside = egui::pos2(rect_gamma.center().x, rect_gamma.bottom() - 60.0);
    for i in 1..=10u8 {
        move_to(&mut harness, to + (beside - to) * (f32::from(i) / 10.0));
    }
    button(&mut harness, beside, false);
    settle(&mut harness);
    let model = harness.state().app().model();
    assert_eq!(
        shown(&harness),
        0,
        "back on the workspace the drag began in"
    );
    assert_eq!(*model.dock(), first, "nothing moved");
    assert_eq!(model.workspaces(window).len(), 2);

    // Again, and this time onto the left target of gamma's pane: beta's
    // pane is docked there, and the window stays on that layout.
    let from = grip_of(&harness, pane_beta);
    let to = on_tab(&harness);
    hold(&mut harness, from, to);
    rest(&mut harness, 4);
    assert_eq!(shown(&harness), 1);
    let rect_gamma = pane_rect(&harness, pane_gamma);
    let tile = rect_gamma.center() + egui::vec2(-ui_egui::DOCK_TILE_PITCH, 0.0);
    for i in 1..=10u8 {
        move_to(&mut harness, to + (tile - to) * (f32::from(i) / 10.0));
    }
    button(&mut harness, tile, false);
    settle(&mut harness);
    let model = harness.state().app().model();
    assert_eq!(
        model.workspaces(window)[0].dock,
        model::Dock::Pane(pane_alpha)
    );
    assert_eq!(
        model.workspaces(window)[1].dock,
        model::Dock::split_of(
            model::SplitDir::Right,
            model::Dock::Pane(pane_beta),
            model::Dock::Pane(pane_gamma),
        ),
        "beta's pane docked left of gamma's"
    );
    assert_eq!(shown(&harness), 1, "the window stays where the pane went");
}

/// Issue 80, rule 9 (SPEC.md *Workspace tabs*, Behaviour; *Move tab left /
/// right*): `Mod+Shift+PageDown` and `Mod+Shift+PageUp` move the shown
/// tab one place right and left, the keyboard's way of reordering, and
/// stop at the ends of the row.
#[test]
fn mod_shift_page_up_and_down_move_the_shown_tab_along_the_row() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let folders = ["alpha-folder", "beta-folder", "gamma-folder"].map(|name| {
        let folder = dir.path().join(name);
        fs::create_dir(&folder).unwrap_or_else(|e| panic!("{e}"));
        folder
    });
    let mut harness = harness(&folders[0], egui::vec2(1400.0, 800.0));
    for folder in &folders[1..] {
        command(&mut harness, app::Command::NewTab(local(folder)));
    }
    let model = harness.state().app().model();
    let first = model.workspaces(model.focused_window)[0].id;
    command(&mut harness, app::Command::ActivateWorkspace(first));
    assert_eq!(
        titles(&harness),
        ["alpha-folder", "beta-folder", "gamma-folder"]
    );
    let key = |harness: &mut Harness<'_, RickUi>, key: egui::Key| {
        harness.key_press_modifiers(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, key);
        harness.step();
        settle(harness);
    };

    // At the left end, left does nothing.
    key(&mut harness, egui::Key::PageUp);
    assert_eq!(
        titles(&harness),
        ["alpha-folder", "beta-folder", "gamma-folder"]
    );
    // Right, right: to the end, the tab still shown.
    key(&mut harness, egui::Key::PageDown);
    assert_eq!(
        titles(&harness),
        ["beta-folder", "alpha-folder", "gamma-folder"]
    );
    assert_eq!(shown(&harness), 1, "the moved tab stays shown");
    key(&mut harness, egui::Key::PageDown);
    assert_eq!(
        titles(&harness),
        ["beta-folder", "gamma-folder", "alpha-folder"]
    );
    assert_eq!(shown(&harness), 2);
    // At the right end, right does nothing; left goes back.
    key(&mut harness, egui::Key::PageDown);
    assert_eq!(
        titles(&harness),
        ["beta-folder", "gamma-folder", "alpha-folder"]
    );
    key(&mut harness, egui::Key::PageUp);
    assert_eq!(
        titles(&harness),
        ["beta-folder", "alpha-folder", "gamma-folder"]
    );
    assert_eq!(shown(&harness), 1);
}

/// Issue 80, rule 9 (SPEC.md *No-ops and cancel*): Escape cancels the drag
/// of a tab and the drag of a pane. The ghost goes and nothing moves: the
/// tabs are in their order, the panes in their layout, and the window
/// shows the workspace it showed when the drag began, also after
/// spring-loading switched it. The button that is still down then drops
/// nothing when it comes up.
#[test]
// Four drags, each cancelled.
#[allow(clippy::too_many_lines)]
fn escape_cancels_the_drag_of_a_tab_or_a_pane_and_restores_the_layout() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let folders = [
        "alpha-folder",
        "beta-folder",
        "gamma-folder",
        "delta-folder",
    ]
    .map(|name| {
        let folder = dir.path().join(name);
        fs::create_dir(&folder).unwrap_or_else(|e| panic!("{e}"));
        folder
    });
    let mut harness = harness(&folders[0], egui::vec2(1400.0, 800.0));
    command(
        &mut harness,
        app::Command::Settings(model::SettingsChange::SpringDelay(500)),
    );
    // alpha and delta side by side; beta; gamma. The first is shown.
    let pane_alpha = harness.state().app().model().focused;
    command(
        &mut harness,
        app::Command::Split {
            pane: pane_alpha,
            dir: model::SplitDir::Right,
        },
    );
    let pane_delta = harness.state().app().model().focused;
    command(
        &mut harness,
        app::Command::Navigate {
            pane: pane_delta,
            to: local(&folders[3]),
        },
    );
    command(&mut harness, app::Command::NewTab(local(&folders[1])));
    command(&mut harness, app::Command::NewTab(local(&folders[2])));
    let pane_gamma = harness.state().app().model().focused;
    let model = harness.state().app().model();
    let window = model.focused_window;
    let ids: Vec<model::WorkspaceId> = model.workspaces(window).iter().map(|w| w.id).collect();
    command(&mut harness, app::Command::ActivateWorkspace(ids[0]));
    let before = harness.state().app().model().workspaces(window).to_vec();
    let unchanged = |harness: &Harness<'_, RickUi>, what: &str| {
        let model = harness.state().app().model();
        assert_eq!(model.workspaces(window), &before[..], "{what}");
        assert_eq!(
            model.window(window).map(|w| w.active),
            Some(ids[0]),
            "{what}"
        );
        assert_eq!(harness.state().drag_ghost(), None, "{what}");
        assert_ne!(
            harness.output().platform_output.cursor_icon,
            egui::CursorIcon::Grabbing,
            "{what}"
        );
    };
    let escape = |harness: &mut Harness<'_, RickUi>| {
        harness.key_press(egui::Key::Escape);
        harness.step();
        harness.step();
    };
    let pitch =
        tab_rect(&harness, "gamma-folder").left() - tab_rect(&harness, "beta-folder").left();
    let beta_home = tab_rect(&harness, "beta-folder");

    // A tab, taken past its neighbour so the row has opened another gap.
    let from = beta_home.center();
    hold(&mut harness, from, from + egui::vec2(pitch * 0.8, 0.0));
    assert!(harness.state().drag_ghost().is_some());
    assert!(
        tab_rect(&harness, "gamma-folder").left() < beta_home.left() + pitch - 1.0,
        "the neighbour has made room"
    );
    escape(&mut harness);
    rest(&mut harness, 2);
    unchanged(&harness, "a tab on the row");
    assert_eq!(
        tab_rect(&harness, "beta-folder"),
        beta_home,
        "back in its place"
    );
    button(&mut harness, from + egui::vec2(pitch * 0.8, 0.0), false);
    settle(&mut harness);
    unchanged(&harness, "the button coming up after the cancel");

    // A tab of one pane that spring-loading took to gamma's workspace,
    // held over a dock target there.
    hold(&mut harness, from, from + egui::vec2(pitch * 0.35, 0.0));
    rest(&mut harness, 4);
    assert_eq!(shown(&harness), 2, "sprung to gamma");
    let tile =
        pane_rect(&harness, pane_gamma).center() + egui::vec2(-ui_egui::DOCK_TILE_PITCH, 0.0);
    for i in 1..=8u8 {
        let start = from + egui::vec2(pitch * 0.35, 0.0);
        move_to(&mut harness, start + (tile - start) * (f32::from(i) / 8.0));
    }
    escape(&mut harness);
    unchanged(&harness, "a tab after spring-loading");
    button(&mut harness, tile, false);
    settle(&mut harness);
    unchanged(&harness, "the button coming up on a dock target");

    // A pane, held over the other pane's dock target.
    let grip_of = |harness: &Harness<'_, RickUi>, pane: model::PaneId| {
        let rect = pane_rect(harness, pane);
        harness
            .query_all_by_label("Drag pane")
            .map(|n| n.rect())
            .find(|r| rect.contains(r.center()))
            .map_or_else(|| panic!("no handle in {pane:?}"), |r| r.center())
    };
    let grip = grip_of(&harness, pane_delta);
    let tile =
        pane_rect(&harness, pane_alpha).center() + egui::vec2(-ui_egui::DOCK_TILE_PITCH, 0.0);
    hold(&mut harness, grip, tile);
    assert!(harness.state().drag_ghost().is_some());
    escape(&mut harness);
    unchanged(&harness, "a pane over a dock target");
    button(&mut harness, tile, false);
    settle(&mut harness);
    unchanged(&harness, "the button coming up on the dock target");

    // A pane that spring-loading took to gamma's workspace.
    let grip = grip_of(&harness, pane_delta);
    let on_tab = tab_rect(&harness, "gamma-folder").center() + egui::vec2(60.0, 0.0);
    hold(&mut harness, grip, on_tab);
    rest(&mut harness, 4);
    assert_eq!(shown(&harness), 2, "sprung to gamma");
    escape(&mut harness);
    unchanged(&harness, "a pane after spring-loading");
    button(&mut harness, on_tab, false);
    settle(&mut harness);
    unchanged(&harness, "the button coming up on the tab row");
}
