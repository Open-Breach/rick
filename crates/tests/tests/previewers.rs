//! Acceptance (issue 18): the previewer plugins. Every plugin loads through
//! the host; a fixture per format opens with the right previewer by sniff;
//! a DOCX converts within 5 s when LibreOffice is present (CI installs it
//! on Linux; macOS and Windows skip with a note); an MP4 plays and seeks
//! and a 3-minute MP3 gets its waveform within 500 ms when GStreamer is
//! present (Linux CI); a 10 000-entry zip lists its first screen within
//! 100 ms; a window of a 1 000 000-row CSV comes within a frame; Markdown
//! with every element renders without panics; and no UI frame goes over
//! 16.6 ms while any of them decodes.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use app::{App, Command, Config};
use egui_kittest::Harness;
use ext::{Content, Extension, Frame, Previewer, Provider, Registry, RenderRequest};
use ext_preview_archive::fixtures as archives;
use model::SelectionChange;
use platform::{ConfigDir, Fake};
use plugins_native::{Host, Library, MANIFEST_FILE, Options, PluginStatus, PluginToml, Source};
use tests::local;
use types::{CancelToken, Entry};
use ui_egui::{EguiHost, RickUi};

/// The plugin crates of this milestone, by library stem and extension id.
const PLUGINS: [(&str, &str); 4] = [
    ("ext_preview_office", "preview-office"),
    ("ext_preview_media", "preview-media"),
    ("ext_preview_archive", "preview-archive"),
    ("ext_preview_csv", "preview-csv"),
];

/// The directory the test binary runs from, where cargo puts every
/// dependency's `cdylib` under its plain name.
fn deps_dir() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_else(|e| panic!("{e}"));
    exe.parent()
        .unwrap_or_else(|| panic!("no parent of {}", exe.display()))
        .to_owned()
}

fn library_of(stem: &str) -> PathBuf {
    let path = deps_dir().join(format!(
        "{}{stem}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    assert!(path.is_file(), "not built: {}", path.display());
    path
}

/// Installs the library of the crate `stem` into `plugins_dir` with the
/// manifest the bundler would write.
fn install(plugins_dir: &Path, stem: &str) -> PathBuf {
    let library = library_of(stem);
    let mut toml: PluginToml = plugins_native::describe(&library).unwrap_or_else(|e| panic!("{e}"));
    toml.library = Library::for_crate(stem);
    let dir = plugins_dir.join(&toml.id);
    toml.write(&dir.join(MANIFEST_FILE))
        .unwrap_or_else(|e| panic!("{e}"));
    let file = toml
        .library
        .for_os(std::env::consts::OS)
        .unwrap_or_else(|| panic!("no library name for this OS"));
    fs::copy(&library, dir.join(file)).unwrap_or_else(|e| panic!("{e}"));
    dir
}

/// The four plugins installed into a temporary plugins directory and
/// loaded by the host, which stays alive (and so mapped) for the process.
fn load_plugins() -> &'static Host {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let plugins = dir.path().join("plugins");
    for (stem, _) in PLUGINS {
        install(&plugins, stem);
    }
    let host = Host::load(&Options {
        dirs: vec![(Source::BesideBinary, plugins)],
        disabled: Vec::new(),
        compiled_in: Vec::new(),
        platform: None,
        prompts: None,
    });
    for (_, id) in PLUGINS {
        let row = host
            .plugins()
            .iter()
            .find(|p| p.id == id)
            .unwrap_or_else(|| panic!("no plugin {id}: {:?}", host.plugins()));
        assert_eq!(row.status, PluginStatus::Loaded, "{id}: {:?}", row.status);
    }
    // The libraries stay mapped: the temporary directory may go, the
    // mappings stay (QUESTIONS.md 85).
    let _ = dir.keep();
    Box::leak(Box::new(host))
}

/// The built-in previewers.
fn built_ins() -> Vec<Arc<dyn Extension>> {
    vec![
        Arc::new(ext_preview_text::TextPreview::new()),
        Arc::new(ext_preview_code::CodePreview::new()),
        Arc::new(ext_preview_image::ImagePreview::new()),
        Arc::new(ext_preview_pdf::PdfPreview::new()),
    ]
}

/// Every previewer the binary would have: the built-ins and the plugins.
fn previewers(host: &Host) -> Vec<Arc<dyn Previewer>> {
    let mut registry = Registry::new();
    for extension in built_ins().into_iter().chain(host.extensions()) {
        registry.load(&*extension).unwrap_or_else(|e| panic!("{e}"));
    }
    registry.previewers().to_vec()
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

fn open(
    previewer: &Arc<dyn Previewer>,
    provider: &Arc<dyn Provider>,
    path: &Path,
) -> Box<dyn ext::PreviewDoc> {
    runtime::block_on(previewer.open(&local(path), Arc::clone(provider), CancelToken::new()))
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

const ALL_MARKDOWN: &str = "# Title\n\nA paragraph with *emphasis*, **strong**, ~~struck~~, `code` and a [link](https://example.org).  \nHard break.\n\n- one\n- two\n  - nested\n\n3. third\n4. fourth\n\n- [x] done\n- [ ] open\n\n> quote\n\n```rust\nfn main() {}\n```\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\n![Alt](picture.png)\n\n---\n\n## End\n";

/// One fixture file and the previewer that must claim it.
struct Fixture {
    path: PathBuf,
    previewer: &'static str,
}

/// The fixture set of every previewer of this milestone, written into
/// `dir`; the media encodings only where GStreamer and its encoders are.
fn fixtures(dir: &Path) -> Vec<Fixture> {
    let mut out = Vec::new();
    // The files written here; the media encodings follow, outside the
    // closure's borrow of `out`.
    {
        let mut put = |name: &str, bytes: &[u8], previewer: &'static str| {
            let path = dir.join(name);
            fs::write(&path, bytes).unwrap_or_else(|e| panic!("{e}"));
            out.push(Fixture { path, previewer });
        };
        put("README.md", ALL_MARKDOWN.as_bytes(), "preview-code");
        put(
            "page.html",
            b"<html><body><h1>Hi</h1><p>Text <b>bold</b></p><ul><li>a</li></ul></body></html>",
            "preview-code",
        );
        let tar = archives::sample_tar(4096);
        put("a.zip", &archives::sample_zip(), "preview-archive");
        put("a.tar", &tar, "preview-archive");
        put("a.tar.gz", &archives::gzip(&tar), "preview-archive");
        put("a.tar.zst", &archives::zstd_bytes(&tar), "preview-archive");
        put("a.tar.xz", &archives::xz(&tar), "preview-archive");
        put("a.tar.bz2", &archives::bzip2_bytes(&tar), "preview-archive");
        put("a.tar.lz4", &archives::lz4(&tar), "preview-archive");
        put(
            "a.7z",
            &archives::sample_7z(false, false),
            "preview-archive",
        );
        put("a.rar", &archives::sample_rar(), "preview-archive");
        put("a.iso", &archives::sample_iso(true), "preview-archive");
        put("data.csv", b"name,age\nAda,36\nAlan,41\n", "preview-csv");
        put("data.tsv", b"name\tage\nAda\t36\n", "preview-csv");
        for kind in ext_preview_office::Kind::ALL {
            let name = format!("doc.{}", kind.extension());
            put(
                &name,
                &ext_preview_office::fixtures::sample(kind, "Fixture", "Rick", 2),
                "preview-office",
            );
        }
        put(
            "tone.wav",
            &ext_preview_media::fixtures::sample_wav(1.0, 8000, 440.0),
            "preview-media",
        );
    }
    #[cfg(feature = "gstreamer")]
    {
        use ext_preview_media::fixtures::encode;
        for (name, outcome) in [
            ("tone.mp3", encode::mp3(&dir.join("tone.mp3"), 2)),
            ("clip.mp4", encode::mp4(&dir.join("clip.mp4"), 30, 160, 120)),
            ("clip.avi", encode::avi(&dir.join("clip.avi"), 30, 160, 120)),
        ] {
            match outcome {
                Ok(()) => out.push(Fixture {
                    path: dir.join(name),
                    previewer: "preview-media",
                }),
                Err(error) => {
                    eprintln!("{name}: not encoded ({error}); skipped");
                    assert!(
                        std::env::var_os("RICK_REQUIRE_GSTREAMER").is_none(),
                        "{name}: GStreamer required but {error}"
                    );
                }
            }
        }
    }
    out
}

#[test]
fn the_four_previewer_plugins_load_through_the_host() {
    let host = load_plugins();
    let previewers = previewers(host);
    assert_eq!(previewers.len(), 8, "four built-ins and four plugins");
    for (_, id) in PLUGINS {
        assert!(
            previewers.iter().any(|p| p.id().0 == id),
            "{id} registered no previewer"
        );
    }
    assert_eq!(host.extensions().len(), 4);
}

#[test]
fn every_fixture_opens_with_the_right_previewer_by_sniff() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let fixtures = fixtures(dir.path());
    let host = load_plugins();
    let previewers = previewers(host);
    let provider = provider();
    let req = RenderRequest {
        width: 300,
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
        let mut doc = open(picked, &provider, &fixture.path);
        let frame = doc.render(0, req).unwrap_or_else(|e| panic!("{name}: {e}"));
        let content = doc.content();
        match fixture.previewer {
            "preview-archive" => {
                let Some(Content::Listing(listing)) = content else {
                    panic!("{name}: no listing");
                };
                assert!(!listing.entries.is_empty(), "{name}");
                assert!(matches!(frame, Frame::Text(_)), "{name}: text fallback");
                assert_eq!(doc.status().len(), 3, "{name}: {:?}", doc.status());
            }
            "preview-csv" => {
                let Some(Content::Grid(grid)) = content else {
                    panic!("{name}: no grid");
                };
                assert_eq!(grid.header()[0], "name", "{name}");
                assert!(grid.rows() >= 1, "{name}");
                assert!(matches!(frame, Frame::Text(_)), "{name}: text fallback");
            }
            "preview-office" => {
                assert!(content.is_none(), "{name}");
                let status = doc.status();
                assert!(
                    ext_preview_office::Kind::ALL
                        .iter()
                        .any(|k| k.name() == status[0]),
                    "{name}: {status:?}"
                );
                let rows = doc.sections().remove(0).rows;
                assert!(
                    rows.iter().any(|(k, v)| k == "Title" && v == "Fixture"),
                    "{name}: {rows:?}"
                );
                match frame {
                    Frame::Rgba { .. } => assert_eq!(doc.pages(), Some(2), "{name}"),
                    Frame::Text(block) => assert!(
                        block.text.contains("LibreOffice") || block.text.contains("pdfium"),
                        "{name}: {}",
                        block.text
                    ),
                    Frame::External(id) => panic!("{name}: external texture {id:?}"),
                }
            }
            "preview-media" => {
                assert!(content.is_none(), "{name}");
                assert!(
                    doc.status()[0].ends_with("audio") || doc.status()[0].ends_with("video"),
                    "{name}: {:?}",
                    doc.status()
                );
                assert!(
                    matches!(frame, Frame::Rgba { .. } | Frame::Text(_)),
                    "{name}"
                );
            }
            "preview-code" => {
                let Some(Content::Document(document)) = content else {
                    panic!("{name}: no document");
                };
                assert!(!document.blocks.is_empty(), "{name}");
                assert!(matches!(frame, Frame::Text(_)), "{name}: source fallback");
            }
            other => panic!("{name}: unexpected previewer {other}"),
        }
        assert_eq!(doc.sections()[0].title.get(), "File info", "{name}");
        assert!(!doc.status().is_empty(), "{name}: status facts");
    }
}

#[test]
fn markdown_with_every_element_renders_without_panics() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let path = dir.path().join("all.md");
    fs::write(&path, ALL_MARKDOWN).unwrap_or_else(|e| panic!("{e}"));
    let previewer: Arc<dyn Previewer> = Arc::new(ext_preview_code::CodePreviewer);
    let provider = provider();
    let doc = open(&previewer, &provider, &path);
    let Some(Content::Document(document)) = doc.content() else {
        panic!("no document");
    };
    assert!(document.blocks.len() >= 11, "{:?}", document.blocks);
    let kinds: Vec<&str> = document
        .blocks
        .iter()
        .map(|b| match b {
            ext::Block::Heading { .. } => "heading",
            ext::Block::Paragraph(_) => "paragraph",
            ext::Block::List { .. } => "list",
            ext::Block::Code { .. } => "code",
            ext::Block::Quote(_) => "quote",
            ext::Block::Table { .. } => "table",
            ext::Block::Image(_) => "image",
            ext::Block::Rule => "rule",
        })
        .collect();
    for kind in [
        "heading",
        "paragraph",
        "list",
        "code",
        "quote",
        "table",
        "image",
        "rule",
    ] {
        assert!(kinds.contains(&kind), "no {kind} in {kinds:?}");
    }
    // Odd input, through the same path.
    for odd in [
        "",
        "#",
        "- ",
        "| a |\n|",
        "```\nunterminated",
        "<table><td>x",
    ] {
        fs::write(&path, odd).unwrap_or_else(|e| panic!("{e}"));
        let doc = open(&previewer, &provider, &path);
        assert!(doc.content().is_some() || odd.is_empty(), "{odd:?}");
    }
}

/// Converts a fresh three-page DOCX (a new path each run, so the cache is
/// never hit) and renders its first page.
#[test]
#[ignore = "timing; run with --ignored in release"]
fn a_docx_converts_within_5_s_when_libreoffice_is_present() {
    if !ext_preview_office::available() {
        eprintln!("LibreOffice not installed; the office conversion is not measured here");
        assert!(
            std::env::var_os("RICK_REQUIRE_LIBREOFFICE").is_none(),
            "LibreOffice required but not found"
        );
        return;
    }
    if !ext_preview_office::pdf::available() {
        eprintln!("pdfium not available; the office conversion is not measured here");
        assert!(
            std::env::var_os("RICK_REQUIRE_PDFIUM").is_none(),
            "pdfium required but not found"
        );
        return;
    }
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let previewer: Arc<dyn Previewer> = Arc::new(ext_preview_office::OfficePreviewer);
    let provider = provider();
    // One conversion to start LibreOffice's profile, which the first run of
    // a fresh install pays once.
    let warm = dir.path().join("warm.docx");
    fs::write(
        &warm,
        ext_preview_office::fixtures::sample_docx("Warm", "Rick", 1),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let _ = open(&previewer, &provider, &warm);
    let mut samples = Vec::new();
    for run in 0..tests::bench::runs() {
        let path = dir.path().join(format!("report-{run}.docx"));
        fs::write(
            &path,
            ext_preview_office::fixtures::sample_docx(&format!("Report {run}"), "Rick", 3),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        let started = Instant::now();
        let mut doc = open(&previewer, &provider, &path);
        let frame = doc
            .render(
                0,
                RenderRequest {
                    width: 400,
                    height: 600,
                    scale: 1.0,
                    dark: false,
                },
            )
            .unwrap_or_else(|e| panic!("{e}"));
        samples.push(started.elapsed());
        assert_eq!(doc.pages(), Some(3), "{:?}", doc.sections());
        assert!(matches!(frame, Frame::Rgba { .. }));
    }
    tests::bench::record("office-convert", &samples);
}

/// Plays the opened video, waits for the position to advance, pauses,
/// seeks to two seconds and checks a frame at the native size.
#[cfg(feature = "gstreamer")]
fn play_and_seek(mut doc: Box<dyn ext::PreviewDoc>) {
    let status = doc.status();
    let transport = doc
        .transport()
        .unwrap_or_else(|| panic!("no transport: {status:?}"));
    transport.play();
    let deadline = Instant::now() + Duration::from_secs(10);
    while doc
        .transport()
        .is_some_and(|t| t.position() < Duration::from_millis(300))
    {
        assert!(Instant::now() < deadline, "playback did not advance");
        std::thread::sleep(Duration::from_millis(20));
    }
    let transport = doc.transport().unwrap_or_else(|| panic!("transport"));
    transport.pause();
    transport.seek(Duration::from_secs(2));
    let deadline = Instant::now() + Duration::from_secs(10);
    while doc
        .transport()
        .is_some_and(|t| t.position().abs_diff(Duration::from_secs(2)) > Duration::from_millis(150))
    {
        assert!(Instant::now() < deadline, "seek did not land");
        std::thread::sleep(Duration::from_millis(20));
    }
    let frame = doc
        .render(
            0,
            RenderRequest {
                width: 100,
                height: 100,
                scale: 1.0,
                dark: false,
            },
        )
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(
        matches!(
            frame,
            Frame::Rgba {
                width: 320,
                height: 240,
                ..
            }
        ),
        "a frame at native size"
    );
}

#[cfg(feature = "gstreamer")]
#[test]
#[ignore = "timing; run with --ignored in release"]
fn an_mp4_plays_and_seeks_and_a_3_minute_mp3_gets_its_waveform_within_500_ms() {
    use ext_preview_media::fixtures::encode;
    use ext_preview_media::gst::GstError;
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let mp4 = dir.path().join("clip.mp4");
    let mp3 = dir.path().join("long.mp3");
    let skip = |outcome: Result<(), GstError>, what: &str| -> bool {
        match outcome {
            Ok(()) => false,
            Err(error) => {
                eprintln!("{what}: {error}; not measured here");
                assert!(
                    std::env::var_os("RICK_REQUIRE_GSTREAMER").is_none(),
                    "{what}: GStreamer required but {error}"
                );
                true
            }
        }
    };
    if skip(encode::mp4(&mp4, 90, 320, 240), "mp4 fixture")
        || skip(encode::mp3(&mp3, 180), "mp3 fixture")
    {
        return;
    }
    let host = load_plugins();
    let previewers = previewers(host);
    let provider = provider();
    let (entry, bytes) = sniff(&provider, &mp4);
    let media = best(&previewers, &entry, &bytes).unwrap_or_else(|| panic!("no previewer"));
    assert_eq!(media.id().0, "preview-media");

    // Plays and seeks, through the plugin as the host loaded it.
    play_and_seek(open(media, &provider, &mp4));

    // The waveform of three minutes of MP3: open and render, as the pane
    // does when the file is selected.
    let mut samples = Vec::new();
    for _ in 0..tests::bench::runs() {
        let started = Instant::now();
        let mut doc = open(media, &provider, &mp3);
        let frame = doc
            .render(
                0,
                RenderRequest {
                    width: 600,
                    height: 120,
                    scale: 1.0,
                    dark: true,
                },
            )
            .unwrap_or_else(|e| panic!("{e}"));
        samples.push(started.elapsed());
        assert!(
            matches!(
                frame,
                Frame::Rgba {
                    width: 600,
                    height: 120,
                    ..
                }
            ),
            "the waveform picture"
        );
        assert_eq!(doc.status()[1], "3:00", "{:?}", doc.status());
    }
    tests::bench::record("waveform", &samples);
}

#[test]
#[ignore = "timing; run with --ignored in release"]
fn a_10000_entry_zip_lists_its_first_screen_within_100_ms() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let path = dir.path().join("many.zip");
    fs::write(&path, archives::big_zip(10_000)).unwrap_or_else(|e| panic!("{e}"));
    let previewer: Arc<dyn Previewer> = Arc::new(ext_preview_archive::ArchivePreviewer);
    let provider = provider();
    let mut samples = Vec::new();
    for _ in 0..tests::bench::runs() {
        let started = Instant::now();
        let doc = open(&previewer, &provider, &path);
        let Some(Content::Listing(listing)) = doc.content() else {
            panic!("no listing");
        };
        // The first screen is the first rows of the tree, folders first.
        let first_screen: Vec<&str> = listing
            .entries
            .iter()
            .take(40)
            .map(|e| e.name.as_str())
            .collect();
        samples.push(started.elapsed());
        assert_eq!(
            listing.entries.len(),
            10_000 + 313,
            "entries and their folders"
        );
        assert_eq!(first_screen[0], "dir0000");
        assert_eq!(first_screen[1], "file00000.txt");
        assert_eq!(doc.status()[0], "10313 entries");
    }
    tests::bench::record("zip-first", &samples);
}

#[test]
#[ignore = "writes a 1 000 000-row CSV; run with --ignored in release"]
fn a_million_row_csv_answers_any_window_within_a_frame() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let path = dir.path().join("rows.csv");
    {
        use std::io::Write as _;
        let mut file =
            std::io::BufWriter::new(fs::File::create(&path).unwrap_or_else(|e| panic!("{e}")));
        writeln!(file, "id,name,city,score,note").unwrap_or_else(|e| panic!("{e}"));
        for i in 0..1_000_000u32 {
            writeln!(
                file,
                "{i},\"Person {i}\",City {},{}.{},\"a note, quoted\"",
                i % 977,
                i % 100,
                i % 7
            )
            .unwrap_or_else(|e| panic!("{e}"));
        }
    }
    let previewer: Arc<dyn Previewer> = Arc::new(ext_preview_csv::CsvPreviewer);
    let provider = provider();
    let started = Instant::now();
    let doc = open(&previewer, &provider, &path);
    println!(
        "1 000 000-row CSV opened and indexed in {:?}",
        started.elapsed()
    );
    let Some(Content::Grid(grid)) = doc.content() else {
        panic!("no grid");
    };
    assert_eq!(grid.rows(), 1_000_000);
    assert_eq!(grid.header(), ["id", "name", "city", "score", "note"]);
    // A screen of rows at offsets all over the file, as scrolling asks.
    let mut samples = Vec::new();
    for _ in 0..tests::bench::runs() {
        let mut worst = Duration::ZERO;
        for k in 0..50u32 {
            let offset = (u64::from(k) * 1_000_000 / 50) as usize;
            let started = Instant::now();
            let rows = grid.cells(offset..offset + 60);
            worst = worst.max(started.elapsed());
            assert_eq!(rows.len(), 60.min(1_000_000 - offset));
            assert_eq!(rows[0][0], offset.to_string());
            assert_eq!(rows[0][4], "a note, quoted");
        }
        samples.push(worst);
    }
    tests::bench::record("csv-window", &samples);
}

/// Runs the UI with the built-ins and the plugins on a folder of fixtures.
fn ui_harness(start: &Path, host: &Host) -> Harness<'static, RickUi> {
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let ui = Arc::new(EguiHost::new());
    let mut extensions = built_ins();
    extensions.extend(host.extensions());
    let app = App::new(Config {
        platform: fake.clone(),
        ui: ui.clone(),
        extensions,
        config_dir: Some(fake.config_dir()),
        start: Some(local(start)),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 900.0))
        .build_eframe(move |cc| RickUi::new(app, ui, &cc.egui_ctx));
    while harness.state().app().is_busy() {
        harness.step();
        std::thread::sleep(Duration::from_millis(5));
    }
    harness.step();
    harness
}

/// Selects `name` and steps until its preview has a frame, returning every
/// UI frame's duration.
fn preview_frames(harness: &mut Harness<'_, RickUi>, name: &str) -> Vec<Duration> {
    let pane = harness.state().app().model().focused;
    let id = harness
        .state()
        .app()
        .model()
        .active_tab(pane)
        .and_then(|(_, t)| t.listing.rows().find(|e| e.name == name).map(|e| e.id))
        .unwrap_or_else(|| panic!("no entry {name}"));
    harness.state_mut().app_mut().handle(
        Command::Select {
            pane,
            change: SelectionChange::Set(vec![id]),
        },
        &mut Vec::new(),
    );
    let started = Instant::now();
    let mut frames = Vec::new();
    loop {
        let frame_started = Instant::now();
        harness.step();
        frames.push(frame_started.elapsed());
        let slot = harness.state().previews().get(pane);
        if slot.is_some_and(ui_egui::preview::Slot::has_frame) {
            break;
        }
        if slot.is_some_and(|s| format!("{s:?}").contains("error: Some")) {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "{name}: no frame after 30 s: {slot:?}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    frames
}

/// The spec's budget: no UI frame over 16.6 ms while any plugin decodes
/// (the decode runs in the preview job; the pane shows a placeholder).
#[test]
#[ignore = "timing; run with --ignored in release"]
fn no_ui_frame_over_16_ms_while_the_previewers_decode() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let fixtures = fixtures(dir.path());
    fs::write(dir.path().join("many.zip"), archives::big_zip(10_000))
        .unwrap_or_else(|e| panic!("{e}"));
    let host = load_plugins();
    let mut harness = ui_harness(dir.path(), host);
    let pane = harness.state().app().model().focused;
    harness
        .state_mut()
        .queue(ui_egui::keys::Intent::ToggleInspector, pane);
    harness.step();
    assert!(harness.state().inspector_open());
    let mut names: Vec<String> = fixtures
        .iter()
        .map(|f| {
            f.path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.push("many.zip".to_owned());
    let mut samples = Vec::new();
    for _ in 0..tests::bench::runs() {
        let mut all = Vec::new();
        for name in &names {
            let frames = preview_frames(&mut harness, name);
            println!("{name}: {} UI frames until the preview", frames.len());
            all.extend(frames);
            // Clear, so the next file is opened anew.
            harness.state_mut().app_mut().handle(
                Command::Select {
                    pane,
                    change: SelectionChange::Set(Vec::new()),
                },
                &mut Vec::new(),
            );
            harness.step();
        }
        all.sort_unstable();
        // The very first frame of a run pays for texture uploads and font
        // atlases; the budget is about the frames while a decode runs.
        samples.push(all[all.len() - 1]);
    }
    tests::bench::record("previewers-frame", &samples);
}
