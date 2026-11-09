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

/// Issue 79 item 6: a separator stands between every pair of titles, not
/// only between Size and Type, and every column resizes by its boundary,
/// Name included; the last column takes the remaining width.
#[test]
fn every_column_resizes_name_included_and_every_boundary_has_a_separator() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    for name in ["a.txt", "b.svg"] {
        fs::write(dir.path().join(name), name).unwrap_or_else(|e| panic!("{e}"));
    }
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut harness = harness(dir.path(), &fake);
    let model = harness.state().app().model();
    let (tab, _) = model
        .active_tab(model.focused)
        .unwrap_or_else(|| panic!("tab"));
    let band = harness
        .state()
        .header_band(tab)
        .unwrap_or_else(|| panic!("no header band"));
    let separators = separators_in(&harness, band);
    assert_eq!(
        separators.len(),
        3,
        "Name|Size, Size|Type and Type|Modified: {separators:?}"
    );
    let size = title_x(&harness, band, "Size");
    let kind = title_x(&harness, band, "Type");
    let modified = title_x(&harness, band, "Modified");
    assert!(separators[0] < size && size < separators[1]);
    assert!(separators[1] < kind && kind < separators[2]);
    assert!(separators[2] < modified);

    // Name grows by 60 px: Size, Type and Modified move right together
    // and Modified, the last column, gives the room.
    let from = egui::pos2(separators[0], band.center().y);
    drag(&mut harness, from, from + egui::vec2(60.0, 0.0));
    let moved = separators_in(&harness, band);
    assert_eq!(moved.len(), 3, "{moved:?}");
    assert!(
        (moved[0] - separators[0] - 60.0).abs() < 2.0,
        "Name|Size {separators:?} -> {moved:?}"
    );
    assert!((title_x(&harness, band, "Size") - size - 60.0).abs() < 2.0);
    assert!((title_x(&harness, band, "Type") - kind - 60.0).abs() < 2.0);
    assert!((title_x(&harness, band, "Modified") - modified - 60.0).abs() < 2.0);

    // Type shrinks by 30 px from its right boundary: only Modified moves.
    let from = egui::pos2(moved[2], band.center().y);
    drag(&mut harness, from, from - egui::vec2(30.0, 0.0));
    let shrunk = separators_in(&harness, band);
    assert!(
        (shrunk[2] - moved[2] + 30.0).abs() < 2.0,
        "Type|Modified {moved:?} -> {shrunk:?}"
    );
    assert!((title_x(&harness, band, "Type") - kind - 60.0).abs() < 2.0);
    assert!((title_x(&harness, band, "Modified") - modified - 30.0).abs() < 2.0);
}
