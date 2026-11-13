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

/// The nodes whose label contains `text` in the focused pane or in the
/// title row above it (the search dropdown), not in the side panel.
fn outside_the_side_panel(harness: &Harness<'_, RickUi>, text: &str) -> usize {
    let pane = harness.state().app().model().focused;
    let rect = harness
        .state()
        .pane_rects()
        .into_iter()
        .find(|(p, _)| *p == pane)
        .map_or_else(|| panic!("no pane rect"), |(_, r)| r);
    harness
        .query_all_by_label_contains(text)
        .filter(|n| {
            let centre = n.rect().center();
            rect.contains(centre) || centre.y < rect.top()
        })
        .count()
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
    // The list row and the dropdown's hit; the others only in the list. The
    // side panel is left out: its Recent section lists the files too once
    // the folder's watch reports them, which on macOS can be before now.
    assert_eq!(outside_the_side_panel(&harness, "gamma.txt"), 2);
    assert_eq!(outside_the_side_panel(&harness, "alpha.txt"), 1);
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
    // The command is listed under the box; the title row's own toggle
    // carries the same name, above it.
    let listed = |harness: &Harness<'_, RickUi>| {
        harness
            .query_all_by_label_contains("Toggle side panel")
            .filter(|node| node.rect().top() > 44.0)
            .count()
    };
    assert_eq!(listed(&harness), 1);
    harness.event(egui::Event::Text("toggle side".to_owned()));
    harness.step();
    assert_eq!(listed(&harness), 1);
    assert!(side_panel_visible(&harness));
    harness.key_press(egui::Key::Enter);
    harness.step();
    harness.step();
    assert!(!side_panel_visible(&harness), "Enter ran Toggle side panel");
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

/// Dragging pane B by its handle onto the centre tile of pane A makes B a
/// workspace of its own, shown, and A's workspace keeps A alone (SPEC.md
/// *Panes*: the centre target opens a new tab).
#[test]
fn a_pane_dragged_onto_another_panes_centre_tile_becomes_a_workspace() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let left = dir.path().join("left-pane");
    let right = dir.path().join("right-pane");
    fs::create_dir(&left).unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir(&right).unwrap_or_else(|e| panic!("{e}"));
    let mut harness = harness(&left, egui::vec2(1400.0, 800.0));
    let (pane_a, pane_b) = two_panes(&mut harness, &right);
    let (tab_b, home_b) = tab_of(&harness, "right-pane");
    assert_eq!(home_b, pane_b);

    // Drag right-pane's handle onto the centre tile of pane A.
    let rect_a = rect_of(&harness, pane_a);
    let from = handle_of(&harness, pane_b);
    drag(&mut harness, from, tile(rect_a, 0.0, 0.0));

    let model = harness.state().app().model();
    let window = model.focused_window;
    assert_eq!(model.pane_of(tab_b), Some(pane_b), "the tab keeps its pane");
    assert_eq!(
        model.workspaces(window).len(),
        2,
        "pane B is a workspace of its own"
    );
    assert_eq!(model.workspaces(window)[0].dock, model::Dock::Pane(pane_a));
    assert_eq!(model.workspaces(window)[1].dock, model::Dock::Pane(pane_b));
    assert_eq!(model.focused, pane_b, "and shown");
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

/// The centre of `pane`'s grab handle in the last frame: the toolbar's
/// button named *Drag pane* inside the pane.
fn handle_of(harness: &Harness<'_, RickUi>, pane: model::PaneId) -> egui::Pos2 {
    let rect = rect_of(harness, pane);
    harness
        .query_all_by_label("Drag pane")
        .map(|n| n.rect())
        .find(|r| rect.contains(r.center()))
        .map_or_else(|| panic!("no handle in {pane:?}"), |r| r.center())
}

/// The centre of a dock tile over `rect`: the cross of five tiles around
/// the pane's centre, `DOCK_TILE_PITCH` apart.
fn tile(rect: egui::Rect, dx: f32, dy: f32) -> egui::Pos2 {
    rect.center() + egui::vec2(dx, dy) * ui_egui::DOCK_TILE_PITCH
}

/// Dragging the right pane by its handle onto the left pane's "left" tile
/// puts it left of the other; onto the "below" tile, under it (SPEC.md
/// *Panes*: the dock cross).
#[test]
fn a_pane_dragged_onto_another_panes_side_tile_docks_there() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let left = dir.path().join("left-pane");
    let right = dir.path().join("right-pane");
    fs::create_dir(&left).unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir(&right).unwrap_or_else(|e| panic!("{e}"));
    let mut harness = harness(&left, egui::vec2(1400.0, 800.0));
    let (pane_a, pane_b) = two_panes(&mut harness, &right);
    let (tab_b, _) = tab_of(&harness, "right-pane");

    // Left tile of pane A: B's pane goes to the left of A.
    let rect_a = rect_of(&harness, pane_a);
    let from = handle_of(&harness, pane_b);
    drag(&mut harness, from, tile(rect_a, -1.0, 0.0));
    let model = harness.state().app().model();
    let home = model.pane_of(tab_b).unwrap_or_else(|| panic!("tab b lost"));
    assert_ne!(home, pane_a, "tab B keeps its own pane");
    assert_eq!(
        *model.dock(),
        model::Dock::split_of(
            model::SplitDir::Right,
            model::Dock::Pane(home),
            model::Dock::Pane(pane_a),
        ),
        "B's pane is left of A"
    );

    // Below tile of pane A: B's pane goes under A.
    let rect_a = rect_of(&harness, pane_a);
    let from = handle_of(&harness, home);
    drag(&mut harness, from, tile(rect_a, 0.0, 1.0));
    let model = harness.state().app().model();
    let home = model.pane_of(tab_b).unwrap_or_else(|| panic!("tab b lost"));
    assert_eq!(
        *model.dock(),
        model::Dock::split_of(
            model::SplitDir::Down,
            model::Dock::Pane(pane_a),
            model::Dock::Pane(home),
        ),
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

/// As [`harness`], with the plain frame of an opaque window (issue 158).
fn plain_harness(start: &Path, size: egui::Vec2) -> Harness<'static, RickUi> {
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: Vec::new(),
        config_dir: Some(fake.config_dir()),
        start: Some(local(start)),
    });
    let mut harness = Harness::builder().with_size(size).build_eframe(move |cc| {
        RickUi::new(app, host, &cc.egui_ctx).with_frame(ui_egui::frame::Mode::Plain)
    });
    settle(&mut harness);
    harness
}

/// Where the window cannot be transparent Rick draws the plain frame
/// (specs/gui-title-row.md, *Window frame*): no margin, so the content
/// fills the window inside a one-pixel border, and the window resizes from
/// its very edge and corners, the grips reaching 4 px inwards.
#[test]
fn the_plain_window_frame_fills_the_window_and_resizes_from_its_edge() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let mut harness = plain_harness(dir.path(), egui::vec2(900.0, 600.0));
    let content = harness
        .state()
        .framed_content()
        .unwrap_or_else(|| panic!("the fake platform has no native chrome, so Rick frames"));
    assert_eq!(
        content,
        egui::Rect::from_min_max(egui::pos2(1.0, 1.0), egui::pos2(899.0, 599.0)),
        "the window inside its one-pixel border"
    );
    let pane = harness.state().app().model().focused;
    let pane_rect = rect_of(&harness, pane);
    assert!(pane_rect.right() > 890.0, "{pane_rect:?} reaches the edge");

    // The cursor over the edges and corners, and just inside the grips.
    let hovered = |harness: &mut Harness<'static, RickUi>, pos: egui::Pos2| {
        harness.event(egui::Event::PointerMoved(pos));
        harness.step();
        harness.step();
        harness.output().platform_output.cursor_icon
    };
    let edge = egui::pos2(2.0, 300.0);
    for (pos, cursor) in [
        (edge, egui::CursorIcon::ResizeWest),
        (egui::pos2(898.0, 300.0), egui::CursorIcon::ResizeEast),
        (egui::pos2(450.0, 598.0), egui::CursorIcon::ResizeSouth),
        (egui::pos2(2.0, 2.0), egui::CursorIcon::ResizeNorthWest),
        (egui::pos2(898.0, 598.0), egui::CursorIcon::ResizeSouthEast),
    ] {
        assert_eq!(hovered(&mut harness, pos), cursor, "at {pos:?}");
    }
    let inside = hovered(&mut harness, egui::pos2(899.0 - 6.0, 590.0));
    assert_ne!(inside, egui::CursorIcon::ResizeEast, "6 px inside the edge");
    assert_ne!(inside, egui::CursorIcon::ResizeSouthEast);

    // Press on the edge and pull: the backend is asked to resize westwards.
    hovered(&mut harness, edge);
    harness.event(egui::Event::PointerButton {
        pos: edge,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::default(),
    });
    harness.step();
    let mut commands = root_commands(&harness);
    for dx in [3.0, 8.0] {
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
        .map_or_default(|out| out.commands.clone())
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
        // Back and Forward, the breadcrumb, the view options and the handle.
        let rects = harness.state().toolbar_rects(tab);
        assert!(rects.len() >= 5, "{width}: {rects:?}");
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
        // The last segment is always there; at 480 the ancestors fold. The
        // workspace tab above the pane carries the folder's name too, so
        // the segment is the one inside the row.
        assert!(
            harness
                .query_all_by_label_contains("lima")
                .any(|n| row.contains(n.rect().center())),
            "{width}: no last segment in the toolbar row"
        );
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
        start: Some(local(start)),
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
    // Paste and Paste as link are both there.
    assert!(
        harness.get_all_by_label_contains("Paste").count() >= 2,
        "Paste and Paste as link"
    );
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
    let pdfium = std::env::var_os("RICK_REQUIRE_PDFIUM").is_some();
    let mut tooks = Vec::new();
    let mut longests = Vec::new();
    // A fresh folder and window per run: the thumbnails are cached per entry.
    for _ in 0..tests::bench::runs() {
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
        println!(
            "icon view: {drawable} thumbnails in {took:?}, {} frames, longest {longest:?}",
            frames.len()
        );
        tooks.push(took);
        longests.push(longest);
    }
    tests::bench::record("thumbnails", &tooks);
    tests::bench::record("thumbnails-frame", &longests);
}

/// Whether the focused window's side panel is shown.
fn side_panel_visible(harness: &Harness<'_, RickUi>) -> bool {
    let model = harness.state().app().model();
    model
        .window(model.focused_window)
        .is_some_and(|w| w.side_panel.visible)
}

/// Issue 79 items 2 and 3: the preview pane's toolbar is the explorer
/// toolbar's height and ends in the same grab handle after a hairline,
/// and the handle drags the pane like any other's (SPEC.md *Preview*:
/// "with the same toolbar height, handle and footer").
#[test]
fn the_preview_toolbar_is_the_explorers_height_and_its_handle_drags_the_pane() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let folder = dir.path().join("photos");
    fs::create_dir(&folder).unwrap_or_else(|e| panic!("{e}"));
    let mut harness = harness(&folder, egui::vec2(1400.0, 800.0));
    let explorer = harness.state().app().model().focused;
    harness
        .state_mut()
        .queue(ui_egui::Intent::ToggleInspector, explorer);
    harness.step();
    harness.step();
    settle(&mut harness);
    let window = harness.state().window_ids()[0];
    let model = harness.state().app().model();
    let preview_tab = model
        .preview_tab(window)
        .unwrap_or_else(|| panic!("no preview tab"));
    let preview = model
        .pane_of(preview_tab)
        .unwrap_or_else(|| panic!("no preview pane"));
    let density = model.settings.density;

    // The toolbar row is the explorer toolbar's height, and the handle
    // sits at its right end inside the pane, the width of every handle.
    let rects = harness.state().preview_rects();
    let toolbar = rects
        .toolbar
        .unwrap_or_else(|| panic!("no preview toolbar"));
    assert!(
        (toolbar.height() - ui_egui::toolbar_height(density)).abs() < 0.5,
        "toolbar {toolbar:?} for {density:?}"
    );
    let handle = rects.handle.unwrap_or_else(|| panic!("no handle"));
    let preview_rect = rect_of(&harness, preview);
    assert!(
        toolbar.expand(0.5).contains_rect(handle) && preview_rect.contains_rect(handle),
        "handle {handle:?} in toolbar {toolbar:?} of pane {preview_rect:?}"
    );
    assert!((handle.width() - ui_egui::HANDLE_WIDTH).abs() < 0.5);
    assert!(
        preview_rect.right() - handle.right() < 20.0,
        "at the right end: {handle:?} in {preview_rect:?}"
    );
    let grip = handle_of(&harness, preview);
    assert!((grip - handle.center()).length() < 1.0, "named Drag pane");

    // Dragging it onto the explorer's left tile docks the preview to the
    // left of the explorer.
    let rect_a = rect_of(&harness, explorer);
    drag(&mut harness, grip, tile(rect_a, -1.0, 0.0));
    let model = harness.state().app().model();
    let home = model
        .pane_of(preview_tab)
        .unwrap_or_else(|| panic!("preview tab lost"));
    assert_eq!(
        *model.dock(),
        model::Dock::split_of(
            model::SplitDir::Right,
            model::Dock::Pane(home),
            model::Dock::Pane(explorer),
        ),
        "the preview is left of the explorer"
    );
}
