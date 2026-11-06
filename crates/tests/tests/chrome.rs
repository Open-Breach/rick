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
    let (pane_a, pane_b) = two_panes(&mut harness, &right);
    let (tab_b, home_b) = tab_of(&harness, "right-pane");
    assert_eq!(home_b, pane_b);

    // Drag right-pane's tab onto the strip of pane A, to the right of its
    // tab. Tab titles are painted, not widgets, so the strips are found from
    // the pane rectangles: each strip is the top row of its pane and its
    // first tab starts at the left.
    let (rect_a, rect_b) = (rect_of(&harness, pane_a), rect_of(&harness, pane_b));
    let from = egui::pos2(rect_b.left() + 50.0, rect_b.top() + 14.0);
    let to = egui::pos2(rect_a.left() + 220.0, rect_a.top() + 14.0);
    drag(&mut harness, from, to);

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

/// Presses the primary button at `from`, moves to `to` in small steps so
/// `egui_dock` sees a drag, releases, and lets the app settle.
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

/// Two panes side by side, the second showing `right`; returns their ids.
fn two_panes(harness: &mut Harness<'_, RickUi>, right: &Path) -> (model::PaneId, model::PaneId) {
    let pane_a = harness.state().app().model().focused;
    harness.state_mut().app_mut().handle(
        app::Command::Split {
            pane: pane_a,
            dir: model::SplitDir::Right,
        },
        &mut Vec::new(),
    );
    settle(harness);
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
            to: local(right),
        },
        &mut Vec::new(),
    );
    settle(harness);
    (pane_a, pane_b)
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

/// Dragging the only tab of the right pane onto the left pane's "left" drop
/// zone puts its pane left of the other; onto the "below" zone, under it.
/// Before the fix the tree snapped back because no tab changed pane.
#[test]
fn a_tab_dragged_onto_another_panes_edge_moves_its_pane_there() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let left = dir.path().join("left-pane");
    let right = dir.path().join("right-pane");
    fs::create_dir(&left).unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir(&right).unwrap_or_else(|e| panic!("{e}"));
    let mut harness = harness(&left, egui::vec2(1400.0, 800.0));
    let (pane_a, pane_b) = two_panes(&mut harness, &right);
    let (tab_b, _) = tab_of(&harness, "right-pane");

    // egui_dock's drop zones are five buttons around the centre of the
    // hovered pane body, each a square of a third of the body's shorter
    // side at most 100 px, 10 px apart. The strip takes the top 30 px.
    let zone = |rect: egui::Rect, dx: f32, dy: f32| {
        let body = egui::Rect::from_min_max(egui::pos2(rect.left(), rect.top() + 30.0), rect.max)
            .shrink(10.0);
        let side = ((body.width() - 20.0) / 3.0)
            .min((body.height() - 20.0) / 3.0)
            .min(100.0);
        body.center() + egui::vec2(dx, dy) * (side + 10.0)
    };

    // Left zone of pane A: B's pane goes to the left of A.
    let rect_a = rect_of(&harness, pane_a);
    let rect_b = rect_of(&harness, pane_b);
    let from = egui::pos2(rect_b.left() + 50.0, rect_b.top() + 14.0);
    drag(&mut harness, from, zone(rect_a, -1.0, 0.0));
    let model = harness.state().app().model();
    let home = model.pane_of(tab_b).unwrap_or_else(|| panic!("tab b lost"));
    assert_ne!(home, pane_a, "tab B keeps its own pane");
    assert_eq!(
        model.dock,
        model::Dock::Split {
            dir: model::SplitDir::Right,
            first: Box::new(model::Dock::Pane(home)),
            second: Box::new(model::Dock::Pane(pane_a)),
        },
        "B's pane is left of A"
    );

    // Below zone of pane A: B's pane goes under A.
    let rect_a = rect_of(&harness, pane_a);
    let rect_b = rect_of(&harness, home);
    let from = egui::pos2(rect_b.left() + 50.0, rect_b.top() + 14.0);
    drag(&mut harness, from, zone(rect_a, 0.0, 1.0));
    let model = harness.state().app().model();
    let home = model.pane_of(tab_b).unwrap_or_else(|| panic!("tab b lost"));
    assert_eq!(
        model.dock,
        model::Dock::Split {
            dir: model::SplitDir::Down,
            first: Box::new(model::Dock::Pane(pane_a)),
            second: Box::new(model::Dock::Pane(home)),
        },
        "B's pane is under A"
    );
    assert_eq!(model.panes().count(), 2, "still two panes");
}

/// Where the system draws no chrome Rick frames the window itself: the
/// content sits inside the margin, and the edge resizes the window through
/// the backend, with the matching cursor while hovered.
#[test]
fn the_window_frame_resizes_from_its_edges() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let mut harness = harness(dir.path(), egui::vec2(900.0, 600.0));
    let content = harness
        .state()
        .framed_content()
        .unwrap_or_else(|| panic!("the fake platform has no native chrome, so Rick frames"));
    let inset = ui_egui::frame::MARGIN + 1.0;
    assert!((content.left() - inset).abs() < 0.5, "{content:?}");
    assert!((content.top() - inset).abs() < 0.5, "{content:?}");
    assert!(
        (content.right() - (900.0 - inset)).abs() < 0.5,
        "{content:?}"
    );
    assert!(
        (content.bottom() - (600.0 - inset)).abs() < 0.5,
        "{content:?}"
    );
    let pane = harness.state().app().model().focused;
    let pane_rect = rect_of(&harness, pane);
    assert!(
        pane_rect.left() >= inset - 0.5,
        "{pane_rect:?} inside the frame"
    );

    // Hover the left edge: the resize cursor.
    let edge = egui::pos2(4.0, 300.0);
    harness.event(egui::Event::PointerMoved(edge));
    harness.step();
    harness.step();
    assert_eq!(
        harness.output().platform_output.cursor_icon,
        egui::CursorIcon::ResizeWest
    );

    // Press and pull: the backend is asked to resize westwards. Each step
    // runs one frame per queued event and keeps the last frame's output, so
    // the press gets a step of its own.
    harness.event(egui::Event::PointerButton {
        pos: edge,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::default(),
    });
    harness.step();
    let mut commands = root_commands(&harness);
    for dx in [-3.0, -8.0] {
        harness.event(egui::Event::PointerMoved(edge + egui::vec2(dx, 0.0)));
        harness.step();
        commands.extend(root_commands(&harness));
    }
    assert!(
        commands.iter().any(|c| matches!(
            c,
            egui::ViewportCommand::BeginResize(egui::viewport::ResizeDirection::West)
        )),
        "{commands:?}"
    );
    harness.event(egui::Event::PointerButton {
        pos: edge,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::default(),
    });
    harness.step();
}

/// The viewport commands the last frame sent to the backend.
fn root_commands(harness: &Harness<'_, RickUi>) -> Vec<egui::ViewportCommand> {
    harness
        .output()
        .viewport_output
        .get(&egui::ViewportId::ROOT)
        .map(|out| out.commands.clone())
        .unwrap_or_default()
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
        // The row is as high as the density says and holds every widget.
        let row = harness
            .state()
            .toolbar_row(tab)
            .unwrap_or_else(|| panic!("{width}: no toolbar row"));
        let density = harness.state().app().model().settings.density;
        assert!(
            (row.height() - ui_egui::toolbar_height(density)).abs() < 0.5,
            "{width}: row {row:?} for {density:?}"
        );
        for (g, r) in &rects {
            assert!(
                row.expand(0.5).contains_rect(*r),
                "{width}: {g} {r:?} sticks out of the row {row:?}"
            );
        }
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
    assert!(
        (right_widths[0] - right_widths[1]).abs() < 0.5,
        "{right_widths:?}"
    );
    assert!(
        (right_widths[1] - right_widths[2]).abs() < 0.5,
        "{right_widths:?}"
    );
}

fn previewers() -> Vec<Arc<dyn ext::Extension>> {
    vec![
        Arc::new(ext_preview_image::ImagePreview::new()),
        Arc::new(ext_preview_pdf::PdfPreview::new()),
    ]
}

fn harness_with(
    start: &Path,
    size: egui::Vec2,
    extensions: Vec<Arc<dyn ext::Extension>>,
) -> Harness<'static, RickUi> {
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions,
        config_dir: Some(fake.config_dir()),
        start: local(start),
    });
    let mut harness = Harness::builder()
        .with_size(size)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    harness
}

/// Writes `n` small PNGs and one PDF into `dir`; returns how many files a
/// previewer can draw.
fn picture_fixtures(dir: &Path, n: usize) -> usize {
    for i in 0..n {
        let seed = u32::try_from(i).unwrap_or(0);
        let shade = u8::try_from(i % 256).unwrap_or(0).wrapping_mul(40);
        let image = image::RgbaImage::from_fn(96, 64, |x, y| {
            let c = u8::try_from((x + y + seed * 7) % 255).unwrap_or(0);
            image::Rgba([c, 255 - c, shade, 255])
        });
        let mut bytes = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap_or_else(|e| panic!("{e}"));
        fs::write(dir.join(format!("picture-{i:03}.png")), bytes.into_inner())
            .unwrap_or_else(|e| panic!("{e}"));
    }
    fs::write(dir.join("paper.pdf"), ext_preview_pdf::sample_pdf(1))
        .unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.join("notes.txt"), "no thumbnail for text").unwrap_or_else(|e| panic!("{e}"));
    n + 1
}

/// Right-clicking below the last row opens the folder's menu: New folder
/// and Paste are there, Rename and Move to trash are not.
#[test]
fn right_click_on_empty_space_opens_the_folder_menu() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("only.txt"), "x").unwrap_or_else(|e| panic!("{e}"));
    let mut harness = harness(dir.path(), egui::vec2(1100.0, 720.0));
    // Something on the clipboard, so Paste applies.
    harness.key_press(egui::Key::ArrowDown);
    harness.step();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::C);
    harness.step();
    assert_eq!(selection_len(&harness), 1);

    let pane = harness.state().app().model().focused;
    let rect = harness
        .state()
        .pane_rects()
        .into_iter()
        .find(|(p, _)| *p == pane)
        .map_or_else(|| panic!("pane rect"), |(_, r)| r);
    // Well below the single row, above the foot.
    let pos = egui::pos2(rect.center().x, rect.bottom() - 80.0);
    harness.event(egui::Event::PointerMoved(pos));
    harness.step();
    harness.event(egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Secondary,
        pressed: true,
        modifiers: egui::Modifiers::default(),
    });
    harness.step();
    harness.event(egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Secondary,
        pressed: false,
        modifiers: egui::Modifiers::default(),
    });
    harness.step();
    harness.step();
    harness.get_by_label_contains("New folder");
    harness.get_by_label_contains("Paste");
    assert!(
        harness.query_by_label_contains("Rename").is_none(),
        "needs an item"
    );
    assert!(
        harness.query_by_label_contains("Move to trash").is_none(),
        "needs an item"
    );
    assert_eq!(
        selection_len(&harness),
        0,
        "the folder menu has no selection"
    );
}

/// The icon view shows a cell per entry, with thumbnails arriving for the
/// pictures and the PDF and the file-type icon staying for the rest.
#[test]
fn the_icon_view_shows_thumbnails_for_pictures() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let drawable = picture_fixtures(dir.path(), 3);
    let mut harness = harness_with(dir.path(), egui::vec2(1100.0, 720.0), previewers());
    harness.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::Num4,
    );
    harness.step();
    let pane = harness.state().app().model().focused;
    assert!(matches!(
        harness
            .state()
            .app()
            .model()
            .active_tab(pane)
            .map(|(_, t)| t.view),
        Some(model::ViewMode::Icons(_))
    ));
    harness.get_by_label("picture-000.png");
    harness.get_by_label("notes.txt");
    // The PDF needs pdfium; without it the pictures alone must arrive.
    let pdfium = std::env::var_os("RICK_REQUIRE_PDFIUM").is_some();
    let expected = if pdfium { drawable } else { drawable - 1 };
    let deadline = Instant::now() + Duration::from_secs(20);
    while harness.state().thumbnails().ready() < expected {
        assert!(Instant::now() < deadline, "thumbnails never arrived");
        harness.step();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(harness.state().thumbnails().ready() <= drawable);
}

/// Icons view on a folder of pictures and a PDF: every thumbnail within
/// 500 ms, no frame over 16.6 ms while they arrive (release; Linux asserts,
/// the others print).
#[test]
#[ignore = "timing; run with --ignored in release"]
fn the_icon_view_thumbnails_arrive_within_500_ms_without_long_frames() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let drawable = picture_fixtures(dir.path(), 40);
    let mut harness = harness_with(dir.path(), egui::vec2(1400.0, 900.0), previewers());
    harness.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::Num4,
    );
    harness.step();
    let pane = harness.state().app().model().focused;
    harness.state_mut().app_mut().handle(
        app::Command::SetView {
            pane,
            mode: model::ViewMode::Icons(model::IconSize::ExtraLarge),
        },
        &mut Vec::new(),
    );
    let pdfium = std::env::var_os("RICK_REQUIRE_PDFIUM").is_some();
    let drawable = if pdfium { drawable } else { drawable - 1 };
    let started = Instant::now();
    let mut frames = Vec::new();
    while harness.state().thumbnails().ready() < drawable {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "only {} of {drawable} thumbnails",
            harness.state().thumbnails().ready()
        );
        let frame = Instant::now();
        harness.step();
        frames.push(frame.elapsed());
        std::thread::sleep(Duration::from_millis(2));
    }
    let took = started.elapsed();
    let longest = frames.iter().copied().max().unwrap_or_default();
    eprintln!(
        "icon view: {drawable} thumbnails in {took:?}, {} frames, longest {longest:?}",
        frames.len()
    );
    if cfg!(target_os = "linux") {
        assert!(took <= Duration::from_millis(500), "{took:?}");
        assert!(longest <= Duration::from_micros(16_600), "{longest:?}");
    }
}
