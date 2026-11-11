//! Acceptance (issue 139, item 9, SPEC.md *Extensions dialogue*): the
//! Extensions dialogue as master and detail.
//!
//! The list groups the plugins by what they contribute, each row with its
//! enable switch and an amber dot when it needs attention; the details
//! show the chosen plugin's id, version and kind, the *Built-in plugin*
//! or *User plugin* chip, the manifest's description, the attention card
//! and the info card whose rows copy. *Settings* opens the plugin's
//! settings page, *Open folder* shows its folder, *Remove* is there for a
//! user's plugin only, and *Install from file…* unpacks a package into
//! the per-user plugins folder. Switching, installing and removing wait
//! for the next start and say so with a *restart* chip.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use app::{Plugin, PluginStatus};
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use ext::Points;
use platform::{ConfigDir, Fake};
use tests::local;
use ui_egui::keys::Intent;
use ui_egui::{EguiHost, RickUi};

/// A plugin row as the binary would hand it over.
fn plugin(id: &str, name: &str, points: Points, dir: &Path) -> Plugin {
    Plugin {
        id: id.to_owned(),
        name: name.to_owned(),
        description: String::new(),
        version: "0.4.0-dev".to_owned(),
        points,
        previews: Vec::new(),
        status: PluginStatus::Loaded,
        path: dir.join(id),
        built_in: true,
        removed: false,
    }
}

/// The plugins of the test: four that are loaded for real, so that what
/// they register is known, one switched off, one the host refused and one
/// of the user's.
fn plugins(beside: &Path, per_user: &Path) -> Vec<Plugin> {
    let user = per_user.join("hello");
    fs::create_dir_all(&user).unwrap_or_else(|e| panic!("{e}"));
    fs::write(user.join("plugin.toml"), "id = \"hello\"\n").unwrap_or_else(|e| panic!("{e}"));
    vec![
        Plugin {
            description: "Shows Git status badges and adds the Git actions.".to_owned(),
            ..plugin("git", "Git", Points::DECORATOR | Points::ACTION, beside)
        },
        Plugin {
            description: "Browse Azure Blob Storage accounts as folders.".to_owned(),
            ..plugin("vfs-azure", "Azure Blob", Points::PROVIDER, beside)
        },
        plugin("ssh", "SSH", Points::PROVIDER | Points::ACTION, beside),
        Plugin {
            previews: vec!["pdf".to_owned()],
            ..plugin("preview-pdf", "PDF preview", Points::PREVIEWER, beside)
        },
        Plugin {
            status: PluginStatus::Disabled,
            previews: vec!["docx".to_owned(), "odt".to_owned()],
            ..plugin(
                "preview-office",
                "Office preview",
                Points::PREVIEWER,
                beside,
            )
        },
        Plugin {
            status: PluginStatus::Refused("built with another toolchain".to_owned()),
            ..plugin("search-tracker", "Tracker", Points::SEARCH_INDEX, beside)
        },
        Plugin {
            built_in: false,
            version: "1.0.0".to_owned(),
            ..plugin("hello", "Hello", Points::ACTION, per_user)
        },
    ]
}

struct Setup {
    harness: Harness<'static, RickUi>,
    fake: Arc<Fake>,
    per_user: PathBuf,
    /// The folder the test's own files go to; it lives as long as the
    /// window.
    scratch: tempfile::TempDir,
}

fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let beside = dir.path().join("opt/rick/plugins");
    let per_user = fake.config_dir().join("plugins");
    let rows = plugins(&beside, &per_user);
    let disabled = model::SettingsChange::Plugin {
        id: "preview-office".to_owned(),
        enabled: false,
    };
    let mut app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: vec![
            Arc::new(ext_git::GitExtension::new()),
            Arc::new(ext_vfs_azure::AzureExtension::new()),
            Arc::new(ext_ssh::SshExtension::new()),
            Arc::new(ext_preview_pdf::PdfPreview::new()),
        ],
        config_dir: Some(fake.config_dir()),
        start: Some(local(dir.path())),
    });
    app.set_plugins(rows);
    app.handle(app::Command::Settings(disabled), &mut Vec::new());
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 820.0))
        .with_pixels_per_point(1.0)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    let pane = harness.state().app().model().focused;
    harness.state_mut().queue(Intent::Extensions, pane);
    steps(&mut harness);
    assert!(harness.state().extensions_open());
    Setup {
        harness,
        fake,
        per_user,
        scratch: dir,
    }
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

fn steps(harness: &mut Harness<'_, RickUi>) {
    for _ in 0..4 {
        harness.step();
    }
}

fn step_until(
    harness: &mut Harness<'_, RickUi>,
    what: &str,
    mut done: impl FnMut(&Harness<'_, RickUi>) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !done(harness) {
        harness.step();
        assert!(Instant::now() < deadline, "{what}");
        std::thread::sleep(Duration::from_millis(5));
    }
    steps(harness);
}

/// Chooses a plugin's row in the list.
fn choose(harness: &mut Harness<'_, RickUi>, name: &str) {
    harness
        .get_by_role_and_label(Role::RadioButton, name)
        .click();
    steps(harness);
}

fn click(harness: &mut Harness<'_, RickUi>, label: &str) {
    harness.get_by_label(label).click();
    steps(harness);
}

fn count(harness: &Harness<'_, RickUi>, label: &str) -> usize {
    harness.query_all_by_label(label).count()
}

fn top(harness: &Harness<'_, RickUi>, label: &str) -> f32 {
    harness.get_by_label(label).rect().top()
}

fn disabled(harness: &Harness<'_, RickUi>) -> Vec<String> {
    harness
        .state()
        .app()
        .model()
        .settings
        .plugins
        .disabled
        .clone()
}

#[test]
#[allow(clippy::too_many_lines)]
fn the_list_groups_the_plugins_and_the_details_show_the_chosen_one() {
    let Setup {
        mut harness,
        fake,
        per_user,
        scratch: _scratch,
    } = setup();

    // The header counts.
    harness.get_by_label("7 installed  ·  6 enabled  ·  1 needs attention");

    // Grouped by what a plugin mainly contributes, in the list's order:
    // SSH is a provider though it has actions, Git an action though it
    // decorates.
    let order = [
        "PROVIDERS",
        "Azure Blob",
        "SSH",
        "PREVIEWERS",
        "Office preview",
        "PDF preview",
        "ACTIONS",
        "Git",
        "Hello",
        "SEARCH",
        "Tracker",
    ];
    let tops: Vec<f32> = order
        .iter()
        .map(|label| {
            harness
                .get_all_by_label(label)
                .map(|node| node.rect())
                .filter(|rect| rect.left() < 600.0)
                .map(|rect| rect.top())
                .fold(f32::INFINITY, f32::min)
        })
        .collect();
    assert!(tops.windows(2).all(|pair| pair[0] < pair[1]), "{tops:?}");

    // Every row has its switch; the one switched off is off.
    for name in [
        "Azure Blob",
        "SSH",
        "PDF preview",
        "Git",
        "Hello",
        "Tracker",
    ] {
        let label = format!("Enable {name}");
        let switch = harness
            .get_all_by_label(&label)
            .next()
            .unwrap_or_else(|| panic!("the switch of {name}"));
        assert_eq!(
            switch.accesskit_node().toggled(),
            Some(egui::accesskit::Toggled::True),
            "{name}"
        );
    }
    let office = harness.get_by_label("Enable Office preview");
    assert_eq!(
        office.accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::False)
    );

    // The first of the list is chosen: its details.
    harness.get_by_label("vfs-azure  ·  0.4.0-dev  ·  Provider");
    harness.get_by_label("Built-in plugin");
    harness.get_by_label("Browse Azure Blob Storage accounts as folders.");
    harness.get_by_label("Provider  ·  azure://");
    harness.get_by_label("Loaded");
    assert_eq!(count(&harness, "Needs attention"), 1, "the dot of Tracker");
    // A built-in plugin can be switched off, not removed; Azure has no
    // settings page.
    assert_eq!(count(&harness, "Remove"), 0);
    assert_eq!(count(&harness, "Settings"), 0);
    assert_eq!(count(&harness, "Config"), 0);

    // Every row of the info card copies; a path is copied whole.
    click(&mut harness, "Copy Location");
    let location = fake.clipboard_text().unwrap_or_default();
    assert!(location.ends_with("vfs-azure"), "{location}");
    click(&mut harness, "Copy Contributes");
    assert_eq!(
        fake.clipboard_text().as_deref(),
        Some("Provider  ·  azure://")
    );

    // A plugin with several points says so; SSH's provider serves ssh://.
    choose(&mut harness, "SSH");
    harness.get_by_label("ssh  ·  0.4.0-dev  ·  Provider");
    harness.get_by_label("Provider  ·  ssh://; Action");
    choose(&mut harness, "Git");
    harness.get_by_label("git  ·  0.4.0-dev  ·  Action");
    harness.get_by_label("Action; Decorator");

    // The refused plugin needs attention: the card gives the host's
    // reason, and Status says what stopped it.
    choose(&mut harness, "Tracker");
    assert_eq!(
        count(&harness, "Needs attention"),
        2,
        "the dot and the card"
    );
    harness.get_by_label("Rick did not load this plugin: built with another toolchain.");
    harness.get_by_label("built with another toolchain");

    // The switch writes the setting; until the next start the row and the
    // details say restart.
    assert_eq!(count(&harness, "restart"), 0);
    choose(&mut harness, "Git");
    let switches: Vec<_> = harness.get_all_by_label("Enable Git").collect();
    assert_eq!(switches.len(), 2, "in the row and in the details");
    switches[0].click();
    steps(&mut harness);
    assert_eq!(disabled(&harness), ["preview-office", "git"]);
    assert_eq!(count(&harness, "restart"), 2);
    harness.get_by_label("7 installed  ·  5 enabled  ·  1 needs attention");
    // Switched back, nothing waits.
    harness
        .get_all_by_label("Enable Git")
        .next()
        .unwrap_or_else(|| panic!("the switch"))
        .click();
    steps(&mut harness);
    assert_eq!(disabled(&harness), ["preview-office"]);
    assert_eq!(count(&harness, "restart"), 0);

    // The user's plugin: its chip, and Open folder shows its folder.
    choose(&mut harness, "Hello");
    harness.get_by_label("hello  ·  1.0.0  ·  Action");
    harness.get_by_label("User plugin");
    click(&mut harness, "Open folder");
    assert_eq!(fake.opened(), [per_user.join("hello")]);

    // The search narrows the list by name or id.
    let field = harness
        .get_all_by_role(Role::TextInput)
        .find(|field| (100.0..260.0).contains(&field.rect().top()) && field.rect().left() < 600.0)
        .unwrap_or_else(|| panic!("the search field"));
    field.click();
    steps(&mut harness);
    let field = harness
        .get_all_by_role(Role::TextInput)
        .find(egui_kittest::Node::is_focused)
        .unwrap_or_else(|| panic!("the search field has the focus"));
    field.type_text("pdf");
    steps(&mut harness);
    assert!(
        harness
            .query_by_role_and_label(Role::RadioButton, "Git")
            .is_none()
    );
    harness.get_by_role_and_label(Role::RadioButton, "PDF preview");
    harness.get_by_label("preview-pdf  ·  0.4.0-dev  ·  Previewer");
    harness.get_by_label("Previewer  ·  pdf");
    // A plugin with settings has the Config row and the Settings button,
    // which opens the settings window on its page.
    harness.get_by_label("Config");
    click(&mut harness, "Copy Config");
    let config = fake.clipboard_text().unwrap_or_default();
    assert!(config.ends_with("preview-pdf.toml"), "{config}");
    click(&mut harness, "Settings");
    assert!(harness.state().settings_open());
    assert!(!harness.state().extensions_open());
}

/// Writes a plugin package: a zip archive of a plugin's folder.
fn package(path: &Path) {
    let file = fs::File::create(path).unwrap_or_else(|e| panic!("{e}"));
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("fresh/plugin.toml", options)
        .unwrap_or_else(|e| panic!("{e}"));
    zip.write_all(
        b"id = \"fresh\"\nname = \"Fresh\"\ndescription = \"Just unpacked.\"\n\
          version = \"2.1.0\"\npoints = [\"previewer\"]\npreviews = [\"fr\"]\n",
    )
    .unwrap_or_else(|e| panic!("{e}"));
    zip.start_file("fresh/libfresh.so", options)
        .unwrap_or_else(|e| panic!("{e}"));
    zip.write_all(b"library").unwrap_or_else(|e| panic!("{e}"));
    zip.finish().unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn install_from_file_unpacks_a_package_and_remove_deletes_a_user_s_plugin() {
    let Setup {
        mut harness,
        fake,
        per_user,
        scratch,
    } = setup();
    let rkp = scratch.path().join("fresh-2.1.0.rkp");
    package(&rkp);

    // Install from file: the package the file dialogue gives is unpacked
    // into the per-user plugins folder, asking nothing; its row is chosen
    // and waits for the next start.
    fake.answer_files(vec![rkp]);
    click(&mut harness, "Install from file…");
    step_until(&mut harness, "the package was not installed", |h| {
        h.state()
            .app()
            .plugins()
            .iter()
            .any(|plugin| plugin.id == "fresh")
    });
    assert!(per_user.join("fresh/plugin.toml").is_file());
    assert!(per_user.join("fresh/libfresh.so").is_file());
    harness.get_by_role_and_label(Role::RadioButton, "Fresh");
    harness.get_by_label("fresh  ·  2.1.0  ·  Previewer");
    harness.get_by_label("User plugin");
    harness.get_by_label("Just unpacked.");
    harness.get_by_label("Previewer  ·  fr");
    harness.get_by_label("Installed; loads at the next start");
    assert_eq!(count(&harness, "restart"), 2, "the row and the details");
    harness.get_by_label("8 installed  ·  7 enabled  ·  1 needs attention");

    // Remove deletes its folder at once, without a question; a plugin
    // that was never loaded leaves no row.
    click(&mut harness, "Remove");
    step_until(&mut harness, "the plugin was not removed", |h| {
        h.state()
            .app()
            .plugins()
            .iter()
            .all(|plugin| plugin.id != "fresh")
    });
    assert!(!per_user.join("fresh").exists());
    assert!(
        harness
            .query_by_role_and_label(Role::RadioButton, "Fresh")
            .is_none()
    );

    // A user's plugin that is loaded stays listed until the next start:
    // its folder is gone, its row says so and has nothing left to remove.
    choose(&mut harness, "Hello");
    assert!(per_user.join("hello").is_dir());
    click(&mut harness, "Remove");
    step_until(&mut harness, "the plugin was not removed", |h| {
        h.state()
            .app()
            .plugins()
            .iter()
            .any(|plugin| plugin.id == "hello" && plugin.removed)
    });
    assert!(!per_user.join("hello").exists());
    assert!(per_user.is_dir(), "the plugins folder itself stays");
    harness.get_by_label("Removed; gone at the next start");
    assert_eq!(count(&harness, "restart"), 2);
    assert_eq!(count(&harness, "Remove"), 0);
    assert_eq!(count(&harness, "Open folder"), 0);
    assert_eq!(count(&harness, "Enable Hello"), 0);
    harness.get_by_label("6 installed  ·  5 enabled  ·  1 needs attention");
    assert!(top(&harness, "Removed; gone at the next start") > 0.0);

    // A built-in plugin has no Remove.
    choose(&mut harness, "Git");
    assert_eq!(count(&harness, "Remove"), 0);
}

#[test]
fn a_plugin_whose_library_was_reported_missing_needs_attention() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("paper.pdf"), tests::media::pdf(2, 300, 400))
        .unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    // The previewer looks in its setting alone: a machine without pdfium.
    let pdf = ext_preview_pdf::PdfPreview::with_search(ext_preview_pdf::Search::SettingOnly);
    let mut app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: vec![Arc::new(pdf)],
        config_dir: Some(fake.config_dir()),
        start: Some(local(dir.path())),
    });
    app.set_plugins(vec![Plugin {
        previews: vec!["pdf".to_owned()],
        ..plugin("preview-pdf", "PDF preview", Points::PREVIEWER, dir.path())
    }]);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 820.0))
        .with_pixels_per_point(1.0)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    let pane = harness.state().app().model().focused;
    let open = |harness: &mut Harness<'_, RickUi>| {
        harness.state_mut().queue(Intent::Extensions, pane);
        steps(harness);
    };

    // Nothing was previewed yet: the plugin is in order.
    open(&mut harness);
    harness.get_by_label("1 installed  ·  1 enabled  ·  0 need attention");
    assert_eq!(count(&harness, "Needs attention"), 0);
    harness.key_press(egui::Key::Escape);
    steps(&mut harness);
    assert!(!harness.state().extensions_open());

    // A PDF in the preview pane: the previewer answers with the
    // missing-library card, and the plugin needs attention from then on.
    harness.state_mut().queue(Intent::ToggleInspector, pane);
    harness.step();
    let id = harness
        .state()
        .app()
        .model()
        .active_tab(pane)
        .and_then(|(_, tab)| {
            tab.listing
                .rows()
                .find(|e| e.name == "paper.pdf")
                .map(|e| e.id)
        })
        .unwrap_or_else(|| panic!("paper.pdf is listed"));
    harness.state_mut().app_mut().handle(
        app::Command::Select {
            pane,
            change: model::SelectionChange::Set(vec![id]),
        },
        &mut Vec::new(),
    );
    step_until(&mut harness, "the library was not reported missing", |h| {
        h.state().app().missing_library("preview-pdf").is_some()
    });
    assert_eq!(
        harness.state().app().missing_library("preview-pdf"),
        Some("pdfium")
    );
    open(&mut harness);
    harness.get_by_label("1 installed  ·  1 enabled  ·  1 needs attention");
    assert_eq!(
        count(&harness, "Needs attention"),
        2,
        "the dot and the card"
    );
    assert!(
        harness
            .query_all_by_label_contains("pdfium was not found")
            .next()
            .is_some(),
        "the card names the library"
    );
}

#[test]
fn a_file_that_is_no_package_is_reported_and_changes_nothing() {
    let Setup {
        mut harness,
        fake,
        per_user,
        scratch,
    } = setup();
    let bogus = scratch.path().join("bogus.rkp");
    fs::write(&bogus, "not a zip archive").unwrap_or_else(|e| panic!("{e}"));
    fake.answer_files(vec![bogus]);
    click(&mut harness, "Install from file…");
    settle(&mut harness);
    steps(&mut harness);
    assert!(
        harness
            .query_all_by_label_contains("is not a plugin package")
            .next()
            .is_some(),
        "the error is shown"
    );
    assert_eq!(harness.state().app().plugins().len(), 7);
    assert!(!per_user.join("bogus").exists());
    assert_eq!(
        fs::read_dir(&per_user).map(Iterator::count).ok(),
        Some(1),
        "only the plugin that was there"
    );
}
