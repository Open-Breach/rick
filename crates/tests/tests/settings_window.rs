//! Acceptance (issue 60, settings window): the left column with the
//! category list, the View page's cards, a switch that changes a setting,
//! the search that narrows the rows and the categories, the Appearance
//! page's segmented controls, and the close button.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use model::Theme;
use platform::{ConfigDir, Fake};
use tests::local;
use ui_egui::{EguiHost, RickUi};

fn harness(start: &Path) -> Harness<'static, RickUi> {
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: vec![Arc::new(ext_preview_pdf::PdfPreview::new())],
        config_dir: Some(fake.config_dir()),
        start: Some(local(start)),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 900.0))
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    harness
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

fn click_at(harness: &mut Harness<'_, RickUi>, at: egui::Pos2) {
    harness.event(egui::Event::PointerMoved(at));
    harness.step();
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        });
        harness.step();
    }
    harness.step();
}

/// The settings window's search field: the text input right under the
/// *Settings* title, apart from the title row's search box and the
/// footer's filter field.
fn settings_search<'h>(harness: &'h Harness<'_, RickUi>) -> egui_kittest::Node<'h> {
    let title = harness.get_by_label("Settings").rect();
    harness
        .get_all_by_role(Role::TextInput)
        .find(|node| {
            let rect = node.rect();
            (rect.left() - title.left()).abs() < 40.0
                && rect.top() > title.bottom()
                && rect.top() < title.bottom() + 80.0
        })
        .unwrap_or_else(|| panic!("no search field under the Settings title"))
}

fn click_role(harness: &mut Harness<'_, RickUi>, role: Role, label: &str) {
    let at = harness.get_by_role_and_label(role, label).rect().center();
    click_at(harness, at);
}

#[test]
fn the_window_has_the_columns_the_pages_the_search_and_a_working_switch() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let mut harness = harness(dir.path());
    assert!(!harness.state().settings_open());
    harness.state_mut().open_settings();
    for _ in 0..3 {
        harness.step();
    }
    assert!(harness.state().settings_open());
    // The column: the title, the three categories, the plugins' header
    // and the PDF plugin's row; the General page is open first.
    harness.get_by_label("Settings");
    for page in ["General", "View", "Appearance"] {
        harness.get_by_role_and_label(Role::RadioButton, page);
    }
    harness.get_by_label("PLUGINS");
    harness.get_by_role_and_label(Role::RadioButton, "PDF preview");
    harness.get_by_label("Interface");
    harness.get_by_label("Session");

    // The View page: its groups, rows and controls.
    click_role(&mut harness, Role::RadioButton, "View");
    for group in ["Files", "Sorting", "Side panel"] {
        harness.get_by_label(group);
    }
    for row in [
        "Folders first",
        "Show hidden files",
        "Show system files",
        "Show file extensions",
        "Zebra stripes",
    ] {
        harness.get_by_role_and_label(Role::CheckBox, row);
    }
    harness.get_by_label("Folders always stay on top, whatever the sort order");
    harness.get_by_label("Sort by");
    for option in ["Ascending", "Descending", "Left", "Right"] {
        harness.get_by_role_and_label(Role::RadioButton, option);
    }
    harness.get_by_role_and_label(Role::CheckBox, "Show side panel");

    // A switch changes the setting it stands for.
    assert!(harness.state().app().model().settings.folders_first);
    click_role(&mut harness, Role::CheckBox, "Folders first");
    harness.step();
    assert!(
        !harness.state().app().model().settings.folders_first,
        "the switch turned Folders first off"
    );

    // The search narrows the rows and the categories to the matches.
    let search = settings_search(&harness);
    search.focus();
    search.type_text("zebra");
    harness.step();
    harness.step();
    harness.get_by_role_and_label(Role::CheckBox, "Zebra stripes");
    assert!(
        harness
            .query_by_role_and_label(Role::CheckBox, "Folders first")
            .is_none(),
        "rows that do not match are gone"
    );
    assert!(
        harness.query_by_label("Sorting").is_none(),
        "groups without a match are gone"
    );
    assert!(
        harness
            .query_by_role_and_label(Role::RadioButton, "General")
            .is_none(),
        "categories without a match are gone"
    );
    harness.get_by_role_and_label(Role::RadioButton, "View");
    for _ in 0..5 {
        harness.key_press(egui::Key::Backspace);
        harness.step();
    }
    harness.step();
    harness.get_by_role_and_label(Role::CheckBox, "Folders first");
    harness.get_by_role_and_label(Role::RadioButton, "General");

    // The Appearance page: Theme as a segmented control that changes it.
    click_role(&mut harness, Role::RadioButton, "Appearance");
    harness.get_by_label("Colours");
    for option in ["System", "Light", "Dark"] {
        harness.get_by_role_and_label(Role::RadioButton, option);
    }
    click_role(&mut harness, Role::RadioButton, "Dark");
    harness.step();
    assert_eq!(harness.state().app().model().settings.theme, Theme::Dark);

    // The plugin page renders the schema's rows; the close button closes.
    click_role(&mut harness, Role::RadioButton, "PDF preview");
    assert!(
        harness.get_all_by_label("PDF preview").count() >= 2,
        "the plugin's page is titled after it"
    );
    assert!(
        harness
            .get_all_by_role(Role::TextInput)
            .filter(|node| node.rect().left() > 400.0)
            .count()
            >= 1,
        "the plugin's Text setting is a field on its page"
    );
    click_role(&mut harness, Role::Button, "Close");
    harness.step();
    assert!(!harness.state().settings_open());
}
