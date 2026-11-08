use std::sync::Arc;

use ext::{
    Capabilities, ConnectionFieldKind, Connector as _, Extension as _, Provider as _, Registry,
};
use platform::Fake;
use types::{EntryKind, Location};

use super::client::{Unsigned, parse_containers, parse_page};
use super::*;

#[test]
fn places_parse_from_locations() {
    let at = |p: &str| Place::of(&Location::new(PROVIDER_ID, p));
    assert_eq!(at("/"), Place::Root);
    assert_eq!(at("/work"), Place::Account("work".into()));
    assert_eq!(
        at("/work/photos/"),
        Place::Container {
            account: "work".into(),
            container: "photos".into()
        }
    );
    assert_eq!(
        at("/work/photos/2026/a b.jpg"),
        Place::Blob {
            account: "work".into(),
            container: "photos".into(),
            name: "2026/a b.jpg".into()
        }
    );
    let place = at("/work/photos/2026/a.jpg");
    assert_eq!(
        place.location().to_string(),
        "azure:/work/photos/2026/a.jpg"
    );
    assert_eq!(place.account(), Some("work"));
}

/// The string to sign is the one the service documents: with the
/// well-known emulator account and key the signature of a fixed request
/// is reproducible.
#[test]
fn the_shared_key_signature_is_stable_and_follows_the_scheme() {
    let key = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        "Eby8vdM02xNOcqFlqUwJPLlmEtlCDXJ1OUzFT50uSRZ6IFsuFq2UVErCz4I6tq/K1SZFPTOtr/KBHBeksoGMGw==",
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let req = Unsigned {
        method: "GET",
        path: "/devstoreaccount1/",
        query: &[("comp", "list")],
        ms_headers: &[
            ("x-ms-version", "2021-12-02"),
            ("x-ms-date", "Sun, 08 Nov 2026 03:00:00 GMT"),
        ],
        content_length: "",
        content_type: "",
    };
    let a = sign(&req, "devstoreaccount1", &key);
    let b = sign(&req, "devstoreaccount1", &key);
    assert_eq!(a, b);
    assert!(a.starts_with("SharedKey devstoreaccount1:"), "{a}");
    assert_eq!(
        a, "SharedKey devstoreaccount1:AgTx2PSEln//D62iWfy3x0qvjgwOOzTFHm1kE3J/pWo=",
        "the signature of the documented string to sign"
    );
    // The headers' order does not matter: the scheme sorts them.
    let swapped = Unsigned {
        ms_headers: &[
            ("x-ms-date", "Sun, 08 Nov 2026 03:00:00 GMT"),
            ("x-ms-version", "2021-12-02"),
        ],
        ..req
    };
    assert_eq!(sign(&swapped, "devstoreaccount1", &key), a);
    // A body changes it.
    let with_body = Unsigned {
        content_length: "5",
        content_type: "application/octet-stream",
        ..req
    };
    assert_ne!(sign(&with_body, "devstoreaccount1", &key), a);
}

#[test]
fn listings_parse() {
    let page = parse_page(
        r#"<?xml version="1.0" encoding="utf-8"?>
<EnumerationResults ServiceEndpoint="http://127.0.0.1:10000/devstoreaccount1" ContainerName="photos">
  <Prefix>2026/</Prefix><Delimiter>/</Delimiter>
  <Blobs>
    <Blob><Name>2026/</Name><Properties><Content-Length>0</Content-Length><Last-Modified>Sun, 08 Nov 2026 03:29:15 GMT</Last-Modified></Properties></Blob>
    <Blob><Name>2026/a.jpg</Name><Properties><Content-Length>1234</Content-Length><Last-Modified>Sun, 08 Nov 2026 03:29:15 GMT</Last-Modified></Properties></Blob>
    <BlobPrefix><Name>2026/raw/</Name></BlobPrefix>
  </Blobs>
  <NextMarker>2!abc</NextMarker>
</EnumerationResults>"#,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(page.prefixes, ["2026/raw/"]);
    assert_eq!(page.blobs.len(), 2);
    assert_eq!(page.blobs[1].size, 1234);
    assert!(page.blobs[1].modified.is_some());
    assert_eq!(page.next.as_deref(), Some("2!abc"));
    let entries = AzureProvider::page_entries("work", "photos", "2026/", page, true);
    let names: Vec<(String, EntryKind)> = entries
        .iter()
        .map(|e| (e.name.clone(), e.kind.clone()))
        .collect();
    assert_eq!(
        names,
        [
            ("raw".to_owned(), EntryKind::Dir),
            ("a.jpg".to_owned(), EntryKind::File)
        ]
    );
    let (containers, next) = parse_containers(
        r"<EnumerationResults><Containers><Container><Name>photos</Name><Properties><Last-Modified>Sun, 08 Nov 2026 03:29:15 GMT</Last-Modified></Properties></Container></Containers><NextMarker/></EnumerationResults>",
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(containers.len(), 1);
    assert_eq!(containers[0].name, "photos");
    assert!(containers[0].modified.is_some());
    assert_eq!(next, None);
}

#[test]
fn the_schema_and_the_extension_register() {
    let schema = schema();
    assert_eq!(schema.title, "Azure Blob");
    assert_eq!(schema.secret_fields().collect::<Vec<_>>(), ["key"]);
    assert!(schema.field("account").is_some_and(|f| f.required));
    assert!(matches!(
        schema.field("endpoint").map(|f| &f.kind),
        Some(ConnectionFieldKind::Text { .. })
    ));
    let ext = AzureExtension::new();
    assert_eq!(ext.manifest().id, ID);
    assert!(matches!(
        Registry::new().load(&ext),
        Err(types::Error::Unsupported(_))
    ));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut registry = Registry::with_platform(fake);
    registry.load(&ext).unwrap_or_else(|e| panic!("{e}"));
    let provider = &registry.providers()[0];
    assert_eq!(provider.id(), PROVIDER_ID);
    assert!(registry.connector_of(PROVIDER_ID).is_some());
    assert_eq!(
        provider.capabilities(&Location::new(PROVIDER_ID, "/")),
        Capabilities::REMOTE
    );
    assert!(
        provider
            .capabilities(&Location::new(PROVIDER_ID, "/work/photos/a"))
            .contains(Capabilities::NATIVE_TRANSFER | Capabilities::WRITE | Capabilities::REMOTE)
    );
}

#[test]
fn the_root_lists_the_accounts_without_a_request() {
    use futures::StreamExt as _;
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let conns = Arc::new(AzureConnections::new(fake));
    let provider = AzureProvider::new(Arc::clone(&conns));
    conns.update(vec![ext::Connection {
        name: "work".into(),
        ..ext::Connection::default()
    }]);
    runtime::block_on(async {
        let batches: Vec<_> = provider
            .list(
                &Location::new(PROVIDER_ID, "/"),
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
        assert_eq!(names, ["work"]);
        assert!(matches!(
            conns.connect("work").await,
            Err(types::Error::Extension { .. })
        ));
        assert!(!conns.is_connected("work"));
    });
}

/// The whole round trip against a real store (issue 21): `RICK_AZURE_ENDPOINT`
/// names an emulator (Azurite in CI), `RICK_AZURE_ACCOUNT` and
/// `RICK_AZURE_KEY` the account (the emulator's well-known ones by
/// default); the test skips with a note without the endpoint, and fails
/// instead under `RICK_REQUIRE_CONTAINERS`.
#[test]
// One scenario against one store, start to finish.
#[allow(clippy::too_many_lines)]
fn against_a_store_list_upload_download_move_across_containers_and_delete_round_trip() {
    use ext::{ListOptions, RemoveMode, TransferMode, WriteOptions};
    use futures::StreamExt as _;
    use platform::Secrets as _;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    let Ok(endpoint) = std::env::var("RICK_AZURE_ENDPOINT") else {
        eprintln!("RICK_AZURE_ENDPOINT is not set: no blob store to test against; skipped");
        assert!(
            std::env::var_os("RICK_REQUIRE_CONTAINERS").is_none(),
            "RICK_REQUIRE_CONTAINERS is set but RICK_AZURE_ENDPOINT is not"
        );
        return;
    };
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let conns = Arc::new(AzureConnections::new(fake.clone()));
    let provider = AzureProvider::new(Arc::clone(&conns));
    let mut conn = ext::Connection {
        name: "test".into(),
        ..ext::Connection::default()
    };
    conn.settings.insert("endpoint".into(), endpoint);
    conn.settings.insert(
        "account".into(),
        std::env::var("RICK_AZURE_ACCOUNT").unwrap_or_else(|_| "devstoreaccount1".into()),
    );
    fake.store_secret(
        &ext::secret_key(PROVIDER_ID, "test", "key"),
        &std::env::var("RICK_AZURE_KEY").unwrap_or_else(|_| {
            "Eby8vdM02xNOcqFlqUwJPLlmEtlCDXJ1OUzFT50uSRZ6IFsuFq2UVErCz4I6tq/K1SZFPTOtr/KBHBeksoGMGw==".into()
        }),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    conns.update(vec![conn]);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let container_a = format!("rick-test-a-{stamp}");
    let container_b = format!("rick-test-b-{stamp}");
    let at = |rest: &str| Location::new(PROVIDER_ID, format!("/test/{rest}"));
    let list = |provider: &AzureProvider, rest: &str| {
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
        for container in [&container_a, &container_b] {
            provider
                .create_dir(&at(container))
                .await
                .unwrap_or_else(|e| panic!("create container: {e}"));
        }
        let containers = list(&provider, "").await;
        assert!(
            containers.contains(&container_a) && containers.contains(&container_b),
            "{containers:?}"
        );

        provider
            .create_dir(&at(&format!("{container_a}/photos")))
            .await
            .unwrap_or_else(|e| panic!("mkdir: {e}"));
        let payload: Vec<u8> = (0..20 * 1024 * 1024u32).map(|i| (i % 241) as u8).collect();
        {
            let mut w = provider
                .open_write(
                    &at(&format!("{container_a}/photos/big.bin")),
                    WriteOptions { overwrite: false },
                )
                .await
                .unwrap_or_else(|e| panic!("open_write: {e}"));
            w.write_all(&payload)
                .await
                .unwrap_or_else(|e| panic!("{e}"));
            w.shutdown()
                .await
                .unwrap_or_else(|e| panic!("shutdown: {e}"));
        }
        let stat = provider
            .stat(&at(&format!("{container_a}/photos/big.bin")))
            .await
            .unwrap_or_else(|e| panic!("stat: {e}"));
        assert_eq!(stat.kind, EntryKind::File);
        assert_eq!(stat.size, Some(payload.len() as u64));
        let mut back = Vec::new();
        provider
            .open_read(&at(&format!("{container_a}/photos/big.bin")))
            .await
            .unwrap_or_else(|e| panic!("open_read: {e}"))
            .read_to_end(&mut back)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(back == payload, "the download is the upload");
        assert_eq!(list(&provider, &container_a).await, ["photos"]);
        assert_eq!(
            list(&provider, &format!("{container_a}/photos")).await,
            ["big.bin"]
        );

        // An upload dropped half-way leaves no blob.
        {
            let mut w = provider
                .open_write(
                    &at(&format!("{container_a}/photos/partial.bin")),
                    WriteOptions { overwrite: false },
                )
                .await
                .unwrap_or_else(|e| panic!("{e}"));
            w.write_all(&payload[..9 * 1024 * 1024])
                .await
                .unwrap_or_else(|e| panic!("{e}"));
            drop(w);
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        assert!(matches!(
            provider
                .stat(&at(&format!("{container_a}/photos/partial.bin")))
                .await,
            Err(types::Error::NotFound)
        ));

        // A small blob, a folder moved within the container, then across.
        {
            let mut w = provider
                .open_write(
                    &at(&format!("{container_a}/notes.txt")),
                    WriteOptions { overwrite: true },
                )
                .await
                .unwrap_or_else(|e| panic!("{e}"));
            w.write_all(b"hello")
                .await
                .unwrap_or_else(|e| panic!("{e}"));
            w.shutdown().await.unwrap_or_else(|e| panic!("{e}"));
        }
        provider
            .create_dir(&at(&format!("{container_a}/archive")))
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        let (sink, mut rx) = ext::ProgressSink::channel();
        provider
            .transfer(
                &[
                    at(&format!("{container_a}/photos")),
                    at(&format!("{container_a}/notes.txt")),
                ],
                &at(&format!("{container_a}/archive")),
                TransferMode::Move,
                sink,
                types::CancelToken::new(),
            )
            .await
            .unwrap_or_else(|e| panic!("transfer: {e}"));
        assert!(matches!(
            *rx.borrow_and_update(),
            types::Progress::Bytes { done, total: Some(total) } if done == total && total == payload.len() as u64 + 5
        ));
        assert_eq!(list(&provider, &container_a).await, ["archive"]);
        assert_eq!(
            list(&provider, &format!("{container_a}/archive")).await,
            ["notes.txt", "photos"]
        );
        let (sink, _rx) = ext::ProgressSink::channel();
        provider
            .transfer(
                &[at(&format!("{container_a}/archive"))],
                &at(&container_b),
                TransferMode::Copy,
                sink,
                types::CancelToken::new(),
            )
            .await
            .unwrap_or_else(|e| panic!("transfer across containers: {e}"));
        assert_eq!(list(&provider, &container_b).await, ["archive"]);
        assert_eq!(list(&provider, &container_a).await, ["archive"]);
        let mut small = String::new();
        provider
            .open_read(&at(&format!("{container_b}/archive/notes.txt")))
            .await
            .unwrap_or_else(|e| panic!("{e}"))
            .read_to_string(&mut small)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(small, "hello");

        provider
            .remove(
                &at(&format!("{container_a}/archive")),
                RemoveMode::Permanent,
            )
            .await
            .unwrap_or_else(|e| panic!("remove: {e}"));
        assert_eq!(list(&provider, &container_a).await, Vec::<String>::new());
        for container in [&container_a, &container_b] {
            provider
                .remove(&at(container), RemoveMode::Permanent)
                .await
                .unwrap_or_else(|e| panic!("remove container: {e}"));
        }
        let containers = list(&provider, "").await;
        assert!(!containers.contains(&container_a) && !containers.contains(&container_b));
    });
}
