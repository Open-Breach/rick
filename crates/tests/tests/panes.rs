//! Acceptance (issue 60, panes): the toolbar's parent chip, breadcrumb,
//! view options button and grab handle, no tab bar over a pane, and the
//! view options menu — view modes, the hidden-files and folders-first
//! switches, refresh, and the per-type filter that narrows the list and
//! clears on navigation.

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
        .map(|(_, t)| t.rows().map(|e| e.name.clone()).collect())
        .unwrap_or_default()
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
