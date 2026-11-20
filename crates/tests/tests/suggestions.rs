//! Acceptance (issue 252, specs/plugin-distribution.md §8): Rick says so
//! when a plugin that is not there would serve what the user is looking
//! at, against a registry of the test's own — a server on this machine
//! and a key made here. No test asks get.openbreach.dev, and a suggestion
//! makes no request at all: the test registry counts what it is asked.
//!
//! **The preview pane.** A file whose extension a plugin of the registry
//! claims and that only the hex view shows has the bar at the top of the
//! pane: the plugin's name and description, **Install**, **Not now** and
//! **Don't suggest again**. Without the hex view the *No previewer* card
//! reads *plugin available* and has **Install …** after *Open with…*. A
//! plugin that is on the disk and disabled gets **Enable** in both
//! places, with no registry at all. *Suggest plugins* off shows neither.
//!
//! **The file list.** A pane that enters a folder of the local file
//! system whose loaded listing holds a plugin's marker — `.git`, hidden
//! or not — has the bar between its toolbar and its list, and no other
//! pane has it. Leaving the folder takes it away; **Not now** silences
//! the plugin until Rick is started again, and **Don't suggest again**
//! writes its id to `[plugins] no_suggest`.

use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use ext::Extension;
use model::{SelectionChange, SettingsChange};
use platform::{ConfigDir, Fake};
use plugins_registry::fixtures::{Server, Signer};
use tests::local;
use tests::registry::{self, RICK};
use ui_egui::keys::Intent;
use ui_egui::{EguiHost, RickUi};

/// Where the test registry serves the 3D previewer's package.
const PACKAGE: &str = "plugins/rick/0.6.0/preview-3d.rkp";
/// The size the index names for a package that is listed and not served.
const LISTED: u64 = 2_411_724;

/// The previewers compiled into the binary.
fn previewers() -> Vec<Arc<dyn Extension>> {
    vec![
        Arc::new(ext_preview_text::TextPreview::new()),
        Arc::new(ext_preview_code::CodePreview::new()),
        Arc::new(ext_preview_image::ImagePreview::new()),
        Arc::new(ext_preview_pdf::PdfPreview::new()),
    ]
}

/// [`previewers`] and the hex view, the previewer of last resort.
fn with_hex() -> Vec<Arc<dyn Extension>> {
    let mut extensions = previewers();
    extensions.push(Arc::new(ext_preview_hex::HexPreview::new()));
    extensions
}

/// The index entry of the 3D previewer for [`RICK`], from its own
/// manifest: listed, and served by nobody.
fn models() -> serde_json::Value {
    registry::plugin(
        ext_preview_3d::Preview3d::new().manifest(),
        &[registry::listed(RICK, LISTED)],
    )
}

/// The name the 3D previewer's manifest gives it.
fn models_name() -> &'static str {
    ext_preview_3d::Preview3d::new().manifest().name
}

/// A package for the id `preview-3d`: a zip archive of the plugin's
/// folder with a manifest and a file in the library's place. It is
/// unpacked and never loaded.
fn models_package() -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    let manifest = "id = \"preview-3d\"\nname = \"3D model preview\"\nversion = \"0.6.0\"\n\
                    points = [\"previewer\"]\npreviews = [\"stl\", \"obj\", \"3mf\"]\n";
    for (name, content) in [
        ("preview-3d/plugin.toml", manifest),
        ("preview-3d/libext_preview_3d.so", "library"),
    ] {
        zip.start_file(name, options)
            .unwrap_or_else(|e| panic!("{e}"));
        zip.write_all(content.as_bytes())
            .unwrap_or_else(|e| panic!("{e}"));
    }
    zip.finish().unwrap_or_else(|e| panic!("{e}")).into_inner()
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    hex::encode(sha2::Sha256::digest(bytes))
}

/// A row of the plugin host: the plugin `id`, on the disk and disabled,
/// whose previewers claim `previews`.
fn disabled(id: &str, name: &str, description: &str, previews: &[&str]) -> app::Plugin {
    app::Plugin {
        id: id.to_owned(),
        name: name.to_owned(),
        description: description.to_owned(),
        version: "0.6.0".to_owned(),
        points: ext::Points::PREVIEWER,
        previews: previews.iter().map(|p| (*p).to_owned()).collect(),
        markers: Vec::new(),
        connections: Vec::new(),
        status: app::PluginStatus::Disabled,
        path: Path::new("/plugins").join(id),
        built_in: true,
        removed: false,
    }
}

/// What a test's window is started with.
struct Start<'a> {
    /// The config directory's platform.
    fake: &'a Arc<Fake>,
    /// `rick.toml`.
    settings: String,
    /// The folder the first tab shows.
    folder: &'a Path,
    /// The plugins the host found.
    rows: Vec<app::Plugin>,
    /// The extensions loaded.
    extensions: Vec<Arc<dyn Extension>>,
    /// The width of the pane the Preview tab is in; none, and the tab
    /// stays closed.
    preview: f32,
}

/// A window of Rick [`RICK`] a minute after its start, when the
/// background check of the plugin list has been, with the Preview tab
/// open.
fn window(start: Start<'_>) -> Harness<'static, RickUi> {
    let config = start.fake.config_dir();
    fs::create_dir_all(&config).unwrap_or_else(|e| panic!("{e}"));
    fs::write(config.join(app::SETTINGS_FILE), &start.settings).unwrap_or_else(|e| panic!("{e}"));
    let host = Arc::new(EguiHost::new());
    let mut app = app::App::new(app::Config {
        platform: start.fake.clone(),
        ui: host.clone(),
        extensions: start.extensions,
        config_dir: Some(config),
        start: Some(local(start.folder)),
    });
    app.set_rick_version(RICK);
    app.set_plugins(start.rows);
    let width = start.preview;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1300.0, 700.0))
        .with_pixels_per_point(1.0)
        .build_eframe(move |cc| {
            let mut ui = RickUi::new(app, host, &cc.egui_ctx);
            ui.set_inspector_width(width);
            ui
        });
    settle(&mut harness);
    registry::background_check(harness.state_mut().app_mut());
    settle(&mut harness);
    let wanted = start.preview > 0.0;
    if harness.state().inspector_open() != wanted {
        let pane = harness.state().app().model().focused;
        harness.state_mut().queue(Intent::ToggleInspector, pane);
        harness.step();
    }
    steps(&mut harness);
    assert_eq!(harness.state().inspector_open(), wanted);
    harness
}

fn fake() -> Arc<Fake> {
    Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")))
}

fn settle(harness: &mut Harness<'_, RickUi>) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        steps(harness);
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
    steps(harness);
}

fn steps(harness: &mut Harness<'_, RickUi>) {
    for _ in 0..4 {
        harness.step();
    }
}

/// Selects `name` in the focused pane and steps until the Preview tab
/// shows it: a document, the centred card, or an error.
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
        if slot.is_some_and(|s| {
            s.entry() == Some(id)
                && (s.has_frame() || s.placeholder().is_some() || s.error().is_some())
        }) {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "{name}: nothing shown: {slot:?}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    steps(harness);
}

/// Clicks the one control called `label` and waits for what comes of it.
fn click(harness: &mut Harness<'_, RickUi>, label: &str) {
    harness.get_by_label(label).click();
    steps(harness);
    settle(harness);
}

/// The suggestion bar of the Preview tab, where it stands.
fn bar(harness: &Harness<'_, RickUi>) -> Option<egui::Rect> {
    harness.state().preview_rects().suggestion
}

/// Whether a control called `label` stands inside `rect`.
fn inside(harness: &Harness<'_, RickUi>, label: &str, rect: egui::Rect) -> bool {
    harness
        .query_all_by_label(label)
        .any(|node| rect.contains_rect(node.rect()))
}

/// A folder with two models no previewer but the hex view takes, and a
/// file nobody's plugin claims.
fn models_folder() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let noise: Vec<u8> = (0..2048u32).map(|i| (i * 31 % 251) as u8).collect();
    for name in ["gear.stl", "bracket.STL", "blob.bin"] {
        fs::write(dir.path().join(name), &noise).unwrap_or_else(|e| panic!("{e}"));
    }
    fs::write(dir.path().join("notes.txt"), "hello\n").unwrap_or_else(|e| panic!("{e}"));
    dir
}

/// Acceptance: selecting an `.stl` file with the 3D plugin not installed
/// shows the bar above the hex view; **Not now** takes it away until Rick
/// is started again.
#[test]
fn a_file_a_plugin_of_the_registry_claims_has_the_bar_above_the_hex_view() {
    let (server, signer, fake) = (Server::start(), Signer::new(1), fake());
    server.publish(&signer, &registry::index(&[models()]));
    let dir = models_folder();
    let mut harness = window(Start {
        fake: &fake,
        settings: registry::rick_toml(&server, &signer, true),
        folder: dir.path(),
        rows: Vec::new(),
        extensions: with_hex(),
        preview: 720.0,
    });
    let pane = harness.state().app().model().focused;
    let asked = server.requests().len();

    // A file of its own previewer, and one that only the hex view takes
    // and no plugin claims: no bar.
    for name in ["notes.txt", "blob.bin"] {
        select(&mut harness, name);
        assert_eq!(bar(&harness), None, "{name}");
        assert_eq!(harness.state().app().preview_suggestion(pane), None);
    }

    select(&mut harness, "gear.stl");
    let slot = harness.state().previews().get(pane);
    assert_eq!(
        slot.and_then(ui_egui::preview::Slot::previewer),
        Some(ext_preview_hex::ID),
        "the hex view shows the file"
    );
    let rects = harness.state().preview_rects();
    let (toolbar, content) = (
        rects.toolbar.unwrap_or_else(|| panic!("toolbar")),
        rects.content.unwrap_or_else(|| panic!("content")),
    );
    let bar_rect = bar(&harness).unwrap_or_else(|| panic!("no bar over the hex view"));
    // At the top of the pane: under the toolbar, over the content, as
    // wide as the pane and one line high.
    assert!(
        bar_rect.top() >= toolbar.bottom() && bar_rect.bottom() <= content.top() + 0.5,
        "{toolbar:?}, {bar_rect:?}, {content:?}"
    );
    assert!((bar_rect.width() - content.width()).abs() < 1.0);
    assert!(
        (bar_rect.height() - ui_egui::footer_height(model::Density::Comfortable)).abs() < 0.5,
        "one line: {bar_rect:?}"
    );
    // The text is built from the plugin's name and its description; the
    // 3D previewer's manifest has no description, so it is the name.
    let name = models_name();
    assert!(inside(&harness, &format!("{name} plugin"), bar_rect));
    for button in ["Install", "Not now", "Don't suggest again"] {
        assert!(inside(&harness, button, bar_rect), "{button}");
    }
    // The buttons follow the text, in this order.
    let left = |label: &str| harness.get_by_label(label).rect().left();
    assert!(left("Install") < left("Not now") && left("Not now") < left("Don't suggest again"));
    assert!(
        harness
            .get_by_label(&format!("{name} plugin"))
            .rect()
            .right()
            < left("Install")
    );

    // The other model has it too; a file of another kind takes it away.
    select(&mut harness, "bracket.STL");
    assert!(bar(&harness).is_some(), "whatever the case of the name");
    select(&mut harness, "notes.txt");
    assert_eq!(bar(&harness), None, "leaving the file takes it away");
    select(&mut harness, "gear.stl");
    assert!(bar(&harness).is_some());

    // **Not now**: gone, for this file and for the next model, and
    // nothing is written.
    click(&mut harness, "Not now");
    assert_eq!(bar(&harness), None);
    select(&mut harness, "bracket.STL");
    assert_eq!(bar(&harness), None, "silenced until Rick is started again");
    assert_eq!(
        harness.state().app().model().settings.plugins.no_suggest,
        [] as [&str; 0]
    );
    assert_eq!(server.requests().len(), asked, "no request was made");
}

/// **Not now** holds until Rick is started again; **Don't suggest again**
/// writes the plugin's id to `[plugins] no_suggest`, and the bar does not
/// come back after a restart.
#[test]
fn not_now_ends_with_the_run_and_dont_suggest_again_holds_after_a_restart() {
    let (server, signer, fake) = (Server::start(), Signer::new(1), fake());
    server.publish(&signer, &registry::index(&[models()]));
    let dir = models_folder();
    let mut harness = window(Start {
        fake: &fake,
        settings: registry::rick_toml(&server, &signer, true),
        folder: dir.path(),
        rows: Vec::new(),
        extensions: with_hex(),
        preview: 720.0,
    });
    select(&mut harness, "gear.stl");
    click(&mut harness, "Not now");
    assert_eq!(bar(&harness), None);
    drop(harness);

    // Started again: suggested again. **Don't suggest again** writes the
    // id, and the start after that does not suggest it.
    let settings = fs::read_to_string(fake.config_dir().join(app::SETTINGS_FILE))
        .unwrap_or_else(|e| panic!("{e}"));
    let mut harness = window(Start {
        fake: &fake,
        settings,
        folder: dir.path(),
        rows: Vec::new(),
        extensions: with_hex(),
        preview: 720.0,
    });
    select(&mut harness, "gear.stl");
    assert!(bar(&harness).is_some(), "a new start suggests again");
    click(&mut harness, "Don't suggest again");
    assert_eq!(bar(&harness), None);
    assert_eq!(
        harness.state().app().model().settings.plugins.no_suggest,
        ["preview-3d"]
    );
    let path = fake.config_dir().join(app::SETTINGS_FILE);
    let started = Instant::now();
    while !fs::read_to_string(&path).is_ok_and(|t| t.contains("no_suggest = [\"preview-3d\"]")) {
        assert!(started.elapsed() < Duration::from_secs(10), "not written");
        std::thread::sleep(Duration::from_millis(20));
    }
    drop(harness);
    let settings = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{e}"));
    let mut harness = window(Start {
        fake: &fake,
        settings,
        folder: dir.path(),
        rows: Vec::new(),
        extensions: with_hex(),
        preview: 720.0,
    });
    select(&mut harness, "gear.stl");
    assert_eq!(bar(&harness), None, "not after a restart either");
}

/// Acceptance: with the hex plugin disabled too, the *No previewer* card
/// has **Install** with the plugin's name after *Open with…*, and
/// **Install** downloads the package the list names from the test
/// registry and unpacks it.
#[test]
fn without_the_hex_view_the_card_has_install_after_open_with() {
    let (server, signer, fake) = (Server::start(), Signer::new(1), fake());
    let package = models_package();
    server.put(PACKAGE, package.clone());
    let entry = registry::plugin(
        ext_preview_3d::Preview3d::new().manifest(),
        &[registry::build(
            RICK,
            RICK,
            &server.url(PACKAGE),
            package.len() as u64,
            &sha256(&package),
        )],
    );
    server.publish(&signer, &registry::index(&[entry]));
    let dir = models_folder();
    let mut harness = window(Start {
        fake: &fake,
        settings: registry::rick_toml(&server, &signer, true),
        folder: dir.path(),
        rows: Vec::new(),
        extensions: previewers(),
        preview: 520.0,
    });
    let pane = harness.state().app().model().focused;

    // A file no plugin claims: the card as it was, with its one button.
    select(&mut harness, "blob.bin");
    assert!(
        harness
            .query_all_by_label_contains("no previewer for .bin")
            .next()
            .is_some()
    );
    assert!(harness.query_by_label_contains("Install ").is_none());

    select(&mut harness, "gear.stl");
    let slot = harness.state().previews().get(pane);
    assert!(slot.is_some_and(|s| s.placeholder().is_some()), "the card");
    assert_eq!(bar(&harness), None, "the card has the button, not a bar");
    assert!(
        harness
            .query_all_by_label_contains("plugin available")
            .next()
            .is_some(),
        "the head keeps *plugin available*"
    );
    let install = format!("Install {}", models_name());
    let with = harness.get_by_label("Open with…").rect();
    let button = harness.get_by_label(&install).rect();
    assert!(
        button.left() > with.right() && (button.center().y - with.center().y).abs() < 1.0,
        "{button:?} after {with:?}"
    );

    // An empty file is given to no previewer, and so to no plugin's: its
    // card has *Move to trash* alone.
    fs::write(dir.path().join("empty.stl"), b"").unwrap_or_else(|e| panic!("{e}"));
    harness
        .state_mut()
        .app_mut()
        .handle(app::Command::Refresh(pane), &mut Vec::new());
    settle(&mut harness);
    select(&mut harness, "empty.stl");
    assert!(
        harness
            .query_all_by_label_contains("This file is empty")
            .next()
            .is_some()
    );
    assert!(harness.query_by_label(&install).is_none());
    assert_eq!(harness.state().app().preview_suggestion(pane), None);
    select(&mut harness, "gear.stl");

    // **Install**: the package of the list, from the test registry, into
    // the per-user plugins folder; it is loaded at the next start, and
    // nothing suggests it any more.
    let asked = server.requests().len();
    click(&mut harness, &install);
    let started = Instant::now();
    while !harness
        .state()
        .app()
        .plugins()
        .iter()
        .any(|row| row.id == "preview-3d" && row.status == app::PluginStatus::Installed)
    {
        settle(&mut harness);
        assert!(started.elapsed() < Duration::from_secs(20), "not installed");
    }
    steps(&mut harness);
    let folder = harness.state().app().plugins_dir().join("preview-3d");
    assert!(folder.join("plugin.toml").is_file(), "unpacked");
    let fetched: Vec<String> = server.requests()[asked..]
        .iter()
        .map(|request| request.path.clone())
        .collect();
    assert_eq!(fetched, [PACKAGE], "the package, once");
    assert_eq!(harness.state().app().preview_suggestion(pane), None);
    assert!(harness.query_by_label(&install).is_none());
}

/// Acceptance: a disabled plugin that claims the file gets **Enable**,
/// with no registry at all — on the bar where the hex view shows the
/// file.
#[test]
fn a_disabled_plugin_that_claims_the_file_gets_enable_with_no_registry() {
    let fake = fake();
    let dir = models_folder();
    let mut harness = window(Start {
        fake: &fake,
        settings: "[plugins]\ndisabled = [\"preview-3d\"]\n".to_owned(),
        folder: dir.path(),
        rows: vec![disabled(
            "preview-3d",
            "3D Models",
            "Shows STL, OBJ and 3MF models.",
            &["stl", "obj", "3mf", "txt"],
        )],
        extensions: with_hex(),
        preview: 720.0,
    });
    let pane = harness.state().app().model().focused;
    assert!(!harness.state().app().plugin_registry().set_up);

    // A loaded previewer of the file's own shows it: nothing is
    // suggested, whatever the plugin's manifest claims.
    select(&mut harness, "notes.txt");
    assert_eq!(bar(&harness), None);
    assert_eq!(harness.state().app().preview_suggestion(pane), None);

    select(&mut harness, "gear.stl");
    let bar_rect = bar(&harness).unwrap_or_else(|| panic!("no bar"));
    assert!(inside(
        &harness,
        "3D Models plugin — Shows STL, OBJ and 3MF models.",
        bar_rect
    ));
    for button in ["Enable", "Not now", "Don't suggest again"] {
        assert!(inside(&harness, button, bar_rect), "{button}");
    }
    assert!(harness.query_by_label("Install").is_none());
    assert_eq!(
        harness
            .state()
            .app()
            .preview_suggestion(pane)
            .map(|s| s.remedy),
        Some(app::Remedy::Enable)
    );

    // **Enable** takes the plugin off `[plugins] disabled`: it is loaded
    // at the next start.
    click(&mut harness, "Enable");
    assert_eq!(
        harness.state().app().model().settings.plugins.disabled,
        [] as [&str; 0]
    );
    assert_eq!(bar(&harness), None);
}

/// Acceptance: *Suggest plugins* off shows neither the bar nor the
/// button; on again, both are back.
#[test]
fn suggest_plugins_off_shows_neither_the_bar_nor_the_button() {
    let (server, signer) = (Server::start(), Signer::new(1));
    server.publish(&signer, &registry::index(&[models()]));
    let dir = models_folder();
    let off = format!(
        "[plugins]\nsuggest = false\n{}",
        registry::rick_toml(&server, &signer, true)
    );
    let switch = |harness: &mut Harness<'_, RickUi>, on: bool| {
        harness.state_mut().app_mut().handle(
            app::Command::Settings(SettingsChange::SuggestPlugins(on)),
            &mut Vec::new(),
        );
        steps(harness);
    };

    // Over the hex view.
    let mut harness = window(Start {
        fake: &fake(),
        settings: off.clone(),
        folder: dir.path(),
        rows: Vec::new(),
        extensions: with_hex(),
        preview: 720.0,
    });
    select(&mut harness, "gear.stl");
    assert_eq!(bar(&harness), None);
    switch(&mut harness, true);
    assert!(bar(&harness).is_some());
    switch(&mut harness, false);
    assert_eq!(bar(&harness), None);
    drop(harness);

    // On the card.
    let mut harness = window(Start {
        fake: &fake(),
        settings: off,
        folder: dir.path(),
        rows: Vec::new(),
        extensions: previewers(),
        preview: 520.0,
    });
    select(&mut harness, "gear.stl");
    let install = format!("Install {}", models_name());
    assert!(harness.query_by_label(&install).is_none());
    assert!(
        harness
            .query_all_by_label_contains("no previewer for .stl")
            .next()
            .is_some(),
        "and the head does not say *plugin available*"
    );
    switch(&mut harness, true);
    harness.get_by_label(&install);
    assert!(
        harness
            .query_all_by_label_contains("plugin available")
            .next()
            .is_some()
    );
}

/// The bar is one line where the pane has the room; in a pane too narrow
/// for the plugin's name beside the three buttons, the buttons stand
/// under the text, on as many lines as they need, and nothing is cut to
/// nothing.
#[test]
fn in_a_narrow_pane_the_bars_buttons_go_under_its_text() {
    let fake = fake();
    let dir = models_folder();
    let mut harness = window(Start {
        fake: &fake,
        settings: "[plugins]\ndisabled = [\"preview-3d\"]\n".to_owned(),
        folder: dir.path(),
        rows: vec![disabled(
            "preview-3d",
            "3D Models",
            "Shows STL, OBJ and 3MF models.",
            &["stl"],
        )],
        extensions: with_hex(),
        preview: 300.0,
    });
    select(&mut harness, "gear.stl");
    let bar_rect = bar(&harness).unwrap_or_else(|| panic!("no bar"));
    let one_line = ui_egui::footer_height(model::Density::Comfortable);
    assert!(
        bar_rect.height() > one_line + 20.0,
        "more than one line: {bar_rect:?}"
    );
    let text = harness
        .get_by_label("3D Models plugin — Shows STL, OBJ and 3MF models.")
        .rect();
    for button in ["Enable", "Not now", "Don't suggest again"] {
        let rect = harness.get_by_label(button).rect();
        assert!(
            bar_rect.contains_rect(rect) && rect.top() >= text.bottom(),
            "{button}: {rect:?} under {text:?} in {bar_rect:?}"
        );
    }
    // The hex view keeps the rest of the pane.
    let content = harness
        .state()
        .preview_rects()
        .content
        .unwrap_or_else(|| panic!("content"));
    assert!(content.top() >= bar_rect.bottom() - 0.5 && content.height() > 200.0);
}

/// Acceptance: selecting ten thousand files one after another with a
/// cached index makes no request; the test registry counts them.
#[test]
fn ten_thousand_selections_with_a_cached_list_make_no_request() {
    let (server, signer, fake) = (Server::start(), Signer::new(1), fake());
    server.publish(&signer, &registry::index(&[models()]));
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let noise: Vec<u8> = (0..256u32).map(|i| (i * 31 % 251) as u8).collect();
    for n in 0..100 {
        let name = if n % 2 == 0 {
            format!("part-{n:03}.stl")
        } else {
            format!("blob-{n:03}.bin")
        };
        fs::write(dir.path().join(name), &noise).unwrap_or_else(|e| panic!("{e}"));
    }
    // A first run fetches the list and caches it.
    let harness = window(Start {
        fake: &fake,
        settings: registry::rick_toml(&server, &signer, true),
        folder: dir.path(),
        rows: Vec::new(),
        extensions: with_hex(),
        preview: 520.0,
    });
    drop(harness);
    let asked = server.requests().len();
    assert_eq!(asked, 2, "the index and its signature, once");

    // The next run has the cached list and the day ahead of it.
    let settings = fs::read_to_string(fake.config_dir().join(app::SETTINGS_FILE))
        .unwrap_or_else(|e| panic!("{e}"));
    let mut harness = window(Start {
        fake: &fake,
        settings,
        folder: dir.path(),
        rows: Vec::new(),
        extensions: with_hex(),
        preview: 520.0,
    });
    assert!(harness.state().app().plugin_registry().index.is_some());
    let pane = harness.state().app().model().focused;
    let ids: Vec<(String, types::EntryId)> = harness
        .state()
        .app()
        .model()
        .active_tab(pane)
        .map(|(_, tab)| tab.listing.rows().map(|e| (e.name.clone(), e.id)).collect())
        .unwrap_or_default();
    assert_eq!(ids.len(), 100);
    let mut suggested = 0;
    for n in 0..10_000 {
        let (name, id) = &ids[n % ids.len()];
        harness.state_mut().app_mut().handle(
            app::Command::Select {
                pane,
                change: SelectionChange::Set(vec![*id]),
            },
            &mut Vec::new(),
        );
        // Now and then a selection is waited for and looked at; the
        // others are overtaken by the next, as a held arrow key does.
        if n % 97 == 0 {
            select(&mut harness, name);
            let stl = name.starts_with("part-");
            assert_eq!(bar(&harness).is_some(), stl, "{name}");
            suggested += usize::from(stl);
        } else {
            let _ = harness.state_mut().app_mut().poll(&mut Vec::new());
        }
    }
    settle(&mut harness);
    assert!(suggested > 0, "the bar was shown on the way");
    assert_eq!(server.requests().len(), asked, "nobody was asked");
}

/// The index entry of Git for [`RICK`], from its own manifest: listed,
/// and served by nobody.
fn git() -> serde_json::Value {
    registry::plugin(
        ext_git::GitExtension::new().manifest(),
        &[registry::listed(RICK, 4_812_800)],
    )
}

/// What the bar says of Git: its manifest's name and description.
fn git_text() -> String {
    let git = ext_git::GitExtension::new();
    let manifest = git.manifest();
    format!("{} plugin — {}", manifest.name, manifest.description)
}

/// A folder with two work trees and a plain folder in it.
fn projects() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    for (name, entries) in [
        ("rick", &[".git", "src", "target"][..]),
        ("trebuchet", &[".git", "docs"][..]),
        ("notes", &[".github", "drafts"][..]),
    ] {
        for entry in entries {
            fs::create_dir_all(dir.path().join(name).join(entry)).unwrap_or_else(|e| panic!("{e}"));
        }
        fs::write(dir.path().join(name).join("README.md"), "# Readme\n")
            .unwrap_or_else(|e| panic!("{e}"));
        fs::write(dir.path().join(name).join(".gitignore"), "target\n")
            .unwrap_or_else(|e| panic!("{e}"));
    }
    dir
}

/// Shows `folder` in `pane` and waits for its listing.
fn enter(harness: &mut Harness<'_, RickUi>, pane: model::PaneId, folder: &Path) {
    harness.state_mut().app_mut().handle(
        app::Command::Navigate {
            pane,
            to: local(folder),
        },
        &mut Vec::new(),
    );
    settle(harness);
}

/// The active tab of `pane`.
fn tab_of(harness: &Harness<'_, RickUi>, pane: model::PaneId) -> model::TabId {
    harness
        .state()
        .app()
        .model()
        .active_tab(pane)
        .unwrap_or_else(|| panic!("no tab in {pane:?}"))
        .0
}

/// The suggestion bar of the file list of `pane`, where it stands.
fn list_bar(harness: &Harness<'_, RickUi>, pane: model::PaneId) -> Option<egui::Rect> {
    harness.state().suggestion_bar(tab_of(harness, pane))
}

/// Acceptance: with a test registry that offers a Git plugin and none
/// installed, entering a folder with `.git` shows the bar in that pane
/// and in no other, between the toolbar and the list; leaving the folder
/// takes it away.
#[test]
fn entering_a_folder_with_a_marker_shows_the_bar_in_that_pane_and_in_no_other() {
    let (server, signer, fake) = (Server::start(), Signer::new(1), fake());
    server.publish(&signer, &registry::index(&[git(), models()]));
    let dir = projects();
    let mut harness = window(Start {
        fake: &fake,
        settings: registry::rick_toml(&server, &signer, true),
        folder: dir.path(),
        rows: Vec::new(),
        extensions: with_hex(),
        preview: 0.0,
    });
    let asked = server.requests().len();
    let left = harness.state().app().model().focused;
    harness.state_mut().app_mut().handle(
        app::Command::Split {
            pane: left,
            dir: model::SplitDir::Right,
        },
        &mut Vec::new(),
    );
    settle(&mut harness);
    let right = harness.state().app().model().focused;
    assert_ne!(left, right);
    assert_eq!(list_bar(&harness, left), None);
    assert_eq!(list_bar(&harness, right), None, "no marker in the parent");

    // The pane does not show hidden entries, and `.git` is one: the
    // listing it has loaded holds it all the same.
    enter(&mut harness, right, &dir.path().join("rick"));
    let tab = tab_of(&harness, right);
    let shown = harness
        .state()
        .app()
        .model()
        .tab(tab)
        .map(|t| t.listing.rows().map(|e| e.name.clone()).collect::<Vec<_>>());
    assert!(shown.is_some_and(|names| !names.contains(&".git".to_owned())));
    let bar_rect = list_bar(&harness, right).unwrap_or_else(|| panic!("no bar"));
    assert_eq!(list_bar(&harness, left), None, "in no other pane");

    // Between the toolbar and the list of that pane, as wide as the pane
    // and one line high.
    let toolbar = harness
        .state()
        .toolbar_row(tab)
        .unwrap_or_else(|| panic!("toolbar"));
    let header = harness
        .state()
        .header_band(tab)
        .unwrap_or_else(|| panic!("header band"));
    assert!(
        bar_rect.top() >= toolbar.bottom() - 0.5 && bar_rect.bottom() <= header.top() + 0.5,
        "{toolbar:?}, {bar_rect:?}, {header:?}"
    );
    assert!((bar_rect.width() - toolbar.width()).abs() < 1.0);
    assert!(
        (bar_rect.height() - ui_egui::footer_height(model::Density::Comfortable)).abs() < 0.5,
        "one line: {bar_rect:?}"
    );
    assert!(inside(&harness, &git_text(), bar_rect), "{}", git_text());
    for button in ["Install", "Not now", "Don't suggest again"] {
        assert!(inside(&harness, button, bar_rect), "{button}");
    }

    // It takes no focus: the keyboard stays where it was.
    assert!(
        ["Install", "Not now", "Don't suggest again"]
            .iter()
            .all(|label| !harness.get_by_label(label).is_focused())
    );

    // Leaving the folder takes it away; a folder whose names only look
    // like the marker has none; the other work tree has it again.
    enter(&mut harness, right, &dir.path().join("notes"));
    assert_eq!(list_bar(&harness, right), None);
    enter(&mut harness, right, &dir.path().join("trebuchet"));
    assert!(list_bar(&harness, right).is_some());
    // And the other pane has its own when it enters one.
    enter(&mut harness, left, &dir.path().join("rick"));
    assert!(list_bar(&harness, left).is_some());
    assert!(list_bar(&harness, right).is_some());
    assert_eq!(server.requests().len(), asked, "no request was made");
}

/// Acceptance: **Not now** takes the bar away until Rick restarts;
/// **Don't suggest again** writes `git` to `[plugins] no_suggest` and the
/// bar does not come back after a restart.
#[test]
fn not_now_and_dont_suggest_again_on_the_bar_of_a_folder() {
    let (server, signer, fake) = (Server::start(), Signer::new(1), fake());
    server.publish(&signer, &registry::index(&[git()]));
    let dir = projects();
    let repo = dir.path().join("rick");
    let start = |settings: String| Start {
        fake: &fake,
        settings,
        folder: &repo,
        rows: Vec::new(),
        extensions: with_hex(),
        preview: 0.0,
    };
    let path = fake.config_dir().join(app::SETTINGS_FILE);
    let written = || fs::read_to_string(&path).unwrap_or_else(|e| panic!("{e}"));

    // The folder is listed before the plugin list arrives: the bar comes
    // with the list.
    let mut harness = window(start(registry::rick_toml(&server, &signer, true)));
    let pane = harness.state().app().model().focused;
    assert!(list_bar(&harness, pane).is_some());
    click(&mut harness, "Not now");
    assert_eq!(list_bar(&harness, pane), None);
    // Not in the other work tree either, and nothing is written.
    enter(&mut harness, pane, &dir.path().join("trebuchet"));
    assert_eq!(list_bar(&harness, pane), None);
    assert_eq!(
        harness.state().app().model().settings.plugins.no_suggest,
        [] as [&str; 0]
    );
    drop(harness);

    // Rick restarts: the bar is back.
    let mut harness = window(start(written()));
    let pane = harness.state().app().model().focused;
    assert!(list_bar(&harness, pane).is_some(), "a new start suggests");
    click(&mut harness, "Don't suggest again");
    assert_eq!(list_bar(&harness, pane), None);
    assert_eq!(
        harness.state().app().model().settings.plugins.no_suggest,
        ["git"]
    );
    let started = Instant::now();
    while !written().contains("no_suggest = [\"git\"]") {
        assert!(started.elapsed() < Duration::from_secs(10), "not written");
        std::thread::sleep(Duration::from_millis(20));
    }
    drop(harness);

    // And after this restart it does not come back.
    let mut harness = window(start(written()));
    let pane = harness.state().app().model().focused;
    assert_eq!(list_bar(&harness, pane), None);
    enter(&mut harness, pane, &dir.path().join("trebuchet"));
    assert_eq!(list_bar(&harness, pane), None);
}

/// A Git plugin that is on the disk and disabled gets **Enable** on the
/// folder's bar, with no registry at all; and *Suggest plugins* off shows
/// no bar.
#[test]
fn a_disabled_plugin_gets_enable_on_the_bar_of_a_folder() {
    let fake = fake();
    let dir = projects();
    let repo = dir.path().join("rick");
    let git = ext_git::GitExtension::new();
    let manifest = git.manifest();
    let mut row = disabled(manifest.id.0, manifest.name, manifest.description, &[]);
    row.markers = manifest.markers.iter().map(|m| (*m).to_owned()).collect();
    let mut harness = window(Start {
        fake: &fake,
        settings: "[plugins]\ndisabled = [\"git\"]\n".to_owned(),
        folder: &repo,
        rows: vec![row],
        extensions: with_hex(),
        preview: 0.0,
    });
    let pane = harness.state().app().model().focused;
    let bar_rect = list_bar(&harness, pane).unwrap_or_else(|| panic!("no bar"));
    for button in ["Enable", "Not now", "Don't suggest again"] {
        assert!(inside(&harness, button, bar_rect), "{button}");
    }
    assert!(harness.query_by_label("Install").is_none());

    // *Suggest plugins* off: no bar; on again, it is back.
    let switch = |harness: &mut Harness<'_, RickUi>, on: bool| {
        harness.state_mut().app_mut().handle(
            app::Command::Settings(SettingsChange::SuggestPlugins(on)),
            &mut Vec::new(),
        );
        steps(harness);
    };
    switch(&mut harness, false);
    assert_eq!(list_bar(&harness, pane), None);
    switch(&mut harness, true);
    assert!(list_bar(&harness, pane).is_some());

    // **Enable** takes Git off `[plugins] disabled`.
    click(&mut harness, "Enable");
    assert_eq!(
        harness.state().app().model().settings.plugins.disabled,
        [] as [&str; 0]
    );
    assert_eq!(list_bar(&harness, pane), None);
}

/// One bar per pane: a model selected in a work tree has the folder's
/// bar over the list and the file's bar over the hex view, each in its
/// own pane and each for its own plugin.
#[test]
fn the_folder_s_bar_and_the_file_s_bar_stand_each_in_its_own_pane() {
    let (server, signer, fake) = (Server::start(), Signer::new(1), fake());
    server.publish(&signer, &registry::index(&[git(), models()]));
    let dir = projects();
    let repo = dir.path().join("rick");
    let noise: Vec<u8> = (0..2048u32).map(|i| (i * 31 % 251) as u8).collect();
    fs::write(repo.join("case.stl"), noise).unwrap_or_else(|e| panic!("{e}"));
    let mut harness = window(Start {
        fake: &fake,
        settings: registry::rick_toml(&server, &signer, true),
        folder: &repo,
        rows: Vec::new(),
        extensions: with_hex(),
        preview: 560.0,
    });
    let pane = harness.state().app().model().focused;
    select(&mut harness, "case.stl");
    let of_list = list_bar(&harness, pane).unwrap_or_else(|| panic!("no bar over the list"));
    let of_file = bar(&harness).unwrap_or_else(|| panic!("no bar over the hex view"));
    assert!(!of_list.intersects(of_file), "{of_list:?}, {of_file:?}");
    assert!(inside(&harness, &git_text(), of_list));
    assert!(inside(
        &harness,
        &format!("{} plugin", models_name()),
        of_file
    ));
    assert_eq!(harness.query_all_by_label("Install").count(), 2);

    // **Not now** on the file's bar silences the 3D previewer and leaves
    // Git's bar where it is.
    let not_now = harness
        .query_all_by_label("Not now")
        .find(|node| of_file.contains_rect(node.rect()))
        .unwrap_or_else(|| panic!("the file's Not now"));
    not_now.click();
    steps(&mut harness);
    settle(&mut harness);
    assert_eq!(bar(&harness), None);
    assert!(list_bar(&harness, pane).is_some());
}

/// Acceptance: entering a folder of 200,000 entries is not measurably
/// slower with suggestions on. Two measurements say so. The folder is
/// entered in turn by a Rick that holds a plugin list with markers, and
/// so holds every name of the listing against them, and by one that knows
/// no marker at all: their medians lie within what two runs of one
/// differ by. And holding the 200,000 names of the loaded listing against
/// the markers, the work itself, is timed alone: it is under a hundredth
/// of entering the folder.
#[test]
#[ignore = "lists 200 000 entries; run with --ignored in release"]
#[allow(clippy::too_many_lines)]
fn entering_a_folder_of_200k_entries_is_not_slower_with_suggestions_on() {
    use tests::fixture::{Config, SizeDistribution};
    const ENTRIES: usize = 200_000;
    let fixture = Config {
        entries: ENTRIES,
        max_depth: 0,
        sizes: SizeDistribution::Empty,
        ..Config::new(ENTRIES, 11)
    }
    .generate()
    .unwrap_or_else(|e| panic!("{e}"));
    // The marker is in the folder: the listing that is held against the
    // markers finds one.
    fs::create_dir(fixture.root().join(".git")).unwrap_or_else(|e| panic!("{e}"));
    let elsewhere = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let (server, signer) = (Server::start(), Signer::new(1));
    server.publish(&signer, &registry::index(&[git(), models()]));

    // A Rick on `settings`, started in `elsewhere`, after its start read
    // or fetched what there is of a plugin list.
    let rick = |settings: String| {
        let fake = fake();
        let config = fake.config_dir();
        fs::create_dir_all(&config).unwrap_or_else(|e| panic!("{e}"));
        fs::write(config.join(app::SETTINGS_FILE), settings).unwrap_or_else(|e| panic!("{e}"));
        let mut app = app::App::new(app::Config {
            platform: fake,
            ui: Arc::new(EguiHost::new()),
            extensions: Vec::new(),
            config_dir: Some(config),
            start: Some(local(elsewhere.path())),
        });
        app.set_rick_version(RICK);
        let mut events = Vec::new();
        for command in [app::RegistryCommand::Start, app::RegistryCommand::Daily] {
            app.handle(app::Command::Registry(command), &mut events);
            wait(&mut app, |app| !app.is_busy());
        }
        app
    };
    // Enters the fixture and says how long its listing took to complete.
    let enter = |app: &mut app::App| {
        let pane = app.model().focused;
        let mut events = Vec::new();
        app.handle(
            app::Command::Navigate {
                pane,
                to: local(elsewhere.path()),
            },
            &mut events,
        );
        wait(app, |app| !app.is_busy());
        let started = Instant::now();
        app.handle(
            app::Command::Navigate {
                pane,
                to: local(fixture.root()),
            },
            &mut events,
        );
        wait(app, |app| {
            app.model()
                .active_tab(pane)
                .is_some_and(|(_, tab)| tab.listing.complete && tab.listing.len() > ENTRIES)
        });
        started.elapsed()
    };
    let median = |samples: &mut Vec<Duration>| {
        samples.sort();
        samples[samples.len() / 2]
    };

    let mut with = rick(registry::rick_toml(&server, &signer, true));
    let mut without = rick(String::new());
    assert!(with.plugin_registry().index.is_some());
    assert!(without.plugin_registry().index.is_none());
    // One entry each to warm the file system's cache and the runtime,
    // then the two in turn, each going first every other time.
    enter(&mut without);
    enter(&mut with);
    let (mut on, mut off) = (Vec::new(), Vec::new());
    for round in 0..6 {
        if round % 2 == 0 {
            off.push(enter(&mut without));
            on.push(enter(&mut with));
        } else {
            on.push(enter(&mut with));
            off.push(enter(&mut without));
        }
    }
    let pane = with.model().focused;
    let found = |app: &app::App| app.folder_suggestion(pane).map(|s| s.id);
    assert_eq!(
        found(&with).as_deref(),
        Some("git"),
        "and the marker was found among them"
    );
    assert_eq!(without.folder_suggestion(without.model().focused), None);

    // The work itself: the 200,000 names of the listing the pane has
    // loaded, held against the markers once more, as when the plugin
    // host's rows arrive after the folder was listed.
    let mut scans = Vec::new();
    for _ in 0..9 {
        let started = Instant::now();
        with.set_plugins(Vec::new());
        scans.push(started.elapsed());
        assert_eq!(found(&with).as_deref(), Some("git"));
    }

    println!("with a plugin list that names markers: {on:?}");
    println!("without: {off:?}");
    let (on, off, scan) = (median(&mut on), median(&mut off), median(&mut scans));
    println!(
        "entering 200 000 entries on {}: {on:?} with a plugin list that names markers, \
         {off:?} without; holding the names against the markers {scan:?}",
        std::env::consts::OS
    );
    if std::env::var_os("RICK_BENCH_RECORD").is_none() {
        // Entering the folder varies by a quarter from one run to the
        // next; the two medians are no further apart than that.
        assert!(
            on <= off + off / 4,
            "{on:?} with suggestions, {off:?} without"
        );
        // What suggestions add is under a hundredth of it.
        assert!(
            scan * 100 <= off,
            "{scan:?} to hold the names against the markers, {off:?} to enter"
        );
    }
}

/// Runs the application's loop until `done` holds, for a minute at most.
fn wait(app: &mut app::App, done: impl Fn(&app::App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut events = Vec::new();
    loop {
        let handled = app.poll(&mut events);
        events.clear();
        if handled == 0 {
            if done(app) {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(Instant::now() < deadline, "did not get there");
    }
}
