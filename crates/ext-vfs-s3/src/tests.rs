use std::sync::Arc;

use ext::{
    Capabilities, ConnectionFieldKind, Connector as _, Extension as _, Provider as _, Registry,
};
use platform::Fake;
use types::{EntryKind, Location};

use super::client::{Unsigned, parse_buckets, parse_page};
use super::*;

#[test]
fn places_parse_from_locations() {
    let at = |p: &str| Place::of(&Location::new(PROVIDER_ID, p));
    assert_eq!(at("/"), Place::Root);
    assert_eq!(at(""), Place::Root);
    assert_eq!(at("/work"), Place::Account("work".into()));
    assert_eq!(at("/work/"), Place::Account("work".into()));
    assert_eq!(
        at("/work/photos"),
        Place::Bucket {
            account: "work".into(),
            bucket: "photos".into()
        }
    );
    assert_eq!(
        at("/work/photos/2026/a b.jpg"),
        Place::Key {
            account: "work".into(),
            bucket: "photos".into(),
            key: "2026/a b.jpg".into()
        }
    );
    assert_eq!(
        at("\\work\\photos\\2026\\"),
        Place::Key {
            account: "work".into(),
            bucket: "photos".into(),
            key: "2026".into()
        }
    );
    let place = at("/work/photos/2026/a.jpg");
    assert_eq!(place.location().to_string(), "s3:/work/photos/2026/a.jpg");
    assert_eq!(place.account(), Some("work"));
    assert_eq!(Place::Root.account(), None);
}

/// The example of the AWS documentation ("Signature calculations for the
/// authorization header: a GET object"): the well-known test credentials
/// and date give the documented signature.
#[test]
fn signature_version_4_matches_the_documented_example() {
    let signed = sign(
        &Unsigned {
            method: "GET",
            host: "examplebucket.s3.amazonaws.com",
            path: "/test.txt",
            query: &[],
            headers: &[("range", "bytes=0-9")],
            payload_hash: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            amz_date: "20130524T000000Z",
        },
        "AKIAIOSFODNN7EXAMPLE",
        "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        "us-east-1",
    );
    let authorization = signed
        .iter()
        .find(|(k, _)| k == "authorization")
        .map(|(_, v)| v.as_str())
        .unwrap_or_default();
    assert_eq!(
        authorization,
        "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, SignedHeaders=host;range;x-amz-content-sha256;x-amz-date, Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
    );
    // The documented "get bucket (list objects)" example, with a query.
    let signed = sign(
        &Unsigned {
            method: "GET",
            host: "examplebucket.s3.amazonaws.com",
            path: "/",
            query: &[("max-keys", "2"), ("prefix", "J")],
            headers: &[],
            payload_hash: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            amz_date: "20130524T000000Z",
        },
        "AKIAIOSFODNN7EXAMPLE",
        "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        "us-east-1",
    );
    assert!(
        signed[0].1.ends_with(
            "Signature=34b48302e7b5fa45bde8084f4b7868a86f0a534bc59db6670ed5711ef69dc6f7"
        ),
        "{}",
        signed[0].1
    );
}

#[test]
fn keys_are_encoded_once_with_their_slashes_kept() {
    assert_eq!(
        client::encode_path("a b/c+d/é.txt"),
        "a%20b/c%2Bd/%C3%A9.txt"
    );
    assert_eq!(client::encode("a/b c"), "a%2Fb%20c");
    assert_eq!(client::encode("unreserved-_.~"), "unreserved-_.~");
}

#[test]
fn listings_and_bucket_lists_parse() {
    let page = parse_page(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <Name>photos</Name><Prefix>2026/</Prefix><Delimiter>/</Delimiter><KeyCount>3</KeyCount>
  <IsTruncated>true</IsTruncated><NextContinuationToken>tok==</NextContinuationToken>
  <Contents><Key>2026/</Key><Size>0</Size><LastModified>2026-11-08T03:29:15.000Z</LastModified></Contents>
  <Contents><Key>2026/a.jpg</Key><Size>1234</Size><LastModified>2026-11-08T03:29:15.000Z</LastModified></Contents>
  <CommonPrefixes><Prefix>2026/raw/</Prefix></CommonPrefixes>
</ListBucketResult>"#,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(page.prefixes, ["2026/raw/"]);
    assert_eq!(page.objects.len(), 2);
    assert_eq!(page.objects[1].key, "2026/a.jpg");
    assert_eq!(page.objects[1].size, 1234);
    assert!(page.objects[1].modified.is_some());
    assert_eq!(page.next.as_deref(), Some("tok=="));
    let entries = S3Provider::page_entries("work", "photos", "2026/", page, true);
    let names: Vec<(String, EntryKind)> = entries
        .iter()
        .map(|e| (e.name.clone(), e.kind.clone()))
        .collect();
    assert_eq!(
        names,
        [
            ("raw".to_owned(), EntryKind::Dir),
            ("a.jpg".to_owned(), EntryKind::File)
        ],
        "the marker of the folder itself is no entry"
    );
    assert_eq!(entries[0].path.to_string(), "/work/photos/2026/raw");
    assert_eq!(entries[1].size, Some(1234));

    let buckets = parse_buckets(
        r#"<ListAllMyBucketsResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Buckets><Bucket><Name>photos</Name><CreationDate>2026-01-02T03:04:05.000Z</CreationDate></Bucket><Bucket><Name>backup</Name></Bucket></Buckets></ListAllMyBucketsResult>"#,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(buckets.len(), 2);
    assert_eq!(buckets[0].name, "photos");
    assert!(buckets[0].created.is_some());
    assert!(buckets[1].created.is_none());
    assert!(parse_page("not xml").is_err());
}

#[test]
fn the_schema_names_the_fields_and_the_secret() {
    let schema = schema();
    assert_eq!(schema.title, "S3");
    assert_eq!(schema.secret_fields().collect::<Vec<_>>(), ["secret_key"]);
    assert!(schema.field("access_key").is_some_and(|f| f.required));
    assert!(matches!(
        schema.field("path_style").map(|f| &f.kind),
        Some(ConnectionFieldKind::Bool { default: true })
    ));
}

#[test]
fn the_extension_registers_a_provider_and_a_connector_with_the_platform() {
    let ext = S3Extension::new();
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
    assert_eq!(
        provider.capabilities(&Location::new(PROVIDER_ID, "/work")),
        Capabilities::REMOTE | Capabilities::WRITE
    );
    assert!(
        provider
            .capabilities(&Location::new(PROVIDER_ID, "/work/photos/a"))
            .contains(Capabilities::NATIVE_TRANSFER | Capabilities::WRITE | Capabilities::REMOTE)
    );
    assert!(
        !provider
            .capabilities(&Location::new(PROVIDER_ID, "/work/photos/a"))
            .contains(Capabilities::RENAME),
        "no rename in an object store"
    );
}

#[test]
fn the_root_lists_the_accounts_without_a_request() {
    use futures::StreamExt as _;
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let conns = Arc::new(S3Connections::new(fake));
    let provider = S3Provider::new(Arc::clone(&conns));
    conns.update(vec![
        ext::Connection {
            name: "work".into(),
            ..ext::Connection::default()
        },
        ext::Connection {
            name: "home".into(),
            ..ext::Connection::default()
        },
    ]);
    assert!(!conns.is_connected("work"));
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
        assert_eq!(names, ["home", "work"]);
        let row = provider
            .stat(&Location::new(PROVIDER_ID, "/work"))
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(row.kind, EntryKind::Dir);
        // Without an access key the connection cannot be used, and says so.
        assert!(matches!(
            conns.connect("work").await,
            Err(types::Error::Extension { .. })
        ));
    });
}

/// The whole round trip against a real store (issue 21): `RICK_S3_ENDPOINT`,
/// `RICK_S3_ACCESS_KEY` and `RICK_S3_SECRET_KEY` name it (`MinIO` in CI);
/// the test skips with a note without them, and fails instead under
/// `RICK_REQUIRE_CONTAINERS`.
#[test]
// One scenario against one store, start to finish.
#[allow(clippy::too_many_lines)]
fn against_a_store_list_upload_download_move_across_buckets_and_delete_round_trip() {
    use ext::{ListOptions, RemoveMode, TransferMode, WriteOptions};
    use futures::StreamExt as _;
    use platform::Secrets as _;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    let Ok(endpoint) = std::env::var("RICK_S3_ENDPOINT") else {
        eprintln!("RICK_S3_ENDPOINT is not set: no S3 store to test against; skipped");
        assert!(
            std::env::var_os("RICK_REQUIRE_CONTAINERS").is_none(),
            "RICK_REQUIRE_CONTAINERS is set but RICK_S3_ENDPOINT is not"
        );
        return;
    };
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let conns = Arc::new(S3Connections::new(fake.clone()));
    let provider = S3Provider::new(Arc::clone(&conns));
    let mut conn = ext::Connection {
        name: "test".into(),
        ..ext::Connection::default()
    };
    conn.settings.insert("endpoint".into(), endpoint);
    conn.settings.insert(
        "access_key".into(),
        std::env::var("RICK_S3_ACCESS_KEY").unwrap_or_else(|_| "minioadmin".into()),
    );
    fake.store_secret(
        &ext::secret_key(PROVIDER_ID, "test", "secret_key"),
        &std::env::var("RICK_S3_SECRET_KEY").unwrap_or_else(|_| "minioadmin".into()),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    conns.update(vec![conn]);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let bucket_a = format!("rick-test-a-{stamp}");
    let bucket_b = format!("rick-test-b-{stamp}");
    let at = |rest: &str| Location::new(PROVIDER_ID, format!("/test/{rest}"));
    let list = |provider: &S3Provider, rest: &str| {
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
        for bucket in [&bucket_a, &bucket_b] {
            provider
                .create_dir(&at(bucket))
                .await
                .unwrap_or_else(|e| panic!("create bucket: {e}"));
        }
        let buckets = list(&provider, "").await;
        assert!(
            buckets.contains(&bucket_a) && buckets.contains(&bucket_b),
            "{buckets:?}"
        );

        // A folder and a file below it: 20 MiB, so the upload is multipart.
        provider
            .create_dir(&at(&format!("{bucket_a}/photos")))
            .await
            .unwrap_or_else(|e| panic!("mkdir: {e}"));
        let payload: Vec<u8> = (0..20 * 1024 * 1024u32).map(|i| (i % 253) as u8).collect();
        {
            let mut w = provider
                .open_write(
                    &at(&format!("{bucket_a}/photos/big.bin")),
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
        assert!(matches!(
            provider
                .open_write(&at(&format!("{bucket_a}/photos/big.bin")), WriteOptions { overwrite: false })
                .await,
            Err(types::Error::Io(e)) if e.kind() == std::io::ErrorKind::AlreadyExists
        ));
        let stat = provider
            .stat(&at(&format!("{bucket_a}/photos/big.bin")))
            .await
            .unwrap_or_else(|e| panic!("stat: {e}"));
        assert_eq!(stat.kind, EntryKind::File);
        assert_eq!(stat.size, Some(payload.len() as u64));
        let dir = provider
            .stat(&at(&format!("{bucket_a}/photos")))
            .await
            .unwrap_or_else(|e| panic!("stat dir: {e}"));
        assert_eq!(dir.kind, EntryKind::Dir);
        let mut back = Vec::new();
        provider
            .open_read(&at(&format!("{bucket_a}/photos/big.bin")))
            .await
            .unwrap_or_else(|e| panic!("open_read: {e}"))
            .read_to_end(&mut back)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back.len(), payload.len());
        assert!(back == payload, "the download is the upload");
        assert_eq!(list(&provider, &bucket_a).await, ["photos"]);
        assert_eq!(
            list(&provider, &format!("{bucket_a}/photos")).await,
            ["big.bin"]
        );

        // An upload dropped half-way leaves no object.
        {
            let mut w = provider
                .open_write(
                    &at(&format!("{bucket_a}/photos/partial.bin")),
                    WriteOptions { overwrite: false },
                )
                .await
                .unwrap_or_else(|e| panic!("open_write: {e}"));
            w.write_all(&payload[..9 * 1024 * 1024])
                .await
                .unwrap_or_else(|e| panic!("{e}"));
            drop(w);
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        assert!(matches!(
            provider
                .stat(&at(&format!("{bucket_a}/photos/partial.bin")))
                .await,
            Err(types::Error::NotFound)
        ));

        // A small file, a folder moved within the bucket, then the folder
        // moved across to the other bucket: server-side copies.
        {
            let mut w = provider
                .open_write(
                    &at(&format!("{bucket_a}/notes.txt")),
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
            .create_dir(&at(&format!("{bucket_a}/archive")))
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        let (sink, mut rx) = ext::ProgressSink::channel();
        provider
            .transfer(
                &[
                    at(&format!("{bucket_a}/photos")),
                    at(&format!("{bucket_a}/notes.txt")),
                ],
                &at(&format!("{bucket_a}/archive")),
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
        assert_eq!(list(&provider, &bucket_a).await, ["archive"]);
        assert_eq!(
            list(&provider, &format!("{bucket_a}/archive")).await,
            ["notes.txt", "photos"]
        );
        assert_eq!(
            list(&provider, &format!("{bucket_a}/archive/photos")).await,
            ["big.bin"]
        );
        let (sink, _rx) = ext::ProgressSink::channel();
        provider
            .transfer(
                &[at(&format!("{bucket_a}/archive"))],
                &at(&bucket_b),
                TransferMode::Copy,
                sink,
                types::CancelToken::new(),
            )
            .await
            .unwrap_or_else(|e| panic!("transfer across buckets: {e}"));
        assert_eq!(list(&provider, &bucket_b).await, ["archive"]);
        assert_eq!(
            list(&provider, &bucket_a).await,
            ["archive"],
            "a copy keeps the source"
        );
        let mut small = String::new();
        provider
            .open_read(&at(&format!("{bucket_b}/archive/notes.txt")))
            .await
            .unwrap_or_else(|e| panic!("{e}"))
            .read_to_string(&mut small)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(small, "hello");

        // Delete a folder, then both buckets.
        assert!(matches!(
            provider
                .remove(&at(&format!("{bucket_a}/archive")), RemoveMode::Trash)
                .await,
            Err(types::Error::Unsupported(_))
        ));
        provider
            .remove(&at(&format!("{bucket_a}/archive")), RemoveMode::Permanent)
            .await
            .unwrap_or_else(|e| panic!("remove: {e}"));
        assert_eq!(list(&provider, &bucket_a).await, Vec::<String>::new());
        for bucket in [&bucket_a, &bucket_b] {
            provider
                .remove(&at(bucket), RemoveMode::Permanent)
                .await
                .unwrap_or_else(|e| panic!("remove bucket: {e}"));
        }
        let buckets = list(&provider, "").await;
        assert!(
            !buckets.contains(&bucket_a) && !buckets.contains(&bucket_b),
            "{buckets:?}"
        );
        conns.disconnect("test");
        assert!(!conns.is_connected("test"));
    });
}
