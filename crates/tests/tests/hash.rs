//! Acceptance (issue 54): Compute hash, headless through the app.
//!
//! A file in an in-memory provider is read exactly once — its `open_read`
//! counter equals the file's size when the job is done — and its digests
//! match the crates'; a local file's four values match MD5, SHA-1, SHA-256
//! and BLAKE3 computed with the crates themselves, in every format, as
//! `md5sum` and friends would print them; a selection of several files
//! gives one block per file in order, and a folder in the selection hides
//! the action; cancelling mid-job on a file of tens of MiB from the
//! fixture generator ends the job with `Error::Cancelled`, stores no
//! result and leaves nothing running; the verify logic tells a SHA-256
//! from a BLAKE3 of the same length, touches only SHA-1 for a 40-character
//! value and calls a 10-character one unrecognised; the format toggle
//! lands in `core.toml` and comes back on the next dialogue; and the egui
//! shell opens the dialogue on the event and fills in its rows.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use app::hash::{self, Format};
use app::{App, Command, Config, Event};
use egui_kittest::kittest::Queryable as _;
use ext::{ActionId, Algorithm, Availability, Digests, Extension, Hashes, Invocation};
use model::{JobState, SelectionChange};
use platform::{ConfigDir as _, Fake};
use sha2::Digest as _;
use tests::fixture::{self, SizeDistribution};
use tests::local;
use tests::mem::{MemExtension, MemProvider};
use types::{Error, Location, ProviderId};
use ui_egui::{EguiHost, Intent, RickUi};

/// A UI whose answers are fixed.
struct Scripted;

impl ext::UiHost for Scripted {
    fn notify(&self, _level: ext::Level, _text: &str) {}
    fn confirm(&self, _title: &str, _text: &str) -> bool {
        true
    }
    fn prompt(&self, _title: &str, default: &str) -> Option<String> {
        Some(default.to_owned())
    }
    fn show(&self, _title: &str, _text: &str) {}
    fn form(&self, form: ext::Form) -> Option<ext::Values> {
        Some(form.defaults())
    }
}

/// An application over a fake platform, driven without a window.
struct Driver {
    app: App,
    fake: Arc<Fake>,
    events: Vec<Event>,
    pane: model::PaneId,
}

impl Driver {
    fn new(start: &Path, extensions: Vec<Arc<dyn Extension>>) -> Self {
        let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
        let app = App::new(Config {
            platform: fake.clone(),
            ui: Arc::new(Scripted),
            extensions,
            config_dir: Some(fake.config_dir()),
            start: Some(local(start)),
        });
        let pane = app.model().focused;
        let mut driver = Self {
            app,
            fake,
            events: Vec::new(),
            pane,
        };
        driver.settle();
        driver
    }

    /// Handles the command and returns what it produced right away,
    /// without waiting for the jobs it started.
    fn send(&mut self, command: Command) -> Vec<Event> {
        self.events.clear();
        self.app.handle(command, &mut self.events);
        std::mem::take(&mut self.events)
    }

    /// Handles the command and waits for everything it started.
    fn step(&mut self, command: Command) -> Vec<Event> {
        self.events.clear();
        self.app.handle(command, &mut self.events);
        self.settle();
        std::mem::take(&mut self.events)
    }

    /// Runs the loop until nothing arrives for a while and nothing runs;
    /// what arrived is appended to `events`.
    fn settle(&mut self) {
        let deadline = Instant::now() + Duration::from_mins(1);
        runtime::block_on(async {
            loop {
                let quiet = tokio::time::timeout(
                    Duration::from_millis(100),
                    self.app.tick(&mut self.events),
                )
                .await
                .is_err();
                if quiet && !self.app.is_busy() {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "did not settle: {:?}",
                    self.app.model().jobs
                );
            }
        });
    }

    fn ctx(&self) -> ext::ActionCtx {
        self.app
            .action_ctx(self.pane, Invocation::ContextMenu)
            .unwrap_or_else(|| panic!("no active tab"))
    }

    /// Runs the action and waits for its job.
    fn run(&mut self, action: &'static str) -> Vec<Event> {
        let ctx = self.ctx();
        self.step(Command::Run {
            action: ActionId(action),
            ctx,
        })
    }

    /// Starts the action and returns without waiting.
    fn start(&mut self, action: &'static str) -> Vec<Event> {
        let ctx = self.ctx();
        self.send(Command::Run {
            action: ActionId(action),
            ctx,
        })
    }

    fn availability(&self, action: &str) -> Availability {
        let ctx = self.ctx();
        self.app
            .registry()
            .actions()
            .iter()
            .find(|a| a.id().0 == action)
            .map_or(Availability::Hidden, |a| a.availability(&ctx))
    }

    fn tab(&self) -> &model::Tab {
        self.app
            .model()
            .active_tab(self.pane)
            .map_or_else(|| panic!("no active tab"), |(_, tab)| tab)
    }

    fn names(&self) -> Vec<String> {
        self.tab().listing.rows().map(|e| e.name.clone()).collect()
    }

    fn select(&mut self, names: &[&str]) {
        let ids: Vec<_> = names
            .iter()
            .map(|name| {
                self.tab()
                    .listing
                    .rows()
                    .find(|e| e.name == *name)
                    .map_or_else(|| panic!("no entry {name} in {:?}", self.names()), |e| e.id)
            })
            .collect();
        self.step(Command::Select {
            pane: self.pane,
            change: SelectionChange::Set(ids),
        });
    }

    fn navigate(&mut self, to: Location) {
        self.step(Command::Navigate {
            pane: self.pane,
            to,
        });
    }

    fn job_state(&self, id: types::JobId) -> Option<JobState> {
        self.app
            .model()
            .jobs
            .get(&id)
            .map(|info| info.state.clone())
    }
}

/// The slot and format the events opened the dialogue on.
fn opened(events: &[Event]) -> Option<(Arc<Hashes>, Format)> {
    events.iter().find_map(|e| match e {
        Event::HashesOpened { hashes, format, .. } => Some((Arc::clone(hashes), *format)),
        _ => None,
    })
}

/// The digests of `bytes` by the crates themselves, independently of the
/// action's hashers.
fn reference(bytes: &[u8]) -> Digests {
    Digests {
        md5: md5::Md5::digest(bytes).into(),
        sha1: sha1::Sha1::digest(bytes).into(),
        sha256: sha2::Sha256::digest(bytes).into(),
        blake3: *blake3::hash(bytes).as_bytes(),
    }
}

/// `len` bytes that are not all alike, the same every run.
fn noise(len: usize, seed: u64) -> Vec<u8> {
    let mut state = seed;
    (0..len)
        .map(|_| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 56) as u8
        })
        .collect()
}

fn result_of(hashes: &Hashes, index: usize) -> Digests {
    hashes
        .result(index)
        .unwrap_or_else(|| panic!("no result for {}", hashes.names()[index]))
        .unwrap_or_else(|e| panic!("{}: {e}", hashes.names()[index]))
}

#[test]
fn a_file_in_memory_is_read_once_and_its_digests_match() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    // Three whole chunks and a partial one, so the last read is short.
    let bytes = noise(3 * hash::CHUNK + 12_345, 1);
    let mem = Arc::new(MemProvider::new(ProviderId("mem"), Duration::ZERO));
    mem.file("/data/blob.bin", &bytes);
    let ext: Arc<dyn Extension> = Arc::new(MemExtension::new(Arc::clone(&mem)));
    let mut d = Driver::new(dir.path(), vec![ext]);
    d.navigate(Location::new(ProviderId("mem"), "/data"));
    d.select(&["blob.bin"]);
    assert_eq!(d.availability("core.hash"), Availability::Enabled);
    // Selecting the file made the app sniff its type; what the job reads
    // is what comes after that.
    let before = mem.read_bytes();

    let events = d.run("core.hash");
    let (hashes, format) = opened(&events).unwrap_or_else(|| panic!("no dialogue: {events:?}"));
    assert_eq!(format, Format::Hex, "lower-case hex until chosen otherwise");
    assert_eq!(hashes.names(), ["blob.bin"]);
    assert!(hashes.is_complete());
    assert_eq!(result_of(&hashes, 0), reference(&bytes));
    assert_eq!(
        mem.read_bytes() - before,
        bytes.len() as u64,
        "the file was read exactly once"
    );
    let id = hashes.job().unwrap_or_else(|| panic!("no job"));
    assert_eq!(d.job_state(id), Some(JobState::Done));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::JobDone { id: done, result: Ok(()) } if *done == id)),
        "{events:?}"
    );
    // The job published byte progress against the file's size.
    assert!(
        events.iter().any(|e| matches!(
            e,
            Event::JobProgress {
                id: p,
                progress: types::Progress::Bytes { done, total: Some(total) }
            } if *p == id && *done == bytes.len() as u64 && *total == bytes.len() as u64
        )),
        "{events:?}"
    );
}

#[test]
fn the_values_match_the_checksum_tools_on_a_local_file() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let bytes = noise(hash::CHUNK + 777, 2);
    fs::write(dir.path().join("release.tar"), &bytes).unwrap_or_else(|e| panic!("{e}"));
    let mut d = Driver::new(dir.path(), Vec::new());
    d.select(&["release.tar"]);
    let events = d.run("core.hash");
    let (hashes, _) = opened(&events).unwrap_or_else(|| panic!("no dialogue: {events:?}"));
    let digests = result_of(&hashes, 0);
    let expected = reference(&bytes);
    // What `md5sum`, `sha1sum`, `sha256sum` and `b3sum` print.
    assert_eq!(
        hash::render(&digests.md5, Format::Hex),
        hex::encode(expected.md5)
    );
    assert_eq!(
        hash::render(&digests.sha1, Format::Hex),
        hex::encode(expected.sha1)
    );
    assert_eq!(
        hash::render(&digests.sha256, Format::Hex),
        hex::encode(expected.sha256)
    );
    assert_eq!(
        hash::render(&digests.blake3, Format::Hex),
        hex::encode(expected.blake3)
    );
    // The other formats re-render the same bytes.
    assert_eq!(
        hash::render(&digests.sha256, Format::HexUpper),
        hex::encode_upper(expected.sha256)
    );
    assert_eq!(hash::render(&digests.sha256, Format::Base64).len(), 44);
    assert_eq!(
        hash::parse_verify(
            &hash::render(&digests.sha256, Format::Base64),
            Format::Base64
        )
        .as_deref(),
        Some(&expected.sha256[..])
    );
    let text = hash::copy_all("release.tar", &digests, Format::Hex);
    assert_eq!(
        text.lines().next(),
        Some(format!("MD5  {}  release.tar", hex::encode(expected.md5)).as_str())
    );
    assert_eq!(text.lines().count(), 4);
}

#[test]
fn several_files_give_one_block_each_in_order_and_a_folder_hides_the_action() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    for (name, text) in [("a.txt", "alpha"), ("b.txt", "beta"), ("c.txt", "gamma")] {
        fs::write(dir.path().join(name), text).unwrap_or_else(|e| panic!("{e}"));
    }
    fs::create_dir(dir.path().join("dir")).unwrap_or_else(|e| panic!("{e}"));
    let mut d = Driver::new(dir.path(), Vec::new());

    d.select(&["a.txt", "b.txt", "c.txt"]);
    let events = d.run("core.hash");
    let (hashes, _) = opened(&events).unwrap_or_else(|| panic!("no dialogue: {events:?}"));
    assert_eq!(hashes.names(), ["a.txt", "b.txt", "c.txt"]);
    assert!(hashes.is_complete());
    for (index, text) in ["alpha", "beta", "gamma"].into_iter().enumerate() {
        assert_eq!(
            result_of(&hashes, index),
            reference(text.as_bytes()),
            "{index}"
        );
    }
    // One job for the three, named after the selection.
    let id = hashes.job().unwrap_or_else(|| panic!("no job"));
    assert_eq!(
        d.app.model().jobs.get(&id).map(|j| j.name.as_str()),
        Some("Hash 3 items")
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Event::JobDone { .. }))
            .count(),
        1
    );

    // A folder in the selection hides the entry; nothing selected greys it.
    d.select(&["a.txt", "dir"]);
    assert_eq!(d.availability("core.hash"), Availability::Hidden);
    let events = d.run("core.hash");
    assert!(opened(&events).is_none(), "{events:?}");
    d.select(&["dir"]);
    assert_eq!(d.availability("core.hash"), Availability::Hidden);
    d.select(&[]);
    assert!(matches!(
        d.availability("core.hash"),
        Availability::Disabled(_)
    ));
}

#[test]
fn cancelling_mid_job_stores_nothing_and_leaves_nothing_running() {
    // One file of 48 MiB from the fixture generator, at its root.
    let fixture = fixture::Config {
        entries: 1,
        sizes: SizeDistribution::Fixed(48 << 20),
        ..fixture::Config::default()
    }
    .generate()
    .unwrap_or_else(|e| panic!("{e}"));
    let mut d = Driver::new(fixture.root(), Vec::new());
    let names = d.names();
    assert_eq!(names.len(), 1, "{names:?}");
    let name = names[0].clone();
    d.select(&[name.as_str()]);

    let events = d.start("core.hash");
    let (hashes, _) = opened(&events).unwrap_or_else(|| panic!("no dialogue: {events:?}"));
    let id = hashes
        .job()
        .unwrap_or_else(|| panic!("the job id is set before the dialogue opens"));
    assert!(
        hashes.result(0).is_none(),
        "the dialogue opens before the job is done"
    );
    let events = d.step(Command::CancelJob(id));
    assert!(
        events.iter().any(|e| matches!(
            e,
            Event::JobDone { id: done, result: Err(Error::Cancelled) } if *done == id
        )),
        "{events:?}"
    );
    assert_eq!(d.job_state(id), Some(JobState::Cancelled));
    assert!(
        hashes.result(0).is_none(),
        "no result is stored for a cancelled job"
    );
    assert!(!hashes.is_complete());
    assert!(!d.app.is_busy(), "nothing is left running");
}

#[test]
fn verify_tells_the_algorithms_apart() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let bytes = noise(4096, 3);
    fs::write(dir.path().join("image.iso"), &bytes).unwrap_or_else(|e| panic!("{e}"));
    let mut d = Driver::new(dir.path(), Vec::new());
    d.select(&["image.iso"]);
    let events = d.run("core.hash");
    let (hashes, format) = opened(&events).unwrap_or_else(|| panic!("no dialogue: {events:?}"));
    let digests = result_of(&hashes, 0);
    let expected = reference(&bytes);

    // A correct SHA-256 matches its row and mismatches BLAKE3's.
    let sha256 = hex::encode(expected.sha256);
    assert_eq!(
        hash::verify(&sha256, format, &digests),
        Some(vec![(Algorithm::Sha256, true), (Algorithm::Blake3, false)])
    );
    // And the reverse, pasted in upper case with whitespace around it.
    let blake3 = format!("  {}\n", hex::encode_upper(expected.blake3));
    assert_eq!(
        hash::verify(&blake3, format, &digests),
        Some(vec![(Algorithm::Sha256, false), (Algorithm::Blake3, true)])
    );
    // 40 characters touch SHA-1 only.
    assert_eq!(
        hash::verify(&hex::encode(expected.sha1), format, &digests),
        Some(vec![(Algorithm::Sha1, true)])
    );
    assert_eq!(
        hash::verify(&"f".repeat(40), format, &digests),
        Some(vec![(Algorithm::Sha1, false)])
    );
    // 32 hex characters are MD5.
    assert_eq!(
        hash::verify(&hex::encode(expected.md5), format, &digests),
        Some(vec![(Algorithm::Md5, true)])
    );
    // Ten characters fit nothing, and so does text that is not hex.
    assert_eq!(hash::verify(&"a".repeat(10), format, &digests), None);
    assert_eq!(hash::verify("not a hash", format, &digests), None);
    // With the Base64 toggle the field reads Base64.
    assert_eq!(
        hash::verify(
            &hash::render(&expected.sha256, Format::Base64),
            Format::Base64,
            &digests
        ),
        Some(vec![(Algorithm::Sha256, true), (Algorithm::Blake3, false)])
    );
    assert_eq!(hash::verify(&sha256, Format::Base64, &digests), None);
}

#[test]
fn the_format_toggle_is_remembered_in_core_toml() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("a.bin"), b"abc").unwrap_or_else(|e| panic!("{e}"));
    let mut d = Driver::new(dir.path(), Vec::new());
    d.select(&["a.bin"]);
    let events = d.run("core.hash");
    assert_eq!(opened(&events).map(|(_, f)| f), Some(Format::Hex));

    d.step(Command::HashFormat(Format::Base64));
    let file = d.fake.config_dir().join("core.toml");
    let table: toml::Table = fs::read_to_string(&file)
        .unwrap_or_else(|e| panic!("{}: {e}", file.display()))
        .parse()
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        table
            .get("hash")
            .and_then(|t| t.get("format"))
            .and_then(|v| v.as_str()),
        Some("base64")
    );
    // The next dialogue opens with it.
    let events = d.run("core.hash");
    assert_eq!(opened(&events).map(|(_, f)| f), Some(Format::Base64));

    d.step(Command::HashFormat(Format::HexUpper));
    let text = fs::read_to_string(&file).unwrap_or_default();
    assert!(text.contains("format = \"hex-upper\""), "{text}");
    let events = d.run("core.hash");
    assert_eq!(opened(&events).map(|(_, f)| f), Some(Format::HexUpper));
}

#[test]
fn the_shell_opens_the_dialogue_and_fills_in_its_rows() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let bytes = noise(70_000, 4);
    fs::write(dir.path().join("notes.txt"), &bytes).unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = App::new(Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: Vec::new(),
        config_dir: Some(fake.config_dir()),
        start: Some(local(dir.path())),
    });
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1400.0, 900.0))
        .with_step_dt(0.05)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    let deadline = Instant::now() + Duration::from_secs(30);
    while harness.state().app().is_busy() {
        harness.step();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    harness.step();
    let pane = harness.state().app().model().focused;
    let id = harness
        .state()
        .app()
        .model()
        .active_tab(pane)
        .and_then(|(_, tab)| tab.listing.rows().find(|e| e.name == "notes.txt"))
        .map_or_else(|| panic!("notes.txt is not listed"), |e| e.id);
    harness.state_mut().app_mut().handle(
        Command::Select {
            pane,
            change: SelectionChange::Set(vec![id]),
        },
        &mut Vec::new(),
    );
    harness.step();
    assert!(!harness.state().hash_dialog().is_open());

    // The action from the shell, as the context menu runs it.
    harness
        .state_mut()
        .queue(Intent::Action(ActionId("core.hash")), pane);
    harness.step();
    harness.step();
    assert!(
        harness.state().hash_dialog().is_open(),
        "the dialogue opens at once"
    );
    assert!(
        harness.query_by_label("Hash — notes.txt").is_some(),
        "titled after the file"
    );
    let hashes = harness
        .state()
        .hash_dialog()
        .hashes()
        .cloned()
        .unwrap_or_else(|| panic!("no slot"));
    let deadline = Instant::now() + Duration::from_secs(30);
    while !hashes.is_complete() || harness.state().app().is_busy() {
        harness.step();
        assert!(Instant::now() < deadline, "the job did not finish");
        std::thread::sleep(Duration::from_millis(5));
    }
    harness.step();
    harness.step();
    assert_eq!(result_of(&hashes, 0), reference(&bytes));
    for label in ["MD5", "SHA-1", "SHA-256", "BLAKE3", "Copy all", "Verify"] {
        assert!(harness.query_by_label(label).is_some(), "{label} is drawn");
    }
    assert_eq!(harness.state().hash_dialog().format(), Some(Format::Hex));

    rows_fit_their_values(&harness, &reference(&bytes));
}

/// Every value field of the dialogue is wide enough for the whole hex
/// SHA-256, 64 monospace characters, and its Copy button sits to the right
/// of it: a field in a grid column must not collapse to the grid's minimum
/// cell width.
fn rows_fit_their_values(harness: &egui_kittest::Harness<'_, RickUi>, digests: &Digests) {
    let values: Vec<String> = Algorithm::ALL
        .into_iter()
        .map(|a| hash::render(digests.get(a), Format::Hex))
        .collect();
    let font = egui::TextStyle::Monospace.resolve(&harness.ctx.global_style());
    let glyph = harness.ctx.fonts_mut(|f| f.glyph_width(&font, '0'));
    let copies: Vec<egui::Rect> = harness
        .get_all_by_role_and_label(egui::accesskit::Role::Button, "Copy")
        .map(|node| node.rect())
        .collect();
    assert_eq!(copies.len(), 4, "a Copy button per row");
    let fields: Vec<(String, egui::Rect)> = harness
        .get_all_by_role(egui::accesskit::Role::TextInput)
        .filter_map(|node| node.value().map(|value| (value, node.rect())))
        .filter(|(value, _)| values.contains(value))
        .collect();
    assert_eq!(fields.len(), 4, "a value field per row: {fields:?}");
    for (value, rect) in &fields {
        assert!(
            rect.width() >= glyph * 64.0,
            "the field of {value} is {} wide, {} characters; 64 are needed",
            rect.width(),
            rect.width() / glyph
        );
        assert!(
            copies.iter().any(|copy| copy.min.x >= rect.max.x - 1.0
                && (copy.center().y - rect.center().y).abs() < rect.height()),
            "no Copy button right of the field of {value} at {rect:?}: {copies:?}"
        );
    }
}
