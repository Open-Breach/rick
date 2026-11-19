//! Acceptance (issue 16): the plugin host and the first plugin.
//!
//! Fixture plugins with a wrong API version, a wrong toolchain, another
//! platform and a panicking entry point are refused or caught, listed with
//! their reason, and the host keeps running; a plugin built for another
//! Rick is refused for that before its library is opened, the manifest
//! file says what each plugin is for, and the package of a separate plugin
//! installs from file and loads (issue 250); a per-user plugin wins over
//! the one beside the binary; disabling a plugin writes `rick.toml` and the
//! next start lists it disabled; a job spawned inside a plugin shows in the
//! job table and cancels from it; the Git plugin decorates a work tree and
//! brings its action group exactly as the compiled-in extension did;
//! loading it costs at most 50 ms on Linux; and loading every plugin leaves
//! the process environment as it was (issue 264).

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use app::{App, Command, Config, Event};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use ext::{API_VERSION, ActionGroup, ActionId, Invocation, Platforms};
use model::{JobState, SettingsChange};
use platform::{ConfigDir, Fake};
use plugins_native::{Host, Library, MANIFEST_FILE, Options, PluginStatus, PluginToml, Source};
use tests::local;
use ui_egui::{EguiHost, Intent, RickUi};

/// The directory the test binary runs from, where cargo puts every
/// dependency's `cdylib` under its plain name.
fn deps_dir() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_else(|e| panic!("{e}"));
    exe.parent()
        .unwrap_or_else(|| panic!("no parent of {}", exe.display()))
        .to_owned()
}

/// The library file of the crate `stem` (`plugin_probe`, `ext_git`).
fn library_of(stem: &str) -> PathBuf {
    let path = deps_dir().join(format!(
        "{}{stem}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    assert!(path.is_file(), "not built: {}", path.display());
    path
}

/// Installs the library of the crate `stem` as a plugin into `plugins_dir`,
/// with the manifest the bundler would write, after `edit` had its say.
/// The manifest is read from the probe plugin when `stem` is a library that
/// cannot describe itself (the panicking fixture).
fn install(plugins_dir: &Path, stem: &str, edit: impl FnOnce(&mut PluginToml)) -> PathBuf {
    let library = library_of(stem);
    let mut toml = plugins_native::describe(&library)
        .or_else(|_| plugins_native::describe(&library_of("plugin_probe")))
        .unwrap_or_else(|e| panic!("{e}"));
    toml.library = Library::for_crate(stem);
    edit(&mut toml);
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

/// Lays `plugins_dir` out as the bundler lays out a bundle's: every plugin
/// library built beside this binary in a folder named after its id, with
/// the manifest the bundler would write. Returns the ids.
fn install_every_plugin(plugins_dir: &Path) -> Vec<String> {
    let (prefix, suffix) = (std::env::consts::DLL_PREFIX, std::env::consts::DLL_SUFFIX);
    let mut stems: Vec<String> = fs::read_dir(deps_dir())
        .unwrap_or_else(|e| panic!("{e}"))
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let stem = name.strip_prefix(prefix)?.strip_suffix(suffix)?;
            // A plugin crate is an `ext-*` one built as a library file;
            // cargo names the file after the crate, with no hash.
            stem.starts_with("ext_").then(|| stem.to_owned())
        })
        .collect();
    stems.sort();
    let mut ids = Vec::new();
    for stem in stems {
        let library = library_of(&stem);
        let mut toml = plugins_native::describe(&library)
            .unwrap_or_else(|e| panic!("{}: {e}", library.display()));
        toml.library = Library::for_crate(&stem);
        let dir = plugins_dir.join(&toml.id);
        toml.write(&dir.join(MANIFEST_FILE))
            .unwrap_or_else(|e| panic!("{e}"));
        let file = toml
            .library
            .for_os(std::env::consts::OS)
            .unwrap_or_else(|| panic!("no library name for this OS"));
        fs::copy(&library, dir.join(file)).unwrap_or_else(|e| panic!("{e}"));
        ids.push(toml.id);
    }
    ids
}

fn options(dirs: Vec<(Source, PathBuf)>) -> Options {
    Options {
        dirs,
        ..Options::default()
    }
}

fn status_of<'a>(host: &'a Host, id: &str) -> &'a PluginStatus {
    &host
        .plugins()
        .iter()
        .find(|p| p.id == id)
        .unwrap_or_else(|| panic!("no plugin {id} in {:?}", host.plugins()))
        .status
}

fn refused_with(host: &Host, id: &str, text: &str) {
    match status_of(host, id) {
        PluginStatus::Refused(reason) if reason.contains(text) => {}
        other => panic!("{id}: expected a refusal mentioning {text:?}, got {other:?}"),
    }
}

#[test]
fn fixture_plugins_are_refused_or_caught_and_the_host_keeps_running() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let plugins = dir.path().join("plugins");
    install(&plugins, "plugin_probe", |t| {
        t.id = "probe-api".into();
        t.api_version = API_VERSION + 1;
    });
    // Built against the contract before a manifest said what the plugin
    // is for (issue 250): API 22.
    install(&plugins, "plugin_probe", |t| {
        t.id = "probe-api-22".into();
        t.api_version = 22;
    });
    install(&plugins, "plugin_probe", |t| {
        t.id = "probe-toolchain".into();
        t.toolchain = "rustc 0.0.0 (nowhere 2000-01-01)".into();
    });
    let elsewhere = if Platforms::current() == Platforms::LINUX {
        "windows"
    } else {
        "linux"
    };
    install(&plugins, "plugin_probe", |t| {
        t.id = "probe-platform".into();
        t.platforms = vec![elsewhere.to_owned()];
    });
    install(&plugins, "plugin_panic", |t| {
        t.id = "panic".into();
        t.name = "Panic".into();
    });
    install(&plugins, "plugin_probe", |_| {});
    fs::create_dir(plugins.join("empty")).unwrap_or_else(|e| panic!("{e}"));

    let host = Host::load(&options(vec![(Source::BesideBinary, plugins)]));
    refused_with(&host, "probe-api", "API version");
    assert_eq!(API_VERSION, 23, "the contract of the triggers");
    refused_with(&host, "probe-api-22", "API version");
    refused_with(&host, "probe-toolchain", "built with");
    refused_with(&host, "probe-platform", "built for");
    refused_with(&host, "panic", "panicked during registration");
    refused_with(&host, "empty", "no plugin.toml");
    assert_eq!(status_of(&host, "probe"), &PluginStatus::Loaded);
    assert_eq!(host.plugins().len(), 7, "{:?}", host.plugins());

    // The host keeps running: the one good plugin is an extension that
    // registers into an application registry.
    let extensions = host.extensions();
    assert_eq!(extensions.len(), 1);
    let mut registry = ext::Registry::new();
    registry
        .load(&*extensions[0])
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(registry.extensions(), &[types::ExtId("probe")]);
    assert_eq!(registry.previewers().len(), 1);
    assert_eq!(registry.actions().len(), 1);
}

/// The reason the host refused the plugin `id` with, word for word.
fn refusal<'a>(host: &'a Host, id: &str) -> &'a str {
    match status_of(host, id) {
        PluginStatus::Refused(reason) => reason,
        other => panic!("{id}: expected a refusal, got {other:?}"),
    }
}

/// Issue 250, item 2: `plugin.toml` names the Rick the plugin was built
/// with, and the host refuses another one's before it opens the library
/// (specs/plugin-distribution.md §3).
#[test]
fn a_plugin_built_for_another_rick_is_refused_and_its_library_never_opened() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let plugins = dir.path().join("plugins");
    // The bundler's manifest names this build's Rick, and loads.
    let own = install(&plugins, "plugin_probe", |t| {
        assert_eq!(t.rick, plugins_native::HOST_RICK);
        assert_eq!(t.rick, env!("CARGO_PKG_VERSION"), "the workspace's version");
    });
    // The release before, with a library that would load.
    let before = install(&plugins, "plugin_probe", |t| {
        t.id = "probe-before".into();
        t.rick = "0.4.1".into();
    });
    // The same with a library whose entry point panics, and with a file
    // that is no library: opening either would give another reason.
    let panics = install(&plugins, "plugin_panic", |t| {
        t.id = "panic-before".into();
        t.rick = "0.4.1".into();
    });
    let broken = install(&plugins, "plugin_probe", |t| {
        t.id = "broken-before".into();
        t.rick = "0.4.1".into();
    });
    let library = Library::for_crate("plugin_probe");
    let library = library
        .for_os(std::env::consts::OS)
        .unwrap_or_else(|| panic!("no library name for this OS"));
    fs::write(broken.join(library), "not a library").unwrap_or_else(|e| panic!("{e}"));
    // A manifest from before the field existed.
    let unknown = install(&plugins, "plugin_probe", |t| t.id = "probe-unknown".into());
    let manifest = unknown.join(MANIFEST_FILE);
    let text = fs::read_to_string(&manifest).unwrap_or_else(|e| panic!("{e}"));
    let without = text.replace(&format!("rick = \"{}\"\n", plugins_native::HOST_RICK), "");
    assert_ne!(without, text, "the manifest names its Rick: {text}");
    fs::write(&manifest, without).unwrap_or_else(|e| panic!("{e}"));
    // A release to come, whose manifest holds a key this host does not
    // know: still refused for its release, and listed by its name.
    let later = install(&plugins, "plugin_probe", |t| {
        t.id = "probe-later".into();
        t.name = "Later probe".into();
        t.version = "99.0.0".into();
        t.rick = "99.0.0".into();
    });
    let manifest = later.join(MANIFEST_FILE);
    let text = fs::read_to_string(&manifest).unwrap_or_else(|e| panic!("{e}"));
    fs::write(&manifest, format!("holograms = true\n{text}")).unwrap_or_else(|e| panic!("{e}"));

    let host = Host::load(&options(vec![(Source::BesideBinary, plugins)]));
    assert_eq!(status_of(&host, "probe"), &PluginStatus::Loaded);
    assert_eq!(refusal(&host, "probe-before"), "Built for Rick 0.4.1");
    assert_eq!(refusal(&host, "panic-before"), "Built for Rick 0.4.1");
    assert_eq!(refusal(&host, "broken-before"), "Built for Rick 0.4.1");
    assert_eq!(refusal(&host, "probe-unknown"), "Built for an unknown Rick");
    assert_eq!(refusal(&host, "probe-later"), "Built for Rick 99.0.0");
    let row = host
        .plugins()
        .iter()
        .find(|p| p.id == "probe-later")
        .unwrap_or_else(|| panic!("{:?}", host.plugins()));
    assert_eq!(
        (row.name.as_str(), row.version.as_str()),
        ("Later probe", "99.0.0")
    );
    assert_eq!(host.extensions().len(), 1, "{:?}", host.plugins());

    // The proof that no library of another release was opened: the list
    // the kernel keeps of the files this process has mapped names the one
    // plugin that loaded and none of the refused ones. The list is Linux's;
    // elsewhere the reasons above stand for it, since a library that
    // panics and a file that is none were refused for their release.
    if std::env::consts::OS == "linux" {
        let maps = fs::read_to_string("/proc/self/maps").unwrap_or_else(|e| panic!("{e}"));
        let mapped = |dir: &Path| {
            let dir = fs::canonicalize(dir).unwrap_or_else(|e| panic!("{e}"));
            maps.contains(&format!("{}/", dir.display()))
        };
        assert!(mapped(&own), "the loaded plugin's library is mapped");
        for dir in [&before, &panics, &broken, &unknown, &later] {
            assert!(!mapped(dir), "{} was opened", dir.display());
        }
    }
    drop(host);
}

/// Issue 250, item 3: a manifest says what the plugin is for, and
/// `plugin.toml` carries it (specs/plugin-distribution.md §5). Git and
/// Subversion name the entry that marks their folders, SSH and the three
/// object stores the kind of connection they add, titled as its entry of
/// *Add…* is; a plugin with neither has two empty lists.
#[test]
fn plugin_toml_carries_what_each_plugin_is_for() {
    let triggers = |stem: &str| {
        let toml = plugins_native::describe(&library_of(stem)).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(toml.api_version, API_VERSION, "{stem}");
        let connections: Vec<(String, String)> = toml
            .connections
            .into_iter()
            .map(|c| (c.section, c.title))
            .collect();
        (toml.markers, connections)
    };
    let marker = |name: &str| (vec![name.to_owned()], Vec::new());
    let connection = |section: &str, title: &str| {
        (
            Vec::<String>::new(),
            vec![(section.to_owned(), title.to_owned())],
        )
    };
    assert_eq!(triggers("ext_git"), marker(".git"));
    assert_eq!(triggers("ext_svn"), marker(".svn"));
    assert_eq!(triggers("ext_ssh"), connection("network", "SSH"));
    assert_eq!(triggers("ext_vfs_s3"), connection("cloud", "S3"));
    assert_eq!(triggers("ext_vfs_azure"), connection("cloud", "Azure Blob"));
    assert_eq!(
        triggers("ext_vfs_gcs"),
        connection("cloud", "Google Cloud Storage")
    );
    for stem in ["ext_vfs_trash", "ext_preview_hex", "plugin_probe"] {
        assert_eq!(triggers(stem), (Vec::new(), Vec::new()), "{stem}");
    }

    // The titles are those of the entries of *Add…*: the English of each
    // dialogue's title.
    for (english, key, title) in [
        (
            include_str!("../../ext-ssh/locales/en.ftl"),
            "ssh-ssh",
            "SSH",
        ),
        (
            include_str!("../../ext-vfs-s3/locales/en.ftl"),
            ext_vfs_s3::schema().title.0,
            "S3",
        ),
        (
            include_str!("../../ext-vfs-azure/locales/en.ftl"),
            ext_vfs_azure::schema().title.0,
            "Azure Blob",
        ),
        (
            include_str!("../../ext-vfs-gcs/locales/en.ftl"),
            ext_vfs_gcs::schema().title.0,
            "Google Cloud Storage",
        ),
    ] {
        assert!(
            english
                .lines()
                .any(|line| line == format!("{key} = {title}")),
            "{key} is not titled {title}"
        );
    }

    // The file as the bundler writes it, for Git and for S3.
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let written = |stem: &str| {
        let folder = install(dir.path(), stem, |_| {});
        fs::read_to_string(folder.join(MANIFEST_FILE)).unwrap_or_else(|e| panic!("{e}"))
    };
    let git = written("ext_git");
    assert!(git.contains("\nmarkers = [\".git\"]\n"), "{git}");
    assert!(!git.contains("[[connections]]"), "{git}");
    let s3 = written("ext_vfs_s3");
    assert!(s3.contains("\nmarkers = []\n"), "{s3}");
    assert!(
        s3.contains("\n[[connections]]\nsection = \"cloud\"\ntitle = \"S3\"\n"),
        "{s3}"
    );
}

/// Issue 250, item 4: the package `cargo xtask package` makes of a
/// separate plugin (specs/plugin-distribution.md §4) is what *Install from
/// file…* reads, the plugin unpacked from it loads at the next start, and
/// the same bundle gives the same bytes. The packaging is the task's own
/// code, run here on a real plugin library.
#[test]
fn a_plugin_s_package_installs_from_file_and_loads_from_the_per_user_folder() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    // A bundle's plugins folder: Git, which is separate, with a file
    // beside its library, and the trash, which is of the default set.
    let bundle = dir.path().join("bundle").join("plugins");
    let git = install(&bundle, "ext_git", |_| {});
    fs::create_dir(git.join("share")).unwrap_or_else(|e| panic!("{e}"));
    fs::write(git.join("share").join("notice.txt"), "beside the library")
        .unwrap_or_else(|e| panic!("{e}"));
    install(&bundle, "ext_vfs_trash", |_| {});
    let separate: BTreeSet<String> = ["ext_git".to_owned()].into_iter().collect();
    let os = std::env::consts::OS;
    let platform = xtask::package::platform_name(os, std::env::consts::ARCH);
    let pack = |out: &Path| {
        xtask::package::write(
            &bundle,
            out,
            &platform,
            os,
            plugins_native::HOST_RICK,
            &separate,
        )
        .unwrap_or_else(|e| panic!("{e}"))
    };
    let first = dir.path().join("packages");
    let second = dir.path().join("again");
    let made = pack(&first);
    assert_eq!(pack(&second), made);
    let [package] = made.as_slice() else {
        panic!("one package, of the one separate plugin: {made:?}");
    };
    assert_eq!(
        package.file,
        format!("git-{}-{platform}.rkp", env!("CARGO_PKG_VERSION"))
    );
    for name in [
        package.file.clone(),
        xtask::package::manifest_name(&platform),
    ] {
        let one = fs::read(first.join(&name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        let two = fs::read(second.join(&name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(one == two, "{name} differs between two runs");
    }

    // *Install from file…* takes the package into an empty per-user
    // plugins folder, the folder whole.
    let per_user = dir.path().join("config").join("plugins");
    let folder = app::plugins::install(&first.join(&package.file), &per_user)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(folder, per_user.join("git"));
    assert_eq!(
        fs::read_to_string(folder.join("share").join("notice.txt")).ok(),
        Some("beside the library".to_owned())
    );
    // The next start loads the plugin from there.
    let host = Host::load(&options(vec![(Source::PerUser, per_user)]));
    assert_eq!(
        status_of(&host, "git"),
        &PluginStatus::Loaded,
        "{:?}",
        host.plugins()
    );
    assert_eq!(host.plugins()[0].dir, folder);
    assert_eq!(host.extensions().len(), 1);
    drop(host);
}

#[test]
fn a_per_user_plugin_wins_over_the_one_beside_the_binary() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let beside = dir.path().join("bin").join("plugins");
    let per_user = dir.path().join("config").join("plugins");
    let beside_dir = install(&beside, "plugin_probe", |_| {});
    let user_dir = install(&per_user, "plugin_probe", |_| {});

    let host = Host::load(&options(vec![
        (Source::BesideBinary, beside),
        (Source::PerUser, per_user),
    ]));
    let rows = host.plugins();
    assert_eq!(rows.len(), 2, "{rows:?}");
    let winner = rows
        .iter()
        .find(|p| p.source == Source::PerUser)
        .unwrap_or_else(|| panic!("{rows:?}"));
    assert_eq!(winner.status, PluginStatus::Loaded);
    assert_eq!(winner.dir, user_dir);
    let loser = rows
        .iter()
        .find(|p| p.source == Source::BesideBinary)
        .unwrap_or_else(|| panic!("{rows:?}"));
    assert_eq!(loser.dir, beside_dir);
    match &loser.status {
        PluginStatus::Refused(reason) => assert!(
            reason.contains("shadowed") && reason.contains(&user_dir.display().to_string()),
            "{reason}"
        ),
        other => panic!("{other:?}"),
    }
    assert_eq!(host.extensions().len(), 1);
    assert_eq!(
        host.loaded()[0].path(),
        user_dir.join(
            Library::for_crate("plugin_probe")
                .for_os(std::env::consts::OS)
                .unwrap_or_default()
        )
    );
}

/// Runs the app's loop until nothing has arrived for a while.
fn settle(app: &mut App, events: &mut Vec<Event>) {
    let deadline = Instant::now() + Duration::from_secs(30);
    runtime::block_on(async {
        loop {
            let quiet = tokio::time::timeout(Duration::from_millis(200), app.tick(events))
                .await
                .is_err();
            if quiet && !app.is_busy() {
                break;
            }
            assert!(Instant::now() < deadline, "did not settle: {app:?}");
        }
    });
}

/// Waits until `check` holds, driving the app meanwhile.
fn wait_for(app: &mut App, events: &mut Vec<Event>, what: &str, check: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !check(app) {
        assert!(Instant::now() < deadline, "{what} did not happen: {app:?}");
        runtime::block_on(async {
            let _ = tokio::time::timeout(Duration::from_millis(50), app.tick(events)).await;
        });
    }
}

#[test]
fn disabling_a_plugin_writes_rick_toml_and_the_next_start_lists_it_disabled() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let plugins = dir.path().join("plugins");
    install(&plugins, "plugin_probe", |_| {});
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let config_dir = fake.config_dir();
    let settings_path = config_dir.join(app::SETTINGS_FILE);
    let ui = Arc::new(EguiHost::new());
    let mut events = Vec::new();
    let mut app = App::new(Config {
        platform: fake,
        ui,
        extensions: Vec::new(),
        config_dir: Some(config_dir.clone()),
        start: Some(local(dir.path())),
    });
    settle(&mut app, &mut events);

    // The Extensions screen's toggle: off.
    app.handle(
        Command::Settings(SettingsChange::Plugin {
            id: "probe".into(),
            enabled: false,
        }),
        &mut events,
    );
    assert_eq!(app.model().settings.plugins.disabled, ["probe"]);
    wait_for(&mut app, &mut events, "rick.toml written", |_| {
        fs::read_to_string(&settings_path).is_ok_and(|t| t.contains("disabled = [\"probe\"]"))
    });
    let text = fs::read_to_string(&settings_path).unwrap_or_else(|e| panic!("{e}"));
    assert!(text.contains("[plugins]"), "{text}");

    // The next start reads the file and lists the plugin disabled, not loaded.
    let settings = app::settings::load(&settings_path).unwrap_or_else(|e| panic!("{e}"));
    let host = Host::load(&Options {
        dirs: vec![(Source::BesideBinary, plugins.clone())],
        disabled: settings.plugins.disabled,
        ..Options::default()
    });
    assert_eq!(status_of(&host, "probe"), &PluginStatus::Disabled);
    assert_eq!(host.extensions().len(), 0);

    // Enabling restores it.
    app.handle(
        Command::Settings(SettingsChange::Plugin {
            id: "probe".into(),
            enabled: true,
        }),
        &mut events,
    );
    wait_for(&mut app, &mut events, "rick.toml rewritten", |_| {
        fs::read_to_string(&settings_path).is_ok_and(|t| !t.contains("\"probe\""))
    });
    let settings = app::settings::load(&settings_path).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(settings.plugins.disabled, [] as [String; 0]);
    let host = Host::load(&Options {
        dirs: vec![(Source::BesideBinary, plugins)],
        disabled: settings.plugins.disabled,
        ..Options::default()
    });
    assert_eq!(status_of(&host, "probe"), &PluginStatus::Loaded);
}

#[test]
fn a_job_spawned_inside_the_plugin_shows_in_the_job_table_and_cancels_from_it() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let plugins = dir.path().join("plugins");
    install(&plugins, "plugin_probe", |_| {});
    let host = Host::load(&options(vec![(Source::BesideBinary, plugins)]));
    assert_eq!(status_of(&host, "probe"), &PluginStatus::Loaded);

    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let ui = Arc::new(EguiHost::new());
    let mut events = Vec::new();
    let mut app = App::new(Config {
        platform: fake.clone(),
        ui,
        extensions: host.extensions(),
        config_dir: Some(fake.config_dir()),
        start: Some(local(dir.path())),
    });
    settle(&mut app, &mut events);
    assert!(app.registry().extensions().contains(&types::ExtId("probe")));

    let pane = app.model().focused;
    let ctx = app
        .action_ctx(pane, Invocation::Palette)
        .unwrap_or_else(|| panic!("no context"));
    app.handle(
        Command::Run {
            action: ActionId("probe.wait"),
            ctx,
        },
        &mut events,
    );
    wait_for(&mut app, &mut events, "the job in the table", |app| {
        app.model().jobs.values().any(|j| {
            j.name == "Probe: wait until cancelled" && matches!(j.state, JobState::Running(_))
        })
    });
    let id = app
        .model()
        .jobs
        .iter()
        .find(|(_, j)| j.name == "Probe: wait until cancelled")
        .map_or_else(|| panic!("job gone"), |(id, _)| *id);

    app.handle(Command::CancelJob(id), &mut events);
    wait_for(&mut app, &mut events, "the job cancelled", |app| {
        app.model()
            .jobs
            .get(&id)
            .is_some_and(|j| j.state == JobState::Cancelled)
    });
    // The host outlives the app: the job's code is in the plugin library.
    drop(app);
    drop(host);
}

/// Runs `git` in `dir` for the set-up.
fn git(dir: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .unwrap_or_else(|e| panic!("git: {e}"));
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A repository with one committed file and one new one.
fn repo(dir: &Path) -> PathBuf {
    let work = dir.join("work");
    fs::create_dir(&work).unwrap_or_else(|e| panic!("{e}"));
    fs::write(work.join("tracked.txt"), "one\n").unwrap_or_else(|e| panic!("{e}"));
    git(&work, &["init", "-q", "-b", "main"]);
    git(&work, &["config", "user.name", "Rick Test"]);
    git(&work, &["config", "user.email", "rick@example.com"]);
    git(&work, &["config", "commit.gpgsign", "false"]);
    git(&work, &["add", "-A"]);
    git(&work, &["commit", "-q", "-m", "first commit"]);
    fs::write(work.join("new.txt"), "new\n").unwrap_or_else(|e| panic!("{e}"));
    work
}

#[test]
fn the_git_plugin_brings_the_badges_and_the_git_group_like_the_compiled_in_extension() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let plugins = dir.path().join("plugins");
    install(&plugins, "ext_git", |_| {});
    let host = Host::load(&options(vec![(Source::BesideBinary, plugins)]));
    assert_eq!(
        status_of(&host, "git"),
        &PluginStatus::Loaded,
        "{:?}",
        host.plugins()
    );
    let row = &host.plugins()[0];
    assert_eq!(row.name, "Git");
    assert_eq!(row.points, ext::Points::DECORATOR | ext::Points::ACTION);

    let work = repo(dir.path());
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let ui = Arc::new(EguiHost::new());
    let mut events = Vec::new();
    let mut app = App::new(Config {
        platform: fake.clone(),
        ui,
        extensions: host.extensions(),
        config_dir: Some(fake.config_dir()),
        start: Some(local(&work)),
    });
    settle(&mut app, &mut events);

    // The Git action group, as in 0.1.
    let git_actions: Vec<&str> = app
        .registry()
        .actions()
        .iter()
        .filter(|a| a.group() == ActionGroup::Extension("Git"))
        .map(|a| a.id().0)
        .collect();
    assert!(git_actions.contains(&"git.commit"), "{git_actions:?}");
    assert!(git_actions.contains(&"git.blame"), "{git_actions:?}");

    // The badges: the new file is untracked, the committed one clean.
    let pane = app.model().focused;
    wait_for(&mut app, &mut events, "the Git decorations", |app| {
        app.model().active_tab(pane).is_some_and(|(_, tab)| {
            let new = tab.listing.entries().iter().find(|e| e.name == "new.txt");
            new.is_some_and(|e| {
                tab.decorations.get(&e.id).is_some_and(|d| {
                    d.cells.iter().any(|(c, v)| {
                        *c == ext_git::STATUS_COLUMN
                            && *v == ext::CellValue::Text("Untracked".into())
                    })
                })
            })
        })
    });
    drop(app);
    drop(host);
}

/// Runs frames until the app has nothing in flight.
fn settle_ui(harness: &mut Harness<'_, RickUi>) {
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

/// The plugin rows as the binary hands them to the app.
fn rows(host: &Host) -> Vec<app::Plugin> {
    host.plugins()
        .iter()
        .map(|p| app::Plugin {
            id: p.id.clone(),
            name: p.name.clone(),
            version: p.version.clone(),
            points: p.points,
            previews: p.previews.clone(),
            markers: Vec::new(),
            connections: Vec::new(),
            status: match &p.status {
                PluginStatus::Loaded => app::PluginStatus::Loaded,
                PluginStatus::Disabled => app::PluginStatus::Disabled,
                PluginStatus::Refused(reason) => app::PluginStatus::Refused(reason.clone()),
            },
            path: p.dir.clone(),
            description: p.description.clone(),
            built_in: p.source == Source::BesideBinary,
            removed: false,
        })
        .collect()
}

#[test]
fn the_extensions_dialogue_lists_the_plugins_and_its_switch_writes_the_setting() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let plugins = dir.path().join("plugins");
    install(&plugins, "plugin_probe", |_| {});
    install(&plugins, "plugin_probe", |t| {
        t.id = "probe-api".into();
        t.name = "Old probe".into();
        t.api_version = API_VERSION + 1;
    });
    install(&plugins, "plugin_probe", |t| {
        t.id = "probe-rick".into();
        t.name = "Probe of 0.4".into();
        t.rick = "0.4.1".into();
    });
    let host = Host::load(&options(vec![(Source::BesideBinary, plugins)]));

    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let ui = Arc::new(EguiHost::new());
    let mut app = App::new(Config {
        platform: fake.clone(),
        ui: ui.clone(),
        extensions: host.extensions(),
        config_dir: Some(fake.config_dir()),
        start: Some(local(dir.path())),
    });
    app.set_plugins(rows(&host));
    let pane = app.model().focused;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 720.0))
        .build_eframe(move |cc| RickUi::new(app, ui, &cc.egui_ctx));
    settle_ui(&mut harness);

    // The burger menu's entry is this intent.
    harness.state_mut().queue(Intent::Extensions, pane);
    harness.step();
    harness.step();
    assert!(harness.state().extensions_open());
    let row = |harness: &mut Harness<'_, RickUi>, name: &str| {
        harness
            .get_by_role_and_label(egui::accesskit::Role::RadioButton, name)
            .click();
        for _ in 0..4 {
            harness.step();
        }
    };
    // Both are listed, under what they mainly contribute, with what the
    // manifest says: the probe's description is the test plugin's.
    harness.get_by_label("PREVIEWERS");
    harness.get_by_label("3 installed  ·  3 enabled  ·  2 need attention");
    row(&mut harness, "Probe");
    harness.get_by_label("The test plugin of the plugin host.");
    harness.get_by_label("Built-in plugin");
    harness.get_by_label("Loaded");
    // The one the host refused needs attention: the card gives the reason.
    row(&mut harness, "Old probe");
    let refused = harness
        .get_all_by_label_contains("extension API version")
        .count();
    assert!(refused >= 1, "the host's reason is shown");
    // The mark of each of the two refused rows, and the chip of the one
    // whose details are open.
    assert_eq!(harness.get_all_by_label("Needs attention").count(), 3);
    // So does the one built for another Rick, in the words of the spec
    // (issue 250).
    row(&mut harness, "Probe of 0.4");
    harness.get_by_label("Rick did not load this plugin: Built for Rick 0.4.1.");

    // The switch of the loaded row writes the setting and says that it
    // waits for the next start.
    row(&mut harness, "Probe");
    harness
        .get_all_by_label("Enable Probe")
        .next()
        .unwrap_or_else(|| panic!("the switch"))
        .click();
    for _ in 0..4 {
        harness.step();
    }
    assert_eq!(
        harness.state().app().model().settings.plugins.disabled,
        ["probe"]
    );
    assert_eq!(harness.get_all_by_label("restart").count(), 2);
    drop(harness);
    drop(host);
}

/// The six plugins of milestone 5 (issue 22) load through the host on the
/// platforms their manifests name, with the points their manifests
/// declare, and are refused elsewhere with the reason; loaded into an
/// app, the walk gives the filter bar its toggle and the SVN group is
/// registered.
#[test]
// One scenario over every search and SVN plugin, start to finish.
#[allow(clippy::too_many_lines)]
fn the_search_and_svn_plugins_load_on_their_platforms() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let plugins = dir.path().join("plugins");
    let expected = [
        (
            "ext_search_walk",
            "search-walk",
            Platforms::ALL,
            ext::Points::SEARCH_INDEX,
        ),
        (
            "ext_search_windows",
            "search-windows",
            Platforms::WINDOWS,
            ext::Points::SEARCH_INDEX,
        ),
        (
            "ext_search_spotlight",
            "search-spotlight",
            Platforms::MACOS,
            ext::Points::SEARCH_INDEX,
        ),
        (
            "ext_search_tracker",
            "search-tracker",
            Platforms::LINUX,
            ext::Points::SEARCH_INDEX,
        ),
        (
            "ext_search_baloo",
            "search-baloo",
            Platforms::LINUX,
            ext::Points::SEARCH_INDEX,
        ),
        (
            "ext_svn",
            "svn",
            Platforms::ALL,
            ext::Points::DECORATOR | ext::Points::ACTION,
        ),
    ];
    for (stem, _, _, _) in &expected {
        install(&plugins, stem, |_| {});
    }
    let host = Host::load(&options(vec![(Source::BesideBinary, plugins)]));
    assert_eq!(host.plugins().len(), expected.len(), "{:?}", host.plugins());
    let mut loaded = 0;
    for (_, id, platforms, points) in &expected {
        let row = host
            .plugins()
            .iter()
            .find(|p| p.id == *id)
            .unwrap_or_else(|| panic!("no row for {id}: {:?}", host.plugins()));
        assert_eq!(row.points, *points, "{id}");
        if platforms.contains(Platforms::current()) {
            assert_eq!(row.status, PluginStatus::Loaded, "{id}");
            loaded += 1;
        } else {
            refused_with(&host, id, "built for");
        }
    }
    assert_eq!(host.extensions().len(), loaded);

    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let ui = Arc::new(EguiHost::new());
    let mut events = Vec::new();
    let mut app = App::new(Config {
        platform: fake.clone(),
        ui,
        extensions: host.extensions(),
        config_dir: Some(fake.config_dir()),
        start: Some(local(dir.path())),
    });
    settle(&mut app, &mut events);
    assert_eq!(
        app.subtree_search(&local(dir.path())),
        ext::Availability::Enabled,
        "the walk is registered"
    );
    let svn_actions: Vec<&str> = app
        .registry()
        .actions()
        .iter()
        .filter(|a| a.group() == ActionGroup::Extension("SVN"))
        .map(|a| a.id().0)
        .collect();
    assert_eq!(
        svn_actions,
        [
            "svn.update",
            "svn.commit",
            "svn.revert",
            "svn.log",
            "svn.add"
        ]
    );
    // The platform connectors of this OS are registered; whether their
    // services run decides what the box asks (search.rs).
    let indexes: Vec<&str> = app
        .registry()
        .search_indexes()
        .iter()
        .map(|i| i.id().0)
        .collect();
    assert!(indexes.contains(&"search-walk"), "{indexes:?}");
    assert_eq!(indexes.len(), loaded - 1, "{indexes:?}");
    drop(app);
    drop(host);
}

/// Acceptance (issue 264): after the host has loaded every plugin of a
/// bundle, the process environment is what it was at start, `PATH` and
/// every other variable, and a program started from here is handed no
/// plugin folder on its `PATH`. On every platform; Windows is where the
/// host changed the `PATH`, for a plugin with a library beside its own.
/// The bundle is a plugins directory laid out as the bundler lays one
/// out, with every plugin library built beside this binary, and with a
/// library beside the archive plugin's, as a bundle carries the 7-Zip
/// library there. In a process of its own: the environment and where a
/// process looks for libraries are things it has once.
#[test]
fn loading_every_plugin_leaves_the_environment_as_it_was() {
    if !tests::alone("plugins::loading_every_plugin_leaves_the_environment_as_it_was") {
        return;
    }
    let environment = || std::env::vars_os().collect::<BTreeMap<OsString, OsString>>();
    let at_start = environment();

    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let plugins = dir.path().join("plugins");
    let ids = install_every_plugin(&plugins);
    assert!(ids.iter().any(|id| id == "vfs-archive"), "{ids:?}");
    assert!(ids.iter().any(|id| id == "preview-media"), "{ids:?}");
    fs::write(
        plugins
            .join("vfs-archive")
            .join(platform::sevenzip::LIBRARY_FILE),
        "not a library; the plugin opens the real one by itself, when asked",
    )
    .unwrap_or_else(|e| panic!("{e}"));

    let host = Host::load(&options(vec![(Source::BesideBinary, plugins.clone())]));
    // Every plugin of the bundle that is for this platform is loaded.
    let mut loaded = 0;
    for id in &ids {
        match status_of(&host, id) {
            PluginStatus::Loaded => loaded += 1,
            PluginStatus::Refused(reason) if reason.starts_with("built for ") => {}
            other => panic!("{id}: {other:?}"),
        }
    }
    assert_eq!(status_of(&host, "vfs-archive"), &PluginStatus::Loaded);
    assert!(loaded > 10, "{loaded} of {} plugins loaded", ids.len());

    let after = environment();
    for (name, value) in &after {
        assert_eq!(
            at_start.get(name),
            Some(value),
            "{} changed or appeared",
            name.to_string_lossy()
        );
    }
    assert_eq!(after.len(), at_start.len(), "a variable is gone");

    // What a program started from here is given: the console's shell, a
    // program opened with Enter.
    let (shell, args) = if std::env::consts::OS == "windows" {
        ("cmd", ["/C", "echo %PATH%"])
    } else {
        ("sh", ["-c", "echo \"$PATH\""])
    };
    let output = std::process::Command::new(shell)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("{shell}: {e}"));
    let path = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(output.status.success() && !path.trim().is_empty(), "{path}");
    let plugins = plugins.display().to_string();
    assert!(
        !path.to_lowercase().contains(&plugins.to_lowercase()),
        "a plugin folder is on the PATH of a program started from here: {path}"
    );
    drop(host);
}

#[test]
#[ignore = "timing; run with --ignored in release"]
fn loading_the_git_plugin_adds_at_most_50_ms_to_startup() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let plugins = dir.path().join("plugins");
    install(&plugins, "ext_git", |_| {});
    let options = options(vec![(Source::BesideBinary, plugins)]);
    let mut samples = Vec::new();
    for _ in 0..tests::bench::runs() {
        let started = Instant::now();
        let host = Host::load(&options);
        samples.push(started.elapsed());
        assert_eq!(status_of(&host, "git"), &PluginStatus::Loaded);
        drop(host);
    }
    tests::bench::record("plugin-load", &samples);
}
