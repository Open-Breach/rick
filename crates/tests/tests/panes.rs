//! Acceptance (issue 60, panes): the toolbar's parent chip, breadcrumb,
//! view options button and grab handle, no tab bar over a pane, and the
//! view options menu — view modes, the hidden-files and folders-first
//! switches, refresh, and the per-type filter that narrows the list and
//! clears on navigation. Issue 80, rules 7 and 8: the ghost of a dragged
//! pane is its own snapshot at a quarter, anchored to the grab point, and
//! the cursor is the open hand over the grip and the closed hand for the
//! whole drag.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use model::{SettingsChange, Theme, ViewMode};
use platform::{ConfigDir, Fake};
use tests::local;
use ui_egui::{EguiHost, RickUi};

fn harness(start: &Path, size: egui::Vec2) -> Harness<'static, RickUi> {
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

/// The names the focused pane lists, in view order.
fn names(harness: &Harness<'_, RickUi>) -> Vec<String> {
    let model = harness.state().app().model();
    model
        .active_tab(model.focused)
        .map_or_default(|(_, t)| t.rows().map(|e| e.name.clone()).collect())
}

fn tab(harness: &Harness<'_, RickUi>) -> model::Tab {
    let model = harness.state().app().model();
    model
        .active_tab(model.focused)
        .map_or_else(|| panic!("no tab"), |(_, t)| t.clone())
}

/// Clicks the node labelled `label` and lets a frame pass.
fn click(harness: &mut Harness<'_, RickUi>, label: &str) {
    harness.get_by_label(label).click();
    harness.step();
    harness.step();
}

/// The topmost opaque fill painted under `at` in the last frame.
fn fill_under(harness: &Harness<'_, RickUi>, at: egui::Pos2) -> Option<egui::Color32> {
    fn walk(shape: &egui::Shape, at: egui::Pos2, found: &mut Option<egui::Color32>) {
        match shape {
            egui::Shape::Rect(r) if r.rect.contains(at) && r.fill.a() == 255 => {
                *found = Some(r.fill);
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    walk(shape, at, found);
                }
            }
            _ => {}
        }
    }
    let mut found = None;
    for clipped in &harness.output().shapes {
        if clipped.clip_rect.contains(at) {
            walk(&clipped.shape, at, &mut found);
        }
    }
    found
}

/// A folder with two SVG files, a PNG, a TOML file and a subfolder.
fn tree() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    for name in ["a.svg", "b.svg", "c.png", "Cargo.toml"] {
        fs::write(dir.path().join(name), name).unwrap_or_else(|e| panic!("{e}"));
    }
    fs::create_dir(dir.path().join("sub")).unwrap_or_else(|e| panic!("{e}"));
    dir
}

#[test]
fn the_toolbar_has_the_parent_chip_and_the_handle_and_no_tab_bar() {
    let dir = tree();
    let mut harness = harness(&dir.path().join("sub"), egui::vec2(1200.0, 700.0));
    let model = harness.state().app().model();
    let pane = model.focused;
    let (tab_id, _) = model.active_tab(pane).unwrap_or_else(|| panic!("tab"));
    let pane_rect = harness
        .state()
        .pane_rects()
        .into_iter()
        .find(|(p, _)| *p == pane)
        .map_or_else(|| panic!("no pane rect"), |(_, r)| r);
    // The toolbar is the first thing in the pane: no bar above it, only
    // its own panel margin.
    let row = harness
        .state()
        .toolbar_row(tab_id)
        .unwrap_or_else(|| panic!("no toolbar row"));
    assert!(
        row.top() - pane_rect.top() < 4.0,
        "toolbar {row:?} starts at the pane's top {pane_rect:?}"
    );
    // The handle and the view options button carry names; the old Up,
    // hidden-files and refresh buttons are gone from the toolbar.
    harness.get_by_label("Drag pane");
    harness.get_by_label("View options");
    assert!(harness.query_by_label("Refresh").is_none());

    // The parent chip goes up one folder.
    click(&mut harness, "Parent directory");
    settle(&mut harness);
    let model = harness.state().app().model();
    let location = model
        .active_tab(pane)
        .map_or_else(|| panic!("tab"), |(_, t)| t.location.clone());
    assert_eq!(
        platform::paths::to_path(&location.path).ok().as_deref(),
        Some(dir.path())
    );
}

#[test]
fn the_view_options_menu_switches_views_flips_switches_and_filters_types() {
    let dir = tree();
    let mut harness = harness(dir.path(), egui::vec2(1200.0, 700.0));
    assert_eq!(
        names(&harness),
        ["sub", "a.svg", "b.svg", "c.png", "Cargo.toml"]
    );

    // List view first, so the Type column's cells do not share the menu's
    // type labels; the menu closes on a view pick.
    click(&mut harness, "View options");
    click(&mut harness, "List");
    settle(&mut harness);
    assert_eq!(tab(&harness).view, ViewMode::List);
    assert!(harness.query_by_label("Refresh now").is_none(), "closed");

    // The menu: three titled groups with their rows.
    click(&mut harness, "View options");
    for label in [
        "Details",
        "List",
        "Icons",
        "Folders first",
        "Refresh now",
        "Show all",
        "Folders",
        "SVG",
        "PNG",
        "TOML",
    ] {
        harness.get_by_label(label);
    }
    // The footer's toggle carries the same name; the menu's row is a switch.
    harness.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Show hidden files");
    // Folders first off: the subfolder sorts among the files.
    click(&mut harness, "Folders first");
    settle(&mut harness);
    assert!(!harness.state().app().model().settings.folders_first);
    assert_eq!(
        names(&harness),
        ["a.svg", "b.svg", "c.png", "Cargo.toml", "sub"]
    );
    click(&mut harness, "Folders first");
    settle(&mut harness);
    assert!(harness.state().app().model().settings.folders_first);

    // The type filter: SVG only, then folders too, then everything.
    click(&mut harness, "SVG");
    settle(&mut harness);
    assert_eq!(names(&harness), ["a.svg", "b.svg"]);
    assert!(tab(&harness).types.is_some());
    click(&mut harness, "Folders");
    settle(&mut harness);
    assert_eq!(names(&harness), ["sub", "a.svg", "b.svg"]);
    click(&mut harness, "Show all");
    settle(&mut harness);
    assert!(tab(&harness).types.is_none());
    assert_eq!(names(&harness).len(), 5);

    // A filter on, then navigation clears it.
    click(&mut harness, "PNG");
    settle(&mut harness);
    assert_eq!(names(&harness), ["c.png"]);
    harness.key_press(egui::Key::Escape);
    harness.step();
    let pane = harness.state().app().model().focused;
    harness.state_mut().app_mut().handle(
        app::Command::Navigate {
            pane,
            to: local(&dir.path().join("sub")),
        },
        &mut Vec::new(),
    );
    settle(&mut harness);
    assert!(tab(&harness).types.is_none(), "cleared on navigation");

    // Show hidden files is the pane's toggle.
    fs::write(dir.path().join("sub/.hidden"), "x").unwrap_or_else(|e| panic!("{e}"));
    settle(&mut harness);
    assert!(!names(&harness).contains(&".hidden".to_owned()));
    click(&mut harness, "View options");
    harness
        .get_by_role_and_label(egui::accesskit::Role::CheckBox, "Show hidden files")
        .click();
    harness.step();
    harness.step();
    settle(&mut harness);
    assert!(tab(&harness).show_hidden);
    assert!(names(&harness).contains(&".hidden".to_owned()));
}

/// Issue 79 item 1: the toolbar row sits on the pane's `bg/pane`, the
/// colour of the active workspace tab, so the tab reads as attached to its
/// pane; only the column header band is tinted.
#[test]
fn the_toolbar_shares_the_panes_background_and_only_the_header_band_is_tinted() {
    let dir = tree();
    let mut harness = harness(dir.path(), egui::vec2(1200.0, 700.0));
    harness.state_mut().app_mut().handle(
        app::Command::Settings(SettingsChange::Theme(Theme::Light)),
        &mut Vec::new(),
    );
    harness.step();
    harness.step();
    let model = harness.state().app().model();
    let pane = model.focused;
    let (tab_id, _) = model.active_tab(pane).unwrap_or_else(|| panic!("tab"));
    let row = harness
        .state()
        .toolbar_row(tab_id)
        .unwrap_or_else(|| panic!("no toolbar row"));
    let band = harness
        .state()
        .header_band(tab_id)
        .unwrap_or_else(|| panic!("no header band"));
    let scheme = ui_egui::theme::Scheme::LIGHT;
    // The padding before Back has nothing but the pane under it.
    let probe = egui::pos2(row.left() + 3.0, row.center().y);
    assert_eq!(
        fill_under(&harness, probe),
        Some(scheme.bg_pane),
        "the toolbar at {probe:?}"
    );
    assert_eq!(
        fill_under(&harness, band.center()),
        Some(scheme.bg_subtle),
        "the header band at {:?}",
        band.center()
    );
}

/// Issue 79 item 5: in edit mode the path field has the control height of
/// Back and Forward, sits on their line and fills the breadcrumb's width.
#[test]
fn the_path_field_has_the_control_height_and_fills_the_breadcrumb() {
    let dir = tree();
    let mut harness = harness(dir.path(), egui::vec2(1200.0, 700.0));
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::L);
    harness.step();
    harness.step();
    let model = harness.state().app().model();
    let (tab_id, _) = model
        .active_tab(model.focused)
        .unwrap_or_else(|| panic!("tab"));
    let density = model.settings.density;
    let row = harness
        .state()
        .toolbar_row(tab_id)
        .unwrap_or_else(|| panic!("no toolbar row"));
    let rects = harness.state().toolbar_rects(tab_id);
    let back = rects
        .iter()
        .find(|(g, _)| *g == "left")
        .map_or_else(|| panic!("no Back: {rects:?}"), |(_, r)| *r);
    let middle = rects
        .iter()
        .find(|(g, _)| *g == "middle")
        .map_or_else(|| panic!("no breadcrumb: {rects:?}"), |(_, r)| *r);
    let field = harness
        .get_all_by_role(egui::accesskit::Role::TextInput)
        .map(|n| n.rect())
        .find(|r| row.contains(r.center()))
        .unwrap_or_else(|| panic!("no path field in the toolbar {row:?}"));
    let size = ui_egui::theme::Metrics::of(density).button_size;
    assert!(
        (field.height() - size).abs() < 0.5,
        "field {field:?} against {size}"
    );
    assert!(
        (field.center().y - back.center().y).abs() < 0.5,
        "field {field:?} on the line of Back {back:?}"
    );
    assert!(
        (field.left() - middle.left()).abs() < 1.0 && (field.width() - middle.width()).abs() < 1.0,
        "field {field:?} fills the breadcrumb {middle:?}"
    );
    // Escape leaves edit mode and brings the breadcrumb back.
    harness.key_press(egui::Key::Escape);
    harness.step();
    harness.step();
    assert!(
        harness
            .get_all_by_role(egui::accesskit::Role::TextInput)
            .all(|n| !row.contains(n.rect().center())),
        "the field is gone"
    );
}

/// The text field inside the toolbar `row`, the path field, if one is
/// shown.
fn toolbar_field(harness: &Harness<'_, RickUi>, row: egui::Rect) -> Option<egui::Rect> {
    harness
        .get_all_by_role(egui::accesskit::Role::TextInput)
        .map(|n| n.rect())
        .find(|r| row.contains(r.center()))
}

/// The text of the path field inside the toolbar `row`.
fn toolbar_field_value(harness: &Harness<'_, RickUi>, row: egui::Rect) -> Option<String> {
    harness
        .get_all_by_role(egui::accesskit::Role::TextInput)
        .find(|n| row.contains(n.rect().center()))
        .and_then(|n| n.value())
}

/// Issue 79 item 10 (SPEC.md *Path bar*, *Editing*): the path field is
/// `bg/control` with an `accent/focus` ring, the whole path is selected on
/// entry so typing replaces it, Escape restores the breadcrumbs unchanged
/// and so does losing the focus; a click on the empty part of the
/// breadcrumb bar opens the field too.
#[test]
// One scenario: the look, the selection, Escape, the click and the focus.
#[allow(clippy::too_many_lines)]
fn the_path_field_is_styled_selects_the_path_and_cancels_on_escape_or_focus_loss() {
    let dir = tree();
    let mut harness = harness(dir.path(), egui::vec2(1200.0, 700.0));
    harness.state_mut().app_mut().handle(
        app::Command::Settings(SettingsChange::Theme(Theme::Light)),
        &mut Vec::new(),
    );
    harness.step();
    harness.step();
    let model = harness.state().app().model();
    let (tab_id, before) = model
        .active_tab(model.focused)
        .unwrap_or_else(|| panic!("tab"));
    let location = before.location.clone();
    let row = harness
        .state()
        .toolbar_row(tab_id)
        .unwrap_or_else(|| panic!("no toolbar row"));

    // Mod+L: the field in the design's colours.
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::L);
    harness.step();
    harness.step();
    let field = toolbar_field(&harness, row).unwrap_or_else(|| panic!("no path field"));
    let scheme = ui_egui::theme::Scheme::LIGHT;
    assert_eq!(
        fill_under(&harness, egui::pos2(field.right() - 4.0, field.center().y)),
        Some(scheme.bg_control),
        "bg/control behind the path"
    );
    let ring = harness
        .output()
        .shapes
        .iter()
        .any(|clipped| match &clipped.shape {
            egui::Shape::Rect(r) => {
                r.stroke.color == scheme.accent_focus
                    && (r.stroke.width - 1.0).abs() < 0.01
                    && r.rect.expand(1.0).contains_rect(field)
                    && field.expand(1.0).contains_rect(r.rect)
            }
            _ => false,
        });
    assert!(ring, "a 1 px accent/focus ring around {field:?}");

    // The whole path is selected: typing replaces it.
    harness.event(egui::Event::Text("x".to_owned()));
    harness.step();
    harness.step();
    let value = toolbar_field_value(&harness, row).unwrap_or_default();
    assert_eq!(value, "x", "the typed text replaces the selected path");

    // Escape: the breadcrumbs are back and nothing moved.
    harness.key_press(egui::Key::Escape);
    harness.step();
    harness.step();
    assert!(toolbar_field(&harness, row).is_none(), "the field is gone");
    harness.get_by_label("Parent directory");
    let model = harness.state().app().model();
    assert_eq!(
        model
            .active_tab(model.focused)
            .map(|(_, t)| t.location.clone()),
        Some(location.clone())
    );

    // A click on the empty part of the breadcrumb bar opens the field; a
    // click elsewhere takes the focus and cancels it.
    let rects = harness.state().toolbar_rects(tab_id);
    let middle = rects
        .iter()
        .find(|(g, _)| *g == "middle")
        .map_or_else(|| panic!("no breadcrumb: {rects:?}"), |(_, r)| *r);
    let empty = egui::pos2(middle.right() - 60.0, middle.center().y);
    harness.event(egui::Event::PointerMoved(empty));
    harness.step();
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos: empty,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        });
        harness.step();
    }
    harness.step();
    assert!(
        toolbar_field(&harness, row).is_some(),
        "a click on the empty breadcrumb edits"
    );
    let list = harness.get_by_label("Cargo.toml").rect().center();
    harness.event(egui::Event::PointerMoved(list));
    harness.step();
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos: list,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        });
        harness.step();
    }
    harness.step();
    harness.step();
    assert!(
        toolbar_field(&harness, row).is_none(),
        "losing the focus cancels"
    );
    let model = harness.state().app().model();
    assert_eq!(
        model
            .active_tab(model.focused)
            .map(|(_, t)| t.location.clone()),
        Some(location)
    );
}

/// The rectangle of `pane` in the last frame.
fn rect_of(harness: &Harness<'_, RickUi>, pane: model::PaneId) -> egui::Rect {
    harness
        .state()
        .pane_rects()
        .iter()
        .find(|(p, _)| *p == pane)
        .map_or_else(|| panic!("no rect for {pane:?}"), |(_, r)| *r)
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

/// Every text shape the last frame painted, with the rectangle it covers.
fn texts(harness: &Harness<'_, RickUi>) -> Vec<(String, egui::Rect)> {
    fn walk(shape: &egui::Shape, clip: egui::Rect, found: &mut Vec<(String, egui::Rect)>) {
        match shape {
            egui::Shape::Text(text) => {
                let rect = text.galley.rect.translate(text.pos.to_vec2());
                found.push((text.galley.job.text.clone(), rect.intersect(clip)));
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    walk(shape, clip, found);
                }
            }
            _ => {}
        }
    }
    let mut found = Vec::new();
    for clipped in &harness.output().shapes {
        walk(&clipped.shape, clipped.clip_rect, &mut found);
    }
    found
}

/// Issue 80, rules 7 and 8 (SPEC.md *Panes*, *Drag ghost*): the grip shows
/// the open hand; while the pane is dragged by it the ghost is the pane's
/// own snapshot, a quarter of its size at 70 % opacity inside a 1 px
/// `border/strong` border, its scaled grip 8 px down and right of the
/// cursor wherever the cursor goes; the cursor is the closed hand over
/// everything, never an arrow; a screen reader is told of the drag; and a
/// release beside the dock targets leaves the layout as it was.
#[test]
// One gesture from the hover to the release.
#[allow(clippy::too_many_lines)]
fn a_dragged_pane_is_followed_by_its_own_snapshot_and_the_closed_hand() {
    let dir = tree();
    let other = dir.path().join("sub");
    fs::write(other.join("inside.txt"), "x").unwrap_or_else(|e| panic!("{e}"));
    let mut harness = harness(dir.path(), egui::vec2(1400.0, 800.0));
    harness.state_mut().app_mut().handle(
        app::Command::Settings(SettingsChange::Theme(Theme::Light)),
        &mut Vec::new(),
    );
    let pane_a = harness.state().app().model().focused;
    harness.state_mut().app_mut().handle(
        app::Command::Split {
            pane: pane_a,
            dir: model::SplitDir::Right,
        },
        &mut Vec::new(),
    );
    settle(&mut harness);
    let pane_b = harness.state().app().model().focused;
    assert_ne!(pane_b, pane_a);
    harness.state_mut().app_mut().handle(
        app::Command::Navigate {
            pane: pane_b,
            to: local(&other),
        },
        &mut Vec::new(),
    );
    settle(&mut harness);
    let layout = harness.state().app().model().dock().clone();
    let rect_a = rect_of(&harness, pane_a);
    let rect_b = rect_of(&harness, pane_b);
    let grip = harness
        .query_all_by_label("Drag pane")
        .map(|n| n.rect())
        .find(|r| rect_b.contains(r.center()))
        .unwrap_or_else(|| panic!("no handle in pane B"));

    // Rule 8: the open hand over the grip, and no ghost before a drag.
    move_to(&mut harness, grip.center());
    harness.step();
    assert_eq!(cursor(&harness), egui::CursorIcon::Grab);
    assert_eq!(harness.state().drag_ghost(), None);

    // Grabbed a little off the grip's centre, so the anchor is told from it.
    let grab = grip.center() + egui::vec2(3.0, -4.0);
    move_to(&mut harness, grab);
    button(&mut harness, grab, true);
    for i in 1..=4u8 {
        move_to(&mut harness, grab + egui::vec2(-4.0, 3.0) * f32::from(i));
    }

    // Where the cursor goes: over the other pane's list, its column
    // header, its filter field, the title row, the side panel, and back
    // over the dragged pane itself.
    let tab_a = harness
        .state()
        .app()
        .model()
        .active_tab(pane_a)
        .map_or_else(|| panic!("tab a"), |(id, _)| id);
    let boundary = harness.state().column_boundaries(tab_a);
    let band = harness
        .state()
        .header_band(tab_a)
        .unwrap_or_else(|| panic!("no header band"));
    let footer = harness
        .state()
        .footer_row(tab_a)
        .unwrap_or_else(|| panic!("no footer"));
    let row = harness
        .state()
        .tab_row()
        .unwrap_or_else(|| panic!("no tab row"));
    let stops = [
        rect_a.center() + egui::vec2(-130.0, 150.0),
        egui::pos2(boundary[0], band.center().y),
        egui::pos2(footer.right() - 60.0, footer.center().y),
        egui::pos2(row.right() - 20.0, row.center().y - 3.0),
        egui::pos2(40.0, 300.0),
        rect_b.center() + egui::vec2(20.0, 200.0),
    ];
    let scheme = ui_egui::theme::Scheme::LIGHT;
    let size = rect_b.size() * ui_egui::drag::GHOST_SCALE;
    let anchor = (grab - rect_b.min) * ui_egui::drag::GHOST_SCALE;
    for stop in stops {
        move_to(&mut harness, stop);
        harness.step();
        // Rule 8: the closed hand, whatever is under the cursor.
        assert_eq!(cursor(&harness), egui::CursorIcon::Grabbing, "at {stop:?}");

        // Rule 7: a quarter of the pane, its scaled grab point 8 px down
        // and right of the cursor (on the pixel grid).
        let ghost = harness
            .state()
            .drag_ghost()
            .unwrap_or_else(|| panic!("no ghost at {stop:?}"));
        assert_eq!(ghost.of, ui_egui::drag::Dragged::Pane);
        assert!(
            (ghost.rect.size() - size).length() < 0.01,
            "{:?} is a quarter of {rect_b:?}",
            ghost.rect
        );
        let held = ghost.rect.min + anchor;
        let wanted = stop + ui_egui::drag::GHOST_OFFSET;
        assert!(
            (held - wanted).abs().max_elem() <= 0.5,
            "the grip of the ghost at {held:?}, the cursor at {stop:?}"
        );

        // The pane itself, scaled: its file name and its breadcrumb are
        // in the ghost, a quarter as tall as text is, and so is far more
        // than a title and five bars.
        assert!(ghost.shapes > 20, "{} shapes", ghost.shapes);
        let small: Vec<(String, egui::Rect)> = texts(&harness)
            .into_iter()
            .filter(|(_, r)| ghost.rect.expand(0.5).contains_rect(*r) && r.height() < 6.0)
            .collect();
        for name in ["inside.txt", "sub"] {
            assert!(
                small.iter().any(|(text, _)| text == name),
                "{name} in the ghost at {stop:?}: {small:?}"
            );
        }

        // Its frame: the 1 px border/strong border at 70 %.
        let border = harness
            .output()
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::Rect(r)
                    if (r.rect.min - ghost.rect.min).length() < 0.01
                        && (r.rect.size() - size).length() < 0.01
                        && r.stroke.width > 0.0 =>
                {
                    Some(r.stroke)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("no border around the ghost at {stop:?}"));
        assert!((border.width - 1.0).abs() < 0.01);
        let faded = scheme
            .border_strong
            .gamma_multiply(ui_egui::drag::GHOST_OPACITY);
        assert_eq!(border.color, faded, "border/strong at 70 %");

        // What a screen reader is told.
        harness.get_by_label("Dragging pane sub");
    }

    // Released beside the dock targets: nothing moved, the ghost is gone
    // and so is the closed hand.
    let away = egui::pos2(rect_a.center().x, rect_a.bottom() - 60.0);
    move_to(&mut harness, away);
    button(&mut harness, away, false);
    settle(&mut harness);
    assert_eq!(*harness.state().app().model().dock(), layout);
    assert_eq!(harness.state().drag_ghost(), None);
    assert_ne!(cursor(&harness), egui::CursorIcon::Grabbing);
    assert!(harness.query_by_label("Dragging pane sub").is_none());
}
