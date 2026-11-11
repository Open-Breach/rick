//! Acceptance (issue 139, item 4, SPEC.md *Dialogues*): the Properties
//! dialogue. Properties of a symbolic link shows its target with a working
//! copy button, and *Open target folder* goes there and selects the
//! target; a broken link reads *Missing* and cannot go anywhere. A
//! folder's contents are counted in the background and the header follows.
//! On the Permissions tab *Apply* and *Revert* do nothing until something
//! differs, a ticked box is applied to the file and read back, and the
//! access control list opens in place, moving with the Access grid.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use app::Contents;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use model::SelectionChange;
use platform::{Acl, ConfigDir, Fake, PosixTag};
use tests::local;
use ui_egui::{EguiHost, RickUi};

fn harness(start: &Path) -> (Harness<'static, RickUi>, Arc<Fake>) {
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
        .with_size(egui::vec2(1400.0, 1000.0))
        .with_step_dt(0.05)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    (harness, fake)
}

/// Runs frames until the app has nothing in flight and a quiet moment
/// passed.
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

/// Runs frames until `done` holds.
fn step_until(
    harness: &mut Harness<'_, RickUi>,
    what: &str,
    mut done: impl FnMut(&Harness<'_, RickUi>) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done(harness) {
        harness.step();
        assert!(Instant::now() < deadline, "{what}");
        std::thread::sleep(Duration::from_millis(10));
    }
    harness.step();
}

fn command(harness: &mut Harness<'_, RickUi>, command: app::Command) {
    harness
        .state_mut()
        .app_mut()
        .handle(command, &mut Vec::new());
    harness.step();
}

/// Selects the row named `name` in the focused pane and opens Properties
/// on it.
fn open_properties(harness: &mut Harness<'_, RickUi>, name: &str) {
    let pane = harness.state().app().model().focused;
    let id = harness
        .state()
        .app()
        .model()
        .active_tab(pane)
        .and_then(|(_, tab)| tab.rows().find(|e| e.name == name).map(|e| e.id))
        .unwrap_or_else(|| panic!("{name} is listed"));
    command(
        harness,
        app::Command::Select {
            pane,
            change: SelectionChange::Set(vec![id]),
        },
    );
    command(harness, app::Command::ShowProperties(pane));
    step_until(harness, "the dialogue did not open", |h| {
        h.state()
            .properties()
            .info()
            .is_some_and(|info| info.entry.name == name)
    });
    // A modal takes a frame to find its size and another its place.
    for _ in 0..3 {
        harness.step();
    }
}

/// Clicks the control named `label`: the press, the release and the frame
/// that shows what they did.
fn click(harness: &mut Harness<'_, RickUi>, label: &str) {
    harness.get_by_label(label).click();
    for _ in 0..4 {
        harness.step();
    }
}

/// Presses Escape.
fn escape(harness: &mut Harness<'_, RickUi>) {
    harness.key_press(egui::Key::Escape);
    for _ in 0..4 {
        harness.step();
    }
}

fn chmod(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap_or_else(|e| panic!("{e}"));
}

fn mode_of(path: &Path) -> u32 {
    fs::metadata(path)
        .unwrap_or_else(|e| panic!("{e}"))
        .permissions()
        .mode()
        & 0o7777
}

/// Where the focused pane is, and the names selected there.
fn shown(harness: &Harness<'_, RickUi>) -> (types::VPath, Vec<String>) {
    let model = harness.state().app().model();
    let (_, tab) = model
        .active_tab(model.focused)
        .unwrap_or_else(|| panic!("a tab"));
    (
        tab.location.path.clone(),
        tab.selected().map(|e| e.name.clone()).collect(),
    )
}

#[test]
fn a_link_shows_its_target_copies_it_and_opens_the_target_s_folder() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let bin = dir.path().join("bin");
    let links = dir.path().join("links");
    fs::create_dir(&bin).unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir(&links).unwrap_or_else(|e| panic!("{e}"));
    fs::write(bin.join("tool"), b"#!x").unwrap_or_else(|e| panic!("{e}"));
    fs::write(bin.join("other"), b"x").unwrap_or_else(|e| panic!("{e}"));
    chmod(&bin.join("tool"), 0o755);
    std::os::unix::fs::symlink("../bin/tool", links.join("tool-link"))
        .unwrap_or_else(|e| panic!("{e}"));
    let (mut harness, fake) = harness(&links);
    open_properties(&mut harness, "tool-link");

    // The header: the type and the link's own size; then what the link
    // holds and what it resolves to.
    harness.get_by_label("Symbolic link  ·  11 B");
    harness.get_by_label("../bin/tool");
    harness.get_by_label("Executable file  ·  3 B");
    assert!(
        harness.query_by_label_contains("11 B (on disk ").is_some(),
        "the Link group's one row"
    );
    assert!(
        harness.query_by_label("Hidden").is_none(),
        "a link has no Options"
    );

    // Each path row's copy button puts its path on the clipboard.
    click(&mut harness, "Copy target");
    assert_eq!(fake.clipboard_text().as_deref(), Some("../bin/tool"));
    click(&mut harness, "Copy location");
    assert_eq!(
        fake.clipboard_text(),
        Some(links.display().to_string()),
        "the folder that holds the link"
    );

    // Open target folder: the pane goes to the folder that holds the
    // target, selects the target there, and the dialogue is gone.
    click(&mut harness, "Open target folder");
    step_until(&mut harness, "the target was not selected", |h| {
        shown(h) == (platform::paths::from_path(&bin), vec!["tool".to_owned()])
    });
    assert!(!harness.state().properties().is_open());
}

#[test]
fn a_broken_link_reads_missing_and_goes_nowhere() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    std::os::unix::fs::symlink("nowhere/at-all", dir.path().join("gone"))
        .unwrap_or_else(|e| panic!("{e}"));
    let (mut harness, _fake) = harness(dir.path());
    open_properties(&mut harness, "gone");
    harness.get_by_label("nowhere/at-all");
    harness.get_by_label("Missing");
    let before = shown(&harness);
    click(&mut harness, "Open target folder");
    settle(&mut harness);
    assert!(
        harness.state().properties().is_open(),
        "the button is off for a broken link"
    );
    assert_eq!(shown(&harness), before);
    // Escape closes the dialogue.
    escape(&mut harness);
    assert!(!harness.state().properties().is_open());
}

#[test]
fn a_folder_s_contents_are_counted_in_the_background() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let stuff = dir.path().join("stuff");
    fs::create_dir_all(stuff.join("sub")).unwrap_or_else(|e| panic!("{e}"));
    fs::write(stuff.join("a.bin"), vec![0u8; 1000]).unwrap_or_else(|e| panic!("{e}"));
    fs::write(stuff.join("b.bin"), vec![0u8; 2000]).unwrap_or_else(|e| panic!("{e}"));
    fs::write(stuff.join("sub/c.bin"), vec![0u8; 500]).unwrap_or_else(|e| panic!("{e}"));
    let (mut harness, fake) = harness(dir.path());
    open_properties(&mut harness, "stuff");
    step_until(&mut harness, "the count did not arrive", |h| {
        h.state()
            .properties()
            .info()
            .is_some_and(|info| matches!(info.contents, Contents::Counted(_)))
    });
    // Two files, the folder inside and its file: 3500 bytes.
    harness.get_by_label("Folder  ·  4 items, 3.4 KiB");
    harness.get_by_label("4 items  ·  3.4 KiB");
    harness.get_by_label("On disk");
    // Options: whether it is hidden, shown and not changed here.
    harness.get_by_label("Hidden");
    click(&mut harness, "Copy location");
    assert_eq!(
        fake.clipboard_text(),
        Some(dir.path().display().to_string())
    );
    // The header's close button closes; the count is the application's to
    // drop.
    escape(&mut harness);
    assert!(!harness.state().properties().is_open());
}

#[test]
fn apply_and_revert_wait_for_a_change_and_a_ticked_box_reaches_the_file() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let file = dir.path().join("notes.txt");
    fs::write(&file, b"hello").unwrap_or_else(|e| panic!("{e}"));
    chmod(&file, 0o640);
    let (mut harness, _fake) = harness(dir.path());
    open_properties(&mut harness, "notes.txt");
    click(&mut harness, "Permissions");
    harness.get_by_label("setuid · setgid · sticky: off");

    // Nothing differs: Apply and Revert do nothing.
    let pending = |h: &Harness<'_, RickUi>| h.state().properties().pending_change();
    assert!(pending(&harness).is_none());
    click(&mut harness, "Apply");
    settle(&mut harness);
    assert_eq!(mode_of(&file), 0o640);
    assert!(!harness.state().properties().applying);

    // A ticked box is a pending change; Revert takes it back.
    click(&mut harness, "Others: Read");
    assert_eq!(pending(&harness).and_then(|c| c.mode), Some(0o644));
    click(&mut harness, "Revert");
    assert!(pending(&harness).is_none());

    // Ticked again and applied: the file has the mode, and the dialogue
    // follows the file, still on its Permissions tab.
    click(&mut harness, "Others: Read");
    click(&mut harness, "Apply");
    step_until(&mut harness, "the mode was not applied", |_| {
        mode_of(&file) == 0o644
    });
    settle(&mut harness);
    assert!(pending(&harness).is_none(), "the controls follow the file");
    harness.get_by_label("OWNERSHIP");
}

#[test]
fn the_owner_dropdown_lists_the_users_and_other_takes_a_typed_name() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let file = dir.path().join("notes.txt");
    fs::write(&file, b"hello").unwrap_or_else(|e| panic!("{e}"));
    let (mut harness, _fake) = harness(dir.path());
    open_properties(&mut harness, "notes.txt");
    click(&mut harness, "Permissions");
    let owner = harness
        .state()
        .properties()
        .info()
        .and_then(|info| info.permissions.as_ref())
        .map_or_else(|| panic!("permissions"), |p| p.owner.clone());

    // The dropdown is named after its row and opens the list of users,
    // which ends in Other…
    harness
        .get_by_role_and_label(Role::ComboBox, "Owner")
        .click();
    for _ in 0..4 {
        harness.step();
    }
    assert!(harness.query_by_label("Other…").is_some(), "the list");

    // Other… puts a field over the list; a name typed there and Enter is
    // the new owner, to be resolved when the change is applied.
    click(&mut harness, "Other…");
    harness.event(egui::Event::Text("qemu-user".to_owned()));
    harness.step();
    harness.key_press(egui::Key::Enter);
    for _ in 0..4 {
        harness.step();
    }
    let change = harness
        .state()
        .properties()
        .pending_change()
        .unwrap_or_else(|| panic!("a pending change"));
    assert_eq!(
        change.owner.as_ref().map(|p| p.display().to_owned()),
        Some("qemu-user".to_owned())
    );
    assert!(
        harness.query_by_label("Other…").is_none(),
        "the list closed"
    );

    // Revert puts the owner back.
    click(&mut harness, "Revert");
    assert!(harness.state().properties().pending_change().is_none());
    assert_eq!(
        harness
            .state()
            .properties()
            .info()
            .and_then(|info| info.permissions.as_ref())
            .map(|p| p.owner.clone()),
        Some(owner)
    );
}

#[test]
fn the_access_control_list_opens_in_place_and_moves_with_the_grid() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let file = dir.path().join("notes.txt");
    fs::write(&file, b"hello").unwrap_or_else(|e| panic!("{e}"));
    chmod(&file, 0o640);
    let (mut harness, _fake) = harness(dir.path());
    open_properties(&mut harness, "notes.txt");
    let posix = harness
        .state()
        .properties()
        .info()
        .and_then(|info| info.permissions.as_ref())
        .is_some_and(|p| matches!(p.acl, Some(Acl::Posix { .. })));
    if !posix {
        eprintln!("no POSIX ACLs on this file system; the list is not checked here");
        return;
    }
    click(&mut harness, "Permissions");
    harness.get_by_label("No extra entries");
    assert!(
        harness.query_by_label("Mask").is_none(),
        "the list is closed"
    );
    click(&mut harness, "Access control list (ACL)");
    assert!(
        harness.query_by_label("Mask").is_none(),
        "no mask without a named entry"
    );

    // Add is off until a name is typed.
    click(&mut harness, "Add");
    assert!(harness.state().properties().pending_change().is_none());
    // The name field stands in the Add button's line.
    let line = harness.get_by_label("Add").rect().center().y;
    let field = harness
        .get_all_by_role(Role::TextInput)
        .find(|node| (node.rect().center().y - line).abs() < 4.0)
        .unwrap_or_else(|| panic!("the name field"));
    field.focus();
    field.type_text("qemu");
    harness.step();
    harness.step();
    click(&mut harness, "Add");

    // The entry and the mask that comes with it; the mode is as it was,
    // its group bits now being the mask.
    let list = |h: &Harness<'_, RickUi>| match h.state().properties().pending_change() {
        Some(change) => match change.acl {
            Some(Acl::Posix { access, .. }) => (access, change.mode),
            other => panic!("a POSIX list: {other:?}"),
        },
        None => panic!("a pending change"),
    };
    let (access, mode) = list(&harness);
    assert!(
        access
            .iter()
            .any(|e| e.tag == PosixTag::User && e.perms == 4)
    );
    assert!(
        access
            .iter()
            .any(|e| e.tag == PosixTag::Mask && e.perms == 4)
    );
    assert_eq!(mode, None, "r-- for the group class, as before");
    harness.get_by_label("qemu");
    harness.get_by_label("user");
    harness.get_by_label("1 extra entry: user qemu  r--");

    // The table's mask is the grid's Group row: each moves the other.
    click(&mut harness, "Mask: Write");
    assert_eq!(list(&harness).1, Some(0o660));
    click(&mut harness, "Group: Write");
    let (access, mode) = list(&harness);
    assert_eq!(mode, None);
    assert!(
        access
            .iter()
            .any(|e| e.tag == PosixTag::Mask && e.perms == 4)
    );

    // Removing the entry takes the mask with it: nothing is pending.
    click(&mut harness, "Remove qemu");
    assert!(harness.state().properties().pending_change().is_none());
    assert_eq!(mode_of(&file), 0o640, "nothing was applied");
}

/// Issue 178: Escape pressed while Properties is on its way — asked for,
/// its entry still being read — calls it off. The dialogue does not open
/// when the read is answered, the key is not the list's, and asking again
/// opens it.
#[test]
fn escape_before_the_dialogue_has_opened_calls_it_off() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("notes.txt"), "notes").unwrap_or_else(|e| panic!("{e}"));
    let (mut harness, fake) = harness(dir.path());
    let pane = harness.state().app().model().focused;
    let id = harness
        .state()
        .app()
        .model()
        .active_tab(pane)
        .and_then(|(_, tab)| tab.rows().find(|e| e.name == "notes.txt").map(|e| e.id))
        .unwrap_or_else(|| panic!("notes.txt is listed"));
    command(
        &mut harness,
        app::Command::Select {
            pane,
            change: SelectionChange::Set(vec![id]),
        },
    );
    let selected = |harness: &Harness<'_, RickUi>| {
        harness
            .state()
            .app()
            .model()
            .active_tab(pane)
            .map_or(0, |(_, tab)| tab.selection.len())
    };

    // The read is held back, as a slow volume would hold it.
    fake.hold_permissions();
    harness.key_press_modifiers(egui::Modifiers::ALT, egui::Key::Enter);
    for _ in 0..4 {
        harness.step();
    }
    assert!(harness.state().app().properties_pending());
    assert!(!harness.state().properties().is_open());

    escape(&mut harness);
    assert!(
        !harness.state().app().properties_pending(),
        "Escape calls it off"
    );
    assert_eq!(
        selected(&harness),
        1,
        "and is not the list's: the selection stays"
    );

    // The volume answers at last: nothing opens.
    fake.release_permissions();
    settle(&mut harness);
    assert!(
        !harness.state().properties().is_open(),
        "a dialogue that was called off does not open"
    );

    // Asked for again, it opens.
    harness.key_press_modifiers(egui::Modifiers::ALT, egui::Key::Enter);
    step_until(&mut harness, "the dialogue did not open", |h| {
        h.state()
            .properties()
            .info()
            .is_some_and(|info| info.entry.name == "notes.txt")
    });
}
