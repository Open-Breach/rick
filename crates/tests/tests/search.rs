//! Acceptance (0.2 milestone 5, issue 22): search as plugins.
//!
//! The walk plugin, loaded through the plugin host, gives the filter bar
//! its *Search in subdirectories* and streams the matches as 0.1's walk did;
//! the title-row search box asks every available index after the current
//! location, lists the hits with their paths and opens one; and each index
//! connector answers through its real service where the runner has it
//! (Windows Search on Windows, Spotlight on macOS, Tracker and Baloo on
//! Linux with the indexers running), skipping with a note elsewhere unless
//! `RICK_REQUIRE_<SERVICE>` says the service must be there.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use app::{App, Command, Config, Event};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use ext::{
    API_VERSION, Availability, Capabilities, Extension, Manifest, Platforms, Points, Query,
    QueryKind, Registry, SearchHit, SearchIndex, SearchScope, Version,
};
use futures::StreamExt;
use futures::stream;
use model::JobState;
use platform::{ConfigDir, Fake};
use plugins_native::{Host, Library, MANIFEST_FILE, Options, PluginStatus, Source};
use tests::local;
use types::{CancelToken, Entry, Error, ExtId};
use ui_egui::{EguiHost, RickUi};

/// The directory the test binary runs from, where cargo puts every
/// dependency's `cdylib` under its plain name.
fn deps_dir() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_else(|e| panic!("{e}"));
    exe.parent()
        .unwrap_or_else(|| panic!("no parent of {}", exe.display()))
        .to_owned()
}

/// Installs the library of the crate `stem` as a plugin into `plugins_dir`
/// with the manifest the bundler would write.
fn install(plugins_dir: &Path, stem: &str) {
    let library = deps_dir().join(format!(
        "{}{stem}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    assert!(library.is_file(), "not built: {}", library.display());
    let mut toml = plugins_native::describe(&library).unwrap_or_else(|e| panic!("{e}"));
    toml.library = Library::for_crate(stem);
    let dir = plugins_dir.join(&toml.id);
    toml.write(&dir.join(MANIFEST_FILE))
        .unwrap_or_else(|e| panic!("{e}"));
    let file = toml
        .library
        .for_os(std::env::consts::OS)
        .unwrap_or_else(|| panic!("no library name for this OS"));
    fs::copy(&library, dir.join(file)).unwrap_or_else(|e| panic!("{e}"));
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

/// Twenty nested levels with one `foo` each, under `root/sub`.
fn nested(root: &Path) {
    let mut dir = root.join("sub");
    for level in 0..20 {
        dir.push(format!("l{level}"));
        fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        fs::write(dir.join(format!("foo{level}.txt")), "x").unwrap_or_else(|e| panic!("{e}"));
    }
}

/// The walk plugin, loaded from the plugins directory, brings the filter
/// bar's subdirectory search back exactly as the kernel's walk of 0.1 did:
/// the matches stream in as a job with their full paths, and a new text
/// supersedes the walk.
#[test]
fn the_walk_plugin_streams_the_subtree_search_through_the_host() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let plugins = dir.path().join("plugins");
    install(&plugins, "ext_search_walk");
    let host = Host::load(&Options {
        dirs: vec![(Source::BesideBinary, plugins)],
        ..Options::default()
    });
    assert_eq!(host.plugins()[0].status, PluginStatus::Loaded);
    assert_eq!(host.plugins()[0].points, Points::SEARCH_INDEX);

    let tree = dir.path().join("tree");
    fs::create_dir(&tree).unwrap_or_else(|e| panic!("{e}"));
    nested(&tree);
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let ui = Arc::new(EguiHost::new());
    let mut events = Vec::new();
    let mut app = App::new(Config {
        platform: fake.clone(),
        ui,
        extensions: host.extensions(),
        config_dir: Some(fake.config_dir()),
        start: Some(local(&tree)),
    });
    settle(&mut app, &mut events);
    assert_eq!(
        app.subtree_search(&local(&tree)),
        ext::Availability::Enabled,
        "the toggle is there"
    );
    assert!(
        app.model().search_indexes.is_empty(),
        "a walk is no platform index"
    );

    let pane = app.model().focused;
    app.handle(Command::Subtree { pane, on: true }, &mut events);
    app.handle(
        Command::Filter {
            pane,
            text: "foo".into(),
        },
        &mut events,
    );
    settle(&mut app, &mut events);
    let (_, tab) = app
        .model()
        .active_tab(pane)
        .unwrap_or_else(|| panic!("tab"));
    let names: Vec<&str> = tab.listing.rows().map(|e| e.name.as_str()).collect();
    assert_eq!(names.len(), 20, "{names:?}");
    assert!(names.iter().all(|n| n.starts_with("foo")));
    let deep = tab
        .listing
        .rows()
        .find(|e| e.name == "foo19.txt")
        .unwrap_or_else(|| panic!("deep match"));
    assert!(deep.path.to_string_lossy().contains("l19"), "full paths");
    let search_jobs: Vec<&model::JobInfo> = app
        .model()
        .jobs
        .values()
        .filter(|j| j.name.starts_with("Search"))
        .collect();
    assert_eq!(search_jobs.len(), 1, "the walk is a job the user sees");
    assert_eq!(search_jobs[0].state, JobState::Done);

    for text in ["fo", "foo1"] {
        app.handle(
            Command::Filter {
                pane,
                text: text.into(),
            },
            &mut events,
        );
    }
    settle(&mut app, &mut events);
    let states: Vec<JobState> = app
        .model()
        .jobs
        .values()
        .filter(|j| j.name.starts_with("Search"))
        .map(|j| j.state.clone())
        .collect();
    assert_eq!(states.len(), 3, "{states:?}");
    assert!(
        states
            .iter()
            .all(|s| matches!(s, JobState::Done | JobState::Cancelled)),
        "{states:?}"
    );
    let (_, tab) = app
        .model()
        .active_tab(pane)
        .unwrap_or_else(|| panic!("tab"));
    assert_eq!(tab.listing.row_count(), 11, "foo1 and foo10 to foo19");
    drop(app);
    drop(host);
}

/// A platform index for the title-row tests: a fixed list of entries,
/// matched by name.
struct FixedIndex {
    manifest: Manifest,
    hits: Vec<Entry>,
}

impl FixedIndex {
    fn new(hits: Vec<Entry>) -> Arc<Self> {
        Arc::new(Self {
            manifest: Manifest {
                id: ExtId("search-fixed"),
                name: "Fixed index",
                description: "",
                version: Version::new(0, 1, 0),
                api_version: API_VERSION,
                requires: Capabilities::empty(),
                points: Points::SEARCH_INDEX,
                platforms: Platforms::ALL,
                previews: &[],
                commands: &[],
            },
            hits,
        })
    }
}

impl Extension for FixedIndex {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        reg.search_index(Arc::new(FixedSearch(self.hits.clone())));
        Ok(())
    }
}

struct FixedSearch(Vec<Entry>);

impl SearchIndex for FixedSearch {
    fn id(&self) -> ExtId {
        ExtId("search-fixed")
    }
    fn availability(&self) -> Availability {
        Availability::Enabled
    }
    fn scope(&self) -> SearchScope {
        SearchScope::Everywhere
    }
    fn query(
        &self,
        q: &Query,
        _cancel: CancelToken,
    ) -> ext::BoxStream<'static, Result<Vec<SearchHit>, Error>> {
        let needle = q.text.to_lowercase();
        let hits: Vec<SearchHit> = self
            .0
            .iter()
            .filter(|e| e.name.to_lowercase().contains(&needle))
            .map(|e| SearchHit {
                entry: e.clone(),
                snippet: None,
            })
            .collect();
        stream::iter(vec![Ok(hits)]).boxed()
    }
}

/// The entries of real files, as the local provider lists them.
fn entries_of(paths: &[PathBuf]) -> Vec<Entry> {
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let provider = vfs_local::LocalProvider::new(fake);
    runtime::block_on(async {
        let mut out = Vec::new();
        for path in paths {
            out.push(
                ext::Provider::stat(&provider, &local(path))
                    .await
                    .unwrap_or_else(|e| panic!("{e}")),
            );
        }
        out
    })
}

/// Runs frames until the app has nothing in flight and a quiet moment
/// passed.
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

/// Steps frames until `done` holds, within `limit`.
fn step_until(
    harness: &mut Harness<'_, RickUi>,
    limit: Duration,
    mut done: impl FnMut(&Harness<'_, RickUi>) -> bool,
) -> bool {
    let deadline = Instant::now() + limit;
    loop {
        if done(harness) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        harness.step();
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The title-row search box lists the current location's matches first and
/// the index's hits after them, each with its folder; Enter on a hit takes
/// the pane to that folder and selects the file. Without an available
/// index the box searches the current location alone, as in 0.1.
#[test]
fn the_search_box_lists_index_hits_with_their_paths_and_opens_one() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let here = dir.path().join("here");
    let elsewhere = dir.path().join("elsewhere").join("2026");
    fs::create_dir_all(&here).unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir_all(&elsewhere).unwrap_or_else(|e| panic!("{e}"));
    fs::write(here.join("budget-notes.txt"), "x").unwrap_or_else(|e| panic!("{e}"));
    fs::write(here.join("other.txt"), "x").unwrap_or_else(|e| panic!("{e}"));
    fs::write(elsewhere.join("Budget.xlsx"), "x").unwrap_or_else(|e| panic!("{e}"));
    fs::write(elsewhere.join("plan.md"), "x").unwrap_or_else(|e| panic!("{e}"));
    let indexed = entries_of(&[elsewhere.join("Budget.xlsx"), elsewhere.join("plan.md")]);

    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = App::new(Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: vec![FixedIndex::new(indexed)],
        config_dir: Some(fake.config_dir()),
        start: Some(local(&here)),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 720.0))
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle_ui(&mut harness);
    assert_eq!(
        harness.state().app().model().search_indexes,
        [ExtId("search-fixed")]
    );

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::P);
    harness.step();
    assert!(harness.state().search_active());
    harness.event(egui::Event::Text("budget".to_owned()));
    harness.step();
    // The hit arrives after the pause for the next keystroke.
    assert!(
        step_until(&mut harness, Duration::from_secs(5), |h| {
            h.query_by_label_contains("Budget.xlsx").is_some()
        }),
        "the index hit shows in the dropdown"
    );
    let folder = elsewhere.to_string_lossy().into_owned();
    harness.get_by_label(folder.as_str());
    // The current location's match comes first, in the list and the
    // dropdown; the hit is in the dropdown only.
    assert_eq!(
        harness
            .get_all_by_label_contains("budget-notes.txt")
            .count(),
        2
    );
    assert_eq!(harness.get_all_by_label_contains("Budget.xlsx").count(), 1);
    assert!(
        harness.query_by_label_contains("plan.md").is_none(),
        "not a match"
    );
    let search = harness.state().app().model().index_search.clone();
    assert_eq!(search.text, "budget");
    assert!(search.done);
    assert_eq!(search.hits.len(), 1);

    // Down to the hit, Enter: the pane goes to its folder and selects it.
    harness.key_press(egui::Key::ArrowDown);
    harness.step();
    harness.key_press(egui::Key::Enter);
    harness.step();
    harness.step();
    assert!(!harness.state().search_active(), "a pick closes the box");
    settle_ui(&mut harness);
    assert!(
        step_until(&mut harness, Duration::from_secs(5), |h| {
            let pane = h.state().app().model().focused;
            h.state()
                .app()
                .model()
                .active_tab(pane)
                .is_some_and(|(_, tab)| {
                    tab.location == local(&elsewhere)
                        && tab
                            .selection
                            .iter()
                            .any(|id| tab.entry(*id).is_some_and(|e| e.name == "Budget.xlsx"))
                })
        }),
        "the pane shows the hit's folder with the hit selected"
    );
    assert!(
        harness.state().app().model().index_search.text.is_empty(),
        "the pick ended the search"
    );

    // Escape clears the search too.
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::P);
    harness.step();
    harness.event(egui::Event::Text("plan".to_owned()));
    harness.step();
    assert!(step_until(&mut harness, Duration::from_secs(5), |h| {
        h.state().app().model().index_search.done
            && h.state().app().model().index_search.text == "plan"
    }));
    harness.key_press(egui::Key::Escape);
    harness.step();
    harness.step();
    assert!(step_until(&mut harness, Duration::from_secs(2), |h| {
        h.state().app().model().index_search.text.is_empty()
    }));
}

/// The dropdown has a width of its own, grows with its rows, and its rows
/// keep their texts apart. In a window too narrow for a full search box
/// the card is still 560 px wide and inside the window; it is as tall as
/// its rows when a hit of the index comes after the folder's own names,
/// where it used to keep its first height; a name too long for its row
/// ends in
/// an ellipsis before the text at its right, a folder too long is cut at
/// its start, and the two never share a pixel. Both stay whole for a
/// screen reader.
#[test]
fn the_dropdown_has_its_own_width_and_keeps_a_name_and_its_folder_apart() {
    const WINDOW: egui::Vec2 = egui::vec2(700.0, 600.0);
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let here = dir.path().join("here");
    let elsewhere = dir
        .path()
        .join("an-archive-of-everything-that-was-ever-scanned")
        .join("letters-and-statements-by-year")
        .join("2026")
        .join("Backup");
    fs::create_dir_all(&here).unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir_all(&elsewhere).unwrap_or_else(|e| panic!("{e}"));
    let long = "report-of-the-third-quarter-with-every-appendix-and-the-notes-of-the-meeting-that-followed.pdf";
    let hit = "report-on-the-container-examination-with-the-letter-and-all-of-its-enclosures.xlsx";
    fs::write(here.join(long), "x").unwrap_or_else(|e| panic!("{e}"));
    fs::write(here.join("report.md"), "x").unwrap_or_else(|e| panic!("{e}"));
    fs::write(elsewhere.join(hit), "x").unwrap_or_else(|e| panic!("{e}"));
    let indexed = entries_of(&[elsewhere.join(hit)]);

    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = App::new(Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: vec![FixedIndex::new(indexed)],
        config_dir: Some(fake.config_dir()),
        start: Some(local(&here)),
    });
    let mut harness = Harness::builder()
        .with_size(WINDOW)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle_ui(&mut harness);
    assert!(harness.state().search_dropdown().is_none(), "closed");

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::P);
    harness.step();
    harness.event(egui::Event::Text("report".to_owned()));
    harness.step();
    assert!(
        step_until(&mut harness, Duration::from_secs(5), |h| {
            h.state().app().model().index_search.done
                && h.state()
                    .search_dropdown()
                    .is_some_and(|drawn| drawn.rows.len() == 3)
        }),
        "two names here and the hit of the index: {:?}",
        harness.state().search_dropdown()
    );
    // The card grows to its rows in the frame after they came.
    harness.step();
    harness.step();
    let drawn = harness
        .state()
        .search_dropdown()
        .unwrap_or_else(|| panic!("no dropdown"));

    // The card: 560 px, wider than the box can ever be, inside the window.
    let card = drawn.card;
    assert!(
        (card.width() - ui_egui::search::DROPDOWN_WIDTH).abs() < 1.0,
        "{card:?}"
    );
    assert!(
        card.left() >= 8.0 - 0.5 && card.right() <= WINDOW.x - 8.0 + 0.5,
        "inside the window: {card:?}"
    );
    // It opened with the two names of this folder and grew for the hit
    // that the index sent after them: three rows of 30 px, none scrolled.
    assert!(card.height() >= 90.0, "{card:?}");

    // Every row is a line of its own, its texts inside the card and apart.
    for (i, row) in drawn.rows.iter().enumerate() {
        assert!(card.contains_rect(row.label_rect), "{row:?} in {card:?}");
        let aside = row.hint_rect.unwrap_or_else(|| panic!("{row:?}"));
        assert!(card.contains_rect(aside), "{row:?} in {card:?}");
        assert!(
            row.label_rect.right() + 23.5 <= aside.left(),
            "the label ends before the text at its right: {row:?}"
        );
        if let Some(next) = drawn.rows.get(i + 1) {
            assert!(
                row.label_rect.bottom() <= next.label_rect.top(),
                "{row:?} above {next:?}"
            );
        }
    }
    let by_label = |label: &str| {
        drawn
            .rows
            .iter()
            .find(|row| row.label == label)
            .unwrap_or_else(|| panic!("no row {label}: {drawn:?}"))
    };
    // A long name is cut, its short type is not; a short name is whole.
    assert!(by_label(long).label_cut && !by_label(long).hint_cut);
    assert!(!by_label("report.md").label_cut);
    // The hit: both its name and its folder are longer than their share.
    assert!(by_label(hit).label_cut && by_label(hit).hint_cut);
    let hint_width = |label: &str| by_label(label).hint_rect.map_or(0.0, |rect| rect.width());
    assert!(
        hint_width(hit) > 4.0 * hint_width("report.md"),
        "a folder has its half of the row, a type what it needs"
    );

    // Cut on the screen, whole for a screen reader: the names, and the
    // folder as a label of its own.
    let folder = elsewhere.to_string_lossy().into_owned();
    harness.get_by_label(folder.as_str());
    assert_eq!(harness.get_all_by_label(hit).count(), 1);
    assert_eq!(
        harness.get_all_by_label(long).count(),
        2,
        "in the list and in the dropdown"
    );
}

/// A click with the primary button at `at`, as the mouse makes it: the
/// pointer goes there, the button goes down, and up a frame later.
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

/// The dropdown as the last frame drew it.
fn dropdown(harness: &Harness<'_, RickUi>) -> ui_egui::search::Drawn {
    harness
        .state()
        .search_dropdown()
        .unwrap_or_else(|| panic!("no dropdown"))
}

/// A click on a hit does what Enter does: the pane goes to the hit's
/// folder and selects the file there, and the box closes. A click on a
/// name of the current folder selects it in the list.
#[test]
fn a_click_on_a_hit_takes_the_pane_to_its_folder_and_selects_it() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let here = dir.path().join("here");
    let elsewhere = dir.path().join("elsewhere").join("2026");
    fs::create_dir_all(&here).unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir_all(&elsewhere).unwrap_or_else(|e| panic!("{e}"));
    fs::write(here.join("budget-notes.txt"), "x").unwrap_or_else(|e| panic!("{e}"));
    fs::write(here.join("other.txt"), "x").unwrap_or_else(|e| panic!("{e}"));
    fs::write(elsewhere.join("Budget.xlsx"), "x").unwrap_or_else(|e| panic!("{e}"));
    let indexed = entries_of(&[elsewhere.join("Budget.xlsx")]);
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = App::new(Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: vec![FixedIndex::new(indexed)],
        config_dir: Some(fake.config_dir()),
        start: Some(local(&here)),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 720.0))
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle_ui(&mut harness);
    let search = |harness: &mut Harness<'_, RickUi>| {
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::P);
        harness.step();
        harness.event(egui::Event::Text("budget".to_owned()));
        harness.step();
        assert!(step_until(harness, Duration::from_secs(5), |h| {
            h.state().app().model().index_search.done
                && h.state()
                    .search_dropdown()
                    .is_some_and(|drawn| drawn.rows.len() == 2)
        }));
        harness.step();
    };
    let row_of = |harness: &Harness<'_, RickUi>, label: &str| {
        dropdown(harness)
            .rows
            .iter()
            .find(|row| row.label == label)
            .unwrap_or_else(|| panic!("no row {label}"))
            .label_rect
            .center()
    };
    let selected_names = |harness: &Harness<'_, RickUi>| -> Vec<String> {
        let model = harness.state().app().model();
        model
            .active_tab(model.focused)
            .map(|(_, tab)| {
                tab.selection
                    .iter()
                    .filter_map(|id| tab.entry(*id).map(|e| e.name.clone()))
                    .collect()
            })
            .unwrap_or_default()
    };

    // A name of this folder: the click selects it here.
    search(&mut harness);
    let at = row_of(&harness, "budget-notes.txt");
    click_at(&mut harness, at);
    assert!(!harness.state().search_active(), "a pick closes the box");
    assert!(harness.state().search_dropdown().is_none());
    assert_eq!(selected_names(&harness), ["budget-notes.txt"]);

    // The hit of the index: the pane goes to its folder and selects it.
    search(&mut harness);
    let at = row_of(&harness, "Budget.xlsx");
    click_at(&mut harness, at);
    assert!(!harness.state().search_active(), "a pick closes the box");
    settle_ui(&mut harness);
    assert!(
        step_until(&mut harness, Duration::from_secs(5), |h| {
            let model = h.state().app().model();
            model
                .active_tab(model.focused)
                .is_some_and(|(_, tab)| tab.location == local(&elsewhere))
                && selected_names(h) == ["Budget.xlsx"]
        }),
        "the pane shows the hit's folder with the hit selected"
    );
}

/// A folder of thirty notes with *note* typed into the search box: thirty
/// rows, twelve of them in view. Returns the window and the folder, which
/// lives as long as the window needs it.
fn thirty_notes() -> (Harness<'static, RickUi>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    for i in 1..=30 {
        fs::write(dir.path().join(format!("note-{i:02}.txt")), "x")
            .unwrap_or_else(|e| panic!("{e}"));
    }
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = App::new(Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: Vec::new(),
        config_dir: Some(fake.config_dir()),
        start: Some(local(dir.path())),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 720.0))
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle_ui(&mut harness);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::P);
    harness.step();
    harness.event(egui::Event::Text("note".to_owned()));
    for _ in 0..3 {
        harness.step();
    }
    let drawn = dropdown(&harness);
    assert_eq!(drawn.rows.len(), 30);
    assert_eq!(drawn.selected, 0);
    assert!(
        drawn.card.height() < 13.0 * 30.0,
        "twelve rows at once: {:?}",
        drawn.card
    );
    assert!(
        !drawn.card.contains_rect(drawn.rows[29].label_rect),
        "below"
    );
    (harness, dir)
}

/// The arrow keys reach every row: they move the highlight past the
/// twelfth and bring its row into view, also while the pointer rests on
/// the list, where the row under it used to take the highlight back in
/// every frame. Enter picks the highlighted row.
#[test]
fn the_arrow_keys_move_the_highlight_past_a_resting_pointer_and_out_of_view() {
    let (mut harness, _dir) = thirty_notes();
    let drawn = dropdown(&harness);
    let card = drawn.card;
    // The pointer comes to rest on the second row and has the highlight.
    harness.event(egui::Event::PointerMoved(drawn.rows[1].label_rect.center()));
    harness.step();
    harness.step();
    assert_eq!(dropdown(&harness).selected, 1, "the pointer has it");
    for _ in 0..18 {
        harness.key_press(egui::Key::ArrowDown);
        harness.step();
    }
    for _ in 0..20 {
        harness.step();
    }
    let drawn = dropdown(&harness);
    assert_eq!(drawn.selected, 19, "the keys moved it past the pointer");
    assert!(
        card.contains_rect(drawn.rows[19].label_rect),
        "the highlighted row is in view: {:?} in {card:?}",
        drawn.rows[19]
    );
    harness.key_press(egui::Key::Enter);
    harness.step();
    harness.step();
    assert!(!harness.state().search_active(), "a pick closes the box");
    let model = harness.state().app().model();
    let (_, tab) = model
        .active_tab(model.focused)
        .unwrap_or_else(|| panic!("tab"));
    let picked: Vec<_> = tab
        .selection
        .iter()
        .filter_map(|id| tab.entry(*id).map(|e| e.name.clone()))
        .collect();
    assert_eq!(picked, ["note-20.txt"]);
}

/// The pointer reaches every row. The wheel scrolls the list to its last
/// row and the list stays there, the row under the pointer highlighted. A
/// drag of the scroll bar scrolls it back without closing it, where the
/// press used to take the keyboard from the box and the list with it.
#[test]
fn the_wheel_and_the_scroll_bar_reach_every_row_and_keep_the_dropdown_open() {
    let (mut harness, _dir) = thirty_notes();
    let card = dropdown(&harness).card;
    let top = egui::pos2(card.center().x, card.top() + 20.0);
    harness.event(egui::Event::PointerMoved(top));
    harness.step();
    for _ in 0..40 {
        harness.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -60.0),
            modifiers: egui::Modifiers::default(),
            phase: egui::TouchPhase::Move,
        });
        harness.step();
    }
    for _ in 0..30 {
        harness.step();
    }
    let drawn = dropdown(&harness);
    assert!(
        card.contains_rect(drawn.rows[29].label_rect),
        "the last row is in view: {:?} in {card:?}",
        drawn.rows[29]
    );
    let under = drawn
        .rows
        .iter()
        .position(|row| {
            row.label_rect.top() <= top.y + 15.0 && row.label_rect.bottom() >= top.y - 15.0
        })
        .unwrap_or_else(|| panic!("no row at {top:?}"));
    assert_eq!(drawn.selected, under, "the row under the pointer");
    assert!(under >= 18, "{under}");

    // The scroll bar, at the card's right edge: dragged from the bottom to
    // the top, button down over several frames.
    let from = egui::pos2(card.right() - 9.0, card.bottom() - 30.0);
    let to = egui::pos2(from.x, card.top() + 12.0);
    harness.event(egui::Event::PointerMoved(from));
    harness.step();
    harness.event(egui::Event::PointerButton {
        pos: from,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::default(),
    });
    harness.step();
    for step in 1..=8_u8 {
        let at = from + (to - from) * (f32::from(step) / 8.0);
        harness.event(egui::Event::PointerMoved(at));
        harness.step();
        assert!(
            harness.state().search_dropdown().is_some(),
            "open while the bar is dragged"
        );
    }
    harness.event(egui::Event::PointerButton {
        pos: to,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::default(),
    });
    for _ in 0..4 {
        harness.step();
    }
    assert!(harness.state().search_active(), "the box has the keyboard");
    let drawn = dropdown(&harness);
    assert!(
        card.contains_rect(drawn.rows[0].label_rect),
        "back at the first row: {:?} in {card:?}",
        drawn.rows[0]
    );
    assert_eq!(harness.state().search_text(), "note", "nothing was picked");
}

/// Without an index the box behaves as in 0.1: the dropdown lists the
/// current location's matches and nothing else, and no index search runs.
#[test]
fn without_an_index_the_search_box_searches_the_current_location_alone() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("alpha.txt"), "x").unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = App::new(Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: Vec::new(),
        config_dir: Some(fake.config_dir()),
        start: Some(local(dir.path())),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 720.0))
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle_ui(&mut harness);
    assert_eq!(
        harness.state().app().model().search_indexes,
        Vec::<ExtId>::new()
    );
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::P);
    harness.step();
    harness.event(egui::Event::Text("alp".to_owned()));
    harness.step();
    harness.step();
    assert_eq!(harness.get_all_by_label_contains("alpha.txt").count(), 2);
    assert!(
        harness
            .query_by_label_contains("Searching the index")
            .is_none()
    );
    let search = &harness.state().app().model().index_search;
    assert!(search.text.is_empty() && search.done && search.hits.is_empty());
}

/// Whether `RICK_REQUIRE_<name>` says the service must be there.
fn required(name: &str) -> bool {
    std::env::var_os(format!("RICK_REQUIRE_{name}")).is_some()
}

/// A connector against its real service: present or not as the probe
/// says, and when present a query answers. With `RICK_SEARCH_FIXTURE` set
/// to a directory the service indexes, a file written there is found by
/// name within a minute and a half, with its path; without, a broad query
/// must just complete.
fn real_service(name: &str, index: &dyn SearchIndex, probe: bool) {
    if !probe {
        assert!(
            !required(name),
            "RICK_REQUIRE_{name} is set but the {name} service is not there: {:?}",
            index.availability()
        );
        println!("{name} not available here; test skipped");
        return;
    }
    assert_eq!(index.availability(), Availability::Enabled);
    assert_eq!(index.scope(), SearchScope::Everywhere);
    let collect = |text: &str, root: Option<PathBuf>| -> Result<Vec<SearchHit>, Error> {
        let query = Query {
            text: text.to_owned(),
            root: root.map(|r| local(&r)),
            kind: QueryKind::Name,
            filters: Vec::new(),
        };
        runtime::block_on(async {
            let mut stream = index.query(&query, CancelToken::new());
            let mut hits = Vec::new();
            while let Some(batch) = stream.next().await {
                hits.extend(batch?);
            }
            Ok(hits)
        })
    };
    let Some(fixture) = std::env::var_os("RICK_SEARCH_FIXTURE").map(PathBuf::from) else {
        let hits = collect("a", None).unwrap_or_else(|e| panic!("{name}: {e}"));
        println!(
            "{name}: {} hits for a broad query; no fixture directory",
            hits.len()
        );
        assert!(hits.iter().all(|h| h.entry.id.provider.0 == "local"));
        return;
    };
    fs::create_dir_all(&fixture).unwrap_or_else(|e| panic!("{e}"));
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let stem = format!("ricksearch{stamp}");
    let file = fixture.join(format!("{stem}.txt"));
    fs::write(&file, "indexed by rick's test\n").unwrap_or_else(|e| panic!("{e}"));
    let deadline = Instant::now() + Duration::from_secs(90);
    let started = Instant::now();
    loop {
        let hits = collect(&stem, Some(fixture.clone())).unwrap_or_else(|e| panic!("{name}: {e}"));
        if let Some(hit) = hits.iter().find(|h| h.entry.name == format!("{stem}.txt")) {
            assert_eq!(hit.entry.path, local(&file).path);
            println!(
                "{name}: found {} after {:?}",
                file.display(),
                started.elapsed()
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "{name} did not index {} within 90 s",
            file.display()
        );
        std::thread::sleep(Duration::from_secs(1));
    }
    let _ = fs::remove_file(&file);
}

#[test]
fn tracker_answers_through_its_real_service_where_it_runs() {
    let index = ext_search_tracker::TrackerIndex::new(Arc::new(ext_search_tracker::Process));
    let probe = runtime::block_on(index.probe());
    real_service("TRACKER", &index, probe);
}

#[test]
fn baloo_answers_through_its_real_service_where_it_runs() {
    let index = ext_search_baloo::BalooIndex::new(Arc::new(ext_search_baloo::Process));
    let probe = runtime::block_on(index.probe());
    real_service("BALOO", &index, probe);
}

#[test]
fn spotlight_answers_through_its_real_service_where_it_runs() {
    let index = ext_search_spotlight::SpotlightIndex::new(Arc::new(ext_search_spotlight::Process));
    let probe = runtime::block_on(index.probe());
    real_service("SPOTLIGHT", &index, probe);
}

#[test]
fn windows_search_answers_through_its_real_service_where_it_runs() {
    let index = ext_search_windows::WindowsSearchIndex::new(Arc::new(ext_search_windows::Process));
    let probe = runtime::block_on(index.probe());
    real_service("WINDOWS_SEARCH", &index, probe);
}
