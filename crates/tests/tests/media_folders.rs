//! Acceptance (issue 221, item 3; SPEC.md *Icons view for a folder of
//! media* and *View mode* under Functions): in the window. A folder of
//! eight photos, a video and a text file opens in Icons and its parent is
//! back in the view it had. A size chosen by hand is the size *Icons*
//! opens in from then on — from the menu's entry and from the shortcut —
//! and, after quitting and starting again, the size the folder of media
//! opens in by itself.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use model::{IconSize, ViewMode};
use platform::{ConfigDir, Fake};
use tests::local;
use ui_egui::keys::Intent;
use ui_egui::{EguiHost, RickUi};

/// A window on `start`, on the configuration `fake` holds.
fn window(fake: &Arc<Fake>, start: &Path) -> Harness<'static, RickUi> {
    let host = Arc::new(EguiHost::new());
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: Vec::new(),
        config_dir: Some(fake.config_dir()),
        start: Some(local(start)),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 800.0))
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

/// The view of the focused pane's tab.
fn view(harness: &Harness<'_, RickUi>) -> ViewMode {
    let model = harness.state().app().model();
    model
        .active_tab(model.focused)
        .map_or_else(|| panic!("no tab"), |(_, tab)| tab.view)
}

/// Takes the focused pane to `to`.
fn go(harness: &mut Harness<'_, RickUi>, to: &Path) {
    let pane = harness.state().app().model().focused;
    harness.state_mut().app_mut().handle(
        app::Command::Navigate {
            pane,
            to: local(to),
        },
        &mut Vec::new(),
    );
    settle(harness);
}

/// Sends the focused pane a shortcut's intent.
fn press(harness: &mut Harness<'_, RickUi>, intent: Intent) {
    let pane = harness.state().app().model().focused;
    harness.state_mut().queue(intent, pane);
    harness.step();
    settle(harness);
}

/// A folder with `photos` — eight photos, a video and a text file — and
/// `papers`, which holds text files only.
fn tree() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let photos = dir.path().join("photos");
    let papers = dir.path().join("papers");
    for folder in [&photos, &papers] {
        fs::create_dir(folder).unwrap_or_else(|e| panic!("{e}"));
    }
    for i in 0..8 {
        fs::write(photos.join(format!("photo-{i}.jpg")), "x").unwrap_or_else(|e| panic!("{e}"));
    }
    for name in ["clip.mp4", "notes.txt"] {
        fs::write(photos.join(name), "x").unwrap_or_else(|e| panic!("{e}"));
    }
    for name in ["a.txt", "b.txt"] {
        fs::write(papers.join(name), "x").unwrap_or_else(|e| panic!("{e}"));
    }
    dir
}

#[test]
fn a_folder_of_media_opens_in_icons_and_icons_opens_in_the_size_chosen_last() {
    let dir = tree();
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let photos = dir.path().join("photos");
    let papers = dir.path().join("papers");
    let mut harness = window(&fake, dir.path());
    assert_eq!(view(&harness), ViewMode::Details);

    // The media folder shows itself in Icons; its parent is as it was.
    go(&mut harness, &photos);
    assert_eq!(view(&harness), ViewMode::Icons(IconSize::Medium));
    go(&mut harness, dir.path());
    assert_eq!(view(&harness), ViewMode::Details);

    // Large, chosen by hand in a folder of papers: one size up from the
    // medium that Icons opens in at first.
    go(&mut harness, &papers);
    press(&mut harness, Intent::ViewIcons);
    assert_eq!(view(&harness), ViewMode::Icons(IconSize::Medium));
    press(&mut harness, Intent::IconSizeUp);
    assert_eq!(view(&harness), ViewMode::Icons(IconSize::Large));
    assert_eq!(
        harness.state().app().model().settings.icon_size,
        IconSize::Large
    );

    // From then on Icons opens in Large: from the shortcut...
    press(&mut harness, Intent::ViewDetails);
    press(&mut harness, Intent::ViewIcons);
    assert_eq!(view(&harness), ViewMode::Icons(IconSize::Large));
    // ...and from the entry of the View options menu.
    press(&mut harness, Intent::ViewDetails);
    harness.get_by_label("View options").click();
    harness.step();
    harness.step();
    harness.get_by_label("Icons").click();
    harness.step();
    settle(&mut harness);
    assert_eq!(view(&harness), ViewMode::Icons(IconSize::Large));
    press(&mut harness, Intent::ViewDetails);

    // Quit, once the size is in the settings file, and start again.
    let file = fake.config_dir().join("rick.toml");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !fs::read_to_string(&file).is_ok_and(|text| text.contains("icon_size = \"large\"")) {
        assert!(Instant::now() < deadline, "the size is not in {file:?}");
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(harness);
    let mut again = window(&fake, &papers);
    assert_eq!(view(&again), ViewMode::Details);
    press(&mut again, Intent::ViewIcons);
    assert_eq!(
        view(&again),
        ViewMode::Icons(IconSize::Large),
        "Icons opens in the size chosen last"
    );
    press(&mut again, Intent::ViewDetails);
    go(&mut again, &photos);
    assert_eq!(
        view(&again),
        ViewMode::Icons(IconSize::Large),
        "and so does the folder of media"
    );
}
