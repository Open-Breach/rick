//! Acceptance (issue 219, item 3; SPEC.md *Copy buttons* under Look and
//! feel): every copy button answers a click in the same way. A click on
//! the breadcrumb's copy button, on a path row of the properties dialogue
//! and on a hash row puts the value on the clipboard and shows the tick —
//! `CHECK` in `status/success` where `COPY` was, in the same box — with
//! *Copied* as the button's name and tooltip, for about 1.5 s; then the
//! button is as it was. A copy button with a label, *Copy all* of the
//! hash dialogue, keeps the label and swaps the glyph.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use ext::ActionId;
use model::SelectionChange;
use platform::{ConfigDir, Fake};
use tests::local;
use ui_egui::keys::Intent;
use ui_egui::{EguiHost, RickUi, icons};

/// The time between two frames of the harness, in seconds.
const STEP: f32 = 0.05;

fn harness(start: &Path) -> (Harness<'static, RickUi>, Arc<Fake>) {
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
        .with_size(egui::vec2(1400.0, 1000.0))
        .with_step_dt(STEP)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    (harness, fake)
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

/// Selects the row named `name` in the focused pane.
fn select(harness: &mut Harness<'_, RickUi>, name: &str) -> model::PaneId {
    let pane = harness.state().app().model().focused;
    let id = harness
        .state()
        .app()
        .model()
        .active_tab(pane)
        .and_then(|(_, tab)| tab.rows().find(|e| e.name == name).map(|e| e.id))
        .unwrap_or_else(|| panic!("{name} is listed"));
    harness.state_mut().app_mut().handle(
        app::Command::Select {
            pane,
            change: SelectionChange::Set(vec![id]),
        },
        &mut Vec::new(),
    );
    harness.step();
    pane
}

/// The copy and tick glyphs the last frame painted inside `rect`, in the
/// order they were painted, each with its colour.
fn glyphs_in(harness: &Harness<'_, RickUi>, rect: egui::Rect) -> Vec<(String, egui::Color32)> {
    harness
        .output()
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) => {
                let glyph = text.galley.job.text.as_str();
                let at = text.galley.rect.translate(text.pos.to_vec2()).center();
                ((glyph == icons::COPY || glyph == icons::DONE) && rect.contains(at)).then(|| {
                    let colour = text
                        .galley
                        .job
                        .sections
                        .first()
                        .map_or(text.fallback_color, |s| s.format.color);
                    (glyph.to_owned(), colour)
                })
            }
            _ => None,
        })
        .collect()
}

/// The rectangle of the one button named `name`.
fn button(harness: &Harness<'_, RickUi>, name: &str) -> egui::Rect {
    let mut found = harness.query_all_by_label(name).map(|n| n.rect());
    let rect = found.next().unwrap_or_else(|| panic!("no {name}"));
    assert!(found.next().is_none(), "one {name}");
    rect
}

/// Whether a node named *Copied* is at `rect`: the button while it shows
/// its tick.
fn copied_at(harness: &Harness<'_, RickUi>, rect: egui::Rect) -> bool {
    harness
        .query_all_by_label("Copied")
        .any(|n| (n.rect().center() - rect.center()).length() < 0.5)
}

/// Clicks with the pointer at `pos`, which stays there.
fn click_at(harness: &mut Harness<'_, RickUi>, pos: egui::Pos2) {
    harness.event(egui::Event::PointerMoved(pos));
    harness.step();
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        });
        harness.step();
    }
}

/// Clicks the copy button named `name` and holds it to the rule: the
/// `COPY` glyph at rest; then, for about 1.5 s, the tick alone in
/// `status/success` in the same box, the button named *Copied* and a
/// tooltip that says so; then the button as it was.
fn answers_the_click(harness: &mut Harness<'_, RickUi>, name: &str) {
    let scheme = ui_egui::theme::scheme_of(&harness.ctx);
    let rect = button(harness, name);
    let at_rest = glyphs_in(harness, rect);
    assert_eq!(at_rest.len(), 1, "{name} at rest: {at_rest:?}");
    assert_eq!(at_rest[0].0, icons::COPY, "{name} at rest");
    let resting_colour = at_rest[0].1;

    // The click takes three frames; four more and the fade is over.
    click_at(harness, rect.center());
    for _ in 0..4 {
        harness.step();
    }
    let shown = glyphs_in(harness, rect);
    assert_eq!(shown.len(), 1, "{name} clicked: {shown:?}");
    assert_eq!(shown[0].0, icons::DONE, "the tick took the glyph's place");
    assert_eq!(shown[0].1, scheme.status_success, "in status/success");
    assert!(copied_at(harness, rect), "{name} is named Copied");
    assert!(
        harness.query_all_by_label(name).next().is_none(),
        "and not {name} meanwhile"
    );
    assert!(
        harness
            .query_all_by_label("Copied")
            .any(|n| !rect.contains(n.rect().center())),
        "the tooltip of {name} reads Copied"
    );

    // One second after the click the tick still shows.
    for _ in 0..14 {
        harness.step();
    }
    assert!(copied_at(harness, rect), "{name} after a second");
    assert_eq!(glyphs_in(harness, rect)[0].0, icons::DONE);

    // And 1.7 s after it the button is as it was, where it was.
    for _ in 0..14 {
        harness.step();
    }
    assert_eq!(button(harness, name), rect, "{name} did not move");
    let back = glyphs_in(harness, rect);
    assert_eq!(back.len(), 1, "{name} back: {back:?}");
    assert_eq!(back[0].0, icons::COPY);
    assert_eq!(back[0].1, resting_colour);
    assert!(!copied_at(harness, rect), "{name} is itself again");
}

#[test]
fn the_breadcrumb_s_copy_button_answers_a_click() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let folder = dir.path().join("reports");
    fs::create_dir(&folder).unwrap_or_else(|e| panic!("{e}"));
    let (mut harness, fake) = harness(&folder);
    answers_the_click(&mut harness, "Copy path");
    assert_eq!(
        fake.clipboard_text(),
        Some(folder.display().to_string()),
        "the folder's path"
    );
}

#[test]
fn a_path_row_of_the_properties_dialogue_answers_a_click() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("notes.txt"), b"hello").unwrap_or_else(|e| panic!("{e}"));
    let (mut harness, fake) = harness(dir.path());
    let pane = select(&mut harness, "notes.txt");
    harness
        .state_mut()
        .app_mut()
        .handle(app::Command::ShowProperties(pane), &mut Vec::new());
    let deadline = Instant::now() + Duration::from_secs(10);
    while harness.state().properties().info().is_none() {
        harness.step();
        assert!(Instant::now() < deadline, "the dialogue did not open");
        std::thread::sleep(Duration::from_millis(10));
    }
    // A modal takes a frame to find its size and another its place.
    for _ in 0..3 {
        harness.step();
    }
    answers_the_click(&mut harness, "Copy location");
    assert_eq!(
        fake.clipboard_text(),
        Some(dir.path().display().to_string()),
        "the folder that holds the file"
    );
}

#[test]
fn a_hash_row_answers_a_click_and_copy_all_keeps_its_label() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("notes.txt"), b"hello").unwrap_or_else(|e| panic!("{e}"));
    let (mut harness, fake) = harness(dir.path());
    let pane = select(&mut harness, "notes.txt");
    harness
        .state_mut()
        .queue(Intent::Action(ActionId("core.hash")), pane);
    harness.step();
    harness.step();
    assert!(harness.state().hash_dialog().is_open());
    settle(&mut harness);

    // A row: the SHA-256 of "hello".
    answers_the_click(&mut harness, "Copy SHA-256");
    assert_eq!(
        fake.clipboard_text().as_deref(),
        Some("2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824")
    );

    // *Copy all* has a label: it keeps it, and its glyph is the tick for
    // as long as a row's.
    let scheme = ui_egui::theme::scheme_of(&harness.ctx);
    let all = button(&harness, "Copy all");
    assert_eq!(glyphs_in(&harness, all)[0].0, icons::COPY);
    click_at(&mut harness, all.center());
    for _ in 0..4 {
        harness.step();
    }
    assert_eq!(
        button(&harness, "Copy all"),
        all,
        "the label and the box stay"
    );
    let shown = glyphs_in(&harness, all);
    assert_eq!(shown.len(), 1, "{shown:?}");
    assert_eq!(shown[0].0, icons::DONE);
    assert_eq!(shown[0].1, scheme.status_success);
    assert!(
        fake.clipboard_text()
            .is_some_and(|text| text.lines().count() > 1),
        "every digest, one per line"
    );
    for _ in 0..28 {
        harness.step();
    }
    assert_eq!(glyphs_in(&harness, all)[0].0, icons::COPY);
}
