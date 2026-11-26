use std::sync::Arc;
use std::time::{Duration, SystemTime};

use ext::{Capabilities, ConnectionFieldKind, Registry};
use platform::Fake;
use types::{EntryKind, Location};

use super::client::{Unsigned, amz_date, parse_buckets, parse_page};
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
        .map_or_default(|(_, v)| v.as_str());
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
    // 2026-11-08T03:29:15Z, to the second.
    let modified = SystemTime::UNIX_EPOCH + Duration::from_secs(1_794_108_555);
    assert_eq!(page.objects[1].modified, Some(modified));
    assert_eq!(amz_date(modified), "20261108T032915Z");
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
    assert_eq!(schema.title.key(), "s3-s3");
    assert_eq!(schema.secret_fields().collect::<Vec<_>>(), ["secret_key"]);
    assert!(schema.field("access_key").is_some_and(|f| f.required));
    assert!(schema.field("buckets").is_some_and(|f| !f.required));
    assert_eq!(
        schema.fields.iter().map(|f| f.key).collect::<Vec<_>>(),
        [
            "endpoint",
            "region",
            "access_key",
            "secret_key",
            "buckets",
            "path_style"
        ],
        "the order of specs/s3-connection.md 1.3"
    );
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
    assert!(
        provider
            .capabilities(&Location::new(PROVIDER_ID, "/work"))
            .contains(Capabilities::REMOTE | Capabilities::WRITE)
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

/// Issue 149: what a place offers. Nothing is created at an account — a
/// bucket needs more than the connection holds, and a file needs a bucket
/// — and each says why; a bucket and every folder below it take folders
/// and files, are searched beneath and are downloaded from.
#[test]
fn capabilities_say_what_each_place_offers() {
    ext::i18n::add_bundled("en", include_str!("../locales/en.ftl"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let provider = S3Provider::new(Arc::new(S3Connections::new(fake)));
    let at = |p: &str| Location::new(PROVIDER_ID, p);
    let creating = Capabilities::CREATE_DIR | Capabilities::CREATE_FILE;

    let account = provider.capabilities(&at("/work"));
    assert!(!account.intersects(creating), "{account:?}");
    assert!(account.contains(Capabilities::SEARCH_SUBTREE | Capabilities::DOWNLOAD));
    assert_eq!(
        provider.unavailable_reason(&at("/work"), Capabilities::CREATE_FILE),
        Some("Files live inside a bucket".to_owned())
    );
    assert_eq!(
        provider.unavailable_reason(&at("/work"), Capabilities::CREATE_DIR),
        Some("Buckets are created in the provider's console".to_owned())
    );
    for inside in ["/work/photos", "/work/photos/2026/may"] {
        let caps = provider.capabilities(&at(inside));
        assert!(
            caps.contains(creating | Capabilities::SEARCH_SUBTREE | Capabilities::DOWNLOAD),
            "{inside}: {caps:?}"
        );
    }
}

/// Issue 149, subfolder search: a listing without a delimiter answers for
/// everything beneath a prefix, each folder on the way once and a folder's
/// marker never as an entry of its own.
#[test]
fn a_flat_listing_gives_the_files_and_each_folder_once() {
    let page = parse_page(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <Name>photos</Name><Prefix>2026/</Prefix><IsTruncated>false</IsTruncated>
  <Contents><Key>2026/</Key><Size>0</Size></Contents>
  <Contents><Key>2026/a.jpg</Key><Size>10</Size></Contents>
  <Contents><Key>2026/may/trip/day 1/b.jpg</Key><Size>20</Size></Contents>
  <Contents><Key>2026/may/trip/day 1/c.jpg</Key><Size>30</Size></Contents>
  <Contents><Key>2026/.cache/thumb.db</Key><Size>5</Size></Contents>
  <Contents><Key>2026/empty/</Key><Size>0</Size></Contents>
</ListBucketResult>"#,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let mut seen = std::collections::HashSet::new();
    let entries = S3Provider::flat_entries("work", "photos", "2026/", page, false, &mut seen);
    let listed: Vec<(String, EntryKind)> = entries
        .iter()
        .map(|e| (e.path.to_string(), e.kind.clone()))
        .collect();
    let dir = |p: &str| (format!("/work/photos/2026/{p}"), EntryKind::Dir);
    let file = |p: &str| (format!("/work/photos/2026/{p}"), EntryKind::File);
    assert_eq!(
        listed,
        [
            file("a.jpg"),
            dir("may"),
            dir("may/trip"),
            dir("may/trip/day 1"),
            file("may/trip/day 1/b.jpg"),
            file("may/trip/day 1/c.jpg"),
            dir("empty"),
        ]
    );
}

/// An address on this machine that refuses a connection: a port that was
/// free a moment ago and has no listener.
fn refusing_address() -> std::net::SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap_or_else(|e| panic!("{e}"));
    listener.local_addr().unwrap_or_else(|e| panic!("{e}"))
}

/// A request that never reaches the service is a lost connection, which
/// keeps the pane where it is, and no error of the plugin's own (SPEC.md
/// *Removed and unreachable locations*).
#[test]
fn a_refused_connection_is_a_lost_connection() {
    let url = format!("http://{}/", refusing_address());
    super::client::install_crypto();
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap_or_else(|e| panic!("{e}"));
    let failure = runtime::block_on(async { client.get(&url).send().await })
        .err()
        .unwrap_or_else(|| panic!("the request was answered"));
    let error = super::client::transport(failure);
    assert!(error.is_unreachable(), "{error}");
}

/// A local stand-in for an S3 service: it answers every request with what
/// `answer` returns for the method and the path with its query, and keeps
/// the request lines it saw. No real account is involved.
pub(crate) struct FakeService {
    /// `http://127.0.0.1:<port>`.
    pub endpoint: String,
    seen: Arc<std::sync::Mutex<Vec<String>>>,
}

impl FakeService {
    pub(crate) fn start(
        answer: impl Fn(&str, &str) -> (u16, String) + Send + Sync + 'static,
    ) -> Self {
        use std::io::{BufRead as _, BufReader, Write as _};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap_or_else(|e| panic!("{e}"));
        let port = listener
            .local_addr()
            .unwrap_or_else(|e| panic!("{e}"))
            .port();
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(match stream.try_clone() {
                    Ok(clone) => clone,
                    Err(_) => continue,
                });
                let mut request = String::new();
                if reader.read_line(&mut request).is_err() {
                    continue;
                }
                let mut line = String::new();
                while reader.read_line(&mut line).is_ok_and(|n| n > 0) && line != "\r\n" {
                    line.clear();
                }
                let mut parts = request.split_whitespace();
                let method = parts.next().unwrap_or_default().to_owned();
                let target = parts.next().unwrap_or_default().to_owned();
                log.lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(format!("{method} {target}"));
                let (status, body) = answer(&method, &target);
                let body = if method == "HEAD" {
                    String::new()
                } else {
                    body
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\ncontent-type: application/xml\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        Self {
            endpoint: format!("http://127.0.0.1:{port}"),
            seen,
        }
    }

    /// The request lines so far, `GET /bucket?list-type=2…`.
    pub(crate) fn seen(&self) -> Vec<String> {
        self.seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

/// An S3 error body, as the services send it.
pub(crate) fn error_body(code: &str, message: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Error><Code>{code}</Code><Message>{message}</Message></Error>"
    )
}

/// A listing with the one object `key`.
pub(crate) fn listing_body(key: &str) -> String {
    format!(
        "<ListBucketResult><IsTruncated>false</IsTruncated><Contents><Key>{key}</Key><Size>3</Size><LastModified>2026-11-19T10:00:00.000Z</LastModified></Contents></ListBucketResult>"
    )
}

/// A provider with the one connection `test` to `endpoint`, with the
/// field `buckets` as typed.
pub(crate) fn connected(endpoint: &str, buckets: &str) -> (Arc<S3Connections>, S3Provider) {
    use platform::Secrets as _;
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let conns = Arc::new(S3Connections::new(fake.clone()));
    let provider = S3Provider::new(Arc::clone(&conns));
    let mut conn = ext::Connection {
        name: "test".into(),
        ..ext::Connection::default()
    };
    conn.settings.insert("endpoint".into(), endpoint.to_owned());
    conn.settings.insert("access_key".into(), "key".into());
    if !buckets.is_empty() {
        conn.settings.insert("buckets".into(), buckets.to_owned());
    }
    fake.store_secret(
        &ext::secret_key(PROVIDER_ID, "test", "secret_key"),
        "secret",
    )
    .unwrap_or_else(|e| panic!("{e}"));
    conns.update(vec![conn]);
    (conns, provider)
}

/// The names a listing of `path` gives, or its error.
pub(crate) async fn names_at(
    provider: &S3Provider,
    path: &str,
    depth: ext::Depth,
) -> Result<Vec<String>, types::Error> {
    use futures::StreamExt as _;
    let mut stream = provider.list(
        &Location::new(PROVIDER_ID, path),
        ext::ListOptions {
            depth,
            ..ext::ListOptions::default()
        },
        types::CancelToken::new(),
    );
    let mut names = Vec::new();
    while let Some(batch) = stream.next().await {
        names.extend(batch?.into_iter().map(|e| e.name));
    }
    Ok(names)
}

#[test]
fn bucket_names_are_split_trimmed_deduplicated_and_kept_in_order() {
    assert_eq!(
        bucket_names(" packages , ,media,packages,, logs "),
        ["packages", "media", "logs"]
    );
    assert_eq!(bucket_names(""), Vec::<String>::new());
    assert_eq!(bucket_names(" , ,"), Vec::<String>::new());
}

/// specs/s3-connection.md 1.4, first line: a key that may not list buckets
/// (every `ListBuckets` is refused here, as R2 refuses a bucket-scoped
/// token) with the bucket named in *Buckets*: the root shows the bucket
/// without a request, and opening it lists its objects.
#[test]
fn a_named_bucket_shows_at_the_root_without_a_request_and_opens() {
    let service = FakeService::start(|_, target| {
        if target == "/" {
            (403, error_body("AccessDenied", "Access Denied"))
        } else {
            (200, listing_body("readme.txt"))
        }
    });
    let (conns, provider) = connected(&service.endpoint, "packages, media");
    runtime::block_on(async {
        let root = names_at(&provider, "/test", ext::Depth::One).await;
        assert_eq!(
            root.unwrap_or_else(|e| panic!("{e}")),
            ["packages", "media"]
        );
        // *Connect* sends nothing either.
        conns
            .connect("test")
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(conns.is_connected("test"));
        assert!(service.seen().is_empty(), "{:?}", service.seen());
        let inside = names_at(&provider, "/test/packages", ext::Depth::One).await;
        assert_eq!(inside.unwrap_or_else(|e| panic!("{e}")), ["readme.txt"]);
        // A bucket that is not in the list opens from the address bar too.
        let other = names_at(&provider, "/test/elsewhere", ext::Depth::One).await;
        assert_eq!(other.unwrap_or_else(|e| panic!("{e}")), ["readme.txt"]);
        assert!(
            service.seen().iter().all(|line| line != "GET /"),
            "ListBuckets was called: {:?}",
            service.seen()
        );
    });
}

/// The listing of unlimited depth beneath an account goes through the
/// names too, without `ListBuckets`.
#[test]
fn everything_beneath_an_account_with_named_buckets_goes_through_the_names() {
    let service = FakeService::start(|_, target| {
        if target == "/" {
            (403, error_body("AccessDenied", "Access Denied"))
        } else if target.starts_with("/packages") {
            (200, listing_body("a.txt"))
        } else {
            (200, listing_body("b.txt"))
        }
    });
    let (_conns, provider) = connected(&service.endpoint, "packages,media");
    runtime::block_on(async {
        let all = names_at(&provider, "/test", ext::Depth::Unlimited).await;
        assert_eq!(
            all.unwrap_or_else(|e| panic!("{e}")),
            ["packages", "media", "a.txt", "b.txt"]
        );
        assert!(service.seen().iter().all(|line| line != "GET /"));
    });
}

/// specs/s3-connection.md 1.4: a key that may list, with *Buckets* empty,
/// lists every bucket as before, and *Connect* asks the service.
#[test]
fn an_empty_buckets_field_lists_the_account_as_before() {
    let service = FakeService::start(|_, _| {
        (
            200,
            "<ListAllMyBucketsResult><Buckets><Bucket><Name>one</Name></Bucket><Bucket><Name>two</Name></Bucket></Buckets></ListAllMyBucketsResult>".to_owned(),
        )
    });
    let (conns, provider) = connected(&service.endpoint, "");
    runtime::block_on(async {
        conns
            .connect("test")
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        let root = names_at(&provider, "/test", ext::Depth::One).await;
        assert_eq!(root.unwrap_or_else(|e| panic!("{e}")), ["one", "two"]);
        assert_eq!(service.seen(), ["GET /", "GET /"]);
    });
}

/// The plugin's English messages, as `register` hands them to the host.
pub(crate) fn english() {
    ext::Registry::new().translations(&[("en", include_str!("../locales/en.ftl"))]);
}

/// The message of listing `path` on a service that answers every request
/// with `status` and the error `code`.
fn refusal(status: u16, code: &'static str, buckets: &str, path: &str) -> String {
    english();
    let service = FakeService::start(move |_, _| (status, error_body(code, "as the service said")));
    let (_conns, provider) = connected(&service.endpoint, buckets);
    let error = runtime::block_on(names_at(&provider, path, ext::Depth::One))
        .err()
        .unwrap_or_else(|| panic!("{code} was not an error"));
    assert!(
        !matches!(error, types::Error::PermissionDenied),
        "{code} was folded into a bare permission denied"
    );
    error.to_string()
}

/// specs/s3-connection.md 1.4, second line: a key that may not list
/// buckets, with *Buckets* empty, gets the list-buckets message with the
/// hint and the service's code, not a bare "permission denied". The test
/// fails before the fix: every 403 was `Error::PermissionDenied`.
#[test]
fn a_refused_bucket_listing_names_the_buckets_field() {
    let text = refusal(403, "AccessDenied", "", "/test");
    assert!(text.contains("not allowed to list buckets"), "{text}");
    assert!(text.contains("Buckets field"), "{text}");
    assert!(text.contains("AccessDenied"), "{text}");
}

/// specs/s3-connection.md 1.4, third line, and the table of 1.2: each
/// refusal has its own message with the service's code.
#[test]
fn each_refusal_has_its_own_message_with_the_service_s_code() {
    let denied = refusal(403, "AccessDenied", "packages", "/test/packages");
    assert!(
        denied.contains("Access to this bucket or object was denied"),
        "{denied}"
    );
    assert!(denied.contains("AccessDenied"), "{denied}");
    assert!(!denied.contains("Buckets field"), "{denied}");

    let key = refusal(403, "InvalidAccessKeyId", "", "/test");
    assert!(key.contains("The access key is not recognised"), "{key}");
    assert!(key.contains("InvalidAccessKeyId"), "{key}");

    let secret = refusal(403, "SignatureDoesNotMatch", "", "/test");
    assert!(secret.contains("usually a wrong secret key"), "{secret}");
    assert!(secret.contains("SignatureDoesNotMatch"), "{secret}");

    let missing = refusal(404, "NoSuchBucket", "pakages", "/test/pakages");
    assert!(
        missing.contains("The bucket does not exist; check the name"),
        "{missing}"
    );
    assert!(missing.contains("NoSuchBucket"), "{missing}");

    let other = refusal(503, "SlowDown", "", "/test");
    assert!(
        other.contains("SlowDown (503): as the service said"),
        "{other}"
    );
}

/// A missing key stays `NotFound`, which the kernel reads, and so does a
/// 404 without a body, what a `HEAD` gets.
#[test]
fn a_missing_key_is_still_not_found() {
    let service = FakeService::start(|method, _| {
        if method == "HEAD" {
            (404, String::new())
        } else {
            (
                404,
                error_body("NoSuchKey", "The specified key does not exist."),
            )
        }
    });
    let (_conns, provider) = connected(&service.endpoint, "");
    runtime::block_on(async {
        let bucket = provider
            .stat(&Location::new(PROVIDER_ID, "/test/gone"))
            .await;
        assert!(matches!(bucket, Err(types::Error::NotFound)), "{bucket:?}");
        let object = provider
            .open_read(&Location::new(PROVIDER_ID, "/test/b/k"))
            .await;
        assert!(matches!(object.err(), Some(types::Error::NotFound)));
    });
}

#[test]
fn an_endpoint_is_split_into_the_endpoint_and_the_bucket_of_its_path() {
    let split = |text: &str| {
        let (endpoint, bucket) = split_endpoint(text);
        (endpoint, bucket.unwrap_or_default())
    };
    assert_eq!(
        split("https://abc123.r2.cloudflarestorage.com/packages"),
        (
            "https://abc123.r2.cloudflarestorage.com".into(),
            "packages".into()
        )
    );
    assert_eq!(
        split(" https://abc123.r2.cloudflarestorage.com/packages/sub/key?x=1 "),
        (
            "https://abc123.r2.cloudflarestorage.com".into(),
            "packages".into()
        )
    );
    assert_eq!(
        split("127.0.0.1:9000/media/"),
        ("127.0.0.1:9000".into(), "media".into())
    );
    // A trailing slash names no bucket and is cleaned all the same.
    assert_eq!(
        split("http://127.0.0.1:9000/"),
        ("http://127.0.0.1:9000".into(), String::new())
    );
    assert_eq!(
        split("http://127.0.0.1:9000"),
        ("http://127.0.0.1:9000".into(), String::new())
    );
    assert_eq!(split(""), (String::new(), String::new()));
}

/// The form of the S3 dialogue with these values, the endpoint just
/// edited.
fn reaction_to(values: &[(&str, &str)], edited: &[&str], settled: bool) -> ext::ConnectionReaction {
    let values: std::collections::BTreeMap<String, String> = values
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    let edited: std::collections::BTreeSet<String> =
        edited.iter().map(|k| (*k).to_owned()).collect();
    react(&ext::ConnectionForm {
        values: &values,
        edited: &edited,
        changed: Some("endpoint"),
        settled,
    })
}

/// specs/s3-connection.md 1.4, sixth line, the dialogue's half: pasting an
/// endpoint that ends in `/packages` leaves the endpoint clean and puts
/// `packages` into *Buckets*, after the names already there.
#[test]
fn a_bucket_path_pasted_onto_the_endpoint_moves_to_the_buckets_field() {
    let pasted = "https://abc123.r2.cloudflarestorage.com/packages";
    let reaction = reaction_to(&[("endpoint", pasted)], &["endpoint"], true);
    assert_eq!(
        reaction.rewrite,
        [
            (
                "endpoint",
                "https://abc123.r2.cloudflarestorage.com".to_owned()
            ),
            ("buckets", "packages".to_owned())
        ]
    );
    let reaction = reaction_to(
        &[("endpoint", pasted), ("buckets", "media,packages , logs")],
        &["endpoint", "buckets"],
        true,
    );
    assert_eq!(
        reaction.rewrite[1],
        ("buckets", "media, packages, logs".to_owned())
    );
    let reaction = reaction_to(
        &[("endpoint", pasted), ("buckets", "media")],
        &["endpoint"],
        true,
    );
    assert_eq!(
        reaction.rewrite[1],
        ("buckets", "media, packages".to_owned())
    );
    // While it is being typed the address stays under the cursor.
    assert_eq!(
        reaction_to(&[("endpoint", pasted)], &["endpoint"], false).rewrite,
        []
    );
    // An endpoint without a path is left as typed.
    let plain = "https://abc123.r2.cloudflarestorage.com";
    assert_eq!(
        reaction_to(&[("endpoint", plain)], &["endpoint"], true).rewrite,
        []
    );
}

/// specs/s3-connection.md 1.4, sixth line, the provider's half, and the
/// connection saved before 0.6.1 with a path on its endpoint: the bucket
/// of the path shows at the root, and the requests go to the endpoint
/// without it. The test fails before the fix: the path was dropped and the
/// root called `ListBuckets`.
#[test]
fn a_connection_saved_with_a_bucket_path_on_its_endpoint_opens_showing_that_bucket() {
    let service = FakeService::start(|_, target| {
        if target == "/" {
            (403, error_body("AccessDenied", "Access Denied"))
        } else {
            (200, listing_body("readme.txt"))
        }
    });
    let (conns, provider) = connected(&format!("{}/packages", service.endpoint), "");
    assert_eq!(conns.buckets("test").unwrap_or_default(), ["packages"]);
    runtime::block_on(async {
        let root = names_at(&provider, "/test", ext::Depth::One).await;
        assert_eq!(root.unwrap_or_else(|e| panic!("{e}")), ["packages"]);
        assert!(service.seen().is_empty(), "{:?}", service.seen());
        let inside = names_at(&provider, "/test/packages", ext::Depth::One).await;
        assert_eq!(inside.unwrap_or_else(|e| panic!("{e}")), ["readme.txt"]);
        let seen = service.seen();
        assert!(seen[0].starts_with("GET /packages/?"), "{seen:?}");
    });
}

/// What an endpoint is recognised as: the provider's name, the region and
/// whether path-style is on.
fn recognised(endpoint: &str) -> Option<(String, Option<String>, bool)> {
    english();
    recognise(endpoint).map(|r| (r.provider.get(), r.region, r.path_style))
}

/// The table of specs/s3-connection.md 1.3, each row as the provider's own
/// documentation has it.
#[test]
fn providers_are_recognised_from_the_endpoint_host() {
    let known = |endpoint: &str, name: &str, region: &str, path_style: bool| {
        assert_eq!(
            recognised(endpoint),
            Some((name.to_owned(), Some(region.to_owned()), path_style)),
            "{endpoint}"
        );
    };
    assert_eq!(recognised(""), Some(("Amazon S3".into(), None, false)));
    known(
        "https://s3.eu-central-1.amazonaws.com",
        "Amazon S3",
        "eu-central-1",
        false,
    );
    known(
        "s3.dualstack.us-west-2.amazonaws.com",
        "Amazon S3",
        "us-west-2",
        false,
    );
    known("https://s3.amazonaws.com", "Amazon S3", "us-east-1", false);
    known(
        "https://abc123.r2.cloudflarestorage.com",
        "Cloudflare R2",
        "auto",
        true,
    );
    known(
        "https://ABC123.eu.r2.cloudflarestorage.com/packages",
        "Cloudflare R2",
        "auto",
        true,
    );
    known(
        "https://s3.us-west-004.backblazeb2.com",
        "Backblaze B2",
        "us-west-004",
        true,
    );
    known(
        "https://s3.eu-central-1.wasabisys.com",
        "Wasabi",
        "eu-central-1",
        true,
    );
    known("https://s3.wasabisys.com", "Wasabi", "us-east-1", true);
    known(
        "https://s3.nl-1.wasabisys.com",
        "Wasabi",
        "eu-central-1",
        true,
    );
    known(
        "https://nyc3.digitaloceanspaces.com",
        "DigitalOcean Spaces",
        "nyc3",
        true,
    );
    known("https://s3.fr-par.scw.cloud", "Scaleway", "fr-par", true);
    known(
        "https://s3.gra.io.cloud.ovh.net:443",
        "OVHcloud",
        "gra",
        true,
    );
    known(
        "https://storage.googleapis.com",
        "Google Cloud Storage (S3-compatible)",
        "auto",
        true,
    );
    // Anything else is the generic form.
    for unknown in [
        "http://127.0.0.1:9000",
        "https://minio.example.org",
        "https://r2.cloudflarestorage.com",
        "https://s3.a.b.amazonaws.com",
        "https://evil-r2.cloudflarestorage.com.example.org",
        "https://fsn1.your-objectstorage.com",
    ] {
        assert_eq!(recognised(unknown), None, "{unknown}");
    }
    for row in TABLE {
        assert!(row.source.starts_with("https://"), "{row:?}");
    }
}

/// What the dialogue's reaction fills and notes for these values.
fn filled(
    values: &[(&str, &str)],
    edited: &[&str],
) -> (Vec<(&'static str, String)>, Option<String>) {
    english();
    let reaction = reaction_to(values, edited, true);
    let note = reaction
        .notes
        .iter()
        .find(|(key, _)| *key == "endpoint")
        .map(|(_, note)| note.clone());
    let values: std::collections::BTreeMap<String, String> = values
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    let edited: std::collections::BTreeSet<String> =
        edited.iter().map(|k| (*k).to_owned()).collect();
    let mut after = values.clone();
    reaction.apply(&mut after, &edited);
    let changed = ["endpoint", "region", "buckets", "path_style"]
        .into_iter()
        .filter_map(|key| after.get(key).map(|value| (key, value.clone())))
        .collect();
    (changed, note)
}

/// specs/s3-connection.md 1.4, fifth line: pasting an R2 endpoint sets
/// region `auto` and path style on and shows *Cloudflare R2*; a hand-typed
/// region is left alone.
#[test]
fn an_r2_endpoint_settles_region_and_path_style_and_leaves_a_typed_region_alone() {
    let r2 = "https://abc123.r2.cloudflarestorage.com";
    let (values, note) = filled(
        &[
            ("endpoint", r2),
            ("region", "us-east-1"),
            ("path_style", "false"),
        ],
        &["endpoint"],
    );
    assert_eq!(note.as_deref(), Some("Cloudflare R2"));
    assert_eq!(
        values,
        [
            ("endpoint", r2.to_owned()),
            ("region", "auto".to_owned()),
            ("path_style", "true".to_owned())
        ]
    );
    let (values, note) = filled(
        &[
            ("endpoint", r2),
            ("region", "weur"),
            ("path_style", "false"),
        ],
        &["endpoint", "region", "path_style"],
    );
    assert_eq!(note.as_deref(), Some("Cloudflare R2"));
    assert_eq!(values[1], ("region", "weur".to_owned()));
    assert_eq!(values[2], ("path_style", "false".to_owned()));
}

/// The sixth line of 1.4 in one step: the bucket's address pasted whole is
/// recognised by its host, and the name goes to *Buckets*.
#[test]
fn an_r2_bucket_address_is_recognised_and_taken_apart_at_once() {
    let (values, note) = filled(
        &[
            (
                "endpoint",
                "https://abc123.r2.cloudflarestorage.com/packages",
            ),
            ("region", "us-east-1"),
        ],
        &["endpoint"],
    );
    assert_eq!(note.as_deref(), Some("Cloudflare R2"));
    assert_eq!(
        values,
        [
            (
                "endpoint",
                "https://abc123.r2.cloudflarestorage.com".to_owned()
            ),
            ("region", "auto".to_owned()),
            ("buckets", "packages".to_owned()),
            ("path_style", "true".to_owned())
        ]
    );
}

/// An empty endpoint is AWS, and a host nothing knows is the generic form:
/// both start from the schema's region, and what recognition had filled
/// for another host does not stay behind.
#[test]
fn an_empty_endpoint_is_aws_and_an_unknown_host_is_the_generic_form() {
    let (values, note) = filled(
        &[("endpoint", ""), ("region", "auto"), ("path_style", "true")],
        &[],
    );
    assert_eq!(note.as_deref(), Some("Amazon S3"));
    assert_eq!(
        values[1..],
        [
            ("region", "us-east-1".to_owned()),
            ("path_style", "false".to_owned())
        ]
    );
    let (values, note) = filled(
        &[
            ("endpoint", "http://127.0.0.1:9000"),
            ("region", "auto"),
            ("path_style", "false"),
        ],
        &["endpoint"],
    );
    assert_eq!(note, None);
    assert_eq!(
        values[1..],
        [
            ("region", "us-east-1".to_owned()),
            ("path_style", "true".to_owned())
        ]
    );
}
