//! Acceptance (issue 251, items 2 to 4, specs/plugin-distribution.md §6
//! and §7): the *Available* view of the Extensions dialogue, the updates,
//! *Install from URL…* and the packaged Rick, against a registry of the
//! test's own — a server on this machine and a key made here. No test
//! asks get.openbreach.dev.
//!
//! A plugin the registry has for this Rick and that is not installed
//! shows under *Available*, with its size; **Install** puts it into the
//! per-user plugins folder, its row moves to *Installed* with the
//! *restart* chip, and after **Restart now** it is loaded and the session
//! is back. A package that is not the one the list describes is deleted
//! and nothing lands in the plugins folder. Where there is no list the
//! view says why — not set up in this build, a pre-release, too old a
//! Rick, offline, switched off — and an index that does not verify
//! leaves the last good one in place and says so.
//!
//! A plugin of the per-user folder that was built for another Rick, that
//! the record marks as the registry's and that the list has this Rick's
//! build of is an update: its card reads the host's reason and offers
//! **Update**, the header counts it, the footer has **Update all**, and
//! the first start that finds it says so in a toast. After the update
//! and a restart the plugin loads.
//!
//! *Install from URL…* takes an `https://` address, downloads the package
//! and handles it as a file. A Rick with the file `package` beside its
//! binary shows the command of its package manager in place of
//! **Install**.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use ext::{Extension as _, Level};
use platform::{ConfigDir, Fake};
use plugins_native::{Host, Library, MANIFEST_FILE, Options, PluginStatus, Source};
use plugins_registry::fixtures::{Server, Signer};
use tests::local;
use tests::registry::{self, RICK};
use ui_egui::keys::Intent;
use ui_egui::{EguiHost, RickUi};

/// Where the test registry serves the probe plugin's package.
const PACKAGE: &str = "plugins/rick/0.6.0/probe.rkp";

/// The package of the probe plugin, a real plugin built with this Rick:
/// a zip archive of its folder as the bundler lays it out, the manifest
/// and the library.
fn probe_package() -> Vec<u8> {
    let exe = std::env::current_exe().unwrap_or_else(|e| panic!("{e}"));
    let deps = exe.parent().unwrap_or_else(|| panic!("no parent"));
    let library = deps.join(format!(
        "{}plugin_probe{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    assert!(library.is_file(), "not built: {}", library.display());
    let mut toml = plugins_native::describe(&library).unwrap_or_else(|e| panic!("{e}"));
    toml.library = Library::for_crate("plugin_probe");
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let manifest = dir.path().join(MANIFEST_FILE);
    toml.write(&manifest).unwrap_or_else(|e| panic!("{e}"));
    let file = toml
        .library
        .for_os(std::env::consts::OS)
        .unwrap_or_else(|| panic!("no library name for this OS"));
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    for (name, bytes) in [
        (MANIFEST_FILE.to_owned(), fs::read(&manifest)),
        (file.to_owned(), fs::read(&library)),
    ] {
        zip.start_file(format!("{}/{name}", toml.id), options)
            .unwrap_or_else(|e| panic!("{e}"));
        zip.write_all(&bytes.unwrap_or_else(|e| panic!("{e}")))
            .unwrap_or_else(|e| panic!("{e}"));
    }
    zip.finish().unwrap_or_else(|e| panic!("{e}")).into_inner()
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    hex::encode(sha2::Sha256::digest(bytes))
}

/// The index entry of the probe plugin for [`RICK`], described as the
/// package `described` and served at [`PACKAGE`].
fn probe(server: &Server, described: &[u8]) -> serde_json::Value {
    registry::plugin(
        plugin_probe::ProbeExtension::new().manifest(),
        &[registry::build(
            env!("CARGO_PKG_VERSION"),
            RICK,
            &server.url(PACKAGE),
            described.len() as u64,
            &sha256(described),
        )],
    )
}

/// The index entry of Git for [`RICK`], listed and not served.
fn git() -> serde_json::Value {
    registry::plugin(
        ext_git::GitExtension::new().manifest(),
        &[registry::listed(RICK, 4_812_800)],
    )
}

/// What a test's window is started with.
struct Start<'a> {
    /// The config directory's platform.
    fake: &'a Arc<Fake>,
    /// `rick.toml`.
    settings: String,
    /// The version Rick asks the registry as.
    rick: &'a str,
    /// The folder the first tab shows.
    folder: &'a Path,
    /// The plugins the host found.
    rows: Vec<app::Plugin>,
    /// The package manager that installed this Rick, when one did.
    packaging: Option<app::plugins::Packaging>,
}

/// The binaries *Restart now* asked to continue in.
type Restarts = Arc<Mutex<Vec<PathBuf>>>;

/// A window of Rick with the Extensions dialogue open, and the list of
/// the binaries a restart was asked for.
fn window(start: &Start<'_>) -> (Harness<'static, RickUi>, Restarts) {
    let (mut harness, restarts) = shell(start);
    let pane = harness.state().app().model().focused;
    harness.state_mut().queue(Intent::Extensions, pane);
    steps(&mut harness);
    assert!(harness.state().extensions_open());
    (harness, restarts)
}

/// A window of Rick after its start, with no dialogue open.
fn shell(start: &Start<'_>) -> (Harness<'static, RickUi>, Restarts) {
    let config = start.fake.config_dir();
    fs::create_dir_all(&config).unwrap_or_else(|e| panic!("{e}"));
    fs::write(config.join(app::SETTINGS_FILE), &start.settings).unwrap_or_else(|e| panic!("{e}"));
    let host = Arc::new(EguiHost::new());
    let mut app = app::App::new(app::Config {
        platform: start.fake.clone(),
        ui: host.clone(),
        extensions: Vec::new(),
        config_dir: Some(config),
        start: Some(local(start.folder)),
    });
    app.set_rick_version(start.rick);
    app.set_plugins(start.rows.clone());
    app.set_packaging(start.packaging);
    let restarts = Restarts::default();
    let hook: ui_egui::Relaunch = {
        let restarts = Arc::clone(&restarts);
        Arc::new(move |binary: &Path| {
            restarts
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(binary.to_owned());
        })
    };
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 820.0))
        .with_pixels_per_point(1.0)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx).with_relaunch(hook));
    settle(&mut harness);
    (harness, restarts)
}

fn fake() -> Arc<Fake> {
    Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")))
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
    steps(harness);
}

fn steps(harness: &mut Harness<'_, RickUi>) {
    for _ in 0..4 {
        harness.step();
    }
}

/// Clicks the one control called `label` and waits for what comes of it.
fn click(harness: &mut Harness<'_, RickUi>, label: &str) {
    harness.get_by_label(label).click();
    steps(harness);
    settle(harness);
}

/// Chooses a row of a list, or a side of the *Installed · Available*
/// control.
fn choose(harness: &mut Harness<'_, RickUi>, name: &str) {
    harness
        .get_by_role_and_label(Role::RadioButton, name)
        .click();
    steps(harness);
    settle(harness);
}

fn count(harness: &Harness<'_, RickUi>, label: &str) -> usize {
    harness.query_all_by_label(label).count()
}

fn listed(harness: &Harness<'_, RickUi>, name: &str) -> bool {
    harness
        .query_by_role_and_label(Role::RadioButton, name)
        .is_some()
}

#[test]
fn an_available_plugin_is_installed_and_loaded_after_restart_now() {
    let (server, signer) = (Server::start(), Signer::new(1));
    let package = probe_package();
    server.put(PACKAGE, package.clone());
    server.publish(
        &signer,
        &registry::index(&[probe(&server, &package), git()]),
    );
    let fake = fake();
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let second = dir.path().join("second");
    fs::create_dir(&second).unwrap_or_else(|e| panic!("{e}"));
    let (mut harness, restarts) = window(&Start {
        fake: &fake,
        settings: registry::rick_toml(&server, &signer, true),
        rick: RICK,
        folder: dir.path(),
        rows: Vec::new(),
        packaging: None,
    });
    // A session worth restoring: a second tab.
    harness
        .state_mut()
        .app_mut()
        .handle(app::Command::NewTab(local(&second)), &mut Vec::new());
    settle(&mut harness);

    // Nothing is installed; two plugins are on offer, and the control
    // counts them.
    harness.get_by_label("0 installed  ·  0 enabled  ·  0 need attention");
    assert!(!listed(&harness, "Probe"));
    choose(&mut harness, "Available 2");
    assert!(listed(&harness, "Probe") && listed(&harness, "Git"));
    // Under the headings of the installed list, each with its size.
    harness.get_by_label("PREVIEWERS");
    harness.get_by_label("ACTIONS");
    harness.get_by_label("4.6 MiB");
    assert_eq!(count(&harness, "restart"), 0);
    assert_eq!(count(&harness, "Restart now"), 0);

    // The details of the plugin: the chip, what it is for, its size and
    // its source, and Install where an installed plugin has its switch.
    choose(&mut harness, "Probe");
    harness.get_by_label("Available");
    harness.get_by_label(&format!(
        "probe  ·  {}  ·  Previewer",
        env!("CARGO_PKG_VERSION")
    ));
    harness.get_by_label("Source");
    let source = harness.state().app().plugin_registry().host.clone();
    assert!(source.starts_with("127.0.0.1:"), "{source}");
    harness.get_by_label(source.as_str());
    assert_eq!(count(&harness, "Enable Probe"), 0, "no switch");

    click(&mut harness, "Install");
    // In the per-user plugins folder; the row is under Installed, chosen,
    // with the restart chip, and no longer on offer.
    let per_user = fake.config_dir().join("plugins");
    assert!(per_user.join("probe").join(MANIFEST_FILE).is_file());
    assert!(listed(&harness, "Probe"));
    harness.get_by_label("1 installed  ·  1 enabled  ·  0 need attention");
    harness.get_by_label("User plugin");
    harness.get_by_label("Installed; loads at the next start");
    assert_eq!(count(&harness, "restart"), 2, "the row and the details");
    harness.get_by_role_and_label(Role::RadioButton, "Available 1");
    assert_eq!(harness.state().toasts(), [], "nothing went wrong");
    let record = fs::read_to_string(per_user.join("installed.toml")).unwrap_or_default();
    assert!(
        record.contains("[probe]\nsource = \"registry\""),
        "{record}"
    );

    // Restart now: the session is saved and Rick is asked to continue
    // from the binary it runs from.
    click(&mut harness, "Restart now");
    let asked = restarts
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    assert_eq!(
        asked,
        [std::env::current_exe().unwrap_or_else(|e| panic!("{e}"))]
    );
    // Every pane of the session with the folder it shows.
    let tabs = |app: &app::App| -> Vec<types::Location> {
        app.model()
            .all_panes()
            .into_iter()
            .filter_map(|pane| app.model().active_tab(pane))
            .map(|(_, tab)| tab.location.clone())
            .collect()
    };
    let before = tabs(harness.state().app());
    assert_eq!(before.len(), 2);
    drop(harness);

    // The process that starts: the host loads the plugin from the
    // per-user folder, and the session is back.
    let plugins = Host::load(&Options {
        dirs: vec![(Source::PerUser, per_user.clone())],
        ..Options::default()
    });
    let [row] = plugins.plugins() else {
        panic!("one plugin: {:?}", plugins.plugins());
    };
    assert_eq!(
        (row.id.as_str(), &row.status),
        ("probe", &PluginStatus::Loaded)
    );
    let restarted = app::App::new(app::Config {
        platform: fake.clone(),
        ui: Arc::new(EguiHost::new()),
        extensions: plugins.extensions(),
        config_dir: Some(fake.config_dir()),
        start: None,
    });
    assert_eq!(tabs(&restarted), before, "the session is back");
    drop(restarted);
    drop(plugins);
}

#[test]
fn a_package_that_differs_from_the_list_is_deleted_and_nothing_is_installed() {
    let (server, signer) = (Server::start(), Signer::new(1));
    let described = probe_package();
    // The registry serves other bytes than the list describes.
    let mut changed = described.clone();
    let middle = changed.len() / 2;
    changed[middle] ^= 0xff;
    server.put(PACKAGE, changed);
    server.publish(&signer, &registry::index(&[probe(&server, &described)]));
    let fake = fake();
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let (mut harness, _) = window(&Start {
        fake: &fake,
        settings: registry::rick_toml(&server, &signer, true),
        rick: RICK,
        folder: dir.path(),
        rows: Vec::new(),
        packaging: None,
    });
    choose(&mut harness, "Available 1");
    click(&mut harness, "Install");

    assert_eq!(
        harness.state().toasts(),
        [(
            Level::Error,
            "Install Probe: The download did not match the plugin list".to_owned()
        )]
    );
    let config = fake.config_dir();
    assert!(!config.join("plugins").join("probe").exists());
    let downloads = config.join(app::CACHE_DIR).join("downloads");
    assert_eq!(
        fs::read_dir(downloads).map(Iterator::count).ok(),
        Some(0),
        "the download is deleted"
    );
    // Still on offer, with Install to try again; nothing waits for a
    // restart.
    harness.get_by_role_and_label(Role::RadioButton, "Available 1");
    harness.get_by_label("Install");
    assert_eq!(count(&harness, "Restart now"), 0);
    harness.get_by_label("0 installed  ·  0 enabled  ·  0 need attention");
}

#[test]
fn an_index_with_a_bad_signature_is_not_used_and_the_view_says_so() {
    let (server, signer) = (Server::start(), Signer::new(1));
    server.publish(&signer, &registry::index(&[git()]));
    let fake = fake();
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let (mut harness, _) = window(&Start {
        fake: &fake,
        settings: registry::rick_toml(&server, &signer, true),
        rick: RICK,
        folder: dir.path(),
        rows: Vec::new(),
        packaging: None,
    });
    choose(&mut harness, "Available 1");
    assert!(listed(&harness, "Git"));
    // The strip under the list dates it and offers Check now.
    assert_eq!(count(&harness, "The plugin list could not be verified"), 0);
    harness.get_by_label_contains("Plugin list of ");

    // The registry now serves a list somebody else signed, with another
    // plugin in it.
    let other = registry::plugin(
        ext_svn::SvnExtension::new().manifest(),
        &[registry::listed(RICK, 1_126_400)],
    );
    server.publish(&Signer::new(2), &registry::index(&[other]));
    click(&mut harness, "Check now");
    harness.get_by_label("The plugin list could not be verified");
    harness.get_by_label("Check now");
    assert!(listed(&harness, "Git"), "the last good list is still used");
    assert!(!listed(&harness, "SVN"));
    harness.get_by_role_and_label(Role::RadioButton, "Available 1");

    // A list the registry's key signed takes the line away.
    let both = registry::index(&[
        git(),
        registry::plugin(
            ext_svn::SvnExtension::new().manifest(),
            &[registry::listed(RICK, 1_126_400)],
        ),
    ]);
    server.publish(&signer, &both);
    click(&mut harness, "Check now");
    assert_eq!(count(&harness, "The plugin list could not be verified"), 0);
    assert!(listed(&harness, "Git") && listed(&harness, "SVN"));

    // The footer names the address while rick.toml names one.
    harness.get_by_label(&format!(
        "The plugin list is read from {}.",
        server.index_url()
    ));
}

#[test]
fn a_build_without_a_key_says_the_list_is_not_set_up_and_asks_nobody() {
    let (server, signer) = (Server::start(), Signer::new(1));
    server.publish(&signer, &registry::index(&[git()]));
    // This build's own key: the committed file is empty, so the registry
    // is off wherever the address points. Should a key be compiled in
    // one day, a key that is none stands for the empty file.
    let built_with_a_key = plugins_registry::Source::new(None, None).key.is_some();
    let mut settings = format!("[registry]\nurl = \"{}\"\n", server.index_url());
    if built_with_a_key {
        settings.push_str("public_key = \"not a key\"\n");
    }
    let fake = fake();
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let (mut harness, _) = window(&Start {
        fake: &fake,
        settings,
        rick: RICK,
        folder: dir.path(),
        rows: Vec::new(),
        packaging: None,
    });
    // No count on the control: nothing is on offer.
    choose(&mut harness, "Available");
    harness.get_by_label("The plugin list is not set up in this build");
    assert_eq!(count(&harness, "Check now"), 0, "there is nothing to check");
    assert!(!listed(&harness, "Git"));
    assert_eq!(server.requests(), [], "nothing was fetched");
}

#[test]
fn a_pre_release_and_too_old_a_rick_say_why_nothing_is_offered() {
    let (server, signer) = (Server::start(), Signer::new(1));
    server.publish(&signer, &registry::index(&[git()]));
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));

    // 0.6.0-dev: the pre-release line and an empty list.
    let fake = self::fake();
    let (mut harness, _) = window(&Start {
        fake: &fake,
        settings: registry::rick_toml(&server, &signer, true),
        rick: "0.6.0-dev",
        folder: dir.path(),
        rows: Vec::new(),
        packaging: None,
    });
    choose(&mut harness, "Available");
    harness
        .get_by_label("Rick 0.6.0-dev is a pre-release. Plugins are published for releases only.");
    assert!(!listed(&harness, "Git"));
    assert_eq!(count(&harness, "Check now"), 0);
    assert_eq!(server.requests(), [], "a pre-release asks nobody");
    drop(harness);

    // A Rick older than the registry's oldest release.
    let fake = self::fake();
    let (mut harness, _) = window(&Start {
        fake: &fake,
        settings: registry::rick_toml(&server, &signer, true),
        rick: "0.5.0",
        folder: dir.path(),
        rows: Vec::new(),
        packaging: None,
    });
    choose(&mut harness, "Available");
    harness.get_by_label(
        "The plugins for Rick 0.5.0 are no longer published. Update Rick to install plugins.",
    );
    assert!(!listed(&harness, "Git"));
}

#[test]
fn offline_and_switched_off_say_so_in_one_line_with_check_now() {
    let (server, signer) = (Server::start(), Signer::new(1));
    server.publish(&signer, &registry::index(&[git()]));
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));

    // The switch off and nothing cached: one line, and Check now, which
    // is the user asking.
    let fake = self::fake();
    let (mut harness, _) = window(&Start {
        fake: &fake,
        settings: registry::rick_toml(&server, &signer, false),
        rick: RICK,
        folder: dir.path(),
        rows: Vec::new(),
        packaging: None,
    });
    choose(&mut harness, "Available");
    harness.get_by_label("Rick is set not to look for plugins online.");
    assert_eq!(server.requests(), [], "opening the view asked nobody");
    click(&mut harness, "Check now");
    assert_eq!(server.requests().len(), 2);
    assert!(listed(&harness, "Git"));
    harness.get_by_role_and_label(Role::RadioButton, "Available 1");
    drop(harness);

    // Nobody answers and nothing is cached: one line, and Check now.
    let (gone, signer) = (Server::start(), Signer::new(1));
    let settings = registry::rick_toml(&gone, &signer, true);
    runtime::block_on(gone.stop());
    let fake = self::fake();
    let (mut harness, _) = window(&Start {
        fake: &fake,
        settings,
        rick: RICK,
        folder: dir.path(),
        rows: Vec::new(),
        packaging: None,
    });
    choose(&mut harness, "Available");
    harness.get_by_label("The plugin list could not be fetched. Rick may be offline.");
    harness.get_by_label("Check now");
}

#[test]
fn the_search_narrows_the_available_list_and_opening_it_fetches() {
    let (server, signer) = (Server::start(), Signer::new(1));
    let svn = registry::plugin(
        ext_svn::SvnExtension::new().manifest(),
        &[registry::listed(RICK, 1_126_400)],
    );
    server.publish(&signer, &registry::index(&[git(), svn]));
    let fake = fake();
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let (mut harness, _) = window(&Start {
        fake: &fake,
        settings: registry::rick_toml(&server, &signer, true),
        rick: RICK,
        folder: dir.path(),
        rows: Vec::new(),
        packaging: None,
    });
    // The start fetched the two files; opening the view fetches again.
    assert_eq!(server.requests().len(), 2);
    choose(&mut harness, "Available 2");
    assert_eq!(server.requests().len(), 4);
    // Back and forth without leaving the view fetches nothing.
    choose(&mut harness, "SVN");
    assert_eq!(server.requests().len(), 4);

    // The dialogue's search field, under the control.
    let control = harness
        .get_by_role_and_label(Role::RadioButton, "Available 2")
        .rect();
    harness
        .get_all_by_role(Role::TextInput)
        .find(|field| {
            let field = field.rect();
            field.top() > control.bottom() && field.top() < control.bottom() + 40.0
        })
        .unwrap_or_else(|| panic!("the search field"))
        .click();
    steps(&mut harness);
    let typed = |harness: &mut Harness<'_, RickUi>, text: &str| {
        harness
            .get_all_by_role(Role::TextInput)
            .find(egui_kittest::Node::is_focused)
            .unwrap_or_else(|| panic!("the search field has the focus"))
            .type_text(text);
        steps(harness);
    };
    typed(&mut harness, "sv");
    assert!(listed(&harness, "SVN") && !listed(&harness, "Git"));
    // The control counts what is on offer, not what the search leaves.
    harness.get_by_role_and_label(Role::RadioButton, "Available 2");
    typed(&mut harness, "zzz");
    harness.get_by_label("No extension matches");
}

/// The plugin rows as the binary hands them to the application.
fn rows(host: &Host) -> Vec<app::Plugin> {
    host.plugins()
        .iter()
        .map(|p| app::Plugin {
            id: p.id.clone(),
            name: p.name.clone(),
            description: p.description.clone(),
            version: p.version.clone(),
            points: p.points,
            previews: p.previews.clone(),
            status: match &p.status {
                PluginStatus::Loaded => app::PluginStatus::Loaded,
                PluginStatus::Disabled => app::PluginStatus::Disabled,
                PluginStatus::Refused(reason) => app::PluginStatus::Refused(reason.clone()),
            },
            path: p.dir.clone(),
            built_in: p.source == Source::BesideBinary,
            removed: false,
        })
        .collect()
}

/// A config directory as a Rick update leaves it: the probe plugin in
/// the per-user plugins folder, installed from the registry and built for
/// Rick 0.4.0. Returns the rows of a host that scanned it.
fn a_plugin_of_the_rick_before(fake: &Arc<Fake>, package: &[u8]) -> Vec<app::Plugin> {
    let per_user = fake.config_dir().join("plugins");
    let scratch = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let file = scratch.path().join("probe.rkp");
    fs::write(&file, package).unwrap_or_else(|e| panic!("{e}"));
    let (plugin, _) = app::plugins::unpack(&file, &per_user, app::plugins::Origin::Registry, None)
        .unwrap_or_else(|e| panic!("{e}"));
    // The manifest of the release before.
    let manifest = plugin.path.join(MANIFEST_FILE);
    let text = fs::read_to_string(&manifest).unwrap_or_else(|e| panic!("{e}"));
    let own = format!("rick = \"{}\"", plugins_native::HOST_RICK);
    assert!(text.contains(&own), "{text}");
    fs::write(&manifest, text.replace(&own, "rick = \"0.4.0\"")).unwrap_or_else(|e| panic!("{e}"));
    let host = Host::load(&Options {
        dirs: vec![(Source::PerUser, per_user)],
        ..Options::default()
    });
    let rows = rows(&host);
    assert_eq!(
        rows[0].status,
        app::PluginStatus::Refused("Built for Rick 0.4.0".to_owned()),
        "the host refuses the build of another Rick"
    );
    rows
}

#[test]
fn a_plugin_built_for_the_rick_before_is_updated_and_loads_after_a_restart() {
    let (server, signer) = (Server::start(), Signer::new(1));
    let package = probe_package();
    server.put(PACKAGE, package.clone());
    server.publish(
        &signer,
        &registry::index(&[probe(&server, &package), git()]),
    );
    let fake = fake();
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let (mut harness, _) = window(&Start {
        fake: &fake,
        settings: registry::rick_toml(&server, &signer, true),
        rick: RICK,
        folder: dir.path(),
        rows: a_plugin_of_the_rick_before(&fake, &package),
        packaging: None,
    });

    // The header counts the update, the card reads the host's reason and
    // offers Update, and the footer has Update all.
    harness.get_by_label("1 installed  ·  1 enabled  ·  1 needs attention  ·  1 update");
    assert_eq!(
        count(&harness, "Needs attention"),
        2,
        "the row's dot and the card"
    );
    assert_eq!(
        count(&harness, "Built for Rick 0.4.0"),
        2,
        "the card, and the Status row under it"
    );
    assert_eq!(count(&harness, "Update"), 1);
    assert_eq!(
        count(&harness, "Update all"),
        2,
        "the footer's, and the one of the start's notice behind the dialogue"
    );
    assert_eq!(count(&harness, "Restart now"), 0);
    // It is in the folder, so it is not on offer: only Git is.
    harness.get_by_role_and_label(Role::RadioButton, "Available 1");

    click(&mut harness, "Update");
    // An install over the old folder: the build of this Rick is there,
    // the record says so, and the row waits for the next start.
    let per_user = fake.config_dir().join("plugins");
    let manifest = fs::read_to_string(per_user.join("probe").join(MANIFEST_FILE))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(
        manifest.contains(&format!("rick = \"{}\"", plugins_native::HOST_RICK)),
        "{manifest}"
    );
    harness.get_by_label("1 installed  ·  1 enabled  ·  0 need attention");
    assert_eq!(count(&harness, "Built for Rick 0.4.0"), 0);
    assert_eq!(count(&harness, "Update"), 0);
    assert_eq!(count(&harness, "Update all"), 0);
    assert_eq!(count(&harness, "restart"), 2);
    harness.get_by_label("Restart now");
    // Nothing failed, and the start's notice went with the last update.
    assert_eq!(harness.state().toasts(), []);
    drop(harness);

    // The next start loads it.
    let host = Host::load(&Options {
        dirs: vec![(Source::PerUser, per_user)],
        ..Options::default()
    });
    assert_eq!(host.plugins()[0].status, PluginStatus::Loaded);
    drop(host);
}

#[test]
fn the_first_start_that_finds_updates_says_so_with_update_all() {
    let (server, signer) = (Server::start(), Signer::new(1));
    let package = probe_package();
    server.put(PACKAGE, package.clone());
    server.publish(&signer, &registry::index(&[probe(&server, &package)]));
    let fake = fake();
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let rows = a_plugin_of_the_rick_before(&fake, &package);
    let start = |rows: Vec<app::Plugin>| Start {
        fake: &fake,
        settings: registry::rick_toml(&server, &signer, true),
        rick: RICK,
        folder: dir.path(),
        rows,
        packaging: None,
    };

    // The start reads the list, finds the update and says so.
    let (harness, _) = shell(&start(rows.clone()));
    assert_eq!(
        harness.state().toasts(),
        [(
            Level::Info,
            "1 plugin needs its build for Rick 0.6.0".to_owned()
        )]
    );
    harness.get_by_label("Update all");
    drop(harness);

    // The next start of this Rick says nothing: it was said. Its button
    // would have updated; the dialogue's footer still does.
    let (mut harness, _) = shell(&start(rows.clone()));
    assert_eq!(harness.state().toasts(), []);
    let pane = harness.state().app().model().focused;
    harness.state_mut().queue(Intent::Extensions, pane);
    steps(&mut harness);
    click(&mut harness, "Update all");
    let per_user = fake.config_dir().join("plugins");
    let manifest = fs::read_to_string(per_user.join("probe").join(MANIFEST_FILE))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(
        manifest.contains(&format!("rick = \"{}\"", plugins_native::HOST_RICK)),
        "{manifest}"
    );
    assert_eq!(count(&harness, "Update all"), 0);
}

#[test]
fn update_all_of_the_toast_installs_the_updates() {
    let (server, signer) = (Server::start(), Signer::new(1));
    let package = probe_package();
    server.put(PACKAGE, package.clone());
    server.publish(&signer, &registry::index(&[probe(&server, &package)]));
    let fake = fake();
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let (mut harness, _) = shell(&Start {
        fake: &fake,
        settings: registry::rick_toml(&server, &signer, true),
        rick: RICK,
        folder: dir.path(),
        rows: a_plugin_of_the_rick_before(&fake, &package),
        packaging: None,
    });
    click(&mut harness, "Update all");
    // The notice went with its button; the plugin is this Rick's build.
    assert_eq!(
        harness
            .state()
            .toasts()
            .iter()
            .filter(|(_, text)| text.contains("needs its build"))
            .count(),
        0
    );
    let manifest = fs::read_to_string(
        fake.config_dir()
            .join("plugins")
            .join("probe")
            .join(MANIFEST_FILE),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert!(
        manifest.contains(&format!("rick = \"{}\"", plugins_native::HOST_RICK)),
        "{manifest}"
    );
    assert_eq!(harness.state().app().updates(), []);
}

#[test]
fn a_packaged_rick_shows_the_command_of_its_package_manager_and_no_install() {
    use app::plugins::{PACKAGE_FILE, Packaging};
    let (server, signer) = (Server::start(), Signer::new(1));
    server.publish(&signer, &registry::index(&[git()]));
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    for (says, command) in [
        ("deb", "sudo apt install rick-plugin-git"),
        ("rpm", "sudo dnf install rick-plugin-git"),
    ] {
        // The file `package` beside the binary, as the deb and the rpm
        // install it.
        let bin = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        fs::write(bin.path().join(PACKAGE_FILE), says).unwrap_or_else(|e| panic!("{e}"));
        let packaging = Packaging::beside(&bin.path().join("rick"));
        assert!(packaging.is_some(), "{says}");
        let fake = self::fake();
        let (mut harness, _) = window(&Start {
            fake: &fake,
            settings: registry::rick_toml(&server, &signer, true),
            rick: RICK,
            folder: dir.path(),
            rows: Vec::new(),
            packaging,
        });
        choose(&mut harness, "Available 1");
        assert!(listed(&harness, "Git"));
        harness.get_by_label("Available");
        harness.get_by_label("Installed through your package manager");
        harness.get_by_label(command);
        assert_eq!(count(&harness, "Install"), 0, "{says}: no Install");
        // Nothing is downloaded, so nothing is said about a download.
        assert_eq!(count(&harness, "4.6 MiB"), 0);
        assert_eq!(count(&harness, "Size"), 0);
        assert_eq!(count(&harness, "Source"), 0);
        // The command copies whole.
        click(&mut harness, "Copy command");
        assert_eq!(fake.clipboard_text().as_deref(), Some(command));
    }

    // An archive has no such file, and Install.
    let bin = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(Packaging::beside(&bin.path().join("rick")), None);
    let fake = self::fake();
    let (mut harness, _) = window(&Start {
        fake: &fake,
        settings: registry::rick_toml(&server, &signer, true),
        rick: RICK,
        folder: dir.path(),
        rows: Vec::new(),
        packaging: None,
    });
    choose(&mut harness, "Available 1");
    harness.get_by_label("Install");
    assert_eq!(count(&harness, "Installed through your package manager"), 0);
}

#[test]
fn install_from_url_downloads_a_package_and_handles_it_as_a_file() {
    let (server, signer) = (Server::start(), Signer::new(1));
    server.publish(&signer, &registry::index(&[git()]));
    // A package somewhere the plugin list knows nothing of.
    let package = probe_package();
    server.put("somewhere/probe-1.0.rkp", package.clone());
    let address = server.url("somewhere/probe-1.0.rkp");
    let fake = fake();
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let (mut harness, _) = window(&Start {
        fake: &fake,
        settings: registry::rick_toml(&server, &signer, true),
        rick: RICK,
        folder: dir.path(),
        rows: Vec::new(),
        packaging: None,
    });

    // Beside Install from file…, in the footer.
    let file = harness.get_by_label("Install from file…").rect();
    let url = harness.get_by_label("Install from URL…").rect();
    assert!(url.right() <= file.left() && (url.top() - file.top()).abs() < 1.0);
    click(&mut harness, "Install from URL…");
    harness.get_by_label("Install from URL");
    harness.get_by_label("Nothing is checked, as for a file.");
    let typed = |harness: &mut Harness<'_, RickUi>, text: &str| {
        harness
            .get_all_by_role(Role::TextInput)
            .find(egui_kittest::Node::is_focused)
            .unwrap_or_else(|| panic!("the address field has the focus"))
            .type_text(text);
        steps(harness);
    };
    // What is no https:// address is said, and Install takes no click.
    typed(&mut harness, "ftp://example.org/probe.rkp");
    harness.get_by_label("Enter the https:// address of a plugin package.");
    click(&mut harness, "Install");
    assert!(!fake.config_dir().join("plugins").exists());
    harness.get_by_label("Install from URL");

    // Escape closes the prompt and leaves the dialogue.
    harness.key_press(egui::Key::Escape);
    steps(&mut harness);
    assert_eq!(count(&harness, "Install from URL"), 0);
    assert!(harness.state().extensions_open());

    // An address: the package is downloaded, unpacked and noted with
    // where it came from.
    click(&mut harness, "Install from URL…");
    typed(&mut harness, &address);
    assert_eq!(
        count(&harness, "Enter the https:// address of a plugin package."),
        0
    );
    click(&mut harness, "Install");
    assert_eq!(count(&harness, "Install from URL"), 0, "the prompt is gone");
    let per_user = fake.config_dir().join("plugins");
    assert!(per_user.join("probe").join(MANIFEST_FILE).is_file());
    assert!(listed(&harness, "Probe"));
    harness.get_by_label("User plugin");
    assert_eq!(count(&harness, "restart"), 2);
    assert_eq!(harness.state().toasts(), []);
    let record = fs::read_to_string(per_user.join("installed.toml")).unwrap_or_default();
    assert!(record.contains("[probe]\nsource = \"url\""), "{record}");
    assert!(record.contains(&format!("url = \"{address}\"")), "{record}");
    assert!(record.contains(&sha256(&package)), "{record}");
}
