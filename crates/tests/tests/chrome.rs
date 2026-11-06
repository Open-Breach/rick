//! Acceptance (GUI chrome): the title row's search box, the toolbar at
//! three widths, a tab dragged between panes, the context menu on empty
//! space.

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

fn selection_len(harness: &Harness<'_, RickUi>) -> usize {
    let app = harness.state().app();
    let pane = app.model().focused;
    app.model()
        .active_tab(pane)
        .map_or(0, |(_, t)| t.selection.len())
}

/// Mod+P focuses the box empty, `>` lists the commands, Enter runs the
/// chosen one, Escape gives the keyboard back to the list.
#[test]
fn the_search_box_searches_names_and_runs_commands() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    for name in ["alpha.txt", "beta.txt", "gamma.txt"] {
        fs::write(dir.path().join(name), name).unwrap_or_else(|e| panic!("{e}"));
    }
    let mut harness = harness(dir.path(), egui::vec2(1100.0, 720.0));

    // By name: typing filters the current location; Enter selects the hit.
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::P);
    harness.step();
    assert!(harness.state().search_active());
    assert_eq!(harness.state().search_text(), "");
    harness.event(egui::Event::Text("gam".to_owned()));
    harness.step();
    // The list row and the dropdown's hit; the others only in the list.
    assert_eq!(harness.query_all_by_label_contains("gamma.txt").count(), 2);
    assert_eq!(harness.query_all_by_label_contains("alpha.txt").count(), 1);
    harness.key_press(egui::Key::Enter);
    harness.step();
    harness.step();
    assert!(!harness.state().search_active(), "a pick closes the box");
    assert_eq!(selection_len(&harness), 1);

    // Commands: `>` lists Actions; Enter runs the highlighted one.
    harness.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::P,
    );
    harness.step();
    assert!(harness.state().search_active());
    assert_eq!(harness.state().search_text(), ">");
    harness.get_by_label_contains("Toggle side panel");
    harness.event(egui::Event::Text("toggle side".to_owned()));
    harness.step();
    harness.get_by_label_contains("Toggle side panel");
    assert!(harness.state().app().model().settings.side_panel.visible);
    harness.key_press(egui::Key::Enter);
    harness.step();
    harness.step();
    assert!(
        !harness.state().app().model().settings.side_panel.visible,
        "Enter ran Toggle side panel"
    );
    assert!(!harness.state().search_active());

    // Escape clears and hands the keyboard back: ArrowDown moves the list.
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::P);
    harness.step();
    harness.event(egui::Event::Text("alp".to_owned()));
    harness.step();
    harness.key_press(egui::Key::Escape);
    harness.step();
    harness.step();
    assert!(!harness.state().search_active());
    assert_eq!(harness.state().search_text(), "");
    harness.key_press(egui::Key::ArrowDown);
    harness.step();
    assert_eq!(
        selection_len(&harness),
        1,
        "the list has the keyboard again"
    );
}

fn tab_of(harness: &Harness<'_, RickUi>, name: &str) -> (model::TabId, model::PaneId) {
    let model = harness.state().app().model();
    model
        .panes()
        .flat_map(|(pane, p)| p.tabs().iter().map(move |t| (*t, pane)))
        .find(|(tab, _)| {
            model
                .tab(*tab)
                .is_some_and(|t| t.location.path.to_string_lossy().ends_with(name))
        })
        .unwrap_or_else(|| panic!("no tab at {name}"))
}

/// Dragging a tab from pane B onto pane A's strip moves it into pane A in
/// the model, instead of snapping back.
#[test]
fn a_tab_dragged_onto_another_panes_strip_moves_there() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let left = dir.path().join("left-pane");
    let right = dir.path().join("right-pane");
    fs::create_dir(&left).unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir(&right).unwrap_or_else(|e| panic!("{e}"));
    let mut harness = harness(&left, egui::vec2(1400.0, 800.0));
    let pane_a = harness.state().app().model().focused;

    // A second pane to the right, showing right-pane.
    harness.state_mut().app_mut().handle(
        app::Command::Split {
            pane: pane_a,
            dir: model::SplitDir::Right,
        },
        &mut Vec::new(),
    );
    settle(&mut harness);
    let pane_b = harness
        .state()
        .app()
        .model()
        .panes()
        .map(|(p, _)| p)
        .find(|p| *p != pane_a)
        .unwrap_or_else(|| panic!("no second pane"));
    harness.state_mut().app_mut().handle(
        app::Command::Navigate {
            pane: pane_b,
            to: local(&right),
        },
        &mut Vec::new(),
    );
    settle(&mut harness);
    let (tab_b, home_b) = tab_of(&harness, "right-pane");
    assert_eq!(home_b, pane_b);

    // Drag right-pane's tab onto the strip of pane A, to the right of its
    // tab, in small steps so egui_dock sees a drag. Tab titles are painted,
    // not widgets, so the strips are found from the pane rectangles: each
    // strip is the top row of its pane and its first tab starts at the left.
    let rects = harness.state().pane_rects();
    let rect_of = |pane| {
        rects
            .iter()
            .find(|(p, _)| *p == pane)
            .map(|(_, r)| *r)
            .unwrap_or_else(|| panic!("no rect for {pane:?}"))
    };
    let (rect_a, rect_b) = (rect_of(pane_a), rect_of(pane_b));
    let from = egui::pos2(rect_b.left() + 50.0, rect_b.top() + 14.0);
    let to = egui::pos2(rect_a.left() + 220.0, rect_a.top() + 14.0);
    harness.event(egui::Event::PointerMoved(from));
    harness.step();
    harness.event(egui::Event::PointerButton {
        pos: from,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::default(),
    });
    harness.step();
    for i in 1..=12 {
        let t = i as f32 / 12.0;
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
    settle(&mut harness);

    let model = harness.state().app().model();
    assert_eq!(
        model.pane_of(tab_b),
        Some(pane_a),
        "the tab moved into pane A"
    );
    assert_eq!(
        model.pane(pane_a).map(|p| p.tabs().len()),
        Some(2),
        "pane A holds both tabs"
    );
    assert!(model.pane(pane_b).is_none(), "the emptied pane is gone");
}

/// The toolbar at three widths with a twelve-segment path: no widget
/// overlaps another, the right group keeps its width, the breadcrumb
/// collapses into a chevron when it must and always shows the last segment.
#[test]
fn the_toolbar_groups_never_overlap_and_the_right_group_keeps_its_width() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let mut deep = dir.path().to_path_buf();
    for name in [
        "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel", "india",
        "juliet", "kilo", "lima",
    ] {
        deep.push(name);
    }
    fs::create_dir_all(&deep).unwrap_or_else(|e| panic!("{e}"));
    let mut right_widths = Vec::new();
    for width in [480.0, 800.0, 1600.0] {
        let mut harness = harness(&deep, egui::vec2(width, 600.0));
        // Pane A takes the whole window: the side panel off keeps the
        // toolbar as wide as the window.
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::B);
        harness.step();
        harness.step();
        let pane = harness.state().app().model().focused;
        let (tab, _) = harness
            .state()
            .app()
            .model()
            .active_tab(pane)
            .unwrap_or_else(|| panic!("tab"));
        let rects = harness.state().toolbar_rects(tab);
        assert!(rects.len() >= 7, "{width}: {rects:?}");
        for (i, (ga, a)) in rects.iter().enumerate() {
            for (gb, b) in rects.iter().skip(i + 1) {
                let overlap = a.shrink(0.5).intersects(b.shrink(0.5));
                assert!(!overlap, "{width}: {ga} {a:?} overlaps {gb} {b:?}");
            }
            assert!(
                a.left() >= -0.5 && a.right() <= width + 0.5,
                "{width}: {ga} {a:?}"
            );
        }
        let right: Vec<&egui::Rect> = rects
            .iter()
            .filter(|(g, _)| *g == "right")
            .map(|(_, r)| r)
            .collect();
        let right_width = right.iter().map(|r| r.right()).fold(0.0, f32::max)
            - right.iter().map(|r| r.left()).fold(f32::INFINITY, f32::min);
        right_widths.push(right_width.round());
        // The last segment is always there; at 480 the ancestors fold.
        harness.get_by_label_contains("lima");
        if width < 500.0 {
            harness.get_by_label(ui_egui::icons::COLLAPSED);
        }
    }
    assert_eq!(right_widths[0], right_widths[1], "{right_widths:?}");
    assert_eq!(right_widths[1], right_widths[2], "{right_widths:?}");
}
