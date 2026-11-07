//! Acceptance (issue 5): each previewer has a fixture set, the score picks
//! the right one by sniff, and a 50 MB image and a 300-page PDF show their
//! first frame within 200 ms without blocking the UI thread.

use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use app::{App, Command, Config, Event};
use egui_kittest::Harness;
use ext::{Extension, Frame, Level, Previewer, Provider, Registry, RenderRequest, UiHost};
use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
use model::SelectionChange;
use platform::{ConfigDir, Fake};
use tests::local;
use types::{CancelToken, Entry, EntryId, Error};
use ui_egui::keys::Intent;
use ui_egui::{EguiHost, RickUi};

/// The four previewers, loaded as the binary loads them.
fn previewers() -> Vec<Arc<dyn Previewer>> {
    let mut registry = Registry::new();
    let extensions: [Arc<dyn Extension>; 4] = [
        Arc::new(ext_preview_text::TextPreview::new()),
        Arc::new(ext_preview_code::CodePreview::new()),
        Arc::new(ext_preview_image::ImagePreview::new()),
        Arc::new(ext_preview_pdf::PdfPreview::new()),
    ];
    for extension in &extensions {
        registry
            .load(&**extension)
            .unwrap_or_else(|e| panic!("{e}"));
    }
    registry.previewers().to_vec()
}

fn extensions() -> Vec<Arc<dyn Extension>> {
    vec![
        Arc::new(ext_preview_text::TextPreview::new()),
        Arc::new(ext_preview_code::CodePreview::new()),
        Arc::new(ext_preview_image::ImagePreview::new()),
        Arc::new(ext_preview_pdf::PdfPreview::new()),
    ]
}

/// A small image whose left half is red and right half blue.
fn sample_image() -> RgbaImage {
    RgbaImage::from_fn(64, 32, |x, _| {
        if x < 32 {
            Rgba([255, 0, 0, 255])
        } else {
            Rgba([0, 0, 255, 255])
        }
    })
}

fn encode(image: &RgbaImage, format: ImageFormat) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    if format == ImageFormat::Jpeg || format == ImageFormat::Bmp {
        DynamicImage::ImageRgba8(image.clone())
            .into_rgb8()
            .write_to(&mut out, format)
            .unwrap_or_else(|e| panic!("{format:?}: {e}"));
    } else {
        image
            .write_to(&mut out, format)
            .unwrap_or_else(|e| panic!("{format:?}: {e}"));
    }
    out.into_inner()
}

/// A JPEG with an EXIF block saying "rotate 90° clockwise".
fn jpeg_rotated(image: &RgbaImage) -> Vec<u8> {
    let jpeg = encode(image, ImageFormat::Jpeg);
    let mut tiff = vec![b'I', b'I', 42, 0, 8, 0, 0, 0];
    tiff.extend_from_slice(&1u16.to_le_bytes());
    tiff.extend_from_slice(&0x0112u16.to_le_bytes());
    tiff.extend_from_slice(&3u16.to_le_bytes());
    tiff.extend_from_slice(&1u32.to_le_bytes());
    tiff.extend_from_slice(&6u16.to_le_bytes());
    tiff.extend_from_slice(&[0, 0]);
    tiff.extend_from_slice(&0u32.to_le_bytes());
    let mut payload = b"Exif\0\0".to_vec();
    payload.extend_from_slice(&tiff);
    let length = u16::try_from(payload.len() + 2).unwrap_or_else(|e| panic!("{e}"));
    let mut out = jpeg[..2].to_vec();
    out.extend_from_slice(&[0xFF, 0xE1]);
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(&payload);
    out.extend_from_slice(&jpeg[2..]);
    out
}

/// One fixture file and the previewer that must claim it.
struct Fixture {
    path: PathBuf,
    previewer: &'static str,
}

/// The fixture set of every previewer, written into `dir`.
fn fixtures(dir: &Path) -> Vec<Fixture> {
    let mut out = Vec::new();
    let mut put = |name: &str, bytes: &[u8], previewer: &'static str| {
        let path = dir.join(name);
        fs::write(&path, bytes).unwrap_or_else(|e| panic!("{e}"));
        out.push(Fixture { path, previewer });
    };
    // Text: UTF-8, Windows-1252, UTF-16 with BOM, no extension, a log.
    put(
        "notes.txt",
        "Hello, wörld.\nSecond line.\n".as_bytes(),
        "preview-text",
    );
    put("latin1.txt", b"caf\xE9 cr\xE8me\n", "preview-text");
    let utf16: Vec<u8> = [0xFF, 0xFE]
        .into_iter()
        .chain("wide text\n".encode_utf16().flat_map(u16::to_le_bytes))
        .collect();
    put("utf16.txt", &utf16, "preview-text");
    put(
        "README",
        b"Plain words without an extension.\n",
        "preview-text",
    );
    put("server.log", b"2026-11-06 info started\n", "preview-text");
    // Code: the spec's languages by extension, plus a shebang.
    put(
        "main.rs",
        b"fn main() {\n    println!(\"hi\");\n}\n",
        "preview-code",
    );
    put("app.ts", b"const x: number = 1;\n", "preview-code");
    put(
        "index.html",
        b"<html><body><p>hi</p></body></html>\n",
        "preview-code",
    );
    put("data.json", b"{\"a\": [1, 2, 3]}\n", "preview-code");
    put("config.yaml", b"a: 1\nb:\n  - x\n", "preview-code");
    put("Main.java", b"class Main { }\n", "preview-code");
    put(
        "run",
        b"#!/usr/bin/env python3\nprint('hi')\n",
        "preview-code",
    );
    // Images: every format, EXIF rotation, and a magic-only name.
    let image = sample_image();
    put(
        "pic.png",
        &encode(&image, ImageFormat::Png),
        "preview-image",
    );
    put(
        "pic.bmp",
        &encode(&image, ImageFormat::Bmp),
        "preview-image",
    );
    put(
        "pic.webp",
        &encode(&image, ImageFormat::WebP),
        "preview-image",
    );
    put(
        "pic.tiff",
        &encode(&image, ImageFormat::Tiff),
        "preview-image",
    );
    put(
        "pic.gif",
        &encode(&image, ImageFormat::Gif),
        "preview-image",
    );
    put("rotated.jpg", &jpeg_rotated(&image), "preview-image");
    put("noext", &encode(&image, ImageFormat::Png), "preview-image");
    // PDF: by header, and by header alone.
    put("doc.pdf", &ext_preview_pdf::sample_pdf(3), "preview-pdf");
    put("odd.bin", &ext_preview_pdf::sample_pdf(1), "preview-pdf");
    out
}

fn provider() -> Arc<dyn Provider> {
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    Arc::new(vfs_local::LocalProvider::new(fake))
}

/// The entry and the first 4 KiB of a file, as the kernel sees them.
fn sniff(provider: &Arc<dyn Provider>, path: &Path) -> (Entry, Vec<u8>) {
    let loc = local(path);
    runtime::block_on(async {
        let entry = provider.stat(&loc).await.unwrap_or_else(|e| panic!("{e}"));
        let bytes = ext::read_bytes(&**provider, &loc, Some(4096), &CancelToken::new())
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        (entry, bytes)
    })
}

fn best<'a>(
    previewers: &'a [Arc<dyn Previewer>],
    entry: &Entry,
    sniff: &[u8],
) -> Option<&'a Arc<dyn Previewer>> {
    previewers
        .iter()
        .map(|p| (p.score(entry, sniff), p))
        .filter(|(score, _)| *score > 0)
        .max_by_key(|(score, _)| *score)
        .map(|(_, p)| p)
}

#[test]
fn the_score_picks_the_right_previewer_by_sniff_and_every_fixture_opens() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let fixtures = fixtures(dir.path());
    let previewers = previewers();
    let provider = provider();
    let pdfium = ext_preview_pdf::available();
    let req = RenderRequest {
        width: 200,
        height: 200,
        scale: 1.0,
        dark: false,
    };
    for fixture in &fixtures {
        let name = fixture
            .path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let (entry, bytes) = sniff(&provider, &fixture.path);
        let picked =
            best(&previewers, &entry, &bytes).unwrap_or_else(|| panic!("{name}: no previewer"));
        assert_eq!(picked.id().0, fixture.previewer, "{name}");
        if fixture.previewer == "preview-pdf" && !pdfium {
            continue;
        }
        let mut doc = runtime::block_on(picked.open(
            &local(&fixture.path),
            Arc::clone(&provider),
            CancelToken::new(),
        ))
        .unwrap_or_else(|e| panic!("{name}: {e}"));
        let frame = doc.render(0, req).unwrap_or_else(|e| panic!("{name}: {e}"));
        match (fixture.previewer, frame) {
            ("preview-text" | "preview-code", Frame::Text(block)) => {
                assert!(!block.text.is_empty(), "{name}");
                if name == "latin1.txt" {
                    assert!(block.text.contains("café"), "{name}: {:?}", block.text);
                }
                if name == "utf16.txt" {
                    assert!(
                        block.text.starts_with("wide text"),
                        "{name}: {:?}",
                        block.text
                    );
                }
                if fixture.previewer == "preview-code" {
                    assert!(!block.spans.is_empty(), "{name}: no colours");
                }
            }
            (
                "preview-image" | "preview-pdf",
                Frame::Rgba {
                    width,
                    height,
                    data,
                },
            ) => {
                assert_eq!(data.len(), width as usize * height as usize * 4, "{name}");
                if name == "rotated.jpg" {
                    assert_eq!(
                        (width, height),
                        (32, 64),
                        "{name}: EXIF orientation applied"
                    );
                }
            }
            (kind, other) => panic!("{name}: {kind} produced {other:?}"),
        }
        assert_eq!(doc.sections()[0].title, "File info", "{name}");
        assert!(!doc.status().is_empty(), "{name}: status facts");
    }
    // Nothing claims noise.
    let noise: Vec<u8> = (0..4096u32)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 13).to_le_bytes()[0])
        .collect();
    let noise_path = dir.path().join("noise.dat");
    fs::write(&noise_path, &noise).unwrap_or_else(|e| panic!("{e}"));
    let (entry, bytes) = sniff(&provider, &noise_path);
    assert!(
        best(&previewers, &entry, &bytes).is_none(),
        "noise has no previewer"
    );
    if !pdfium {
        assert!(
            std::env::var_os("RICK_REQUIRE_PDFIUM").is_none(),
            "pdfium required but not found"
        );
    }
}

/// A host that answers nothing; the app test asks no questions.
struct Quiet;

impl UiHost for Quiet {
    fn notify(&self, _: Level, _: &str) {}
    fn confirm(&self, _: &str, _: &str) -> bool {
        false
    }
    fn prompt(&self, _: &str, _: &str) -> Option<String> {
        None
    }
    fn show(&self, _title: &str, _text: &str) {}
}

/// The app follows the focused pane's single selected file.
#[test]
fn the_app_opens_a_preview_for_the_selection() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let fixtures = fixtures(dir.path());
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut app = App::new(Config {
        platform: fake.clone(),
        ui: Arc::new(Quiet),
        extensions: extensions(),
        config_dir: Some(fake.config_dir()),
        start: Some(local(dir.path())),
    });
    let pane = app.model().focused;
    let mut events = Vec::new();
    settle(&mut app, &mut events);
    let id_of = |app: &App, name: &str| -> EntryId {
        app.model()
            .active_tab(pane)
            .and_then(|(_, t)| t.listing.rows().find(|e| e.name == name).map(|e| e.id))
            .unwrap_or_else(|| panic!("no entry {name}"))
    };

    let png = id_of(&app, "pic.png");
    events.clear();
    app.handle(
        Command::Select {
            pane,
            change: SelectionChange::Set(vec![png]),
        },
        &mut events,
    );
    assert_eq!(app.preview_target(), Some((pane, png)));
    settle(&mut app, &mut events);
    let ready = events
        .iter()
        .any(|e| matches!(e, Event::PreviewReady { pane: p, .. } if *p == pane));
    assert!(ready, "{events:?}");
    let doc_info = events.iter().find_map(|e| match e {
        Event::PreviewReady { doc, .. } => Some(doc.sections().remove(0).rows),
        _ => None,
    });
    assert!(doc_info.is_some_and(|i| i.iter().any(|(k, v)| k == "Dimensions" && v == "64 × 32")));

    // Two files: no preview. A folder: no preview. Noise: a failure event.
    let rs = id_of(&app, "main.rs");
    app.handle(
        Command::Select {
            pane,
            change: SelectionChange::Set(vec![png, rs]),
        },
        &mut events,
    );
    assert_eq!(app.preview_target(), None);
    let noise = dir.path().join("noise.dat");
    fs::write(&noise, [0u8, 159, 7, 255, 3, 128, 64, 1].repeat(64))
        .unwrap_or_else(|e| panic!("{e}"));
    app.handle(Command::Refresh(pane), &mut events);
    settle(&mut app, &mut events);
    let noise_id = id_of(&app, "noise.dat");
    events.clear();
    app.handle(
        Command::Select {
            pane,
            change: SelectionChange::Set(vec![noise_id]),
        },
        &mut events,
    );
    settle(&mut app, &mut events);
    assert!(
        events.iter().any(|e| matches!(e, Event::PreviewFailed { pane: p, error: Error::Unsupported(_) } if *p == pane)),
        "{events:?}"
    );
    assert!(
        app.model().jobs.is_empty(),
        "previews stay out of the job table"
    );
    drop(fixtures);
}

fn settle(app: &mut App, events: &mut Vec<Event>) {
    let deadline = Instant::now() + Duration::from_secs(20);
    runtime::block_on(async {
        loop {
            let quiet = tokio::time::timeout(Duration::from_millis(80), app.tick(events))
                .await
                .is_err();
            if quiet && !app.is_busy() {
                break;
            }
            assert!(Instant::now() < deadline, "did not settle");
        }
    });
}

/// A 48 MB uncompressed bitmap: 4096 × 4096 × 3 bytes.
fn big_bitmap(path: &Path) {
    let image = image::RgbImage::from_fn(4096, 4096, |x, y| {
        image::Rgb([
            u8::try_from(x / 16).unwrap_or(255),
            u8::try_from(y / 16).unwrap_or(255),
            128,
        ])
    });
    let mut out = Cursor::new(Vec::new());
    image
        .write_to(&mut out, ImageFormat::Bmp)
        .unwrap_or_else(|e| panic!("{e}"));
    fs::write(path, out.into_inner()).unwrap_or_else(|e| panic!("{e}"));
}

fn ui_harness(start: &Path) -> Harness<'static, RickUi> {
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = App::new(Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: extensions(),
        config_dir: Some(fake.config_dir()),
        start: Some(local(start)),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 900.0))
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    while harness.state().app().is_busy() {
        harness.step();
        std::thread::sleep(Duration::from_millis(5));
    }
    harness.step();
    harness
}

/// Selects `name` in the harness and steps until its preview has a frame,
/// returning the time it took and every frame's duration.
fn preview_timing(harness: &mut Harness<'_, RickUi>, name: &str) -> (Duration, Vec<Duration>) {
    let pane = harness.state().app().model().focused;
    let id = harness
        .state()
        .app()
        .model()
        .active_tab(pane)
        .and_then(|(_, t)| t.listing.rows().find(|e| e.name == name).map(|e| e.id))
        .unwrap_or_else(|| panic!("no entry {name}"));
    let mut frames = Vec::new();
    let started = Instant::now();
    harness.state_mut().app_mut().handle(
        Command::Select {
            pane,
            change: SelectionChange::Set(vec![id]),
        },
        &mut Vec::new(),
    );
    let first_frame = loop {
        let frame_started = Instant::now();
        harness.step();
        frames.push(frame_started.elapsed());
        if harness
            .state()
            .previews()
            .get(pane)
            .is_some_and(ui_egui::preview::Slot::has_frame)
        {
            break started.elapsed();
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "{name}: no frame after 10 s: {:?}",
            harness.state().previews().get(pane)
        );
    };
    (first_frame, frames)
}

/// The spec's budget: the first frame within 200 ms, and no UI frame over
/// 16.6 ms while the decode runs elsewhere (QUESTIONS.md 30).
#[test]
#[ignore = "writes a 48 MB bitmap and a 300-page PDF; run with --ignored in release"]
fn a_50_mb_image_and_a_300_page_pdf_show_a_first_frame_within_200_ms_without_blocking_the_ui() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    big_bitmap(&dir.path().join("huge.bmp"));
    fs::write(
        dir.path().join("book.pdf"),
        ext_preview_pdf::sample_pdf(300),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let mut harness = ui_harness(dir.path());
    let pane = harness.state().app().model().focused;
    harness.state_mut().queue(Intent::ToggleInspector, pane);
    harness.step();
    assert!(harness.state().inspector_open());

    let (image_first, image_frames) = preview_timing(&mut harness, "huge.bmp");
    println!(
        "48 MB BMP: first frame after {image_first:?} over {} UI frames",
        image_frames.len()
    );

    let mut all_frames = image_frames;
    let pdf_first = if ext_preview_pdf::available() {
        let (first, frames) = preview_timing(&mut harness, "book.pdf");
        println!(
            "300-page PDF: first frame after {first:?} over {} UI frames",
            frames.len()
        );
        let slot = harness
            .state()
            .previews()
            .get(pane)
            .unwrap_or_else(|| panic!("slot"));
        assert!(format!("{slot:?}").contains("pages: Some(300)"), "{slot:?}");
        all_frames.extend(frames);
        Some(first)
    } else {
        assert!(
            std::env::var_os("RICK_REQUIRE_PDFIUM").is_none(),
            "pdfium required but not found"
        );
        None
    };

    all_frames.sort_unstable();
    let worst = all_frames[all_frames.len() - 1];
    let p95 = all_frames[all_frames.len() * 95 / 100];
    println!("UI frames while previewing: p95 {p95:?}, worst {worst:?}");
    assert!(
        image_first < Duration::from_millis(200),
        "image first frame {image_first:?}"
    );
    if let Some(first) = pdf_first {
        assert!(
            first < Duration::from_millis(200),
            "PDF first frame {first:?}"
        );
    }
    assert!(
        worst < Duration::from_micros(16_600),
        "a UI frame took {worst:?}"
    );
}
