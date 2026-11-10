//! Acceptance (issue 139, item 3, SPEC.md *Dialogues* and *Execution*): the
//! paste-conflict dialogue. Pasting a file whose name is 80 characters
//! long over an existing one: the dialogue stays 520 px wide, both names
//! are cut inside their cards, Enter keeps both, the new file arrives as
//! `name (2).ext` and the existing file is untouched. With several
//! conflicts the title counts them and the checkbox applies the chosen
//! button to the rest. Two folders merge or are skipped, and have no
//! *Keep both*.

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
use ui_egui::{EguiHost, RickUi};

fn harness(start: &Path) -> Harness<'static, RickUi> {
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
        .with_size(egui::vec2(1400.0, 800.0))
        .with_step_dt(0.05)
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

/// Selects the rows named `names` in the focused pane.
fn select(harness: &mut Harness<'_, RickUi>, names: &[&str]) {
    let pane = harness.state().app().model().focused;
    let ids: Vec<types::EntryId> = harness
        .state()
        .app()
        .model()
        .active_tab(pane)
        .map_or_else(|| panic!("no tab"), |(_, tab)| tab.clone())
        .rows()
        .filter(|e| names.contains(&e.name.as_str()))
        .map(|e| e.id)
        .collect();
    assert_eq!(ids.len(), names.len(), "every name has its row");
    command(
        harness,
        app::Command::Select {
            pane,
            change: SelectionChange::Set(ids),
        },
    );
}

fn run(harness: &mut Harness<'_, RickUi>, action: &'static str) {
    let pane = harness.state().app().model().focused;
    harness
        .state_mut()
        .queue(Intent::Action(ActionId(action)), pane);
    harness.step();
    harness.step();
}

/// Copies `names` from `from` and pastes them into `into`, then waits for
/// the dialogue.
fn paste_over(harness: &mut Harness<'_, RickUi>, names: &[&str], into: &Path) {
    select(harness, names);
    run(harness, "core.copy");
    let pane = harness.state().app().model().focused;
    command(
        harness,
        app::Command::Navigate {
            pane,
            to: local(into),
        },
    );
    run(harness, "core.paste");
    wait_for_dialogue(harness);
}

fn wait_for_dialogue(harness: &mut Harness<'_, RickUi>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while harness.state().app().model().conflicts.is_empty() {
        assert!(Instant::now() < deadline, "no conflict dialogue");
        harness.step();
        std::thread::sleep(Duration::from_millis(5));
    }
    // The dialogue takes a frame to find its place.
    harness.step();
    harness.step();
}

/// The dialogue's outer width, from its footer: Cancel sits one padding
/// from the left edge and the last button one padding from the right.
fn dialogue_width(harness: &Harness<'_, RickUi>, last: &str) -> f32 {
    const PAD: f32 = 24.0;
    let left = harness.get_by_label("Cancel").rect().left();
    let right = harness.get_by_label(last).rect().right();
    right - left + 2.0 * PAD
}

#[test]
fn a_long_name_keeps_the_dialogue_at_520_px_and_enter_keeps_both() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let name = "Camera white appartment 5-14-2026 1.32.41pm GMT2 - 5-14-2026 1.33.08pm GMT2a.mp4";
    assert_eq!(name.chars().count(), 80);
    fs::create_dir(dir.path().join("sub")).unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join(name), "the new file").unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("sub").join(name), "existing").unwrap_or_else(|e| panic!("{e}"));
    let mut harness = harness(dir.path());
    paste_over(&mut harness, &[name], &dir.path().join("sub"));

    // The title is the question: no name in it, and one conflict is not
    // counted.
    harness.get_by_label("Replace file?");
    harness.get_by_label_contains("A file with this name is already in");
    harness.get_by_label("EXISTING FILE");
    harness.get_by_label("NEW FILE");
    assert!(harness.query_by_label_contains("Do the same").is_none());
    assert!(
        harness.query_by_label("Rename").is_none(),
        "no rename field"
    );
    for button in ["Cancel", "Skip", "Replace", "Keep both"] {
        harness.get_by_label(button);
    }

    // 520 px wide whatever the name, and both names cut inside the cards.
    let width = dialogue_width(&harness, "Keep both");
    assert!((width - 520.0).abs() < 1.0, "the dialogue is {width} px");
    let left = harness.get_by_label("Cancel").rect().left();
    let right = harness.get_by_label("Keep both").rect().right();
    let caption = harness.get_by_label("EXISTING FILE").rect();
    let footer = harness.get_by_label("Cancel").rect();
    let in_cards: Vec<egui::Rect> = harness
        .query_all_by_label(name)
        .map(|node| node.rect())
        .filter(|r| r.top() > caption.top() && r.bottom() < footer.top())
        .filter(|r| r.left() > left)
        .collect();
    assert_eq!(in_cards.len(), 2, "the name stands in both cards");
    for rect in in_cards {
        assert!(rect.right() <= right, "the name ends inside its card");
        assert!(
            rect.width() < 420.0,
            "an 80 character name is cut: {} px",
            rect.width()
        );
    }

    // Enter is Keep both: the new file arrives numbered, the existing one
    // is as it was.
    harness.key_press(egui::Key::Enter);
    harness.step();
    settle(&mut harness);
    assert_eq!(
        harness.state().app().model().conflicts,
        Vec::new(),
        "the question is answered"
    );
    let kept =
        "Camera white appartment 5-14-2026 1.32.41pm GMT2 - 5-14-2026 1.33.08pm GMT2a (2).mp4";
    assert_eq!(
        fs::read_to_string(dir.path().join("sub").join(kept)).ok(),
        Some("the new file".into())
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("sub").join(name)).ok(),
        Some("existing".into()),
        "the existing file is untouched"
    );
}

#[test]
fn several_conflicts_are_counted_and_the_checkbox_answers_for_the_rest() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir(dir.path().join("sub")).unwrap_or_else(|e| panic!("{e}"));
    for name in ["a.txt", "b.txt", "c.txt"] {
        fs::write(dir.path().join(name), "new").unwrap_or_else(|e| panic!("{e}"));
        fs::write(dir.path().join("sub").join(name), "old").unwrap_or_else(|e| panic!("{e}"));
    }
    // A fourth file has no conflict and is not counted.
    fs::write(dir.path().join("d.txt"), "new").unwrap_or_else(|e| panic!("{e}"));
    let mut harness = harness(dir.path());
    paste_over(
        &mut harness,
        &["a.txt", "b.txt", "c.txt", "d.txt"],
        &dir.path().join("sub"),
    );

    harness.get_by_label("Replace file? (1 of 3)");
    let width = dialogue_width(&harness, "Keep both");
    assert!((width - 520.0).abs() < 1.0, "the dialogue is {width} px");
    // One answer for this one only: the next conflict is asked about.
    harness.get_by_label("Skip").click();
    harness.step();
    wait_for_dialogue(&mut harness);
    harness.get_by_label("Replace file? (2 of 3)");
    harness
        .get_by_label("Do the same for the remaining conflict")
        .click();
    harness.step();
    harness.get_by_label("Keep both").click();
    harness.step();
    settle(&mut harness);
    assert!(
        harness.state().app().model().conflicts.is_empty(),
        "the third conflict was answered by the checkbox"
    );
    let read = |name: &str| fs::read_to_string(dir.path().join("sub").join(name)).ok();
    assert_eq!(read("a.txt"), Some("old".into()), "skipped");
    assert!(read("a (2).txt").is_none());
    for name in ["b.txt", "c.txt"] {
        assert_eq!(read(name), Some("old".into()), "{name} is kept");
    }
    assert_eq!(read("b (2).txt"), Some("new".into()));
    assert_eq!(read("c (2).txt"), Some("new".into()));
    assert_eq!(read("d.txt"), Some("new".into()), "no conflict, pasted");
}

#[test]
fn two_folders_merge_or_are_skipped_and_cannot_be_kept_twice() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir_all(dir.path().join("photos")).unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir_all(dir.path().join("sub/photos")).unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("photos/new.jpg"), "new").unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("sub/photos/old.jpg"), "old").unwrap_or_else(|e| panic!("{e}"));
    let mut harness = harness(dir.path());
    paste_over(&mut harness, &["photos"], &dir.path().join("sub"));

    harness.get_by_label("Merge folder?");
    harness.get_by_label_contains("A folder with this name is already in");
    harness.get_by_label("EXISTING FOLDER");
    harness.get_by_label("NEW FOLDER");
    harness.get_by_label("Merge");
    harness.get_by_label("Skip");
    assert!(harness.query_by_label("Keep both").is_none());
    assert!(harness.query_by_label("Replace").is_none());
    let width = dialogue_width(&harness, "Merge");
    assert!((width - 520.0).abs() < 1.0, "the dialogue is {width} px");

    // Enter is Skip, the choice that loses nothing here.
    harness.key_press(egui::Key::Enter);
    harness.step();
    settle(&mut harness);
    assert_eq!(
        harness.state().app().model().conflicts,
        Vec::new(),
        "the question is answered"
    );
    assert!(!dir.path().join("sub/photos/new.jpg").exists(), "skipped");
    assert!(!dir.path().join("sub/photos (2)").exists());

    // Merge brings the new file in beside the old one.
    run(&mut harness, "core.paste");
    wait_for_dialogue(&mut harness);
    harness.get_by_label("Merge").click();
    harness.step();
    settle(&mut harness);
    assert!(dir.path().join("sub/photos/new.jpg").exists(), "merged in");
    assert!(dir.path().join("sub/photos/old.jpg").exists(), "kept");
}
