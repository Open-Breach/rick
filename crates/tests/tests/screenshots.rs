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
        .map_or_default(|(_, t)| {
            t.listing
                .rows()
                .filter(|e| e.name.starts_with("IMG_"))
                .take(3)
                .map(|e| e.id)
                .collect()
        });
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

    // A photo with its File info card pinned.
    preview(&mut harness, "IMG_1200.jpg");
    harness.get_by_label("File info").click();
    settle(&mut harness);
    shoot(&mut harness, "preview-image-info-pinned");

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
/// cross and its ghost — in Light and Dark at both densities.
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

/// A renderer that draws the pointer as the cursor the frame asked for:
/// the open hand for `Grab`, the closed hand for `Grabbing`, where
/// kittest's own renderer draws one arrow-like triangle whatever the
/// cursor is. The frame sequences of the drags are rendered through it, so
/// a frame shows which cursor the user sees.
struct HandCursor(egui_kittest::wgpu::WgpuTestRenderer);

impl egui_kittest::TestRenderer for HandCursor {
    fn handle_delta(&mut self, delta: &mut egui::TexturesDelta) {
        self.0.handle_delta(delta);
    }

    fn render(
        &mut self,
        ctx: &egui::Context,
        output: &egui::FullOutput,
    ) -> Result<image::RgbaImage, String> {
        let closed = match output.platform_output.cursor_icon {
            egui::CursorIcon::Grab => false,
            egui::CursorIcon::Grabbing => true,
            _ => return self.0.render(ctx, output),
        };
        let Some(at) = ctx.input(|i| i.pointer.hover_pos()) else {
            return self.0.render(ctx, output);
        };
        let mut output = output.clone();
        // The harness appended its triangle last; the hand takes its place.
        output.shapes.pop();
        let clip_rect = ctx.content_rect();
        output.shapes.extend(
            hand(at, closed)
                .into_iter()
                .map(|shape| egui::epaint::ClippedShape { clip_rect, shape }),
        );
        self.0.render(ctx, &output)
    }
}

/// A hand cursor around `at` as shapes: a palm with a thumb and four
/// fingers, stretched for the open hand and curled into knuckles for the
/// closed one; white with a black outline, like the system's.
fn hand(at: egui::Pos2, closed: bool) -> Vec<egui::Shape> {
    let outline = egui::Stroke::new(1.2, egui::Color32::BLACK);
    let part = |min: egui::Vec2, size: egui::Vec2, radius: f32| {
        egui::Shape::Rect(egui::epaint::RectShape::new(
            egui::Rect::from_min_size(at + min, size),
            radius,
            egui::Color32::WHITE,
            outline,
            egui::StrokeKind::Middle,
        ))
    };
    let finger = if closed { 5.0 } else { 10.0 };
    let mut shapes = Vec::new();
    // The thumb, then the fingers, then the palm over their roots.
    let thumb = if closed {
        part(egui::vec2(-10.0, 0.0), egui::vec2(5.0, 6.0), 2.5)
    } else {
        part(egui::vec2(-11.5, -1.0), egui::vec2(4.5, 9.0), 2.2)
    };
    shapes.push(thumb);
    for i in 0..4u8 {
        let x = -7.0 + f32::from(i) * 3.6;
        shapes.push(part(
            egui::vec2(x, -1.0 - finger),
            egui::vec2(3.4, finger + 4.0),
            1.7,
        ));
    }
    shapes.push(part(egui::vec2(-7.5, 0.0), egui::vec2(15.0, 11.0), 4.0));
    shapes
}

/// A window like [`shell`]'s whose frames show the hand cursors.
fn shell_with_hands(start: &Path, size: egui::Vec2) -> Harness<'static, RickUi> {
    shell_with_hands_at(start, size, 0.25)
}

/// [`shell_with_hands`] with frames `dt` seconds apart: a sixtieth shows an
/// animation frame by frame.
fn shell_with_hands_at(start: &Path, size: egui::Vec2, dt: f32) -> Harness<'static, RickUi> {
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
        .with_pixels_per_point(2.0)
        .with_step_dt(dt)
        .renderer(HandCursor(egui_kittest::wgpu::WgpuTestRenderer::new()))
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    harness
}

/// The frames of one sequence, to be laid out as a sheet.
struct Frames {
    name: String,
    frames: Vec<image::RgbaImage>,
    /// Keep only this many pixel rows from the top of each frame: a
    /// sequence that plays in the title row is laid out as a film strip.
    strip: Option<u32>,
}

impl Frames {
    fn new(name: String) -> Self {
        Self {
            name,
            frames: Vec::new(),
            strip: None,
        }
    }

    /// A sequence of which only the top `rows` pixel rows matter.
    fn strip(name: String, rows: u32) -> Self {
        Self {
            name,
            frames: Vec::new(),
            strip: Some(rows),
        }
    }

    /// Renders the frame as it stands, saves it as `<name>-<n>-<step>` and
    /// keeps it for the sheet.
    fn shoot(&mut self, harness: &mut Harness<'_, RickUi>, step: &str) {
        let mut image = harness
            .render()
            .unwrap_or_else(|e| panic!("no renderer for screenshots: {e}"));
        if let Some(rows) = self.strip {
            let (w, h) = (image.width(), rows.min(image.height()));
            image = image::imageops::crop_imm(&image, 0, 0, w, h).to_image();
        }
        let n = self.frames.len() + 1;
        let path = out_dir().join(format!(
            "{}-{}-{n}-{step}.png",
            std::env::consts::OS,
            self.name
        ));
        image.save(&path).unwrap_or_else(|e| panic!("{e}"));
        eprintln!("frame: {}", path.display());
        self.frames.push(image);
    }

    /// Saves the sequence as one sheet, `<name>.png`: the frames at half
    /// size, two to a row, in order; a film strip at full size, one frame
    /// under the other.
    fn sheet(&self) {
        const GUTTER: u32 = 12;
        let Some(first) = self.frames.first() else {
            return;
        };
        if self.strip.is_some() {
            let (w, h) = (first.width(), first.height());
            let rows = u32::try_from(self.frames.len()).unwrap_or(1);
            let mut sheet = image::RgbaImage::from_pixel(
                w + 2 * GUTTER,
                rows * h + (rows + 1) * GUTTER,
                image::Rgba([128, 128, 128, 255]),
            );
            for (i, frame) in self.frames.iter().enumerate() {
                let i = u32::try_from(i).unwrap_or(0);
                let y = GUTTER + i * (h + GUTTER);
                image::imageops::overlay(&mut sheet, frame, i64::from(GUTTER), i64::from(y));
            }
            let path = out_dir().join(format!("{}-{}.png", std::env::consts::OS, self.name));
            sheet.save(&path).unwrap_or_else(|e| panic!("{e}"));
            eprintln!("sheet: {}", path.display());
            return;
        }
        let (w, h) = (first.width() / 2, first.height() / 2);
        let rows = u32::try_from(self.frames.len().div_ceil(2)).unwrap_or(1);
        let mut sheet = image::RgbaImage::from_pixel(
            2 * w + 3 * GUTTER,
            rows * h + (rows + 1) * GUTTER,
            image::Rgba([128, 128, 128, 255]),
        );
        for (i, frame) in self.frames.iter().enumerate() {
            let small = image::imageops::resize(frame, w, h, image::imageops::FilterType::Triangle);
            let i = u32::try_from(i).unwrap_or(0);
            let x = GUTTER + (i % 2) * (w + GUTTER);
            let y = GUTTER + (i / 2) * (h + GUTTER);
            image::imageops::overlay(&mut sheet, &small, i64::from(x), i64::from(y));
        }
        let path = out_dir().join(format!("{}-{}.png", std::env::consts::OS, self.name));
        sheet.save(&path).unwrap_or_else(|e| panic!("{e}"));
        eprintln!("sheet: {}", path.display());
    }
}

/// Moves the pointer to `to` in `steps` frames.
fn glide(harness: &mut Harness<'_, RickUi>, from: egui::Pos2, to: egui::Pos2, steps: u8) {
    for i in 1..=steps {
        let t = f32::from(i) / f32::from(steps);
        harness.event(egui::Event::PointerMoved(from + (to - from) * t));
        harness.step();
    }
    harness.step();
}

/// Presses or releases the primary button at `pos`.
fn primary(harness: &mut Harness<'_, RickUi>, pos: egui::Pos2, pressed: bool) {
    harness.event(egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    });
    harness.step();
}

/// Issue 80, rules 7 and 8, as frame sequences in Light and Dark at both
/// densities. A pane: the open hand over its grip; grabbed, its ghost — the
/// pane's own snapshot at a quarter — with its grip 8 px off the closed
/// hand; over the other pane with the dock cross; over the tab row with
/// the slot of a new tab. A tab: the open hand over it; riding the row,
/// lifted with a shadow; off the row, following the closed hand.
#[test]
#[ignore = "needs a wgpu adapter; CI runs it and uploads the images"]
fn frames_of_the_drag_ghost_and_the_cursors() {
    use egui_kittest::kittest::Queryable as _;
    let (_dir, deep) = sample_tree();
    // Something to see in the pane that is dragged.
    fs::create_dir_all(deep.join("raw/contact-sheets")).unwrap_or_else(|e| panic!("{e}"));
    for i in 0..9u32 {
        fs::write(
            deep.join(format!("raw/DSC_{:04}.nef", 4310 + i)),
            vec![0u8; 2048 * (i as usize + 3)],
        )
        .unwrap_or_else(|e| panic!("{e}"));
    }
    let mut harness = shell_with_hands(&deep, egui::vec2(1360.0, 800.0));
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
    // Two more workspaces, and the first one shown again.
    let parent = deep
        .parent()
        .unwrap_or_else(|| panic!("parent"))
        .to_path_buf();
    for folder in [parent, deep.join("edited")] {
        harness
            .state_mut()
            .app_mut()
            .handle(app::Command::NewTab(local(&folder)), &mut Vec::new());
        settle(&mut harness);
    }
    let model = harness.state().app().model();
    let first = model.workspaces(model.focused_window)[0].id;
    harness
        .state_mut()
        .app_mut()
        .handle(app::Command::ActivateWorkspace(first), &mut Vec::new());
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
        let mut frames = Frames::new(format!("issue80-ghost-{theme_name}-{density_name}"));
        let rects = harness.state().pane_rects();
        let left_rect = rects
            .iter()
            .find(|(p, _)| *p == pane)
            .map_or_else(|| panic!("left pane"), |(_, r)| *r);
        let right_rect = rects
            .iter()
            .find(|(p, _)| *p == second)
            .map_or_else(|| panic!("right pane"), |(_, r)| *r);
        let row = harness
            .state()
            .tab_row()
            .unwrap_or_else(|| panic!("no tab row"));

        // The pane, by its grip.
        let grip = harness
            .query_all_by_label("Drag pane")
            .map(|n| n.rect())
            .find(|r| right_rect.contains(r.center()))
            .unwrap_or_else(|| panic!("handle of the right pane"))
            .center();
        glide(&mut harness, grip + egui::vec2(-80.0, 60.0), grip, 4);
        frames.shoot(&mut harness, "pane-grip-open-hand");
        primary(&mut harness, grip, true);
        let near = grip + egui::vec2(-60.0, 90.0);
        glide(&mut harness, grip, near, 6);
        frames.shoot(&mut harness, "pane-grabbed");
        let tile = left_rect.center() + egui::vec2(-ui_egui::DOCK_TILE_PITCH, 0.0);
        glide(&mut harness, near, tile, 10);
        frames.shoot(&mut harness, "pane-over-a-dock-target");
        let on_row = egui::pos2(row.left() + 2.0 * 222.0 + 40.0, row.center().y);
        glide(&mut harness, tile, on_row, 10);
        frames.shoot(&mut harness, "pane-over-the-tab-row");
        // Back to where it was: released beside every target.
        let away = egui::pos2(right_rect.center().x, right_rect.bottom() - 40.0);
        glide(&mut harness, on_row, away, 6);
        primary(&mut harness, away, false);
        settle(&mut harness);

        // The middle tab, by its body: a little along the row, where the
        // row still keeps its own place for it, then off the row.
        let tab = ["edited", "holiday-2026"]
            .iter()
            .filter_map(|title| {
                harness
                    .query_all_by_label(title)
                    .map(|n| n.rect())
                    .find(|r| row.contains(r.center()))
            })
            .min_by(|a, b| a.left().total_cmp(&b.left()))
            .unwrap_or_else(|| panic!("the middle tab"));
        let hold = tab.center() + egui::vec2(-20.0, 0.0);
        glide(&mut harness, away, hold, 6);
        frames.shoot(&mut harness, "tab-open-hand");
        primary(&mut harness, hold, true);
        let along = hold + egui::vec2(-70.0, 0.0);
        glide(&mut harness, hold, along, 8);
        frames.shoot(&mut harness, "tab-riding-the-row");
        let off = egui::pos2(along.x + 180.0, row.bottom() + 170.0);
        glide(&mut harness, along, off, 8);
        frames.shoot(&mut harness, "tab-off-the-row");
        primary(&mut harness, off, false);
        settle(&mut harness);
        frames.sheet();
    }
}

/// Issue 80, rule 1, as a film strip of the title row in Light and Dark at
/// both densities, a sixtieth of a second per step: the first of three
/// tabs taken by its middle; pulled along the row, lifted; past its
/// neighbour's middle, the neighbour on its way to the place the tab came
/// from, in three frames of the slide; arrived, the gap open where the tab
/// will land and no insertion line anywhere; at the last place of the row;
/// let go a little short of it, settling; settled, in the new order.
#[test]
#[ignore = "needs a wgpu adapter; CI runs it and uploads the images"]
fn frames_of_a_tab_dragged_along_the_row() {
    use egui_kittest::kittest::Queryable as _;
    let (_dir, deep) = sample_tree();
    for (theme, density) in [
        (Theme::Light, Density::Comfortable),
        (Theme::Light, Density::Compact),
        (Theme::Dark, Density::Comfortable),
        (Theme::Dark, Density::Compact),
    ] {
        // A window of its own for each: the drag ends with the tabs in
        // another order.
        let mut harness = shell_with_hands_at(&deep, egui::vec2(1360.0, 800.0), 1.0 / 60.0);
        let parent = deep
            .parent()
            .unwrap_or_else(|| panic!("parent"))
            .to_path_buf();
        for folder in [parent, deep.join("raw")] {
            harness
                .state_mut()
                .app_mut()
                .handle(app::Command::NewTab(local(&folder)), &mut Vec::new());
            settle(&mut harness);
        }
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
        let row = harness
            .state()
            .tab_row()
            .unwrap_or_else(|| panic!("no tab row"));
        // The title row and the top of the panes, in pixels at 2×.
        let rows = ((row.bottom() + 60.0) * 2.0) as u32;
        let mut frames =
            Frames::strip(format!("issue80-reorder-{theme_name}-{density_name}"), rows);
        let tab = |harness: &Harness<'_, RickUi>, title: &str| {
            harness
                .query_all_by_label(title)
                .map(|n| n.rect())
                .find(|r| row.contains(r.center()))
                .unwrap_or_else(|| panic!("no tab {title}"))
        };
        let first = tab(&harness, "day-one");
        let pitch = tab(&harness, "holiday-2026").left() - first.left();
        let grab = first.center();
        let at = |travelled: f32| grab + egui::vec2(travelled, 0.0);
        let still = |harness: &mut Harness<'_, RickUi>| {
            for _ in 0..12 {
                harness.step();
            }
        };

        glide(&mut harness, at(-30.0) + egui::vec2(0.0, 90.0), grab, 6);
        frames.shoot(&mut harness, "open-hand");
        primary(&mut harness, grab, true);
        glide(&mut harness, grab, at(70.0), 8);
        still(&mut harness);
        frames.shoot(&mut harness, "lifted");
        // Past the neighbour's middle: its slide, frame by frame.
        glide(&mut harness, at(70.0), at(pitch / 2.0 - 6.0), 6);
        still(&mut harness);
        harness.event(egui::Event::PointerMoved(at(pitch / 2.0 + 8.0)));
        harness.step();
        harness.step();
        frames.shoot(&mut harness, "neighbour-slides-1");
        harness.step();
        harness.step();
        frames.shoot(&mut harness, "neighbour-slides-2");
        harness.step();
        harness.step();
        harness.step();
        frames.shoot(&mut harness, "neighbour-slides-3");
        still(&mut harness);
        frames.shoot(&mut harness, "gap-open");
        glide(
            &mut harness,
            at(pitch / 2.0 + 8.0),
            at(2.0 * pitch - 50.0),
            20,
        );
        still(&mut harness);
        frames.shoot(&mut harness, "last-place");
        primary(&mut harness, at(2.0 * pitch - 50.0), false);
        harness.step();
        harness.step();
        frames.shoot(&mut harness, "settling");
        still(&mut harness);
        settle(&mut harness);
        frames.shoot(&mut harness, "settled");
        frames.sheet();
    }
}

/// Issue 80, rules 2 and 3, as frame sequences in Light and Dark at both
/// densities: the open hand over a tab that holds one pane; the tab
/// dragged down into the shown workspace, the four side targets on the
/// pane under the cursor; on the left target, lit, with the preview of the
/// half; dropped, its pane docked there and the tab gone from the row; and
/// a tab that holds two panes dragged over the same pane, offered nothing.
#[test]
#[ignore = "needs a wgpu adapter; CI runs it and uploads the images"]
fn frames_of_a_tab_docked_as_a_pane() {
    use egui_kittest::kittest::Queryable as _;
    let (_dir, deep) = sample_tree();
    fs::create_dir_all(deep.join("raw/contact-sheets")).unwrap_or_else(|e| panic!("{e}"));
    for i in 0..9u32 {
        fs::write(
            deep.join(format!("raw/DSC_{:04}.nef", 4310 + i)),
            vec![0u8; 2048 * (i as usize + 3)],
        )
        .unwrap_or_else(|e| panic!("{e}"));
    }
    for (theme, density) in [
        (Theme::Light, Density::Comfortable),
        (Theme::Light, Density::Compact),
        (Theme::Dark, Density::Comfortable),
        (Theme::Dark, Density::Compact),
    ] {
        // A window of its own for each: the drop changes the layout.
        let mut harness = shell_with_hands(&deep, egui::vec2(1360.0, 800.0));
        let shown_pane = harness.state().app().model().focused;
        // A tab of one pane, then a tab of two.
        harness.state_mut().app_mut().handle(
            app::Command::NewTab(local(&deep.join("raw"))),
            &mut Vec::new(),
        );
        settle(&mut harness);
        let parent = deep
            .parent()
            .unwrap_or_else(|| panic!("parent"))
            .to_path_buf();
        harness
            .state_mut()
            .app_mut()
            .handle(app::Command::NewTab(local(&parent)), &mut Vec::new());
        settle(&mut harness);
        let pane = harness.state().app().model().focused;
        harness.state_mut().app_mut().handle(
            app::Command::Split {
                pane,
                dir: model::SplitDir::Right,
            },
            &mut Vec::new(),
        );
        settle(&mut harness);
        harness
            .state_mut()
            .app_mut()
            .handle(app::Command::FocusPane(shown_pane), &mut Vec::new());
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
        let mut frames = Frames::new(format!("issue80-tab-to-pane-{theme_name}-{density_name}"));
        let row = harness
            .state()
            .tab_row()
            .unwrap_or_else(|| panic!("no tab row"));
        let tab = |harness: &Harness<'_, RickUi>, title: &str| {
            harness
                .query_all_by_label(title)
                .map(|n| n.rect())
                .find(|r| row.contains(r.center()))
                .unwrap_or_else(|| panic!("no tab {title}"))
                .center()
        };
        let pane_rect = harness
            .state()
            .pane_rects()
            .into_iter()
            .find(|(p, _)| *p == shown_pane)
            .map_or_else(|| panic!("the shown pane"), |(_, r)| r);
        let tile = pane_rect.center() + egui::vec2(-ui_egui::DOCK_TILE_PITCH, 0.0);
        let beside = pane_rect.center() + egui::vec2(150.0, 120.0);

        // The tab of two panes first, while the layout is still one pane:
        // nothing is offered.
        let two = tab(&harness, "holiday-2026");
        glide(&mut harness, two + egui::vec2(0.0, 120.0), two, 4);
        primary(&mut harness, two, true);
        glide(&mut harness, two, tile, 12);
        let nothing = harness
            .render()
            .unwrap_or_else(|e| panic!("no renderer for screenshots: {e}"));
        let away = egui::pos2(pane_rect.center().x, pane_rect.bottom() - 60.0);
        glide(&mut harness, tile, away, 4);
        primary(&mut harness, away, false);
        settle(&mut harness);

        // The tab of one pane.
        let one = tab(&harness, "raw");
        glide(&mut harness, away, one, 6);
        frames.shoot(&mut harness, "open-hand");
        primary(&mut harness, one, true);
        glide(&mut harness, one, beside, 12);
        frames.shoot(&mut harness, "four-targets");
        glide(&mut harness, beside, tile, 8);
        frames.shoot(&mut harness, "left-target");
        primary(&mut harness, tile, false);
        settle(&mut harness);
        frames.shoot(&mut harness, "docked");
        // The frame taken first closes the sequence.
        frames.frames.push(nothing);
        frames.sheet();
    }
}

/// A window for the spring-loading and Escape sequences: the first
/// workspace holds two panes, `day-one` and `raw`, the second `edited`,
/// the third `holiday-2026`; the first is shown, and tabs spring after
/// half a second. Returns the window with the two panes of the first
/// workspace and the pane of the second.
fn three_workspaces(
    deep: &Path,
    theme: Theme,
    density: Density,
) -> (
    Harness<'static, RickUi>,
    model::PaneId,
    model::PaneId,
    model::PaneId,
) {
    let mut harness = shell_with_hands(deep, egui::vec2(1360.0, 800.0));
    let first = harness.state().app().model().focused;
    let run = |harness: &mut Harness<'static, RickUi>, command: app::Command| {
        harness
            .state_mut()
            .app_mut()
            .handle(command, &mut Vec::new());
        settle(harness);
    };
    run(
        &mut harness,
        app::Command::Split {
            pane: first,
            dir: model::SplitDir::Right,
        },
    );
    let second = harness.state().app().model().focused;
    run(
        &mut harness,
        app::Command::Navigate {
            pane: second,
            to: local(&deep.join("raw")),
        },
    );
    run(
        &mut harness,
        app::Command::NewTab(local(&deep.join("edited"))),
    );
    let edited = harness.state().app().model().focused;
    let parent = deep
        .parent()
        .unwrap_or_else(|| panic!("parent"))
        .to_path_buf();
    run(&mut harness, app::Command::NewTab(local(&parent)));
    let model = harness.state().app().model();
    let shown = model.workspaces(model.focused_window)[0].id;
    run(&mut harness, app::Command::ActivateWorkspace(shown));
    run(&mut harness, app::Command::FocusPane(first));
    for change in [
        SettingsChange::Theme(theme),
        SettingsChange::Density(density),
        SettingsChange::SpringDelay(500),
    ] {
        run(&mut harness, app::Command::Settings(change));
    }
    (harness, first, second, edited)
}

/// The name of a scheme and a density in a file name.
fn look(theme: Theme, density: Density) -> String {
    let theme = match theme {
        Theme::Light => "light",
        Theme::Dark => "dark",
        Theme::System => "system",
    };
    let density = match density {
        Density::Comfortable => "comfortable",
        Density::Compact => "compact",
    };
    format!("{theme}-{density}")
}

/// The four looks every sequence is rendered in.
const LOOKS: [(Theme, Density); 4] = [
    (Theme::Light, Density::Comfortable),
    (Theme::Light, Density::Compact),
    (Theme::Dark, Density::Comfortable),
    (Theme::Dark, Density::Compact),
];

/// The rectangle of the workspace tab titled `title` in the title row.
fn tab_in_row(harness: &Harness<'_, RickUi>, title: &str) -> egui::Rect {
    use egui_kittest::kittest::Queryable as _;
    let row = harness
        .state()
        .tab_row()
        .unwrap_or_else(|| panic!("no tab row"));
    harness
        .query_all_by_label(title)
        .map(|n| n.rect())
        .find(|r| row.contains(r.center()))
        .unwrap_or_else(|| panic!("no tab {title}"))
}

/// The rectangle of `pane` in the last frame.
fn rect_of_pane(harness: &Harness<'_, RickUi>, pane: model::PaneId) -> egui::Rect {
    harness
        .state()
        .pane_rects()
        .into_iter()
        .find(|(p, _)| *p == pane)
        .map_or_else(|| panic!("no rect for {pane:?}"), |(_, r)| r)
}

/// The centre of the grab handle of `pane`.
fn grip_of_pane(harness: &Harness<'_, RickUi>, pane: model::PaneId) -> egui::Pos2 {
    use egui_kittest::kittest::Queryable as _;
    let rect = rect_of_pane(harness, pane);
    harness
        .query_all_by_label("Drag pane")
        .map(|n| n.rect())
        .find(|r| rect.contains(r.center()))
        .map_or_else(|| panic!("no handle in {pane:?}"), |r| r.center())
}

/// Runs `frames` frames with the pointer where it is, a quarter of a
/// second each.
fn hold_still(harness: &mut Harness<'_, RickUi>, frames: usize) {
    for _ in 0..frames {
        harness.step();
    }
}

/// Issue 80, rules 4 and 5, as frame sequences in Light and Dark at both
/// densities. A pane held on another workspace's tab, before the delay
/// and after it, the window then showing that workspace; the pane on a
/// dock target there; docked. A tab of one pane leaning on its neighbour,
/// before and after. A file held on a tab, before and after, the drag
/// then over a folder of the workspace that opened.
#[test]
#[ignore = "needs a wgpu adapter; CI runs it and uploads the images"]
fn frames_of_spring_loaded_tabs() {
    use egui_kittest::kittest::Queryable as _;
    let (_dir, deep) = sample_tree();
    fs::create_dir_all(deep.join("edited/print")).unwrap_or_else(|e| panic!("{e}"));
    for i in 0..5u32 {
        fs::write(
            deep.join(format!("edited/IMG_{:04}-final.png", 1200 + i)),
            vec![0u8; 4096],
        )
        .unwrap_or_else(|e| panic!("{e}"));
    }
    for (theme, density) in LOOKS {
        let (mut harness, first, second, edited) = three_workspaces(&deep, theme, density);
        let mut frames = Frames::new(format!("issue80-spring-{}", look(theme, density)));

        // The second pane of the shown workspace, held on the tab of
        // `edited`: on its right half, where the slot of a new tab opens
        // after it.
        let grip = grip_of_pane(&harness, second);
        let on_tab = tab_in_row(&harness, "edited").center() + egui::vec2(60.0, 0.0);
        glide(&mut harness, grip + egui::vec2(-40.0, 60.0), grip, 3);
        primary(&mut harness, grip, true);
        glide(&mut harness, grip, on_tab, 10);
        frames.shoot(&mut harness, "pane-rests-on-a-tab");
        hold_still(&mut harness, 3);
        settle(&mut harness);
        frames.shoot(&mut harness, "its-workspace-opens");
        let tile =
            rect_of_pane(&harness, edited).center() + egui::vec2(-ui_egui::DOCK_TILE_PITCH, 0.0);
        glide(&mut harness, on_tab, tile, 10);
        frames.shoot(&mut harness, "pane-on-a-dock-target-there");
        primary(&mut harness, tile, false);
        settle(&mut harness);
        frames.shoot(&mut harness, "docked");

        // The tab of one pane, `holiday-2026`, leaning on its neighbour.
        let model = harness.state().app().model();
        let shown = model.workspaces(model.focused_window)[0].id;
        harness
            .state_mut()
            .app_mut()
            .handle(app::Command::ActivateWorkspace(shown), &mut Vec::new());
        settle(&mut harness);
        let tab = tab_in_row(&harness, "holiday-2026");
        let neighbour = ["edited", "raw"]
            .iter()
            .find_map(|title| {
                let row = harness.state().tab_row()?;
                harness
                    .query_all_by_label(title)
                    .map(|n| n.rect())
                    .find(|r| row.contains(r.center()) && r.left() < tab.left() - 1.0)
            })
            .unwrap_or_else(|| panic!("the neighbour of the last tab"));
        let pitch = tab.left() - neighbour.left();
        let hold = tab.center();
        let lean = hold - egui::vec2(pitch * 0.35, 0.0);
        glide(&mut harness, tile, hold, 6);
        primary(&mut harness, hold, true);
        glide(&mut harness, hold, lean, 6);
        frames.shoot(&mut harness, "tab-leans-on-its-neighbour");
        hold_still(&mut harness, 3);
        settle(&mut harness);
        frames.shoot(&mut harness, "the-neighbours-workspace-opens");
        // Back to its place: let go off the row, on no target.
        let away = egui::pos2(lean.x, 700.0);
        glide(&mut harness, lean, away, 6);
        primary(&mut harness, away, false);
        settle(&mut harness);

        // A file of the shown workspace, held on the tab of the one-pane
        // workspace, then over a folder there.
        let file = harness
            .query_all_by_label("notes.md")
            .map(|n| n.rect())
            .find(|r| rect_of_pane(&harness, first).contains(r.center()))
            .unwrap_or_else(|| panic!("notes.md in the first pane"))
            .center();
        let on_tab = tab_in_row(&harness, "holiday-2026").center();
        glide(&mut harness, away, file, 6);
        primary(&mut harness, file, true);
        glide(&mut harness, file, on_tab, 10);
        frames.shoot(&mut harness, "file-rests-on-a-tab");
        hold_still(&mut harness, 3);
        settle(&mut harness);
        let folder = harness
            .query_all_by_label("day-one")
            .map(|n| n.rect())
            .find(|r| r.top() > on_tab.y + 60.0)
            .unwrap_or_else(|| panic!("the folder row in the workspace that opened"))
            .center();
        glide(&mut harness, on_tab, folder, 8);
        frames.shoot(&mut harness, "file-over-a-folder-there");
        harness.key_press(egui::Key::Escape);
        harness.step();
        primary(&mut harness, folder, false);
        settle(&mut harness);
        frames.sheet();
    }
}

/// Issue 80, rule 9, as frame sequences in Light and Dark at both
/// densities: a tab dragged past its neighbour, and the row after Escape;
/// a pane that spring-loading took to another workspace, on a dock target
/// there, and the window after Escape, on the workspace the drag began in
/// with its layout as it was.
#[test]
#[ignore = "needs a wgpu adapter; CI runs it and uploads the images"]
fn frames_of_escape_cancelling_a_drag() {
    let (_dir, deep) = sample_tree();
    for (theme, density) in LOOKS {
        let (mut harness, _first, second, edited) = three_workspaces(&deep, theme, density);
        let mut frames = Frames::new(format!("issue80-escape-{}", look(theme, density)));

        // A tab taken past its neighbour.
        let tab = tab_in_row(&harness, "edited");
        let pitch = tab_in_row(&harness, "holiday-2026").left() - tab.left();
        let hold = tab.center();
        let past = hold + egui::vec2(pitch * 0.8, 0.0);
        glide(&mut harness, hold + egui::vec2(0.0, 80.0), hold, 3);
        primary(&mut harness, hold, true);
        glide(&mut harness, hold, past, 8);
        hold_still(&mut harness, 2);
        frames.shoot(&mut harness, "tab-past-its-neighbour");
        harness.key_press(egui::Key::Escape);
        harness.step();
        hold_still(&mut harness, 3);
        frames.shoot(&mut harness, "escape-the-row-as-it-was");
        primary(&mut harness, past, false);
        settle(&mut harness);

        // A pane that spring-loading took to the workspace of `edited`.
        let grip = grip_of_pane(&harness, second);
        let on_tab = tab_in_row(&harness, "edited").center() + egui::vec2(60.0, 0.0);
        glide(&mut harness, past, grip, 6);
        primary(&mut harness, grip, true);
        glide(&mut harness, grip, on_tab, 10);
        hold_still(&mut harness, 3);
        settle(&mut harness);
        let tile =
            rect_of_pane(&harness, edited).center() + egui::vec2(-ui_egui::DOCK_TILE_PITCH, 0.0);
        glide(&mut harness, on_tab, tile, 10);
        frames.shoot(&mut harness, "pane-on-a-target-of-the-sprung-workspace");
        harness.key_press(egui::Key::Escape);
        harness.step();
        hold_still(&mut harness, 2);
        frames.shoot(&mut harness, "escape-the-layout-as-it-was");
        primary(&mut harness, tile, false);
        settle(&mut harness);
        frames.sheet();
    }
}

/// Issue 80, rule 6, as frame sequences in Light and Dark at both
/// densities: a tab of one pane dragged from the first window onto the
/// second window's tab row, then on over its pane and dropped on a dock
/// target there; and a pane dragged by its handle over the second
/// window's pane, then over its tab row and dropped. The harness draws
/// the second window inside the first; on a desktop they are two windows.
#[test]
#[ignore = "needs a wgpu adapter; CI runs it and uploads the images"]
fn frames_of_a_drag_between_two_windows() {
    use egui_kittest::kittest::Queryable as _;
    let (_dir, deep) = sample_tree();
    let parent = deep
        .parent()
        .unwrap_or_else(|| panic!("parent"))
        .to_path_buf();
    for (theme, density) in LOOKS {
        let mut harness = shell_with_hands(&deep, egui::vec2(1800.0, 1000.0));
        let run = |harness: &mut Harness<'static, RickUi>, command: app::Command| {
            harness
                .state_mut()
                .app_mut()
                .handle(command, &mut Vec::new());
            settle(harness);
        };
        for change in [
            SettingsChange::Theme(theme),
            SettingsChange::Density(density),
        ] {
            run(&mut harness, app::Command::Settings(change));
        }
        // The first window: `day-one` beside `raw`, and `edited` in a tab
        // of its own. The second: `holiday-2026`.
        let first = harness.state().app().model().focused_window;
        let day_one = harness.state().app().model().focused;
        run(
            &mut harness,
            app::Command::Split {
                pane: day_one,
                dir: model::SplitDir::Right,
            },
        );
        let raw = harness.state().app().model().focused;
        run(
            &mut harness,
            app::Command::Navigate {
                pane: raw,
                to: local(&deep.join("raw")),
            },
        );
        run(
            &mut harness,
            app::Command::NewTab(local(&deep.join("edited"))),
        );
        let shown = harness.state().app().model().workspaces(first)[0].id;
        run(&mut harness, app::Command::ActivateWorkspace(shown));
        run(&mut harness, app::Command::NewWindow);
        let second = harness.state().app().model().focused_window;
        let holiday = harness.state().app().model().focused;
        run(
            &mut harness,
            app::Command::Navigate {
                pane: holiday,
                to: local(&parent),
            },
        );
        let mut frames = Frames::new(format!("issue80-windows-{}", look(theme, density)));
        let row = |harness: &Harness<'_, RickUi>| {
            harness
                .state()
                .tab_row_of(second)
                .unwrap_or_else(|| panic!("the second window's tab row"))
        };
        let pane_there = |harness: &Harness<'_, RickUi>, pane: model::PaneId| {
            harness
                .state()
                .pane_rects_of(second)
                .into_iter()
                .find(|(p, _)| *p == pane)
                .map_or_else(|| panic!("no rect for {pane:?}"), |(_, r)| r)
        };

        // The tab of `edited`, taken in the top of its row, which the
        // second window leaves free.
        let tab = tab_in_row(&harness, "edited");
        let hold = tab.center_top() + egui::vec2(0.0, 5.0);
        let grab = hold - tab.min;
        let there = row(&harness);
        let in_row = egui::pos2(there.left() + grab.x, there.center().y);
        glide(&mut harness, hold + egui::vec2(60.0, 0.0), hold, 3);
        primary(&mut harness, hold, true);
        glide(&mut harness, hold, in_row, 10);
        hold_still(&mut harness, 2);
        frames.shoot(&mut harness, "tab-in-the-other-windows-row");
        let tile =
            pane_there(&harness, holiday).center() + egui::vec2(ui_egui::DOCK_TILE_PITCH, 0.0);
        glide(&mut harness, in_row, tile, 10);
        hold_still(&mut harness, 2);
        frames.shoot(&mut harness, "tab-on-a-target-of-the-other-windows-pane");
        primary(&mut harness, tile, false);
        settle(&mut harness);
        frames.shoot(&mut harness, "docked-in-the-other-window");

        // The pane of `raw`, by its handle.
        let rect = harness
            .state()
            .pane_rects_of(first)
            .into_iter()
            .find(|(p, _)| *p == raw)
            .map_or_else(|| panic!("no rect for {raw:?}"), |(_, r)| r);
        let grip = harness
            .query_all_by_label("Drag pane")
            .map(|n| n.rect())
            .find(|r| rect.contains(r.center()))
            .map_or_else(|| panic!("no handle"), |r| r.center());
        let tile =
            pane_there(&harness, holiday).center() + egui::vec2(-ui_egui::DOCK_TILE_PITCH, 0.0);
        glide(&mut harness, tile, grip, 4);
        primary(&mut harness, grip, true);
        glide(&mut harness, grip, tile, 12);
        hold_still(&mut harness, 2);
        frames.shoot(&mut harness, "pane-over-the-other-windows-pane");
        let there = row(&harness);
        let slot = egui::pos2(
            there.left() + ui_egui::theme::TAB_WIDTH + 2.0,
            there.center().y,
        );
        glide(&mut harness, tile, slot, 10);
        hold_still(&mut harness, 2);
        frames.shoot(&mut harness, "pane-over-the-other-windows-row");
        primary(&mut harness, slot, false);
        settle(&mut harness);
        frames.shoot(&mut harness, "a-new-tab-in-the-other-window");
        frames.sheet();
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
        .map_or_default(|(_, t)| {
            t.rows()
                .filter(|e| e.name == "IMG_1201.png" || e.name == "IMG_1202.png")
                .map(|e| e.id)
                .collect()
        });
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
        .map_or_else(|| fake.config_dir(), std::path::Path::to_path_buf);
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

/// Issue 139, item 2: the application menu under its button — the three
/// groups, an icon on every entry, the shortcut hints, the submenu carets
/// and the version beside About Rick — with the pointer on New window as
/// the Figma frame has it (`138:4031`, dark `138:4115`), in Light and Dark
/// at both densities. The installer is loaded where it runs, for its
/// submenu.
#[test]
#[ignore = "needs a wgpu adapter; CI runs it and uploads the images"]
fn screenshots_of_the_app_menu() {
    use egui_kittest::kittest::Queryable as _;
    let (_dir, deep) = sample_tree();
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: vec![
            Arc::new(ext_preview_text::TextPreview::new()),
            Arc::new(ext_preview_image::ImagePreview::new()),
            Arc::new(ext_installer::InstallerExtension::new()),
        ],
        config_dir: Some(fake.config_dir()),
        start: Some(local(&deep)),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 720.0))
        .with_pixels_per_point(2.0)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
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
        harness.get_by_label("Menu").click();
        harness.step();
        harness.step();
        let first = harness.get_by_label("New window").rect().center();
        harness.event(egui::Event::PointerMoved(first));
        harness.step();
        harness.step();
        shoot(
            &mut harness,
            &format!("app-menu-{theme_name}-{density_name}"),
        );
        // A submenu is a card of the same form: Layouts, opened by hover.
        let layouts = harness.get_by_label_contains("Layouts").rect().center();
        harness.event(egui::Event::PointerMoved(layouts));
        for _ in 0..4 {
            harness.step();
        }
        shoot(
            &mut harness,
            &format!("app-menu-layouts-{theme_name}-{density_name}"),
        );
        harness.key_press(egui::Key::Escape);
        harness.step();
        harness.step();
    }
}

/// Issue 139, item 3: the paste-conflict dialogue as the Figma frames
/// have it (`141:4082`) — one conflict; the third of twelve with the
/// checkbox; and two folders, which the frames do not draw — in Light and
/// Dark at both densities. The files are sparse, with the frames' sizes
/// and dates.
#[test]
#[ignore = "needs a wgpu adapter; CI runs it and uploads the images"]
#[allow(clippy::too_many_lines)]
fn screenshots_of_the_paste_conflict_dialogue() {
    use egui_kittest::kittest::Queryable as _;
    use std::time::SystemTime;
    const MIB: f64 = 1024.0 * 1024.0;
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let (from, into) = (dir.path().join("from"), dir.path().join("test"));
    fs::create_dir_all(from.join("Holiday")).unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir_all(into.join("Holiday")).unwrap_or_else(|e| panic!("{e}"));
    let long = "Camera white appartment 5-14-2026 1.32.41pm GMT2 - 5-14-2026 1.33.08pm GMT2.mp4";
    // A sparse file of `mib` MiB, modified `ago` before now.
    let sparse = |path: &Path, mib: f64, ago: Duration| {
        let file = fs::File::create(path).unwrap_or_else(|e| panic!("{e}"));
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        file.set_len((mib * MIB) as u64)
            .unwrap_or_else(|e| panic!("{e}"));
        file.set_modified(SystemTime::now() - ago)
            .unwrap_or_else(|e| panic!("{e}"));
    };
    let mut names = vec![
        "Alpha.txt".to_owned(),
        "Beta.txt".to_owned(),
        long.to_owned(),
    ];
    names.extend((1..=9).map(|n| format!("Notes {n:02}.txt")));
    for name in &names {
        let (old, new) = if name == long {
            (48.2, 51.7)
        } else {
            (0.1, 0.2)
        };
        sparse(&into.join(name), old, Duration::from_hours(26));
        sparse(&from.join(name), new, Duration::from_hours(2));
    }
    let mut harness = shell(&from, egui::vec2(1280.0, 800.0));
    let run = |harness: &mut Harness<'_, RickUi>, action: &'static str| {
        let pane = harness.state().app().model().focused;
        harness
            .state_mut()
            .queue(Intent::Action(ext::ActionId(action)), pane);
        harness.step();
        harness.step();
    };
    // Copies the rows named by `pick` in `from` and pastes them into
    // `into`, then waits for the dialogue.
    let paste = |harness: &mut Harness<'_, RickUi>, pick: &dyn Fn(&str) -> bool| {
        let pane = harness.state().app().model().focused;
        harness.state_mut().app_mut().handle(
            app::Command::Navigate {
                pane,
                to: local(&from),
            },
            &mut Vec::new(),
        );
        settle(harness);
        let ids: Vec<types::EntryId> = harness
            .state()
            .app()
            .model()
            .active_tab(pane)
            .map_or_else(|| panic!("no tab"), |(_, tab)| tab.clone())
            .rows()
            .filter(|e| pick(&e.name))
            .map(|e| e.id)
            .collect();
        harness.state_mut().app_mut().handle(
            app::Command::Select {
                pane,
                change: SelectionChange::Set(ids),
            },
            &mut Vec::new(),
        );
        settle(harness);
        run(harness, "core.copy");
        harness.state_mut().app_mut().handle(
            app::Command::Navigate {
                pane,
                to: local(&into),
            },
            &mut Vec::new(),
        );
        settle(harness);
        run(harness, "core.paste");
        let deadline = Instant::now() + Duration::from_secs(10);
        while harness.state().app().model().conflicts.is_empty() {
            assert!(Instant::now() < deadline, "no conflict dialogue");
            harness.step();
            std::thread::sleep(Duration::from_millis(5));
        }
        harness.step();
        harness.step();
    };
    let cancel = |harness: &mut Harness<'_, RickUi>| {
        harness.get_by_label("Cancel").click();
        harness.step();
        settle(harness);
    };
    let modes = [
        (Theme::Light, Density::Comfortable, "light-comfortable"),
        (Theme::Light, Density::Compact, "light-compact"),
        (Theme::Dark, Density::Comfortable, "dark-comfortable"),
        (Theme::Dark, Density::Compact, "dark-compact"),
    ];
    let set = |harness: &mut Harness<'_, RickUi>, theme: Theme, density: Density| {
        for change in [
            SettingsChange::Theme(theme),
            SettingsChange::Density(density),
        ] {
            harness
                .state_mut()
                .app_mut()
                .handle(app::Command::Settings(change), &mut Vec::new());
        }
        // The pointer rests outside the dialogue, on no button.
        harness.event(egui::Event::PointerMoved(egui::pos2(40.0, 780.0)));
        for _ in 0..4 {
            harness.step();
        }
    };

    // One conflict: no count, no checkbox.
    paste(&mut harness, &|name| name == long);
    for (theme, density, mode) in modes {
        set(&mut harness, theme, density);
        shoot(&mut harness, &format!("paste-conflict-one-{mode}"));
    }
    cancel(&mut harness);

    // The third of twelve, with the checkbox.
    paste(&mut harness, &|name| name != "Holiday");
    for _ in 0..2 {
        harness.get_by_label("Skip").click();
        harness.step();
        let deadline = Instant::now() + Duration::from_secs(10);
        while harness.state().app().model().conflicts.is_empty() {
            assert!(Instant::now() < deadline, "no further conflict");
            harness.step();
            std::thread::sleep(Duration::from_millis(5));
        }
        harness.step();
        harness.step();
    }
    for (theme, density, mode) in modes {
        set(&mut harness, theme, density);
        shoot(&mut harness, &format!("paste-conflict-several-{mode}"));
    }
    cancel(&mut harness);

    // Two folders: Merge, and no Keep both.
    paste(&mut harness, &|name| name == "Holiday");
    for (theme, density, mode) in modes {
        set(&mut harness, theme, density);
        shoot(&mut harness, &format!("paste-conflict-folders-{mode}"));
    }
    cancel(&mut harness);
}

/// Issue 139, item 5: the preview pane as the Figma frames have it
/// (`149:4630`) — nothing selected; a photo with the File info card
/// hanging under the info button, and with it pinned; a file no previewer
/// claims; and a PDF on a machine without pdfium — in Light and Dark at
/// both densities, in a pane 600 px wide as the frames are.
#[test]
#[ignore = "needs a wgpu adapter; CI runs it and uploads the images"]
#[allow(clippy::too_many_lines)]
fn screenshots_of_the_file_info_card() {
    use egui_kittest::kittest::Queryable as _;
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let root = dir.path().join("16-binary-hex");
    fs::create_dir_all(&root).unwrap_or_else(|e| panic!("{e}"));
    let photo = "Camera white appartment 5-14-2026 1.32.41pm GMT2.jpg";
    fs::write(root.join(photo), media::photo_jpeg(1024, 768)).unwrap_or_else(|e| panic!("{e}"));
    let noise: Vec<u8> = (0..4096u32)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 13).to_le_bytes()[0])
        .collect();
    fs::write(root.join("random-4k.bin"), noise).unwrap_or_else(|e| panic!("{e}"));
    fs::write(
        root.join("hallway_chandeliers_sign_v2.pdf"),
        media::pdf(1, 595, 842),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    // A window whose PDF previewer looks in its setting alone, as on a
    // machine without pdfium, with the hex previewer loaded.
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: vec![
            Arc::new(ext_preview_text::TextPreview::new()),
            Arc::new(ext_preview_image::ImagePreview::new()),
            Arc::new(ext_preview_hex::HexPreview::new()),
            Arc::new(ext_preview_pdf::PdfPreview::with_search(
                ext_preview_pdf::Search::SettingOnly,
            )),
        ],
        config_dir: Some(fake.config_dir()),
        start: Some(local(&root)),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_pixels_per_point(2.0)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    harness.state_mut().set_inspector_width(600.0);
    let pane = harness.state().app().model().focused;
    harness.state_mut().queue(Intent::ToggleInspector, pane);
    settle(&mut harness);
    let modes = [
        (Theme::Light, Density::Comfortable, "light-comfortable"),
        (Theme::Light, Density::Compact, "light-compact"),
        (Theme::Dark, Density::Comfortable, "dark-comfortable"),
        (Theme::Dark, Density::Compact, "dark-compact"),
    ];
    let set = |harness: &mut Harness<'_, RickUi>, theme: Theme, density: Density| {
        for change in [
            SettingsChange::Theme(theme),
            SettingsChange::Density(density),
        ] {
            harness
                .state_mut()
                .app_mut()
                .handle(app::Command::Settings(change), &mut Vec::new());
        }
        // The pointer rests on nothing.
        harness.event(egui::Event::PointerMoved(egui::pos2(300.0, 780.0)));
        for _ in 0..4 {
            harness.step();
        }
    };

    // Nothing selected.
    for (theme, density, mode) in modes {
        set(&mut harness, theme, density);
        shoot(&mut harness, &format!("preview-empty-{mode}"));
    }
    eprintln!("the preview pane: {:?}", harness.state().inspector_rect());

    // A photo: the card hanging under the info button.
    preview(&mut harness, photo);
    for (theme, density, mode) in modes {
        set(&mut harness, theme, density);
        harness.get_by_label("File info").hover();
        for _ in 0..6 {
            harness.step();
        }
        shoot(&mut harness, &format!("preview-info-hover-{mode}"));
    }

    // The card pinned above the picture.
    harness.get_by_label("File info").click();
    settle(&mut harness);
    for (theme, density, mode) in modes {
        set(&mut harness, theme, density);
        shoot(&mut harness, &format!("preview-info-pinned-{mode}"));
    }
    harness.get_by_label("File info").click();
    settle(&mut harness);

    // A file no previewer claims, and a PDF without pdfium: the card
    // centred, with what can be done.
    for (name, scene) in [
        ("random-4k.bin", "preview-no-previewer"),
        ("hallway_chandeliers_sign_v2.pdf", "preview-missing-library"),
    ] {
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
        settle(&mut harness);
        for _ in 0..6 {
            harness.step();
        }
        for (theme, density, mode) in modes {
            set(&mut harness, theme, density);
            shoot(&mut harness, &format!("{scene}-{mode}"));
        }
    }
}

/// Issue 139, item 6: the tables of the preview as the Figma frame has
/// them (`150:4368`) — the archive tree and the CSV grid under the one
/// header band, striped from the second row — in Light and Dark at both
/// densities.
#[test]
#[ignore = "needs a wgpu adapter; CI runs it and uploads the images"]
fn screenshots_of_the_preview_tables() {
    use std::io::Write as _;
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let root = dir.path().join("tables");
    fs::create_dir_all(&root).unwrap_or_else(|e| panic!("{e}"));
    // The frame's archive: a folder, a folder inside it, three documents.
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for folder in [
        "Kreditkarte - Farbcodiert/",
        "Kreditkarte - Farbcodiert/2024/",
    ] {
        zip.add_directory(folder, options)
            .unwrap_or_else(|e| panic!("{e}"));
    }
    for (index, kib) in [150usize, 249, 135].into_iter().enumerate() {
        let name = format!(
            "Kreditkarte - Farbcodiert/2024/default_0{}_2024_annotated.pdf",
            index + 1
        );
        zip.start_file(name, options)
            .unwrap_or_else(|e| panic!("{e}"));
        // Text that deflates a little, as a document would.
        let body: Vec<u8> = (0..kib * 1024)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 11).to_le_bytes()[0] | 0x20)
            .collect();
        zip.write_all(&body).unwrap_or_else(|e| panic!("{e}"));
    }
    let bytes = zip.finish().unwrap_or_else(|e| panic!("{e}")).into_inner();
    fs::write(root.join("Kreditkarte.zip"), bytes).unwrap_or_else(|e| panic!("{e}"));
    fs::write(
        root.join("products.csv"),
        "id,title,description,price\n\
         1,\"Rick, the file manager\",\"Fast listing, \"\"god-like\"\"\",0.00\n\
         2,Multi-line field,First line of two,12.50\n\
         3,Trailing spaces  ,  leading spaces,7\n\
         4,Unquoted,plain,3\n",
    )
    .unwrap_or_else(|e| panic!("{e}"));

    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: vec![
            Arc::new(ext_preview_text::TextPreview::new()),
            Arc::new(ext_preview_markup::MarkupPreview::new()),
            Arc::new(ext_preview_archive::ArchivePreview::new()),
        ],
        config_dir: Some(fake.config_dir()),
        start: Some(local(&root)),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_pixels_per_point(2.0)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    harness.state_mut().set_inspector_width(600.0);
    let modes = [
        (Theme::Light, Density::Comfortable, "light-comfortable"),
        (Theme::Light, Density::Compact, "light-compact"),
        (Theme::Dark, Density::Comfortable, "dark-comfortable"),
        (Theme::Dark, Density::Compact, "dark-compact"),
    ];
    for (name, scene) in [
        ("Kreditkarte.zip", "preview-table-archive"),
        ("products.csv", "preview-table-csv"),
    ] {
        preview(&mut harness, name);
        settle(&mut harness);
        for (theme, density, mode) in modes {
            for change in [
                SettingsChange::Theme(theme),
                SettingsChange::Density(density),
            ] {
                harness
                    .state_mut()
                    .app_mut()
                    .handle(app::Command::Settings(change), &mut Vec::new());
            }
            // The pointer rests on nothing.
            harness.event(egui::Event::PointerMoved(egui::pos2(300.0, 780.0)));
            for _ in 0..4 {
                harness.step();
            }
            shoot(&mut harness, &format!("{scene}-{mode}"));
        }
    }
    eprintln!("the preview pane: {:?}", harness.state().inspector_rect());
}

/// The small items of issue 139 (item 11), in Light and Dark at both
/// densities: the window group with its wider steps beside a console's
/// tab titled *Console*; the Hash dialogue's secondary line cut before
/// the close button; and a failed job — the notice that stays, and the
/// drawer's row at rest and unfolded to the whole error.
#[test]
#[ignore = "needs a wgpu adapter; CI runs it and uploads the images"]
#[allow(clippy::too_many_lines)]
fn screenshots_of_the_small_items() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::{NodeT as _, Queryable as _};
    use ext::{
        API_VERSION, Action, ActionCtx, ActionGroup, ActionId, ActionOutcome, Availability,
        Capabilities, Extension, Manifest, Platforms, Points, Registry, Shortcut, UiHost, Version,
    };
    use types::Error;

    /// An action whose job fails with a long error.
    struct Sync;
    impl Action for Sync {
        fn id(&self) -> ActionId {
            ActionId("test.sync")
        }
        fn label(&self) -> String {
            "Sync photos".to_owned()
        }
        fn group(&self) -> ActionGroup {
            ActionGroup::Core
        }
        fn default_shortcut(&self) -> Option<Shortcut> {
            None
        }
        fn availability(&self, _: &ActionCtx) -> Availability {
            Availability::Enabled
        }
        fn run(
            &self,
            _ctx: ActionCtx,
            jobs: &runtime::JobRunner,
            _ui: &dyn UiHost,
        ) -> Result<ActionOutcome, Error> {
            let handle = jobs.spawn_named("Copy 14 items to archive", |_job| async move {
                Err::<(), Error>(Error::Io(std::io::Error::other(
                    "Invalid argument: the destination \
                     /mnt/archive/2026/photos/holiday/day-one/edited/IMG_1204-final-retouched.png \
                     could not be written, because the volume it is on was taken away while the \
                     copy was running",
                )))
            });
            Ok(ActionOutcome::Started(handle.id))
        }
    }
    struct SyncExtension(Manifest);
    impl Extension for SyncExtension {
        fn manifest(&self) -> &Manifest {
            &self.0
        }
        fn register(&self, reg: &mut Registry) -> Result<(), Error> {
            reg.action(Arc::new(Sync));
            Ok(())
        }
    }

    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let folder = dir.path().join("holiday-2026");
    fs::create_dir_all(&folder).unwrap_or_else(|e| panic!("{e}"));
    let long = format!("{}.tar.zst", "backup-of-the-whole-photo-archive-".repeat(4));
    fs::write(folder.join(&long), vec![7_u8; 70_000]).unwrap_or_else(|e| panic!("{e}"));
    fs::write(folder.join("notes.txt"), "notes").unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: vec![
            Arc::new(ext_action_terminal::TerminalExtension::new()),
            Arc::new(SyncExtension(Manifest {
                id: types::ExtId("test-sync"),
                name: "Sync",
                description: "",
                version: Version::new(1, 0, 0),
                api_version: API_VERSION,
                requires: Capabilities::empty(),
                points: Points::ACTION,
                platforms: Platforms::ALL,
                previews: &[],
                commands: &[],
            })),
        ],
        config_dir: Some(fake.config_dir()),
        start: Some(local(&folder)),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 720.0))
        .with_pixels_per_point(2.0)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    let modes = [
        (Theme::Light, Density::Comfortable, "light-comfortable"),
        (Theme::Light, Density::Compact, "light-compact"),
        (Theme::Dark, Density::Comfortable, "dark-comfortable"),
        (Theme::Dark, Density::Compact, "dark-compact"),
    ];
    let set = |harness: &mut Harness<'_, RickUi>, theme: Theme, density: Density| {
        for change in [
            SettingsChange::Theme(theme),
            SettingsChange::Density(density),
        ] {
            harness
                .state_mut()
                .app_mut()
                .handle(app::Command::Settings(change), &mut Vec::new());
        }
        harness.event(egui::Event::PointerMoved(egui::pos2(640.0, 500.0)));
        for _ in 0..4 {
            harness.step();
        }
    };
    let explorer = harness.state().app().model().focused;

    // The hash dialogue on the file with the long name.
    let id = harness
        .state()
        .app()
        .model()
        .active_tab(explorer)
        .and_then(|(_, tab)| tab.listing.rows().find(|e| e.name == long).map(|e| e.id))
        .unwrap_or_else(|| panic!("the file is listed"));
    harness.state_mut().app_mut().handle(
        app::Command::Select {
            pane: explorer,
            change: SelectionChange::Set(vec![id]),
        },
        &mut Vec::new(),
    );
    harness.step();
    harness
        .state_mut()
        .queue(Intent::Action(ActionId("core.hash")), explorer);
    settle(&mut harness);
    for (theme, density, mode) in modes {
        set(&mut harness, theme, density);
        shoot(&mut harness, &format!("small-hash-{mode}"));
    }
    harness.get_by_role_and_label(Role::Button, "Done").click();
    settle(&mut harness);

    // A failed job: the notice, and the drawer's row at rest.
    harness
        .state_mut()
        .queue(Intent::Action(ActionId("test.sync")), explorer);
    let deadline = Instant::now() + Duration::from_secs(20);
    while harness.state().toasts().is_empty() {
        harness.step();
        assert!(Instant::now() < deadline, "the job did not fail");
        std::thread::sleep(Duration::from_millis(5));
    }
    for (theme, density, mode) in modes {
        set(&mut harness, theme, density);
        shoot(&mut harness, &format!("small-error-{mode}"));
    }
    // The row unfolded to the whole text.
    harness
        .query_all_by_label_contains("Copy 14 items to archive · ")
        .find(|node| node.accesskit_node().role() == Role::Button)
        .unwrap_or_else(|| panic!("the failed row"))
        .click();
    for _ in 0..4 {
        harness.step();
    }
    for (theme, density, mode) in modes {
        set(&mut harness, theme, density);
        shoot(&mut harness, &format!("small-error-unfolded-{mode}"));
    }

    // A console's tab, beside the window group.
    harness
        .state_mut()
        .queue(Intent::Action(ActionId("terminal.console")), explorer);
    let deadline = Instant::now() + Duration::from_secs(20);
    while harness.state().app().model().focused == explorer {
        harness.step();
        assert!(Instant::now() < deadline, "no console");
        std::thread::sleep(Duration::from_millis(5));
    }
    for (theme, density, mode) in modes {
        set(&mut harness, theme, density);
        shoot(&mut harness, &format!("small-title-row-{mode}"));
    }
}

/// SPEC.md *Title bar colour* (issue 139, item 10): the title row on
/// every preset's band with three workspace tabs, the middle one hovered,
/// in Light and Dark at both densities; and the setting's card on the
/// Appearance page, with a preset and with *Custom* chosen.
#[test]
#[ignore = "needs a wgpu adapter; CI runs it and uploads the images"]
#[allow(clippy::too_many_lines)]
fn screenshots_of_the_title_bar_colour() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable as _;
    use model::TitlebarColour;
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let folders = ["icons — android-studio", "flowcpt", "Downloads"];
    for folder in folders {
        fs::create_dir_all(dir.path().join(folder)).unwrap_or_else(|e| panic!("{e}"));
    }
    let mut harness = shell(&dir.path().join(folders[0]), egui::vec2(1600.0, 300.0));
    for folder in &folders[1..] {
        harness.state_mut().app_mut().handle(
            app::Command::NewTab(local(&dir.path().join(folder))),
            &mut Vec::new(),
        );
        settle(&mut harness);
    }
    let model = harness.state().app().model();
    let first = model.workspaces(model.focused_window)[0].id;
    harness
        .state_mut()
        .app_mut()
        .handle(app::Command::ActivateWorkspace(first), &mut Vec::new());
    settle(&mut harness);
    let modes = [
        (Theme::Light, Density::Comfortable, "light-comfortable"),
        (Theme::Light, Density::Compact, "light-compact"),
        (Theme::Dark, Density::Comfortable, "dark-comfortable"),
        (Theme::Dark, Density::Compact, "dark-compact"),
    ];
    let change = |harness: &mut Harness<'_, RickUi>, change: SettingsChange| {
        harness
            .state_mut()
            .app_mut()
            .handle(app::Command::Settings(change), &mut Vec::new());
    };
    let presets = [
        (TitlebarColour::DustyRose, "dusty-rose"),
        (TitlebarColour::WarmGrey, "warm-grey"),
        (TitlebarColour::SlateTeal, "slate-teal"),
        (TitlebarColour::Sage, "sage"),
        (TitlebarColour::Plum, "plum"),
        (TitlebarColour::DarkBand, "dark-band"),
    ];
    for (preset, name) in presets {
        change(&mut harness, SettingsChange::TitlebarColour(preset));
        for (theme, density, mode) in modes {
            change(&mut harness, SettingsChange::Theme(theme));
            change(&mut harness, SettingsChange::Density(density));
            for _ in 0..4 {
                harness.step();
            }
            // At rest: the pointer is nowhere near the row.
            harness.event(egui::Event::PointerMoved(egui::pos2(800.0, 280.0)));
            for _ in 0..4 {
                harness.step();
            }
            shoot(&mut harness, &format!("titlebar-{name}-{mode}"));
            // An inactive tab under the pointer, on the default band and
            // on the one that turns the row to light text.
            let hovered = matches!(preset, TitlebarColour::DustyRose | TitlebarColour::DarkBand)
                && density == Density::Comfortable;
            if hovered {
                let tab = harness.get_by_label("flowcpt").rect().center();
                harness.event(egui::Event::PointerMoved(tab));
                harness.step();
                harness.step();
                shoot(&mut harness, &format!("titlebar-hover-{name}-{mode}"));
            }
        }
    }

    // The setting's card, in a window tall enough for the page.
    std::mem::drop(harness);
    let mut harness = shell(&dir.path().join(folders[0]), egui::vec2(1280.0, 800.0));
    harness.state_mut().open_settings();
    for _ in 0..4 {
        harness.step();
    }
    harness
        .get_by_role_and_label(Role::RadioButton, "Appearance")
        .click();
    for (choice, name) in [
        (TitlebarColour::DustyRose, "setting"),
        (TitlebarColour::Custom, "setting-custom"),
    ] {
        change(&mut harness, SettingsChange::TitlebarColour(choice));
        for (theme, density, mode) in modes {
            change(&mut harness, SettingsChange::Theme(theme));
            change(&mut harness, SettingsChange::Density(density));
            harness.event(egui::Event::PointerMoved(egui::pos2(300.0, 780.0)));
            for _ in 0..4 {
                harness.step();
            }
            shoot(&mut harness, &format!("titlebar-{name}-{mode}"));
        }
    }
}

/// SPEC.md *Extensions dialogue* (issue 139, item 9): the list grouped by
/// what the plugins contribute, with a plugin switched off and one that
/// needs attention, and the details of a built-in provider, of the plugin
/// the host refused and of a user's plugin that waits for a restart — in
/// Light and Dark at both densities.
#[test]
#[ignore = "needs a wgpu adapter; CI runs it and uploads the images"]
#[allow(clippy::too_many_lines)]
fn screenshots_of_the_extensions_dialogue() {
    use app::{Plugin, PluginStatus};
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable as _;
    use ext::Extension;
    let (_keep, root) = sample_tree();
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    // The plugins that ship, loaded for real so that the dialogue shows
    // what they register, as rows beside the binary.
    let loaded: Vec<Arc<dyn Extension>> = vec![
        Arc::new(ext_vfs_azure::AzureExtension::new()),
        Arc::new(ext_vfs_s3::S3Extension::new()),
        Arc::new(ext_vfs_gcs::GcsExtension::new()),
        Arc::new(ext_ssh::SshExtension::new()),
        Arc::new(ext_vfs_archive::ArchiveExtension::new()),
        Arc::new(ext_vfs_trash::TrashExtension::new()),
        Arc::new(ext_preview_3d::Preview3d::new()),
        Arc::new(ext_preview_archive::ArchivePreview::new()),
        Arc::new(ext_preview_hex::HexPreview::new()),
        Arc::new(ext_preview_media::MediaPreview::new()),
        Arc::new(ext_preview_pdf::PdfPreview::new()),
        Arc::new(ext_git::GitExtension::new()),
        Arc::new(ext_action_rename::RenameExtension::new()),
    ];
    let beside = Path::new("/opt/rick/plugins");
    let row = |ext: &dyn Extension| {
        let manifest = ext.manifest();
        Plugin {
            id: manifest.id.0.to_owned(),
            name: manifest.name.to_owned(),
            description: manifest.description.to_owned(),
            version: manifest.version.to_string(),
            points: manifest.points,
            previews: manifest.previews.iter().map(|p| (*p).to_owned()).collect(),
            status: PluginStatus::Loaded,
            path: beside.join(manifest.id.0),
            built_in: true,
            removed: false,
        }
    };
    let mut rows: Vec<Plugin> = loaded.iter().map(|ext| row(&**ext)).collect();
    // One switched off, one the host refused, one of the user's.
    rows.push(Plugin {
        status: PluginStatus::Disabled,
        ..row(&ext_preview_office::OfficePreview::new())
    });
    rows.push(Plugin {
        status: PluginStatus::Refused(
            "built with rustc 1.97.0, and this Rick with rustc 1.99.1".to_owned(),
        ),
        ..row(&ext_search_tracker::TrackerExtension::new())
    });
    let per_user = fake.config_dir().join("plugins");
    rows.push(Plugin {
        id: "word-count".to_owned(),
        name: "Word count".to_owned(),
        description: "Counts the words, lines and characters of the selected text files."
            .to_owned(),
        version: "1.2.0".to_owned(),
        points: ext::Points::ACTION,
        previews: Vec::new(),
        status: PluginStatus::Loaded,
        path: per_user.join("word-count"),
        built_in: false,
        removed: false,
    });
    let mut app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: loaded,
        config_dir: Some(fake.config_dir()),
        start: Some(local(&root)),
    });
    app.set_plugins(rows);
    app.handle(
        app::Command::Settings(SettingsChange::Plugin {
            id: "preview-office".to_owned(),
            enabled: false,
        }),
        &mut Vec::new(),
    );
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 840.0))
        .with_pixels_per_point(2.0)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    let modes = [
        (Theme::Light, Density::Comfortable, "light-comfortable"),
        (Theme::Light, Density::Compact, "light-compact"),
        (Theme::Dark, Density::Comfortable, "dark-comfortable"),
        (Theme::Dark, Density::Compact, "dark-compact"),
    ];
    let set = |harness: &mut Harness<'_, RickUi>, theme: Theme, density: Density| {
        for change in [
            SettingsChange::Theme(theme),
            SettingsChange::Density(density),
        ] {
            harness
                .state_mut()
                .app_mut()
                .handle(app::Command::Settings(change), &mut Vec::new());
        }
        // The pointer rests outside the dialogue, on no button.
        harness.event(egui::Event::PointerMoved(egui::pos2(40.0, 820.0)));
        for _ in 0..4 {
            harness.step();
        }
    };
    let choose = |harness: &mut Harness<'_, RickUi>, name: &str| {
        harness
            .get_by_role_and_label(Role::RadioButton, name)
            .click();
        for _ in 0..4 {
            harness.step();
        }
    };
    let pane = harness.state().app().model().focused;
    harness.state_mut().queue(Intent::Extensions, pane);
    for _ in 0..4 {
        harness.step();
    }

    // A built-in provider, the first of the list.
    choose(&mut harness, "Azure Blob");
    for (theme, density, mode) in modes {
        set(&mut harness, theme, density);
        shoot(&mut harness, &format!("extensions-{mode}"));
    }
    // The plugin the host refused, at the list's end: the attention card.
    harness.event(egui::Event::PointerMoved(egui::pos2(400.0, 500.0)));
    harness.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -600.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    });
    for _ in 0..12 {
        harness.step();
    }
    choose(&mut harness, "Tracker");
    for (theme, density, mode) in modes {
        set(&mut harness, theme, density);
        shoot(&mut harness, &format!("extensions-attention-{mode}"));
    }
    // A user's plugin, switched off a moment ago: the restart chips and
    // Remove.
    choose(&mut harness, "Word count");
    harness
        .get_all_by_label("Enable Word count")
        .next()
        .unwrap_or_else(|| panic!("the switch"))
        .click();
    for (theme, density, mode) in [modes[0], modes[2]] {
        set(&mut harness, theme, density);
        shoot(&mut harness, &format!("extensions-user-{mode}"));
    }
}

/// SPEC.md *Keyboard shortcuts dialogue* (issue 139, item 8): the
/// dialogue on *File operations* with a changed row and a cell that
/// records, on *Git* with a chord that Copy has too — the amber chip —
/// and with a chord pressed in the search field, in Light and Dark at
/// both densities.
#[test]
#[ignore = "needs a wgpu adapter; CI runs it and uploads the images"]
#[allow(clippy::too_many_lines)]
fn screenshots_of_the_keyboard_shortcuts_dialogue() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable as _;
    let (_keep, root) = sample_tree();
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: vec![
            Arc::new(ext_git::GitExtension::new()),
            Arc::new(ext_svn::SvnExtension::new()),
            Arc::new(ext_ssh::SshExtension::new()),
            Arc::new(ext_action_compress::CompressExtension::new()),
            Arc::new(ext_action_compare::CompareExtension::new()),
            Arc::new(ext_action_rename::RenameExtension::new()),
            Arc::new(ext_action_terminal::TerminalExtension::new()),
        ],
        config_dir: Some(fake.config_dir()),
        start: Some(local(&root)),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_pixels_per_point(2.0)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    let modes = [
        (Theme::Light, Density::Comfortable, "light-comfortable"),
        (Theme::Light, Density::Compact, "light-compact"),
        (Theme::Dark, Density::Comfortable, "dark-comfortable"),
        (Theme::Dark, Density::Compact, "dark-compact"),
    ];
    let steps = |harness: &mut Harness<'_, RickUi>| {
        for _ in 0..4 {
            harness.step();
        }
    };
    let set = |harness: &mut Harness<'_, RickUi>, theme: Theme, density: Density| {
        for change in [
            SettingsChange::Theme(theme),
            SettingsChange::Density(density),
        ] {
            harness
                .state_mut()
                .app_mut()
                .handle(app::Command::Settings(change), &mut Vec::new());
        }
        // The pointer rests outside the dialogue, on no button.
        harness.event(egui::Event::PointerMoved(egui::pos2(40.0, 780.0)));
        for _ in 0..4 {
            harness.step();
        }
    };
    let bind = |harness: &mut Harness<'_, RickUi>, action: &str, binding: &str| {
        harness.state_mut().app_mut().handle(
            app::Command::Settings(SettingsChange::Shortcut {
                action: action.to_owned(),
                binding: Some(binding.to_owned()),
            }),
            &mut Vec::new(),
        );
    };
    let choose = |harness: &mut Harness<'_, RickUi>, category: &str| {
        harness
            .get_by_role_and_label(Role::RadioButton, category)
            .click();
        for _ in 0..4 {
            harness.step();
        }
    };
    let cell = |harness: &mut Harness<'_, RickUi>, name: &str| {
        harness.get_by_label(name).click();
        for _ in 0..4 {
            harness.step();
        }
    };
    let ctrl = platform::MOD_KEY;

    // A changed row, a chord taken twice.
    bind(&mut harness, "core.duplicate", "Mod+Alt+D");
    bind(&mut harness, "git.commit", "Mod+K");
    bind(&mut harness, "svn.commit", "Mod+K");
    let pane = harness.state().app().model().focused;
    harness.state_mut().queue(Intent::Shortcuts, pane);
    steps(&mut harness);

    // File operations, one cell recording.
    choose(&mut harness, "File operations");
    cell(
        &mut harness,
        &format!("Shortcut for New folder: {ctrl}+Shift+N"),
    );
    for (theme, density, mode) in modes {
        set(&mut harness, theme, density);
        shoot(&mut harness, &format!("shortcuts-{mode}"));
    }
    harness.key_press(egui::Key::Escape);
    steps(&mut harness);

    // Git, whose commit shares its chord with SVN's.
    choose(&mut harness, "Git");
    for (theme, density, mode) in modes {
        set(&mut harness, theme, density);
        shoot(&mut harness, &format!("shortcuts-git-{mode}"));
    }

    // All, narrowed by a chord pressed in the search field.
    choose(&mut harness, "All");
    harness
        .get_all_by_role(Role::TextInput)
        .find(|field| (100.0..300.0).contains(&field.rect().top()))
        .unwrap_or_else(|| panic!("the search field"))
        .click();
    steps(&mut harness);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::K);
    steps(&mut harness);
    for (theme, density, mode) in [modes[0], modes[2]] {
        set(&mut harness, theme, density);
        shoot(&mut harness, &format!("shortcuts-chord-{mode}"));
    }
}

/// Issue 139, item 7: the size limit of remote previews as the Figma
/// frame has it (`151:4550`) — the File info card for a picture above the
/// limit on a cloud account, and the download half way, with its progress
/// under the Portal burst — in Light and Dark at both densities. The
/// account is an in-memory stand-in whose reads a gate holds.
#[test]
#[ignore = "needs a wgpu adapter; CI runs it and uploads the images"]
fn screenshots_of_the_remote_size_cap() {
    use egui_kittest::kittest::Queryable as _;
    use tests::mem::{MemExtension, MemProvider};
    use types::{Location, ProviderId};
    const MIB: f64 = 1024.0 * 1024.0;
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let mem = Arc::new(MemProvider::new(ProviderId("azure"), Duration::ZERO));
    mem.set_native(true);
    mem.dir("/flowcptstorage/media");
    let mut photo = media::photo_jpeg(64, 48);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    photo.resize((18.4 * MIB) as usize, 0);
    mem.file("/flowcptstorage/media/3K6ehNLhnRsPfWhzju8Qe2Lk.jpg", &photo);
    ext::i18n::add_bundled("en", "screenshot-cloud-provider = Azure Blob");
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: vec![
            Arc::new(ext_preview_image::ImagePreview::new()),
            Arc::new(MemExtension::with_accounts(
                Arc::clone(&mem),
                "screenshot-cloud-provider",
            )),
        ],
        config_dir: Some(fake.config_dir()),
        start: Some(local(dir.path())),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_pixels_per_point(2.0)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    harness.state_mut().set_inspector_width(600.0);
    let pane = harness.state().app().model().focused;
    harness.state_mut().queue(Intent::ToggleInspector, pane);
    harness.state_mut().app_mut().handle(
        app::Command::Navigate {
            pane,
            to: Location::new(ProviderId("azure"), "/flowcptstorage/media"),
        },
        &mut Vec::new(),
    );
    settle(&mut harness);
    let id = harness
        .state()
        .app()
        .model()
        .active_tab(pane)
        .and_then(|(_, t)| t.listing.rows().next().map(|e| e.id))
        .unwrap_or_else(|| panic!("the picture is listed"));
    harness.state_mut().app_mut().handle(
        app::Command::Select {
            pane,
            change: SelectionChange::Set(vec![id]),
        },
        &mut Vec::new(),
    );
    let wait = |harness: &mut Harness<'_, RickUi>,
                what: &str,
                done: &dyn Fn(&ui_egui::preview::Slot) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !harness.state().previews().get(pane).is_some_and(done) {
            assert!(Instant::now() < deadline, "{what}");
            harness.step();
            std::thread::sleep(Duration::from_millis(5));
        }
        for _ in 0..6 {
            harness.step();
        }
    };
    let modes = [
        (Theme::Light, Density::Comfortable, "light-comfortable"),
        (Theme::Light, Density::Compact, "light-compact"),
        (Theme::Dark, Density::Comfortable, "dark-comfortable"),
        (Theme::Dark, Density::Compact, "dark-compact"),
    ];
    let set = |harness: &mut Harness<'_, RickUi>, theme: Theme, density: Density| {
        for change in [
            SettingsChange::Theme(theme),
            SettingsChange::Density(density),
        ] {
            harness
                .state_mut()
                .app_mut()
                .handle(app::Command::Settings(change), &mut Vec::new());
        }
        // The pointer rests on nothing.
        harness.event(egui::Event::PointerMoved(egui::pos2(300.0, 780.0)));
        for _ in 0..4 {
            harness.step();
        }
    };

    // Above the limit: the card.
    wait(&mut harness, "no size-limit card", &|slot| {
        slot.capped().is_some()
    });
    for (theme, density, mode) in modes {
        set(&mut harness, theme, density);
        shoot(&mut harness, &format!("preview-size-limit-{mode}"));
    }

    // Preview this file, held at 7.7 of 18.4 MiB.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let held = (7.7 * MIB) as u64;
    mem.hold_reads_at(held);
    harness.get_by_label("Preview this file").click();
    wait(&mut harness, "no download half way", &|slot| {
        slot.download().is_some_and(|(read, _)| read >= held)
    });
    for (theme, density, mode) in modes {
        set(&mut harness, theme, density);
        shoot(&mut harness, &format!("preview-downloading-{mode}"));
    }
    mem.release_reads();
    settle(&mut harness);
}

/// Issue 139, item 4: the Properties dialogue as the Figma frames have it
/// (`144:4115`) — a folder and a symbolic link on the General tab, the
/// Permissions tab, and the tab with the access control list open — in
/// Light and Dark at both densities. The folder's files are sparse, so
/// what they occupy on disk is next to nothing; the list's extra entry is
/// a user this machine has, where the file system takes one.
#[cfg(unix)]
#[test]
#[ignore = "needs a wgpu adapter; CI runs it and uploads the images"]
#[allow(clippy::too_many_lines)]
fn screenshots_of_the_properties_dialogue() {
    use egui_kittest::kittest::Queryable as _;
    use platform::{Acl, PermissionChange, Permissions as _, PosixAce, PosixTag};
    use std::os::unix::fs::PermissionsExt as _;
    const MIB: u64 = 1024 * 1024;
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let home = dir.path().join("home");
    let serge = home.join("serge");
    let rider = serge.join(".local/opt/jetbrains-rider/current/bin");
    let bin = serge.join(".local/bin");
    fs::create_dir_all(&rider).unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir_all(&bin).unwrap_or_else(|e| panic!("{e}"));
    let sparse = |path: &Path, bytes: u64| {
        let file = fs::File::create(path).unwrap_or_else(|e| panic!("{e}"));
        file.set_len(bytes).unwrap_or_else(|e| panic!("{e}"));
    };
    // Twelve items inside, 3.4 GiB.
    sparse(&rider.join("rider"), 2 * MIB + 100 * 1024);
    fs::set_permissions(rider.join("rider"), fs::Permissions::from_mode(0o755))
        .unwrap_or_else(|e| panic!("{e}"));
    sparse(&serge.join("backup.tar"), 3 * 1024 * MIB);
    sparse(&serge.join("movie.mkv"), 407 * MIB);
    for name in ["notes.md", "todo.txt"] {
        sparse(&serge.join(name), 12 * 1024);
    }
    std::os::unix::fs::symlink(rider.join("rider"), bin.join("jetbrains-rider"))
        .unwrap_or_else(|e| panic!("{e}"));
    fs::set_permissions(&serge, fs::Permissions::from_mode(0o710))
        .unwrap_or_else(|e| panic!("{e}"));

    let mut harness = shell(&home, egui::vec2(1280.0, 1000.0));
    let modes = [
        (Theme::Light, Density::Comfortable, "light-comfortable"),
        (Theme::Light, Density::Compact, "light-compact"),
        (Theme::Dark, Density::Comfortable, "dark-comfortable"),
        (Theme::Dark, Density::Compact, "dark-compact"),
    ];
    let set = |harness: &mut Harness<'_, RickUi>, theme: Theme, density: Density| {
        for change in [
            SettingsChange::Theme(theme),
            SettingsChange::Density(density),
        ] {
            harness
                .state_mut()
                .app_mut()
                .handle(app::Command::Settings(change), &mut Vec::new());
        }
        // The pointer rests outside the dialogue, on no button.
        harness.event(egui::Event::PointerMoved(egui::pos2(40.0, 980.0)));
        for _ in 0..4 {
            harness.step();
        }
    };
    // Goes to `folder`, selects `name` there and opens Properties on it.
    let open =
        |harness: &mut Harness<'_, RickUi>, folder: &Path, name: &str| {
            let pane = harness.state().app().model().focused;
            harness.state_mut().app_mut().handle(
                app::Command::Navigate {
                    pane,
                    to: local(folder),
                },
                &mut Vec::new(),
            );
            settle(harness);
            let id = harness
                .state()
                .app()
                .model()
                .active_tab(pane)
                .and_then(|(_, tab)| tab.rows().find(|e| e.name == name).map(|e| e.id))
                .unwrap_or_else(|| panic!("{name} is listed"));
            for command in [
                app::Command::Select {
                    pane,
                    change: SelectionChange::Set(vec![id]),
                },
                app::Command::ShowProperties(pane),
            ] {
                harness
                    .state_mut()
                    .app_mut()
                    .handle(command, &mut Vec::new());
            }
            let deadline = Instant::now() + Duration::from_secs(10);
            // Open, and a folder counted.
            while !harness.state().properties().info().is_some_and(|info| {
                info.entry.name == name && info.contents != app::Contents::Counting
            }) {
                assert!(Instant::now() < deadline, "no Properties dialogue");
                harness.step();
                std::thread::sleep(Duration::from_millis(5));
            }
            for _ in 0..4 {
                harness.step();
            }
        };
    let close = |harness: &mut Harness<'_, RickUi>| {
        harness.key_press(egui::Key::Escape);
        for _ in 0..4 {
            harness.step();
        }
        settle(harness);
    };

    // The folder, on the General tab.
    open(&mut harness, &home, "serge");
    for (theme, density, mode) in modes {
        set(&mut harness, theme, density);
        shoot(&mut harness, &format!("properties-folder-{mode}"));
    }
    close(&mut harness);

    // The symbolic link.
    open(&mut harness, &bin, "jetbrains-rider");
    for (theme, density, mode) in modes {
        set(&mut harness, theme, density);
        shoot(&mut harness, &format!("properties-link-{mode}"));
    }
    close(&mut harness);

    // An extra entry on the folder's list: a user of this machine, with
    // execute, under a mask of execute — where the file system takes it.
    let pane = harness.state().app().model().focused;
    let posix = matches!(
        platform::Native.permissions(&serge).map(|p| p.acl),
        Ok(Some(Acl::Posix { .. }))
    );
    let users = platform::Native.principals().ok().filter(|_| posix);
    let extra = users.and_then(|p| {
        p.users.into_iter().find(|u| {
            u.id != "0"
                && u.name
                    .as_deref()
                    .is_some_and(|name| name.chars().all(char::is_alphanumeric))
        })
    });
    if let Some(user) = extra {
        let ace = |tag, qualifier, perms| PosixAce {
            tag,
            qualifier,
            perms,
        };
        let change = PermissionChange {
            acl: Some(Acl::Posix {
                access: vec![
                    ace(PosixTag::UserObj, None, 7),
                    ace(PosixTag::User, Some(user), 1),
                    ace(PosixTag::GroupObj, None, 0),
                    ace(PosixTag::Mask, None, 1),
                    ace(PosixTag::Other, None, 0),
                ],
                default: Vec::new(),
            }),
            ..PermissionChange::default()
        };
        harness.state_mut().app_mut().handle(
            app::Command::SetPermissions {
                pane,
                request: app::PermissionRequest {
                    targets: vec![serge.clone()],
                    change,
                    recursive: None,
                },
            },
            &mut Vec::new(),
        );
        settle(&mut harness);
    }

    // The Permissions tab, then the list open.
    open(&mut harness, &home, "serge");
    harness.get_by_label("Permissions").click();
    for _ in 0..4 {
        harness.step();
    }
    for (theme, density, mode) in modes {
        set(&mut harness, theme, density);
        shoot(&mut harness, &format!("properties-permissions-{mode}"));
    }
    if harness
        .query_by_label("Access control list (ACL)")
        .is_some()
    {
        harness.get_by_label("Access control list (ACL)").click();
        for _ in 0..4 {
            harness.step();
        }
        for (theme, density, mode) in modes {
            set(&mut harness, theme, density);
            shoot(&mut harness, &format!("properties-acl-{mode}"));
        }
    }
    close(&mut harness);
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
    // Install as application is Linux only (specs/ext-installer.md): the
    // action is hidden elsewhere and the dialogue cannot open, so the
    // Linux runner takes these pictures.
    if !ext_installer::supported() {
        return;
    }
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

/// Issue 173: the Git clone dialogue — opened on the empty area with a
/// Git URL on the clipboard, in Light and Dark; then with the options
/// open; and with nothing on the clipboard, where the field asks for the
/// URL.
#[test]
#[ignore = "needs a wgpu adapter; CI runs it and uploads the images"]
fn screenshots_of_the_git_clone_dialogue() {
    use egui_kittest::kittest::Queryable as _;
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let projects = dir.path().join("projects");
    fs::create_dir_all(projects.join("notes")).unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: vec![Arc::new(ext_git::GitExtension::new())],
        config_dir: Some(fake.config_dir()),
        start: Some(local(&projects)),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1920.0, 1000.0))
        .with_pixels_per_point(2.0)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    let open = |harness: &mut Harness<'_, RickUi>, clipboard: &str| {
        platform::Clipboard::set_text(&*fake, clipboard).unwrap_or_else(|e| panic!("{e}"));
        let pane = harness.state().app().model().focused;
        harness.state_mut().app_mut().handle(
            app::Command::Select {
                pane,
                change: SelectionChange::Set(Vec::new()),
            },
            &mut Vec::new(),
        );
        settle(harness);
        harness
            .state_mut()
            .queue(Intent::Action(ext::ActionId("git.clone")), pane);
        step_until(harness, Duration::from_secs(15), |h| {
            h.query_by_label("Clone repository").is_some()
        });
        harness.event(egui::Event::PointerMoved(egui::pos2(960.0, 30.0)));
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
    let url = "https://github.com/Open-Breach/rick.git";
    for (theme, name) in [(Theme::Light, "light"), (Theme::Dark, "dark")] {
        harness.state_mut().app_mut().handle(
            app::Command::Settings(SettingsChange::Theme(theme)),
            &mut Vec::new(),
        );
        settle(&mut harness);
        open(&mut harness, url);
        shoot(&mut harness, &format!("git-clone-dialogue-{name}"));
        close(&mut harness);
    }
    harness.state_mut().app_mut().handle(
        app::Command::Settings(SettingsChange::Theme(Theme::Light)),
        &mut Vec::new(),
    );
    settle(&mut harness);
    // The options open.
    open(&mut harness, url);
    let options = harness
        .get_by_role_and_label(egui::accesskit::Role::CheckBox, "Options")
        .rect()
        .center();
    harness.event(egui::Event::PointerMoved(options));
    harness.step();
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos: options,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        });
        harness.step();
    }
    harness.event(egui::Event::PointerMoved(egui::pos2(960.0, 30.0)));
    for _ in 0..8 {
        harness.step();
    }
    shoot(&mut harness, "git-clone-dialogue-options");
    close(&mut harness);
    // Nothing on the clipboard that is a Git URL.
    open(&mut harness, "meeting notes");
    shoot(&mut harness, "git-clone-dialogue-blank");
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

/// [`settle`], and no integration check or apply in flight: the answer of
/// a job that has ended may still be waiting for the frame that folds it
/// into the model.
fn settle_integration(harness: &mut Harness<'_, RickUi>) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        settle(harness);
        let integration = &harness.state().app().model().integration;
        if !integration.checking && integration.applying.is_none() {
            break;
        }
        assert!(Instant::now() < deadline, "the integration did not settle");
    }
}

/// Issue 120: the integration dialog as a Rick unpacked from the archive
/// shows it at its first start on a GNOME desktop — every row on, then
/// with the dock favourite switched off and a `rick` link of another
/// origin in the command's way — in Light and Dark.
#[test]
#[ignore = "needs a wgpu adapter; CI runs it and uploads the images"]
fn screenshots_of_the_integration_dialog() {
    use egui::accesskit::Role;
    use platform::integration::{Check, Desktop, Item};
    let (dir, deep) = sample_tree();
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let home = fake.home_dir();
    let integration =
        platform::FakeIntegration::new("Fedora Linux 43 · GNOME", Some(Desktop::Gnome));
    integration.set(
        Item::InstallLocation,
        Check {
            place: Some(home.join(".local/opt/rick")),
            target: Some(home.join("Downloads/rick-0.3.0")),
            ..Check::missing()
        },
    );
    integration.set(Item::LauncherEntry, Check::missing());
    integration.set(
        Item::DefaultFileManager,
        Check {
            name: Some("Files".to_owned()),
            ..Check::missing()
        },
    );
    integration.set(Item::ShowInFolder, Check::missing());
    integration.set(
        Item::RickCommand,
        Check {
            place: Some(home.join(".local/bin/rick")),
            ..Check::missing()
        },
    );
    integration.set(Item::DockFavourite, Check::missing());
    fake.set_integration(Arc::new(integration.clone()));
    let host = Arc::new(EguiHost::new());
    // The installer's command makes the install row one Rick can perform.
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: vec![Arc::new(ext_installer::InstallerExtension::new())],
        config_dir: Some(fake.config_dir()),
        start: Some(local(&deep)),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 900.0))
        .with_pixels_per_point(2.0)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle_integration(&mut harness);
    assert!(harness.state().integration_dialog().is_open());
    let set_theme = |harness: &mut Harness<'_, RickUi>, theme: Theme| {
        harness.state_mut().app_mut().handle(
            app::Command::Settings(SettingsChange::Theme(theme)),
            &mut Vec::new(),
        );
        settle_integration(harness);
        harness.step();
    };
    for (theme, name) in [(Theme::Light, "light"), (Theme::Dark, "dark")] {
        set_theme(&mut harness, theme);
        shoot(&mut harness, &format!("integration-dialog-{name}"));
    }

    // A row switched off, and a link of another origin in a row's way.
    integration.set(
        Item::RickCommand,
        Check {
            place: Some(home.join(".local/bin/rick")),
            foreign: vec![home.join(".local/bin/rick")],
            ..Check::missing()
        },
    );
    harness.state_mut().app_mut().handle(
        app::Command::Integration(app::IntegrationCommand::Check { startup: false }),
        &mut Vec::new(),
    );
    settle_integration(&mut harness);
    let at = harness
        .get_by_role_and_label(Role::CheckBox, "Dock favourite")
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
    harness.event(egui::Event::PointerMoved(egui::pos2(40.0, 880.0)));
    harness.step();
    harness.step();
    for (theme, name) in [(Theme::Light, "light"), (Theme::Dark, "dark")] {
        set_theme(&mut harness, theme);
        shoot(&mut harness, &format!("integration-dialog-choices-{name}"));
    }
    std::mem::drop(dir);
}

/// Issue 120: Settings → Integration on a GNOME desktop where Rick is
/// installed and set up but for the default file manager, which the
/// system reset, and the dock favourite, which was skipped — the top of
/// the page and its end, in Light and Dark.
#[test]
#[ignore = "needs a wgpu adapter; CI runs it and uploads the images"]
fn screenshots_of_the_integration_settings() {
    use egui::accesskit::Role;
    use platform::integration::{Check, Desktop, Item};
    let (dir, deep) = sample_tree();
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let home = fake.home_dir();
    let integration =
        platform::FakeIntegration::new("Fedora Linux 43 · GNOME", Some(Desktop::Gnome));
    integration.set(
        Item::InstallLocation,
        Check {
            place: Some(home.join(".local/opt/rick/0.3.0")),
            ..Check::set_up()
        },
    );
    integration.set(
        Item::LauncherEntry,
        Check {
            place: Some(home.join(".local/share")),
            ..Check::set_up()
        },
    );
    integration.set(
        Item::DefaultFileManager,
        Check {
            name: Some("Files".to_owned()),
            ..Check::missing()
        },
    );
    integration.set(Item::ShowInFolder, Check::set_up());
    integration.set(
        Item::RickCommand,
        Check {
            place: Some(home.join(".local/bin/rick")),
            target: Some(home.join(".local/opt/rick/current/rick")),
            ..Check::set_up()
        },
    );
    integration.set(Item::DockFavourite, Check::missing());
    fake.set_integration(Arc::new(integration));
    // An earlier start's choices, with the dialog silenced so the page is
    // not under it.
    let config = fake.config_dir();
    fs::create_dir_all(&config).unwrap_or_else(|e| panic!("{e}"));
    fs::write(
        config.join(app::SETTINGS_FILE),
        "[integration]\nask = false\n\n\
         [integration.items.default-file-manager]\nstate = \"set-up\"\nchecked = 2026-11-08\n\n\
         [integration.items.dock-favourite]\nstate = \"declined\"\nchecked = 2026-11-08\n",
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let host = Arc::new(EguiHost::new());
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: Vec::new(),
        config_dir: Some(config),
        start: Some(local(&deep)),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_pixels_per_point(2.0)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle_integration(&mut harness);
    harness.state_mut().open_settings();
    for _ in 0..3 {
        harness.step();
    }
    let at = harness
        .get_by_role_and_label(Role::RadioButton, "Integration")
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
    harness.event(egui::Event::PointerMoved(egui::pos2(640.0, 20.0)));
    settle_integration(&mut harness);
    let set_theme = |harness: &mut Harness<'_, RickUi>, theme: Theme| {
        harness.state_mut().app_mut().handle(
            app::Command::Settings(SettingsChange::Theme(theme)),
            &mut Vec::new(),
        );
        settle_integration(harness);
        harness.step();
    };
    for (theme, name) in [(Theme::Light, "light"), (Theme::Dark, "dark")] {
        set_theme(&mut harness, theme);
        shoot(&mut harness, &format!("integration-settings-{name}"));
    }
    // The end of the page: the switches and Reset choices.
    let over = harness.get_by_label("Terminal and links").rect().center();
    harness.event(egui::Event::PointerMoved(over));
    harness.step();
    for _ in 0..4 {
        harness.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -400.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::default(),
        });
        harness.step();
    }
    harness.event(egui::Event::PointerMoved(egui::pos2(640.0, 20.0)));
    harness.step();
    for (theme, name) in [(Theme::Light, "light"), (Theme::Dark, "dark")] {
        set_theme(&mut harness, theme);
        shoot(&mut harness, &format!("integration-settings-end-{name}"));
    }
    std::mem::drop(dir);
}

/// Issue 149: the List view in Light and in Dark — the Downloads folder of
/// the Figma frame *List view mode*: folders first, columns of one width
/// filled top to bottom, a selected row, the cursor's ring and a long name
/// cut to its column.
#[test]
#[ignore = "needs a wgpu adapter; CI runs it and uploads the images"]
fn screenshots_of_the_list_view() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let downloads = dir.path().join("Downloads");
    for folder in [
        "Backup",
        "android-studio-rabbit1-linux",
        "code-stable-x64-1793133762",
        "rick-day-zero",
        "support-B87E-1788041812146",
    ] {
        fs::create_dir_all(downloads.join(folder)).unwrap_or_else(|e| panic!("{e}"));
    }
    let mut files: Vec<String> = [
        "20260828_CA ABR TITEL VERSAND.pdf",
        "a-photograph-with-a-name-so-long-that-no-column-of-the-list-view-could-hold-it-whole.jpg",
        "us-states-with-detail.json",
        "notes.md",
        "main.rs",
        "invoice-2026-08.pdf",
        "rick-0.3.0-linux-x64.tar.gz",
    ]
    .map(str::to_owned)
    .to_vec();
    files.extend((1..=40).map(|i| format!("20260903_1215{i:02}.jpg")));
    files.extend((1..=12).map(|i| format!("report-{i:02}.xlsx")));
    for name in &files {
        fs::write(downloads.join(name), name).unwrap_or_else(|e| panic!("{e}"));
    }
    let mut harness = shell(&downloads, egui::vec2(1100.0, 520.0));
    let pane = harness.state().app().model().focused;
    let send = |harness: &mut Harness<'static, RickUi>, command: app::Command| {
        harness
            .state_mut()
            .app_mut()
            .handle(command, &mut Vec::new());
        settle(harness);
    };
    send(
        &mut harness,
        app::Command::SetView {
            pane,
            mode: model::ViewMode::List,
        },
    );
    // One row selected, with the keyboard cursor on it.
    harness.key_press(egui::Key::Home);
    harness.step();
    for _ in 0..7 {
        harness.key_press(egui::Key::ArrowDown);
        harness.step();
    }
    settle(&mut harness);
    for (theme, name) in [(Theme::Light, "light"), (Theme::Dark, "dark")] {
        send(
            &mut harness,
            app::Command::Settings(SettingsChange::Theme(theme)),
        );
        shoot(&mut harness, &format!("list-view-{name}"));
    }
}

/// The fixture folders of the README's picture: a maker's `Projects`
/// folder and in it the project around the sample model
/// `assets/readme/robot-dog.stl`, every file and folder with a fixed
/// modification time so the Modified column reads the same on every run.
/// Returns `Projects` and the project folder.
fn readme_tree(home: &Path) -> (PathBuf, PathBuf) {
    use std::time::SystemTime;
    let projects = home.join("Projects");
    let project = projects.join("robot-dog");
    let folders = [
        projects.join("desk-lamp"),
        projects.join("plant-waterer"),
        projects.join("weather-station"),
        project.join("parts"),
        project.join("photos"),
        project.join("prints"),
    ];
    for folder in &folders {
        fs::create_dir_all(folder).unwrap_or_else(|e| panic!("{e}"));
    }
    let model = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/readme/robot-dog.stl");
    let model = fs::read(&model).unwrap_or_else(|e| panic!("{}: {e}", model.display()));
    let files: [(&Path, &str, Vec<u8>); 10] = [
        (
            &projects,
            "ideas.md",
            b"# Ideas\n\n- A robot cat, obviously.\n".to_vec(),
        ),
        (
            &projects,
            "filament.csv",
            b"material,colour,grams\nPLA,grey,640\nTPU,black,210\n".to_vec(),
        ),
        (&project, "robot-dog.stl", model),
        (
            &project,
            "README.md",
            b"# Robot dog\n\nA desk-sized robot dog, printed in six parts.\n".to_vec(),
        ),
        (&project, "assembly.pdf", ext_preview_pdf::sample_pdf(4)),
        (
            &project,
            "parts-list.csv",
            b"part,count,material\nbody,1,PLA\nleg,4,PLA\near,2,TPU\n".to_vec(),
        ),
        (&project, "front.jpg", media::photo_jpeg(640, 480)),
        (
            &project,
            "print-settings.json",
            br#"{"layer":0.2,"infill":15,"supports":false}"#.to_vec(),
        ),
        (
            &project,
            "wag.rs",
            b"fn main() {\n    println!(\"wag\");\n}\n".to_vec(),
        ),
        (&project, "notes.txt", b"Glue the ears last.\n".to_vec()),
    ];
    // From 2026-11-02 09:00 UTC, each entry ninety minutes after the one
    // before; the folders that hold them last.
    let start = SystemTime::UNIX_EPOCH + Duration::from_hours(498_225);
    let stamp = |path: &Path, n: u64| {
        fs::File::open(path)
            .and_then(|file| file.set_modified(start + Duration::from_mins(n * 90)))
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    };
    let mut n = 0;
    for (dir, name, bytes) in files {
        let path = dir.join(name);
        fs::write(&path, bytes).unwrap_or_else(|e| panic!("{e}"));
        stamp(&path, n);
        n += 1;
    }
    for folder in folders.iter().chain([&project, &projects]) {
        stamp(folder, n);
        n += 1;
    }
    (projects, project)
}

/// specs/readme.md *Screenshot*: the picture of the README, a two-pane
/// window with the preview pane open on the sample robot dog, in Light and
/// in Dark at the same size and state. Rendered from the fixture folders
/// above and a fixed configuration (Comfortable, ISO dates), each scheme
/// in a window of its own built the same way, so two runs on the same
/// code give the same picture; `cargo xtask readme` runs this test and
/// copies the two files to `docs/screenshots/`.
#[test]
#[ignore = "needs a wgpu adapter; CI runs it and uploads the images"]
fn screenshots_of_the_readme() {
    for (theme, name) in [(Theme::Light, "light"), (Theme::Dark, "dark")] {
        let mut harness = readme_window(theme);
        shoot(&mut harness, &format!("readme-{name}"));
    }
}

/// The window of the README's picture in `theme`, settled.
fn readme_window(theme: Theme) -> Harness<'static, RickUi> {
    use platform::StandardFolders as _;
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let home = fake
        .standard_folder(platform::StandardFolder::Home)
        .unwrap_or_else(|| panic!("the fake's home"));
    let (projects, project) = readme_tree(&home);
    let host = Arc::new(EguiHost::new());
    let mut app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: vec![
            Arc::new(ext_preview_text::TextPreview::new()),
            Arc::new(ext_preview_code::CodePreview::new()),
            Arc::new(ext_preview_image::ImagePreview::new()),
            Arc::new(ext_preview_pdf::PdfPreview::new()),
            Arc::new(ext_preview_3d::Preview3d::new()),
        ],
        config_dir: Some(fake.config_dir()),
        start: Some(local(&projects)),
    });
    // The fixture configuration, before the first frame.
    for change in [
        SettingsChange::Theme(theme),
        SettingsChange::Density(Density::Comfortable),
        SettingsChange::DateFormat(model::DateFormat::Iso),
    ] {
        app.handle(app::Command::Settings(change), &mut Vec::new());
    }
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1680.0, 920.0))
        .with_pixels_per_point(1.5)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    let send = |harness: &mut Harness<'static, RickUi>, command: app::Command| {
        harness
            .state_mut()
            .app_mut()
            .handle(command, &mut Vec::new());
        settle(harness);
    };
    // Projects on the left; the split's new pane, which has the focus,
    // goes into the project, and the preview pane opens beside it.
    let left = harness.state().app().model().focused;
    send(
        &mut harness,
        app::Command::Split {
            pane: left,
            dir: model::SplitDir::Right,
        },
    );
    let pane = harness.state().app().model().focused;
    assert_ne!(pane, left);
    send(
        &mut harness,
        app::Command::Navigate {
            pane,
            to: local(&project),
        },
    );
    harness.state_mut().set_inspector_width(460.0);
    preview(&mut harness, "robot-dog.stl");
    // The model is drawn off the UI thread: the picture is taken once no
    // render has been in flight for a while and every animation has run
    // out, since a fade a frame short of its end is a picture one step of
    // a colour away from the next.
    let mut quiet = 0;
    step_until(&mut harness, Duration::from_secs(20), |harness| {
        let slot = harness.state().previews().get(pane);
        if slot.is_some_and(|s| s.has_frame() && !s.rendering()) {
            quiet += 1;
        } else {
            quiet = 0;
        }
        quiet >= 20
    });
    std::thread::sleep(Duration::from_millis(800));
    hold_still(&mut harness, 8);
    harness
}
