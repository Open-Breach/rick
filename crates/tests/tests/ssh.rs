//! Acceptance (issue 34): the SSH plugin through the application, against
//! the SSH server in the test process (`ext_ssh::fake_sshd`), so every OS
//! runs it; the `gnu` and `busybox` containers and the bastion network of
//! CI are the plugin's own tests.
//!
//! Hosts from `~/.ssh/config` appear in Network without any setup and Rick
//! hosts are added, edited and removed through the commands the host
//! editor sends, the password in the keychain and never in `rick.toml`; a
//! second tab on the same host asks nothing again; a same-host move
//! produces no bytes through `open_read` while a copy to the local disk
//! streams with progress and cancels cleanly; a large text file previews
//! at once as a prefix with *Load more* and a large image waits behind the
//! cap until *Preview anyway*; the plugin loads through the host, and
//! without it the Network section is empty.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use app::{App, Command, Config, ConnectionOp, Event, TransferMode};
use ext::platform::{ConfigDir as _, Fake, Secrets as _};
use ext::{
    API_VERSION, Capabilities, Extension, Manifest, Platforms, Points, Prompts, Provider, Registry,
    async_trait,
};
use ext_ssh::fake_sshd::{FakeSshd, Options};
use ext_ssh::{Hosts, Pool, SshConnector, SshProvider};
use model::{JobState, SelectionChange};
use plugins_native::{Host, Library, MANIFEST_FILE, Options as HostOptions, Source};
use tests::local;
use types::{Answer, EntryId, Error, Location, ProviderId, Question, QuestionKind};

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

/// *Trust and save* every host key; no secret is typed.
fn trust_and_save(q: &Question) -> Answer {
    match &q.kind {
        QuestionKind::Choice { .. } if q.title == "Unknown host key" => Answer::Choice(0),
        _ => Answer::Cancelled,
    }
}

/// An application over `fake` with `extensions` besides the core, driven
/// without a window; the questions the plugin asks arrive as
/// [`Event::Question`] and are answered by `answer_with` through
/// [`Command::AnswerQuestion`], the way the dialogue answers them.
struct Driver {
    app: App,
    events: Vec<Event>,
    asked: Vec<Question>,
    answer_with: Box<dyn Fn(&Question) -> Answer>,
}

impl Driver {
    fn new(
        fake: &Arc<Fake>,
        start: &Path,
        extensions: Vec<Arc<dyn Extension>>,
        answer_with: impl Fn(&Question) -> Answer + 'static,
    ) -> Self {
        let app = App::with_prompts(
            Config {
                platform: fake.clone(),
                ui: Arc::new(Scripted),
                extensions,
                config_dir: Some(fake.config_dir()),
                start: Some(local(start)),
            },
            Prompts::new(),
        );
        let mut driver = Self {
            app,
            events: Vec::new(),
            asked: Vec::new(),
            answer_with: Box::new(answer_with),
        };
        driver.settle();
        driver
    }

    fn send(&mut self, command: Command) {
        self.app.handle(command, &mut self.events);
        self.answer();
    }

    /// Answers the questions that arrived as events, as the dialogue would.
    fn answer(&mut self) {
        let questions: Vec<Question> = self
            .events
            .iter()
            .filter_map(|e| match e {
                Event::Question(q) => Some(q.clone()),
                _ => None,
            })
            .filter(|q| {
                self.app
                    .model()
                    .questions
                    .iter()
                    .any(|open| open.id == q.id)
            })
            .collect();
        for question in questions {
            let answer = (self.answer_with)(&question);
            self.asked.push(question.clone());
            self.app.handle(
                Command::AnswerQuestion {
                    id: question.id,
                    answer,
                },
                &mut self.events,
            );
        }
    }

    /// Waits until nothing has arrived for a while and nothing runs.
    fn settle(&mut self) {
        let deadline = Instant::now() + Duration::from_mins(1);
        loop {
            let quiet = runtime::block_on(async {
                tokio::time::timeout(Duration::from_millis(100), self.app.tick(&mut self.events))
                    .await
                    .is_err()
            });
            self.answer();
            if quiet && !self.app.is_busy() {
                break;
            }
            assert!(Instant::now() < deadline, "did not settle: {:?}", self.app);
        }
    }

    /// Runs the loop until `done`, within `limit`.
    fn run_until(&mut self, limit: Duration, mut done: impl FnMut(&App, &[Event]) -> bool) -> bool {
        let deadline = Instant::now() + limit;
        while Instant::now() < deadline {
            runtime::block_on(async {
                let _ = tokio::time::timeout(
                    Duration::from_millis(50),
                    self.app.tick(&mut self.events),
                )
                .await;
            });
            self.answer();
            if done(&self.app, &self.events) {
                return true;
            }
        }
        done(&self.app, &self.events)
    }

    fn questions(&self) -> Vec<Question> {
        self.asked.clone()
    }

    fn network_names(&self) -> Vec<String> {
        self.app
            .model()
            .network
            .iter()
            .map(|e| e.name.clone())
            .collect()
    }

    fn pane(&self) -> model::PaneId {
        self.app.model().focused
    }

    fn id_of(&self, name: &str) -> EntryId {
        self.app
            .model()
            .active_tab(self.pane())
            .and_then(|(_, t)| t.listing.rows().find(|e| e.name == name).map(|e| e.id))
            .unwrap_or_else(|| panic!("no entry {name} in {:?}", self.names()))
    }

    fn names(&self) -> Vec<String> {
        self.app
            .model()
            .active_tab(self.pane())
            .map(|(_, t)| t.listing.rows().map(|e| e.name.clone()).collect())
            .unwrap_or_default()
    }

    fn navigate(&mut self, to: Location) {
        let pane = self.pane();
        self.send(Command::Navigate { pane, to });
        self.settle();
    }
}

const SSH: ProviderId = ProviderId("ssh");

fn at(alias: &str, path: &str) -> Location {
    ext_ssh::location(alias, path.as_bytes())
}

fn fake_with_ssh_dir() -> Arc<Fake> {
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    fs::create_dir_all(fake.home_dir().join(".ssh")).unwrap_or_else(|e| panic!("{e}"));
    fake
}

/// A "remote home" with a folder of files and a file to download.
fn remote_home() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir(home.path().join("photos")).unwrap_or_else(|e| panic!("{e}"));
    fs::write(home.path().join("photos").join("a.jpg"), vec![1u8; 300_000])
        .unwrap_or_else(|e| panic!("{e}"));
    fs::write(
        home.path().join("photos").join("b c.raw"),
        vec![2u8; 200_000],
    )
    .unwrap_or_else(|e| panic!("{e}"));
    fs::write(home.path().join("notes.txt"), b"hello").unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir(home.path().join("archive")).unwrap_or_else(|e| panic!("{e}"));
    home
}

fn start_server(home: &Path) -> FakeSshd {
    runtime::block_on(FakeSshd::start(
        Options {
            home: home.to_path_buf(),
            ..Options::default()
        },
        21,
    ))
    .unwrap_or_else(|e| panic!("{e}"))
}

fn values(port: u16, extra: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut values = vec![
        ("host".to_owned(), "127.0.0.1".to_owned()),
        ("port".to_owned(), port.to_string()),
        ("user".to_owned(), "rick".to_owned()),
        ("password".to_owned(), "rickpass".to_owned()),
    ];
    values.extend(
        extra
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned())),
    );
    values
}

#[test]
// One scenario start to finish; splitting it would repeat the server.
#[allow(clippy::too_many_lines)]
fn config_hosts_appear_in_network_and_rick_hosts_are_added_edited_and_removed() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let home = remote_home();
    let server = start_server(home.path());
    let fake = fake_with_ssh_dir();
    fs::write(
        fake.home_dir().join(".ssh").join("config"),
        "Host nas\n  HostName 192.0.2.1\n  User serge\nHost *.example.com\n  User ops\n",
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let mut d = Driver::new(
        &fake,
        dir.path(),
        vec![Arc::new(ext_ssh::SshExtension::new())],
        trust_and_save,
    );
    assert_eq!(d.app.model().network_providers, [SSH]);
    assert_eq!(
        d.network_names(),
        ["nas"],
        "the config host shows up without any setup; the wildcard does not"
    );

    // Add a Rick host: the password goes to the keychain, the rest to
    // rick.toml, and the row appears.
    d.send(Command::AddConnection {
        provider: SSH,
        name: "box".into(),
        values: values(
            server.port,
            &[("default_dir", &home.path().to_string_lossy())],
        ),
    });
    assert!(d.run_until(Duration::from_secs(10), |app, _| {
        app.model().network.iter().any(|e| e.name == "box")
    }));
    d.settle();
    assert_eq!(d.network_names(), ["box", "nas"]);
    let settings = fs::read_to_string(fake.config_dir().join(app::SETTINGS_FILE))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(settings.contains("[[connections]]"), "{settings}");
    assert!(settings.contains("provider = \"ssh\""), "{settings}");
    assert!(settings.contains("name = \"box\""), "{settings}");
    assert!(
        !settings.contains("rickpass"),
        "the secret stays out: {settings}"
    );
    assert_eq!(
        fake.secrets().get("ssh/box/password").map(String::as_str),
        Some("rickpass")
    );
    // The editor's jump-host choice names the hosts known now.
    let connector = d
        .app
        .registry()
        .connector_of(SSH)
        .cloned()
        .unwrap_or_else(|| panic!("connector"));
    let schema = connector.schema();
    assert!(matches!(
        schema.field("jump").map(|f| &f.kind),
        Some(ext::ConnectionFieldKind::Choice { options, .. }) if options == &["", "box", "nas"]
    ));

    // Browsing the host: the row's location is its default directory.
    d.navigate(at("box", ""));
    let mut names = d.names();
    names.sort();
    assert_eq!(names, ["archive", "notes.txt", "photos"]);
    assert_eq!(d.questions().len(), 1, "one host-key question");
    assert!(connector.is_connected("box"));
    // A second tab on the same host asks nothing again.
    d.send(Command::NewTab(at("box", "")));
    d.settle();
    assert_eq!(d.questions().len(), 1);

    // Edit: another default directory, the password left alone.
    d.send(Command::EditConnection {
        provider: SSH,
        name: "box".into(),
        values: vec![
            ("host".into(), "127.0.0.1".into()),
            ("port".into(), server.port.to_string()),
            ("user".into(), "rick".into()),
            (
                "default_dir".into(),
                home.path().join("photos").to_string_lossy().into_owned(),
            ),
        ],
    });
    d.settle();
    let settings = fs::read_to_string(fake.config_dir().join(app::SETTINGS_FILE))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(settings.contains("photos"), "{settings}");
    assert_eq!(
        fake.secrets().get("ssh/box/password").map(String::as_str),
        Some("rickpass"),
        "an edit without a password keeps the keychain's"
    );
    d.navigate(at("box", ""));
    let mut names = d.names();
    names.sort();
    assert_eq!(names, ["a.jpg", "b c.raw"], "the edited default directory");

    // Remove: the row, the table and the keychain entries go.
    d.send(Command::Connection {
        provider: SSH,
        name: "box".into(),
        op: ConnectionOp::Remove,
    });
    d.settle();
    assert_eq!(d.network_names(), ["nas"]);
    let settings = fs::read_to_string(fake.config_dir().join(app::SETTINGS_FILE))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(!settings.contains("[[connections]]"), "{settings}");
    assert!(!fake.secrets().contains_key("ssh/box/password"));
    assert!(!connector.is_connected("box"));
}

/// A provider that counts the bytes read through `open_read`, around the
/// SSH provider.
struct Counting {
    inner: Arc<dyn Provider>,
    read: Arc<AtomicU64>,
}

impl std::fmt::Debug for Counting {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Counting")
            .field("read", &self.read.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

/// A reader that adds what it reads to a counter.
struct Counted {
    inner: ext::ReadHandle,
    read: Arc<AtomicU64>,
}

impl tokio::io::AsyncRead for Counted {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let poll = std::pin::Pin::new(&mut self.inner).poll_read(cx, buf);
        if poll.is_ready() {
            let read = (buf.filled().len() - before) as u64;
            self.read.fetch_add(read, Ordering::Relaxed);
        }
        poll
    }
}

#[async_trait]
impl Provider for Counting {
    fn id(&self) -> ProviderId {
        self.inner.id()
    }
    fn capabilities(&self, loc: &Location) -> Capabilities {
        self.inner.capabilities(loc)
    }
    fn list(
        &self,
        loc: &Location,
        opts: ext::ListOptions,
        cancel: types::CancelToken,
    ) -> ext::EntryStream {
        self.inner.list(loc, opts, cancel)
    }
    async fn stat(&self, loc: &Location) -> Result<types::Entry, Error> {
        self.inner.stat(loc).await
    }
    async fn open_read(&self, loc: &Location) -> Result<ext::ReadHandle, Error> {
        let inner = self.inner.open_read(loc).await?;
        Ok(Box::new(Counted {
            inner,
            read: Arc::clone(&self.read),
        }))
    }
    async fn open_read_at(&self, loc: &Location, offset: u64) -> Result<ext::ReadHandle, Error> {
        let inner = self.inner.open_read_at(loc, offset).await?;
        Ok(Box::new(Counted {
            inner,
            read: Arc::clone(&self.read),
        }))
    }
    async fn open_write(
        &self,
        loc: &Location,
        opts: ext::WriteOptions,
    ) -> Result<ext::WriteHandle, Error> {
        self.inner.open_write(loc, opts).await
    }
    async fn create_dir(&self, loc: &Location) -> Result<types::Entry, Error> {
        self.inner.create_dir(loc).await
    }
    async fn rename(&self, from: &Location, to: &Location) -> Result<types::Entry, Error> {
        self.inner.rename(from, to).await
    }
    async fn remove(&self, loc: &Location, mode: ext::RemoveMode) -> Result<(), Error> {
        self.inner.remove(loc, mode).await
    }
    async fn link(
        &self,
        target: &Location,
        at: &Location,
        kind: ext::LinkKind,
    ) -> Result<types::Entry, Error> {
        self.inner.link(target, at, kind).await
    }
    async fn copy(
        &self,
        from: &Location,
        to: &Location,
        opts: ext::CopyOptions,
        progress: ext::ProgressSink,
        cancel: types::CancelToken,
    ) -> Result<(), Error> {
        self.inner.copy(from, to, opts, progress, cancel).await
    }
    async fn transfer(
        &self,
        items: &[Location],
        to: &Location,
        mode: ext::TransferMode,
        progress: ext::ProgressSink,
        cancel: types::CancelToken,
    ) -> Result<(), Error> {
        self.inner.transfer(items, to, mode, progress, cancel).await
    }
    fn remote_preview_cap(&self, loc: &Location) -> Option<u64> {
        self.inner.remote_preview_cap(loc)
    }
    fn remote_preview_always(&self, loc: &Location) -> bool {
        self.inner.remote_preview_always(loc)
    }
    fn watch(&self, loc: &Location) -> Result<ext::ChangeStream, Error> {
        self.inner.watch(loc)
    }
}

/// The SSH extension with its provider counted: what the no-bytes
/// assertion of the acceptance looks through.
struct CountedSsh {
    manifest: Manifest,
    read: Arc<AtomicU64>,
}

impl Extension for CountedSsh {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        let platform = reg
            .platform()
            .ok_or(Error::Unsupported("needs the platform"))?
            .clone();
        let hosts = Arc::new(Hosts::new(platform));
        let pool = Pool::new(hosts, reg.prompts().cloned());
        reg.provider(Arc::new(Counting {
            inner: Arc::new(SshProvider::new(Arc::clone(&pool))),
            read: Arc::clone(&self.read),
        }));
        reg.connector(Arc::new(SshConnector::new(pool)));
        Ok(())
    }
}

fn counted_ssh() -> (Arc<AtomicU64>, Arc<dyn Extension>) {
    let read = Arc::new(AtomicU64::new(0));
    let ext: Arc<dyn Extension> = Arc::new(CountedSsh {
        manifest: Manifest {
            id: ext_ssh::ID,
            name: "SSH",
            version: ext::Version::new(0, 0, 0),
            api_version: API_VERSION,
            requires: Capabilities::empty(),
            points: Points::PROVIDER,
            platforms: Platforms::ALL,
            previews: &[],
        },
        read: Arc::clone(&read),
    });
    (read, ext)
}

#[test]
// One scenario start to finish; splitting it would repeat the server.
#[allow(clippy::too_many_lines)]
fn a_same_host_move_streams_nothing_and_a_copy_to_the_local_disk_streams_and_cancels() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let home = remote_home();
    // A big file for the download and its cancel.
    let big = home.path().join("big.bin");
    fs::write(&big, vec![7u8; 48 * 1024 * 1024]).unwrap_or_else(|e| panic!("{e}"));
    let server = start_server(home.path());
    let fake = fake_with_ssh_dir();
    fake.store_secret("ssh/box/password", "rickpass")
        .unwrap_or_else(|e| panic!("{e}"));
    let (read, ext) = counted_ssh();
    let mut d = Driver::new(&fake, dir.path(), vec![ext], trust_and_save);
    d.send(Command::AddConnection {
        provider: SSH,
        name: "box".into(),
        values: {
            let mut v = values(
                server.port,
                &[("default_dir", &home.path().to_string_lossy())],
            );
            v.retain(|(k, _)| k != "password");
            v
        },
    });
    d.settle();
    let home_text = home.path().to_string_lossy().into_owned();

    // A move of a folder and a file on the host: no bytes through the
    // machine, the tree lands under its names.
    d.navigate(at("box", ""));
    read.store(0, Ordering::Relaxed);
    d.send(Command::Receive {
        items: vec![
            at("box", &format!("{home_text}/photos")),
            at("box", &format!("{home_text}/notes.txt")),
        ],
        to: at("box", &format!("{home_text}/archive")),
        mode: TransferMode::Move,
    });
    d.settle();
    assert_eq!(
        read.load(Ordering::Relaxed),
        0,
        "nothing streamed through the machine"
    );
    assert!(
        home.path()
            .join("archive")
            .join("photos")
            .join("b c.raw")
            .is_file()
    );
    assert!(home.path().join("archive").join("notes.txt").is_file());
    assert!(!home.path().join("photos").exists());
    assert!(
        d.app
            .model()
            .jobs
            .values()
            .all(|j| j.state == JobState::Done),
        "{:?}",
        d.app.model().jobs
    );

    // The undo moves back on the host too.
    d.send(Command::Undo);
    d.settle();
    assert_eq!(read.load(Ordering::Relaxed), 0);
    assert!(home.path().join("photos").join("a.jpg").is_file());

    // A copy on the host, through cp -a: still no bytes here.
    d.send(Command::Receive {
        items: vec![at("box", &format!("{home_text}/photos"))],
        to: at("box", &format!("{home_text}/archive")),
        mode: TransferMode::Copy,
    });
    d.settle();
    assert_eq!(read.load(Ordering::Relaxed), 0);
    assert!(
        home.path()
            .join("archive")
            .join("photos")
            .join("a.jpg")
            .is_file()
    );
    assert!(home.path().join("photos").join("a.jpg").is_file());

    // A copy to the local disk streams every byte with progress.
    let down = dir.path().join("down");
    fs::create_dir_all(&down).unwrap_or_else(|e| panic!("{e}"));
    d.send(Command::Receive {
        items: vec![at("box", &format!("{home_text}/photos"))],
        to: local(&down),
        mode: TransferMode::Copy,
    });
    d.settle();
    assert_eq!(
        read.load(Ordering::Relaxed),
        500_000,
        "both files streamed down"
    );
    assert_eq!(
        fs::read(down.join("photos").join("a.jpg"))
            .map(|b| b.len())
            .ok(),
        Some(300_000)
    );
    assert_eq!(
        fs::read(down.join("photos").join("b c.raw"))
            .map(|b| b.len())
            .ok(),
        Some(200_000)
    );
    assert!(
        d.events.iter().any(|e| matches!(
            e,
            Event::JobProgress {
                progress: types::Progress::Bytes { .. },
                ..
            }
        )),
        "progress in bytes was reported"
    );

    // The big file's download, cancelled half-way, ends cancelled and
    // leaves the connection usable.
    d.events.clear();
    d.send(Command::Receive {
        items: vec![at("box", &format!("{home_text}/big.bin"))],
        to: local(&down),
        mode: TransferMode::Copy,
    });
    let started = d.run_until(Duration::from_secs(20), |app, _| {
        app.model()
            .jobs
            .iter()
            .any(|(_, j)| matches!(&j.state, JobState::Running(types::Progress::Bytes { done, .. }) if *done > 0))
    });
    assert!(started, "{:?}", d.app.model().jobs);
    let job = d
        .app
        .model()
        .jobs
        .iter()
        .find(|(_, j)| matches!(j.state, JobState::Running(_)))
        .map_or_else(|| panic!("no running job"), |(id, _)| *id);
    d.send(Command::CancelJob(job));
    d.settle();
    assert_eq!(
        d.app.model().jobs.get(&job).map(|j| j.state.clone()),
        Some(JobState::Cancelled)
    );
    assert!(
        read.load(Ordering::Relaxed) < 500_000 + 48 * 1024 * 1024,
        "the download stopped early"
    );
    // The connection still serves.
    d.navigate(at("box", ""));
    assert!(d.names().contains(&"big.bin".to_owned()));
}

#[test]
// One scenario start to finish; splitting it would repeat the server.
#[allow(clippy::too_many_lines)]
fn previews_over_ssh_serve_a_prefix_with_load_more_and_cap_a_whole_file() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let home = remote_home();
    // A text file well over the first chunk, and a "picture" over the cap.
    let line = "the quick brown fox jumps over the lazy dog, again and again\n";
    let mut text = String::new();
    while text.len() < 3 * 1024 * 1024 {
        text.push_str(line);
    }
    fs::write(home.path().join("big.log"), &text).unwrap_or_else(|e| panic!("{e}"));
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.resize(6 * 1024 * 1024, 0);
    fs::write(home.path().join("huge.png"), &png).unwrap_or_else(|e| panic!("{e}"));
    let server = start_server(home.path());
    let fake = fake_with_ssh_dir();
    fake.store_secret("ssh/box/password", "rickpass")
        .unwrap_or_else(|e| panic!("{e}"));
    let mut d = Driver::new(
        &fake,
        dir.path(),
        vec![
            Arc::new(ext_ssh::SshExtension::new()),
            Arc::new(ext_preview_text::TextPreview::new()),
            Arc::new(ext_preview_image::ImagePreview::new()),
        ],
        trust_and_save,
    );
    d.send(Command::AddConnection {
        provider: SSH,
        name: "box".into(),
        values: {
            let mut v = values(
                server.port,
                &[("default_dir", &home.path().to_string_lossy())],
            );
            v.retain(|(k, _)| k != "password");
            v
        },
    });
    d.settle();
    d.navigate(at("box", ""));
    let pane = d.pane();

    // The text file previews at once, as a prefix with Load more.
    let log = d.id_of("big.log");
    d.events.clear();
    d.send(Command::Select {
        pane,
        change: SelectionChange::Set(vec![log]),
    });
    d.settle();
    assert!(
        d.events
            .iter()
            .any(|e| matches!(e, Event::PreviewReady { pane: p, .. } if *p == pane)),
        "{:?}",
        d.events
    );
    let prefix = d.events.iter().find_map(|e| match e {
        Event::PreviewPrefix { served, total, .. } => Some((*served, *total)),
        _ => None,
    });
    assert_eq!(
        prefix,
        Some((app::REMOTE_PREFIX_CHUNK, Some(text.len() as u64)))
    );
    let doc_size = d.events.iter().find_map(|e| match e {
        Event::PreviewReady { doc, .. } => doc.sections().first().map(|s| s.rows.clone()),
        _ => None,
    });
    assert!(
        doc_size.is_some_and(|rows| rows.iter().any(|(k, v)| k == "Size" && v.contains("3."))),
        "the file's size, not the prefix's"
    );
    // Load more serves the next chunk.
    d.events.clear();
    d.send(Command::PreviewMore(pane));
    d.settle();
    let prefix = d.events.iter().find_map(|e| match e {
        Event::PreviewPrefix { served, .. } => Some(*served),
        _ => None,
    });
    assert_eq!(prefix, Some(2 * app::REMOTE_PREFIX_CHUNK));

    // The picture is above the cap: the facts and Preview anyway, no read.
    let picture = d.id_of("huge.png");
    d.events.clear();
    d.send(Command::Select {
        pane,
        change: SelectionChange::Set(vec![picture]),
    });
    d.settle();
    let capped = d.events.iter().find_map(|e| match e {
        Event::PreviewCapped { size, cap, .. } => Some((*size, *cap)),
        _ => None,
    });
    assert_eq!(capped, Some((6 * 1024 * 1024, app::REMOTE_PREVIEW_CAP)));
    assert!(
        !d.events
            .iter()
            .any(|e| matches!(e, Event::PreviewReady { .. }))
    );
    // Preview anyway opens it; a padded signature is no picture, so the
    // previewer reports that instead of the cap.
    d.events.clear();
    d.send(Command::PreviewAnyway(pane));
    d.settle();
    assert!(
        d.events
            .iter()
            .any(|e| matches!(e, Event::PreviewReady { .. } | Event::PreviewFailed { .. })),
        "{:?}",
        d.events
    );
    assert!(
        !d.events
            .iter()
            .any(|e| matches!(e, Event::PreviewCapped { .. }))
    );
    // A small file on the host previews whole.
    let notes = d.id_of("notes.txt");
    d.events.clear();
    d.send(Command::Select {
        pane,
        change: SelectionChange::Set(vec![notes]),
    });
    d.settle();
    assert!(
        d.events
            .iter()
            .any(|e| matches!(e, Event::PreviewReady { .. }))
    );
    assert!(
        !d.events
            .iter()
            .any(|e| matches!(e, Event::PreviewPrefix { .. }))
    );

    // Issue 50, *Always preview on <host>*: a second host on the same
    // server, so the switch can be told from the cap.
    fake.store_secret("ssh/box2/password", "rickpass")
        .unwrap_or_else(|e| panic!("{e}"));
    d.send(Command::AddConnection {
        provider: SSH,
        name: "box2".into(),
        values: {
            let mut v = values(
                server.port,
                &[("default_dir", &home.path().to_string_lossy())],
            );
            v.retain(|(k, _)| k != "password");
            v
        },
    });
    d.settle();
    // Two more large "pictures": huge.png is forced open on this pane by
    // the Preview anyway above, so the switch is shown on other files.
    let mut other = b"\x89PNG\r\n\x1a\n".to_vec();
    other.resize(5 * 1024 * 1024, 1);
    fs::write(home.path().join("other.png"), &other).unwrap_or_else(|e| panic!("{e}"));
    fs::write(home.path().join("third.png"), &other).unwrap_or_else(|e| panic!("{e}"));
    d.navigate(at("box", ""));
    let pane = d.pane();
    let picture = d.id_of("other.png");
    d.events.clear();
    d.send(Command::Select {
        pane,
        change: SelectionChange::Set(vec![picture]),
    });
    d.settle();
    let source = d.events.iter().find_map(|e| match e {
        Event::PreviewCapped { source, .. } => Some(source.clone()),
        _ => None,
    });
    assert_eq!(
        source,
        Some(Some("box".to_owned())),
        "the cap message names the host: {:?}",
        d.events
    );
    // Always preview on box: the file loads and the host's switch is set.
    d.events.clear();
    d.send(Command::PreviewAlways(pane));
    d.settle();
    assert!(
        d.events
            .iter()
            .any(|e| matches!(e, Event::PreviewReady { .. } | Event::PreviewFailed { .. })),
        "{:?}",
        d.events
    );
    let switch = |d: &Driver, name: &str| {
        d.app
            .model()
            .settings
            .connections
            .iter()
            .find(|c| c.provider == "ssh" && c.name == name)
            .and_then(|c| c.settings.get("preview_always").cloned())
    };
    assert_eq!(switch(&d, "box"), Some("true".to_owned()));
    assert_eq!(switch(&d, "box2"), None);
    let text = fs::read_to_string(fake.config_dir().join(app::SETTINGS_FILE))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(text.contains("preview_always = \"true\""), "{text}");
    // The next large image on box previews without the message.
    let third = d.id_of("third.png");
    d.events.clear();
    d.send(Command::Select {
        pane,
        change: SelectionChange::Set(vec![third]),
    });
    d.settle();
    assert!(
        !d.events
            .iter()
            .any(|e| matches!(e, Event::PreviewCapped { .. })),
        "no cap on box any more: {:?}",
        d.events
    );
    assert!(
        d.events
            .iter()
            .any(|e| matches!(e, Event::PreviewReady { .. } | Event::PreviewFailed { .. }))
    );
    // The same image on the other host still shows the message.
    d.navigate(at("box2", ""));
    let pane = d.pane();
    let picture = d.id_of("huge.png");
    d.events.clear();
    d.send(Command::Select {
        pane,
        change: SelectionChange::Set(vec![picture]),
    });
    d.settle();
    assert!(
        d.events
            .iter()
            .any(|e| matches!(e, Event::PreviewCapped { source: Some(s), .. } if s == "box2")),
        "box2 keeps its cap: {:?}",
        d.events
    );
    // Unticking the switch (the host editor's checkbox, the row's menu)
    // brings the message back on box.
    d.send(Command::SetPreviewAlways {
        provider: SSH,
        name: "box".into(),
        on: false,
    });
    d.settle();
    assert_eq!(switch(&d, "box"), Some("false".to_owned()));
    // A different file first, so the next look at third.png is a fresh
    // preview and not the one already open.
    d.navigate(at("box", ""));
    let pane = d.pane();
    let notes = d.id_of("notes.txt");
    d.send(Command::Select {
        pane,
        change: SelectionChange::Set(vec![notes]),
    });
    d.settle();
    let third = d.id_of("third.png");
    d.events.clear();
    d.send(Command::Select {
        pane,
        change: SelectionChange::Set(vec![third]),
    });
    d.settle();
    assert!(
        d.events
            .iter()
            .any(|e| matches!(e, Event::PreviewCapped { .. })),
        "the cap is back: {:?}",
        d.events
    );
}

/// The directory the test binary runs from, where cargo puts every
/// dependency's `cdylib` under its plain name.
fn library_of(stem: &str) -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_else(|e| panic!("{e}"));
    let path = exe
        .parent()
        .unwrap_or_else(|| panic!("no parent of {}", exe.display()))
        .join(format!(
            "{}{stem}{}",
            std::env::consts::DLL_PREFIX,
            std::env::consts::DLL_SUFFIX
        ));
    assert!(path.is_file(), "not built: {}", path.display());
    path
}

#[test]
fn the_plugin_loads_through_the_host_and_without_it_network_is_empty() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let plugins = dir.path().join("plugins");
    let library = library_of("ext_ssh");
    let mut toml = plugins_native::describe(&library).unwrap_or_else(|e| panic!("{e}"));
    toml.library = Library::for_crate("ext_ssh");
    let plugin_dir = plugins.join(&toml.id);
    toml.write(&plugin_dir.join(MANIFEST_FILE))
        .unwrap_or_else(|e| panic!("{e}"));
    let file = toml
        .library
        .for_os(std::env::consts::OS)
        .unwrap_or_else(|| panic!("no library name for this OS"));
    fs::copy(&library, plugin_dir.join(file)).unwrap_or_else(|e| panic!("{e}"));
    let fake = fake_with_ssh_dir();
    let prompts = Prompts::new();
    let host = Host::load(&HostOptions {
        dirs: vec![(Source::BesideBinary, plugins)],
        platform: Some(fake.clone()),
        prompts: Some(Arc::clone(&prompts)),
        ..HostOptions::default()
    });
    assert_eq!(host.plugins().len(), 1);
    let plugin = &host.plugins()[0];
    assert_eq!(plugin.id, "ssh");
    assert_eq!(
        plugin.status,
        plugins_native::PluginStatus::Loaded,
        "{plugin:?}"
    );
    assert_eq!(plugin.points, Points::PROVIDER | Points::ACTION);
    let mut registry = Registry::with_platform(fake.clone()).with_prompts(prompts);
    for extension in host.extensions() {
        registry.load(&*extension).unwrap_or_else(|e| panic!("{e}"));
    }
    assert_eq!(registry.providers()[0].id(), SSH);
    assert_eq!(
        registry.connector_of(SSH).map(|c| c.schema().title.key()),
        Some("ssh-ssh")
    );
    let mut actions: Vec<&str> = registry.actions().iter().map(|a| a.id().0).collect();
    actions.sort_unstable();
    assert_eq!(actions, ["ssh.console", "ssh.terminal"]);

    // Without the plugin, Network lists nothing and nothing breaks.
    fs::write(
        fake.home_dir().join(".ssh").join("config"),
        "Host nas\n  HostName 192.0.2.1\n",
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let d = Driver::new(&fake, dir.path(), Vec::new(), trust_and_save);
    assert!(
        d.app.model().network_providers.is_empty(),
        "{:?}",
        d.app.model().network_providers
    );
    let names = d.network_names();
    assert!(names.is_empty(), "{names:?}");
    assert!(
        ui_egui::side_panel::sections_shown(&d.app).contains(&model::Section::Network),
        "the section stays, with its Add… row"
    );
}

/// Runs frames until the app has nothing in flight and a quiet moment
/// passed, answering nothing: the dialogues are the test's to click.
fn settle_ui(harness: &mut egui_kittest::Harness<'_, ui_egui::RickUi>) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        for _ in 0..3 {
            harness.step();
        }
        let app = harness.state().app();
        if !app.is_busy() || !app.model().questions.is_empty() {
            std::thread::sleep(Duration::from_millis(50));
            harness.step();
            let app = harness.state().app();
            if !app.is_busy() || !app.model().questions.is_empty() {
                break;
            }
        }
        assert!(Instant::now() < deadline, "did not settle");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn step_until(
    harness: &mut egui_kittest::Harness<'_, ui_egui::RickUi>,
    limit: Duration,
    mut done: impl FnMut(&egui_kittest::Harness<'_, ui_egui::RickUi>) -> bool,
) -> bool {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        harness.step();
        if done(harness) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    done(harness)
}

/// The side panel rows whose label contains `text`: left of the focused
/// pane.
fn panel_rects(
    harness: &egui_kittest::Harness<'_, ui_egui::RickUi>,
    text: &str,
) -> Vec<egui::Rect> {
    use egui_kittest::kittest::Queryable as _;

    let pane = harness.state().app().model().focused;
    let pane_left = harness
        .state()
        .pane_rects()
        .into_iter()
        .find(|(p, _)| *p == pane)
        .map_or(400.0, |(_, r)| r.left());
    let mut rects: Vec<egui::Rect> = harness
        .query_all_by_label_contains(text)
        .map(|n| n.rect())
        .filter(|r| r.center().x < pane_left)
        .collect();
    rects.sort_by(|a, b| a.top().total_cmp(&b.top()));
    rects
}

fn click_at(
    harness: &mut egui_kittest::Harness<'_, ui_egui::RickUi>,
    at: egui::Pos2,
    button: egui::PointerButton,
) {
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(at));
    harness.step();
    for pressed in [true, false] {
        harness.input_mut().events.push(egui::Event::PointerButton {
            pos: at,
            button,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        harness.step();
    }
    harness.step();
}

/// Clicks the button labelled `label` once the frame has settled: a
/// dialogue moves to its place over its first frames.
fn click_label(harness: &mut egui_kittest::Harness<'_, ui_egui::RickUi>, label: &str) {
    use egui_kittest::kittest::Queryable as _;

    harness.step();
    harness.step();
    harness.get_by_label(label).click();
    harness.step();
    harness.step();
}

fn press_escape(harness: &mut egui_kittest::Harness<'_, ui_egui::RickUi>) {
    for pressed in [true, false] {
        harness.input_mut().events.push(egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        harness.step();
    }
    harness.step();
}

#[test]
// One scenario through the panel, start to finish: the rows' menus and
// the host-key dialogue.
#[allow(clippy::too_many_lines)]
fn host_rows_offer_the_ssh_actions_and_the_host_key_dialogue_asks_before_connecting() {
    use egui_kittest::kittest::Queryable as _;

    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let home = remote_home();
    let server = start_server(home.path());
    let fake = fake_with_ssh_dir();
    fs::write(
        fake.home_dir().join(".ssh").join("config"),
        "Host nas\n  HostName 192.0.2.1\n",
    )
    .unwrap_or_else(|e| panic!("{e}"));
    fake.store_secret("ssh/box/password", "rickpass")
        .unwrap_or_else(|e| panic!("{e}"));
    let host = Arc::new(ui_egui::EguiHost::new());
    let app = App::with_prompts(
        Config {
            platform: fake.clone(),
            ui: host.clone(),
            extensions: vec![Arc::new(ext_ssh::SshExtension::new())],
            config_dir: Some(fake.config_dir()),
            start: Some(local(dir.path())),
        },
        Prompts::new(),
    );
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1400.0, 800.0))
        .with_step_dt(0.05)
        .build_eframe(move |cc| ui_egui::RickUi::new(app, host, &cc.egui_ctx));
    settle_ui(&mut harness);
    harness.state_mut().app_mut().handle(
        Command::AddConnection {
            provider: SSH,
            name: "box".into(),
            values: {
                let mut v = values(
                    server.port,
                    &[("default_dir", &home.path().to_string_lossy())],
                );
                v.retain(|(k, _)| k != "password");
                v
            },
        },
        &mut Vec::new(),
    );
    assert!(step_until(&mut harness, Duration::from_secs(10), |h| {
        h.state()
            .app()
            .model()
            .network
            .iter()
            .any(|e| e.name == "box")
    }));
    settle_ui(&mut harness);

    // The Rick host's row: the connection items, Edit… and Remove, and the
    // SSH actions.
    let row = panel_rects(&harness, "box");
    assert!(!row.is_empty(), "{row:?}");
    click_at(
        &mut harness,
        row[0].center(),
        egui::PointerButton::Secondary,
    );
    harness.step();
    for label in [
        "Connect",
        "Edit…",
        "Remove connection",
        "Open console here",
        "Open in external terminal",
    ] {
        assert!(
            harness.query_by_label_contains(label).is_some(),
            "{label} in the host row's menu"
        );
    }
    press_escape(&mut harness);
    assert!(
        harness
            .query_by_label_contains("Open console here")
            .is_none()
    );

    // The config host's row: read-only in Rick, so neither Edit… nor
    // Remove, but the actions.
    let row = panel_rects(&harness, "nas");
    assert!(!row.is_empty(), "{row:?}");
    click_at(
        &mut harness,
        row[0].center(),
        egui::PointerButton::Secondary,
    );
    harness.step();
    assert!(
        harness
            .query_by_label_contains("Open console here")
            .is_some()
    );
    assert!(harness.query_by_label_contains("Edit…").is_none());
    assert!(
        harness
            .query_by_label_contains("Remove connection")
            .is_none()
    );
    press_escape(&mut harness);

    // Clicking the Rick host's row opens it: the unknown host key asks
    // first, in a dialogue, and Trust and save lets the listing through
    // and writes the key to known_hosts.
    let row = panel_rects(&harness, "box");
    click_at(&mut harness, row[0].center(), egui::PointerButton::Primary);
    assert!(
        step_until(&mut harness, Duration::from_secs(10), |h| {
            h.query_by_label_contains("Unknown host key").is_some()
        }),
        "the host-key dialogue"
    );
    assert_eq!(harness.state().app().model().questions.len(), 1);
    for label in ["Trust and save", "Trust once", "Cancel"] {
        assert!(harness.query_by_label(label).is_some(), "{label}");
    }
    click_label(&mut harness, "Trust and save");
    settle_ui(&mut harness);
    assert!(
        harness
            .query_by_label_contains("Unknown host key")
            .is_none()
    );
    let model = harness.state().app().model();
    let (_, tab) = model
        .active_tab(model.focused)
        .unwrap_or_else(|| panic!("a tab"));
    assert_eq!(tab.location, at("box", ""));
    assert!(tab.failure.is_none(), "{:?}", tab.failure);
    let mut names: Vec<String> = tab.listing.rows().map(|e| e.name.clone()).collect();
    names.sort();
    assert_eq!(names, ["archive", "notes.txt", "photos"]);
    let known = fs::read_to_string(fake.home_dir().join(".ssh").join("known_hosts"))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(
        known.contains(&ext_ssh::known_hosts::host_pattern(
            "127.0.0.1",
            server.port
        )),
        "{known}"
    );
}

#[test]
// One scenario start to finish; splitting it would repeat the server.
#[allow(clippy::too_many_lines)]
fn a_connection_that_fails_shows_a_banner_with_retry_not_a_dialogue() {
    use egui_kittest::kittest::Queryable as _;

    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let home = remote_home();
    // A port nothing listens on yet.
    let port = {
        let listener =
            std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap_or_else(|e| panic!("{e}"));
        listener
            .local_addr()
            .unwrap_or_else(|e| panic!("{e}"))
            .port()
    };
    let fake = fake_with_ssh_dir();
    let host = Arc::new(ui_egui::EguiHost::new());
    let app = App::with_prompts(
        Config {
            platform: fake.clone(),
            ui: host.clone(),
            extensions: vec![Arc::new(ext_ssh::SshExtension::new())],
            config_dir: Some(fake.config_dir()),
            start: Some(local(dir.path())),
        },
        Prompts::new(),
    );
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1400.0, 800.0))
        .with_step_dt(0.05)
        .build_eframe(move |cc| ui_egui::RickUi::new(app, host, &cc.egui_ctx));
    settle_ui(&mut harness);
    harness.state_mut().app_mut().handle(
        Command::AddConnection {
            provider: SSH,
            name: "dead".into(),
            values: values(port, &[("default_dir", &home.path().to_string_lossy())]),
        },
        &mut Vec::new(),
    );
    assert!(step_until(&mut harness, Duration::from_secs(10), |h| {
        h.state()
            .app()
            .model()
            .network
            .iter()
            .any(|e| e.name == "dead")
    }));
    settle_ui(&mut harness);

    // Opening the host fails to connect: the tab shows the banner with
    // Retry and no dialogue opened.
    let pane = harness.state().app().model().focused;
    harness.state_mut().app_mut().handle(
        Command::Navigate {
            pane,
            to: at("dead", ""),
        },
        &mut Vec::new(),
    );
    assert!(
        step_until(&mut harness, Duration::from_secs(30), |h| {
            let model = h.state().app().model();
            model
                .active_tab(model.focused)
                .is_some_and(|(_, t)| t.failure.is_some())
        }),
        "the failure"
    );
    settle_ui(&mut harness);
    let model = harness.state().app().model();
    let (_, tab) = model
        .active_tab(model.focused)
        .unwrap_or_else(|| panic!("a tab"));
    let failure = tab.failure.clone().unwrap_or_default();
    assert!(failure.contains("could not be listed"), "{failure}");
    assert!(model.questions.is_empty(), "no dialogue");
    assert_eq!(tab.listing.rows().count(), 0);
    assert!(
        harness
            .query_by_label_contains("could not be listed")
            .is_some()
    );
    assert!(harness.query_by_label("Retry").is_some());
    assert!(
        harness
            .query_by_label_contains("Unknown host key")
            .is_none()
    );

    // The server comes up; Retry lists again, which asks for the host key
    // this time and then shows the listing with the banner gone.
    let server = runtime::block_on(FakeSshd::start(
        Options {
            home: home.path().to_path_buf(),
            port,
            ..Options::default()
        },
        23,
    ))
    .unwrap_or_else(|e| panic!("{e}"));
    click_label(&mut harness, "Retry");
    assert!(
        step_until(&mut harness, Duration::from_secs(10), |h| {
            h.query_by_label_contains("Unknown host key").is_some()
        }),
        "the host-key dialogue after Retry"
    );
    click_label(&mut harness, "Trust once");
    settle_ui(&mut harness);
    let model = harness.state().app().model();
    let (_, tab) = model
        .active_tab(model.focused)
        .unwrap_or_else(|| panic!("a tab"));
    assert!(tab.failure.is_none(), "{:?}", tab.failure);
    let mut names: Vec<String> = tab.listing.rows().map(|e| e.name.clone()).collect();
    names.sort();
    assert_eq!(names, ["archive", "notes.txt", "photos"]);
    assert!(harness.query_by_label("Retry").is_none());
    assert!(
        !fake.home_dir().join(".ssh").join("known_hosts").exists()
            || !fs::read_to_string(fake.home_dir().join(".ssh").join("known_hosts"))
                .unwrap_or_default()
                .contains(&ext_ssh::known_hosts::host_pattern("127.0.0.1", port)),
        "Trust once saves nothing"
    );
    drop(server);
}
