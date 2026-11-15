//! Acceptance (issue 221, item 2; SPEC.md *Set as desktop background*
//! under Functions and *Toolbar* under Preview): one selected image file
//! of the local file system has the entry in its context menu and a
//! button in the preview pane's toolbar, and either points the desktop at
//! the file where it lies — one action in two places. A text file has
//! neither, and on a desktop Rick cannot set neither has the image. A
//! failure is told like any other action's.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use model::SelectionChange;
use platform::{ConfigDir, Fake};
use tests::local;
use ui_egui::keys::Intent;
use ui_egui::{EguiHost, RickUi, icons};

/// The action's name, in the menu and as the button's tooltip.
const LABEL: &str = "Set as desktop background";

/// A folder with a photo, `lake.jpg`, and a text file, `notes.txt`.
fn tree() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let photo = image::RgbImage::from_fn(64, 48, |x, y| {
        image::Rgb([
            u8::try_from(x * 4).unwrap_or(255),
            u8::try_from(y * 5).unwrap_or(255),
            160,
        ])
    });
    let mut bytes = std::io::Cursor::new(Vec::new());
    photo
        .write_to(&mut bytes, image::ImageFormat::Jpeg)
        .unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("lake.jpg"), bytes.into_inner()).unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("notes.txt"), "a note\n").unwrap_or_else(|e| panic!("{e}"));
    dir
}

/// A window on `start` with the image previewer and the Preview tab open.
fn window(fake: &Arc<Fake>, start: &Path) -> Harness<'static, RickUi> {
    let host = Arc::new(EguiHost::new());
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: vec![
            Arc::new(ext_preview_text::TextPreview::new()),
            Arc::new(ext_preview_image::ImagePreview::new()),
        ],
        config_dir: Some(fake.config_dir()),
        start: Some(local(start)),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 800.0))
        .with_pixels_per_point(1.0)
        .build_eframe(move |cc| {
            let mut ui = RickUi::new(app, host, &cc.egui_ctx);
            ui.set_inspector_width(420.0);
            ui
        });
    settle(&mut harness);
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

/// Selects `name` in the focused pane and steps until its preview shows.
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
            "{name}: no preview: {slot:?}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    harness.step();
    harness.step();
}

/// Opens the context menu of the row named `name`.
fn menu_of(harness: &mut Harness<'_, RickUi>, name: &str) {
    let pane_rect = harness.state().inspector_rect();
    let at = harness
        .get_all_by_label(name)
        .map(|node| node.rect())
        .find(|rect| pane_rect.is_none_or(|pane| !pane.contains(rect.center())))
        .unwrap_or_else(|| panic!("no row {name}"))
        .center();
    harness.event(egui::Event::PointerMoved(at));
    harness.step();
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::default(),
        });
        harness.step();
    }
    harness.step();
    // Every file's menu has Properties: the menu is open.
    harness.get_by_label("Properties");
}

/// Closes the open menu.
fn close_menu(harness: &mut Harness<'_, RickUi>) {
    harness.key_press(egui::Key::Escape);
    harness.step();
    harness.step();
    assert!(harness.query_by_label("Properties").is_none());
}

/// The labels of the preview pane's toolbar, left to right.
fn toolbar(harness: &Harness<'_, RickUi>) -> Vec<String> {
    harness
        .state()
        .preview_toolbar()
        .iter()
        .map(|item| item.label.clone())
        .collect()
}

#[test]
fn the_context_menu_of_an_image_points_the_desktop_at_it() {
    let dir = tree();
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut harness = window(&fake, dir.path());

    // A photo: the entry ends the Open group, over Cut.
    menu_of(&mut harness, "lake.jpg");
    let y =
        |harness: &Harness<'_, RickUi>, label: &str| harness.get_by_label(label).rect().center().y;
    let entry = y(&harness, LABEL);
    assert!(
        y(&harness, "Open") < entry && entry < y(&harness, "Cut"),
        "in the Open group: {entry}"
    );
    harness.get_by_label(LABEL).click();
    settle(&mut harness);
    assert_eq!(
        fake.desktop_backgrounds(),
        [dir.path().join("lake.jpg")],
        "the file where it lies, and nothing copied"
    );
    assert!(
        harness.query_by_label("Properties").is_none(),
        "the menu closed"
    );

    // A text file has no such entry.
    menu_of(&mut harness, "notes.txt");
    assert!(harness.query_by_label(LABEL).is_none());
    close_menu(&mut harness);

    // Nor has the photo on a desktop whose background Rick cannot set.
    fake.set_background_formats(&[]);
    menu_of(&mut harness, "lake.jpg");
    assert!(harness.query_by_label(LABEL).is_none());
    close_menu(&mut harness);
    assert_eq!(fake.desktop_backgrounds().len(), 1, "nothing more was set");
}

#[test]
fn the_preview_toolbar_has_the_button_wherever_the_menu_has_the_entry() {
    let dir = tree();
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut harness = window(&fake, dir.path());

    // A photo: the button stands after Fit and 1:1.
    select(&mut harness, "lake.jpg");
    let labels = toolbar(&harness);
    let at = |label: &str| {
        labels
            .iter()
            .position(|l| l == label)
            .unwrap_or_else(|| panic!("no {label} in {labels:?}"))
    };
    assert_eq!(at("1:1"), at("Fit") + 1, "{labels:?}");
    assert_eq!(at(LABEL), at("1:1") + 1, "{labels:?}");

    // It is the icon alone, and it runs the action on the file shown.
    let pane_rect = harness
        .state()
        .inspector_rect()
        .unwrap_or_else(|| panic!("pane"));
    assert!(
        !harness
            .query_all_by_label(LABEL)
            .any(|node| pane_rect.contains(node.rect().center())),
        "no label beside the icon"
    );
    harness
        .get_all_by_label(icons::MONITOR)
        .find(|node| pane_rect.contains(node.rect().center()))
        .unwrap_or_else(|| panic!("no button in the pane"))
        .click();
    settle(&mut harness);
    assert_eq!(fake.desktop_backgrounds(), [dir.path().join("lake.jpg")]);

    // A text file has no such button.
    select(&mut harness, "notes.txt");
    assert!(!toolbar(&harness).iter().any(|l| l == LABEL));

    // Nor has the photo on a desktop whose background Rick cannot set.
    fake.set_background_formats(&[]);
    select(&mut harness, "lake.jpg");
    let labels = toolbar(&harness);
    assert!(labels.iter().any(|l| l == "Fit"), "{labels:?}");
    assert!(!labels.iter().any(|l| l == LABEL), "{labels:?}");
}

#[test]
fn a_desktop_that_refuses_is_told_like_any_other_failure() {
    let dir = tree();
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut harness = window(&fake, dir.path());
    select(&mut harness, "lake.jpg");
    fake.set_background_refusal(Some("the desktop said no"));

    let pane = harness.state().app().model().focused;
    let ctx = harness
        .state()
        .app()
        .action_ctx(pane, ext::Invocation::ContextMenu)
        .unwrap_or_else(|| panic!("ctx"));
    let mut events = Vec::new();
    harness.state_mut().app_mut().handle(
        app::Command::Run {
            action: ext::ActionId("core.set_desktop_background"),
            ctx,
        },
        &mut events,
    );
    assert!(
        events.iter().any(|event| matches!(
            event,
            app::Event::Notify { level: ext::Level::Error, text }
                if text.contains("the desktop said no")
        )),
        "the refusal reaches the user: {events:?}"
    );
    assert!(fake.desktop_backgrounds().is_empty());
}
