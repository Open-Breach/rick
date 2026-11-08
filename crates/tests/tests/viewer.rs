//! Acceptance (issue 17, the viewer): Enter on a picture opens a viewer tab
//! and Right steps to the next one with the position in the title; F11 and
//! Escape toggle fullscreen and Escape closes; a 300-page PDF shows the page
//! strip and jumps to a page typed into the counter; Mod+F in a text file
//! counts and highlights its matches; the zoom is remembered across two
//! JPEGs; an unknown binary opens in hex when the plugin is loaded, and a
//! file nothing claims says so; the Preview tab docks below and tears out;
//! and stepping through 50 images keeps every frame under the budget.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::{By, Queryable as _};
use ext::Extension;
use ext::Text;
use model::{DockTarget, SelectionChange, Side, TabId, TabKind};
use platform::{ConfigDir, Fake};
use tests::{local, media};
use ui_egui::keys::Intent;
use ui_egui::viewer::Zoom;
use ui_egui::{EguiHost, RickUi, icons};

/// The four compiled-in previewers.
fn previewers() -> Vec<Arc<dyn Extension>> {
    vec![
        Arc::new(ext_preview_text::TextPreview::new()),
        Arc::new(ext_preview_code::CodePreview::new()),
        Arc::new(ext_preview_image::ImagePreview::new()),
        Arc::new(ext_preview_pdf::PdfPreview::new()),
    ]
}

/// The previewers with the hex plugin, as the bundle loads it.
fn with_hex() -> Vec<Arc<dyn Extension>> {
    let mut extensions = previewers();
    extensions.push(Arc::new(ext_preview_hex::HexPreview::new()));
    extensions
}

fn window(
    fake: &Arc<Fake>,
    start: &Path,
    size: egui::Vec2,
    extensions: Vec<Arc<dyn Extension>>,
) -> Harness<'static, RickUi> {
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
        .with_pixels_per_point(1.0)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    harness
}

fn settle(harness: &mut Harness<'_, RickUi>) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        for _ in 0..3 {
            harness.step();
        }
        if !harness.state().app().is_busy() {
            std::thread::sleep(Duration::from_millis(30));
            harness.step();
            if !harness.state().app().is_busy() {
                break;
            }
        }
        assert!(Instant::now() < deadline, "did not settle");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Steps frames until `done` holds, within `limit`.
fn step_until(
    harness: &mut Harness<'_, RickUi>,
    what: &str,
    limit: Duration,
    mut done: impl FnMut(&Harness<'_, RickUi>) -> bool,
) {
    let deadline = Instant::now() + limit;
    while !done(harness) {
        assert!(Instant::now() < deadline, "{what}: not in time");
        harness.step();
        std::thread::sleep(Duration::from_millis(2));
    }
    harness.step();
}

/// A small PNG with a colour of its own.
fn png(width: u32, height: u32, tint: u8) -> Vec<u8> {
    let image = image::RgbaImage::from_fn(width, height, |x, y| {
        image::Rgba([
            tint,
            u8::try_from(x % 256).unwrap_or(0),
            u8::try_from(y % 256).unwrap_or(0),
            255,
        ])
    });
    let mut bytes = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap_or_else(|e| panic!("{e}"));
    bytes.into_inner()
}

fn id_of(harness: &Harness<'_, RickUi>, name: &str) -> types::EntryId {
    let pane = harness.state().app().model().focused;
    harness
        .state()
        .app()
        .model()
        .active_tab(pane)
        .and_then(|(_, t)| t.rows().find(|e| e.name == name).map(|e| e.id))
        .unwrap_or_else(|| panic!("no entry {name}"))
}

/// The viewer tab active in the focused pane, if any.
fn viewer_tab(harness: &Harness<'_, RickUi>) -> Option<TabId> {
    let model = harness.state().app().model();
    model
        .active_tab(model.focused)
        .filter(|(_, tab)| tab.kind == TabKind::Viewer)
        .map(|(id, _)| id)
}

/// The name of the file the viewer shows.
fn shown(harness: &Harness<'_, RickUi>) -> Option<String> {
    let tab = viewer_tab(harness)?;
    let tab = harness.state().app().model().tab(tab)?;
    ui_egui::viewer::shown(tab).map(|e| e.name.clone())
}

/// The viewer tab's title, as the strip shows it.
fn title(harness: &Harness<'_, RickUi>) -> String {
    let tab = viewer_tab(harness).unwrap_or_else(|| panic!("no viewer tab"));
    harness
        .state()
        .app()
        .model()
        .tab(tab)
        .map(ui_egui::viewer::title)
        .unwrap_or_default()
}

/// Selects `name` in the focused explorer and presses Enter; waits for the
/// viewer tab to open and show a frame or say why not.
fn open_in_viewer(harness: &mut Harness<'_, RickUi>, name: &str) -> TabId {
    let pane = harness.state().app().model().focused;
    let id = id_of(harness, name);
    harness.state_mut().app_mut().handle(
        app::Command::Select {
            pane,
            change: SelectionChange::Set(vec![id]),
        },
        &mut Vec::new(),
    );
    harness.step();
    harness.key_press(egui::Key::Enter);
    step_until(harness, "viewer tab", Duration::from_secs(10), |h| {
        viewer_tab(h).is_some() && shown(h).as_deref() == Some(name)
    });
    wait_frame(harness);
    viewer_tab(harness).unwrap_or_else(|| panic!("no viewer tab"))
}

/// Steps until the viewer's slot has a frame or an error for its file.
fn wait_frame(harness: &mut Harness<'_, RickUi>) {
    step_until(harness, "frame", Duration::from_secs(20), |h| {
        let pane = h.state().app().model().focused;
        h.state()
            .previews()
            .get(pane)
            .is_some_and(|s| s.has_frame() || s.error().is_some())
    });
    harness.step();
}

fn press(harness: &mut Harness<'_, RickUi>, modifiers: egui::Modifiers, key: egui::Key) {
    harness.key_press_modifiers(modifiers, key);
    harness.step();
    harness.step();
}

/// Clicks the icon button showing `glyph` inside `rect`; the same glyph may
/// serve elsewhere in the window.
fn click_in(harness: &mut Harness<'_, RickUi>, rect: egui::Rect, glyph: &str) {
    harness
        .get_all_by_label(glyph)
        .find(|node| rect.contains(node.rect().center()))
        .unwrap_or_else(|| panic!("no {glyph:?} in {rect:?}"))
        .click();
    harness.step();
    harness.step();
}

/// Enter on a PNG opens a viewer tab in the pane; Right steps to the next
/// picture in sort order with "2 of N" in the title, Home and End go to
/// the ends; F11 fills the screen and Escape leaves it, then closes the
/// tab; the explorer tab is untouched throughout.
// One scenario, step by step.
#[allow(clippy::too_many_lines)]
#[test]
fn enter_opens_a_viewer_tab_and_the_arrows_step_through_the_folder() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    for (name, tint) in [("a.png", 10), ("b.png", 120), ("c.png", 230)] {
        fs::write(dir.path().join(name), png(64, 48, tint)).unwrap_or_else(|e| panic!("{e}"));
    }
    fs::write(dir.path().join("notes.txt"), "plain\n").unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir(dir.path().join("sub")).unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut harness = window(&fake, dir.path(), egui::vec2(1100.0, 700.0), previewers());
    let pane = harness.state().app().model().focused;
    let explorer = harness
        .state()
        .app()
        .model()
        .active_tab(pane)
        .map_or_else(|| panic!("explorer tab"), |(id, _)| id);

    let viewer = open_in_viewer(&mut harness, "a.png");
    assert_ne!(viewer, explorer, "a tab of its own");
    assert_eq!(
        harness.state().app().model().pane_of(viewer),
        Some(pane),
        "in the same pane"
    );
    assert!(
        title(&harness).ends_with("a.png · 1 of 4"),
        "{}",
        title(&harness)
    );
    let slot = harness.state().previews().get(pane);
    assert!(
        slot.is_some_and(ui_egui::preview::Slot::has_frame),
        "{slot:?}"
    );
    assert!(fake.opened().is_empty(), "nothing went to the system");
    // The explorer tab keeps its listing and selection.
    let tab = harness
        .state()
        .app()
        .model()
        .tab(explorer)
        .unwrap_or_else(|| panic!("explorer"));
    assert_eq!(tab.listing.len(), 5);
    assert_eq!(tab.selection.len(), 1);

    press(&mut harness, egui::Modifiers::NONE, egui::Key::ArrowRight);
    step_until(&mut harness, "b.png", Duration::from_secs(10), |h| {
        shown(h).as_deref() == Some("b.png")
    });
    assert!(
        title(&harness).ends_with("b.png · 2 of 4"),
        "{}",
        title(&harness)
    );
    wait_frame(&mut harness);
    press(&mut harness, egui::Modifiers::NONE, egui::Key::End);
    assert_eq!(shown(&harness).as_deref(), Some("notes.txt"));
    assert!(
        title(&harness).ends_with("notes.txt · 4 of 4"),
        "{}",
        title(&harness)
    );
    press(&mut harness, egui::Modifiers::NONE, egui::Key::ArrowRight);
    assert_eq!(
        shown(&harness).as_deref(),
        Some("notes.txt"),
        "stays at the end"
    );
    press(&mut harness, egui::Modifiers::NONE, egui::Key::Home);
    assert_eq!(shown(&harness).as_deref(), Some("a.png"));
    press(&mut harness, egui::Modifiers::NONE, egui::Key::PageDown);
    assert_eq!(
        shown(&harness).as_deref(),
        Some("b.png"),
        "PageDown steps files too"
    );

    // F11 and Escape.
    assert_eq!(harness.state().viewer_fullscreen(), None);
    press(&mut harness, egui::Modifiers::NONE, egui::Key::F11);
    assert_eq!(harness.state().viewer_fullscreen(), Some(viewer));
    assert!(
        harness.query_by_label("Toggle preview pane").is_none(),
        "the chrome is gone in fullscreen"
    );
    press(&mut harness, egui::Modifiers::NONE, egui::Key::Escape);
    assert_eq!(harness.state().viewer_fullscreen(), None);
    assert_eq!(
        viewer_tab(&harness),
        Some(viewer),
        "the first Escape only leaves fullscreen"
    );
    press(&mut harness, egui::Modifiers::NONE, egui::Key::F11);
    assert_eq!(harness.state().viewer_fullscreen(), Some(viewer));
    press(&mut harness, egui::Modifiers::NONE, egui::Key::F11);
    assert_eq!(harness.state().viewer_fullscreen(), None, "F11 toggles");
    press(&mut harness, egui::Modifiers::NONE, egui::Key::Escape);
    assert_eq!(viewer_tab(&harness), None, "Escape closes the viewer");
    assert!(harness.state().app().model().tab(viewer).is_none());
    assert_eq!(
        harness
            .state()
            .app()
            .model()
            .active_tab(pane)
            .map(|(id, _)| id),
        Some(explorer)
    );

    // Enter on another file of the folder reuses one viewer tab per folder.
    let again = open_in_viewer(&mut harness, "c.png");
    harness.state_mut().queue(Intent::PrevTab, pane);
    harness.step();
    harness.step();
    assert_eq!(viewer_tab(&harness), None, "back on the explorer tab");
    let reused = open_in_viewer(&mut harness, "a.png");
    assert_eq!(reused, again, "the pane's viewer tab for this folder");
    let count = harness
        .state()
        .app()
        .model()
        .pane(pane)
        .map_or(0, |p| p.tabs().len());
    assert_eq!(count, 2, "one explorer, one viewer");
}

/// A 300-page PDF: the page strip shows tiles, the counter takes "150" and
/// goes there, the strip follows, and `PageDown` steps a page.
#[test]
fn a_300_page_pdf_shows_the_page_strip_and_jumps_by_direct_entry() {
    if !ext_preview_pdf::available() {
        assert!(
            std::env::var_os("RICK_REQUIRE_PDFIUM").is_none(),
            "pdfium required but not found"
        );
        return;
    }
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("book.pdf"), media::pdf(300, 300, 400))
        .unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut harness = window(&fake, dir.path(), egui::vec2(1100.0, 700.0), previewers());
    let pane = harness.state().app().model().focused;
    let viewer = open_in_viewer(&mut harness, "book.pdf");
    step_until(&mut harness, "300 pages", Duration::from_secs(20), |h| {
        h.state()
            .previews()
            .get(pane)
            .is_some_and(|s| s.pages() == Some(300))
    });
    let (toolbar, strip, strip_pages) = {
        let state = harness
            .state()
            .viewer(viewer)
            .unwrap_or_else(|| panic!("viewer state"));
        (
            state.rects.toolbar.unwrap_or_else(|| panic!("toolbar")),
            state.rects.strip,
            state.strip_pages.clone(),
        )
    };
    assert!(strip.is_some(), "a page strip for paged content");
    assert!(
        strip_pages.contains(&0) && strip_pages.len() > 3,
        "tiles for the first pages: {strip_pages:?}"
    );
    step_until(&mut harness, "a thumbnail", Duration::from_secs(20), |h| {
        h.state()
            .viewer(viewer)
            .is_some_and(|s| s.strip_thumbs.contains(&0))
    });

    // Direct entry: the counter in the toolbar reads 1; type 150 and press
    // Enter. The same value may show elsewhere in the window.
    harness
        .get_all(By::new().value("1"))
        .find(|node| toolbar.contains(node.rect().center()))
        .unwrap_or_else(|| panic!("no counter reading 1"))
        .focus();
    harness.step();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.step();
    harness
        .get_all(By::new().value("1"))
        .find(|node| toolbar.contains(node.rect().center()))
        .unwrap_or_else(|| panic!("no counter reading 1"))
        .type_text("150");
    harness.step();
    harness.key_press(egui::Key::Enter);
    harness.step();
    harness.step();
    assert_eq!(
        harness.state().previews().get(pane).map(|s| s.page),
        Some(149),
        "page 150 is shown"
    );
    step_until(
        &mut harness,
        "strip follows",
        Duration::from_secs(10),
        |h| {
            h.state()
                .viewer(viewer)
                .is_some_and(|s| s.strip_pages.contains(&149))
        },
    );
    assert!(
        harness
            .get_all(By::new().value("150"))
            .any(|node| toolbar.contains(node.rect().center())),
        "the counter reads 150"
    );
    press(&mut harness, egui::Modifiers::NONE, egui::Key::PageDown);
    assert_eq!(
        harness.state().previews().get(pane).map(|s| s.page),
        Some(150)
    );
    press(&mut harness, egui::Modifiers::NONE, egui::Key::PageUp);
    assert_eq!(
        harness.state().previews().get(pane).map(|s| s.page),
        Some(149)
    );
    wait_frame(&mut harness);
    assert!(
        harness
            .state()
            .viewer(viewer)
            .is_some_and(|s| s.frame_size.is_some()),
        "the page is drawn"
    );
}

/// Mod+F in a text file: the find bar opens with the field focused, typing
/// "fox" finds 3 matches, the count reads "1 of 3 matches", Enter and the
/// buttons step through them, and all three are highlighted in place.
#[test]
fn find_in_a_text_file_counts_and_highlights_the_matches() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    fs::write(
        dir.path().join("story.txt"),
        "The quick brown fox.\nA lazy dog.\nFOX again,\nand a fox at the end.\n",
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut harness = window(&fake, dir.path(), egui::vec2(1000.0, 600.0), previewers());
    let viewer = open_in_viewer(&mut harness, "story.txt");
    assert!(
        harness
            .state()
            .viewer(viewer)
            .is_some_and(|s| s.find.is_none())
    );

    press(&mut harness, egui::Modifiers::COMMAND, egui::Key::F);
    assert!(
        harness
            .state()
            .viewer(viewer)
            .is_some_and(|s| s.find.is_some()),
        "Mod+F opens find in the viewer, not the list's filter"
    );
    harness.event(egui::Event::Text("fox".to_owned()));
    harness.step();
    harness.step();
    let find = |h: &Harness<'_, RickUi>| {
        h.state()
            .viewer(viewer)
            .and_then(|s| s.find.as_ref())
            .map(|f| (f.query.clone(), f.matches.len(), f.current))
            .unwrap_or_default()
    };
    assert_eq!(find(&harness), ("fox".to_owned(), 3, Some(0)));
    harness.get_by_label("1 of 3 matches");
    assert_eq!(
        harness.state().viewer(viewer).map(|s| s.drawn_highlights),
        Some(3),
        "all three highlighted in place"
    );
    harness.key_press(egui::Key::Enter);
    harness.step();
    harness.step();
    assert_eq!(find(&harness).2, Some(1));
    let bar = harness
        .state()
        .viewer(viewer)
        .and_then(|s| s.rects.find)
        .unwrap_or_else(|| panic!("find bar"));
    click_in(&mut harness, bar, icons::PREVIOUS);
    assert_eq!(find(&harness).2, Some(0));
    click_in(&mut harness, bar, icons::NEXT);
    click_in(&mut harness, bar, icons::NEXT);
    assert_eq!(find(&harness).2, Some(2));
    click_in(&mut harness, bar, icons::NEXT);
    assert_eq!(find(&harness).2, Some(0), "around the end");
    click_in(&mut harness, bar, icons::CLOSE);
    assert!(
        harness
            .state()
            .viewer(viewer)
            .is_some_and(|s| s.find.is_none())
    );
    assert_eq!(
        harness.state().viewer(viewer).map(|s| s.drawn_highlights),
        Some(0)
    );
}

/// Zoom in on one JPEG, step to the next: the zoom holds; a PNG in the same
/// folder opens at Fit, since the zoom is remembered per type.
#[test]
fn the_zoom_is_remembered_across_two_jpegs() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("a.jpg"), media::photo_jpeg(80, 60))
        .unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("b.jpg"), media::photo_jpeg(60, 80))
        .unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("c.png"), png(40, 40, 50)).unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut harness = window(&fake, dir.path(), egui::vec2(1000.0, 600.0), previewers());
    let viewer = open_in_viewer(&mut harness, "a.jpg");
    let zoom = |h: &Harness<'_, RickUi>| h.state().viewer(viewer).map(|s| s.zoom);
    assert_eq!(zoom(&harness), Some(Zoom::Fit));
    let fitted = harness
        .state()
        .viewer(viewer)
        .and_then(|s| s.frame_size)
        .unwrap_or_else(|| panic!("frame"));
    assert_eq!(
        fitted,
        egui::vec2(80.0, 60.0),
        "a small picture is not enlarged to fit"
    );

    press(&mut harness, egui::Modifiers::COMMAND, egui::Key::Plus);
    assert_eq!(zoom(&harness), Some(Zoom::Free(1.25)));
    press(&mut harness, egui::Modifiers::COMMAND, egui::Key::Plus);
    assert!(matches!(zoom(&harness), Some(Zoom::Free(f)) if (f - 1.5625).abs() < 1e-4));
    wait_frame(&mut harness);
    let zoomed = harness
        .state()
        .viewer(viewer)
        .and_then(|s| s.frame_size)
        .unwrap_or_else(|| panic!("frame"));
    assert!((zoomed.x - 125.0).abs() < 0.5, "{zoomed:?}");

    press(&mut harness, egui::Modifiers::NONE, egui::Key::ArrowRight);
    step_until(&mut harness, "b.jpg", Duration::from_secs(10), |h| {
        shown(h).as_deref() == Some("b.jpg")
    });
    assert!(
        matches!(zoom(&harness), Some(Zoom::Free(f)) if (f - 1.5625).abs() < 1e-4),
        "the JPEG zoom holds: {:?}",
        zoom(&harness)
    );
    press(&mut harness, egui::Modifiers::NONE, egui::Key::ArrowRight);
    step_until(&mut harness, "c.png", Duration::from_secs(10), |h| {
        shown(h).as_deref() == Some("c.png")
    });
    assert_eq!(
        zoom(&harness),
        Some(Zoom::Fit),
        "another type, its own zoom"
    );
    press(&mut harness, egui::Modifiers::COMMAND, egui::Key::Num2);
    assert_eq!(zoom(&harness), Some(Zoom::FitWidth));
    press(&mut harness, egui::Modifiers::COMMAND, egui::Key::Num1);
    assert_eq!(zoom(&harness), Some(Zoom::Actual));
    press(&mut harness, egui::Modifiers::COMMAND, egui::Key::Num0);
    assert_eq!(zoom(&harness), Some(Zoom::Fit));
    assert!(matches!(
        harness.state().zoom_for("x.jpg"),
        Some(Zoom::Free(_))
    ));
    assert_eq!(harness.state().zoom_for("x.png"), Some(Zoom::Fit));
}

/// With the hex plugin loaded an unknown binary opens in the viewer as a
/// hex dump; without it Enter hands the file to the system, and a viewer
/// that steps onto it says there is no preview.
#[test]
fn an_unknown_binary_opens_in_hex_with_the_plugin_and_says_so_without() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let bytes: Vec<u8> = (0..4096u32).map(|i| (i * 7 % 251) as u8).collect();
    fs::write(dir.path().join("blob.bin"), &bytes).unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("a.png"), png(32, 32, 5)).unwrap_or_else(|e| panic!("{e}"));

    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut harness = window(&fake, dir.path(), egui::vec2(1000.0, 600.0), with_hex());
    let pane = harness.state().app().model().focused;
    open_in_viewer(&mut harness, "blob.bin");
    let status = harness
        .state()
        .viewer(viewer_tab(&harness).unwrap_or_else(|| panic!("viewer")))
        .map(|s| s.status.clone())
        .unwrap_or_default();
    assert!(status.starts_with("Hex · 4.0 KiB"), "{status}");
    let slot = harness.state().previews().get(pane);
    assert!(
        slot.is_some_and(|s| s.is_text() && s.has_frame()),
        "{slot:?}"
    );
    assert!(
        slot.and_then(|s| s.text_view())
            .is_some_and(|v| v.text().starts_with("00000000  00 07 0e 15")),
        "the dump starts with the offset and the bytes"
    );
    assert_eq!(
        fake.opened(),
        Vec::<std::path::PathBuf>::new(),
        "the viewer took it, not the system"
    );
    drop(harness);

    // Without the plugin.
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut harness = window(&fake, dir.path(), egui::vec2(1000.0, 600.0), previewers());
    let pane = harness.state().app().model().focused;
    let id = id_of(&harness, "blob.bin");
    harness.state_mut().app_mut().handle(
        app::Command::Select {
            pane,
            change: SelectionChange::Set(vec![id]),
        },
        &mut Vec::new(),
    );
    harness.step();
    harness.key_press(egui::Key::Enter);
    step_until(
        &mut harness,
        "opened externally",
        Duration::from_secs(10),
        |_| !fake.opened().is_empty(),
    );
    assert_eq!(
        fake.opened(),
        [dir.path().join("blob.bin")],
        "the system's default application"
    );
    assert_eq!(viewer_tab(&harness), None);

    open_in_viewer(&mut harness, "a.png");
    press(&mut harness, egui::Modifiers::NONE, egui::Key::ArrowRight);
    step_until(&mut harness, "blob.bin", Duration::from_secs(10), |h| {
        shown(h).as_deref() == Some("blob.bin")
    });
    step_until(&mut harness, "no preview", Duration::from_secs(10), |h| {
        h.state()
            .previews()
            .get(pane)
            .is_some_and(|s| s.error().is_some())
    });
    harness.get_by_label("No preview for this file");
    assert_eq!(fake.opened().len(), 1, "stepping onto it opens nothing");
}

/// The Preview tab opens right of the explorer, docks below it and tears
/// out into a window of its own, following the explorer's selection all
/// the while.
// One scenario, step by step.
#[allow(clippy::too_many_lines)]
#[test]
fn the_preview_tab_docks_below_and_tears_out() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("a.png"), png(32, 32, 5)).unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("b.txt"), "text\n").unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut harness = window(&fake, dir.path(), egui::vec2(1100.0, 700.0), previewers());
    let model = harness.state().app().model();
    let pane = model.focused;
    let window = model.focused_window;
    assert_eq!(model.preview_tab(window), None);

    harness.state_mut().queue(Intent::ToggleInspector, pane);
    harness.step();
    harness.step();
    let preview = harness
        .state()
        .app()
        .model()
        .preview_tab(window)
        .unwrap_or_else(|| panic!("preview tab"));
    let preview_pane = harness
        .state()
        .app()
        .model()
        .pane_of(preview)
        .unwrap_or_else(|| panic!("preview pane"));
    let rect_of = |h: &Harness<'_, RickUi>, p| {
        h.state()
            .pane_rects()
            .into_iter()
            .find(|(pane, _)| *pane == p)
            .map_or_else(|| panic!("no rect for {p:?}"), |(_, r)| r)
    };
    let explorer_rect = rect_of(&harness, pane);
    let preview_rect = rect_of(&harness, preview_pane);
    assert!(
        preview_rect.left() >= explorer_rect.right() - 1.0,
        "right of the explorer: {explorer_rect:?} {preview_rect:?}"
    );
    assert_eq!(
        harness.state().app().model().focused,
        pane,
        "the focus stays"
    );

    // It follows the selection.
    let id = id_of(&harness, "a.png");
    harness.state_mut().app_mut().handle(
        app::Command::Select {
            pane,
            change: SelectionChange::Set(vec![id]),
        },
        &mut Vec::new(),
    );
    step_until(
        &mut harness,
        "preview frame",
        Duration::from_secs(20),
        |h| {
            h.state()
                .previews()
                .get(pane)
                .is_some_and(ui_egui::preview::Slot::has_frame)
        },
    );
    assert_eq!(harness.state().app().preview_target(), Some((pane, id)));

    // Docked below.
    harness.state_mut().app_mut().handle(
        app::Command::MoveTab {
            tab: preview,
            to: DockTarget::Beside {
                pane,
                side: Side::Bottom,
            },
        },
        &mut Vec::new(),
    );
    harness.step();
    harness.step();
    harness.step();
    let preview_pane = harness
        .state()
        .app()
        .model()
        .pane_of(preview)
        .unwrap_or_else(|| panic!("preview pane"));
    let explorer_rect = rect_of(&harness, pane);
    let preview_rect = rect_of(&harness, preview_pane);
    assert!(
        preview_rect.top() >= explorer_rect.bottom() - 1.0
            && (preview_rect.left() - explorer_rect.left()).abs() < 1.0,
        "below the explorer: {explorer_rect:?} {preview_rect:?}"
    );
    assert_eq!(
        harness.state().app().model().preview_tab(window),
        Some(preview)
    );
    harness.state_mut().queue(Intent::ToggleInspector, pane);
    harness.step();
    harness.step();
    assert_eq!(
        harness.state().app().model().preview_tab(window),
        None,
        "the toggle closes it wherever it is"
    );
    harness.state_mut().queue(Intent::ToggleInspector, pane);
    harness.step();
    harness.step();
    let preview = harness
        .state()
        .app()
        .model()
        .preview_tab(window)
        .unwrap_or_else(|| panic!("preview tab"));

    // Torn out: a window of its own; the preview still follows the first
    // window's explorer.
    harness.state_mut().app_mut().handle(
        app::Command::MoveTab {
            tab: preview,
            to: DockTarget::NewWindow,
        },
        &mut Vec::new(),
    );
    harness.step();
    harness.step();
    let model = harness.state().app().model();
    assert_eq!(model.windows().count(), 2);
    let other = model
        .windows()
        .map(|(id, _)| id)
        .find(|id| *id != window)
        .unwrap_or_else(|| panic!("second window"));
    assert_eq!(model.preview_tab(other), Some(preview));
    assert_eq!(model.preview_tab(window), None);
    assert_eq!(
        model.preview_follows(),
        model.active_tab(pane).map(|(id, _)| id)
    );
    let id = id_of(&harness, "b.txt");
    harness.state_mut().app_mut().handle(
        app::Command::Select {
            pane,
            change: SelectionChange::Set(vec![id]),
        },
        &mut Vec::new(),
    );
    harness.step();
    assert_eq!(harness.state().app().preview_target(), Some((pane, id)));
}

/// Stepping through 50 images with Right: every UI frame under 16.6 ms, the
/// decodes running off the UI thread. Release builds on Linux assert;
/// others print.
#[test]
#[ignore = "timing; run with --ignored in release"]
fn stepping_through_fifty_images_keeps_every_frame_under_the_budget() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    for i in 0..50u32 {
        fs::write(
            dir.path().join(format!("img_{i:02}.png")),
            png(640, 480, u8::try_from(i * 5).unwrap_or(0)),
        )
        .unwrap_or_else(|e| panic!("{e}"));
    }
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut harness = window(&fake, dir.path(), egui::vec2(1200.0, 800.0), previewers());
    let pane = harness.state().app().model().focused;
    let mut worsts = Vec::new();
    for run in 0..tests::bench::runs() {
        if run > 0 {
            press(&mut harness, egui::Modifiers::NONE, egui::Key::Home);
            wait_frame(&mut harness);
        } else {
            open_in_viewer(&mut harness, "img_00.png");
        }
        let mut frames = Vec::new();
        for i in 1..50usize {
            harness.key_press(egui::Key::ArrowRight);
            let started = Instant::now();
            harness.step();
            frames.push(started.elapsed());
            let want = format!("img_{i:02}.png");
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let started = Instant::now();
                harness.step();
                frames.push(started.elapsed());
                let there = shown(&harness).as_deref() == Some(want.as_str())
                    && harness
                        .state()
                        .previews()
                        .get(pane)
                        .is_some_and(|s| s.has_frame() && !s.rendering());
                if there {
                    break;
                }
                assert!(Instant::now() < deadline, "{want} never showed");
            }
        }
        frames.sort_unstable();
        let worst = frames[frames.len() - 1];
        let p95 = frames[frames.len() * 95 / 100];
        println!(
            "viewer: {} frames stepping through 50 images, p95 {p95:?}, worst {worst:?}",
            frames.len()
        );
        worsts.push(worst);
    }
    tests::bench::record("viewer-frame", &worsts);
}

/// A media previewer standing in for milestone 4's: `*.fakemedia` files are
/// a 90-second clip whose transport lives in memory.
struct FakeMedia {
    manifest: ext::Manifest,
}

impl FakeMedia {
    fn new() -> Self {
        Self {
            manifest: ext::Manifest {
                id: types::ExtId("fake-media"),
                name: "Fake media",
                version: ext::Version::new(0, 1, 0),
                api_version: ext::API_VERSION,
                requires: ext::Capabilities::empty(),
                points: ext::Points::PREVIEWER,
                platforms: ext::Platforms::ALL,
            },
        }
    }
}

impl Extension for FakeMedia {
    fn manifest(&self) -> &ext::Manifest {
        &self.manifest
    }

    fn register(&self, reg: &mut ext::Registry) -> Result<(), types::Error> {
        reg.previewer(Arc::new(FakeMediaPreviewer));
        Ok(())
    }
}

struct FakeMediaPreviewer;

#[ext::async_trait]
impl ext::Previewer for FakeMediaPreviewer {
    fn id(&self) -> types::ExtId {
        types::ExtId("fake-media")
    }

    fn score(&self, entry: &types::Entry, _sniff: &[u8]) -> u8 {
        if entry.name.ends_with(".fakemedia") {
            255
        } else {
            0
        }
    }

    async fn open(
        &self,
        _loc: &types::Location,
        _provider: Arc<dyn ext::Provider>,
        _cancel: types::CancelToken,
    ) -> Result<Box<dyn ext::PreviewDoc>, types::Error> {
        Ok(Box::new(FakeMediaDoc {
            transport: FakeTransport {
                playing: false,
                position: Duration::ZERO,
                duration: Duration::from_secs(90),
            },
        }))
    }
}

struct FakeTransport {
    playing: bool,
    position: Duration,
    duration: Duration,
}

impl ext::Transport for FakeTransport {
    fn play(&mut self) {
        self.playing = true;
    }
    fn pause(&mut self) {
        self.playing = false;
    }
    fn seek(&mut self, to: Duration) {
        self.position = to.min(self.duration);
    }
    fn position(&self) -> Duration {
        self.position
    }
    fn duration(&self) -> Option<Duration> {
        Some(self.duration)
    }
    fn playing(&self) -> bool {
        self.playing
    }
}

struct FakeMediaDoc {
    transport: FakeTransport,
}

impl ext::PreviewDoc for FakeMediaDoc {
    fn pages(&self) -> Option<usize> {
        None
    }
    fn render(
        &mut self,
        _page: usize,
        _req: ext::RenderRequest,
    ) -> Result<ext::Frame, types::Error> {
        Ok(ext::Frame::Rgba {
            width: 16,
            height: 9,
            data: vec![40u8; 16 * 9 * 4].into(),
        })
    }
    fn text_layer(&self) -> Option<&ext::TextLayer> {
        None
    }
    fn sections(&self) -> Vec<ext::InfoSection> {
        vec![ext::InfoSection {
            id: "file",
            title: Text("File info"),
            icon: ext::Icon::Info,
            rows: vec![("Duration".to_owned(), "1:30".to_owned())],
        }]
    }
    fn status(&self) -> Vec<String> {
        vec!["Fake media".to_owned(), "1:30".to_owned()]
    }
    fn controls(&self) -> Vec<ext::PreviewControl> {
        Vec::new()
    }
    fn set_control(
        &mut self,
        _id: ext::ControlId,
        _value: ext::ControlValue,
    ) -> Result<(), types::Error> {
        Err(types::Error::Unsupported("no controls"))
    }
    fn transport(&mut self) -> Option<&mut dyn ext::Transport> {
        Some(&mut self.transport)
    }
}

/// A document with a transport gets the transport bar: play and pause from
/// the button and Space, J and L seek ten seconds, K pauses, and the clock
/// follows.
#[test]
fn media_gets_a_transport_bar_with_the_player_keys() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("clip.fakemedia"), b"not really media")
        .unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut extensions = previewers();
    extensions.push(Arc::new(FakeMedia::new()));
    let mut harness = window(&fake, dir.path(), egui::vec2(1000.0, 600.0), extensions);
    let viewer = open_in_viewer(&mut harness, "clip.fakemedia");
    let bar = harness
        .state()
        .viewer(viewer)
        .and_then(|s| s.rects.transport)
        .unwrap_or_else(|| panic!("a transport bar for media"));
    let in_bar = |h: &Harness<'_, RickUi>, glyph: &str| {
        h.get_all_by_label(glyph)
            .any(|node| bar.contains(node.rect().center()))
    };
    harness.get_by_label("0:00 / 1:30");
    assert!(in_bar(&harness, icons::PLAY), "paused: the play button");

    press(&mut harness, egui::Modifiers::NONE, egui::Key::Space);
    assert!(in_bar(&harness, icons::PAUSE), "Space plays");
    press(&mut harness, egui::Modifiers::NONE, egui::Key::L);
    harness.get_by_label("0:10 / 1:30");
    press(&mut harness, egui::Modifiers::NONE, egui::Key::L);
    harness.get_by_label("0:20 / 1:30");
    press(&mut harness, egui::Modifiers::NONE, egui::Key::J);
    harness.get_by_label("0:10 / 1:30");
    press(&mut harness, egui::Modifiers::NONE, egui::Key::K);
    assert!(in_bar(&harness, icons::PLAY), "K pauses");
    press(&mut harness, egui::Modifiers::NONE, egui::Key::J);
    press(&mut harness, egui::Modifiers::NONE, egui::Key::J);
    harness.get_by_label("0:00 / 1:30");
    click_in(&mut harness, bar, icons::PLAY);
    assert!(in_bar(&harness, icons::PAUSE), "the button plays");
    click_in(&mut harness, bar, icons::PAUSE);
    assert!(in_bar(&harness, icons::PLAY), "and pauses");
    assert_eq!(
        harness.state().viewer(viewer).map(|s| s.status.clone()),
        Some("Fake media · 1:30".to_owned())
    );
}
