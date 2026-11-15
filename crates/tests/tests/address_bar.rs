//! Acceptance (issue 219, item 6; SPEC.md *Path bar*): the address bar.
//! A typed path that names a file opens the file's folder in the same tab
//! with the file selected, in view and previewed; a path of which only
//! the beginning exists opens the deepest folder that does and names the
//! rest in a banner without *Retry*, and creates nothing; a right-click
//! anywhere on the bar opens a menu whose *Paste from clipboard* goes
//! where the clipboard's text leads without the text field ever showing;
//! and the bar at rest is one rounded surface with the Up button, a
//! divider, the segments and the copy button on it, bordered under the
//! pointer.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use platform::{Clipboard as _, ConfigDir, Fake};
use tests::local;
use ui_egui::{EguiHost, RickUi};

fn harness(start: &Path) -> (Harness<'static, RickUi>, Arc<Fake>) {
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: vec![Arc::new(ext_preview_text::TextPreview::new())],
        config_dir: Some(fake.config_dir()),
        start: Some(local(start)),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1300.0, 700.0))
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

/// The focused pane's active tab.
fn tab<'a>(harness: &'a Harness<'_, RickUi>) -> (model::TabId, &'a model::Tab) {
    let model = harness.state().app().model();
    model
        .active_tab(model.focused)
        .unwrap_or_else(|| panic!("no active tab"))
}

/// The names selected in the focused pane.
fn selected(harness: &Harness<'_, RickUi>) -> Vec<String> {
    tab(harness).1.selected().map(|e| e.name.clone()).collect()
}

/// Types `path` into the path field and confirms it with Enter.
fn type_path(harness: &mut Harness<'_, RickUi>, path: &str) {
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::L);
    harness.step();
    harness.step();
    assert!(harness.state().path_text(tab(harness).0).is_some());
    harness.event(egui::Event::Text(path.to_owned()));
    harness.step();
    harness.key_press(egui::Key::Enter);
    settle(harness);
    assert!(
        harness.state().path_text(tab(harness).0).is_none(),
        "the breadcrumbs are back"
    );
}

/// The breadcrumb bar: the toolbar's middle part.
fn bar(harness: &Harness<'_, RickUi>) -> egui::Rect {
    harness
        .state()
        .toolbar_rects(tab(harness).0)
        .into_iter()
        .find(|(group, _)| *group == "middle")
        .map_or_else(|| panic!("no breadcrumb bar"), |(_, rect)| rect)
}

/// Presses and releases `button` at `at`.
fn click_at(harness: &mut Harness<'_, RickUi>, at: egui::Pos2, button: egui::PointerButton) {
    harness.event(egui::Event::PointerMoved(at));
    harness.step();
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos: at,
            button,
            pressed,
            modifiers: egui::Modifiers::default(),
        });
        harness.step();
    }
    harness.step();
    harness.step();
}

/// A folder `docs` with enough files to scroll, and a folder `other`.
fn tree() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let docs = dir.path().join("docs");
    fs::create_dir(&docs).unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir(dir.path().join("other")).unwrap_or_else(|e| panic!("{e}"));
    for i in 0..120 {
        fs::write(docs.join(format!("file-{i:03}.txt")), format!("{i}\n"))
            .unwrap_or_else(|e| panic!("{e}"));
    }
    fs::write(docs.join("zeta-notes.txt"), "the last one\n").unwrap_or_else(|e| panic!("{e}"));
    dir
}

/// Item 6a: Enter in the path field with a file's path. The pane goes to
/// the file's folder in the same tab, selects the file and scrolls it
/// into view, and the preview follows — what a click on the file does. A
/// folder's path behaves as before.
#[test]
fn a_typed_file_path_opens_its_folder_with_the_file_selected_in_view_and_previewed() {
    let dir = tree();
    let docs = dir.path().join("docs");
    let (mut harness, _fake) = harness(&dir.path().join("other"));
    let workspaces = |h: &Harness<'_, RickUi>| {
        let model = h.state().app().model();
        model.workspaces(model.focused_window).len()
    };
    assert_eq!(workspaces(&harness), 1);

    type_path(&mut harness, &docs.join("zeta-notes.txt").to_string_lossy());
    let (tab_id, t) = tab(&harness);
    assert_eq!(t.location, local(&docs), "the file's folder");
    assert_eq!(t.history.back.len(), 1, "one step, in the same tab");
    assert_eq!(workspaces(&harness), 1, "and not in a new one");
    assert_eq!(selected(&harness), ["zeta-notes.txt"]);
    assert!(t.missing.is_none() && t.failure.is_none());

    // The last of 121 rows is in view: between the toolbar and the footer.
    for _ in 0..4 {
        harness.step();
    }
    let top = harness
        .state()
        .toolbar_row(tab_id)
        .unwrap_or_else(|| panic!("toolbar"))
        .bottom();
    let bottom = harness
        .state()
        .footer_row(tab_id)
        .unwrap_or_else(|| panic!("footer"))
        .top();
    let row = harness.get_by_label("zeta-notes.txt").rect();
    assert!(
        row.top() >= top && row.bottom() <= bottom,
        "the file's row {row:?} is in view between {top} and {bottom}"
    );

    // The preview follows the selection, as after a click.
    let app = harness.state().app();
    let pane = app.model().focused;
    let id = tab(&harness)
        .1
        .rows()
        .find(|e| e.name == "zeta-notes.txt")
        .map_or_else(|| panic!("listed"), |e| e.id);
    assert_eq!(app.preview_target(), Some((pane, id)));

    // Another file of the folder the pane is in already.
    type_path(&mut harness, &docs.join("file-003.txt").to_string_lossy());
    assert_eq!(tab(&harness).1.location, local(&docs));
    assert_eq!(selected(&harness), ["file-003.txt"]);

    // A folder's path: there, with nothing selected, as it always was.
    type_path(&mut harness, &dir.path().join("other").to_string_lossy());
    assert_eq!(tab(&harness).1.location, local(&dir.path().join("other")));
    assert_eq!(selected(&harness), Vec::<String>::new());
}

/// Item 6b: a path whose last two folders do not exist. The pane goes to
/// the deepest folder of the path that exists and the banner over the
/// list names the two, with no *Retry*; nothing is created, and the next
/// navigation takes the banner away. A path of which nothing exists is
/// reported as before.
#[test]
fn a_path_that_exists_only_in_part_opens_the_deepest_folder_and_names_the_rest() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let reports = dir.path().join("reports");
    fs::create_dir(&reports).unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir(dir.path().join("other")).unwrap_or_else(|e| panic!("{e}"));
    let (mut harness, _fake) = harness(&dir.path().join("other"));

    let typed = reports.join("2026").join("q3");
    type_path(&mut harness, &typed.to_string_lossy());
    let (_, t) = tab(&harness);
    assert_eq!(t.location, local(&reports), "the deepest folder there is");
    assert_eq!(t.missing.as_deref(), Some("2026/q3"));
    assert!(t.failure.is_none(), "not a failure");
    harness.get_by_label("2026/q3 does not exist here");
    assert!(
        harness.query_by_label("Retry").is_none(),
        "the banner has no Retry"
    );
    assert!(!reports.join("2026").exists(), "nothing is created");

    // A trailing separator and a file's name at the end change nothing.
    let typed = format!("{}/2026/q3/summary.txt/", reports.to_string_lossy());
    type_path(&mut harness, &typed);
    assert_eq!(tab(&harness).1.location, local(&reports));
    harness.get_by_label("2026/q3/summary.txt does not exist here");

    // Gone at the next navigation.
    type_path(&mut harness, &dir.path().join("other").to_string_lossy());
    assert!(tab(&harness).1.missing.is_none());
    assert!(
        harness
            .query_all_by_label_contains("does not exist here")
            .next()
            .is_none(),
        "the banner went with the folder"
    );

    // Nothing of it exists: the failure it always was, with Retry.
    type_path(&mut harness, "no/such/place");
    let (_, t) = tab(&harness);
    assert!(t.missing.is_none());
    assert!(t.failure.is_some(), "reported as before");
    harness.get_by_label("Retry");
}

/// Item 6c: a right-click anywhere on the bar — a segment, the Up button,
/// the empty part — opens the menu of three entries. *Paste from
/// clipboard* ends at the file whose path the clipboard holds, without
/// the text field ever showing; with no text on the clipboard it is
/// greyed and does nothing. *Copy path* copies the folder's path, and
/// *Edit path* opens the text field.
#[test]
fn the_bars_menu_pastes_from_the_clipboard_copies_the_path_and_edits_it() {
    let dir = tree();
    let docs = dir.path().join("docs");
    let other = dir.path().join("other");
    let (mut harness, fake) = harness(&other);
    let editing = |h: &Harness<'_, RickUi>| h.state().path_text(tab(h).0).is_some();
    let entries = ["Paste from clipboard", "Copy path", "Edit path"];
    let close = |harness: &mut Harness<'_, RickUi>| {
        harness.key_press(egui::Key::Escape);
        harness.step();
        harness.step();
    };

    // Anywhere on the bar: the last segment, the Up button, the empty
    // part at its right.
    let bar_rect = bar(&harness);
    let places = [
        harness
            .query_all_by_label("other")
            .map(|n| n.rect())
            .find(|r| bar_rect.contains(r.center()))
            .unwrap_or_else(|| panic!("no last segment"))
            .center(),
        harness.get_by_label("Parent directory").rect().center(),
        egui::pos2(bar_rect.right() - 20.0, bar_rect.center().y),
    ];
    for at in places {
        click_at(&mut harness, at, egui::PointerButton::Secondary);
        for entry in entries {
            assert_eq!(
                harness.query_all_by_label(entry).count(),
                usize::from(entry != "Copy path") + usize::from(entry == "Copy path") * 2,
                "{entry} after a right-click at {at:?}"
            );
        }
        assert!(!editing(&harness), "a right-click does not edit");
        assert_eq!(tab(&harness).1.location, local(&other), "nor navigate");
        close(&mut harness);
        assert!(harness.query_by_label("Paste from clipboard").is_none());
    }

    // A file's path on the clipboard, with the home folder's tilde to
    // expand: Paste from clipboard ends at the file.
    let home = fake.home_dir();
    fs::create_dir_all(&home).unwrap_or_else(|e| panic!("{e}"));
    fs::write(home.join("pasted.txt"), "pasted\n").unwrap_or_else(|e| panic!("{e}"));
    fake.set_text("  ~/pasted.txt\n")
        .unwrap_or_else(|e| panic!("{e}"));
    click_at(&mut harness, places[2], egui::PointerButton::Secondary);
    harness.get_by_label("Paste from clipboard").click();
    for _ in 0..3 {
        harness.step();
        assert!(!editing(&harness), "the text field never shows");
    }
    settle(&mut harness);
    assert!(!editing(&harness));
    assert_eq!(tab(&harness).1.location, local(&home));
    assert_eq!(selected(&harness), ["pasted.txt"]);

    // No text on the clipboard: greyed, with the reason, and inert.
    fake.set_text("").unwrap_or_else(|e| panic!("{e}"));
    let empty = egui::pos2(bar(&harness).right() - 20.0, bar(&harness).center().y);
    click_at(&mut harness, empty, egui::PointerButton::Secondary);
    let paste = harness.get_by_label("Paste from clipboard").rect().center();
    harness.event(egui::Event::PointerMoved(paste));
    for _ in 0..8 {
        harness.step();
    }
    harness.get_by_label("The clipboard holds no text");
    click_at(&mut harness, paste, egui::PointerButton::Primary);
    settle(&mut harness);
    assert_eq!(tab(&harness).1.location, local(&home), "it does nothing");
    close(&mut harness);

    // Copy path: the folder's path, as the copy button copies it.
    type_path(&mut harness, &docs.to_string_lossy());
    let empty = egui::pos2(bar(&harness).right() - 20.0, bar(&harness).center().y);
    click_at(&mut harness, empty, egui::PointerButton::Secondary);
    let menu_entry = harness
        .query_all_by_label("Copy path")
        .map(|n| n.rect())
        .find(|r| !bar(&harness).contains(r.center()))
        .unwrap_or_else(|| panic!("no Copy path in the menu"));
    click_at(
        &mut harness,
        menu_entry.center(),
        egui::PointerButton::Primary,
    );
    settle(&mut harness);
    assert_eq!(fake.clipboard_text(), Some(docs.display().to_string()));

    // Edit path: the text field, as Mod+L opens it.
    click_at(&mut harness, empty, egui::PointerButton::Secondary);
    harness.get_by_label("Edit path").click();
    harness.step();
    harness.step();
    assert!(editing(&harness));
    assert_eq!(
        harness.state().path_text(tab(&harness).0),
        Some(docs.to_string_lossy().as_ref())
    );
}

/// The rectangles the last frame filled with `colour` inside `area`.
fn filled(
    harness: &Harness<'_, RickUi>,
    area: egui::Rect,
    colour: egui::Color32,
) -> Vec<egui::Rect> {
    harness
        .output()
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Rect(r) if r.fill == colour && area.contains_rect(r.rect) => Some(r.rect),
            _ => None,
        })
        .collect()
}

/// The rectangles the last frame outlined in `colour` inside `area`.
fn outlined(
    harness: &Harness<'_, RickUi>,
    area: egui::Rect,
    colour: egui::Color32,
) -> Vec<egui::Rect> {
    harness
        .output()
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Rect(r)
                if r.stroke.color == colour
                    && r.stroke.width > 0.0
                    && area.contains_rect(r.rect) =>
            {
                Some(r.rect)
            }
            _ => None,
        })
        .collect()
}

/// Item 6d: the bar at rest is one `bg/subtle` surface, `button/size`
/// tall with 6 px at its left, and has no border; the Up button on it is
/// 28 × 26 with no fill, then a divider of 1 × 18 px in `border/strong`
/// 6 px after it. Under the pointer the surface has a 1 px
/// `border/strong` border, and the hovered Up button is `bg/control`.
#[test]
fn the_bar_at_rest_is_one_surface_with_a_border_under_the_pointer() {
    let dir = tree();
    let (mut harness, _fake) = harness(&dir.path().join("docs"));
    let scheme = ui_egui::theme::scheme_of(&harness.ctx);
    let row = harness
        .state()
        .toolbar_row(tab(&harness).0)
        .unwrap_or_else(|| panic!("toolbar"));
    let middle = bar(&harness);

    // At rest, the pointer elsewhere.
    harness.event(egui::Event::PointerMoved(egui::pos2(
        row.center().x,
        row.bottom() + 200.0,
    )));
    harness.step();
    harness.step();
    let surface = filled(&harness, row, scheme.bg_subtle)
        .into_iter()
        .find(|r| (r.width() - middle.width()).abs() < 0.5)
        .unwrap_or_else(|| panic!("no surface in {row:?}"));
    assert!(
        (surface.height() - 34.0).abs() < 0.5,
        "button/size tall in Comfortable: {surface:?}"
    );
    assert!(
        (surface.center().y - row.center().y).abs() < 0.5,
        "on the line of Back and Forward"
    );
    assert_eq!(
        outlined(&harness, row, scheme.border_strong),
        Vec::<egui::Rect>::new(),
        "no border at rest"
    );
    let up = harness.get_by_label("Parent directory").rect();
    assert!(
        (up.width() - 28.0).abs() < 0.5 && (up.height() - 26.0).abs() < 0.5,
        "{up:?}"
    );
    assert!(
        (up.left() - surface.left() - 6.0).abs() < 0.5,
        "6 px of padding at the left: {up:?} on {surface:?}"
    );
    assert_eq!(
        filled(&harness, row, scheme.bg_control),
        Vec::<egui::Rect>::new(),
        "the Up button has no fill of its own"
    );
    let divider = filled(&harness, surface, scheme.border_strong);
    assert_eq!(divider.len(), 1, "one divider: {divider:?}");
    assert!(
        (divider[0].width() - 1.0).abs() < 0.01 && (divider[0].height() - 18.0).abs() < 0.01,
        "1 × 18 px: {:?}",
        divider[0]
    );
    assert!(
        (divider[0].left() - up.right() - 6.0).abs() < 0.5,
        "6 px after the Up button"
    );

    // The pointer on the Up button: the border, and the button one step
    // up from the surface.
    harness.event(egui::Event::PointerMoved(up.center()));
    harness.step();
    harness.step();
    let border = outlined(&harness, row, scheme.border_strong);
    assert_eq!(border, [surface], "a border around the surface");
    assert_eq!(filled(&harness, row, scheme.bg_control), [up]);

    // The pointer on a segment: that segment is the one lit.
    let segment = harness
        .query_all_by_label("docs")
        .map(|n| n.rect())
        .find(|r| middle.contains(r.center()))
        .unwrap_or_else(|| panic!("no last segment"));
    harness.event(egui::Event::PointerMoved(segment.center()));
    harness.step();
    harness.step();
    assert_eq!(filled(&harness, row, scheme.bg_control), [segment]);
    assert_eq!(outlined(&harness, row, scheme.border_strong), [surface]);
}

/// Items 6a and 6b on a provider that is not the local one: the rule
/// asks the provider what each place is, so it holds wherever the
/// provider can say. On the tests' in-memory provider a file's path
/// selects the file in its folder, and a path that ends in two missing
/// folders stops at the deepest one there is.
#[test]
fn a_typed_path_is_followed_on_any_provider_that_says_what_is_there() {
    use tests::mem::{MemExtension, MemProvider};
    use types::{Location, ProviderId};

    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let mem = Arc::new(MemProvider::new(ProviderId("mem"), Duration::ZERO));
    mem.dir("/projects");
    mem.dir("/projects/rick");
    mem.file("/projects/rick/notes.md", b"# Notes\n");
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: vec![Arc::new(MemExtension::new(Arc::clone(&mem)))],
        config_dir: Some(fake.config_dir()),
        start: Some(local(dir.path())),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1300.0, 700.0))
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    let pane = harness.state().app().model().focused;
    let go = |harness: &mut Harness<'_, RickUi>, path: &str| {
        harness.state_mut().app_mut().handle(
            app::Command::GoTo {
                pane,
                to: Location::new(ProviderId("mem"), path),
            },
            &mut Vec::new(),
        );
        settle(harness);
    };
    let at = |path: &str| Location::new(ProviderId("mem"), path);

    go(&mut harness, "/projects/rick/notes.md");
    assert_eq!(tab(&harness).1.location, at("/projects/rick"));
    assert_eq!(selected(&harness), ["notes.md"]);

    go(&mut harness, "/projects/rick/docs/2026");
    let (_, t) = tab(&harness);
    assert_eq!(t.location, at("/projects/rick"));
    assert_eq!(t.missing.as_deref(), Some("docs/2026"));
    assert!(
        !mem.paths().iter().any(|p| p.contains("docs")),
        "nothing is created: {:?}",
        mem.paths()
    );

    // A folder: there, as a navigation always went.
    go(&mut harness, "/projects");
    let (_, t) = tab(&harness);
    assert_eq!(t.location, at("/projects"));
    assert!(t.missing.is_none());
    assert_eq!(selected(&harness), Vec::<String>::new());
}
