//! Acceptance (issue 84): the icon previewer as a plugin. It loads through
//! the host with the four extensions its manifest claims and takes an ICO,
//! a CUR, an ICNS and an XPM by sniff; and with the plugin disabled in
//! Extensions an `.ico` shows the *Plugin available* placeholder, from the
//! manifest alone.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use app::{App, Command, Config};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use ext::{Extension, Provider, Registry};
use model::SelectionChange;
use platform::{ConfigDir, Fake};
use plugins_native::{Host, Library, MANIFEST_FILE, Options, PluginStatus, PluginToml, Source};
use tests::local;
use types::{CancelToken, Entry};
use ui_egui::{EguiHost, Intent, RickUi};

/// The plugin's library stem and extension id.
const STEM: &str = "ext_preview_icons";
const ID: &str = "preview-icons";

/// The fixtures of the plugin's own tests.
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../ext-preview-icons/tests/fixtures")
        .join(name)
}

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

/// The manifest the bundler would write for the plugin.
fn manifest() -> PluginToml {
    let mut toml = plugins_native::describe(&library_of(STEM)).unwrap_or_else(|e| panic!("{e}"));
    toml.library = Library::for_crate(STEM);
    toml
}

/// Installs the plugin into `plugins_dir` with its manifest.
fn install(plugins_dir: &Path) -> PluginToml {
    let toml = manifest();
    let dir = plugins_dir.join(&toml.id);
    toml.write(&dir.join(MANIFEST_FILE))
        .unwrap_or_else(|e| panic!("{e}"));
    let file = toml
        .library
        .for_os(std::env::consts::OS)
        .unwrap_or_else(|| panic!("no library name for this OS"));
    fs::copy(library_of(STEM), dir.join(file)).unwrap_or_else(|e| panic!("{e}"));
    toml
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

/// The plugin loads through the host, its manifest claims the four
/// extensions, and its previewer takes every fixture by sniff and opens it
/// with the variants as pages.
#[test]
fn the_icons_plugin_loads_through_the_host_and_claims_the_fixtures_by_sniff() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let plugins = dir.path().join("plugins");
    let toml = install(&plugins);
    assert_eq!(toml.id, ID);
    assert_eq!(toml.previews, ["cur", "icns", "ico", "xpm"]);
    let host = Host::load(&Options {
        dirs: vec![(Source::BesideBinary, plugins)],
        disabled: Vec::new(),
        compiled_in: Vec::new(),
        platform: None,
        prompts: None,
        settings: None,
    });
    let row = host
        .plugins()
        .iter()
        .find(|p| p.id == ID)
        .unwrap_or_else(|| panic!("no plugin {ID}: {:?}", host.plugins()));
    assert_eq!(row.status, PluginStatus::Loaded, "{:?}", row.status);

    let mut registry = Registry::new();
    for extension in host.extensions() {
        registry.load(&*extension).unwrap_or_else(|e| panic!("{e}"));
    }
    let previewer = registry
        .previewers()
        .iter()
        .find(|p| p.id().0 == ID)
        .cloned()
        .unwrap_or_else(|| panic!("no previewer"));
    let provider = provider();
    for (name, pages) in [
        ("multi.ico", Some(3)),
        ("arrow.cur", None),
        ("mixed.icns", Some(2)),
        ("named.xpm", None),
    ] {
        let path = fixture(name);
        let (entry, bytes) = sniff(&provider, &path);
        assert_eq!(previewer.score(&entry, &bytes), 200, "{name}");
        let doc = runtime::block_on(previewer.open(
            &local(&path),
            Arc::clone(&provider),
            CancelToken::new(),
        ))
        .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(doc.pages(), pages, "{name}");
    }
    // The libraries stay mapped (QUESTIONS.md 85).
    let _ = dir.keep();
    Box::leak(Box::new(host));
}

/// A window on `start` with the Preview tab open and only the built-in
/// previewers, as a binary without the plugin would have.
fn window(fake: &Arc<Fake>, start: &Path) -> Harness<'static, RickUi> {
    let host = Arc::new(EguiHost::new());
    let extensions: Vec<Arc<dyn Extension>> = vec![
        Arc::new(ext_preview_text::TextPreview::new()),
        Arc::new(ext_preview_code::CodePreview::new()),
        Arc::new(ext_preview_image::ImagePreview::new()),
        Arc::new(ext_preview_pdf::PdfPreview::new()),
    ];
    let app = App::new(Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions,
        config_dir: Some(fake.config_dir()),
        start: Some(local(start)),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 700.0))
        .with_pixels_per_point(1.0)
        .build_eframe(move |cc| {
            let mut ui = RickUi::new(app, host, &cc.egui_ctx);
            ui.set_inspector_width(340.0);
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

/// Selects `name` and steps until the pane shows its placeholder.
fn select_unclaimed(harness: &mut Harness<'_, RickUi>, name: &str) {
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
    loop {
        harness.step();
        let slot = harness.state().previews().get(pane);
        if slot.is_some_and(|s| s.placeholder().is_some() || s.error().is_some()) {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "{name}: no placeholder: {slot:?}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    harness.step();
    harness.step();
}

/// With the plugin disabled in Extensions, an `.ico` shows the *Plugin
/// available* placeholder: the host answers from the plugin's manifest,
/// without loading it.
#[test]
fn with_the_plugin_disabled_an_ico_shows_the_plugin_available_placeholder() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    fs::copy(fixture("multi.ico"), dir.path().join("app.ico")).unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("blob.xyz"), [0u8, 1, 2, 3]).unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut harness = window(&fake, dir.path());
    // The plugin's real manifest, as the binary reports a disabled plugin
    // found in the plugins directory.
    let toml = manifest();
    harness.state_mut().app_mut().set_plugins(vec![app::Plugin {
        id: toml.id.clone(),
        name: toml.name.clone(),
        version: toml.version.clone(),
        points: ext::Points::PREVIEWER,
        previews: toml.previews.clone(),
        status: app::PluginStatus::Disabled,
        path: dir.path().join("plugins").join(&toml.id),
    }]);
    let pane = harness.state().app().model().focused;

    select_unclaimed(&mut harness, "app.ico");
    let slot = harness
        .state()
        .previews()
        .get(pane)
        .unwrap_or_else(|| panic!("slot"));
    assert_eq!(
        slot.placeholder().map(|p| p.plugin.clone()),
        Some(Some(ID.to_owned())),
        "the disabled plugin claims .ico"
    );
    assert!(!slot.has_frame(), "no frame at all");
    harness.get_by_label("Plugin available");
    assert!(harness.query_by_label("No plugin available").is_none());

    select_unclaimed(&mut harness, "blob.xyz");
    harness.get_by_label("No plugin available");
}
