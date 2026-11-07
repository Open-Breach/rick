//! Acceptance (preview pane layout): the pane's toolbar, status bar and info
//! badges; wrap, line numbers and pretty print; the pinned section per
//! content kind surviving files and restarts; the pane's width never set by
//! its content; and the Git badge only inside a work tree.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use ext::Extension;
use model::SelectionChange;
use platform::{ConfigDir, Fake};
use tests::{local, media};
use ui_egui::keys::Intent;
use ui_egui::{EguiHost, RickUi, icons};

/// The four previewers.
fn previewers() -> Vec<Arc<dyn Extension>> {
    vec![
        Arc::new(ext_preview_text::TextPreview::new()),
        Arc::new(ext_preview_code::CodePreview::new()),
        Arc::new(ext_preview_image::ImagePreview::new()),
        Arc::new(ext_preview_pdf::PdfPreview::new()),
    ]
}

/// A window on `start`, sharing `fake` (and so its settings file) with
/// earlier windows, with the preview pane `pane_width` wide and open.
fn window(
    fake: &Arc<Fake>,
    start: &Path,
    size: egui::Vec2,
    pane_width: f32,
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
        .build_eframe(move |cc| {
            let mut ui = RickUi::new(app, host, &cc.egui_ctx);
            ui.set_inspector_width(pane_width);
            ui
        });
    settle(&mut harness);
    let pane = harness.state().app().model().focused;
    harness.state_mut().queue(Intent::ToggleInspector, pane);
    harness.step();
    harness.step();
    assert!(harness.state().inspector_open());
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

/// Selects `name` in the focused pane and steps until its preview has a
/// frame.
fn select(harness: &mut Harness<'_, RickUi>, name: &str) {
    let pane = harness.state().app().model().focused;
    let id = harness
        .state()
        .app()
        .model()
        .active_tab(pane)
        .and_then(|(_, t)| t.listing.rows().find(|e| e.name == name).map(|e| e.id))
        .unwrap_or_else(|| panic!("no entry {name}"));
    harness.state_mut().app_mut().handle(
        app::Command::Select {
            pane,
            change: SelectionChange::Set(vec![id]),
        },
        &mut Vec::new(),
    );
    let started = Instant::now();
    loop {
        harness.step();
        let slot = harness.state().previews().get(pane);
        if slot.is_some_and(|s| s.has_frame() || s.error().is_some()) {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "{name}: no frame: {slot:?}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    harness.step();
    harness.step();
}

fn toolbar_item(harness: &Harness<'_, RickUi>, label: &str) -> Option<bool> {
    harness
        .state()
        .preview_toolbar()
        .iter()
        .find(|item| item.label == label)
        .unwrap_or_else(|| {
            panic!(
                "no toolbar item {label}: {:?}",
                harness.state().preview_toolbar()
            )
        })
        .on
}

/// The ids of the pinned badges, in order.
fn pinned_ids(harness: &Harness<'_, RickUi>) -> Vec<String> {
    harness
        .state()
        .preview_badges()
        .iter()
        .filter(|b| b.pinned)
        .map(|b| b.id.clone())
        .collect()
}

/// Clicks the widget labelled `glyph` inside the preview pane; the same
/// glyph may serve elsewhere in the window (the File info badge shares its
/// glyph with the title row's toggle).
fn click_in_pane(harness: &mut Harness<'_, RickUi>, glyph: &str) {
    let pane_rect = harness
        .state()
        .inspector_rect()
        .unwrap_or_else(|| panic!("pane"));
    harness
        .get_all_by_label(glyph)
        .find(|node| pane_rect.contains(node.rect().center()))
        .unwrap_or_else(|| panic!("no {glyph:?} in the pane"))
        .click();
}

/// Hovers the widget labelled `glyph` and steps until `label` shows: the
/// overlay comes after egui's tooltip delay.
fn hover_until(harness: &mut Harness<'_, RickUi>, glyph: &str, label: &str) {
    let started = Instant::now();
    // One move, then the pointer stays still: egui shows a tooltip only
    // after the pointer has rested for the delay.
    harness.get_by_label(glyph).hover();
    loop {
        harness.step();
        if harness.query_by_label(label).is_some() {
            return;
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "hovering {glyph:?} never showed {label:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn settings_text(harness: &Harness<'_, RickUi>) -> String {
    fs::read_to_string(harness.state().app().settings_path()).unwrap_or_default()
}

fn wait_settings(harness: &mut Harness<'_, RickUi>, needle: &str) -> String {
    let started = Instant::now();
    loop {
        settle(harness);
        let text = settings_text(harness);
        if text.contains(needle) {
            return text;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "settings never got {needle:?}: {text}"
        );
    }
}

/// No "Inspector"; the toolbar has the file icon, Line numbers on and Wrap
/// off; the status bar reads language, encoding and lines; toggling Wrap
/// writes the setting and the next file opens wrapped.
#[test]
fn a_text_file_shows_its_toolbar_and_status_and_wrap_is_remembered() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("notes.txt"), "Hello\nworld\n").unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("other.txt"), "one\ntwo\nthree\n").unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut harness = window(
        &fake,
        dir.path(),
        egui::vec2(1200.0, 700.0),
        320.0,
        previewers(),
    );
    select(&mut harness, "notes.txt");

    assert!(
        harness.query_by_label_contains("Inspector").is_none(),
        "the word Inspector appears nowhere"
    );
    // The file icon sits in the pane's toolbar (and in the list row).
    let pane_rect = harness
        .state()
        .inspector_rect()
        .unwrap_or_else(|| panic!("pane"));
    assert!(
        harness
            .get_all_by_label(icons::by_extension("notes.txt"))
            .any(|node| pane_rect.contains(node.rect().center())),
        "the file icon is in the pane's toolbar"
    );
    let labels: Vec<&str> = harness
        .state()
        .preview_toolbar()
        .iter()
        .map(|i| i.label.as_str())
        .collect();
    assert_eq!(labels, ["File", "Line numbers", "Word wrap"]);
    assert_eq!(toolbar_item(&harness, "Line numbers"), Some(true));
    assert_eq!(toolbar_item(&harness, "Word wrap"), Some(false));
    assert_eq!(
        harness.state().preview_status(),
        "Plain text · UTF-8 · 2 lines"
    );

    harness.get_by_label(icons::WRAP).click();
    harness.step();
    harness.step();
    assert_eq!(toolbar_item(&harness, "Word wrap"), Some(true));
    let text = wait_settings(&mut harness, "wrap = true");
    assert!(text.contains("[preview.text]"), "{text}");
    assert!(text.contains("line_numbers = true"), "{text}");

    select(&mut harness, "other.txt");
    assert_eq!(
        toolbar_item(&harness, "Word wrap"),
        Some(true),
        "the default changed"
    );
    assert_eq!(
        harness.state().preview_status(),
        "Plain text · UTF-8 · 3 lines"
    );
}

/// Pretty print is off for every newly shown file and never written down.
#[test]
fn pretty_print_starts_off_for_every_file() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    fs::write(
        dir.path().join("data.json"),
        r#"{"a":[1,2,3],"b":{"c":true}}"#,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("more.json"), r#"{"x":1}"#).unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut harness = window(
        &fake,
        dir.path(),
        egui::vec2(1200.0, 700.0),
        320.0,
        previewers(),
    );
    let pane = harness.state().app().model().focused;
    select(&mut harness, "data.json");
    assert_eq!(toolbar_item(&harness, "Pretty print"), Some(false));
    let raw_lines = harness
        .state()
        .previews()
        .get(pane)
        .map_or(0, |s| s.drawn_lines().len());
    assert_eq!(raw_lines, 1, "one minified line");
    assert_eq!(harness.state().preview_status(), "JSON · UTF-8 · 1 lines");

    harness.get_by_label(icons::PRETTY_PRINT).click();
    let started = Instant::now();
    loop {
        harness.step();
        if toolbar_item(&harness, "Pretty print") == Some(true)
            && harness
                .state()
                .previews()
                .get(pane)
                .is_some_and(|s| s.drawn_lines().len() > 1)
        {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "pretty print never showed"
        );
        std::thread::sleep(Duration::from_millis(2));
    }

    select(&mut harness, "more.json");
    assert_eq!(
        toolbar_item(&harness, "Pretty print"),
        Some(false),
        "off again"
    );
    settle(&mut harness);
    assert!(
        !settings_text(&harness).contains("pretty"),
        "never remembered"
    );
}

/// Wrap off: a 400-character line is one row wider than the pane behind a
/// horizontal scrollbar. Wrap on: several rows, no horizontal scrollbar,
/// every row starting at the text column past the gutter.
#[test]
fn wrap_off_scrolls_sideways_and_wrap_on_breaks_at_the_pane_width() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let long: String = (0..400u32)
        .map(|i| char::from(b'a' + u8::try_from(i % 26).unwrap_or(0)))
        .collect();
    fs::write(dir.path().join("long.txt"), format!("{long}\nshort\n"))
        .unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut harness = window(
        &fake,
        dir.path(),
        egui::vec2(1100.0, 700.0),
        300.0,
        previewers(),
    );
    let pane = harness.state().app().model().focused;
    select(&mut harness, "long.txt");

    let slot = harness
        .state()
        .previews()
        .get(pane)
        .unwrap_or_else(|| panic!("slot"));
    let content = harness
        .state()
        .preview_rects()
        .content
        .unwrap_or_else(|| panic!("content"));
    assert!(content.width() <= 300.0, "{content:?}");
    let first = slot
        .drawn_lines()
        .iter()
        .find(|l| l.line == 0)
        .unwrap_or_else(|| panic!("line 0 not drawn: {:?}", slot.drawn_lines()));
    assert_eq!(first.rows, 1, "one row with wrap off");
    assert!(
        first.width > content.width(),
        "{first:?} wider than {content:?}"
    );
    assert!(slot.h_scrollbar(), "a horizontal scrollbar");
    assert!(first.x > first.gutter_x, "a gutter before the text");

    harness.get_by_label(icons::WRAP).click();
    harness.step();
    harness.step();
    harness.step();
    let slot = harness
        .state()
        .previews()
        .get(pane)
        .unwrap_or_else(|| panic!("slot"));
    let first = slot
        .drawn_lines()
        .iter()
        .find(|l| l.line == 0)
        .unwrap_or_else(|| panic!("line 0 not drawn: {:?}", slot.drawn_lines()));
    assert!(first.rows > 1, "{first:?} wraps");
    assert!(first.width <= content.width(), "{first:?} fits {content:?}");
    assert!(!slot.h_scrollbar(), "no horizontal scrollbar with wrap on");
    for line in slot.drawn_lines() {
        assert!(
            line.x > line.gutter_x + 8.0,
            "{line:?}: every row starts at the text column, past the gutter"
        );
        assert!(
            (line.x - first.x).abs() < 0.5,
            "{line:?} shares the text column"
        );
    }
}

/// With wrap on, a 200 000-line file costs each frame under 16.6 ms: the
/// frame that toggles wrap and the first three scroll frames. Release
/// builds on Linux assert; others print.
#[test]
#[ignore = "timing; run with --ignored in release"]
fn two_hundred_thousand_wrapped_lines_keep_every_frame_under_the_budget() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let mut text = String::with_capacity(200_000 * 40);
    for i in 0..200_000u32 {
        text.push_str("line ");
        text.push_str(&i.to_string());
        text.push(' ');
        text.push_str(&"x".repeat((i % 97) as usize));
        text.push('\n');
    }
    fs::write(dir.path().join("big.log"), text).unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut harness = window(
        &fake,
        dir.path(),
        egui::vec2(1200.0, 800.0),
        360.0,
        previewers(),
    );
    let pane = harness.state().app().model().focused;
    select(&mut harness, "big.log");
    let content = harness
        .state()
        .preview_rects()
        .content
        .unwrap_or_else(|| panic!("content"));

    harness.get_by_label(icons::WRAP).click();
    let started = Instant::now();
    harness.step();
    let mut frames = vec![("toggle wrap", started.elapsed())];
    harness.step();
    assert_eq!(toolbar_item(&harness, "Word wrap"), Some(true));
    harness.event(egui::Event::PointerMoved(content.center()));
    harness.step();
    for i in 0..3 {
        harness.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -400.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::default(),
        });
        let started = Instant::now();
        harness.step();
        frames.push((["scroll 1", "scroll 2", "scroll 3"][i], started.elapsed()));
    }
    let drawn = harness
        .state()
        .previews()
        .get(pane)
        .map_or(0, |s| s.drawn_lines().len());
    println!("200k lines wrapped: {frames:?}; {drawn} lines laid out in the last frame");
    assert!(
        drawn > 0 && drawn < 400,
        "only the visible lines are laid out: {drawn}"
    );
    let budget = Duration::from_micros(16_600);
    let release = !cfg!(debug_assertions);
    for (name, took) in &frames {
        if release && cfg!(target_os = "linux") {
            assert!(*took < budget, "{name} took {took:?}");
        } else if *took >= budget {
            println!("{name} took {took:?} (over budget; not asserted here)");
        }
    }
}

/// A photo has File info and EXIF badges; hovering shows the rows, clicking
/// pins the panel (and pushes the content down), pinning another replaces
/// it, and the pin survives another photo and a restart.
#[test]
fn exif_badges_hover_pin_and_survive_files_and_restarts() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("a.jpg"), media::photo_jpeg(80, 60))
        .unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("b.jpg"), media::photo_jpeg(60, 80))
        .unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut harness = window(
        &fake,
        dir.path(),
        egui::vec2(1200.0, 700.0),
        340.0,
        previewers(),
    );
    select(&mut harness, "a.jpg");

    let titles: Vec<(String, bool)> = harness
        .state()
        .preview_badges()
        .iter()
        .map(|b| (b.title.clone(), b.pinned))
        .collect();
    assert_eq!(
        titles,
        [("File info".to_owned(), false), ("EXIF".to_owned(), false)]
    );
    assert!(
        harness.state().preview_rects().panel.is_none(),
        "nothing pinned yet"
    );
    assert!(
        harness.query_by_label("Make").is_none(),
        "rows hidden until hovered"
    );

    // Hover: the overlay with the rows.
    hover_until(&mut harness, icons::CAMERA, "Make");
    harness.get_by_label("Rick Cameras");

    // Click: pinned, the content moves down by the panel's height.
    let content_before = harness
        .state()
        .preview_rects()
        .content
        .unwrap_or_else(|| panic!("content"));
    harness.get_by_label(icons::CAMERA).click();
    settle(&mut harness);
    let rects = harness.state().preview_rects();
    let panel = rects
        .panel
        .unwrap_or_else(|| panic!("EXIF pinned: {rects:?}"));
    let content_after = rects.content.unwrap_or_else(|| panic!("content"));
    assert!(panel.height() > 40.0, "{panel:?}");
    assert!(
        (content_after.top() - (content_before.top() + panel.height() + 1.0)).abs() < 1.5,
        "content {content_before:?} -> {content_after:?}, panel {panel:?}"
    );
    assert!(
        harness
            .state()
            .preview_badges()
            .iter()
            .any(|b| b.id == "exif" && b.pinned),
        "{:?}",
        harness.state().preview_badges()
    );
    assert!(
        settings_text(&harness).contains("image = \"exif\"")
            || wait_settings(&mut harness, "image = \"exif\"").contains("exif")
    );

    // Pinning File info unpins EXIF.
    click_in_pane(&mut harness, icons::INFO);
    settle(&mut harness);
    assert_eq!(pinned_ids(&harness), ["file"]);

    // Another photo keeps the pin.
    select(&mut harness, "b.jpg");
    assert_eq!(
        pinned_ids(&harness),
        ["file"],
        "the pin follows the content kind"
    );
    wait_settings(&mut harness, "image = \"file\"");

    // And a restart.
    drop(harness);
    let mut harness = window(
        &fake,
        dir.path(),
        egui::vec2(1200.0, 700.0),
        340.0,
        previewers(),
    );
    select(&mut harness, "a.jpg");
    assert_eq!(pinned_ids(&harness), ["file"], "the pin survives a restart");
    assert!(harness.state().preview_rects().panel.is_some());
}

/// A landscape A3 page and a 6000-pixel-wide image in a 300-point pane: the
/// pane stays 300 points wide and the frame is drawn no wider.
#[test]
fn wide_frames_fit_a_narrow_pane_instead_of_widening_it() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("a3.pdf"), media::pdf(1, 1190, 842))
        .unwrap_or_else(|e| panic!("{e}"));
    let wide = image::RgbaImage::from_fn(6000, 400, |x, y| {
        image::Rgba([
            u8::try_from(x / 24).unwrap_or(255),
            u8::try_from(y / 2).unwrap_or(255),
            90,
            255,
        ])
    });
    let mut bytes = std::io::Cursor::new(Vec::new());
    wide.write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("wide.png"), bytes.into_inner()).unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut harness = window(
        &fake,
        dir.path(),
        egui::vec2(1100.0, 700.0),
        300.0,
        previewers(),
    );
    let pane = harness.state().app().model().focused;
    let pane_width = harness.state().inspector_rect().map_or(0.0, |r| r.width());
    assert!((pane_width - 300.0).abs() < 1.0, "{pane_width}");

    let check = |harness: &mut Harness<'_, RickUi>, name: &str| {
        select(harness, name);
        for _ in 0..3 {
            harness.step();
        }
        let width = harness.state().inspector_rect().map_or(0.0, |r| r.width());
        assert!(
            (width - 300.0).abs() < 1.0,
            "{name}: the pane is {width} wide"
        );
        let slot = harness
            .state()
            .previews()
            .get(pane)
            .unwrap_or_else(|| panic!("slot"));
        assert!(slot.has_frame(), "{name}: {slot:?}");
        let frame = slot
            .frame_rect()
            .unwrap_or_else(|| panic!("{name}: no frame rect"));
        assert!(frame.width() <= 300.0 + 0.5, "{name}: frame {frame:?}");
        let content = harness
            .state()
            .preview_rects()
            .content
            .unwrap_or_else(|| panic!("content"));
        assert!(
            frame.width() > content.width() * 0.8,
            "{name}: the frame uses the width: {frame:?} in {content:?}"
        );
    };
    check(&mut harness, "wide.png");
    // A raster has Fit and 1:1, with Fit on.
    assert_eq!(toolbar_item(&harness, "Fit"), Some(true));
    assert_eq!(toolbar_item(&harness, "1:1"), Some(false));
    assert!(harness.state().preview_status().starts_with("6000 × 400"));
    if ext_preview_pdf::available() {
        check(&mut harness, "a3.pdf");
        // A paged document has the pager instead of Fit and 1:1.
        assert_eq!(toolbar_item(&harness, "Previous page"), None);
        assert_eq!(toolbar_item(&harness, "Next page"), None);
        assert!(
            !harness
                .state()
                .preview_toolbar()
                .iter()
                .any(|item| item.label == "Fit"),
            "{:?}",
            harness.state().preview_toolbar()
        );
        assert!(harness.state().preview_status().starts_with("1 page"));
    } else {
        assert!(
            std::env::var_os("RICK_REQUIRE_PDFIUM").is_none(),
            "pdfium required but not found"
        );
    }
}

/// A fitted picture sits in the middle of the content; at 1:1 it pans by
/// dragging; the status bar is as high as the file list's status line.
#[test]
fn fitted_frames_are_centred_and_actual_size_pans_by_dragging() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let wide = image::RgbaImage::from_fn(1600, 300, |x, y| {
        image::Rgba([
            u8::try_from(x / 7).unwrap_or(255),
            u8::try_from(y).unwrap_or(255),
            120,
            255,
        ])
    });
    let mut bytes = std::io::Cursor::new(Vec::new());
    wide.write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("wide.png"), bytes.into_inner()).unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut harness = window(
        &fake,
        dir.path(),
        egui::vec2(1100.0, 700.0),
        400.0,
        previewers(),
    );
    let pane = harness.state().app().model().focused;
    select(&mut harness, "wide.png");

    // Fitted: centred both ways in the content.
    let rects = harness.state().preview_rects();
    let content = rects.content.unwrap_or_else(|| panic!("content"));
    let frame = harness
        .state()
        .previews()
        .get(pane)
        .and_then(ui_egui::preview::Slot::frame_rect)
        .unwrap_or_else(|| panic!("frame"));
    assert!(
        (frame.center() - content.center()).length() < 1.5,
        "frame {frame:?} centred in {content:?}"
    );
    assert!(
        frame.width() <= content.width() + 0.5 && frame.width() < 1600.0,
        "fitted into the pane: {frame:?} in {content:?}"
    );

    // The status bar is a body-text row, like the file list's status line.
    let status = rects.status.unwrap_or_else(|| panic!("status"));
    let body = egui::TextStyle::Body
        .resolve(&harness.ctx.style_of(egui::Theme::Light))
        .size;
    assert!(
        status.height() >= body + 5.0,
        "status {status:?} for body {body}"
    );

    // 1:1: the picture is wider than the pane and dragging it pans.
    click_in_pane(&mut harness, icons::ACTUAL_SIZE);
    let started = Instant::now();
    loop {
        harness.step();
        let slot = harness.state().previews().get(pane);
        if slot.is_some_and(|s| s.actual_size && s.has_frame() && !s.rendering()) {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "no 1:1 frame: {slot:?}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    harness.step();
    assert_eq!(toolbar_item(&harness, "1:1"), Some(true));
    let before = harness
        .state()
        .previews()
        .get(pane)
        .map_or(egui::Vec2::ZERO, ui_egui::preview::Slot::pan_offset);
    assert_eq!(before, egui::Vec2::ZERO);
    let from = content.center();
    drag(&mut harness, from, from - egui::vec2(120.0, 0.0));
    let after = harness
        .state()
        .previews()
        .get(pane)
        .map_or(egui::Vec2::ZERO, ui_egui::preview::Slot::pan_offset);
    assert!(
        after.x > 60.0 && after.y.abs() < 1.0,
        "dragging left pans right: {before:?} -> {after:?}"
    );
}

/// Presses the primary button at `from`, moves to `to` in small steps and
/// releases.
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
    for i in 1..=8u8 {
        let pos = from + (to - from) * (f32::from(i) / 8.0);
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
}

/// Runs `git` in `dir`.
fn git(dir: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .unwrap_or_else(|e| panic!("git: {e}"));
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The Git badge appears for a file inside a work tree and not outside.
#[test]
fn the_git_badge_appears_only_inside_a_work_tree() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let repo = dir.path().join("repo");
    let plain = dir.path().join("plain");
    fs::create_dir_all(&repo).unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir_all(&plain).unwrap_or_else(|e| panic!("{e}"));
    fs::write(repo.join("tracked.txt"), "one\n").unwrap_or_else(|e| panic!("{e}"));
    fs::write(plain.join("loose.txt"), "two\n").unwrap_or_else(|e| panic!("{e}"));
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.name", "Rick Test"]);
    git(&repo, &["config", "user.email", "rick@example.com"]);
    git(&repo, &["config", "commit.gpgsign", "false"]);
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "first"]);

    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut extensions = previewers();
    extensions.push(Arc::new(ext_git::GitExtension::new()));
    let mut harness = window(&fake, &repo, egui::vec2(1200.0, 700.0), 320.0, extensions);
    select(&mut harness, "tracked.txt");
    let badges: Vec<&str> = harness
        .state()
        .preview_badges()
        .iter()
        .map(|b| b.id.as_str())
        .collect();
    assert_eq!(
        badges,
        ["file", "git"],
        "{:?}",
        harness.state().preview_badges()
    );
    hover_until(&mut harness, icons::BRANCH, "Branch");
    harness.get_by_label("main");

    let pane = harness.state().app().model().focused;
    harness.state_mut().app_mut().handle(
        app::Command::Navigate {
            pane,
            to: local(&plain),
        },
        &mut Vec::new(),
    );
    settle(&mut harness);
    select(&mut harness, "loose.txt");
    let badges: Vec<&str> = harness
        .state()
        .preview_badges()
        .iter()
        .map(|b| b.id.as_str())
        .collect();
    assert_eq!(badges, ["file"]);
    assert!(harness.query_by_label(icons::BRANCH).is_none());
}
