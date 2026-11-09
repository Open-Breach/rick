//! Acceptance (issue 60, footer): the footer row is the toolbar's height
//! under the list; the hidden-files toggle, the folder and file counts,
//! the selection pill, the counts while a filter narrows the list, and
//! Escape clearing the filter.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use model::{Density, SelectionChange};
use platform::{ConfigDir, Fake};
use tests::local;
use ui_egui::{EguiHost, RickUi};

fn harness(start: &Path, fake: &Arc<Fake>) -> Harness<'static, RickUi> {
    let host = Arc::new(EguiHost::new());
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: Vec::new(),
        config_dir: Some(fake.config_dir()),
        start: Some(local(start)),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 700.0))
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

fn command(harness: &mut Harness<'_, RickUi>, command: app::Command) {
    harness
        .state_mut()
        .app_mut()
        .handle(command, &mut Vec::new());
    settle(harness);
}

/// Two folders, three files of 5, 6 and 7 bytes, and a hidden file.
fn tree() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    for name in ["alpha", "beta"] {
        fs::create_dir(dir.path().join(name)).unwrap_or_else(|e| panic!("{e}"));
    }
    for (name, bytes) in [("apple.txt", 5), ("banana.md", 6), ("cherry.rs", 7)] {
        fs::write(dir.path().join(name), vec![b'x'; bytes]).unwrap_or_else(|e| panic!("{e}"));
    }
    fs::write(dir.path().join(".hidden"), "x").unwrap_or_else(|e| panic!("{e}"));
    dir
}

fn entry_id(harness: &Harness<'_, RickUi>, name: &str) -> types::EntryId {
    let pane = harness.state().app().model().focused;
    harness
        .state()
        .app()
        .model()
        .active_tab(pane)
        .and_then(|(_, t)| t.rows().find(|e| e.name == name).map(|e| e.id))
        .unwrap_or_else(|| panic!("no entry {name}"))
}

#[test]
fn the_footer_is_the_toolbar_tall_and_counts_folders_files_and_the_selection() {
    let dir = tree();
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut harness = harness(dir.path(), &fake);
    let pane = harness.state().app().model().focused;
    let tab = harness
        .state()
        .app()
        .model()
        .active_tab(pane)
        .map_or_else(|| panic!("no tab"), |(id, _)| id);
    // The row: as tall as the toolbar, below the last row of the list.
    let row = harness
        .state()
        .footer_row(tab)
        .unwrap_or_else(|| panic!("no footer"));
    let toolbar = ui_egui::toolbar_height(Density::Comfortable);
    assert!(
        (row.height() - toolbar).abs() < 0.5,
        "{} against {toolbar}",
        row.height()
    );
    let last = harness.get_by_label("cherry.rs").rect();
    assert!(row.top() >= last.bottom(), "under the list");
    // The counts, read aloud with their nouns; no pill without a selection.
    harness.get_by_label("2 folders");
    harness.get_by_label("3 files");
    harness.get_by_label("18 B in files");
    assert!(harness.query_by_label_contains("selected").is_none());
    // A selection brings the pill.
    let apple = entry_id(&harness, "apple.txt");
    command(
        &mut harness,
        app::Command::Select {
            pane,
            change: SelectionChange::Set(vec![apple]),
        },
    );
    harness.get_by_label("1 selected · 5 B");
    // The hidden-files toggle shows the dot-file and names its new state.
    harness.get_by_label("Show hidden files").click();
    harness.step();
    settle(&mut harness);
    assert!(
        harness
            .state()
            .app()
            .model()
            .active_tab(pane)
            .is_some_and(|(_, t)| t.show_hidden)
    );
    harness.get_by_label("Hide hidden files");
    harness.get_by_label("4 files");
    harness.get_by_label(".hidden");
}

#[test]
fn a_filter_narrows_the_counts_and_escape_clears_it() {
    let dir = tree();
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut harness = harness(dir.path(), &fake);
    let pane = harness.state().app().model().focused;
    command(
        &mut harness,
        app::Command::Filter {
            pane,
            text: "an".to_owned(),
        },
    );
    harness.get_by_label("banana.md");
    assert!(harness.query_by_label("apple.txt").is_none());
    harness.get_by_label("0 of 2 folders");
    harness.get_by_label("1 of 3 files");
    // Escape clears the filter; the counts are whole again.
    harness.key_press(egui::Key::Escape);
    harness.step();
    settle(&mut harness);
    assert!(
        harness
            .state()
            .app()
            .model()
            .active_tab(pane)
            .is_some_and(|(_, t)| t.filter.is_empty())
    );
    harness.get_by_label("3 files");
    harness.get_by_label("apple.txt");
}
