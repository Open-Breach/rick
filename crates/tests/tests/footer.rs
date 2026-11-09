//! Acceptance (issue 60, footer): the footer row is the toolbar's height
//! under the list; the hidden-files toggle, the folder and file counts,
//! the selection pill, the counts while a filter narrows the list, and
//! Escape clearing the filter.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use model::{Density, SelectionChange};
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

/// Two folders, three files of 5, 6 and 7 bytes, and a hidden file.
fn tree() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    for name in ["alpha", "beta"] {
        fs::create_dir(dir.path().join(name)).unwrap_or_else(|e| panic!("{e}"));
    }
    for (name, bytes) in [("apple.txt", 5), ("banana.md", 6), ("cherry.rs", 7)] {
        fs::write(dir.path().join(name), vec![b'x'; bytes]).unwrap_or_else(|e| panic!("{e}"));
    }
    fs::write(dir.path().join(".hidden"), "x").unwrap_or_else(|e| panic!("{e}"));
    dir
}

fn entry_id(harness: &Harness<'_, RickUi>, name: &str) -> types::EntryId {
    let pane = harness.state().app().model().focused;
    harness
        .state()
        .app()
        .model()
        .active_tab(pane)
        .and_then(|(_, t)| t.rows().find(|e| e.name == name).map(|e| e.id))
        .unwrap_or_else(|| panic!("no entry {name}"))
}

/// The horizontal hairlines painted in the last frame within `band`, a
/// strip of the window: egui draws them as line segments.
fn hairlines_in(harness: &Harness<'_, RickUi>, band: egui::Rect) -> usize {
    fn walk(shape: &egui::Shape, band: egui::Rect, count: &mut usize) {
        match shape {
            egui::Shape::LineSegment { points, .. }
                if (points[0].y - points[1].y).abs() < 0.5
                    && band.y_range().contains(points[0].y)
                    && points[0].x.min(points[1].x) <= band.center().x
                    && points[0].x.max(points[1].x) >= band.center().x =>
            {
                *count += 1;
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    walk(shape, band, count);
                }
            }
            _ => {}
        }
    }
    let mut count = 0;
    for clipped in &harness.output().shapes {
        walk(&clipped.shape, band, &mut count);
    }
    count
}

/// Issue 79 item 4: the footer is the Figma status bar's height (the
/// hairline, the 26 px field and the toolbar padding; QUESTIONS.md 254),
/// not the toolbar's, and one hairline, its own, separates it from the
/// list — egui's panel separator had drawn a second one.
#[test]
fn the_footer_is_the_status_bars_height_under_one_hairline_and_counts_the_folder() {
    let dir = tree();
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut harness = harness(dir.path(), &fake);
    let pane = harness.state().app().model().focused;
    let tab = harness
        .state()
        .app()
        .model()
        .active_tab(pane)
        .map_or_else(|| panic!("no tab"), |(id, _)| id);
    // The row: as tall as the toolbar, below the last row of the list.
    let row = harness
        .state()
        .footer_row(tab)
        .unwrap_or_else(|| panic!("no footer"));
    let footer = ui_egui::footer_height(Density::Comfortable);
    assert!(
        (row.height() - footer).abs() < 0.5,
        "{} against {footer}",
        row.height()
    );
    let last = harness.get_by_label("cherry.rs").rect();
    assert!(row.top() >= last.bottom(), "under the list");
    // One hairline at the footer's top edge, and no other line near it.
    let edge = egui::Rect::from_x_y_ranges(row.x_range(), row.top() - 4.0..=row.top() + 4.0);
    assert_eq!(hairlines_in(&harness, edge), 1, "hairlines at {edge:?}");
    // The toggle is the field's height, not a tool button's.
    let toggle = harness.get_by_label("Show hidden files").rect();
    assert!(
        (toggle.height() - 26.0).abs() < 0.5,
        "toggle {toggle:?} is 26 px tall"
    );
    // The counts, read aloud with their nouns; no pill without a selection.
    harness.get_by_label("2 folders");
    harness.get_by_label("3 files");
    harness.get_by_label("18 B in files");
    assert!(harness.query_by_label_contains("selected").is_none());
    // A selection brings the pill.
    let apple = entry_id(&harness, "apple.txt");
    command(
        &mut harness,
        app::Command::Select {
            pane,
            change: SelectionChange::Set(vec![apple]),
        },
    );
    harness.get_by_label("1 selected · 5 B");
    // The hidden-files toggle shows the dot-file and names its new state.
    harness.get_by_label("Show hidden files").click();
    harness.step();
    settle(&mut harness);
    assert!(
        harness
            .state()
            .app()
            .model()
            .active_tab(pane)
            .is_some_and(|(_, t)| t.show_hidden)
    );
    harness.get_by_label("Hide hidden files");
    harness.get_by_label("4 files");
    harness.get_by_label(".hidden");
}

#[test]
fn a_filter_narrows_the_counts_and_escape_clears_it() {
    let dir = tree();
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut harness = harness(dir.path(), &fake);
    let pane = harness.state().app().model().focused;
    command(
        &mut harness,
        app::Command::Filter {
            pane,
            text: "an".to_owned(),
        },
    );
    harness.get_by_label("banana.md");
    assert!(harness.query_by_label("apple.txt").is_none());
    harness.get_by_label("0 of 2 folders");
    harness.get_by_label("1 of 3 files");
    // Escape clears the filter; the counts are whole again.
    harness.key_press(egui::Key::Escape);
    harness.step();
    settle(&mut harness);
    assert!(
        harness
            .state()
            .app()
            .model()
            .active_tab(pane)
            .is_some_and(|(_, t)| t.filter.is_empty())
    );
    harness.get_by_label("3 files");
    harness.get_by_label("apple.txt");
}
