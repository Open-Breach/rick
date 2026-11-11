//! Acceptance (issue 149, item 7, the Tracker connector), in the running
//! window, on a machine with neither `tracker3` nor `localsearch` (SPEC.md
//! *Indexed search*, *Attention*).
//!
//! The connector disables itself quietly: the search box is asked and
//! shows no error, the search behaves as it does without an index, and the
//! Extensions dialogue shows the plugin with the attention dot and its
//! reason, *Neither localsearch nor tracker3 was found*.
//!
//! A file of its own, since what a plugin reports to the host is one
//! record for the process.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use app::{Plugin, PluginStatus};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use ext::{
    API_VERSION, BoxStream, Capabilities, Extension, Level, Manifest, Platforms, Points, Registry,
    Version,
};
use ext_search_tracker::{Command, Service, TrackerIndex};
use futures::StreamExt as _;
use platform::{ConfigDir, Fake};
use tests::local;
use types::{CancelToken, Error};
use ui_egui::keys::Intent;
use ui_egui::{EguiHost, RickUi};

/// A machine on which no tool of the indexer can be started.
struct NothingInstalled;

impl Service for NothingInstalled {
    fn output(&self, command: &Command) -> Result<String, String> {
        Err(format!(
            "{}: No such file or directory (os error 2)",
            command.program
        ))
    }

    fn stream(
        &self,
        command: Command,
        _cancel: CancelToken,
    ) -> BoxStream<'static, Result<Vec<String>, Error>> {
        let error = ext_search_tracker::service::tool_error(format!(
            "{}: No such file or directory (os error 2)",
            command.program
        ));
        futures::stream::once(async move { Err(error) }).boxed()
    }
}

/// The Tracker connector over that machine, under the plugin's own id.
struct Tracker(Manifest);

impl Extension for Tracker {
    fn manifest(&self) -> &Manifest {
        &self.0
    }

    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        reg.translations(&[(
            "en",
            include_str!("../../ext-search-tracker/locales/en.ftl"),
        )]);
        reg.search_index(Arc::new(TrackerIndex::new(Arc::new(NothingInstalled))));
        Ok(())
    }
}

fn tracker() -> Arc<dyn Extension> {
    Arc::new(Tracker(Manifest {
        id: ext_search_tracker::ID,
        name: "Tracker",
        description: "Searches the GNOME file index from the search box.",
        version: Version::new(0, 4, 0),
        api_version: API_VERSION,
        requires: Capabilities::empty(),
        points: Points::SEARCH_INDEX,
        platforms: Platforms::ALL,
        previews: &[],
        commands: &[],
    }))
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

fn window(dir: &Path) -> Harness<'static, RickUi> {
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let mut app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: vec![tracker()],
        config_dir: Some(fake.config_dir()),
        start: Some(local(dir)),
    });
    app.set_plugins(vec![Plugin {
        id: ext_search_tracker::ID.0.to_owned(),
        name: "Tracker".to_owned(),
        description: String::new(),
        version: "0.4.0-dev".to_owned(),
        points: Points::SEARCH_INDEX,
        previews: Vec::new(),
        status: PluginStatus::Loaded,
        path: dir.join("search-tracker"),
        built_in: true,
        removed: false,
    }]);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 820.0))
        .with_pixels_per_point(1.0)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    steps(&mut harness);
    harness
}

#[test]
fn with_neither_tool_the_search_shows_no_error_and_the_plugin_needs_attention() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(dir.path().join("notes.txt"), "notes").unwrap_or_else(|e| panic!("{e}"));
    let mut harness = window(dir.path());

    // The connector looked at start, found neither and said so, quietly.
    step_until(
        &mut harness,
        "the connector did not look for its tools",
        |_| ext::attention::of(ext_search_tracker::ID.0).is_some(),
    );
    assert_eq!(
        ext::attention::of(ext_search_tracker::ID.0).as_deref(),
        Some("Neither localsearch nor tracker3 was found")
    );
    assert!(
        harness.state().toasts().is_empty(),
        "nothing is said in the window: {:?}",
        harness.state().toasts()
    );

    // The search box asks: no index is there to answer, and no error
    // reaches it. The box searches as it does without an index.
    harness.state_mut().app_mut().handle(
        app::Command::IndexSearch {
            text: "notes".to_owned(),
        },
        &mut Vec::new(),
    );
    for _ in 0..30 {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    let model = harness.state().app().model();
    assert!(
        model.search_indexes.is_empty(),
        "{:?}",
        model.search_indexes
    );
    assert_eq!(model.index_search.errors, [] as [String; 0]);
    assert_eq!(model.index_search.hits.len(), 0);
    assert!(
        !harness
            .state()
            .toasts()
            .iter()
            .any(|(level, _)| *level != Level::Info),
        "{:?}",
        harness.state().toasts()
    );
    assert!(
        harness
            .query_all_by_label_contains("No such file")
            .next()
            .is_none()
    );

    // The Extensions dialogue: the dot, and the card with the reason.
    let pane = harness.state().app().model().focused;
    harness.state_mut().queue(Intent::Extensions, pane);
    steps(&mut harness);
    assert!(harness.state().extensions_open());
    harness.get_by_label("1 installed  ·  1 enabled  ·  1 needs attention");
    assert_eq!(
        harness.query_all_by_label("Needs attention").count(),
        2,
        "the dot and the card"
    );
    harness.get_by_label("Neither localsearch nor tracker3 was found");
}
