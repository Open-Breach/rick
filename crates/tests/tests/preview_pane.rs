//! Acceptance (preview pane layout): the Preview tab's toolbar, status bar
//! and File info card; wrap, line numbers and pretty print; the card
//! hanging under the info button, pinned per previewer and surviving files
//! and restarts; the centred card where there is nothing to show, and the
//! empty state; the pane's width never set by its content; and the Git
//! section only inside a work tree.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use ext::Extension;
use model::SelectionChange;
use platform::{Clipboard as _, ConfigDir, Fake};
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
/// earlier windows, with the Preview tab open in a pane `pane_width` wide.
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
    // A restored session brings the Preview tab back; open it otherwise.
    if !harness.state().inspector_open() {
        let pane = harness.state().app().model().focused;
        harness.state_mut().queue(Intent::ToggleInspector, pane);
        harness.step();
    }
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
        // The rects say whether the Preview tab was drawn at all: they are
        // set by the pane's draw and stay `None` while it never ran.
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "{name}: no frame: {slot:?}; follows {:?}, preview pane {:?}, target {:?}, \
             inspector open {}, rects {:?}",
            harness.state().app().model().preview_follows(),
            harness.state().app().preview_pane(),
            harness.state().app().preview_target(),
            harness.state().inspector_open(),
            harness.state().preview_rects(),
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

/// The labels of the File info card's rows, in order.
fn field_labels(harness: &Harness<'_, RickUi>) -> Vec<String> {
    harness
        .state()
        .preview_fields()
        .iter()
        .map(|(label, _)| label.clone())
        .collect()
}

/// The value of the File info card's row `label`.
fn field(harness: &Harness<'_, RickUi>, label: &str) -> String {
    harness
        .state()
        .preview_fields()
        .iter()
        .find(|(l, _)| l == label)
        .map_or_else(
            || panic!("no field {label}: {:?}", harness.state().preview_fields()),
            |(_, value)| value.clone(),
        )
}

/// Clicks the widget labelled `glyph` inside the preview pane; the same
/// glyph or name may serve elsewhere in the window.
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
    assert_eq!(
        labels,
        ["File", "Line numbers", "Word wrap", "Copy"],
        "the text controls, then Copy closing the middle group"
    );
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
    assert!(content.width() <= 304.0, "{content:?}");
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

    let mut worsts = Vec::new();
    for run in 0..tests::bench::runs() {
        if run > 0 {
            // Wrap off again, so the next run toggles it on from the start.
            harness.get_by_label(icons::WRAP).click();
            harness.step();
            harness.step();
            assert_eq!(toolbar_item(&harness, "Word wrap"), Some(false));
        }
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
        worsts.push(
            frames
                .iter()
                .map(|(_, took)| *took)
                .max()
                .unwrap_or_default(),
        );
    }
    tests::bench::record("wrap-frame", &worsts);
}

/// One card for a photo's facts (SPEC.md *File info card*): its rows are
/// the previewer's fields with *Modified* after *Size*; hovering the info
/// button hangs the card under it with the first five and *Show all n
/// fields*; the pin, or a click on the button, pins it above the content,
/// which moves down; the pin is the previewer's, so another photo keeps it
/// and so does a restart; a click unpins.
#[test]
#[allow(clippy::too_many_lines)]
fn the_file_info_card_hovers_pins_and_the_pin_survives_files_and_restarts() {
    use ui_egui::inspector::Info;
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
        440.0,
        previewers(),
    );
    select(&mut harness, "a.jpg");

    assert_eq!(harness.state().preview_info(), Info::Unpinned);
    let labels = field_labels(&harness);
    assert_eq!(
        labels[..4],
        ["Format", "Dimensions", "Size", "Modified"],
        "the pane supplies Modified after Size: {labels:?}"
    );
    assert_eq!(field(&harness, "Camera"), "Rick Cameras");
    assert_eq!(field(&harness, "Dimensions"), "80 × 60");
    // As the Date format setting writes it: smart with the time, and the
    // picture was written a moment ago.
    let modified = field(&harness, "Modified");
    assert!(modified.starts_with("Today "), "{modified}");
    assert_eq!(modified.len(), "Today 17:29".len(), "{modified}");
    assert!(
        harness.state().preview_rects().panel.is_none(),
        "nothing pinned yet"
    );
    assert!(
        harness.query_by_label("Rick Cameras").is_none(),
        "the rows are hidden until the button is hovered"
    );

    // Hover: the card hangs under the button, inside the pane, with the
    // name, the type and the size in its head, five rows and the line
    // that unfolds the rest.
    let all = labels.len();
    hover_until(&mut harness, "File info", "Rick Cameras");
    assert!(
        harness
            .query_all_by_label_contains("JPEG image  ·  ")
            .next()
            .is_some(),
        "the type and the size under the name"
    );
    let rects = harness.state().preview_rects();
    let card = rects
        .popover
        .unwrap_or_else(|| panic!("the popover: {rects:?}"));
    let content = rects.content.unwrap_or_else(|| panic!("content"));
    assert!(card.width() <= 360.5, "{card:?}");
    assert!(
        card.top() >= content.top() && card.right() <= content.right(),
        "{card:?} in {content:?}"
    );
    harness.get_by_label(format!("Show all {all} fields").as_str());
    assert!(
        harness.query_by_label("Model").is_none(),
        "the sixth row waits behind the line"
    );

    // The pin in the card's head pins it: the card sits above the
    // content, which keeps its place in the pane and gives up the room.
    harness.get_by_label("Pin file info").click();
    settle(&mut harness);
    let rects = harness.state().preview_rects();
    let panel = rects.panel.unwrap_or_else(|| panic!("pinned: {rects:?}"));
    assert!(rects.popover.is_none(), "{rects:?}");
    assert!(panel.height() > 150.0, "{panel:?}");
    assert!(
        (panel.width() - (content.width() - 24.0)).abs() < 1.0,
        "the pane's width inside the content's margin: {panel:?}"
    );
    assert_eq!(harness.state().preview_info(), Info::Pinned);
    wait_settings(&mut harness, "preview-image = true");
    let frame = harness
        .state()
        .previews()
        .get(
            harness
                .state()
                .app()
                .preview_pane()
                .unwrap_or_else(|| panic!("pane")),
        )
        .and_then(ui_egui::preview::Slot::frame_rect)
        .unwrap_or_else(|| panic!("a frame"));
    assert!(frame.top() >= panel.bottom(), "{frame:?} under {panel:?}");

    // Another photo keeps the pin: it is the previewer's.
    select(&mut harness, "b.jpg");
    assert_eq!(harness.state().preview_info(), Info::Pinned);
    assert_eq!(field(&harness, "Dimensions"), "60 × 80");

    // And a restart.
    drop(harness);
    let mut harness = window(
        &fake,
        dir.path(),
        egui::vec2(1200.0, 700.0),
        440.0,
        previewers(),
    );
    select(&mut harness, "a.jpg");
    assert_eq!(
        harness.state().preview_info(),
        Info::Pinned,
        "the pin survives a restart"
    );
    assert!(harness.state().preview_rects().panel.is_some());

    // Show all n fields unfolds the rest in place, each further section
    // under its title; the line folds them again.
    assert!(harness.query_by_label("EXIF").is_none());
    click_in_pane(&mut harness, &format!("Show all {all} fields"));
    settle(&mut harness);
    harness.get_by_label("EXIF");
    harness.get_by_label("Make");

    // A click on the info button unpins.
    click_in_pane(&mut harness, "File info");
    settle(&mut harness);
    assert_eq!(harness.state().preview_info(), Info::Unpinned);
    assert!(harness.state().preview_rects().panel.is_none());
    let started = Instant::now();
    while settings_text(&harness).contains("preview-image = true") {
        settle(&mut harness);
        assert!(started.elapsed() < Duration::from_secs(10), "still pinned");
    }
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
    // The dock's divider and its rounding take a few points.
    assert!((pane_width - 300.0).abs() < 4.0, "{pane_width}");

    let check = |harness: &mut Harness<'_, RickUi>, name: &str| {
        select(harness, name);
        for _ in 0..3 {
            harness.step();
        }
        let width = harness.state().inspector_rect().map_or(0.0, |r| r.width());
        assert!(
            (width - pane_width).abs() < 0.5,
            "{name}: the pane is {width} wide, it opened {pane_width} wide"
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
        assert!(frame.width() <= width + 0.5, "{name}: frame {frame:?}");
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

    // The status bar is the explorer footer's height (issue 79 item 4).
    let status = rects.status.unwrap_or_else(|| panic!("status"));
    let footer = ui_egui::footer_height(model::Density::Comfortable);
    assert!(
        (status.height() - footer).abs() < 0.5,
        "status {status:?} against {footer}"
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
fn the_git_section_appears_only_inside_a_work_tree() {
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
    // The Git section arrives with the decoration job, which may land after
    // the preview's first frame.
    let started = Instant::now();
    while !field_labels(&harness).iter().any(|l| l == "Branch")
        && started.elapsed() < Duration::from_secs(10)
    {
        harness.step();
    }
    assert_eq!(field(&harness, "Branch"), "main");
    // It is a section of the one card, after the previewer's: unfolded,
    // it stands under its title.
    let all = field_labels(&harness).len();
    let unfold = format!("Show all {all} fields");
    hover_until(&mut harness, "File info", &unfold);
    harness.get_by_label(unfold.as_str()).click();
    settle(&mut harness);
    harness.get_by_label("GIT");
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
    assert!(
        !field_labels(&harness).iter().any(|l| l == "Branch"),
        "{:?}",
        harness.state().preview_fields()
    );
}

/// The table of the pane's content as last drawn.
fn table_of(harness: &Harness<'_, RickUi>) -> ui_egui::table::Drawn {
    let pane = harness.state().app().model().focused;
    harness
        .state()
        .previews()
        .get(pane)
        .and_then(|slot| slot.content_state().table.clone())
        .unwrap_or_else(|| panic!("the content is no table"))
}

/// Issue 139, item 6 (SPEC.md *Tables in previews*): the archive tree and
/// the CSV grid are one table — a 30 px header band with a divider
/// between every pair of header cells, the rows starting right under it;
/// a divider dragged resizes the column before it, never under 48 px, and
/// the last column takes the rest; a double-click fits the column; the
/// widths are remembered per previewer, by position, across files and
/// restarts.
#[test]
#[allow(clippy::too_many_lines)]
fn preview_tables_share_a_header_whose_dividers_resize_and_are_remembered() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    fs::write(
        dir.path().join("data.csv"),
        "id,title,description,price\n1,Rick,Fast listing,0.00\n2,Multi-line field,First line,12.50\n",
    )
    .unwrap_or_else(|e| panic!("{e}"));
    for name in ["bundle.zip", "other.zip"] {
        fs::write(
            dir.path().join(name),
            ext_preview_archive::fixtures::sample_zip(),
        )
        .unwrap_or_else(|e| panic!("{e}"));
    }
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let size = egui::vec2(1400.0, 700.0);
    let mut harness = window(&fake, dir.path(), size, 600.0, content_previewers());

    // The archive tree: Name, Type, Modified, Size and Packed.
    select(&mut harness, "bundle.zip");
    let table = table_of(&harness);
    let content = harness
        .state()
        .preview_rects()
        .content
        .unwrap_or_else(|| panic!("content"));
    assert!((table.header.height() - 30.0).abs() < 0.1, "{table:?}");
    assert!((table.header.top() - content.top()).abs() < 0.1);
    assert_eq!(table.widths.len(), 5, "{table:?}");
    assert_eq!(
        table.dividers.len(),
        4,
        "one between every pair of header cells"
    );
    for title in ["Name", "Type", "Modified", "Size", "Packed"] {
        assert!(in_pane(&harness, title) >= 1, "the {title} title");
    }
    let first = table.first_row.unwrap_or_else(|| panic!("rows"));
    assert!(
        (first.top() - table.header.bottom()).abs() < 0.1,
        "the first row sits right under the header: {table:?}"
    );
    assert!(table.widths.iter().all(|w| *w >= 48.0), "{table:?}");
    let total: f32 = table.widths.iter().sum();
    assert!(
        (total - content.width()).abs() < 1.0,
        "the last column takes the rest: {table:?} in {content:?}"
    );

    // Dragging the second divider widens Type by the drag, and the last
    // column gives the room.
    let y = table.header.center().y;
    let divider = egui::pos2(table.dividers[1], y);
    drag(&mut harness, divider, divider + egui::vec2(40.0, 0.0));
    settle(&mut harness);
    let wider = table_of(&harness);
    // The first few points of a drag are egui's threshold.
    let grown = wider.widths[1] - table.widths[1];
    assert!(
        (33.0..=40.5).contains(&grown),
        "{:?} -> {:?}",
        table.widths,
        wider.widths
    );
    assert!((wider.widths[0] - table.widths[0]).abs() < 0.1);
    assert!((wider.widths[4] - (table.widths[4] - grown)).abs() < 0.1);
    let text = wait_settings(&mut harness, "preview-archive = [");
    assert!(text.contains("[preview.columns]"), "{text}");

    // No column gets narrower than 48 px.
    let divider = egui::pos2(wider.dividers[1], y);
    drag(&mut harness, divider, divider - egui::vec2(400.0, 0.0));
    settle(&mut harness);
    let narrow = table_of(&harness);
    assert!((narrow.widths[1] - 48.0).abs() < 0.1, "{narrow:?}");

    // A double-click fits the column to the cells in view: Type holds
    // "Folder" and extensions, which need more than 48 px with the padding.
    // Both clicks go into one frame's input: the harness's clock moves a
    // quarter second per frame, more than the double-click window.
    let divider = egui::pos2(narrow.dividers[1], y);
    harness.event(egui::Event::PointerMoved(divider));
    harness.step();
    for _ in 0..2 {
        for pressed in [true, false] {
            harness.input_mut().events.push(egui::Event::PointerButton {
                pos: divider,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::default(),
            });
        }
    }
    harness.step();
    settle(&mut harness);
    let fitted = table_of(&harness);
    assert!(
        fitted.widths[1] > 48.5 && fitted.widths[1] < table.widths[1] + 40.0,
        "{fitted:?}"
    );

    // Another archive opens with the widths the first was left with, and
    // so does a new window.
    select(&mut harness, "other.zip");
    let other = table_of(&harness);
    assert!(
        (other.widths[1] - fitted.widths[1]).abs() < 1.0,
        "{other:?} after {fitted:?}"
    );
    let remembered = fitted.widths[1];
    settle(&mut harness);
    let fitted_text = format!("{}", remembered.round());
    wait_settings(&mut harness, &fitted_text);
    drop(harness);
    let mut harness = window(&fake, dir.path(), size, 600.0, content_previewers());
    select(&mut harness, "bundle.zip");
    assert!(
        (table_of(&harness).widths[1] - remembered).abs() < 1.0,
        "the widths survive a restart"
    );

    // The CSV grid is the same table, its widths kept by position under
    // its own previewer.
    select(&mut harness, "data.csv");
    let grid = table_of(&harness);
    assert!((grid.header.height() - 30.0).abs() < 0.1);
    assert_eq!(grid.widths.len(), 4);
    assert_eq!(grid.dividers.len(), 3);
    let y = grid.header.center().y;
    let divider = egui::pos2(grid.dividers[0], y);
    drag(&mut harness, divider, divider + egui::vec2(30.0, 0.0));
    settle(&mut harness);
    let grown = table_of(&harness).widths[0] - grid.widths[0];
    assert!((23.0..=30.5).contains(&grown), "{grown}");
    wait_settings(&mut harness, "preview-markup = [");
}

/// Selects `name` and steps until the pane shows its placeholder for it.
fn select_unclaimed(harness: &mut Harness<'_, RickUi>, name: &str) {
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
        if slot.is_some_and(|s| s.placeholder().is_some() || s.error().is_some()) {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "{name}: no placeholder: {slot:?}, shown by {:?}",
            slot.and_then(ui_egui::preview::Slot::previewer)
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    harness.step();
    harness.step();
}

/// Issue 139, item 5, *No previewer*: a file nobody claims shows the File
/// info card, centred — not a hex dump and not a bare line: its head says
/// *no previewer for .bin*, or *plugin available* when a disabled plugin's
/// manifest claims the type; its rows are where the file is, when it was
/// modified and its permissions; *Open as hex* shows this file with the
/// hex previewer, while that is loaded, until the selection moves on.
#[test]
#[allow(clippy::too_many_lines)]
fn a_file_no_previewer_claims_shows_the_centred_card_with_open_as_hex() {
    use ui_egui::inspector::Info;
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("random.bin"), [0u8, 1, 2, 3, 255, 254])
        .unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("notes.abc"), b"abc\0").unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut extensions = previewers();
    extensions.push(Arc::new(ext_preview_hex::HexPreview::new()));
    let mut harness = window(
        &fake,
        dir.path(),
        egui::vec2(1300.0, 700.0),
        520.0,
        extensions,
    );
    // A disabled plugin in the plugins directory whose manifest claims
    // `.abc`, as the binary would have reported it.
    harness.state_mut().app_mut().set_plugins(vec![app::Plugin {
        id: "preview-abc".to_owned(),
        name: "ABC preview".to_owned(),
        version: "0.2.0".to_owned(),
        points: ext::Points::PREVIEWER,
        previews: vec!["abc".to_owned()],
        status: app::PluginStatus::Disabled,
        path: dir.path().join("plugins").join("preview-abc"),
        description: String::new(),
        built_in: false,
        removed: false,
    }]);
    let pane = harness.state().app().model().focused;

    select_unclaimed(&mut harness, "random.bin");
    let slot = harness
        .state()
        .previews()
        .get(pane)
        .unwrap_or_else(|| panic!("slot"));
    assert_eq!(
        slot.placeholder().map(|p| p.plugin.clone()),
        Some(None),
        "no plugin claims .bin"
    );
    assert!(!slot.has_frame(), "no hex dump, no frame at all");
    assert!(slot.text_view().is_none());
    assert!(slot.error().is_none(), "not an error either");
    for _ in 0..3 {
        harness.step();
    }
    // The head, the rows and the buttons, all inside the pane.
    assert!(in_pane(&harness, "random.bin") >= 1, "the name");
    harness.get_by_label("Binary file  ·  6 B  ·  no previewer for .bin");
    assert!(harness.query_by_label("No plugin available").is_none());
    for label in ["Location", "Modified"] {
        assert_eq!(in_pane(&harness, label), 1, "the {label} row");
    }
    #[cfg(unix)]
    {
        assert_eq!(in_pane(&harness, "Permissions"), 1);
        assert!(
            harness.query_all_by_label_contains("-rw-").next().is_some(),
            "the mode string and owner:group"
        );
    }
    // The card stands in the middle of the content, 400 px wide.
    let content = harness
        .state()
        .preview_rects()
        .content
        .unwrap_or_else(|| panic!("content"));
    let hex = harness.get_by_label("Open as hex").rect();
    let with = harness.get_by_label("Open with…").rect();
    assert!(hex.left() < with.left(), "Open as hex comes first");
    assert!(
        (hex.left() - (content.center().x - 200.0 + 15.0)).abs() < 2.0,
        "left-aligned in a 400 px card centred in {content:?}: {hex:?}"
    );
    assert!(
        (hex.center().y - content.center().y).abs() < content.height() / 3.0,
        "{hex:?} in {content:?}"
    );
    // Nothing for the info button to open: the card is on show already.
    assert_eq!(harness.state().preview_info(), Info::Inert);
    assert_eq!(harness.state().preview_fields(), [], "no rows to list");

    // Open as hex: this file, in the pane, through the hex previewer.
    harness.get_by_label("Open as hex").click();
    let started = Instant::now();
    loop {
        harness.step();
        let slot = harness.state().previews().get(pane);
        if slot.is_some_and(|s| s.previewer() == Some(ext_preview_hex::ID) && s.has_doc()) {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "no hex view: {slot:?}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    settle(&mut harness);
    assert!(harness.query_by_label("Open as hex").is_none());

    // The other file: a disabled plugin claims its type.
    select_unclaimed(&mut harness, "notes.abc");
    let slot = harness
        .state()
        .previews()
        .get(pane)
        .unwrap_or_else(|| panic!("slot"));
    assert_eq!(
        slot.placeholder().map(|p| p.plugin.clone()),
        Some(Some("preview-abc".to_owned())),
        "the disabled plugin claims .abc"
    );
    for _ in 0..3 {
        harness.step();
    }
    assert!(
        harness
            .query_all_by_label_contains("plugin available")
            .next()
            .is_some()
    );
    // And back: the hex view was for that one look at the file.
    select_unclaimed(&mut harness, "random.bin");
    for _ in 0..3 {
        harness.step();
    }
    harness.get_by_label("Open as hex");
}

/// Nothing selected (SPEC.md *Nothing selected*): the glyph with *No file
/// selected* and the hint under it, a toolbar of the info button — inert —
/// close and the handle, and an empty footer.
#[test]
fn with_nothing_selected_the_pane_shows_the_empty_state() {
    use ui_egui::inspector::Info;
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("notes.txt"), "hello\n").unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut harness = window(
        &fake,
        dir.path(),
        egui::vec2(1200.0, 700.0),
        400.0,
        previewers(),
    );
    settle(&mut harness);
    let content = harness
        .state()
        .preview_rects()
        .content
        .unwrap_or_else(|| panic!("content"));
    let title = harness.get_by_label("No file selected").rect();
    let hint = harness
        .get_by_label("Select a file to preview it here")
        .rect();
    assert!((title.center().x - content.center().x).abs() < 1.0);
    assert!((hint.center().x - content.center().x).abs() < 1.0);
    assert!(title.bottom() <= hint.top(), "{title:?} over {hint:?}");
    assert!(content.contains(title.center()) && content.contains(hint.center()));
    assert_eq!(harness.state().preview_info(), Info::Inert);
    assert!(
        harness.state().preview_toolbar().is_empty(),
        "no file icon, no controls: {:?}",
        harness.state().preview_toolbar()
    );
    assert_eq!(harness.state().preview_status(), "", "an empty footer");

    // A file selected: its preview, and the empty state is gone.
    select(&mut harness, "notes.txt");
    assert!(harness.query_by_label("No file selected").is_none());
    assert_eq!(harness.state().preview_info(), Info::Unpinned);
}

/// The four built-in previewers plus the markup (Markdown, HTML, CSV) and
/// archive previewer plugins, linked in for the content layout tests.
fn content_previewers() -> Vec<Arc<dyn Extension>> {
    let mut all = previewers();
    all.push(Arc::new(ext_preview_markup::MarkupPreview::new()));
    all.push(Arc::new(ext_preview_archive::ArchivePreview::new()));
    all
}

/// The labels inside the pane with this exact text.
fn in_pane(harness: &Harness<'_, RickUi>, label: &str) -> usize {
    let pane_rect = harness
        .state()
        .inspector_rect()
        .unwrap_or_else(|| panic!("pane"));
    harness
        .query_all_by_label(label)
        .filter(|node| pane_rect.contains(node.rect().center()))
        .count()
}

const README: &str = "# Title\n\nA paragraph with *emphasis*, **strong** and a [link](https://example.org/).\n\n- one\n- two\n\n```rust\nfn main() {}\n```\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\n---\n\n## The end\n";

/// Issue 50: `README.md` reaches the pane as a document laid out by the
/// pane, with the *Rendered* toggle on, and the toggle shows the
/// highlighted source; a `.csv` is a grid with a header row; a zip is its
/// file list in the explorer's columns with the stats line, folding in
/// place.
#[test]
// One scenario over the three content kinds; splitting it would repeat the
// window.
#[allow(clippy::too_many_lines)]
fn markdown_csv_and_a_zip_are_laid_out_by_the_pane_itself() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("README.md"), README).unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("data.csv"), "name,age\nAda,36\nAlan,41\n")
        .unwrap_or_else(|e| panic!("{e}"));
    fs::write(
        dir.path().join("bundle.zip"),
        ext_preview_archive::fixtures::sample_zip(),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut harness = window(
        &fake,
        dir.path(),
        egui::vec2(1200.0, 760.0),
        420.0,
        content_previewers(),
    );
    let pane = harness.state().app().model().focused;

    // The document: laid out by the pane, not the textual fallback.
    select(&mut harness, "README.md");
    let slot = harness
        .state()
        .previews()
        .get(pane)
        .unwrap_or_else(|| panic!("slot"));
    assert!(
        matches!(slot.content(), Some(ext::Content::Document(_))),
        "a document: {slot:?}"
    );
    assert!(slot.text_view().is_none(), "not the source");
    assert!(
        matches!(slot.content_drawn(), ui_egui::content::Drawn::Document { blocks } if *blocks == 7),
        "{:?}",
        slot.content_drawn()
    );
    assert_eq!(
        toolbar_item(&harness, "Rendered"),
        Some(true),
        "on for a new file"
    );
    assert!(in_pane(&harness, "Title") >= 1, "the heading is laid out");
    assert!(in_pane(&harness, "link") >= 1, "the link is laid out");
    assert_eq!(in_pane(&harness, "# Title"), 0, "no raw source");

    // The toggle: the highlighted source instead.
    click_in_pane(&mut harness, icons::RENDERED);
    settle(&mut harness);
    let started = Instant::now();
    loop {
        harness.step();
        if harness
            .state()
            .previews()
            .get(pane)
            .is_some_and(|s| s.text_view().is_some())
        {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "no source view"
        );
    }
    assert_eq!(toolbar_item(&harness, "Rendered"), Some(false));
    let slot = harness
        .state()
        .previews()
        .get(pane)
        .unwrap_or_else(|| panic!("slot"));
    assert!(slot.content().is_none());
    assert!(
        slot.text_view()
            .is_some_and(|v| v.text().starts_with("# Title")),
        "the raw source"
    );
    // And back.
    click_in_pane(&mut harness, icons::RENDERED);
    settle(&mut harness);
    assert_eq!(toolbar_item(&harness, "Rendered"), Some(true));
    assert!(matches!(
        harness
            .state()
            .previews()
            .get(pane)
            .and_then(|s| s.content()),
        Some(ext::Content::Document(_))
    ));

    // The grid: a header row and the rows.
    select(&mut harness, "data.csv");
    let slot = harness
        .state()
        .previews()
        .get(pane)
        .unwrap_or_else(|| panic!("slot"));
    assert!(matches!(slot.content(), Some(ext::Content::Grid(_))));
    assert_eq!(
        *slot.content_drawn(),
        ui_egui::content::Drawn::Grid {
            header: vec!["name".to_owned(), "age".to_owned()],
            rows: 2,
        }
    );
    assert!(in_pane(&harness, "name") >= 1);
    assert!(in_pane(&harness, "Ada") >= 1);
    assert!(in_pane(&harness, "41") >= 1);

    // The listing: the explorer's columns, the stats line, folding.
    select(&mut harness, "bundle.zip");
    let slot = harness
        .state()
        .previews()
        .get(pane)
        .unwrap_or_else(|| panic!("slot"));
    assert!(matches!(slot.content(), Some(ext::Content::Listing(_))));
    let ui_egui::content::Drawn::Listing { rows, stats } = slot.content_drawn().clone() else {
        panic!("{:?}", slot.content_drawn());
    };
    assert_eq!(rows, 4, "docs, docs/readme.txt, a.txt, secret.bin");
    assert_ne!(stats, "");
    assert!(in_pane(&harness, "readme.txt") >= 1);
    // The archive tree's columns; in a pane this narrow the last ones are
    // reached by scrolling sideways.
    assert!(in_pane(&harness, "Type") >= 1, "the Type title");
    assert!(
        harness.query_all_by_label("Packed").next().is_some(),
        "the Packed title"
    );
    assert!(in_pane(&harness, &stats) >= 1, "the stats line");
    // Folding docs hides its contents.
    click_in_pane(&mut harness, icons::TREE_OPEN);
    settle(&mut harness);
    assert!(matches!(
        harness
            .state()
            .previews()
            .get(pane)
            .map(|s| s.content_drawn().clone()),
        Some(ui_egui::content::Drawn::Listing { rows: 3, .. })
    ));
    assert_eq!(in_pane(&harness, "readme.txt"), 0);
}

/// Issue 50, *Copy*: the button puts the whole file's text on the
/// clipboard, beyond the window the pane loaded; for Markdown the raw
/// source; a file list has no Copy.
#[test]
fn copy_puts_the_whole_text_on_the_clipboard_and_the_raw_source_for_markdown() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let code = "fn main() {\n    println!(\"hi\");\n}\n";
    fs::write(dir.path().join("main.rs"), code).unwrap_or_else(|e| panic!("{e}"));
    // Past the 1 MiB window the text previewer loads.
    let line = "the quick brown fox jumps over the lazy dog, again and again\n";
    let mut big = String::new();
    while big.len() < 1_200_000 {
        big.push_str(line);
    }
    fs::write(dir.path().join("big.log"), &big).unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("README.md"), README).unwrap_or_else(|e| panic!("{e}"));
    fs::write(
        dir.path().join("bundle.zip"),
        ext_preview_archive::fixtures::sample_zip(),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut harness = window(
        &fake,
        dir.path(),
        egui::vec2(1200.0, 760.0),
        420.0,
        content_previewers(),
    );
    let wait_clipboard = |harness: &mut Harness<'_, RickUi>, fake: &Fake| -> String {
        let started = Instant::now();
        loop {
            settle(harness);
            if let Some(text) = fake.clipboard_text() {
                return text;
            }
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "nothing copied"
            );
        }
    };

    // A code file: its full text.
    select(&mut harness, "main.rs");
    let pane = harness.state().app().model().focused;
    click_in_pane(&mut harness, icons::COPY);
    assert_eq!(wait_clipboard(&mut harness, &fake), code);

    // Past the loaded window: the whole file all the same.
    fake.set_text("").unwrap_or_else(|e| panic!("{e}"));
    select(&mut harness, "big.log");
    assert!(
        harness
            .state()
            .previews()
            .get(pane)
            .and_then(|s| s.text_layer())
            .is_some_and(|l| l.pages[0].len() < big.len()),
        "the pane loaded a window, not the file"
    );
    click_in_pane(&mut harness, icons::COPY);
    let started = Instant::now();
    loop {
        settle(&mut harness);
        if fake.clipboard_text().is_some_and(|t| t.len() == big.len()) {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the whole file: {:?}",
            fake.clipboard_text().map(|t| t.len())
        );
    }
    assert_eq!(fake.clipboard_text().as_deref(), Some(big.as_str()));

    // Markdown: the raw source, not the rendered document.
    fake.set_text("").unwrap_or_else(|e| panic!("{e}"));
    select(&mut harness, "README.md");
    click_in_pane(&mut harness, icons::COPY);
    let started = Instant::now();
    loop {
        settle(&mut harness);
        if fake.clipboard_text().as_deref() == Some(README) {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the raw source: {:?}",
            fake.clipboard_text()
        );
    }

    select(&mut harness, "bundle.zip");
    assert!(
        !harness
            .state()
            .preview_toolbar()
            .iter()
            .any(|item| item.label == "Copy"),
        "a file list has no Copy: {:?}",
        harness.state().preview_toolbar()
    );
}

/// Issue 50, *Third-party libraries*: with pdfium out of reach a PDF shows
/// the in-pane message with *Browse…* and *Retry*; browsing to the library
/// stores its path in `preview-pdf.toml` and the PDF renders; setting the
/// path by hand and pressing *Retry* does the same; and a previewer that
/// finds the library by itself writes the hit back into the setting.
#[test]
// One scenario start to finish; splitting it would repeat the windows.
#[allow(clippy::too_many_lines)]
fn a_missing_pdfium_shows_the_message_and_browsing_to_it_stores_the_setting() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("paper.pdf"), media::pdf(2, 300, 400))
        .unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    // The previewer looks in its setting alone: a machine without pdfium.
    let mut harness = window(
        &fake,
        dir.path(),
        egui::vec2(1100.0, 700.0),
        360.0,
        vec![Arc::new(ext_preview_pdf::PdfPreview::with_search(
            ext_preview_pdf::Search::SettingOnly,
        ))],
    );
    let pane = harness.state().app().model().focused;
    select(&mut harness, "paper.pdf");
    let slot = harness
        .state()
        .previews()
        .get(pane)
        .unwrap_or_else(|| panic!("slot"));
    let Some(ext::Content::MissingLibrary(missing)) = slot.content() else {
        panic!("the message, not a preview: {slot:?}");
    };
    assert_eq!(missing.name, "pdfium");
    assert_eq!(
        missing.setting,
        (ext_preview_pdf::ID, ext_preview_pdf::LIBRARY_SETTING)
    );
    assert!(slot.pages().is_none(), "nothing rendered");
    // The File info card, centred: what is missing, where a download
    // goes, and the three buttons.
    assert_eq!(in_pane(&harness, "Download pdfium"), 1);
    assert!(in_pane(&harness, "Browse…") >= 1, "the Browse… button");
    assert!(in_pane(&harness, "Retry") >= 1, "the Retry button");
    assert!(
        in_pane(&harness, "paper.pdf") >= 1,
        "the file's name in the card's head"
    );
    assert_eq!(in_pane(&harness, "Needs pdfium, which was not found"), 1);
    assert_eq!(in_pane(&harness, "Destination"), 1);
    assert_eq!(in_pane(&harness, "Open folder"), 1);
    // No path is on show until *Where Rick looked* is opened.
    assert!(
        harness
            .query_all_by_label_contains("Looked in")
            .next()
            .is_none(),
        "the search places stay folded"
    );
    click_in_pane(&mut harness, "Where Rick looked");
    for _ in 0..4 {
        harness.step();
    }
    assert!(
        harness
            .query_all_by_label_contains("Looked in")
            .next()
            .is_some(),
        "unfolded: the places and the loader's error"
    );
    assert!(
        harness
            .state()
            .preview_status()
            .contains("pdfium not found"),
        "{}",
        harness.state().preview_status()
    );
    let settings_file = fake.config_dir().join("preview-pdf.toml");
    assert!(!settings_file.exists(), "nothing written for a miss");

    // The library, where this machine has one.
    let Some(library) = ext_preview_pdf::Library::default()
        .candidates()
        .into_iter()
        .find(|path| path.is_file())
    else {
        eprintln!("no pdfium library on this machine; Browse… and the write-back are not checked");
        assert!(
            std::env::var_os("RICK_REQUIRE_PDFIUM").is_none(),
            "pdfium required but not found"
        );
        return;
    };

    // Browse… to it: the choice goes into the setting and the PDF renders.
    fake.answer_files(vec![library.clone()]);
    click_in_pane(&mut harness, "Browse…");
    let rendered = |harness: &Harness<'_, RickUi>, pane| {
        harness
            .state()
            .previews()
            .get(pane)
            .is_some_and(|s| s.pages() == Some(2) && s.frame_size(0).is_some())
    };
    let started = Instant::now();
    while !rendered(&harness, pane) {
        settle(&mut harness);
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "the PDF did not render after Browse…: {:?}",
            harness.state().previews().get(pane)
        );
    }
    let text = fs::read_to_string(&settings_file).unwrap_or_else(|e| panic!("{e}"));
    // TOML quotes a Windows path as a literal string, so the file is read
    // as TOML rather than matched as text.
    let table: toml::Table = toml::from_str(&text).unwrap_or_else(|e| panic!("{e}: {text}"));
    assert_eq!(
        table.get("library").and_then(toml::Value::as_str),
        Some(library.to_string_lossy().as_ref()),
        "{text}"
    );
    assert_eq!(
        toolbar_item(&harness, "Next page"),
        None,
        "the pager is there"
    );

    // Retry after the setting was put right by hand: a second window
    // without the library, the setting set as the Settings screen would,
    // then Retry.
    drop(harness);
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut harness = window(
        &fake,
        dir.path(),
        egui::vec2(1100.0, 700.0),
        360.0,
        vec![Arc::new(ext_preview_pdf::PdfPreview::with_search(
            ext_preview_pdf::Search::SettingOnly,
        ))],
    );
    let pane = harness.state().app().model().focused;
    select(&mut harness, "paper.pdf");
    assert!(matches!(
        harness
            .state()
            .previews()
            .get(pane)
            .and_then(|s| s.content()),
        Some(ext::Content::MissingLibrary(_))
    ));
    harness.state_mut().app_mut().handle(
        app::Command::ExtSetting {
            ext: ext_preview_pdf::ID.0.to_owned(),
            key: ext_preview_pdf::LIBRARY_SETTING.to_owned(),
            value: ext::SettingValue::Text(library.display().to_string()),
        },
        &mut Vec::new(),
    );
    click_in_pane(&mut harness, "Retry");
    let started = Instant::now();
    while !rendered(&harness, pane) {
        settle(&mut harness);
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "the PDF did not render after Retry"
        );
    }

    // A previewer that looks everywhere finds the library by itself and
    // writes where into the setting.
    drop(harness);
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut harness = window(
        &fake,
        dir.path(),
        egui::vec2(1100.0, 700.0),
        360.0,
        vec![Arc::new(ext_preview_pdf::PdfPreview::new())],
    );
    let pane = harness.state().app().model().focused;
    select(&mut harness, "paper.pdf");
    let started = Instant::now();
    while !rendered(&harness, pane) {
        settle(&mut harness);
        assert!(started.elapsed() < Duration::from_secs(20), "no render");
    }
    let text = fs::read_to_string(fake.config_dir().join("preview-pdf.toml"))
        .unwrap_or_else(|e| panic!("the setting was not written back: {e}"));
    // Read as TOML: a Windows path is written as a literal string.
    let table: toml::Table = toml::from_str(&text).unwrap_or_else(|e| panic!("{e}: {text}"));
    let written = table
        .get("library")
        .and_then(toml::Value::as_str)
        .map_or_else(|| panic!("{text}"), Path::new);
    assert!(written.is_file(), "{}", written.display());
}

/// Issue 83, *Vector images*: an SVG fills the pane in Fit, rasterised at
/// the pane's size rather than its own 64 px, is rasterised again when the
/// pane gets wider, carries the *Rendered* toggle to its highlighted
/// source, and *Copy* puts the raw SVG on the clipboard; an SVGZ copies
/// the decompressed XML.
#[test]
// One scenario start to finish; splitting it would repeat the window.
#[allow(clippy::too_many_lines)]
fn an_svg_is_rasterised_at_the_pane_size_and_again_after_a_resize_and_copy_gives_the_source() {
    use std::io::Write as _;

    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"64\" height=\"64\">\n  <circle cx=\"32\" cy=\"32\" r=\"24\" fill=\"#ff0000\"/>\n</svg>\n";
    fs::write(dir.path().join("disc.svg"), svg).unwrap_or_else(|e| panic!("{e}"));
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(svg.as_bytes())
        .unwrap_or_else(|e| panic!("{e}"));
    fs::write(
        dir.path().join("disc.svgz"),
        gz.finish().unwrap_or_else(|e| panic!("{e}")),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut harness = window(
        &fake,
        dir.path(),
        egui::vec2(1100.0, 700.0),
        340.0,
        previewers(),
    );
    let pane = harness.state().app().model().focused;
    let frame = |harness: &Harness<'_, RickUi>| {
        harness
            .state()
            .previews()
            .get(pane)
            .and_then(|s| s.frame_size(0))
    };

    select(&mut harness, "disc.svg");
    let first = frame(&harness).unwrap_or_else(|| panic!("no frame"));
    assert_eq!(first.0, first.1, "the picture's own aspect: {first:?}");
    assert!(
        first.0 > 64 && first.0 <= 384,
        "rasterised at the pane's quantised size, not its 64 px: {first:?}"
    );
    assert!(
        harness
            .state()
            .preview_status()
            .starts_with("SVG · 64 × 64 px · "),
        "{}",
        harness.state().preview_status()
    );
    assert_eq!(toolbar_item(&harness, "Rendered"), Some(true));
    assert_eq!(toolbar_item(&harness, "Copy"), None, "Copy is there");

    // A bigger window, and so a bigger pane: a new rasterisation at the
    // new size.
    harness.set_size(egui::vec2(1600.0, 1000.0));
    let started = Instant::now();
    loop {
        settle(&mut harness);
        if frame(&harness).is_some_and(|size| size != first) {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "not rendered again after the resize: {:?}",
            frame(&harness)
        );
    }
    let second = frame(&harness).unwrap_or_else(|| panic!("no frame"));
    assert!(
        second.0 > first.0 && second.0 == second.1,
        "{first:?} then {second:?}"
    );

    // Copy: the raw source.
    click_in_pane(&mut harness, icons::COPY);
    let started = Instant::now();
    loop {
        settle(&mut harness);
        if fake.clipboard_text().as_deref() == Some(svg) {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the raw SVG: {:?}",
            fake.clipboard_text()
        );
    }

    // Rendered off: the highlighted source laid out as text.
    click_in_pane(&mut harness, icons::for_icon(ext::Icon::Rendered));
    let started = Instant::now();
    loop {
        settle(&mut harness);
        if harness
            .state()
            .previews()
            .get(pane)
            .is_some_and(|s| s.text_view().is_some())
        {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "no source view: {:?}",
            harness.state().previews().get(pane)
        );
    }
    assert_eq!(toolbar_item(&harness, "Rendered"), Some(false));

    // An SVGZ: Copy gives the decompressed XML.
    fake.set_text("").unwrap_or_else(|e| panic!("{e}"));
    select(&mut harness, "disc.svgz");
    assert!(
        harness
            .state()
            .preview_status()
            .starts_with("SVGZ · 64 × 64 px · "),
        "{}",
        harness.state().preview_status()
    );
    click_in_pane(&mut harness, icons::COPY);
    let started = Instant::now();
    loop {
        settle(&mut harness);
        if fake.clipboard_text().as_deref() == Some(svg) {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the decompressed XML: {:?}",
            fake.clipboard_text()
        );
    }
}
