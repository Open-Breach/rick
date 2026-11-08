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
use model::SelectionChange;
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

    // Two panes, each with its tab strip.
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
    shoot(&mut harness, "two-panes-tab-strips");

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
    });
    fake.add_volume(platform::Volume {
        name: "HOLIDAY".into(),
        mount_point: stick.path().to_path_buf(),
        kind: platform::VolumeKind::Removable,
        fs_type: "exfat".into(),
        device: None,
        total: Some(64 << 30),
        free: Some(21 << 30),
    });
    fake.add_volume(platform::Volume {
        name: "nas".into(),
        mount_point: stick.path().join("nas"),
        kind: platform::VolumeKind::Network,
        fs_type: "cifs".into(),
        device: None,
        total: Some(4 << 40),
        free: Some(1 << 40),
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
