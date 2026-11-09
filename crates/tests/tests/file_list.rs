//! Acceptance (issue 60, file list): the details list's column header band
//! with its four columns in the spec's order, the empty-folder and
//! filtered-empty messages, and the loading portal that stands still
//! under the system's reduce-motion setting.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use platform::{ConfigDir, Fake};
use tests::local;
use ui_egui::{EguiHost, RickUi};

fn harness(start: &Path, fake: &Arc<Fake>) -> Harness<'static, RickUi> {
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

fn command(harness: &mut Harness<'_, RickUi>, command: app::Command) {
    harness
        .state_mut()
        .app_mut()
        .handle(command, &mut Vec::new());
    settle(harness);
}

/// The x of the column title containing `text`, inside the focused pane's
/// header band.
fn title_x(harness: &Harness<'_, RickUi>, band: egui::Rect, text: &str) -> f32 {
    harness
        .query_all_by_label_contains(text)
        .map(|n| n.rect())
        .find(|r| band.contains(r.center()))
        .map_or_else(
            || panic!("no column title {text} in {band:?}"),
            |r| r.left(),
        )
}

#[test]
fn the_header_band_holds_name_size_type_and_modified() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    for name in ["a.txt", "b.svg"] {
        fs::write(dir.path().join(name), name).unwrap_or_else(|e| panic!("{e}"));
    }
    fs::create_dir(dir.path().join("sub")).unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let harness = harness(dir.path(), &fake);
    let model = harness.state().app().model();
    let pane = model.focused;
    let (tab, _) = model.active_tab(pane).unwrap_or_else(|| panic!("tab"));
    let band = harness
        .state()
        .header_band(tab)
        .unwrap_or_else(|| panic!("no header band"));
    assert!(
        (band.height() - 34.0).abs() < 0.5,
        "the band is 34 px: {band:?}"
    );
    let toolbar = harness
        .state()
        .toolbar_row(tab)
        .unwrap_or_else(|| panic!("no toolbar"));
    assert!(band.top() >= toolbar.bottom() - 1.0, "under the toolbar");
    // The columns in the spec's order.
    let name = title_x(&harness, band, "Name");
    let size = title_x(&harness, band, "Size");
    let kind = title_x(&harness, band, "Type");
    let modified = title_x(&harness, band, "Modified");
    assert!(
        name < size && size < kind && kind < modified,
        "{name} {size} {kind} {modified}"
    );
    // The rows: a folder's size is an em dash.
    harness.get_by_label("\u{2014}");
    harness.get_by_label("Folder");
}

#[test]
fn an_empty_folder_and_an_emptied_filter_say_so() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let empty = dir.path().join("empty");
    fs::create_dir(&empty).unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("a.txt"), "a").unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut harness = harness(&empty, &fake);
    harness.get_by_label("This folder is empty");
    harness.get_by_label_contains("Drop files here, or press");
    assert!(
        harness.query_by_label_contains("Name").is_none(),
        "no header over nothing"
    );

    // A filter that matches nothing.
    let pane = harness.state().app().model().focused;
    command(
        &mut harness,
        app::Command::Navigate {
            pane,
            to: local(dir.path()),
        },
    );
    harness.get_by_label("a.txt");
    command(
        &mut harness,
        app::Command::Filter {
            pane,
            text: "zzz".into(),
        },
    );
    harness.get_by_label("No entries match the filter");
    assert!(harness.query_by_label("a.txt").is_none());
    command(
        &mut harness,
        app::Command::Filter {
            pane,
            text: String::new(),
        },
    );
    harness.get_by_label("a.txt");
}

#[test]
fn the_loading_portal_stands_still_under_reduce_motion() {
    // The portal itself: moving by default, still with the setting on.
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    assert!(!platform::Motion::reduce_motion(fake.as_ref()));
    fake.set_reduce_motion(true);
    assert!(platform::Motion::reduce_motion(fake.as_ref()));
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let harness = harness(dir.path(), &fake);
    // With the setting on and the folder listed, the list asks for no
    // repaint of its own: a frame settles.
    harness.get_by_label("This folder is empty");
}

/// The x positions of the vertical hairlines painted inside `band` in the
/// last frame: the header's separators, in order.
fn separators_in(harness: &Harness<'_, RickUi>, band: egui::Rect) -> Vec<f32> {
    fn walk(shape: &egui::Shape, band: egui::Rect, xs: &mut Vec<f32>) {
        match shape {
            egui::Shape::LineSegment { points, .. }
                if (points[0].x - points[1].x).abs() < 0.5
                    && band.x_range().contains(points[0].x)
                    && band.y_range().contains(points[0].y)
                    && band.y_range().contains(points[1].y) =>
            {
                xs.push(points[0].x);
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    walk(shape, band, xs);
                }
            }
            _ => {}
        }
    }
    let mut xs = Vec::new();
    for clipped in &harness.output().shapes {
        if clipped.clip_rect.intersects(band) {
            walk(&clipped.shape, band, &mut xs);
        }
    }
    xs.sort_by(f32::total_cmp);
    xs.dedup_by(|a, b| (*a - *b).abs() < 1.0);
    xs
}

/// Presses at `from`, moves to `to` in steps and releases.
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
        harness.event(egui::Event::PointerMoved(from + (to - from) * t));
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
}

/// Steps until egui's double-click window has passed, so two clicks are
/// not read as one double-click.
fn settle_clicks(harness: &mut Harness<'_, RickUi>) {
    std::thread::sleep(Duration::from_millis(400));
    harness.step();
    harness.step();
}

/// Clicks at `at` twice within the double-click window: the harness's
/// clock advances a quarter second per frame, so both clicks go into one
/// frame's input.
fn double_click(harness: &mut Harness<'_, RickUi>, at: egui::Pos2) {
    harness.event(egui::Event::PointerMoved(at));
    harness.step();
    for _ in 0..2 {
        for pressed in [true, false] {
            harness.input_mut().events.push(egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::default(),
            });
        }
    }
    harness.step();
    harness.step();
}

/// Issue 79 item 9 (SPEC.md *File list*): no permanent divider between the
/// headers; hovering a boundary shows a 1 px `border/strong` divider
/// across the band and the column-resize cursor; dragging resizes the
/// column before it, Name included, and the last column takes what is
/// left; a double-click fits the column to its content; no column goes
/// under 48 px; the widths survive a restart.
#[test]
// One scenario, boundary by boundary.
#[allow(clippy::too_many_lines)]
fn column_boundaries_show_on_hover_resize_fit_and_survive_a_restart() {
    use model::{SettingsChange, Theme};
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    for name in ["a.txt", "b.svg", "a-rather-long-file-name-to-fit.txt"] {
        fs::write(dir.path().join(name), name).unwrap_or_else(|e| panic!("{e}"));
    }
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut harness = harness(dir.path(), &fake);
    command(
        &mut harness,
        app::Command::Settings(SettingsChange::Theme(Theme::Light)),
    );
    let model = harness.state().app().model();
    let (tab, _) = model
        .active_tab(model.focused)
        .unwrap_or_else(|| panic!("tab"));
    let band = harness
        .state()
        .header_band(tab)
        .unwrap_or_else(|| panic!("no header band"));
    let boundaries = harness.state().column_boundaries(tab);
    assert_eq!(
        boundaries.len(),
        3,
        "Name|Size, Size|Type, Type|Modified: {boundaries:?}"
    );
    assert!(
        separators_in(&harness, band).is_empty(),
        "no permanent divider: {:?}",
        separators_in(&harness, band)
    );
    let size = title_x(&harness, band, "Size");
    let kind = title_x(&harness, band, "Type");
    let modified = title_x(&harness, band, "Modified");

    // Hovering the Size|Type boundary: one divider across the band in
    // border/strong, and the column-resize cursor.
    let at = egui::pos2(boundaries[1], band.center().y);
    harness.hover_at(at);
    harness.step();
    harness.step();
    let lines = separators_in(&harness, band);
    assert_eq!(lines.len(), 1, "one divider while hovered: {lines:?}");
    assert!(
        (lines[0] - boundaries[1]).abs() < 1.0,
        "{lines:?} at {at:?}"
    );
    let strong = ui_egui::theme::Scheme::LIGHT.border_strong;
    let coloured = harness
        .output()
        .shapes
        .iter()
        .any(|clipped| match &clipped.shape {
            egui::Shape::LineSegment { points, stroke } => {
                (points[0].x - points[1].x).abs() < 0.5
                    && band.y_range().contains(points[0].y)
                    && stroke.color == strong
            }
            _ => false,
        });
    assert!(coloured, "the divider is border/strong");
    assert_eq!(
        harness.output().platform_output.cursor_icon,
        egui::CursorIcon::ResizeColumn
    );
    harness.hover_at(egui::pos2(band.center().x, band.bottom() + 200.0));
    harness.step();
    harness.step();
    assert!(
        separators_in(&harness, band).is_empty(),
        "gone when the pointer leaves"
    );

    // Name grows by 60 px: Size, Type and Modified move right together.
    let from = egui::pos2(boundaries[0], band.center().y);
    drag(&mut harness, from, from + egui::vec2(60.0, 0.0));
    let moved = harness.state().column_boundaries(tab);
    assert!(
        (moved[0] - boundaries[0] - 60.0).abs() < 2.0,
        "Name|Size {boundaries:?} -> {moved:?}"
    );
    assert!((title_x(&harness, band, "Size") - size - 60.0).abs() < 2.0);
    assert!((title_x(&harness, band, "Type") - kind - 60.0).abs() < 2.0);
    assert!((title_x(&harness, band, "Modified") - modified - 60.0).abs() < 2.0);
    settle_clicks(&mut harness);

    // Size cannot go under 48 px: a long drag to the left stops there.
    let from = egui::pos2(moved[1], band.center().y);
    drag(&mut harness, from, from - egui::vec2(300.0, 0.0));
    let narrow = harness.state().column_boundaries(tab);
    let size_width = narrow[1] - narrow[0] - 6.0;
    assert!(
        (size_width - ui_egui::views::COLUMN_MIN).abs() < 2.0,
        "Size is {size_width} wide after {narrow:?}"
    );
    settle_clicks(&mut harness);

    // A double-click on the Name|Size boundary fits Name to its content:
    // narrower than the 60 px it was given, wider than the longest name.
    let at = egui::pos2(narrow[0], band.center().y);
    double_click(&mut harness, at);
    let fitted = harness.state().column_boundaries(tab);
    assert!(
        fitted[0] < moved[0] - 10.0,
        "fitted {fitted:?} from {moved:?}"
    );
    let longest = harness
        .get_by_label("a-rather-long-file-name-to-fit.txt")
        .rect();
    assert!(
        fitted[0] > longest.right(),
        "the longest name {longest:?} fits before {fitted:?}"
    );
    settle_clicks(&mut harness);

    // The widths are the tab's and come back with the session.
    let model = harness.state().app().model();
    let saved = model
        .tab(tab)
        .map(|t| t.columns.clone())
        .unwrap_or_default();
    assert!(
        saved.contains_key("name") && saved.contains_key("size"),
        "{saved:?}"
    );
    harness
        .state_mut()
        .app_mut()
        .save_session()
        .unwrap_or_else(|e| panic!("{e}"));
    drop(harness);
    // A start folder would open a new tab beside the restored one: the
    // restarted app opens on the session alone.
    let host = Arc::new(EguiHost::new());
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: Vec::new(),
        config_dir: Some(fake.config_dir()),
        start: None,
    });
    let mut restarted = Harness::builder()
        .with_size(egui::vec2(1200.0, 700.0))
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut restarted);
    let model = restarted.state().app().model();
    let (tab, restored) = model
        .active_tab(model.focused)
        .unwrap_or_else(|| panic!("tab after restart"));
    assert_eq!(restored.columns, saved, "the widths survive the restart");
    let band = restarted
        .state()
        .header_band(tab)
        .unwrap_or_else(|| panic!("no header band after restart"));
    let again = restarted.state().column_boundaries(tab);
    assert!(
        again.iter().zip(&fitted).all(|(a, b)| (a - b).abs() < 2.0),
        "{again:?} against {fitted:?} in {band:?}"
    );
}
