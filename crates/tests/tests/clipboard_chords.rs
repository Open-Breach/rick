//! Acceptance (issue 176): copy, cut and paste by keyboard in the file
//! list, as a window really delivers the chords, and *Paste* in the context
//! menu.
//!
//! A window does not deliver Mod+C, Mod+X and Mod+V as key presses: the
//! press becomes a copy, cut or paste event, the paste event only when the
//! clipboard holds text, and the key's release alone arrives as a key. The
//! tests here feed the list exactly that, not the key presses the shortcut
//! table's test injects. The chords through a real window are
//! `real_chords.rs`.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::accesskit::Role;
use egui::{Event, Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use ext::ActionId;
use model::SelectionChange;
use platform::{ConfigDir, Fake};
use tests::local;
use ui_egui::keys::Intent;
use ui_egui::{EguiHost, RickUi};

fn window(start: &Path) -> Harness<'static, RickUi> {
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
        .with_size(egui::vec2(1300.0, 800.0))
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

fn steps(harness: &mut Harness<'_, RickUi>) {
    for _ in 0..4 {
        harness.step();
    }
}

fn command(harness: &mut Harness<'_, RickUi>, command: app::Command) {
    harness
        .state_mut()
        .app_mut()
        .handle(command, &mut Vec::new());
    settle(harness);
}

fn go(harness: &mut Harness<'_, RickUi>, to: &Path) {
    let pane = harness.state().app().model().focused;
    command(
        harness,
        app::Command::Navigate {
            pane,
            to: local(to),
        },
    );
}

/// Selects the row named `name` in the focused pane, once it is listed: a
/// file that was just pasted is on disk before the list has heard of it.
fn select(harness: &mut Harness<'_, RickUi>, name: &str) {
    let pane = harness.state().app().model().focused;
    let deadline = Instant::now() + Duration::from_secs(20);
    let id = loop {
        let listed = harness
            .state()
            .app()
            .model()
            .active_tab(pane)
            .and_then(|(_, tab)| tab.listing.rows().find(|e| e.name == name).map(|e| e.id));
        if let Some(id) = listed {
            break id;
        }
        assert!(Instant::now() < deadline, "{name} is not listed");
        std::thread::sleep(Duration::from_millis(10));
        harness.step();
    };
    command(
        harness,
        app::Command::Select {
            pane,
            change: SelectionChange::Set(vec![id]),
        },
    );
}

/// How often `action` was dispatched since the last clearing.
fn dispatched(harness: &Harness<'_, RickUi>, action: &'static str) -> usize {
    harness
        .state()
        .dispatched()
        .iter()
        .filter(|d| d.intent == Intent::Action(ActionId(action)))
        .count()
}

/// Waits for `path` to exist, or not to.
fn wait_for(harness: &mut Harness<'_, RickUi>, path: &Path, there: bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while path.exists() != there {
        settle(harness);
        assert!(
            Instant::now() < deadline,
            "{} should {}exist",
            path.display(),
            if there { "" } else { "not " }
        );
    }
}

/// The paste chord with no text on the clipboard: the press leaves nothing,
/// the release of V arrives.
fn paste_without_text(harness: &mut Harness<'_, RickUi>, modifiers: Modifiers) {
    harness.key_up_modifiers(modifiers, Key::V);
    steps(harness);
}

#[test]
fn copy_cut_and_paste_reach_the_list_as_a_window_delivers_them() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let (src, dst) = (dir.path().join("src"), dir.path().join("dst"));
    std::fs::create_dir_all(&src).unwrap_or_else(|e| panic!("{e}"));
    std::fs::create_dir_all(&dst).unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(src.join("one.txt"), "one").unwrap_or_else(|e| panic!("{e}"));
    let mut harness = window(&src);

    // Mod+C arrives as a copy event, with no key press.
    select(&mut harness, "one.txt");
    harness.state_mut().clear_dispatched();
    harness.event_modifiers(Event::Copy, Modifiers::COMMAND);
    steps(&mut harness);
    assert_eq!(dispatched(&harness, "core.copy"), 1);

    // Mod+V with files and no text on the clipboard: no event on the
    // press, the key's release is all there is — and the copy arrives.
    go(&mut harness, &dst);
    harness.state_mut().clear_dispatched();
    paste_without_text(&mut harness, Modifiers::COMMAND);
    assert_eq!(dispatched(&harness, "core.paste"), 1);
    wait_for(&mut harness, &dst.join("one.txt"), true);
    assert!(src.join("one.txt").exists(), "a copy leaves the original");

    // Mod+X arrives as a cut event.
    select(&mut harness, "one.txt");
    harness.state_mut().clear_dispatched();
    harness.event_modifiers(Event::Cut, Modifiers::COMMAND);
    steps(&mut harness);
    assert_eq!(dispatched(&harness, "core.cut"), 1);

    // Mod+V with text on the clipboard, the key held: the paste event comes
    // and repeats, and it is still one paste, at the release.
    go(&mut harness, dir.path());
    harness.state_mut().clear_dispatched();
    for _ in 0..3 {
        harness.event_modifiers(Event::Paste("some text".to_owned()), Modifiers::COMMAND);
        steps(&mut harness);
    }
    assert_eq!(dispatched(&harness, "core.paste"), 0, "not on the event");
    harness.key_up_modifiers(Modifiers::COMMAND, Key::V);
    steps(&mut harness);
    assert_eq!(
        dispatched(&harness, "core.paste"),
        1,
        "once, at the release"
    );
    wait_for(&mut harness, &dir.path().join("one.txt"), true);
    wait_for(&mut harness, &dst.join("one.txt"), false);

    // Mod+Shift+V is swallowed as a paste too, and Mod+Shift+C as a copy:
    // the modifiers held tell which action the keymap means.
    let shifted = Modifiers::COMMAND | Modifiers::SHIFT;
    select(&mut harness, "one.txt");
    harness.state_mut().clear_dispatched();
    harness.event_modifiers(Event::Copy, shifted);
    steps(&mut harness);
    assert_eq!(dispatched(&harness, "core.copy"), 0);
    assert_eq!(dispatched(&harness, "core.copy_path_and_name"), 1);
    harness.state_mut().clear_dispatched();
    paste_without_text(&mut harness, shifted);
    assert_eq!(dispatched(&harness, "core.paste"), 0);
    assert_eq!(dispatched(&harness, "core.paste_link"), 1);
}

#[test]
fn a_text_field_keeps_the_chords_for_its_text() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(dir.path().join("abc.txt"), "a").unwrap_or_else(|e| panic!("{e}"));
    std::fs::create_dir(dir.path().join("sub")).unwrap_or_else(|e| panic!("{e}"));
    let mut harness = window(dir.path());
    select(&mut harness, "abc.txt");
    harness.event_modifiers(Event::Copy, Modifiers::COMMAND);
    steps(&mut harness);
    let filter = |harness: &Harness<'_, RickUi>| {
        let model = harness.state().app().model();
        model
            .active_tab(model.focused)
            .map_or_default(|(_, tab)| tab.filter.clone())
    };

    // The filter field has the keyboard.
    harness.key_press_modifiers(Modifiers::COMMAND, Key::F);
    steps(&mut harness);
    harness.event(Event::Text("ab".to_owned()));
    steps(&mut harness);
    assert_eq!(filter(&harness), "ab");

    // A paste there is the field's: the text goes in, no file is pasted.
    harness.state_mut().clear_dispatched();
    harness.event_modifiers(Event::Paste("c".to_owned()), Modifiers::COMMAND);
    steps(&mut harness);
    harness.key_up_modifiers(Modifiers::COMMAND, Key::V);
    steps(&mut harness);
    assert_eq!(filter(&harness), "abc");
    assert_eq!(dispatched(&harness, "core.paste"), 0);
    // And a copy or a cut there is no copy or cut of the selected file.
    harness.event_modifiers(Event::Copy, Modifiers::COMMAND);
    harness.event_modifiers(Event::Cut, Modifiers::COMMAND);
    steps(&mut harness);
    assert_eq!(dispatched(&harness, "core.copy"), 0);
    assert_eq!(dispatched(&harness, "core.cut"), 0);
    settle(&mut harness);
    assert!(!dir.path().join("abc (2).txt").exists());
}

/// Opens the context menu on the empty area of the focused pane.
fn menu_on_empty_space(harness: &mut Harness<'_, RickUi>) {
    let pane = harness.state().app().model().focused;
    let rect = harness
        .state()
        .pane_rects()
        .into_iter()
        .find(|(p, _)| *p == pane)
        .map_or_else(|| panic!("the pane's rect"), |(_, r)| r);
    let pos = egui::pos2(rect.center().x, rect.bottom() - 80.0);
    harness.event(Event::PointerMoved(pos));
    harness.step();
    for pressed in [true, false] {
        harness.event(Event::PointerButton {
            pos,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::default(),
        });
        harness.step();
    }
    steps(harness);
}

/// The menu's *Paste* entry: whether it is there, and greyed.
fn paste_entry(harness: &Harness<'_, RickUi>) -> Option<bool> {
    harness
        .query_by_role_and_label(Role::Button, "Paste")
        .map(|node| node.accesskit_node().is_disabled())
}

fn close_menu(harness: &mut Harness<'_, RickUi>) {
    harness.key_press(Key::Escape);
    steps(harness);
    assert_eq!(paste_entry(harness), None, "the menu is closed");
}

#[test]
fn paste_in_the_menu_is_greyed_until_something_is_copied_and_goes_into_the_folder() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let (sub, other) = (dir.path().join("sub"), dir.path().join("other"));
    std::fs::create_dir(&sub).unwrap_or_else(|e| panic!("{e}"));
    std::fs::create_dir(&other).unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(dir.path().join("one.txt"), "one").unwrap_or_else(|e| panic!("{e}"));
    let mut harness = window(dir.path());

    // Nothing copied yet: Paste is there on the empty area and on a folder,
    // greyed; Paste as link, like every other entry, is absent.
    menu_on_empty_space(&mut harness);
    assert_eq!(paste_entry(&harness), Some(true), "greyed on empty space");
    assert!(harness.query_by_label("Paste as link").is_none());
    harness.get_by_label("New folder");
    close_menu(&mut harness);
    harness.get_by_label("sub").click_secondary();
    steps(&mut harness);
    assert_eq!(paste_entry(&harness), Some(true), "greyed on a folder");
    // A click on the greyed entry pastes nothing.
    harness.get_by_role_and_label(Role::Button, "Paste").click();
    settle(&mut harness);
    assert!(harness.state().app().model().jobs.is_empty());
    assert_eq!(harness.state().toasts(), Vec::new(), "and says nothing");
    close_menu(&mut harness);

    // Copy the file: on the folder, Paste is live and pastes into it.
    select(&mut harness, "one.txt");
    harness.event_modifiers(Event::Copy, Modifiers::COMMAND);
    steps(&mut harness);
    harness.get_by_label("sub").click_secondary();
    steps(&mut harness);
    assert_eq!(paste_entry(&harness), Some(false));
    harness.get_by_role_and_label(Role::Button, "Paste").click();
    steps(&mut harness);
    wait_for(&mut harness, &sub.join("one.txt"), true);
    assert!(
        !dir.path().join("one (2).txt").exists(),
        "into the folder, not beside it"
    );
    assert_eq!(harness.state().app().model().conflicts.len(), 0);

    // On the empty area it pastes into the folder the pane shows.
    go(&mut harness, &other);
    menu_on_empty_space(&mut harness);
    assert_eq!(paste_entry(&harness), Some(false));
    harness.get_by_label("Paste as link");
    harness.get_by_role_and_label(Role::Button, "Paste").click();
    steps(&mut harness);
    wait_for(&mut harness, &other.join("one.txt"), true);

    // On a file the entry is there too, for the folder the file is in.
    go(&mut harness, dir.path());
    harness.get_by_label("one.txt").click_secondary();
    steps(&mut harness);
    assert_eq!(paste_entry(&harness), Some(false));
    close_menu(&mut harness);
}
