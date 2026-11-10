//! Acceptance (issue 139, item 8, SPEC.md *Keyboard shortcuts
//! dialogue*): in Keyboard shortcuts, Git's actions are under *Git* only
//! and the commit row appears once, as *Git commit*; recording a chord
//! that is already taken shows the *also:* chip on both rows.
//!
//! With it: the categories and what they hold, Escape and Backspace while
//! a cell records, the reset button of a changed row, *Reset all to
//! defaults*, and the search field, which narrows the list by a name or
//! by a chord pressed in it.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use ext::ActionId;
use platform::{ConfigDir, Fake};
use tests::local;
use ui_egui::keys::{self, Category, Intent};
use ui_egui::{EguiHost, RickUi};

/// Every plugin that ships with actions.
fn plugins() -> Vec<Arc<dyn ext::Extension>> {
    vec![
        Arc::new(ext_git::GitExtension::new()),
        Arc::new(ext_svn::SvnExtension::new()),
        Arc::new(ext_ssh::SshExtension::new()),
        Arc::new(ext_action_compress::CompressExtension::new()),
        Arc::new(ext_action_compare::CompareExtension::new()),
        Arc::new(ext_action_rename::RenameExtension::new()),
        Arc::new(ext_action_terminal::TerminalExtension::new()),
        Arc::new(ext_installer::InstallerExtension::new()),
        Arc::new(ext_vfs_trash::TrashExtension::new()),
    ]
}

fn harness(start: &Path) -> Harness<'static, RickUi> {
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: plugins(),
        config_dir: Some(fake.config_dir()),
        start: Some(local(start)),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 800.0))
        .with_pixels_per_point(1.0)
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

/// Opens the dialogue from its intent, as the menu and the palette do.
fn open(harness: &mut Harness<'_, RickUi>) {
    let pane = harness.state().app().model().focused;
    harness.state_mut().queue(Intent::Shortcuts, pane);
    steps(harness);
    assert!(harness.state().shortcuts_open());
}

/// Chooses a category in the left column.
fn choose(harness: &mut Harness<'_, RickUi>, category: &str) {
    harness
        .get_by_role_and_label(Role::RadioButton, category)
        .click();
    steps(harness);
}

fn count(harness: &Harness<'_, RickUi>, label: &str) -> usize {
    harness.query_all_by_label(label).count()
}

/// The name of a row's shortcut cell.
fn cell(action: &str, keys: &str) -> String {
    format!("Shortcut for {action}: {keys}")
}

/// Clicks a row's shortcut cell, which records.
fn record(harness: &mut Harness<'_, RickUi>, action: &str, keys: &str) {
    harness.get_by_label(&cell(action, keys)).click();
    steps(harness);
    harness.get_by_label(&cell(action, "Press keys…  Esc to cancel"));
}

fn chord(text: &str) -> ext::Shortcut {
    text.parse().unwrap_or_else(|e| panic!("{text}: {e}"))
}

fn bound(harness: &Harness<'_, RickUi>, id: &str) -> Option<String> {
    harness
        .state()
        .app()
        .model()
        .settings
        .shortcuts
        .get(id)
        .cloned()
}

const GIT: [&str; 9] = [
    "Git commit",
    "Stash",
    "Stash pop",
    "Pull",
    "Push",
    "Checkout branch",
    "Discard changes",
    "Show log",
    "Blame",
];

#[test]
#[allow(clippy::too_many_lines)]
fn git_s_actions_are_under_git_only_and_a_taken_chord_shows_the_chip_on_both_rows() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let mut harness = harness(dir.path());
    let ctrl = platform::MOD_KEY;
    open(&mut harness);
    harness.get_by_label("Click a shortcut to change it. Changes apply at once.");

    // The categories: All, Rick's own, then one per plugin with actions.
    for category in [
        "All",
        "Navigation",
        "File operations",
        "View",
        "Selection",
        "Panes & tabs",
        "Search",
        "Preview pane",
        "Bulk rename",
        "Compare",
        "Compress and extract",
        "Console",
        "Git",
        "SSH",
        "SVN",
        "Trash",
    ] {
        harness.get_by_role_and_label(Role::RadioButton, category);
    }

    // Under All every action is there once: one commit row per plugin,
    // each under its own name, and no bare "Commit".
    assert_eq!(count(&harness, "Git commit"), 1);
    assert_eq!(count(&harness, "SVN commit"), 1);
    assert_eq!(count(&harness, "Commit"), 0);
    harness.get_by_label("FILE OPERATIONS");
    harness.get_by_label("GIT · PLUGIN");

    // File operations holds the kernel's actions and no plugin's.
    choose(&mut harness, "File operations");
    harness.get_by_label("Copy");
    harness.get_by_label("Undo");
    assert_eq!(count(&harness, "GIT · PLUGIN"), 0);
    for action in GIT
        .iter()
        .chain(&["SVN commit", "Bulk rename…", "Compress…"])
    {
        assert_eq!(count(&harness, action), 0, "{action} under File operations");
    }

    // Git holds its nine, and nothing else.
    choose(&mut harness, "Git");
    harness.get_by_label("GIT · PLUGIN");
    for action in GIT {
        assert_eq!(count(&harness, action), 1, "{action} under Git");
    }
    assert_eq!(count(&harness, "Copy"), 0);
    assert_eq!(count(&harness, "SVN commit"), 0);

    // Recording: the cell says what to press, Escape leaves the binding
    // and the dialogue as they were.
    record(&mut harness, "Git commit", "Not set");
    harness.key_press(egui::Key::Escape);
    steps(&mut harness);
    assert!(
        harness.state().shortcuts_open(),
        "Escape ended the recording only"
    );
    harness.get_by_label(&cell("Git commit", "Not set"));
    assert_eq!(bound(&harness, "git.commit"), None);

    // A chord that Copy has: it is taken, and both rows say so.
    record(&mut harness, "Git commit", "Not set");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::C);
    steps(&mut harness);
    assert_eq!(bound(&harness, "git.commit").as_deref(), Some("Mod+C"));
    harness.get_by_label(&cell("Git commit", &format!("{ctrl}+C")));
    harness.get_by_label("also: Copy");
    choose(&mut harness, "All");
    harness.get_by_label("also: Copy");
    harness.get_by_label("also: Git commit");
    // While it stands the chord runs Rick's own action.
    assert_eq!(
        harness.state().keys().lookup(chord("Mod+C")),
        Some(Intent::Action(ActionId("core.copy")))
    );

    // The changed row's reset button is live and names the default; the
    // row at its default has an inert one.
    let copy_reset = format!("Reset Copy to default: {ctrl}+C");
    assert!(
        harness
            .get_by_label(&copy_reset)
            .accesskit_node()
            .is_disabled()
    );
    choose(&mut harness, "Git");
    let reset = harness.get_by_label("Reset Git commit to default: Not set");
    assert!(!reset.accesskit_node().is_disabled());
    reset.click();
    steps(&mut harness);
    assert_eq!(bound(&harness, "git.commit"), None);
    harness.get_by_label(&cell("Git commit", "Not set"));
    choose(&mut harness, "All");
    assert_eq!(count(&harness, "also: Copy"), 0);
    assert_eq!(count(&harness, "also: Git commit"), 0);

    // Backspace while recording unbinds; the row then differs from its
    // default, and Reset all to defaults takes every change back.
    choose(&mut harness, "File operations");
    record(&mut harness, "Copy", &format!("{ctrl}+C"));
    harness.key_press(egui::Key::Backspace);
    steps(&mut harness);
    assert_eq!(bound(&harness, "core.copy").as_deref(), Some(""));
    harness.get_by_label(&cell("Copy", "Not set"));
    choose(&mut harness, "Git");
    record(&mut harness, "Blame", "Not set");
    harness.key_press(egui::Key::F9);
    steps(&mut harness);
    assert_eq!(bound(&harness, "git.blame").as_deref(), Some("F9"));
    harness.get_by_label("Reset all to defaults").click();
    steps(&mut harness);
    assert_eq!(
        harness.state().app().model().settings.shortcuts,
        std::collections::BTreeMap::new(),
        "every change is taken back"
    );
    harness.get_by_label(&cell("Blame", "Not set"));
    choose(&mut harness, "File operations");
    harness.get_by_label(&cell("Copy", &format!("{ctrl}+C")));

    // Close: the header's round button and the footer's primary.
    let closes: Vec<_> = harness.get_all_by_label("Close").collect();
    assert_eq!(closes.len(), 2);
    closes
        .into_iter()
        .max_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
        .unwrap_or_else(|| panic!("the footer's Close"))
        .click();
    steps(&mut harness);
    assert!(!harness.state().shortcuts_open());
}

#[test]
fn the_search_field_narrows_by_a_name_or_by_a_pressed_chord() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let mut harness = harness(dir.path());
    open(&mut harness);

    // The field has the focus as the dialogue opens: typing narrows the
    // list to the actions whose name contains the text.
    let field = harness
        .get_all_by_role(Role::TextInput)
        .find(egui_kittest::Node::is_focused)
        .unwrap_or_else(|| panic!("the search field has the focus"));
    field.type_text("commit");
    steps(&mut harness);
    assert_eq!(count(&harness, "Git commit"), 1);
    assert_eq!(count(&harness, "SVN commit"), 1);
    assert_eq!(count(&harness, "Copy"), 0);
    assert_eq!(count(&harness, "Stash"), 0);

    // A chord pressed in the field replaces the text and narrows the
    // list to what it is bound to: Mod+F2 is Bulk rename.
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::F2);
    steps(&mut harness);
    assert_eq!(count(&harness, "Bulk rename…"), 1);
    assert_eq!(count(&harness, "Git commit"), 0);
    assert_eq!(count(&harness, "Rename"), 0);
    // The clipboard chords arrive as copy, cut and paste events.
    harness.event(egui::Event::Copy);
    steps(&mut harness);
    assert_eq!(count(&harness, "Copy"), 1);
    assert_eq!(count(&harness, "Bulk rename…"), 0);
    // A function key is a chord without a modifier.
    harness.key_press(egui::Key::F5);
    steps(&mut harness);
    assert_eq!(count(&harness, "Refresh"), 1);
    assert_eq!(count(&harness, "Copy"), 0);
    // A chord nothing is bound to leaves no row.
    harness.key_press_modifiers(egui::Modifiers::ALT, egui::Key::F12);
    steps(&mut harness);
    harness.get_by_label("No action matches");

    // Backspace takes the chord away, and the list is whole again.
    harness.key_press(egui::Key::Backspace);
    steps(&mut harness);
    assert_eq!(count(&harness, "Copy"), 1);
    assert_eq!(count(&harness, "Git commit"), 1);
    assert!(harness.state().shortcuts_open());
}

#[test]
fn every_action_is_listed_once_under_one_name_and_a_plugin_s_under_the_plugin() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let harness = harness(dir.path());
    let rows = keys::rows_of(harness.state().app());

    // No intent twice, and no two rows of the list alike.
    let mut names: Vec<&str> = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        assert!(
            rows[..index].iter().all(|other| other.intent != row.intent),
            "{:?} is listed twice",
            row.intent
        );
        if row.category != Category::List {
            assert!(
                !names.contains(&row.label.as_str()),
                "two rows named {}",
                row.label
            );
            names.push(&row.label);
        }
    }

    // At the defaults no chord is bound twice, so no row carries a chip.
    for row in &rows {
        let others = harness.state().keys().conflicts(row.intent);
        assert_eq!(others, Vec::new(), "{} at its default", row.label);
    }

    // A registered action is the kernel's, under File operations, or a
    // plugin's, under the plugin that registered it.
    let category = |id: &str| {
        rows.iter()
            .find(|row| row.intent.id() == id)
            .map_or_else(|| panic!("{id} is not listed"), |row| row.category)
    };
    assert_eq!(category("core.copy"), Category::FileOperations);
    assert_eq!(category("ops.undo"), Category::FileOperations);
    for (id, plugin) in [
        ("git.commit", "Git"),
        ("git.blame", "Git"),
        ("svn.commit", "SVN"),
        ("rename.bulk", "Bulk rename"),
        ("compress.compress", "Compress and extract"),
    ] {
        match category(id) {
            Category::Plugin { name, .. } => assert_eq!(name, plugin, "{id}"),
            other => panic!("{id} is filed under {other:?}"),
        }
    }
    // The window commands are among Panes & tabs, and the search commands
    // under Search.
    assert_eq!(category("window.new"), Category::Panes);
    assert_eq!(category("window.side_panel"), Category::Panes);
    assert_eq!(category("nav.palette"), Category::Search);
    assert_eq!(category("nav.quick_open"), Category::Search);
    assert_eq!(category("view.filter"), Category::Search);
    assert_eq!(category("view.subtree"), Category::Search);
    assert_eq!(category("preview.play"), Category::Preview);

    // Rick's own rows come before every plugin's, the plugins by name.
    let first_plugin = rows
        .iter()
        .position(|row| matches!(row.category, Category::Plugin { .. }))
        .unwrap_or_else(|| panic!("no plugin row"));
    assert!(
        rows[first_plugin..]
            .iter()
            .all(|row| matches!(row.category, Category::Plugin { .. }))
    );
    let mut plugins: Vec<&str> = Vec::new();
    for row in &rows[first_plugin..] {
        if let Category::Plugin { name, .. } = row.category
            && plugins.last() != Some(&name)
        {
            plugins.push(name);
        }
    }
    let mut sorted = plugins.clone();
    sorted.sort_by_key(|name| name.to_lowercase());
    sorted.dedup();
    assert_eq!(plugins, sorted);
}
