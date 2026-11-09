//! Screenshots of the GUI chrome for the pull request, rendered headlessly
//! with wgpu where the runner has an adapter (software ones do). Written to
//! `target/screenshots/<os>-<name>.png`; CI uploads the folder. Built with
//! the `screenshots` feature only, which brings wgpu in.
#![cfg(feature = "screenshots")]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use model::{Density, SelectionChange, SettingsChange, Theme};
use platform::{ConfigDir, Fake};
use tests::{local, media};
use ui_egui::keys::Intent;
use ui_egui::{EguiHost, RickUi, icons};

fn shell(start: &Path, size: egui::Vec2) -> Harness<'static, RickUi> {
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: vec![
            Arc::new(ext_preview_text::TextPreview::new()),
            Arc::new(ext_preview_code::CodePreview::new()),
            Arc::new(ext_preview_image::ImagePreview::new()),
            Arc::new(ext_preview_pdf::PdfPreview::new()),
        ],
        config_dir: Some(fake.config_dir()),
        start: Some(local(start)),
    });
    let mut harness = Harness::builder()
        .with_size(size)
        .with_pixels_per_point(2.0)
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

fn out_dir() -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/screenshots");
    fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    dir
}

fn shoot(harness: &mut Harness<'_, RickUi>, name: &str) {
    let image = harness
        .render()
        .unwrap_or_else(|e| panic!("no renderer for screenshots: {e}"));
    let path = out_dir().join(format!("{}-{name}.png", std::env::consts::OS));
    image.save(&path).unwrap_or_else(|e| panic!("{e}"));
    eprintln!("screenshot: {}", path.display());
}

/// A folder with pictures, a PDF, text and subfolders, under a deep path.
fn sample_tree() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let mut deep = dir.path().to_path_buf();
    for name in [
        "projects",
        "rick",
        "fixtures",
        "gallery",
        "holiday-2026",
        "day-one",
    ] {
        deep.push(name);
    }
    fs::create_dir_all(deep.join("raw")).unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir_all(deep.join("edited")).unwrap_or_else(|e| panic!("{e}"));
    for i in 0..9u32 {
        let image = image::RgbaImage::from_fn(160, 100, |x, y| {
            let c = ((x * 2 + y + i * 23) % 255) as u8;
            image::Rgba([c, (y * 2 % 255) as u8, 255 - c, 255])
        });
        let mut bytes = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap_or_else(|e| panic!("{e}"));
        fs::write(
            deep.join(format!("IMG_{:04}.png", 1200 + i)),
            bytes.into_inner(),
        )
        .unwrap_or_else(|e| panic!("{e}"));
    }
    fs::write(deep.join("itinerary.pdf"), ext_preview_pdf::sample_pdf(2))
        .unwrap_or_else(|e| panic!("{e}"));
    fs::write(deep.join("notes.md"), "# Day one\n").unwrap_or_else(|e| panic!("{e}"));
    fs::write(deep.join("main.rs"), "fn main() {}\n").unwrap_or_else(|e| panic!("{e}"));
    (dir, deep)
}

/// Opens the preview pane and selects `name`, waiting for its frame.
fn preview(harness: &mut Harness<'_, RickUi>, name: &str) {
    let pane = harness.state().app().model().focused;
    if !harness.state().inspector_open() {
        harness.state_mut().queue(Intent::ToggleInspector, pane);
        harness.step();
    }
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
    // Step before looking: until the pane has run a frame the slot still
    // holds the previous file's frame.
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        harness.step();
        if harness
            .state()
            .previews()
            .get(pane)
            .is_some_and(|s| s.has_frame() || s.error().is_some())
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
        assert!(Instant::now() < deadline, "{name}: no frame");
    }
    harness.step();
    harness.step();
}

/// A folder for the preview pane shots: prose, JSON, a photo, an A3 page.
fn preview_tree() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let root = dir.path().join("preview");
    fs::create_dir_all(&root).unwrap_or_else(|e| panic!("{e}"));
    let sentence = "The quick brown fox jumps over the lazy dog, and then it keeps running along the river bank until the long line finally ends here.\n";
    let mut prose = String::new();
    for i in 1..=40u32 {
        prose.push_str(&i.to_string());
        prose.push_str("  ");
        prose.push_str(sentence);
    }
    fs::write(root.join("journal.txt"), prose).unwrap_or_else(|e| panic!("{e}"));
    fs::write(
        root.join("trip.json"),
        r#"{"trip":"holiday-2026","days":[{"day":1,"place":"Lisbon","photos":9},{"day":2,"place":"Sintra","photos":14}],"budget":{"currency":"EUR","total":1840.5}}"#,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    fs::write(root.join("IMG_1200.jpg"), media::photo_jpeg(640, 480))
        .unwrap_or_else(|e| panic!("{e}"));
    fs::write(root.join("map-a3.pdf"), media::pdf(1, 1190, 842)).unwrap_or_else(|e| panic!("{e}"));
    (dir, root)
}

#[test]
#[ignore = "needs a wgpu adapter; CI runs it and uploads the images"]
fn screenshots_of_the_chrome() {
    let (_dir, deep) = sample_tree();

    // Title row and details view, side panel on.
    let mut harness = shell(&deep, egui::vec2(1280.0, 800.0));
    shoot(&mut harness, "title-row-and-details");

    // Two panes and a second workspace tab.
    let pane = harness.state().app().model().focused;
    harness.state_mut().app_mut().handle(
        app::Command::Split {
            pane,
            dir: model::SplitDir::Right,
        },
        &mut Vec::new(),
    );
    settle(&mut harness);
    let second = harness
        .state()
        .app()
        .model()
        .panes()
        .map(|(p, _)| p)
        .find(|p| *p != pane)
        .unwrap_or_else(|| panic!("second pane"));
    harness.state_mut().app_mut().handle(
        app::Command::Navigate {
            pane: second,
            to: local(&deep.join("raw")),
        },
        &mut Vec::new(),
    );
    harness.state_mut().app_mut().handle(
        app::Command::NewTab(local(&deep.join("edited"))),
        &mut Vec::new(),
    );
    settle(&mut harness);
    shoot(&mut harness, "two-panes-and-tabs");

    // Extra-large icon view with thumbnails.
    let mut harness = shell(&deep, egui::vec2(1280.0, 800.0));
    let pane = harness.state().app().model().focused;
    harness.state_mut().app_mut().handle(
        app::Command::SetView {
            pane,
            mode: model::ViewMode::Icons(model::IconSize::ExtraLarge),
        },
        &mut Vec::new(),
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    while harness.state().thumbnails().ready() < 10 && Instant::now() < deadline {
        harness.step();
        std::thread::sleep(Duration::from_millis(5));
    }
    harness.step();
    shoot(&mut harness, "icons-extra-large");

    // Narrow window: the breadcrumb collapses.
    let mut harness = shell(&deep, egui::vec2(520.0, 600.0));
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::B);
    harness.step();
    harness.step();
    shoot(&mut harness, "narrow-collapsed-breadcrumb");
}

/// Presses on the row labelled `label` and pulls it to `to` in steps, so
/// egui takes it for a drag; the button stays down.
fn pull(harness: &mut Harness<'_, RickUi>, label: &str, to: egui::Pos2) {
    use egui_kittest::kittest::Queryable as _;
    let from = harness.get_by_label(label).rect().center();
    harness.event(egui::Event::PointerMoved(from));
    harness.step();
    harness.event(egui::Event::PointerButton {
        pos: from,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
    for i in 1..=8u8 {
        let t = f32::from(i) / 8.0;
        harness.event(egui::Event::PointerMoved(from + (to - from) * t));
        harness.step();
    }
    harness.step();
}

fn drop(harness: &mut Harness<'_, RickUi>, at: egui::Pos2) {
    harness.event(egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
    settle(harness);
}

/// Drag and drop: a drag in progress with its ghost and indicator, the
/// spring-loaded folder, and a drop onto Pinned. The fourth picture the
/// issue asks for, a drag from Rick into the system file manager, needs
/// a desktop and a drag source the backend does not offer (QUESTIONS.md
/// 62), so it cannot be rendered here.
#[test]
#[ignore = "needs a wgpu adapter; CI runs it and uploads the images"]
fn screenshots_of_drag_and_drop() {
    use egui_kittest::kittest::Queryable as _;
    let (_dir, deep) = sample_tree();
    let mut harness = shell(&deep, egui::vec2(1280.0, 800.0));
    harness.state_mut().app_mut().handle(
        app::Command::Settings(model::SettingsChange::SpringDelay(600)),
        &mut Vec::new(),
    );
    settle(&mut harness);

    // Three pictures selected and pulled over the `raw` folder: the ghost
    // with its count badge and the move indicator.
    let pane = harness.state().app().model().focused;
    let ids: Vec<types::EntryId> = harness
        .state()
        .app()
        .model()
        .active_tab(pane)
        .map(|(_, t)| {
            t.listing
                .rows()
                .filter(|e| e.name.starts_with("IMG_"))
                .take(3)
                .map(|e| e.id)
                .collect()
        })
        .unwrap_or_default();
    harness.state_mut().app_mut().handle(
        app::Command::Select {
            pane,
            change: model::SelectionChange::Set(ids),
        },
        &mut Vec::new(),
    );
    settle(&mut harness);
    let raw = harness.get_by_label("raw").rect().center();
    pull(&mut harness, "IMG_1200.png", raw);
    shoot(&mut harness, "dnd-ghost-and-indicator");

    // Held there: the pane spring-loads into `raw`.
    for _ in 0..4 {
        harness.step();
    }
    settle(&mut harness);
    shoot(&mut harness, "dnd-spring-loaded-folder");
    harness.key_press(egui::Key::Escape);
    harness.step();
    drop(&mut harness, raw);

    // A folder dropped onto the Places list becomes a place; a file over
    // the panel is refused.
    let mut harness = shell(&deep, egui::vec2(1280.0, 800.0));
    let documents = panel_row(&harness, "Documents");
    pull(&mut harness, "raw", documents);
    shoot(&mut harness, "dnd-over-places");
    drop(&mut harness, documents);
    panel_row(&harness, "raw");
    shoot(&mut harness, "dnd-dropped-onto-places");
    pull(&mut harness, "notes.md", documents);
    shoot(&mut harness, "dnd-file-refused-by-the-panel");
    harness.key_press(egui::Key::Escape);
    harness.step();
    drop(&mut harness, documents);
}

/// The centre of the side panel row whose label contains `text`: left of
/// the focused pane.
fn panel_row(harness: &Harness<'_, RickUi>, text: &str) -> egui::Pos2 {
    let pane = harness.state().app().model().focused;
    let rect = harness
        .state()
        .pane_rects()
        .into_iter()
        .find(|(p, _)| *p == pane)
        .map_or_else(|| panic!("no pane rect"), |(_, r)| r);
    harness
        .query_all_by_label_contains(text)
        .map(|n| n.rect())
        .find(|r| r.center().x < rect.left())
        .map_or_else(|| panic!("no {text} in the side panel"), |r| r.center())
}

#[test]
#[ignore = "needs a wgpu adapter; CI runs it and uploads the images"]
fn screenshots_of_the_preview_pane() {
    let (_dir, root) = preview_tree();

    // Text with wrap off, then on.
    let mut harness = shell(&root, egui::vec2(1280.0, 800.0));
    preview(&mut harness, "journal.txt");
    shoot(&mut harness, "preview-text-wrap-off");
    harness.get_by_label(icons::WRAP).click();
    settle(&mut harness);
    shoot(&mut harness, "preview-text-wrap-on");
    harness.get_by_label(icons::WRAP).click();
    settle(&mut harness);

    // JSON pretty-printed.
    preview(&mut harness, "trip.json");
    harness.get_by_label(icons::PRETTY_PRINT).click();
    settle(&mut harness);
    shoot(&mut harness, "preview-json-pretty");

    // A photo with its EXIF section pinned.
    preview(&mut harness, "IMG_1200.jpg");
    harness.get_by_label(icons::CAMERA).click();
    settle(&mut harness);
    shoot(&mut harness, "preview-image-exif-pinned");

    // An A3 page in a narrow pane.
    let mut harness = shell(&root, egui::vec2(1100.0, 760.0));
    harness.state_mut().set_inspector_width(300.0);
    preview(&mut harness, "map-a3.pdf");
    shoot(&mut harness, "preview-pdf-narrow-pane");
}

/// Issue 60: the title row with three workspace tabs, the second one
/// shown, in Light and Dark at both densities, as the Figma frame *Main
/// view*.
#[test]
#[ignore = "needs a wgpu adapter; CI runs it and uploads the images"]
fn screenshots_of_the_title_row() {
    let (_dir, deep) = sample_tree();
    let mut harness = shell(&deep, egui::vec2(1920.0, 1000.0));
    let parent = deep
        .parent()
        .unwrap_or_else(|| panic!("parent"))
        .to_path_buf();
    // Three workspaces: the deep folder, its parent, the raw folder; the
    // middle one shown.
    for folder in [parent, deep.join("raw")] {
        harness
            .state_mut()
            .app_mut()
            .handle(app::Command::NewTab(local(&folder)), &mut Vec::new());
        settle(&mut harness);
    }
    let model = harness.state().app().model();
    let middle = model.workspaces(model.focused_window)[1].id;
    harness
        .state_mut()
        .app_mut()
        .handle(app::Command::ActivateWorkspace(middle), &mut Vec::new());
    settle(&mut harness);
    for (theme, density) in [
        (Theme::Light, Density::Comfortable),
        (Theme::Light, Density::Compact),
        (Theme::Dark, Density::Comfortable),
        (Theme::Dark, Density::Compact),
    ] {
        for change in [
            SettingsChange::Theme(theme),
            SettingsChange::Density(density),
        ] {
            harness
                .state_mut()
                .app_mut()
                .handle(app::Command::Settings(change), &mut Vec::new());
        }
        settle(&mut harness);
        harness.step();
        harness.step();
        let theme_name = match theme {
            Theme::Light => "light",
            Theme::Dark => "dark",
            Theme::System => "system",
        };
        let density_name = match density {
            Density::Comfortable => "comfortable",
            Density::Compact => "compact",
        };
        shoot(
            &mut harness,
            &format!("title-row-{theme_name}-{density_name}"),
        );
    }
}

/// Issue 60: the panes — two panes with their toolbars (parent chip,
/// breadcrumb, view options, handle), the view options menu open on the
/// left one, and a pane dragged by its handle over the other with the dock
/// cross and the ghost card — in Light and Dark at both densities.
#[test]
#[ignore = "needs a wgpu adapter; CI runs it and uploads the images"]
fn screenshots_of_the_panes() {
    use egui_kittest::kittest::Queryable as _;
    let (_dir, deep) = sample_tree();
    let mut harness = shell(&deep, egui::vec2(1920.0, 1000.0));
    let pane = harness.state().app().model().focused;
    harness.state_mut().app_mut().handle(
        app::Command::Split {
            pane,
            dir: model::SplitDir::Right,
        },
        &mut Vec::new(),
    );
    settle(&mut harness);
    let second = harness.state().app().model().focused;
    harness.state_mut().app_mut().handle(
        app::Command::Navigate {
            pane: second,
            to: local(&deep.join("raw")),
        },
        &mut Vec::new(),
    );
    settle(&mut harness);
    harness
        .state_mut()
        .app_mut()
        .handle(app::Command::FocusPane(pane), &mut Vec::new());
    settle(&mut harness);
    for (theme, density) in [
        (Theme::Light, Density::Comfortable),
        (Theme::Light, Density::Compact),
        (Theme::Dark, Density::Comfortable),
        (Theme::Dark, Density::Compact),
    ] {
        for change in [
            SettingsChange::Theme(theme),
            SettingsChange::Density(density),
        ] {
            harness
                .state_mut()
                .app_mut()
                .handle(app::Command::Settings(change), &mut Vec::new());
        }
        settle(&mut harness);
        let theme_name = match theme {
            Theme::Light => "light",
            Theme::Dark => "dark",
            Theme::System => "system",
        };
        let density_name = match density {
            Density::Comfortable => "comfortable",
            Density::Compact => "compact",
        };
        // The view options menu of the left pane.
        let rects = harness.state().pane_rects();
        let left_rect = rects
            .iter()
            .find(|(p, _)| *p == pane)
            .map_or_else(|| panic!("left pane"), |(_, r)| *r);
        let right_rect = rects
            .iter()
            .find(|(p, _)| *p == second)
            .map_or_else(|| panic!("right pane"), |(_, r)| *r);
        let button = harness
            .query_all_by_label("View options")
            .map(|n| n.rect())
            .find(|r| left_rect.contains(r.center()))
            .unwrap_or_else(|| panic!("view options of the left pane"));
        harness.event(egui::Event::PointerMoved(button.center()));
        harness.step();
        for pressed in [true, false] {
            harness.event(egui::Event::PointerButton {
                pos: button.center(),
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::default(),
            });
            harness.step();
        }
        harness.step();
        shoot(
            &mut harness,
            &format!("panes-view-options-{theme_name}-{density_name}"),
        );
        harness.key_press(egui::Key::Escape);
        harness.step();
        harness.step();

        // The right pane dragged by its handle over the left pane's left
        // tile, held there.
        let handle = harness
            .query_all_by_label("Drag pane")
            .map(|n| n.rect())
            .find(|r| right_rect.contains(r.center()))
            .unwrap_or_else(|| panic!("handle of the right pane"));
        let from = handle.center();
        let to = left_rect.center() + egui::vec2(-ui_egui::DOCK_TILE_PITCH, 0.0);
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
            harness.event(egui::Event::PointerMoved(from + (to - from) * t));
            harness.step();
        }
        harness.step();
        shoot(
            &mut harness,
            &format!("panes-drag-{theme_name}-{density_name}"),
        );
        // Back to where it was: release over nothing, outside every tile.
        let away = egui::pos2(right_rect.center().x, right_rect.bottom() - 40.0);
        harness.event(egui::Event::PointerMoved(away));
        harness.step();
        harness.event(egui::Event::PointerButton {
            pos: away,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::default(),
        });
        harness.step();
        settle(&mut harness);
    }
}

/// Issue 60: the file list — the header band and the row states (a
/// selection with the keyboard cursor in the focused pane, a selection in
/// the other pane), the empty folder, and the portal burst at three
/// moments of its loop — in Light and Dark at both densities.
#[test]
#[ignore = "needs a wgpu adapter; CI runs it and uploads the images"]
fn screenshots_of_the_file_list() {
    use egui_kittest::kittest::Queryable as _;
    let (_dir, deep) = sample_tree();
    let empty = deep.join("empty-folder");
    fs::create_dir_all(&empty).unwrap_or_else(|e| panic!("{e}"));
    // The `raw` folder gets pictures of its own for the right pane.
    let raw = deep.join("raw");
    for i in 0..5usize {
        fs::write(
            raw.join(format!("IMG_{:04}.jpg", 1200 + i)),
            vec![0u8; 1200 + 300 * i],
        )
        .unwrap_or_else(|e| panic!("{e}"));
    }
    let mut harness = shell(&deep, egui::vec2(1920.0, 1000.0));
    let pane = harness.state().app().model().focused;
    harness.state_mut().app_mut().handle(
        app::Command::Split {
            pane,
            dir: model::SplitDir::Right,
        },
        &mut Vec::new(),
    );
    settle(&mut harness);
    let second = harness.state().app().model().focused;
    harness.state_mut().app_mut().handle(
        app::Command::Navigate {
            pane: second,
            to: local(&raw),
        },
        &mut Vec::new(),
    );
    settle(&mut harness);
    step_until(&mut harness, Duration::from_secs(10), |h| {
        h.state()
            .app()
            .model()
            .active_tab(second)
            .is_some_and(|(_, t)| t.rows().next().is_some())
    });
    // A selection in the right pane, then the focus and a selection with
    // the cursor in the left one: blue there, grey in the other.
    select_named(&mut harness, "IMG_1200.jpg");
    harness
        .state_mut()
        .app_mut()
        .handle(app::Command::FocusPane(pane), &mut Vec::new());
    settle(&mut harness);
    let row = harness.get_by_label("IMG_1201.png").rect().center();
    harness.event(egui::Event::PointerMoved(row));
    harness.step();
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos: row,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        });
        harness.step();
    }
    harness.event(egui::Event::PointerMoved(egui::pos2(960.0, 30.0)));
    settle(&mut harness);
    for (theme, density) in [
        (Theme::Light, Density::Comfortable),
        (Theme::Light, Density::Compact),
        (Theme::Dark, Density::Comfortable),
        (Theme::Dark, Density::Compact),
    ] {
        for change in [
            SettingsChange::Theme(theme),
            SettingsChange::Density(density),
        ] {
            harness
                .state_mut()
                .app_mut()
                .handle(app::Command::Settings(change), &mut Vec::new());
        }
        settle(&mut harness);
        let theme_name = match theme {
            Theme::Light => "light",
            Theme::Dark => "dark",
            Theme::System => "system",
        };
        let density_name = match density {
            Density::Comfortable => "comfortable",
            Density::Compact => "compact",
        };
        shoot(
            &mut harness,
            &format!("file-list-rows-{theme_name}-{density_name}"),
        );
    }
    // The empty folder in the right pane.
    harness.state_mut().app_mut().handle(
        app::Command::Navigate {
            pane: second,
            to: local(&empty),
        },
        &mut Vec::new(),
    );
    settle(&mut harness);
    for theme in [Theme::Light, Theme::Dark] {
        harness.state_mut().app_mut().handle(
            app::Command::Settings(SettingsChange::Theme(theme)),
            &mut Vec::new(),
        );
        settle(&mut harness);
        let theme_name = match theme {
            Theme::Light => "light",
            Theme::Dark => "dark",
            Theme::System => "system",
        };
        shoot(&mut harness, &format!("file-list-empty-{theme_name}"));
    }
    // The portal burst on its own, at three moments of its loop.
    for (theme, dark) in [("light", false), ("dark", true)] {
        let scheme = if dark {
            &ui_egui::theme::Scheme::DARK
        } else {
            &ui_egui::theme::Scheme::LIGHT
        };
        let mut portal = Harness::builder()
            .with_size(egui::vec2(480.0, 160.0))
            .with_pixels_per_point(2.0)
            .build_ui(move |ui| {
                let rect = ui.max_rect();
                ui.painter().rect_filled(rect, 0.0, scheme.bg_pane);
                for (i, time) in [0.0, 0.4, 0.8].into_iter().enumerate() {
                    #[allow(clippy::cast_precision_loss)]
                    let x = rect.left() + 80.0 + i as f32 * 160.0;
                    ui_egui::portal::burst(
                        ui.painter(),
                        egui::pos2(x, rect.center().y),
                        time,
                        scheme.accent_focus,
                        scheme.icon_default,
                        false,
                    );
                }
            });
        // The glyphs come from the icon font, which the shell installs; a
        // bare harness gets it here.
        let fake = Fake::new().unwrap_or_else(|e| panic!("{e}"));
        ui_egui::fonts::install(&portal.ctx, &platform::SystemFonts::system_fonts(&fake));
        portal.step();
        portal.step();
        let image = portal
            .render()
            .unwrap_or_else(|e| panic!("no renderer for screenshots: {e}"));
        let path = out_dir().join(format!("{}-portal-burst-{theme}.png", std::env::consts::OS));
        image.save(&path).unwrap_or_else(|e| panic!("{e}"));
        eprintln!("screenshot: {}", path.display());
    }
}

/// Issue 60: the footer — the hidden-files toggle in both states, the
/// counts, the selection pill, the narrowed counts and the filter field
/// with its text — in Light and Dark at both densities.
#[test]
#[ignore = "needs a wgpu adapter; CI runs it and uploads the images"]
fn screenshots_of_the_footer() {
    let (_dir, deep) = sample_tree();
    fs::write(deep.join(".thumbs.db"), "x").unwrap_or_else(|e| panic!("{e}"));
    let mut harness = shell(&deep, egui::vec2(1920.0, 1000.0));
    let pane = harness.state().app().model().focused;
    harness.state_mut().app_mut().handle(
        app::Command::Split {
            pane,
            dir: model::SplitDir::Right,
        },
        &mut Vec::new(),
    );
    settle(&mut harness);
    let second = harness.state().app().model().focused;
    // The right pane: hidden files shown and a filter narrowing the list.
    harness
        .state_mut()
        .app_mut()
        .handle(app::Command::ToggleHidden(second), &mut Vec::new());
    harness.state_mut().app_mut().handle(
        app::Command::Filter {
            pane: second,
            text: "IMG_120".to_owned(),
        },
        &mut Vec::new(),
    );
    settle(&mut harness);
    // The left pane: focused by a click on a row, then two pictures
    // selected.
    let row = harness
        .get_all_by_label("IMG_1201.png")
        .map(|n| n.rect())
        .min_by(|a, b| a.left().total_cmp(&b.left()))
        .unwrap_or_else(|| panic!("no row"))
        .center();
    harness.event(egui::Event::PointerMoved(row));
    harness.step();
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos: row,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        });
        harness.step();
    }
    settle(&mut harness);
    let ids: Vec<_> = harness
        .state()
        .app()
        .model()
        .active_tab(pane)
        .map(|(_, t)| {
            t.rows()
                .filter(|e| e.name == "IMG_1201.png" || e.name == "IMG_1202.png")
                .map(|e| e.id)
                .collect()
        })
        .unwrap_or_default();
    harness.state_mut().app_mut().handle(
        app::Command::Select {
            pane,
            change: SelectionChange::Set(ids),
        },
        &mut Vec::new(),
    );
    harness.event(egui::Event::PointerMoved(egui::pos2(960.0, 30.0)));
    settle(&mut harness);
    for (theme, density) in [
        (Theme::Light, Density::Comfortable),
        (Theme::Light, Density::Compact),
        (Theme::Dark, Density::Comfortable),
        (Theme::Dark, Density::Compact),
    ] {
        for change in [
            SettingsChange::Theme(theme),
            SettingsChange::Density(density),
        ] {
            harness
                .state_mut()
                .app_mut()
                .handle(app::Command::Settings(change), &mut Vec::new());
        }
        settle(&mut harness);
        let theme_name = match theme {
            Theme::Light => "light",
            Theme::Dark => "dark",
            Theme::System => "system",
        };
        let density_name = match density {
            Density::Comfortable => "comfortable",
            Density::Compact => "compact",
        };
        shoot(&mut harness, &format!("footer-{theme_name}-{density_name}"));
    }
}

/// Issue 79 item 6: the column header with the pointer on the boundary
/// between Size and Type, in Light and Dark at both densities, for the
/// comparison with the frame *Column header — resize hover*.
#[test]
#[ignore = "needs a wgpu adapter; CI runs it and uploads the images"]
fn screenshots_of_the_column_header() {
    use egui_kittest::kittest::Queryable as _;
    let (_dir, deep) = sample_tree();
    let mut harness = shell(&deep, egui::vec2(1920.0, 1000.0));
    for (theme, density) in [
        (Theme::Light, Density::Comfortable),
        (Theme::Light, Density::Compact),
        (Theme::Dark, Density::Comfortable),
        (Theme::Dark, Density::Compact),
    ] {
        for change in [
            SettingsChange::Theme(theme),
            SettingsChange::Density(density),
        ] {
            harness
                .state_mut()
                .app_mut()
                .handle(app::Command::Settings(change), &mut Vec::new());
        }
        settle(&mut harness);
        // The boundary before the Type title: its cell's left edge less
        // half the gap between cells.
        let kind = harness.get_by_label_contains("Type").rect();
        harness.event(egui::Event::PointerMoved(egui::pos2(
            kind.left() - 3.0,
            kind.center().y,
        )));
        harness.step();
        harness.step();
        let theme_name = match theme {
            Theme::Light => "light",
            Theme::Dark => "dark",
            Theme::System => "system",
        };
        let density_name = match density {
            Density::Comfortable => "comfortable",
            Density::Compact => "compact",
        };
        shoot(
            &mut harness,
            &format!("column-header-{theme_name}-{density_name}"),
        );
    }
}

/// Issue 79 item 5: the path field in edit mode — Mod+L on the focused
/// pane — in Light and Dark at both densities, for the comparison with
/// the frame *Toolbar — path editing*.
#[test]
#[ignore = "needs a wgpu adapter; CI runs it and uploads the images"]
fn screenshots_of_the_path_field() {
    let (_dir, deep) = sample_tree();
    let mut harness = shell(&deep, egui::vec2(1920.0, 1000.0));
    let pane = harness.state().app().model().focused;
    for (theme, density) in [
        (Theme::Light, Density::Comfortable),
        (Theme::Light, Density::Compact),
        (Theme::Dark, Density::Comfortable),
        (Theme::Dark, Density::Compact),
    ] {
        for change in [
            SettingsChange::Theme(theme),
            SettingsChange::Density(density),
        ] {
            harness
                .state_mut()
                .app_mut()
                .handle(app::Command::Settings(change), &mut Vec::new());
        }
        settle(&mut harness);
        harness.state_mut().queue(Intent::FocusPath, pane);
        harness.step();
        harness.step();
        let theme_name = match theme {
            Theme::Light => "light",
            Theme::Dark => "dark",
            Theme::System => "system",
        };
        let density_name = match density {
            Density::Comfortable => "comfortable",
            Density::Compact => "compact",
        };
        shoot(
            &mut harness,
            &format!("path-field-{theme_name}-{density_name}"),
        );
        harness.key_press(egui::Key::Escape);
        harness.step();
    }
}

/// Issue 60: the side panel — the Places rows with the Trash pill, the
/// section headers, the drive cards and the operations drawer with two
/// running jobs — in Light and Dark at both densities.
#[test]
#[ignore = "needs a wgpu adapter; CI runs it and uploads the images"]
fn screenshots_of_the_side_panel() {
    use platform::Trash as _;
    let (_dir, deep) = sample_tree();
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let stick = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    // The trash's disk: the fake's own folder, small, so the pill is tinted.
    let fake_root = fake
        .config_dir()
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| fake.config_dir());
    fake.add_volume(platform::Volume {
        name: "System".into(),
        mount_point: fake_root,
        kind: platform::VolumeKind::Fixed,
        fs_type: "ext4".into(),
        device: None,
        total: Some(64 << 20),
        free: Some(38 << 20),
        read_only: false,
    });
    // The drives of the Figma frames and the *Drive usage — thresholds*
    // example: a warning and a critical fill, a read-only live USB and a
    // removable stick.
    for (name, sub, kind, fs, total, free, read_only) in [
        (
            "home",
            "home",
            platform::VolumeKind::Fixed,
            "ext4",
            1 << 40,
            154 << 30,
            false,
        ),
        (
            "Data",
            "data",
            platform::VolumeKind::Fixed,
            "ext4",
            2 << 40,
            100 << 30,
            false,
        ),
        (
            "Fedora-WS-Live-43",
            "live",
            platform::VolumeKind::Removable,
            "iso9660",
            2 << 30,
            0,
            true,
        ),
        (
            "test",
            "test",
            platform::VolumeKind::Removable,
            "exfat",
            64 << 30,
            59 << 30,
            false,
        ),
    ] {
        fake.add_volume(platform::Volume {
            name: name.into(),
            mount_point: stick.path().join(sub),
            kind,
            fs_type: fs.into(),
            device: None,
            total: Some(total),
            free: Some(free),
            read_only,
        });
    }
    fake.add_volume(platform::Volume {
        name: "nas".into(),
        mount_point: stick.path().join("nas"),
        kind: platform::VolumeKind::Network,
        fs_type: "cifs".into(),
        device: None,
        total: Some(4 << 40),
        free: Some(1 << 40),
        read_only: false,
    });
    // 2.3 MiB in the trash: about 3.5 % of the small disk, the mid tint.
    let junk = deep.join("old-export.bin");
    fs::write(&junk, vec![0u8; 2_400_000]).unwrap_or_else(|e| panic!("{e}"));
    fake.trash(&junk).unwrap_or_else(|e| panic!("{e}"));
    let host = Arc::new(EguiHost::new());
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: vec![Arc::new(ext_vfs_trash::TrashExtension::new())],
        config_dir: Some(fake.config_dir()),
        start: Some(local(&deep)),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1920.0, 1000.0))
        .with_pixels_per_point(2.0)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    step_until(&mut harness, Duration::from_secs(5), |h| {
        h.state().app().model().volumes.len() == 6 && h.state().app().model().trash.size > 0
    });
    // Two jobs held half way: a copy with the spinner, an archive job.
    let (go_tx, go_rx) = runtime::watch::channel(false);
    let jobs = harness.state().app().jobs().clone();
    for (name, total) in [
        ("Copy holiday-2026 to HOLIDAY", 2_300_000_000u64),
        ("Extract archive.tar.gz", 1_900_000_000u64),
    ] {
        let mut go = go_rx.clone();
        jobs.spawn_named(name, move |ctx| async move {
            let report = |done| {
                ctx.progress.report(types::Progress::Bytes {
                    done,
                    total: Some(total),
                });
            };
            for step in 0..40u64 {
                report(total / 100 * step);
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            while !*go.borrow_and_update() {
                tokio::select! {
                    changed = go.changed() => {
                        if changed.is_err() {
                            break;
                        }
                    }
                    () = tokio::time::sleep(Duration::from_millis(50)) => report(total / 100 * 40),
                }
            }
            Ok::<(), types::Error>(())
        });
    }
    step_until(&mut harness, Duration::from_secs(5), |h| {
        h.state().app().model().jobs.len() == 2
            && h.query_all_by_label_contains(" of ").count() >= 2
    });
    harness.event(egui::Event::PointerMoved(egui::pos2(960.0, 30.0)));
    for (theme, density) in [
        (Theme::Light, Density::Comfortable),
        (Theme::Light, Density::Compact),
        (Theme::Dark, Density::Comfortable),
        (Theme::Dark, Density::Compact),
    ] {
        for change in [
            SettingsChange::Theme(theme),
            SettingsChange::Density(density),
        ] {
            harness
                .state_mut()
                .app_mut()
                .handle(app::Command::Settings(change), &mut Vec::new());
        }
        for _ in 0..6 {
            harness.step();
        }
        let theme_name = match theme {
            Theme::Light => "light",
            Theme::Dark => "dark",
            Theme::System => "system",
        };
        let density_name = match density {
            Density::Comfortable => "comfortable",
            Density::Compact => "compact",
        };
        shoot(
            &mut harness,
            &format!("side-panel-{theme_name}-{density_name}"),
        );
    }
    let _ = go_tx.send(true);
    step_until(&mut harness, Duration::from_secs(20), |h| {
        h.state().app().model().jobs.is_empty()
    });
}

/// Issue 60: the context menus — a file's menu with its groups and the
/// empty area's menu — in Light and Dark at both densities.
#[test]
#[ignore = "needs a wgpu adapter; CI runs it and uploads the images"]
fn screenshots_of_the_context_menus() {
    use egui_kittest::kittest::Queryable as _;
    let (_dir, deep) = sample_tree();
    let mut harness = shell(&deep, egui::vec2(1920.0, 1000.0));
    let pane = harness.state().app().model().focused;
    let tab = harness
        .state()
        .app()
        .model()
        .active_tab(pane)
        .map_or_else(|| panic!("no tab"), |(id, _)| id);
    let right_click = |harness: &mut Harness<'_, RickUi>, at: egui::Pos2| {
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
        harness.step();
    };
    for (theme, density) in [
        (Theme::Light, Density::Comfortable),
        (Theme::Light, Density::Compact),
        (Theme::Dark, Density::Comfortable),
        (Theme::Dark, Density::Compact),
    ] {
        for change in [
            SettingsChange::Theme(theme),
            SettingsChange::Density(density),
        ] {
            harness
                .state_mut()
                .app_mut()
                .handle(app::Command::Settings(change), &mut Vec::new());
        }
        settle(&mut harness);
        let theme_name = match theme {
            Theme::Light => "light",
            Theme::Dark => "dark",
            Theme::System => "system",
        };
        let density_name = match density {
            Density::Comfortable => "comfortable",
            Density::Compact => "compact",
        };
        // The file's menu on a picture.
        let row = harness.get_by_label("IMG_1203.png").rect().center();
        right_click(&mut harness, row);
        shoot(
            &mut harness,
            &format!("context-menu-file-{theme_name}-{density_name}"),
        );
        harness.key_press(egui::Key::Escape);
        harness.step();
        harness.step();
        // The empty area's menu.
        let toolbar = harness
            .state()
            .toolbar_row(tab)
            .unwrap_or_else(|| panic!("no toolbar"));
        let last = harness.get_by_label("notes.md").rect();
        right_click(
            &mut harness,
            egui::pos2(toolbar.left() + 400.0, last.bottom() + 120.0),
        );
        shoot(
            &mut harness,
            &format!("context-menu-empty-{theme_name}-{density_name}"),
        );
        harness.key_press(egui::Key::Escape);
        harness.step();
        harness.step();
    }
}

/// Issue 60: the install dialogue — the JetBrains-shaped archive with its
/// icon, the effects card and the collapsed Advanced options — in Light
/// and Dark at both densities, then with Advanced options expanded, and
/// the archive whose version is missing.
#[test]
#[ignore = "needs a wgpu adapter; CI runs it and uploads the images"]
fn screenshots_of_the_install_dialogue() {
    use egui_kittest::kittest::Queryable as _;
    use ext_installer::fixtures::Fixture;
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    for fixture in [Fixture::JetBrains, Fixture::Tarbomb] {
        fixture.write(dir.path()).unwrap_or_else(|e| panic!("{e}"));
    }
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: vec![Arc::new(ext_installer::InstallerExtension::new())],
        config_dir: Some(fake.config_dir()),
        start: Some(local(dir.path())),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1920.0, 1000.0))
        .with_pixels_per_point(2.0)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    let open = |harness: &mut Harness<'_, RickUi>, archive: &str| {
        let pane = harness.state().app().model().focused;
        let id = harness
            .state()
            .app()
            .model()
            .active_tab(pane)
            .and_then(|(_, t)| t.rows().find(|e| e.name == archive).map(|e| e.id))
            .unwrap_or_else(|| panic!("no row {archive}"));
        harness.state_mut().app_mut().handle(
            app::Command::Select {
                pane,
                change: SelectionChange::Set(vec![id]),
            },
            &mut Vec::new(),
        );
        settle(harness);
        harness
            .state_mut()
            .queue(Intent::Action(ext::ActionId("installer.install")), pane);
        step_until(harness, Duration::from_secs(15), |h| {
            h.query_by_label("Install for").is_some()
        });
        harness.event(egui::Event::PointerMoved(egui::pos2(960.0, 30.0)));
        // The modal grows to its body's share of the screen over a few
        // frames.
        for _ in 0..8 {
            harness.step();
        }
    };
    let close = |harness: &mut Harness<'_, RickUi>| {
        harness.key_press(egui::Key::Escape);
        harness.step();
        harness.step();
        settle(harness);
    };
    for (theme, density) in [
        (Theme::Light, Density::Comfortable),
        (Theme::Light, Density::Compact),
        (Theme::Dark, Density::Comfortable),
        (Theme::Dark, Density::Compact),
    ] {
        for change in [
            SettingsChange::Theme(theme),
            SettingsChange::Density(density),
        ] {
            harness
                .state_mut()
                .app_mut()
                .handle(app::Command::Settings(change), &mut Vec::new());
        }
        settle(&mut harness);
        let theme_name = match theme {
            Theme::Light => "light",
            Theme::Dark => "dark",
            Theme::System => "system",
        };
        let density_name = match density {
            Density::Comfortable => "comfortable",
            Density::Compact => "compact",
        };
        open(&mut harness, Fixture::JetBrains.file_name());
        shoot(
            &mut harness,
            &format!("install-dialogue-{theme_name}-{density_name}"),
        );
        close(&mut harness);
    }
    // Light, Comfortable: Advanced options expanded, and the archive
    // without a version.
    for change in [
        SettingsChange::Theme(Theme::Light),
        SettingsChange::Density(Density::Comfortable),
    ] {
        harness
            .state_mut()
            .app_mut()
            .handle(app::Command::Settings(change), &mut Vec::new());
    }
    settle(&mut harness);
    open(&mut harness, Fixture::JetBrains.file_name());
    // The body scrolls: a touch-phased wheel over it, applied at once,
    // brings the card into view before the click.
    let body = harness.get_by_label("Install for").rect().center();
    harness.event(egui::Event::PointerMoved(body));
    harness.step();
    for (phase, delta) in [
        (egui::TouchPhase::Start, 0.0),
        (egui::TouchPhase::Move, -4000.0),
        (egui::TouchPhase::End, 0.0),
    ] {
        harness.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, delta),
            modifiers: egui::Modifiers::default(),
            phase,
        });
        harness.step();
    }
    harness.step();
    let advanced = harness
        .get_by_role_and_label(egui::accesskit::Role::CheckBox, "Advanced options")
        .rect()
        .center();
    harness.event(egui::Event::PointerMoved(advanced));
    harness.step();
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos: advanced,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        });
        harness.step();
    }
    harness.event(egui::Event::PointerMoved(egui::pos2(960.0, 30.0)));
    harness.step();
    harness.step();
    shoot(&mut harness, "install-dialogue-advanced");
    close(&mut harness);
    open(&mut harness, Fixture::Tarbomb.file_name());
    shoot(&mut harness, "install-dialogue-no-version");
    close(&mut harness);
}

/// Issue 60: the hash dialogue — a file's four digests with the Verify
/// field prefilled from the clipboard and matching its SHA-256 — in Light
/// and Dark at both densities, then the hashing state with the portal
/// burst and the progress line.
#[test]
#[ignore = "needs a wgpu adapter; CI runs it and uploads the images"]
fn screenshots_of_the_hash_dialogue() {
    use platform::Clipboard as _;
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    // A deterministic 300 KiB of noise, and 160 MiB of zeros the job takes
    // long enough over to be caught hashing.
    let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
    let small: Vec<u8> = (0..300 * 1024)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 24) as u8
        })
        .collect();
    let archive = "rick-0.3.0-linux-x64.tar.gz";
    let image = "rick-0.3.0-linux-x64.AppImage";
    fs::write(dir.path().join(archive), &small).unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join(image), vec![0u8; 160 << 20]).unwrap_or_else(|e| panic!("{e}"));
    let sha256 = app::hash::render(
        &app::hash::digests_of(&small).sha256,
        app::hash::Format::Hex,
    );
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: Vec::new(),
        config_dir: Some(fake.config_dir()),
        start: Some(local(dir.path())),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1920.0, 1000.0))
        .with_pixels_per_point(2.0)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    let open = |harness: &mut Harness<'_, RickUi>, name: &str| {
        let pane = harness.state().app().model().focused;
        let id = harness
            .state()
            .app()
            .model()
            .active_tab(pane)
            .and_then(|(_, t)| t.rows().find(|e| e.name == name).map(|e| e.id))
            .unwrap_or_else(|| panic!("no row {name}"));
        harness.state_mut().app_mut().handle(
            app::Command::Select {
                pane,
                change: SelectionChange::Set(vec![id]),
            },
            &mut Vec::new(),
        );
        settle(harness);
        harness
            .state_mut()
            .queue(Intent::Action(ext::ActionId("core.hash")), pane);
        step_until(harness, Duration::from_secs(15), |h| {
            h.state().hash_dialog().is_open()
        });
        harness.event(egui::Event::PointerMoved(egui::pos2(960.0, 30.0)));
        harness.step();
    };
    let wait_done = |harness: &mut Harness<'_, RickUi>| {
        step_until(harness, Duration::from_secs(30), |h| {
            h.state()
                .hash_dialog()
                .hashes()
                .is_some_and(|hashes| hashes.is_complete())
        });
        for _ in 0..8 {
            harness.step();
        }
    };
    let close = |harness: &mut Harness<'_, RickUi>| {
        harness.key_press(egui::Key::Escape);
        harness.step();
        harness.step();
        settle(harness);
    };
    for (theme, density) in [
        (Theme::Light, Density::Comfortable),
        (Theme::Light, Density::Compact),
        (Theme::Dark, Density::Comfortable),
        (Theme::Dark, Density::Compact),
    ] {
        for change in [
            SettingsChange::Theme(theme),
            SettingsChange::Density(density),
        ] {
            harness
                .state_mut()
                .app_mut()
                .handle(app::Command::Settings(change), &mut Vec::new());
        }
        settle(&mut harness);
        let theme_name = match theme {
            Theme::Light => "light",
            Theme::Dark => "dark",
            Theme::System => "system",
        };
        let density_name = match density {
            Density::Comfortable => "comfortable",
            Density::Compact => "compact",
        };
        fake.set_text(&sha256).unwrap_or_else(|e| panic!("{e}"));
        open(&mut harness, archive);
        wait_done(&mut harness);
        shoot(
            &mut harness,
            &format!("hash-dialogue-{theme_name}-{density_name}"),
        );
        close(&mut harness);
    }
    // Light, Comfortable: caught hashing the large file, nothing on the
    // clipboard that looks like a hash.
    for change in [
        SettingsChange::Theme(Theme::Light),
        SettingsChange::Density(Density::Comfortable),
    ] {
        harness
            .state_mut()
            .app_mut()
            .handle(app::Command::Settings(change), &mut Vec::new());
    }
    settle(&mut harness);
    fake.set_text("release notes")
        .unwrap_or_else(|e| panic!("{e}"));
    open(&mut harness, image);
    for _ in 0..3 {
        harness.step();
        std::thread::sleep(Duration::from_millis(40));
    }
    shoot(&mut harness, "hash-dialogue-hashing");
    close(&mut harness);
}

/// Issue 60: the settings window — the View page in Light and Dark at both
/// densities, then the General and Appearance pages, the PDF plugin's page
/// and a search that narrows the rows.
#[test]
#[ignore = "needs a wgpu adapter; CI runs it and uploads the images"]
fn screenshots_of_the_settings_window() {
    use egui::accesskit::Role;
    let (dir, deep) = sample_tree();
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: vec![Arc::new(ext_preview_pdf::PdfPreview::new())],
        config_dir: Some(fake.config_dir()),
        start: Some(local(&deep)),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1920.0, 1000.0))
        .with_pixels_per_point(2.0)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    let click = |harness: &mut Harness<'_, RickUi>, role: Role, label: &str| {
        let at = harness.get_by_role_and_label(role, label).rect().center();
        harness.event(egui::Event::PointerMoved(at));
        harness.step();
        for pressed in [true, false] {
            harness.event(egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::default(),
            });
            harness.step();
        }
        harness.event(egui::Event::PointerMoved(egui::pos2(960.0, 30.0)));
        harness.step();
        harness.step();
    };
    harness.state_mut().open_settings();
    for _ in 0..3 {
        harness.step();
    }
    click(&mut harness, Role::RadioButton, "View");
    for (theme, density) in [
        (Theme::Light, Density::Comfortable),
        (Theme::Light, Density::Compact),
        (Theme::Dark, Density::Comfortable),
        (Theme::Dark, Density::Compact),
    ] {
        for change in [
            SettingsChange::Theme(theme),
            SettingsChange::Density(density),
        ] {
            harness
                .state_mut()
                .app_mut()
                .handle(app::Command::Settings(change), &mut Vec::new());
        }
        settle(&mut harness);
        let theme_name = match theme {
            Theme::Light => "light",
            Theme::Dark => "dark",
            Theme::System => "system",
        };
        let density_name = match density {
            Density::Comfortable => "comfortable",
            Density::Compact => "compact",
        };
        shoot(
            &mut harness,
            &format!("settings-window-{theme_name}-{density_name}"),
        );
    }
    for change in [
        SettingsChange::Theme(Theme::Light),
        SettingsChange::Density(Density::Comfortable),
    ] {
        harness
            .state_mut()
            .app_mut()
            .handle(app::Command::Settings(change), &mut Vec::new());
    }
    settle(&mut harness);
    click(&mut harness, Role::RadioButton, "General");
    shoot(&mut harness, "settings-window-general");
    click(&mut harness, Role::RadioButton, "Appearance");
    shoot(&mut harness, "settings-window-appearance");
    click(&mut harness, Role::RadioButton, "PDF preview");
    shoot(&mut harness, "settings-window-plugin");
    click(&mut harness, Role::RadioButton, "View");
    let title = harness.get_by_label("Settings").rect();
    let search = harness
        .get_all_by_role(Role::TextInput)
        .find(|node| {
            let rect = node.rect();
            (rect.left() - title.left()).abs() < 40.0
                && rect.top() > title.bottom()
                && rect.top() < title.bottom() + 80.0
        })
        .unwrap_or_else(|| panic!("no search field under the Settings title"));
    search.focus();
    search.type_text("hidden");
    harness.step();
    harness.step();
    shoot(&mut harness, "settings-window-search");
    // The folder outlives the harness.
    let _folder = dir;
}

/// Issue 17: the Preview tab docked below the explorer and torn out into a
/// window of its own.
#[test]
#[ignore = "needs a wgpu adapter; CI runs it and uploads the images"]
fn screenshots_of_the_preview_tab_docked_and_torn_out() {
    use model::{DockTarget, Side};
    let (_dir, root) = preview_tree();

    // The Preview tab docked below the explorer.
    let mut harness = shell(&root, egui::vec2(1280.0, 800.0));
    preview(&mut harness, "IMG_1200.jpg");
    let model = harness.state().app().model();
    let pane = model.focused;
    let preview_tab = model
        .preview_tab(model.focused_window)
        .unwrap_or_else(|| panic!("preview tab"));
    harness.state_mut().app_mut().handle(
        app::Command::MoveTab {
            tab: preview_tab,
            to: DockTarget::Beside {
                pane,
                side: Side::Bottom,
            },
        },
        &mut Vec::new(),
    );
    settle(&mut harness);
    shoot(&mut harness, "preview-tab-docked-below");

    // Torn out: the window that holds only the Preview tab.
    harness.state_mut().app_mut().handle(
        app::Command::MoveTab {
            tab: preview_tab,
            to: DockTarget::NewWindow,
        },
        &mut Vec::new(),
    );
    settle(&mut harness);
    shoot(&mut harness, "preview-tab-torn-out");
}

/// Steps frames until `done` holds, within `limit`.
fn step_until(
    harness: &mut Harness<'_, RickUi>,
    limit: Duration,
    mut done: impl FnMut(&Harness<'_, RickUi>) -> bool,
) {
    let deadline = Instant::now() + limit;
    while !done(harness) {
        assert!(Instant::now() < deadline, "condition not met in time");
        harness.step();
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn select_named(harness: &mut Harness<'_, RickUi>, name: &str) {
    let pane = harness.state().app().model().focused;
    let id = harness
        .state()
        .app()
        .model()
        .active_tab(pane)
        .and_then(|(_, t)| t.rows().find(|e| e.name == name).map(|e| e.id))
        .unwrap_or_else(|| panic!("no entry {name}"));
    harness.state_mut().app_mut().handle(
        app::Command::Select {
            pane,
            change: SelectionChange::Set(vec![id]),
        },
        &mut Vec::new(),
    );
    settle(harness);
}

/// The side panel and list: Places with This Machine and Network, the
/// Trash view with items, the operations area mid-copy, tree view expanded,
/// inline rename and the conflict dialogue.
#[test]
#[ignore = "needs a wgpu adapter; CI runs it and uploads the images"]
fn screenshots_of_the_side_panel_and_list() {
    use egui_kittest::kittest::Queryable as _;
    let (_dir, deep) = sample_tree();
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let stick = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    fake.add_volume(platform::Volume {
        name: "System".into(),
        mount_point: stick.path().join("system"),
        kind: platform::VolumeKind::Fixed,
        fs_type: "ext4".into(),
        device: None,
        total: Some(512 << 30),
        free: Some(200 << 30),
        read_only: false,
    });
    fake.add_volume(platform::Volume {
        name: "HOLIDAY".into(),
        mount_point: stick.path().to_path_buf(),
        kind: platform::VolumeKind::Removable,
        fs_type: "exfat".into(),
        device: None,
        total: Some(64 << 30),
        free: Some(21 << 30),
        read_only: false,
    });
    fake.add_volume(platform::Volume {
        name: "nas".into(),
        mount_point: stick.path().join("nas"),
        kind: platform::VolumeKind::Network,
        fs_type: "cifs".into(),
        device: None,
        total: Some(4 << 40),
        free: Some(1 << 40),
        read_only: false,
    });
    let host = Arc::new(EguiHost::new());
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: vec![
            Arc::new(ext_preview_text::TextPreview::new()),
            Arc::new(ext_preview_image::ImagePreview::new()),
            Arc::new(ext_vfs_trash::TrashExtension::new()),
        ],
        config_dir: Some(fake.config_dir()),
        start: Some(local(&deep)),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_pixels_per_point(2.0)
        .with_step_dt(0.05)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    step_until(&mut harness, Duration::from_secs(5), |h| {
        h.state().app().model().volumes.len() == 3
    });
    harness.step();
    shoot(&mut harness, "side-panel-places-and-sections");

    // The This Machine overview.
    let pane = harness.state().app().model().focused;
    harness.state_mut().app_mut().handle(
        app::Command::Navigate {
            pane,
            to: vfs_rick::Virtual::Machine.location(),
        },
        &mut Vec::new(),
    );
    settle(&mut harness);
    shoot(&mut harness, "this-machine-overview");
    harness
        .state_mut()
        .app_mut()
        .handle(app::Command::Back(pane), &mut Vec::new());
    settle(&mut harness);

    // The Trash view with items.
    select_named(&mut harness, "IMG_1203.png");
    harness
        .state_mut()
        .queue(Intent::Action(ext::ActionId("core.trash")), pane);
    harness.step();
    settle(&mut harness);
    select_named(&mut harness, "notes.md");
    harness
        .state_mut()
        .queue(Intent::Action(ext::ActionId("core.trash")), pane);
    harness.step();
    settle(&mut harness);
    // The Trash view, served by the trash plugin, with the two items.
    harness.state_mut().app_mut().handle(
        app::Command::Navigate {
            pane,
            to: vfs_rick::Virtual::Trash.location(),
        },
        &mut Vec::new(),
    );
    settle(&mut harness);
    step_until(&mut harness, Duration::from_secs(5), |h| {
        h.state()
            .app()
            .model()
            .active_tab(pane)
            .is_some_and(|(_, t)| t.listing.len() == 2)
    });
    harness.step();
    shoot(&mut harness, "trash-view-with-items");
    harness
        .state_mut()
        .app_mut()
        .handle(app::Command::Back(pane), &mut Vec::new());
    settle(&mut harness);

    // Tree view, `raw` expanded.
    harness.state_mut().app_mut().handle(
        app::Command::SetView {
            pane,
            mode: model::ViewMode::Tree,
        },
        &mut Vec::new(),
    );
    settle(&mut harness);
    fs::write(deep.join("raw/IMG_1300.dng"), "raw").unwrap_or_else(|e| panic!("{e}"));
    fs::write(deep.join("raw/IMG_1301.dng"), "raw").unwrap_or_else(|e| panic!("{e}"));
    let raw = harness
        .state()
        .app()
        .model()
        .active_tab(pane)
        .and_then(|(_, t)| t.rows().find(|e| e.name == "raw").map(|e| e.id))
        .unwrap_or_else(|| panic!("raw"));
    harness
        .state_mut()
        .app_mut()
        .handle(app::Command::TreeExpand { pane, id: raw }, &mut Vec::new());
    settle(&mut harness);
    shoot(&mut harness, "tree-view-expanded");

    // Inline rename: F2 on a row.
    harness.state_mut().app_mut().handle(
        app::Command::SetView {
            pane,
            mode: model::ViewMode::Details,
        },
        &mut Vec::new(),
    );
    settle(&mut harness);
    select_named(&mut harness, "IMG_1204.png");
    harness.key_press(egui::Key::F2);
    harness.step();
    harness.step();
    shoot(&mut harness, "inline-rename");
    harness.key_press(egui::Key::Escape);
    harness.step();

    // The conflict dialogue: paste a file over itself in `edited`.
    fs::write(deep.join("edited/IMG_1205.png"), "older").unwrap_or_else(|e| panic!("{e}"));
    select_named(&mut harness, "IMG_1205.png");
    harness
        .state_mut()
        .queue(Intent::Action(ext::ActionId("core.copy")), pane);
    harness.step();
    harness.state_mut().app_mut().handle(
        app::Command::Navigate {
            pane,
            to: local(&deep.join("edited")),
        },
        &mut Vec::new(),
    );
    settle(&mut harness);
    harness
        .state_mut()
        .queue(Intent::Action(ext::ActionId("core.paste")), pane);
    harness.step();
    step_until(&mut harness, Duration::from_secs(5), |h| {
        !h.state().app().model().conflicts.is_empty()
    });
    harness.step();
    harness.step();
    shoot(&mut harness, "conflict-dialogue");
    harness.get_by_label("Cancel").click();
    harness.step();
    settle(&mut harness);

    // The operations area mid-copy: a 1 GiB sparse file through the fast
    // path shows its row with progress; the picture is taken on the first
    // frame that shows it running.
    let big = deep.join("archive.bin");
    {
        use std::io::{Seek, SeekFrom, Write};
        let mut file = fs::File::create(&big).unwrap_or_else(|e| panic!("{e}"));
        file.write_all(&vec![0x5Au8; 4 << 20])
            .unwrap_or_else(|e| panic!("{e}"));
        file.seek(SeekFrom::Start((1 << 30) - (4 << 20)))
            .unwrap_or_else(|e| panic!("{e}"));
        file.write_all(&vec![0xA5u8; 4 << 20])
            .unwrap_or_else(|e| panic!("{e}"));
        file.set_len(1 << 30).unwrap_or_else(|e| panic!("{e}"));
    }
    harness.state_mut().app_mut().handle(
        app::Command::Navigate {
            pane,
            to: local(&deep),
        },
        &mut Vec::new(),
    );
    settle(&mut harness);
    select_named(&mut harness, "archive.bin");
    harness
        .state_mut()
        .queue(Intent::Action(ext::ActionId("core.copy")), pane);
    harness.step();
    harness.state_mut().app_mut().handle(
        app::Command::Navigate {
            pane,
            to: local(&deep.join("edited")),
        },
        &mut Vec::new(),
    );
    settle(&mut harness);
    harness
        .state_mut()
        .queue(Intent::Action(ext::ActionId("core.paste")), pane);
    harness.step();
    step_until(&mut harness, Duration::from_secs(10), |h| {
        !h.state().app().model().jobs.is_empty()
    });
    harness.step();
    shoot(&mut harness, "operations-area-mid-copy");
    settle(&mut harness);
}

/// The scheme and density tokens (issue 60): the Figma main frame's
/// layout — two panes and the preview — in Light and Dark, Comfortable
/// and Compact, for the comparison with the frames.
/// The About dialog (specs/about-dialog.md) in both schemes and both
/// densities, beside the Figma frame *About Rick*, and its What's new page.
#[test]
#[ignore = "needs a wgpu adapter; CI runs it and uploads the images"]
fn screenshots_of_the_about_dialog() {
    let (_dir, deep) = sample_tree();
    let mut harness = shell(&deep, egui::vec2(1100.0, 1000.0));
    let build = ui_egui::about::Build {
        version: "0.3.0-dev".to_owned(),
        commit: "a1b2c3d".to_owned(),
        target: "x86_64-unknown-linux-gnu".to_owned(),
        date: "2026-11-09".to_owned(),
        toolchain: "rustc 1.97.0".to_owned(),
        contributors: "Claude\nSerge\n",
        roles: "",
        plugins_dir: None,
        third_party: None,
        plugin_licences: Vec::new(),
    };
    harness.state_mut().set_build(build);
    for (theme, density) in [
        (Theme::Light, Density::Comfortable),
        (Theme::Light, Density::Compact),
        (Theme::Dark, Density::Comfortable),
        (Theme::Dark, Density::Compact),
    ] {
        for change in [
            SettingsChange::Theme(theme),
            SettingsChange::Density(density),
        ] {
            harness
                .state_mut()
                .app_mut()
                .handle(app::Command::Settings(change), &mut Vec::new());
        }
        harness.state_mut().open_about();
        // The pointer out of the way, so no button shows a hover tint.
        harness.event(egui::Event::PointerMoved(egui::pos2(20.0, 980.0)));
        settle(&mut harness);
        harness.step();
        harness.step();
        let theme_name = match theme {
            Theme::Light => "light",
            Theme::Dark => "dark",
            Theme::System => "system",
        };
        let density_name = match density {
            Density::Comfortable => "comfortable",
            Density::Compact => "compact",
        };
        shoot(&mut harness, &format!("about-{theme_name}-{density_name}"));
        if theme == Theme::Light && density == Density::Comfortable {
            let at = harness
                .get_by_role_and_label(egui::accesskit::Role::Button, "What's new")
                .rect()
                .center();
            harness.event(egui::Event::PointerMoved(at));
            harness.step();
            for pressed in [true, false] {
                harness.event(egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::default(),
                });
                harness.step();
            }
            harness.event(egui::Event::PointerMoved(egui::pos2(20.0, 980.0)));
            harness.step();
            harness.step();
            shoot(&mut harness, "about-whats-new");
        }
        harness.key_press(egui::Key::Escape);
        harness.step();
        harness.key_press(egui::Key::Escape);
        harness.step();
    }
}

/// The Third-party licences page (specs/about-dialog.md) beside the Figma
/// frame *About — Third-party licences*: the list with a few crates, and
/// the list with a row open.
#[test]
#[ignore = "needs a wgpu adapter; CI runs it and uploads the images"]
fn screenshots_of_the_third_party_licences() {
    let (_dir, deep) = sample_tree();
    let mut harness = shell(&deep, egui::vec2(1100.0, 1000.0));
    let component =
        |name: &str, version: &str, license: &str, author: &str, repo: &str, text: &str| {
            ui_egui::about::Component {
                name: name.to_owned(),
                version: version.to_owned(),
                license: license.to_owned(),
                copyright: vec![author.to_owned()],
                repository: repo.to_owned(),
                text: text.to_owned(),
            }
        };
    let mit = "MIT License\n\nCopyright (c) 2018 Emil Ernerfeldt\n\nPermission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the \"Software\"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:\n\nThe above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.";
    let apache = "Apache License\nVersion 2.0, January 2004\nhttp://www.apache.org/licenses/\n\nTERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION";
    let build = ui_egui::about::Build {
        version: "0.3.0-dev".to_owned(),
        commit: "a1b2c3d".to_owned(),
        target: "x86_64-unknown-linux-gnu".to_owned(),
        date: "2026-11-09".to_owned(),
        toolchain: "rustc 1.97.0".to_owned(),
        contributors: "Claude\nSerge\n",
        roles: "",
        plugins_dir: None,
        third_party: Some(vec![
            component(
                "egui",
                "0.36.2",
                "MIT",
                "Emil Ernerfeldt",
                "https://github.com/emilk/egui",
                mit,
            ),
            component(
                "eframe",
                "0.36.2",
                "Apache-2.0",
                "Emil Ernerfeldt",
                "https://github.com/emilk/egui",
                apache,
            ),
            component(
                "egui_dock",
                "0.21.0",
                "MIT",
                "Adam Gąsior",
                "https://github.com/Adanos020/egui_dock",
                mit,
            ),
            component(
                "wgpu",
                "30.0.1",
                "Apache-2.0",
                "gfx-rs developers",
                "https://github.com/gfx-rs/wgpu",
                apache,
            ),
            component(
                "tokio",
                "1.49.0",
                "MIT",
                "Tokio Contributors",
                "https://github.com/tokio-rs/tokio",
                mit,
            ),
            component(
                "serde",
                "1.0.228",
                "Apache-2.0",
                "David Tolnay",
                "https://github.com/serde-rs/serde",
                apache,
            ),
            component(
                "gix",
                "0.90.0",
                "Apache-2.0",
                "Sebastian Thiel",
                "https://github.com/GitoxideLabs/gitoxide",
                apache,
            ),
            component(
                "image",
                "0.25.9",
                "Apache-2.0",
                "The image-rs Developers",
                "https://github.com/image-rs/image",
                apache,
            ),
        ]),
        plugin_licences: Vec::new(),
    };
    harness.state_mut().set_build(build);
    harness.state_mut().app_mut().handle(
        app::Command::Settings(SettingsChange::Theme(Theme::Light)),
        &mut Vec::new(),
    );
    harness.state_mut().open_about();
    settle(&mut harness);
    let click = |harness: &mut Harness<'_, RickUi>, label: &str| {
        let at = harness
            .get_by_role_and_label(egui::accesskit::Role::Button, label)
            .rect()
            .center();
        harness.event(egui::Event::PointerMoved(at));
        harness.step();
        for pressed in [true, false] {
            harness.event(egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::default(),
            });
            harness.step();
        }
        harness.event(egui::Event::PointerMoved(egui::pos2(20.0, 980.0)));
        harness.step();
        harness.step();
    };
    click(&mut harness, "Third-party licences");
    shoot(&mut harness, "about-licences");
    click(&mut harness, "egui 0.36.2");
    shoot(&mut harness, "about-licences-open");
    harness.state_mut().app_mut().handle(
        app::Command::Settings(SettingsChange::Theme(Theme::Dark)),
        &mut Vec::new(),
    );
    settle(&mut harness);
    harness.step();
    shoot(&mut harness, "about-licences-dark");
}

#[test]
#[ignore = "needs a wgpu adapter; CI runs it and uploads the images"]
fn screenshots_of_the_schemes_and_densities() {
    let (_dir, deep) = sample_tree();
    let mut harness = shell(&deep, egui::vec2(1920.0, 1000.0));
    let pane = harness.state().app().model().focused;
    harness.state_mut().app_mut().handle(
        app::Command::Split {
            pane,
            dir: model::SplitDir::Right,
        },
        &mut Vec::new(),
    );
    settle(&mut harness);
    let second = harness
        .state()
        .app()
        .model()
        .panes()
        .map(|(p, _)| p)
        .find(|p| *p != pane)
        .unwrap_or_else(|| panic!("second pane"));
    // The left pane on the small parent folder, the right one on the
    // pictures with one selected and previewed, as the Figma frame.
    let parent = deep
        .parent()
        .unwrap_or_else(|| panic!("parent"))
        .to_path_buf();
    harness.state_mut().app_mut().handle(
        app::Command::Navigate {
            pane,
            to: local(&parent),
        },
        &mut Vec::new(),
    );
    settle(&mut harness);
    harness
        .state_mut()
        .app_mut()
        .handle(app::Command::FocusPane(second), &mut Vec::new());
    step_until(&mut harness, Duration::from_secs(10), |h| {
        h.state()
            .app()
            .model()
            .active_tab(second)
            .is_some_and(|(_, t)| t.rows().next().is_some())
    });
    let name = "IMG_1200.png";
    preview(&mut harness, name);
    for (theme, density) in [
        (Theme::Light, Density::Comfortable),
        (Theme::Light, Density::Compact),
        (Theme::Dark, Density::Comfortable),
        (Theme::Dark, Density::Compact),
    ] {
        for change in [
            SettingsChange::Theme(theme),
            SettingsChange::Density(density),
        ] {
            harness
                .state_mut()
                .app_mut()
                .handle(app::Command::Settings(change), &mut Vec::new());
        }
        settle(&mut harness);
        harness.step();
        harness.step();
        let theme_name = match theme {
            Theme::Light => "light",
            Theme::Dark => "dark",
            Theme::System => "system",
        };
        let density_name = match density {
            Density::Comfortable => "comfortable",
            Density::Compact => "compact",
        };
        shoot(&mut harness, &format!("scheme-{theme_name}-{density_name}"));
    }
}
