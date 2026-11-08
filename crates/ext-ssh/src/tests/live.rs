//! The plugin against a live server: the SSH server in this process
//! ([`fake_sshd`]) on every OS, and the containers of CI where
//! `RICK_SSH_GNU_PORT` and friends name them (skipped with a note
//! elsewhere; `RICK_REQUIRE_CONTAINERS` makes the skip a failure).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use ext::platform::{ConfigDir as _, Fake, Secrets as _};
use ext::{
    Connection, Connector as _, ListOptions, Prompts, Provider as _, RemoveMode, TransferMode,
    WriteOptions,
};
use futures::StreamExt as _;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use types::{Answer, EntryKind, Error, Location, Question, QuestionKind};

use crate::connection::{MAX_CHANNELS, Pool, SshConnector};
use crate::fake_sshd::{self, FakeSshd, Options};
use crate::hosts::Hosts;
use crate::profile::Profile;
use crate::provider::SshProvider;
use crate::{PROVIDER_ID, location};

/// An SSH agent that holds whatever keys the test adds; it serves over a
/// Unix socket, so only the Unix test runs it.
#[cfg(unix)]
#[derive(Clone)]
struct Agent;

#[cfg(unix)]
impl russh::keys::agent::server::Agent for Agent {}

/// A UI host that answers nothing: the actions under test ask nothing.
struct Quiet;

impl ext::UiHost for Quiet {
    fn notify(&self, _: ext::Level, _: &str) {}
    fn confirm(&self, _: &str, _: &str) -> bool {
        true
    }
    fn prompt(&self, _: &str, _: &str) -> Option<String> {
        None
    }
    fn show(&self, _: &str, _: &str) {}
    fn form(&self, _: ext::Form) -> Option<ext::Values> {
        None
    }
}

/// The world of one test: a fake platform, the hosts, the pool and the
/// provider over them, and the questions the pool asked.
struct World {
    fake: Arc<Fake>,
    pool: Arc<Pool>,
    provider: SshProvider,
    connector: SshConnector,
    asked: Arc<std::sync::Mutex<Vec<Question>>>,
}

impl World {
    /// A world whose prompts answer every question with `answer_with`.
    fn new(answer_with: impl Fn(&Question) -> Answer + Send + Sync + 'static) -> Self {
        let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
        std::fs::create_dir_all(fake.home_dir().join(".ssh")).unwrap_or_else(|e| panic!("{e}"));
        let hosts = Arc::new(Hosts::new(fake.clone()));
        let prompts = Prompts::new();
        let asked: Arc<std::sync::Mutex<Vec<Question>>> = Arc::default();
        {
            let asked = Arc::clone(&asked);
            prompts.answer_with(move |q| {
                asked
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(q.clone());
                answer_with(q)
            });
        }
        let pool = Pool::new(hosts, Some(prompts));
        Self {
            fake,
            provider: SshProvider::new(Arc::clone(&pool)),
            connector: SshConnector::new(Arc::clone(&pool)),
            pool,
            asked,
        }
    }

    /// A Rick host of `alias` on the loopback `port`, with `fields` on top.
    fn host(alias: &str, port: u16, fields: &[(&str, &str)]) -> Connection {
        let mut settings: BTreeMap<String, String> = fields
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        settings.insert("host".into(), "127.0.0.1".into());
        settings.insert("port".into(), port.to_string());
        settings.insert("user".into(), "rick".into());
        Connection {
            name: alias.into(),
            settings,
        }
    }

    fn questions(&self) -> Vec<Question> {
        self.asked
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn known_hosts(&self) -> std::path::PathBuf {
        self.fake.home_dir().join(".ssh").join("known_hosts")
    }

    fn remember(&self, alias: &str, field: &str, secret: &str) {
        self.fake
            .store_secret(&ext::secret_key(PROVIDER_ID, alias, field), secret)
            .unwrap_or_else(|e| panic!("{e}"));
    }

    async fn names(&self, loc: &Location) -> Vec<String> {
        let batches: Vec<_> = self
            .provider
            .list(
                loc,
                ListOptions {
                    hidden: true,
                    ..ListOptions::default()
                },
                types::CancelToken::new(),
            )
            .collect()
            .await;
        let mut names: Vec<String> = batches
            .into_iter()
            .flat_map(|b| b.unwrap_or_else(|e| panic!("list: {e}")))
            .map(|e| e.name)
            .collect();
        names.sort();
        names
    }
}

/// *Trust and save* for every host key, nothing else.
fn trust_and_save(q: &Question) -> Answer {
    match &q.kind {
        QuestionKind::Choice { .. } if q.title == "Unknown host key" => Answer::Choice(0),
        _ => Answer::Cancelled,
    }
}

/// A "remote home" with the awkward names of the acceptance test.
fn remote_home() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(home.path().join("plain.txt"), b"hello").unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(home.path().join("with space.txt"), b"spaced").unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(home.path().join("new\nline.txt"), b"lined").unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(home.path().join(".hidden"), b"shh").unwrap_or_else(|e| panic!("{e}"));
    std::fs::create_dir(home.path().join("folder")).unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(home.path().join("folder").join("inner.txt"), b"in")
        .unwrap_or_else(|e| panic!("{e}"));
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;
        // A name that is not UTF-8; the file systems of macOS refuse one
        // (EILSEQ), and the home then does without it.
        let raw = std::ffi::OsStr::from_bytes(b"not\xffutf8.bin");
        if let Err(error) = std::fs::write(home.path().join(raw), b"raw") {
            eprintln!("no non-UTF-8 name on this file system: {error}");
        }
        std::os::unix::fs::symlink("plain.txt", home.path().join("link"))
            .unwrap_or_else(|e| panic!("{e}"));
    }
    home
}

/// Whether the remote home got the name that is not UTF-8.
fn has_raw_name(home: &Path) -> bool {
    #[cfg(unix)]
    let present = {
        use std::os::unix::ffi::OsStrExt as _;
        home.join(std::ffi::OsStr::from_bytes(b"not\xffutf8.bin"))
            .exists()
    };
    #[cfg(not(unix))]
    let present = {
        let _ = home;
        false
    };
    present
}

fn at(alias: &str, path: &str) -> Location {
    location(alias, path.as_bytes())
}

#[test]
// One scenario start to finish; splitting it would repeat the server.
#[allow(clippy::too_many_lines)]
fn against_the_server_browse_read_write_rename_delete_and_transfer_round_trip() {
    let home = remote_home();
    let world = World::new(trust_and_save);
    world.remember("box", "password", "rickpass");
    runtime::block_on(async {
        let server = FakeSshd::start(
            Options {
                home: home.path().to_path_buf(),
                ..Options::default()
            },
            1,
        )
        .await
        .unwrap_or_else(|e| panic!("{e}"));
        world
            .connector
            .update(vec![World::host("box", server.port, &[])]);
        world
            .connector
            .connect("box")
            .await
            .unwrap_or_else(|e| panic!("connect: {e}"));
        assert!(world.connector.is_connected("box"));
        assert_eq!(
            world.pool.profile("box").await.ok(),
            Some(Profile::Gnu),
            "the local userland is GNU"
        );
        // The unknown host key was asked about once and saved.
        let questions = world.questions();
        assert_eq!(questions.len(), 1, "{questions:?}");
        assert_eq!(questions[0].title, "Unknown host key");
        assert!(
            questions[0].text.contains("SHA256:"),
            "{}",
            questions[0].text
        );
        let known = std::fs::read_to_string(world.known_hosts()).unwrap_or_default();
        assert!(
            known.starts_with(&format!("[127.0.0.1]:{} ssh-ed25519 ", server.port)),
            "{known}"
        );

        // The bare alias is the home; the entries carry absolute paths.
        let root = at("box", "");
        let mut names = world.names(&root).await;
        names.retain(|n| n != "link");
        let mut expected = vec![
            ".hidden",
            "folder",
            "new\nline.txt",
            "plain.txt",
            "with space.txt",
        ];
        if has_raw_name(home.path()) {
            expected.push("not\u{fffd}utf8.bin");
        }
        expected.sort_unstable();
        assert_eq!(names, expected);
        let visible = world
            .provider
            .list(&root, ListOptions::default(), types::CancelToken::new())
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .flat_map(Result::unwrap_or_default)
            .collect::<Vec<_>>();
        assert!(
            visible.iter().all(|e| e.name != ".hidden"),
            "hidden stays hidden"
        );
        let plain = visible
            .iter()
            .find(|e| e.name == "plain.txt")
            .unwrap_or_else(|| panic!("plain.txt listed"));
        assert_eq!(plain.kind, EntryKind::File);
        assert_eq!(plain.size, Some(5));
        assert!(plain.modified.is_some());
        let home_text = home.path().to_string_lossy();
        assert_eq!(plain.path.to_string(), format!("/box{home_text}/plain.txt"));
        let folder = visible
            .iter()
            .find(|e| e.name == "folder")
            .unwrap_or_else(|| panic!("folder listed"));
        assert_eq!(folder.kind, EntryKind::Dir);
        assert_eq!(folder.size, None);

        // Stat, read, and read from an offset.
        let plain_loc = at("box", &format!("{home_text}/plain.txt"));
        let stat = world
            .provider
            .stat(&plain_loc)
            .await
            .unwrap_or_else(|e| panic!("stat: {e}"));
        assert_eq!(stat.size, Some(5));
        assert_eq!(stat.name, "plain.txt");
        let mut text = Vec::new();
        world
            .provider
            .open_read(&plain_loc)
            .await
            .unwrap_or_else(|e| panic!("open_read: {e}"))
            .read_to_end(&mut text)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(text, b"hello");
        let mut tail = Vec::new();
        world
            .provider
            .open_read_at(&plain_loc, 3)
            .await
            .unwrap_or_else(|e| panic!("open_read_at: {e}"))
            .read_to_end(&mut tail)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(tail, b"lo");
        let spaced = at("box", &format!("{home_text}/with space.txt"));
        let mut text = Vec::new();
        world
            .provider
            .open_read(&spaced)
            .await
            .unwrap_or_else(|e| panic!("{e}"))
            .read_to_end(&mut text)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(text, b"spaced");
        let missing = at("box", &format!("{home_text}/nope.txt"));
        assert!(matches!(
            world.provider.stat(&missing).await,
            Err(Error::NotFound)
        ));
        let mut sink = Vec::new();
        let read_missing = world
            .provider
            .open_read(&missing)
            .await
            .unwrap_or_else(|e| panic!("{e}"))
            .read_to_end(&mut sink)
            .await;
        assert!(
            matches!(&read_missing, Err(e) if e.kind() == std::io::ErrorKind::NotFound),
            "{read_missing:?}"
        );

        // Write through the temporary name, read back, refuse an overwrite.
        let payload: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        let data_loc = at("box", &format!("{home_text}/folder/data.bin"));
        {
            let mut w = world
                .provider
                .open_write(&data_loc, WriteOptions { overwrite: false })
                .await
                .unwrap_or_else(|e| panic!("open_write: {e}"));
            w.write_all(&payload)
                .await
                .unwrap_or_else(|e| panic!("{e}"));
            w.shutdown()
                .await
                .unwrap_or_else(|e| panic!("shutdown: {e}"));
        }
        assert_eq!(
            std::fs::read(home.path().join("folder").join("data.bin")).ok(),
            Some(payload.clone())
        );
        assert!(
            std::fs::read_dir(home.path().join("folder"))
                .unwrap_or_else(|e| panic!("{e}"))
                .filter_map(Result::ok)
                .all(|e| !e.file_name().to_string_lossy().contains(".rick-upload")),
            "no temporary name is left"
        );
        assert!(matches!(
            world.provider.open_write(&data_loc, WriteOptions { overwrite: false }).await,
            Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::AlreadyExists
        ));

        // mkdir, rename, link, delete.
        let sub = at("box", &format!("{home_text}/folder/sub dir"));
        let made = world
            .provider
            .create_dir(&sub)
            .await
            .unwrap_or_else(|e| panic!("create_dir: {e}"));
        assert_eq!(made.kind, EntryKind::Dir);
        assert!(matches!(
            world.provider.create_dir(&sub).await,
            Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::AlreadyExists
        ));
        let renamed = at("box", &format!("{home_text}/folder/renamed.bin"));
        world
            .provider
            .rename(&data_loc, &renamed)
            .await
            .unwrap_or_else(|e| panic!("rename: {e}"));
        assert!(home.path().join("folder").join("renamed.bin").is_file());
        let link = world
            .provider
            .link(
                &renamed,
                &at("box", &format!("{home_text}/folder/lnk")),
                ext::LinkKind::Symbolic,
            )
            .await
            .unwrap_or_else(|e| panic!("link: {e}"));
        assert!(
            matches!(link.kind, EntryKind::Symlink { broken: false, .. }),
            "{link:?}"
        );

        // A native move and copy run on the host.
        let (sink, _rx) = ext::ProgressSink::channel();
        world
            .provider
            .transfer(
                std::slice::from_ref(&renamed),
                &sub,
                TransferMode::Move,
                sink.clone(),
                types::CancelToken::new(),
            )
            .await
            .unwrap_or_else(|e| panic!("move: {e}"));
        assert!(
            home.path()
                .join("folder")
                .join("sub dir")
                .join("renamed.bin")
                .is_file()
        );
        assert!(!home.path().join("folder").join("renamed.bin").exists());
        world
            .provider
            .transfer(
                &[at("box", &format!("{home_text}/folder/sub dir"))],
                &at("box", &home_text),
                TransferMode::Copy,
                sink,
                types::CancelToken::new(),
            )
            .await
            .unwrap_or_else(|e| panic!("copy: {e}"));
        assert!(home.path().join("sub dir").join("renamed.bin").is_file());
        assert!(
            home.path()
                .join("folder")
                .join("sub dir")
                .join("renamed.bin")
                .is_file()
        );

        // Disk usage and the single-file copy.
        let usage = world
            .provider
            .disk_usage(&at("box", &home_text))
            .await
            .unwrap_or_else(|e| panic!("df: {e}"));
        assert!(
            usage.is_some_and(|(total, free)| total > 0 && free <= total),
            "{usage:?}"
        );
        let (sink, _rx) = ext::ProgressSink::channel();
        world
            .provider
            .copy(
                &plain_loc,
                &at("box", &format!("{home_text}/copy.txt")),
                ext::CopyOptions::default(),
                sink,
                types::CancelToken::new(),
            )
            .await
            .unwrap_or_else(|e| panic!("cp: {e}"));
        assert_eq!(
            std::fs::read(home.path().join("copy.txt")).ok(),
            Some(b"hello".to_vec())
        );

        assert!(matches!(
            world.provider.remove(&sub, RemoveMode::Trash).await,
            Err(Error::Unsupported(_))
        ));
        world
            .provider
            .remove(
                &at("box", &format!("{home_text}/folder")),
                RemoveMode::Permanent,
            )
            .await
            .unwrap_or_else(|e| panic!("rm: {e}"));
        assert!(!home.path().join("folder").exists());
        assert!(matches!(
            world.provider.remove(&missing, RemoveMode::Permanent).await,
            Err(Error::NotFound)
        ));

        // One connection carried all of it; a second connect asks nothing.
        assert_eq!(world.pool.connections("box").len(), 1);
        assert_eq!(server.connections.load(Ordering::Relaxed), 1);
        world.connector.disconnect("box");
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(!world.connector.is_connected("box"));
        let _ = world.names(&root).await;
        assert!(
            world.connector.is_connected("box"),
            "the next operation reconnects"
        );
        assert_eq!(world.questions().len(), 1, "the saved key is known now");
    });
}

#[test]
fn eleven_channels_open_a_second_connection_and_a_closed_one_is_replaced() {
    let home = remote_home();
    let world = World::new(trust_and_save);
    world.remember("box", "password", "rickpass");
    runtime::block_on(async {
        let server = FakeSshd::start(
            Options {
                home: home.path().to_path_buf(),
                ..Options::default()
            },
            2,
        )
        .await
        .unwrap_or_else(|e| panic!("{e}"));
        world
            .connector
            .update(vec![World::host("box", server.port, &[])]);
        let mut leases = Vec::new();
        for _ in 0..MAX_CHANNELS {
            leases.push(
                world
                    .pool
                    .lease("box")
                    .await
                    .unwrap_or_else(|e| panic!("lease: {e}")),
            );
        }
        assert_eq!(world.pool.connections("box").len(), 1);
        leases.push(
            world
                .pool
                .lease("box")
                .await
                .unwrap_or_else(|e| panic!("11th lease: {e}")),
        );
        assert_eq!(
            world.pool.connections("box").len(),
            2,
            "the eleventh channel opens a second connection"
        );
        assert_eq!(server.connections.load(Ordering::Relaxed), 2);
        drop(leases);
        tokio::time::sleep(Duration::from_millis(100)).await;
        let lives = world.pool.connections("box");
        assert!(lives.iter().all(|l| l.channels() == 0));
        assert_eq!(
            world.questions().len(),
            1,
            "one host-key question for both connections"
        );
    });
}

#[test]
// One scenario start to finish; splitting it would repeat the server.
#[allow(clippy::too_many_lines)]
fn a_host_behind_a_bastion_and_a_two_hop_chain_browse_like_a_direct_one() {
    let home = remote_home();
    let world = World::new(trust_and_save);
    for alias in ["bastion", "hop", "target", "other"] {
        world.remember(alias, "password", "rickpass");
    }
    runtime::block_on(async {
        let options = || Options {
            home: home.path().to_path_buf(),
            ..Options::default()
        };
        let bastion = FakeSshd::start(options(), 3)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        let hop = FakeSshd::start(options(), 4)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        let target = FakeSshd::start(options(), 5)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        world.connector.update(vec![
            World::host("bastion", bastion.port, &[]),
            World::host("hop", hop.port, &[("jump", "bastion")]),
            World::host("target", target.port, &[("jump", "bastion, hop")]),
            World::host("other", target.port, &[("jump", "bastion")]),
        ]);
        let home_text = home.path().to_string_lossy().into_owned();
        // Two hops: bastion, then hop, then the target.
        let names = world.names(&at("target", "")).await;
        assert!(names.contains(&"plain.txt".to_owned()), "{names:?}");
        assert_eq!(bastion.connections.load(Ordering::Relaxed), 1);
        assert_eq!(hop.connections.load(Ordering::Relaxed), 1);
        assert_eq!(target.connections.load(Ordering::Relaxed), 1);
        // Each hop got its own host-key check.
        let titles: Vec<String> = world.questions().iter().map(|q| q.title.clone()).collect();
        assert_eq!(titles, ["Unknown host key"; 3]);
        let known = std::fs::read_to_string(world.known_hosts()).unwrap_or_default();
        assert_eq!(known.lines().count(), 3, "{known}");
        // A second target behind the same bastion shares its connection.
        let names = world.names(&at("other", "")).await;
        assert!(names.contains(&"plain.txt".to_owned()));
        assert_eq!(
            bastion.connections.load(Ordering::Relaxed),
            1,
            "one bastion connection"
        );
        assert_eq!(world.pool.connections("bastion").len(), 1);
        assert_eq!(target.connections.load(Ordering::Relaxed), 2);
        assert_eq!(
            world.questions().len(),
            3,
            "the target's key is known from the first hop chain"
        );
        // Operations through the chain work like direct ones.
        let mut text = Vec::new();
        world
            .provider
            .open_read(&at("target", &format!("{home_text}/plain.txt")))
            .await
            .unwrap_or_else(|e| panic!("{e}"))
            .read_to_end(&mut text)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(text, b"hello");
        // The bastion stays open while a target behind it is in use.
        let bastion_live = &world.pool.connections("bastion")[0];
        assert!(
            bastion_live.channels() >= 1,
            "the tunnels count as channels"
        );
        // A hop that refuses names itself.
        let closed = FakeSshd::start(
            Options {
                forwarding: false,
                ..options()
            },
            6,
        )
        .await
        .unwrap_or_else(|e| panic!("{e}"));
        world.remember("wall", "password", "rickpass");
        world.remember("behind", "password", "rickpass");
        let mut list = vec![
            World::host("bastion", bastion.port, &[]),
            World::host("hop", hop.port, &[("jump", "bastion")]),
            World::host("target", target.port, &[("jump", "bastion, hop")]),
            World::host("other", target.port, &[("jump", "bastion")]),
            World::host("wall", closed.port, &[]),
            World::host("behind", target.port, &[("jump", "wall")]),
        ];
        world.connector.update(list.clone());
        let error = world
            .pool
            .live("behind")
            .await
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(error.contains("wall"), "the hop is named: {error}");
        // A ProxyJump loop is refused, not followed.
        list.push(World::host("loop-a", target.port, &[("jump", "loop-b")]));
        list.push(World::host("loop-b", target.port, &[("jump", "loop-a")]));
        world.connector.update(list);
        let error = world
            .pool
            .live("loop-a")
            .await
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(error.contains("loops"), "{error}");
    });
}

#[test]
// One scenario start to finish; splitting it would repeat the server.
#[allow(clippy::too_many_lines)]
fn host_keys_are_never_trusted_silently() {
    let home = remote_home();
    runtime::block_on(async {
        let server = FakeSshd::start(
            Options {
                home: home.path().to_path_buf(),
                ..Options::default()
            },
            7,
        )
        .await
        .unwrap_or_else(|e| panic!("{e}"));
        // Nobody to ask: refused.
        let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
        let hosts = Arc::new(Hosts::new(fake.clone()));
        let pool = Pool::new(Arc::clone(&hosts), None);
        hosts.update(vec![World::host("box", server.port, &[])]);
        fake.store_secret(&ext::secret_key(PROVIDER_ID, "box", "password"), "rickpass")
            .unwrap_or_else(|e| panic!("{e}"));
        let error = pool
            .live("box")
            .await
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(error.contains("nobody can be asked"), "{error}");
        assert!(!fake.home_dir().join(".ssh").join("known_hosts").exists());

        // Cancel: refused, nothing saved.
        let world = World::new(|_| Answer::Cancelled);
        world.remember("box", "password", "rickpass");
        world
            .connector
            .update(vec![World::host("box", server.port, &[])]);
        let error = world
            .pool
            .live("box")
            .await
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(error.contains("not trusted"), "{error}");
        assert!(!world.known_hosts().exists());

        // Trust once: connects, saves nothing.
        let world = World::new(|q| match &q.kind {
            QuestionKind::Choice { .. } => Answer::Choice(1),
            QuestionKind::Secret { .. } => Answer::Cancelled,
        });
        world.remember("box", "password", "rickpass");
        world
            .connector
            .update(vec![World::host("box", server.port, &[])]);
        world
            .pool
            .live("box")
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(!world.known_hosts().exists());

        // Trust and save, then the saved key changes: the warning with
        // Cancel as its default refuses; Connect once connects; Replace
        // asks again and rewrites the entry.
        let world = World::new(trust_and_save);
        world.remember("box", "password", "rickpass");
        world
            .connector
            .update(vec![World::host("box", server.port, &[])]);
        world
            .pool
            .live("box")
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        world.pool.disconnect("box");
        tokio::time::sleep(Duration::from_millis(100)).await;
        let other = fake_sshd::test_key(99).public_key().clone();
        crate::known_hosts::replace(&world.known_hosts(), "127.0.0.1", server.port, &other)
            .unwrap_or_else(|e| panic!("{e}"));
        let error = world
            .pool
            .live("box")
            .await
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(error.contains("changed"), "{error}");
        let warning = world
            .questions()
            .last()
            .cloned()
            .unwrap_or_else(|| panic!("asked"));
        assert_eq!(warning.title, "Host key changed");
        assert!(warning.warning);
        assert!(matches!(
            warning.kind,
            QuestionKind::Choice { default: 0, .. }
        ));
        assert!(warning.text.contains("WARNING"));

        let world2 = World::new(|q| match q.title.as_str() {
            "Host key changed" => Answer::Choice(1),
            _ => Answer::Cancelled,
        });
        world2.remember("box", "password", "rickpass");
        std::fs::create_dir_all(world2.fake.home_dir().join(".ssh"))
            .unwrap_or_else(|e| panic!("{e}"));
        std::fs::copy(world.known_hosts(), world2.known_hosts()).unwrap_or_else(|e| panic!("{e}"));
        world2
            .connector
            .update(vec![World::host("box", server.port, &[])]);
        world2
            .pool
            .live("box")
            .await
            .unwrap_or_else(|e| panic!("connect once: {e}"));
        let kept = std::fs::read_to_string(world2.known_hosts()).unwrap_or_default();
        assert_eq!(
            crate::known_hosts::check_text(&kept, "127.0.0.1", server.port, &other),
            crate::known_hosts::Verdict::Known,
            "connect once leaves the file alone"
        );

        let world3 = World::new(|q| match q.title.as_str() {
            "Host key changed" => Answer::Choice(2),
            "Replace the saved host key?" => Answer::Choice(1),
            _ => Answer::Cancelled,
        });
        world3.remember("box", "password", "rickpass");
        std::fs::create_dir_all(world3.fake.home_dir().join(".ssh"))
            .unwrap_or_else(|e| panic!("{e}"));
        std::fs::copy(world.known_hosts(), world3.known_hosts()).unwrap_or_else(|e| panic!("{e}"));
        world3
            .connector
            .update(vec![World::host("box", server.port, &[])]);
        world3
            .pool
            .live("box")
            .await
            .unwrap_or_else(|e| panic!("replace: {e}"));
        let titles: Vec<String> = world3.questions().iter().map(|q| q.title.clone()).collect();
        assert_eq!(titles, ["Host key changed", "Replace the saved host key?"]);
        let replaced = std::fs::read_to_string(world3.known_hosts()).unwrap_or_default();
        assert_eq!(
            crate::known_hosts::check_text(&replaced, "127.0.0.1", server.port, &server.host_key),
            crate::known_hosts::Verdict::Known
        );
        assert_eq!(replaced.lines().count(), 1, "{replaced}");
    });
}

#[test]
// One scenario start to finish; splitting it would repeat the server.
#[allow(clippy::too_many_lines)]
fn identity_files_passphrases_keyboard_interactive_and_passwords_authenticate() {
    let home = remote_home();
    runtime::block_on(async {
        let identity = fake_sshd::test_key(11);
        let locked = russh::keys::decode_secret_key(
            fake_sshd::ENCRYPTED_KEY,
            Some(fake_sshd::ENCRYPTED_KEY_PASSPHRASE),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        let keys_only = FakeSshd::start(
            Options {
                home: home.path().to_path_buf(),
                password: None,
                authorized: vec![identity.public_key().clone(), locked.public_key().clone()],
                ..Options::default()
            },
            8,
        )
        .await
        .unwrap_or_else(|e| panic!("{e}"));

        // An identity file named by the host editor.
        let world = World::new(trust_and_save);
        let id_file = world.fake.home_dir().join(".ssh").join("id_box");
        fake_sshd::write_identity(&id_file, &identity).unwrap_or_else(|e| panic!("{e}"));
        world.connector.update(vec![World::host(
            "box",
            keys_only.port,
            &[("identity_file", &id_file.to_string_lossy())],
        )]);
        world
            .pool
            .live("box")
            .await
            .unwrap_or_else(|e| panic!("identity file: {e}"));

        // A default identity file, ~/.ssh/id_ed25519, without any setting.
        let world = World::new(trust_and_save);
        fake_sshd::write_identity(
            &world.fake.home_dir().join(".ssh").join("id_ed25519"),
            &identity,
        )
        .unwrap_or_else(|e| panic!("{e}"));
        world
            .connector
            .update(vec![World::host("box", keys_only.port, &[])]);
        world
            .pool
            .live("box")
            .await
            .unwrap_or_else(|e| panic!("default identity: {e}"));

        // An encrypted key asks for its passphrase and remembers it when
        // told to; the next connection takes it from the keychain.
        let asked_passphrase = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&asked_passphrase);
        let world = World::new(move |q| match &q.kind {
            QuestionKind::Secret { remember } => {
                counter.fetch_add(1, Ordering::Relaxed);
                assert!(q.title.starts_with("Passphrase for"), "{q:?}");
                Answer::Secret {
                    secret: fake_sshd::ENCRYPTED_KEY_PASSPHRASE.to_owned(),
                    remember: remember.unwrap_or(false),
                }
            }
            QuestionKind::Choice { .. } => trust_and_save(q),
        });
        let locked_file = world.fake.home_dir().join(".ssh").join("id_locked");
        std::fs::write(&locked_file, fake_sshd::ENCRYPTED_KEY).unwrap_or_else(|e| panic!("{e}"));
        world.connector.update(vec![World::host(
            "box",
            keys_only.port,
            &[
                ("identity_file", &locked_file.to_string_lossy()),
                ("remember_password", "true"),
            ],
        )]);
        world
            .pool
            .live("box")
            .await
            .unwrap_or_else(|e| panic!("passphrase: {e}"));
        assert_eq!(asked_passphrase.load(Ordering::Relaxed), 1);
        assert_eq!(
            world
                .fake
                .secrets()
                .get("ssh/box/passphrase")
                .map(String::as_str),
            Some(fake_sshd::ENCRYPTED_KEY_PASSPHRASE),
            "remembered in the keychain, never in a file"
        );
        world.pool.disconnect("box");
        tokio::time::sleep(Duration::from_millis(100)).await;
        world
            .pool
            .live("box")
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            asked_passphrase.load(Ordering::Relaxed),
            1,
            "the keychain answered"
        );

        // Keyboard-interactive, then a typed password remembered on
        // request, then the keychain's password.
        let interactive = FakeSshd::start(
            Options {
                home: home.path().to_path_buf(),
                keyboard_interactive: true,
                ..Options::default()
            },
            9,
        )
        .await
        .unwrap_or_else(|e| panic!("{e}"));
        let typed = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&typed);
        let world = World::new(move |q| match &q.kind {
            QuestionKind::Secret { remember } => {
                counter.fetch_add(1, Ordering::Relaxed);
                Answer::Secret {
                    secret: "rickpass".to_owned(),
                    remember: remember.unwrap_or(false),
                }
            }
            QuestionKind::Choice { .. } => trust_and_save(q),
        });
        world.connector.update(vec![World::host(
            "box",
            interactive.port,
            &[("remember_password", "true")],
        )]);
        world
            .pool
            .live("box")
            .await
            .unwrap_or_else(|e| panic!("keyboard-interactive: {e}"));
        assert_eq!(typed.load(Ordering::Relaxed), 1);
        assert_eq!(
            world
                .fake
                .secrets()
                .get("ssh/box/password")
                .map(String::as_str),
            Some("rickpass")
        );
        let toml_text =
            std::fs::read_to_string(world.fake.config_dir().join("rick.toml")).unwrap_or_default();
        assert!(!toml_text.contains("rickpass"));
        world.pool.disconnect("box");
        tokio::time::sleep(Duration::from_millis(100)).await;
        world
            .pool
            .live("box")
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            typed.load(Ordering::Relaxed),
            1,
            "the keychain's password served"
        );

        // A wrong password three times gives up on the method.
        let world = World::new(|q| match &q.kind {
            QuestionKind::Secret { .. } => Answer::Secret {
                secret: "wrong".to_owned(),
                remember: false,
            },
            QuestionKind::Choice { .. } => trust_and_save(q),
        });
        world
            .connector
            .update(vec![World::host("box", interactive.port, &[])]);
        let error = world
            .pool
            .live("box")
            .await
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(error.contains("refused every authentication"), "{error}");
    });
}

#[cfg(unix)]
#[test]
fn the_agent_authenticates_when_it_holds_a_key() {
    use futures::StreamExt as _;

    let home = remote_home();
    runtime::block_on(async {
        let identity = fake_sshd::test_key(12);
        let server = FakeSshd::start(
            Options {
                home: home.path().to_path_buf(),
                password: None,
                authorized: vec![identity.public_key().clone()],
                ..Options::default()
            },
            13,
        )
        .await
        .unwrap_or_else(|e| panic!("{e}"));
        let world = World::new(trust_and_save);
        let socket = world.fake.home_dir().join("agent.sock");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap_or_else(|e| panic!("{e}"));
        let accepting = futures::stream::unfold(listener, |listener| async move {
            let next = listener.accept().await.map(|(stream, _)| stream);
            Some((next, listener))
        })
        .boxed();
        tokio::spawn(russh::keys::agent::server::serve(accepting, Agent));
        let mut client = russh::keys::agent::client::AgentClient::connect_uds(&socket)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        client
            .add_identity(&identity, &[])
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        // The configuration names the agent for this host.
        std::fs::write(
            world.fake.home_dir().join(".ssh").join("config"),
            format!(
                "Host agentbox\n  HostName 127.0.0.1\n  Port {}\n  User rick\n  IdentityAgent {}\n",
                server.port,
                socket.display()
            ),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        world
            .pool
            .live("agentbox")
            .await
            .unwrap_or_else(|e| panic!("agent: {e}"));
        assert!(world.pool.is_connected("agentbox"));
    });
}

#[test]
fn the_console_opens_a_shell_in_the_folder_and_the_terminal_gets_the_command() {
    use ext::Console as _;

    let home = remote_home();
    let world = World::new(trust_and_save);
    world.remember("box", "password", "rickpass");
    runtime::block_on(async {
        let server = FakeSshd::start(
            Options {
                home: home.path().to_path_buf(),
                ..Options::default()
            },
            14,
        )
        .await
        .unwrap_or_else(|e| panic!("{e}"));
        world.connector.update(vec![World::host(
            "box",
            server.port,
            &[("default_dir", "/")],
        )]);
        let folder = home.path().join("folder");
        let console = crate::console::SshConsole::start(
            Arc::clone(&world.pool),
            "box",
            folder.to_string_lossy().as_bytes().to_vec(),
            80,
            24,
        );
        let deadline = Instant::now() + Duration::from_secs(20);
        while world.pool.connections("box").is_empty() && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        console.write(b"pwd\n");
        let folder_text = folder.to_string_lossy().into_owned();
        let shown = loop {
            let text = console.text();
            if text.contains(&folder_text) {
                break text;
            }
            assert!(Instant::now() < deadline, "no pwd in {text:?}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        assert!(shown.contains(&folder_text));
        assert!(console.exit_status().is_none());
        console.write(b"exit\n");
        let deadline = Instant::now() + Duration::from_secs(10);
        while console.exit_status().is_none() && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(console.exit_status(), Some(0));

        // The external terminal gets `ssh` with the folder.
        let actions = crate::actions::all(&world.pool);
        let ctx = ext::ActionCtx {
            pane: ext::PaneId(1),
            location: at("box", &folder_text),
            selection: Vec::new(),
            invoked_from: ext::Invocation::ContextMenu,
            other: None,
        };
        let outcome = actions[1]
            .run(ctx.clone(), &ext::JobRunner::new(), &Quiet)
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(outcome, ext::ActionOutcome::Done));
        let terminals = world.fake.terminals();
        assert_eq!(terminals.len(), 1);
        let command = terminals[0].1.clone().unwrap_or_default();
        assert_eq!(
            &command[..4],
            ["ssh", "-p", &server.port.to_string(), "rick@127.0.0.1"]
        );
        assert_eq!(command[4], "-t");
        assert!(
            command[5].starts_with(&format!("cd '{folder_text}' && exec")),
            "{}",
            command[5]
        );
        // The embedded console action opens a tab at once.
        let outcome = actions[0]
            .run(ctx, &ext::JobRunner::new(), &Quiet)
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(
            outcome,
            ext::ActionOutcome::View(ext::View::Console(_))
        ));
    });
}

/// A container's host for the CI round trips: the variables
/// `containers.sh` sets, or `None` with a note (a failure under
/// `RICK_REQUIRE_CONTAINERS`).
fn container(port_var: &str) -> Option<(u16, String, String, String)> {
    let Ok(port) = std::env::var(port_var) else {
        eprintln!("{port_var} is not set: no sshd container to test against; skipped");
        assert!(
            std::env::var_os("RICK_REQUIRE_CONTAINERS").is_none(),
            "RICK_REQUIRE_CONTAINERS is set but {port_var} is not"
        );
        return None;
    };
    Some((
        port.parse().unwrap_or(22),
        std::env::var("RICK_SSH_USER").unwrap_or_else(|_| "rick".into()),
        std::env::var("RICK_SSH_PASSWORD").unwrap_or_else(|_| "rickpass".into()),
        std::env::var("RICK_SSH_KEY_FILE").unwrap_or_default(),
    ))
}

/// The round trip against a container: the names with spaces, newlines
/// and bytes that are no UTF-8, written, listed, read from an offset,
/// moved and copied on the host, removed.
#[allow(clippy::too_many_lines)] // one round trip, step after step
async fn container_round_trip(world: &World, alias: &str, expected: Profile) {
    let profile = world
        .pool
        .profile(alias)
        .await
        .unwrap_or_else(|e| panic!("probe: {e}"));
    assert_eq!(profile, expected);
    let home = String::from_utf8_lossy(
        &world
            .pool
            .default_dir(alias)
            .await
            .unwrap_or_else(|e| panic!("{e}")),
    )
    .into_owned();
    let unique = format!(
        "{home}/rick-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis())
    );
    world
        .provider
        .create_dir(&at(alias, &unique))
        .await
        .unwrap_or_else(|e| panic!("mkdir: {e}"));
    let names: [&[u8]; 4] = [
        b"plain.txt",
        b"with space.txt",
        b"new\nline.txt",
        b"not\xffutf8.bin",
    ];
    for name in names {
        let mut path = format!("{unique}/").into_bytes();
        path.extend_from_slice(name);
        let mut w = world
            .provider
            .open_write(&location(alias, &path), WriteOptions { overwrite: false })
            .await
            .unwrap_or_else(|e| panic!("open_write: {e}"));
        w.write_all(b"0123456789")
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        w.shutdown()
            .await
            .unwrap_or_else(|e| panic!("shutdown: {e}"));
    }
    let listed = world
        .provider
        .list(
            &at(alias, &unique),
            ListOptions::default(),
            types::CancelToken::new(),
        )
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .flat_map(|b| b.unwrap_or_else(|e| panic!("list: {e}")))
        .collect::<Vec<_>>();
    let mut raw: Vec<Vec<u8>> = listed
        .iter()
        .map(|e| e.path.file_name().unwrap_or_default().to_vec())
        .collect();
    raw.sort();
    let mut wanted: Vec<Vec<u8>> = names.iter().map(|n| n.to_vec()).collect();
    wanted.sort();
    assert_eq!(raw, wanted, "every byte of every name survives the listing");
    assert!(listed.iter().all(|e| e.size == Some(10)), "{listed:?}");
    let mut tail = Vec::new();
    world
        .provider
        .open_read_at(&at(alias, &format!("{unique}/plain.txt")), 7)
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .read_to_end(&mut tail)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(tail, b"789");
    world
        .provider
        .rename(
            &at(alias, &format!("{unique}/plain.txt")),
            &at(alias, &format!("{unique}/renamed.txt")),
        )
        .await
        .unwrap_or_else(|e| panic!("rename: {e}"));
    world
        .provider
        .create_dir(&at(alias, &format!("{unique}/sub")))
        .await
        .unwrap_or_else(|e| panic!("mkdir sub: {e}"));
    let (sink, _rx) = ext::ProgressSink::channel();
    world
        .provider
        .transfer(
            &[at(alias, &format!("{unique}/renamed.txt"))],
            &at(alias, &format!("{unique}/sub")),
            TransferMode::Move,
            sink.clone(),
            types::CancelToken::new(),
        )
        .await
        .unwrap_or_else(|e| panic!("mv: {e}"));
    world
        .provider
        .transfer(
            &[at(alias, &format!("{unique}/with space.txt"))],
            &at(alias, &format!("{unique}/sub")),
            TransferMode::Copy,
            sink,
            types::CancelToken::new(),
        )
        .await
        .unwrap_or_else(|e| panic!("cp: {e}"));
    let sub = world.names(&at(alias, &format!("{unique}/sub"))).await;
    assert_eq!(sub, ["renamed.txt", "with space.txt"]);
    let stat = world
        .provider
        .stat(&at(alias, &format!("{unique}/sub/renamed.txt")))
        .await
        .unwrap_or_else(|e| panic!("stat: {e}"));
    assert_eq!(stat.size, Some(10));
    world
        .provider
        .remove(&at(alias, &unique), RemoveMode::Permanent)
        .await
        .unwrap_or_else(|e| panic!("rm: {e}"));
    assert!(matches!(
        world.provider.stat(&at(alias, &unique)).await,
        Err(Error::NotFound)
    ));
}

/// Against the Debian and Alpine containers of CI: the `gnu` and `busybox`
/// profiles, with the password, the key file and, through the bastion
/// network, one and two hops.
#[test]
fn against_the_containers_the_gnu_and_busybox_profiles_and_the_bastion_round_trip() {
    let Some((gnu_port, user, password, key_file)) = container("RICK_SSH_GNU_PORT") else {
        return;
    };
    let busybox_port: u16 = std::env::var("RICK_SSH_BUSYBOX_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or_else(|| panic!("RICK_SSH_BUSYBOX_PORT"));
    let bastion_port: u16 = std::env::var("RICK_SSH_BASTION_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or_else(|| panic!("RICK_SSH_BASTION_PORT"));
    let hop = std::env::var("RICK_SSH_HOP").unwrap_or_else(|_| "rick-ssh-hop".into());
    let target = std::env::var("RICK_SSH_TARGET").unwrap_or_else(|_| "rick-ssh-target".into());
    let world = World::new(trust_and_save);
    let host = |alias: &str, host: &str, port: u16, fields: &[(&str, &str)]| {
        let mut connection = World::host(alias, port, fields);
        connection.settings.insert("host".into(), host.to_owned());
        connection.settings.insert("user".into(), user.clone());
        connection
    };
    world.remember("gnu", "password", &password);
    world.remember("bastion", "password", &password);
    world.remember("hop", "password", &password);
    world.remember("target", "password", &password);
    world.connector.update(vec![
        host("gnu", "127.0.0.1", gnu_port, &[]),
        host(
            "busybox",
            "127.0.0.1",
            busybox_port,
            &[("identity_file", &key_file)],
        ),
        host("bastion", "127.0.0.1", bastion_port, &[]),
        host("hop", &hop, 22, &[("jump", "bastion")]),
        host("target", &target, 22, &[("jump", "bastion, hop")]),
    ]);
    runtime::block_on(async {
        container_round_trip(&world, "gnu", Profile::Gnu).await;
        container_round_trip(&world, "busybox", Profile::Busybox).await;
        container_round_trip(&world, "hop", Profile::Gnu).await;
        container_round_trip(&world, "target", Profile::Busybox).await;
        assert_eq!(world.pool.connections("bastion").len(), 1);
        // Eleven channels on a real sshd open a second connection.
        let mut leases = Vec::new();
        for _ in 0..=MAX_CHANNELS {
            leases.push(
                world
                    .pool
                    .lease("gnu")
                    .await
                    .unwrap_or_else(|e| panic!("lease: {e}")),
            );
        }
        assert_eq!(world.pool.connections("gnu").len(), 2);
        drop(leases);
        // The passphrase-protected key, from the keychain.
        if let (Ok(locked), Ok(phrase)) = (
            std::env::var("RICK_SSH_LOCKED_KEY_FILE"),
            std::env::var("RICK_SSH_PASSPHRASE"),
        ) {
            world.remember("locked", "passphrase", &phrase);
            let mut list = vec![host(
                "locked",
                "127.0.0.1",
                gnu_port,
                &[("identity_file", &locked)],
            )];
            list.extend([
                host("gnu", "127.0.0.1", gnu_port, &[]),
                host(
                    "busybox",
                    "127.0.0.1",
                    busybox_port,
                    &[("identity_file", &key_file)],
                ),
                host("bastion", "127.0.0.1", bastion_port, &[]),
            ]);
            world.connector.update(list);
            world
                .pool
                .live("locked")
                .await
                .unwrap_or_else(|e| panic!("locked key: {e}"));
        }
        // The agent, when the job started one.
        if std::env::var_os("SSH_AUTH_SOCK").is_some() {
            std::fs::write(
                world.fake.home_dir().join(".ssh").join("config"),
                format!("Host agent\n  HostName 127.0.0.1\n  Port {gnu_port}\n  User {user}\n"),
            )
            .unwrap_or_else(|e| panic!("{e}"));
            world
                .pool
                .live("agent")
                .await
                .unwrap_or_else(|e| panic!("agent: {e}"));
        }
    });
    let _ = Path::new(&key_file);
}
