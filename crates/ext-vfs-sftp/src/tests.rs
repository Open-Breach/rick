use std::sync::Arc;

use ext::{
    Capabilities, ConnectionFieldKind, Connector as _, Extension as _, Provider as _, Registry,
};
use futures::StreamExt as _;
use platform::Fake;
use russh_sftp::protocol::FileAttributes;
use types::{Attrs, EntryKind, Location};

use super::*;

#[test]
fn locations_split_into_connection_and_remote_path() {
    assert_eq!(split(&Location::new(PROVIDER_ID, "/")), None);
    assert_eq!(split(&Location::new(PROVIDER_ID, "")), None);
    assert_eq!(
        split(&Location::new(PROVIDER_ID, "/nas")),
        Some(("nas".to_owned(), String::new()))
    );
    assert_eq!(
        split(&Location::new(PROVIDER_ID, "/nas/")),
        Some(("nas".to_owned(), String::new()))
    );
    assert_eq!(
        split(&Location::new(PROVIDER_ID, "/nas/photos/a b.jpg")),
        Some(("nas".to_owned(), "photos/a b.jpg".to_owned()))
    );
    assert_eq!(
        split(&Location::new(PROVIDER_ID, "\\nas\\photos")),
        Some(("nas".to_owned(), "photos".to_owned()))
    );
    assert_eq!(location("nas", "").to_string(), "sftp:/nas");
    assert_eq!(
        location("nas", "photos/a").to_string(),
        "sftp:/nas/photos/a"
    );
    assert_eq!(location("nas", "/photos/").to_string(), "sftp:/nas/photos");
}

#[test]
fn entries_take_kind_size_times_and_flags_from_the_attributes() {
    let attrs = FileAttributes {
        size: Some(1234),
        permissions: Some(0o100_755),
        mtime: Some(1_700_000_000),
        atime: Some(1_600_000_000),
        ..FileAttributes::default()
    };
    let e = provider::entry("nas", "bin/tool", &attrs, None);
    assert_eq!(e.name, "tool");
    assert_eq!(e.path.to_string(), "/nas/bin/tool");
    assert_eq!(e.kind, EntryKind::File);
    assert_eq!(e.size, Some(1234));
    assert!(e.attrs.contains(Attrs::EXECUTABLE));
    assert!(!e.attrs.contains(Attrs::HIDDEN));
    assert!(e.modified.is_some() && e.accessed.is_some());
    assert_eq!(e.id.provider, PROVIDER_ID);
    let again = provider::entry("nas", "bin/tool", &attrs, None);
    assert_eq!(e.id, again.id, "ids are stable across listings");

    let dir = FileAttributes {
        permissions: Some(0o040_555),
        ..FileAttributes::default()
    };
    let d = provider::entry("nas", ".config", &dir, None);
    assert_eq!(d.kind, EntryKind::Dir);
    assert_eq!(d.size, None);
    assert!(d.attrs.contains(Attrs::HIDDEN));
    assert!(d.attrs.contains(Attrs::READONLY));

    let link = FileAttributes {
        permissions: Some(0o120_777),
        ..FileAttributes::default()
    };
    let l = provider::entry("nas", "latest", &link, Some(("/srv/v2".to_owned(), true)));
    assert!(matches!(l.kind, EntryKind::Symlink { broken: true, .. }));
}

#[test]
fn the_schema_names_the_fields_and_the_secrets() {
    let schema = schema();
    assert_eq!(schema.title, "SFTP");
    let secrets: Vec<&str> = schema.secret_fields().collect();
    assert_eq!(secrets, ["passphrase", "password"]);
    assert!(schema.field("host").is_some_and(|f| f.required));
    assert!(
        schema
            .field("key_file")
            .is_some_and(|f| f.kind == ConnectionFieldKind::Path)
    );
    assert!(matches!(
        schema.field("port").map(|f| &f.kind),
        Some(ConnectionFieldKind::Integer { default: 22, .. })
    ));
}

#[test]
fn the_extension_registers_a_provider_and_a_connector_with_the_platform() {
    let ext = SftpExtension::new();
    assert_eq!(ext.manifest().id, ID);
    assert!(matches!(
        Registry::new().load(&ext),
        Err(types::Error::Unsupported(_))
    ));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut registry = Registry::with_platform(fake);
    registry.load(&ext).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(registry.providers().len(), 1);
    let connector = registry
        .connector_of(PROVIDER_ID)
        .unwrap_or_else(|| panic!("connector"));
    assert_eq!(connector.schema().title, "SFTP");
    let provider = &registry.providers()[0];
    assert_eq!(provider.id(), PROVIDER_ID);
    assert_eq!(
        provider.capabilities(&Location::new(PROVIDER_ID, "/")),
        Capabilities::REMOTE
    );
    assert!(
        provider
            .capabilities(&Location::new(PROVIDER_ID, "/nas/x"))
            .contains(Capabilities::NATIVE_TRANSFER | Capabilities::REMOTE | Capabilities::WRITE)
    );
}

#[test]
fn the_root_lists_the_connections_without_connecting() {
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let conns = Arc::new(SftpConnections::new(fake));
    let provider = SftpProvider::new(Arc::clone(&conns));
    let mut nas = ext::Connection {
        name: "nas".into(),
        ..ext::Connection::default()
    };
    nas.settings.insert("host".into(), "192.0.2.1".into());
    conns.update(vec![
        nas,
        ext::Connection {
            name: "box".into(),
            ..ext::Connection::default()
        },
    ]);
    assert_eq!(conns.names(), ["box", "nas"]);
    assert!(!conns.is_connected("nas"));
    runtime::block_on(async {
        let root = Location::new(PROVIDER_ID, "/");
        let batches: Vec<_> = provider
            .list(
                &root,
                ext::ListOptions::default(),
                types::CancelToken::new(),
            )
            .collect()
            .await;
        let names: Vec<String> = batches
            .into_iter()
            .flat_map(Result::unwrap_or_default)
            .map(|e| e.name)
            .collect();
        assert_eq!(names, ["box", "nas"]);
        let row = provider
            .stat(&Location::new(PROVIDER_ID, "/nas"))
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(row.kind, EntryKind::Dir);
        assert_eq!(row.path.to_string(), "/nas");
        assert!(matches!(
            provider.stat(&Location::new(PROVIDER_ID, "/gone")).await,
            Err(types::Error::NotFound)
        ));
        // A connection without a host fails to connect with a message of
        // the plugin's own, not a panic.
        assert!(matches!(
            conns.connect("box").await,
            Err(types::Error::Extension { .. })
        ));
    });
    // Forgetting a connection drops it from the list.
    conns.update(Vec::new());
    assert!(conns.names().is_empty());
}

/// The whole round trip against a real server (issue 21): `RICK_SFTP_HOST`,
/// `RICK_SFTP_PORT`, `RICK_SFTP_USER` and `RICK_SFTP_PASSWORD` (or
/// `RICK_SFTP_KEY_FILE`) name it, `RICK_SFTP_FOLDER` the folder to work in
/// (a chrooted user's writable one); the test skips with a note without
/// them, and fails instead under `RICK_REQUIRE_CONTAINERS`.
#[test]
// One scenario against one server, start to finish.
#[allow(clippy::too_many_lines)]
fn against_a_server_list_upload_download_rename_move_and_delete_round_trip() {
    use ext::{ListOptions, RemoveMode, TransferMode, WriteOptions};
    use platform::{ConfigDir as _, Secrets as _};
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    let Ok(host) = std::env::var("RICK_SFTP_HOST") else {
        eprintln!("RICK_SFTP_HOST is not set: no SFTP server to test against; skipped");
        assert!(
            std::env::var_os("RICK_REQUIRE_CONTAINERS").is_none(),
            "RICK_REQUIRE_CONTAINERS is set but RICK_SFTP_HOST is not"
        );
        return;
    };
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let conns = Arc::new(SftpConnections::new(fake.clone()));
    let provider = SftpProvider::new(Arc::clone(&conns));
    let mut conn = ext::Connection {
        name: "test".into(),
        ..ext::Connection::default()
    };
    conn.settings.insert("host".into(), host);
    conn.settings.insert(
        "port".into(),
        std::env::var("RICK_SFTP_PORT").unwrap_or_else(|_| "22".into()),
    );
    conn.settings.insert(
        "user".into(),
        std::env::var("RICK_SFTP_USER").unwrap_or_else(|_| "rick".into()),
    );
    if let Ok(folder) = std::env::var("RICK_SFTP_FOLDER") {
        conn.settings.insert("folder".into(), folder);
    }
    if let Ok(key) = std::env::var("RICK_SFTP_KEY_FILE") {
        conn.settings.insert("key_file".into(), key);
    } else {
        fake.store_secret(
            &ext::secret_key(PROVIDER_ID, "test", "password"),
            &std::env::var("RICK_SFTP_PASSWORD").unwrap_or_default(),
        )
        .unwrap_or_else(|e| panic!("{e}"));
    }
    conns.update(vec![conn]);
    let unique = format!(
        "rick-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis())
    );
    let at = |rest: &str| location("test", rest);
    let list = |provider: &SftpProvider, rest: &str| {
        let stream = provider.list(
            &at(rest),
            ListOptions {
                hidden: true,
                ..ListOptions::default()
            },
            types::CancelToken::new(),
        );
        async move {
            let batches: Vec<_> = stream.collect().await;
            let mut names: Vec<String> = batches
                .into_iter()
                .flat_map(|b| b.unwrap_or_else(|e| panic!("{e}")))
                .map(|e| e.name)
                .collect();
            names.sort();
            names
        }
    };
    runtime::block_on(async {
        conns
            .connect("test")
            .await
            .unwrap_or_else(|e| panic!("connect: {e}"));
        assert!(conns.is_connected("test"));
        // The login folder lists; it may be empty.
        let _ = list(&provider, "").await;

        // A folder, a file written and read back.
        provider
            .create_dir(&at(&unique))
            .await
            .unwrap_or_else(|e| panic!("mkdir: {e}"));
        assert!(matches!(
            provider.create_dir(&at(&unique)).await,
            Err(types::Error::Io(e)) if e.kind() == std::io::ErrorKind::AlreadyExists
        ));
        let payload: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        {
            let mut w = provider
                .open_write(
                    &at(&format!("{unique}/data.bin")),
                    WriteOptions { overwrite: false },
                )
                .await
                .unwrap_or_else(|e| panic!("open_write: {e}"));
            w.write_all(&payload)
                .await
                .unwrap_or_else(|e| panic!("{e}"));
            w.shutdown().await.unwrap_or_else(|e| panic!("{e}"));
        }
        assert!(matches!(
            provider
                .open_write(&at(&format!("{unique}/data.bin")), WriteOptions { overwrite: false })
                .await,
            Err(types::Error::Io(e)) if e.kind() == std::io::ErrorKind::AlreadyExists
        ));
        let stat = provider
            .stat(&at(&format!("{unique}/data.bin")))
            .await
            .unwrap_or_else(|e| panic!("stat: {e}"));
        assert_eq!(stat.kind, EntryKind::File);
        assert_eq!(stat.size, Some(payload.len() as u64));
        let mut back = Vec::new();
        provider
            .open_read(&at(&format!("{unique}/data.bin")))
            .await
            .unwrap_or_else(|e| panic!("open_read: {e}"))
            .read_to_end(&mut back)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back, payload);
        assert_eq!(list(&provider, &unique).await, ["data.bin"]);

        // Rename, then a native move into a subfolder: a remote rename.
        provider
            .rename(
                &at(&format!("{unique}/data.bin")),
                &at(&format!("{unique}/renamed.bin")),
            )
            .await
            .unwrap_or_else(|e| panic!("rename: {e}"));
        provider
            .create_dir(&at(&format!("{unique}/sub")))
            .await
            .unwrap_or_else(|e| panic!("mkdir sub: {e}"));
        let (sink, _rx) = ext::ProgressSink::channel();
        provider
            .transfer(
                &[at(&format!("{unique}/renamed.bin"))],
                &at(&format!("{unique}/sub")),
                TransferMode::Move,
                sink.clone(),
                types::CancelToken::new(),
            )
            .await
            .unwrap_or_else(|e| panic!("transfer: {e}"));
        assert_eq!(list(&provider, &unique).await, ["sub"]);
        assert_eq!(
            list(&provider, &format!("{unique}/sub")).await,
            ["renamed.bin"]
        );
        assert!(matches!(
            provider
                .transfer(
                    &[at(&format!("{unique}/sub/renamed.bin"))],
                    &at(&unique),
                    TransferMode::Copy,
                    sink,
                    types::CancelToken::new(),
                )
                .await,
            Err(types::Error::Unsupported(_))
        ));

        // A symbolic link, then the whole folder removed.
        let link = provider
            .link(
                &at(&format!("{unique}/sub/renamed.bin")),
                &at(&format!("{unique}/link")),
                ext::LinkKind::Symbolic,
            )
            .await
            .unwrap_or_else(|e| panic!("link: {e}"));
        assert!(
            matches!(link.kind, EntryKind::Symlink { broken: false, .. }),
            "{link:?}"
        );
        assert!(matches!(
            provider.remove(&at(&unique), RemoveMode::Trash).await,
            Err(types::Error::Unsupported(_))
        ));
        provider
            .remove(&at(&unique), RemoveMode::Permanent)
            .await
            .unwrap_or_else(|e| panic!("remove: {e}"));
        assert!(matches!(
            provider.stat(&at(&unique)).await,
            Err(types::Error::NotFound)
        ));

        // Disconnect drops the session; the next operation reconnects.
        conns.disconnect("test");
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert!(!conns.is_connected("test"));
        let _ = list(&provider, "").await;
        assert!(conns.is_connected("test"));
    });
    // The host key was learnt into the fake home's known_hosts.
    let known = fake.home_dir().join(".ssh").join("known_hosts");
    assert!(known.is_file(), "{}", known.display());
}
