//! Acceptance (issue 50, *Opening files*): Enter on an archive enters it as
//! a location; with `archives.on_open = "system"` it goes to the default
//! application; Mod+Enter (the `core.open_default` action) goes to the
//! default application in both cases; and Enter on a video opens the
//! system player and creates no tab, the preview pane being the only
//! viewer.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use app::{App, Command, Config, Event};
use ext::{ActionId, Invocation, Level, UiHost};
use model::{OnOpen, SelectionChange, SettingsChange, TabKind};
use platform::{ConfigDir, Fake};
use tests::local;
use types::{EntryId, Location, ProviderId};

/// A UI that answers nothing; the scenarios ask no question.
#[derive(Default)]
struct Quiet;

impl UiHost for Quiet {
    fn notify(&self, _level: Level, _text: &str) {}
    fn confirm(&self, _title: &str, _text: &str) -> bool {
        false
    }
    fn prompt(&self, _title: &str, default: &str) -> Option<String> {
        Some(default.to_owned())
    }
    fn show(&self, _title: &str, _text: &str) {}
    fn form(&self, form: ext::Form) -> Option<ext::Values> {
        Some(form.defaults())
    }
}

struct Driver {
    app: App,
    fake: Arc<Fake>,
    events: Vec<Event>,
    pane: model::PaneId,
}

impl Driver {
    fn new(start: &Path, extensions: Vec<Arc<dyn ext::Extension>>) -> Self {
        let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
        let app = App::new(Config {
            platform: fake.clone(),
            ui: Arc::new(Quiet),
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

    fn step(&mut self, command: Command) -> Vec<Event> {
        self.events.clear();
        self.app.handle(command, &mut self.events);
        self.settle();
        std::mem::take(&mut self.events)
    }

    fn settle(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(30);
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
                assert!(Instant::now() < deadline, "did not settle: {:?}", self.app);
            }
        });
    }

    fn tab(&self) -> &model::Tab {
        self.app
            .model()
            .active_tab(self.pane)
            .map_or_else(|| panic!("no active tab"), |(_, tab)| tab)
    }

    fn tabs(&self) -> usize {
        self.app.model().all_panes().into_iter().fold(0, |n, pane| {
            n + self.app.model().pane(pane).map_or(0, |p| p.tabs().len())
        })
    }

    fn id_of(&self, name: &str) -> EntryId {
        self.tab()
            .listing
            .rows()
            .find(|e| e.name == name)
            .map_or_else(|| panic!("no entry {name}"), |e| e.id)
    }

    fn select(&mut self, name: &str) -> Vec<Event> {
        let id = self.id_of(name);
        self.step(Command::Select {
            pane: self.pane,
            change: SelectionChange::Set(vec![id]),
        })
    }

    /// Enter on `name`.
    fn open(&mut self, name: &str) -> Vec<Event> {
        let id = self.id_of(name);
        self.step(Command::Open {
            pane: self.pane,
            id,
        })
    }

    /// Mod+Enter on the selection: the `core.open_default` action.
    fn open_default(&mut self) -> Vec<Event> {
        let ctx = self
            .app
            .action_ctx(self.pane, Invocation::Shortcut)
            .unwrap_or_else(|| panic!("no active tab"));
        self.step(Command::Run {
            action: ActionId("core.open_default"),
            ctx,
        })
    }
}

/// A zip with one file in a folder.
fn write_zip(path: &Path) {
    let mut writer = zip::ZipWriter::new(File::create(path).unwrap_or_else(|e| panic!("{e}")));
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    writer
        .start_file("docs/readme.txt", options)
        .unwrap_or_else(|e| panic!("{e}"));
    writer
        .write_all(b"inside the zip\n")
        .unwrap_or_else(|e| panic!("{e}"));
    writer.finish().unwrap_or_else(|e| panic!("{e}"));
}

/// The archive location of `archive` at its root.
fn inside(archive: &Path) -> Location {
    Location::new(ProviderId::ARCHIVE, platform::paths::from_path(archive))
}

#[test]
fn enter_enters_an_archive_unless_the_setting_sends_it_to_the_system() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let zip = dir.path().join("bundle.zip");
    write_zip(&zip);
    let mut d = Driver::new(
        dir.path(),
        vec![
            Arc::new(ext_vfs_archive::ArchiveExtension::new()),
            Arc::new(ext_preview_text::TextPreview::new()),
        ],
    );
    let tabs = d.tabs();

    // Enter enters the archive in the current tab: no new tab, nothing
    // opened outside.
    d.open("bundle.zip");
    assert_eq!(d.tab().location, inside(&zip));
    assert_eq!(d.tabs(), tabs, "the same tab");
    assert_eq!(d.fake.opened(), Vec::<PathBuf>::new());

    // Mod+Enter on the archive goes to the default application whatever
    // the setting.
    d.step(Command::Navigate {
        pane: d.pane,
        to: local(dir.path()),
    });
    d.select("bundle.zip");
    d.open_default();
    assert_eq!(d.fake.opened(), vec![zip.clone()]);
    assert_eq!(d.tab().location, local(dir.path()), "stayed in the folder");

    // With `archives.on_open = "system"` Enter hands it out too.
    d.step(Command::Settings(SettingsChange::ArchivesOnOpen(
        OnOpen::System,
    )));
    assert_eq!(
        d.app.model().settings.archives.on_open,
        OnOpen::System,
        "the setting applied"
    );
    d.open("bundle.zip");
    assert_eq!(d.fake.opened(), vec![zip.clone(), zip.clone()]);
    assert_eq!(d.tab().location, local(dir.path()), "not entered");
    assert_eq!(d.tabs(), tabs);
    d.select("bundle.zip");
    d.open_default();
    assert_eq!(
        d.fake.opened().len(),
        3,
        "Mod+Enter: the default application"
    );

    // The setting is in rick.toml.
    let text = fs::read_to_string(d.fake.config_dir().join(app::SETTINGS_FILE))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(
        text.contains("[archives]") && text.contains("on_open = \"system\""),
        "{text}"
    );
}

#[test]
fn enter_on_a_video_opens_the_system_player_and_no_tab() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let clip = dir.path().join("clip.mp4");
    // A file the media previewer claims by its `ftyp` box; with GStreamer
    // built in a real clip, which also has a first frame and a transport.
    #[cfg(feature = "gstreamer")]
    let real = ext_preview_media::fixtures::encode::mp4(&clip, 30, 160, 120).is_ok();
    #[cfg(not(feature = "gstreamer"))]
    let real = false;
    if !real {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&32u32.to_be_bytes());
        bytes.extend_from_slice(b"ftypisom");
        bytes.extend_from_slice(&[0, 0, 2, 0]);
        bytes.extend_from_slice(b"isomiso2mp41");
        bytes.extend_from_slice(b"\0\0\0\x08free");
        fs::write(&clip, bytes).unwrap_or_else(|e| panic!("{e}"));
    }
    let mut d = Driver::new(
        dir.path(),
        vec![
            Arc::new(ext_preview_media::MediaPreview::new()),
            Arc::new(ext_vfs_archive::ArchiveExtension::new()),
        ],
    );
    let tabs = d.tabs();

    // Selecting it shows it in the preview pane.
    let events = d.select("clip.mp4");
    let doc = events.into_iter().find_map(|e| match e {
        Event::PreviewReady { doc, .. } => Some(doc),
        _ => None,
    });
    let mut doc = doc.unwrap_or_else(|| panic!("the media previewer did not open the clip"));
    assert!(
        doc.status().iter().any(|s| s.ends_with("video")),
        "a video: {:?}",
        doc.status()
    );
    if real {
        assert!(doc.transport().is_some(), "the transport for the toolbar");
    }

    // Enter: the system player, nothing else.
    let before = d.app.model().preview_tab(d.app.model().focused_window);
    d.open("clip.mp4");
    assert_eq!(d.fake.opened(), vec![clip]);
    assert_eq!(d.tabs(), tabs, "no tab appeared");
    assert_eq!(d.tab().kind, TabKind::Explorer);
    assert_eq!(
        d.app.model().preview_tab(d.app.model().focused_window),
        before,
        "the preview tab is as it was"
    );
    assert!(d.app.model().windows().count() == 1, "no window appeared");
}
