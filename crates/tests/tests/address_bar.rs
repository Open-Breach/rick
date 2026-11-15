//! Acceptance (issue 219, items 6a and 6b; SPEC.md *A file's path*, *A
//! path that exists only in part*): a typed path that names a file opens
//! the file's folder in the same tab with the file selected, in view and
//! previewed; a path of which only the beginning exists opens the deepest
//! folder that does and names the rest in a banner without *Retry*, and
//! creates nothing.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use platform::{ConfigDir, Fake};
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
