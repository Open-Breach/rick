//! Acceptance (issue 20): the archive and trash providers as plugins.
//!
//! A 10 000-entry zip lists its first batch within 50 ms; a tar.zst opens
//! a file inside for the preview; adding a file to a zip rewrites it and
//! the new entry lists; RAR is extract-only; an ISO lists and previews a
//! file; a folder copied out of a 7z to disk round-trips byte-identical;
//! the Trash location lists trashed items, restores one and empties; both
//! plugins load through the host like the Git plugin (the bundle's
//! `--list-plugins` is checked by CI).

use std::fs::{self, File};
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use app::{App, Command, Config, Event, TransferMode};
use ext::{
    ActionId, Capabilities, Frame, Invocation, Level, ListOptions, Provider, RenderRequest, UiHost,
    WriteOptions,
};
use ext_vfs_archive::ArchiveProvider;
use futures::StreamExt;
use model::SelectionChange;
use platform::{ConfigDir, Fake, Trash as _};
use plugins_native::{Host, Library, MANIFEST_FILE, Options, PluginStatus, Source};
use tests::local;
use types::{CancelToken, Entry, EntryId, Error, Location, ProviderId};

/// An archive location: the archive's path with `inner` below it.
fn inside(archive: &Path, inner: &str) -> Location {
    let mut path = platform::paths::from_path(archive);
    if !inner.is_empty() {
        path = path.join(inner.as_bytes());
    }
    Location::new(ProviderId::ARCHIVE, path)
}

/// A UI whose answers the script sets in advance.
#[derive(Default)]
struct Scripted {
    confirm: Mutex<bool>,
}

impl UiHost for Scripted {
    fn notify(&self, _level: Level, _text: &str) {}
    fn confirm(&self, _title: &str, _text: &str) -> bool {
        *self.confirm.lock().unwrap_or_else(PoisonError::into_inner)
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
    ui: Arc<Scripted>,
    events: Vec<Event>,
    pane: model::PaneId,
}

impl Driver {
    fn new(start: &Path, extensions: Vec<Arc<dyn ext::Extension>>) -> Self {
        let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
        let ui = Arc::new(Scripted::default());
        let app = App::new(Config {
            platform: fake.clone(),
            ui: ui.clone(),
            extensions,
            config_dir: Some(fake.config_dir()),
            start: Some(local(start)),
        });
        let pane = app.model().focused;
        let mut driver = Self {
            app,
            fake,
            ui,
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

    fn run(&mut self, action: &'static str) -> Vec<Event> {
        let ctx = self
            .app
            .action_ctx(self.pane, Invocation::Shortcut)
            .unwrap_or_else(|| panic!("no active tab"));
        self.step(Command::Run {
            action: ActionId(action),
            ctx,
        })
    }

    /// Waits until nothing has arrived for a while and nothing runs.
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

    fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.tab().listing.rows().map(|e| e.name.clone()).collect();
        names.sort();
        names
    }

    fn id_of(&self, name: &str) -> EntryId {
        self.tab()
            .listing
            .rows()
            .find(|e| e.name == name)
            .map_or_else(|| panic!("no entry {name} in {:?}", self.names()), |e| e.id)
    }

    fn select(&mut self, name: &str) -> Vec<Event> {
        let id = self.id_of(name);
        self.step(Command::Select {
            pane: self.pane,
            change: SelectionChange::Set(vec![id]),
        })
    }

    /// The text the preview rendered for the selected file.
    fn preview_text(&mut self, name: &str) -> String {
        let events = self.select(name);
        let doc = events.into_iter().find_map(|e| match e {
            Event::PreviewReady { doc, .. } => Some(doc),
            _ => None,
        });
        let mut doc = doc.unwrap_or_else(|| panic!("no preview of {name}"));
        let frame = doc
            .render(
                0,
                RenderRequest {
                    width: 400,
                    height: 300,
                    scale: 1.0,
                    dark: false,
                },
            )
            .unwrap_or_else(|e| panic!("{e}"));
        match frame {
            Frame::Text(block) => block.text,
            other => panic!("not text: {other:?}"),
        }
    }
}

fn list(provider: &dyn Provider, at: &Location) -> Vec<Entry> {
    runtime::block_on(async {
        let mut stream = provider.list(at, ListOptions::default(), CancelToken::new());
        let mut out = Vec::new();
        while let Some(batch) = stream.next().await {
            out.extend(batch.unwrap_or_else(|e| panic!("{at}: {e}")));
        }
        out
    })
}

/// A zip of `count` small files spread over a hundred folders.
fn big_zip(path: &Path, count: usize) {
    let mut writer = zip::ZipWriter::new(File::create(path).unwrap_or_else(|e| panic!("{e}")));
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for i in 0..count {
        writer
            .start_file(format!("folder{:03}/file{i:05}.txt", i % 100), options)
            .unwrap_or_else(|e| panic!("{e}"));
        writer
            .write_all(format!("entry {i}").as_bytes())
            .unwrap_or_else(|e| panic!("{e}"));
    }
    writer.finish().unwrap_or_else(|e| panic!("{e}"));
}

/// A tar.zst holding `docs/readme.txt`.
fn tar_zst(path: &Path) {
    let file = File::create(path).unwrap_or_else(|e| panic!("{e}"));
    let encoder = zstd::stream::write::Encoder::new(file, 3).unwrap_or_else(|e| panic!("{e}"));
    let mut builder = tar::Builder::new(encoder);
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Regular);
    header.set_mode(0o644);
    header.set_mtime(1_700_000_000);
    let text = b"hello from inside the tar.zst\n";
    header.set_size(text.len() as u64);
    builder
        .append_data(&mut header, "docs/readme.txt", &text[..])
        .unwrap_or_else(|e| panic!("{e}"));
    let encoder = builder.into_inner().unwrap_or_else(|e| panic!("{e}"));
    encoder.finish().unwrap_or_else(|e| panic!("{e}"));
}

/// A 7z holding a `data` folder of files of several sizes, including an
/// empty one and a nested folder.
fn sevenz_with_folder(path: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut seed: u32 = 7;
    for (name, len) in [
        ("data/empty.bin", 0usize),
        ("data/small.bin", 17),
        ("data/medium.bin", 70_000),
        ("data/nested/deep.bin", 4_096),
        ("data/nested/text.txt", 1_000),
    ] {
        let bytes: Vec<u8> = (0..len)
            .map(|_| {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                ((seed >> 16) & 0xff).to_le_bytes()[0]
            })
            .collect();
        files.push((name.to_owned(), bytes));
    }
    let mut writer = sevenz_rust2::ArchiveWriter::create(path).unwrap_or_else(|e| panic!("{e}"));
    writer
        .push_archive_entry(
            sevenz_rust2::ArchiveEntry::new_directory("data"),
            None::<File>,
        )
        .unwrap_or_else(|e| panic!("{e}"));
    writer
        .push_archive_entry(
            sevenz_rust2::ArchiveEntry::new_directory("data/nested"),
            None::<File>,
        )
        .unwrap_or_else(|e| panic!("{e}"));
    for (name, bytes) in &files {
        writer
            .push_archive_entry(
                sevenz_rust2::ArchiveEntry::new_file(name),
                Some(Cursor::new(bytes.clone())),
            )
            .unwrap_or_else(|e| panic!("{e}"));
    }
    writer.finish().unwrap_or_else(|e| panic!("{e}"));
    files
}

/// An ISO 9660 image with Rock Ridge names holding `docs/notes.txt`.
fn iso(path: &Path) {
    use hadris_fs::{Content, Node, Tree};
    let mut tree = Tree::new();
    tree.insert("docs", Node::dir())
        .unwrap_or_else(|e| panic!("{e}"));
    tree.insert(
        "docs/notes.txt",
        Node::file(Content::bytes("hello from the disc\n")),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    tree.insert("README", Node::file(Content::bytes("top")))
        .unwrap_or_else(|e| panic!("{e}"));
    let mut image: Vec<u8> = Vec::new();
    hadris_iso::sync::write(
        &mut image,
        &tree,
        &hadris_iso::IsoOptions::new()
            .with_rock_ridge()
            .with_joliet(),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    fs::write(path, image).unwrap_or_else(|e| panic!("{e}"));
}

fn archive_and_text() -> Vec<Arc<dyn ext::Extension>> {
    vec![
        Arc::new(ext_preview_text::TextPreview::new()),
        Arc::new(ext_vfs_archive::ArchiveExtension::new()),
    ]
}

#[test]
#[ignore = "timing; run with --ignored in release"]
fn a_10_000_entry_zip_lists_its_first_batch_within_50_ms() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let path = dir.path().join("big.zip");
    big_zip(&path, 10_000);
    let mut samples = Vec::new();
    for _ in 0..tests::bench::runs() {
        // A fresh provider: the index is read, not cached.
        let provider = ArchiveProvider::new();
        let started = Instant::now();
        let first = runtime::block_on(async {
            let mut stream = provider.list(
                &inside(&path, ""),
                ListOptions::default(),
                CancelToken::new(),
            );
            stream.next().await
        });
        samples.push(started.elapsed());
        let first = first
            .unwrap_or_else(|| panic!("no batch"))
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(first.len(), 100, "the hundred folders");
    }
    tests::bench::record("archive-zip-first", &samples);
}

#[test]
fn a_tar_zst_opens_a_file_inside_for_preview() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let path = dir.path().join("bundle.tar.zst");
    tar_zst(&path);
    let mut d = Driver::new(dir.path(), archive_and_text());
    // Opening the archive from the folder listing enters it as a location.
    let id = d.id_of("bundle.tar.zst");
    d.step(Command::Open { pane: d.pane, id });
    assert_eq!(d.tab().location, inside(&path, ""));
    assert_eq!(d.names(), ["docs"]);
    let docs = d.id_of("docs");
    d.step(Command::Open {
        pane: d.pane,
        id: docs,
    });
    assert_eq!(d.tab().location, inside(&path, "docs"));
    assert_eq!(d.names(), ["readme.txt"]);
    let text = d.preview_text("readme.txt");
    assert_eq!(text, "hello from inside the tar.zst\n");
    // A plain file is no location: the text previewer claims it, so it opens
    // in a viewer tab of the pane (issue 17), not with the system default,
    // which takes the files no previewer claims.
    d.step(Command::Navigate {
        pane: d.pane,
        to: local(dir.path()),
    });
    fs::write(dir.path().join("plain.txt"), "plain").unwrap_or_else(|e| panic!("{e}"));
    d.step(Command::Refresh(d.pane));
    let plain = d.id_of("plain.txt");
    d.step(Command::Open {
        pane: d.pane,
        id: plain,
    });
    assert_eq!(d.tab().location, local(dir.path()));
    assert_eq!(d.tab().kind, model::TabKind::Viewer, "a viewer tab opened");
    assert_eq!(
        d.fake.opened(),
        Vec::<PathBuf>::new(),
        "nothing opened outside"
    );
}

#[test]
fn adding_a_file_to_a_zip_rewrites_it_and_the_new_entry_lists() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let path = dir.path().join("small.zip");
    big_zip(&path, 3);
    let before = fs::metadata(&path).map_or_else(|e| panic!("{e}"), |m| m.len());
    let new_file = dir.path().join("new.txt");
    fs::write(&new_file, "the new entry\n").unwrap_or_else(|e| panic!("{e}"));
    let mut d = Driver::new(dir.path(), archive_and_text());
    // Dropped from outside, as a copy into the archive's root.
    d.step(Command::Receive {
        items: vec![local(&new_file)],
        to: inside(&path, ""),
        mode: TransferMode::Copy,
    });
    assert_ne!(
        fs::metadata(&path).map(|m| m.len()).ok(),
        Some(before),
        "the archive was rewritten"
    );
    d.step(Command::Navigate {
        pane: d.pane,
        to: inside(&path, ""),
    });
    assert_eq!(
        d.names(),
        ["folder000", "folder001", "folder002", "new.txt"]
    );
    assert_eq!(d.preview_text("new.txt"), "the new entry\n");
    // New folder and Rename work in the zip too: the provider says WRITE.
    let ctx = d
        .app
        .action_ctx(d.pane, Invocation::ContextMenu)
        .unwrap_or_else(|| panic!("ctx"));
    let provider = d
        .app
        .mounts()
        .resolve(&ctx.location)
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(
        provider
            .capabilities(&ctx.location)
            .contains(Capabilities::WRITE)
    );
    d.run("core.new_folder");
    assert!(
        d.names().contains(&"New folder".to_owned()),
        "{:?}",
        d.names()
    );
}

#[test]
fn rar_is_extract_only() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../ext-vfs-archive/fixtures/version.rar");
    let path = dir.path().join("version.rar");
    fs::copy(&fixture, &path).unwrap_or_else(|e| panic!("{e}"));
    let mounts = vfs::Mounts::new();
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    mounts.mount(Arc::new(vfs_local::LocalProvider::new(fake)));
    let archive = Arc::new(ArchiveProvider::new());
    mounts.mount(archive.clone());
    let entries = list(&*archive, &inside(&path, ""));
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "VERSION");
    assert_eq!(
        archive.capabilities(&inside(&path, "")),
        Capabilities::empty()
    );
    // Out to disk through the kernel's streamed copy.
    let out = dir.path().join("VERSION");
    let (sink, _rx) = runtime::ProgressSink::channel();
    runtime::block_on(vfs::copy(
        &mounts,
        &inside(&path, "VERSION"),
        &local(&out),
        ext::CopyOptions::default(),
        sink,
        CancelToken::new(),
        &runtime::PauseGate::open(),
    ))
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(fs::read(&out).ok(), Some(b"unrar-0.4.0".to_vec()));
    // Nothing goes in.
    assert!(matches!(
        runtime::block_on(archive.open_write(&inside(&path, "new"), WriteOptions::default())),
        Err(Error::Unsupported(_))
    ));
}

#[test]
fn an_iso_lists_and_previews_a_file() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let path = dir.path().join("disc.iso");
    iso(&path);
    let mut d = Driver::new(dir.path(), archive_and_text());
    let id = d.id_of("disc.iso");
    d.step(Command::Open { pane: d.pane, id });
    assert_eq!(d.names(), ["README", "docs"]);
    d.step(Command::Navigate {
        pane: d.pane,
        to: inside(&path, "docs"),
    });
    assert_eq!(d.names(), ["notes.txt"]);
    assert_eq!(d.preview_text("notes.txt"), "hello from the disc\n");
}

#[test]
fn a_folder_copied_out_of_a_7z_round_trips_byte_identical() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let path = dir.path().join("data.7z");
    let files = sevenz_with_folder(&path);
    let mounts = vfs::Mounts::new();
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    mounts.mount(Arc::new(vfs_local::LocalProvider::new(fake)));
    mounts.mount(Arc::new(ArchiveProvider::new()));
    let target = dir.path().join("out");
    let (sink, _rx) = runtime::ProgressSink::channel();
    runtime::block_on(vfs::copy(
        &mounts,
        &inside(&path, "data"),
        &local(&target),
        ext::CopyOptions::default(),
        sink,
        CancelToken::new(),
        &runtime::PauseGate::open(),
    ))
    .unwrap_or_else(|e| panic!("{e}"));
    for (name, bytes) in &files {
        let rel = name.strip_prefix("data/").unwrap_or(name);
        let copied = fs::read(target.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"));
        assert_eq!(&copied, bytes, "{rel}");
    }
    assert!(target.join("nested").is_dir());
}

#[test]
fn the_trash_location_lists_restores_and_empties() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    for name in ["a.txt", "b.txt"] {
        fs::write(dir.path().join(name), name).unwrap_or_else(|e| panic!("{e}"));
    }
    let mut d = Driver::new(
        dir.path(),
        vec![Arc::new(ext_vfs_trash::TrashExtension::new())],
    );
    d.select("b.txt");
    d.run("core.trash");
    assert!(!dir.path().join("b.txt").exists());
    assert_eq!(d.fake.list().map(|l| l.len()).ok(), Some(1));

    d.step(Command::Navigate {
        pane: d.pane,
        to: ext_vfs_trash::location(),
    });
    assert_eq!(d.names(), ["b.txt"]);
    d.select("b.txt");
    d.run("trash.restore");
    assert!(dir.path().join("b.txt").exists(), "restored");
    assert_eq!(d.names(), [] as [String; 0]);
    assert_eq!(d.app.undo_label().as_deref(), Some("Restore b.txt"));

    // Trash it again from the folder, then empty from the view.
    d.step(Command::Navigate {
        pane: d.pane,
        to: local(dir.path()),
    });
    d.select("b.txt");
    d.run("core.trash");
    d.step(Command::Navigate {
        pane: d.pane,
        to: ext_vfs_trash::location(),
    });
    assert_eq!(d.names(), ["b.txt"]);
    *d.ui.confirm.lock().unwrap_or_else(PoisonError::into_inner) = true;
    d.run("trash.empty");
    assert_eq!(d.fake.list().map(|l| l.len()).ok(), Some(0));
    assert_eq!(d.names(), [] as [String; 0]);
}

/// The library file of the crate `stem` beside the test binary.
fn library_of(stem: &str) -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_else(|e| panic!("{e}"));
    let path = exe
        .parent()
        .unwrap_or_else(|| panic!("no parent"))
        .join(format!(
            "{}{stem}{}",
            std::env::consts::DLL_PREFIX,
            std::env::consts::DLL_SUFFIX
        ));
    assert!(path.is_file(), "not built: {}", path.display());
    path
}

/// Installs the plugin `stem` into `plugins_dir` as the bundler would.
fn install(plugins_dir: &Path, stem: &str) {
    let library = library_of(stem);
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

#[test]
fn both_plugins_load_through_the_host() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let plugins = dir.path().join("plugins");
    install(&plugins, "ext_vfs_trash");
    install(&plugins, "ext_vfs_archive");
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Host::load(&Options {
        dirs: vec![(Source::BesideBinary, plugins)],
        platform: Some(fake.clone()),
        ..Options::default()
    });
    for (id, name, points) in [
        (
            "vfs-trash",
            "Trash",
            ext::Points::PROVIDER | ext::Points::DECORATOR | ext::Points::ACTION,
        ),
        ("vfs-archive", "Archives", ext::Points::PROVIDER),
    ] {
        let row = host
            .plugins()
            .iter()
            .find(|p| p.id == id)
            .unwrap_or_else(|| panic!("no {id} in {:?}", host.plugins()));
        assert_eq!(row.status, PluginStatus::Loaded, "{id}: {:?}", row.status);
        assert_eq!(row.name, name);
        assert_eq!(row.points, points);
    }
    // Registered into an app, the view and the provider are mounted and
    // the trash plugin uses the platform the host lent.
    let ui = Arc::new(Scripted::default());
    let app = App::new(Config {
        platform: fake.clone(),
        ui,
        extensions: host.extensions(),
        config_dir: Some(fake.config_dir()),
        start: Some(local(dir.path())),
    });
    assert!(
        app.registry()
            .views()
            .iter()
            .any(|(name, _)| name == "trash"),
        "the trash view is registered for the rick provider to serve"
    );
    assert!(app.mounts().provider(ProviderId::ARCHIVE).is_some());
    assert!(
        app.registry()
            .actions()
            .iter()
            .any(|a| a.id() == ActionId("trash.empty"))
    );
    drop(app);
    drop(host);
}
