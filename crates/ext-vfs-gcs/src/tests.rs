use std::sync::Arc;
use std::time::{Duration, SystemTime};

use ext::{Capabilities, ConnectionFieldKind, Registry};
use platform::Fake;
use types::{EntryKind, Location};

use super::auth::{ServiceAccount, parse_token};
use super::client::{parse_buckets, parse_page, parse_rewrite};
use super::*;

#[test]
fn places_parse_from_locations() {
    let at = |p: &str| Place::of(&Location::new(PROVIDER_ID, p));
    assert_eq!(at("/"), Place::Root);
    assert_eq!(at("/proj"), Place::Project("proj".into()));
    assert_eq!(
        at("/proj/photos"),
        Place::Bucket {
            project: "proj".into(),
            bucket: "photos".into()
        }
    );
    assert_eq!(
        at("/proj/photos/2026/a b.jpg"),
        Place::Object {
            project: "proj".into(),
            bucket: "photos".into(),
            name: "2026/a b.jpg".into()
        }
    );
    let place = at("/proj/photos/2026/a.jpg");
    assert_eq!(place.location().to_string(), "gcs:/proj/photos/2026/a.jpg");
    assert_eq!(place.project(), Some("proj"));
    assert_eq!(client::encode("dir/a b.txt"), "dir%2Fa%20b.txt");
}

#[test]
fn json_answers_parse() {
    let page = parse_page(
        r#"{"kind":"storage#objects","prefixes":["2026/raw/"],"items":[
            {"name":"2026/","size":"0","updated":"2026-11-08T03:29:15.198Z"},
            {"name":"2026/a.jpg","size":"1234","updated":"2026-11-08T03:29:15.198Z"}],
          "nextPageToken":"tok"}"#,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(page.prefixes, ["2026/raw/"]);
    assert_eq!(page.objects[1].size, 1234);
    // 2026-11-08T03:29:15.198Z, to the millisecond.
    let modified = SystemTime::UNIX_EPOCH + Duration::from_millis(1_794_108_555_198);
    assert_eq!(page.objects[1].modified, Some(modified));
    assert_eq!(page.next.as_deref(), Some("tok"));
    let entries = GcsProvider::page_entries("proj", "photos", "2026/", page, true);
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
    let (buckets, next) = parse_buckets(
        r#"{"kind":"storage#buckets","items":[{"name":"photos","timeCreated":"2026-01-02T03:04:05Z"}]}"#,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(buckets.len(), 1);
    assert!(buckets[0].created.is_some());
    assert_eq!(next, None);
    let step = parse_rewrite(
        r#"{"kind":"storage#rewriteResponse","totalBytesRewritten":"1048576","objectSize":"3145728","done":false,"rewriteToken":"abc"}"#,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        (step.done, step.written, step.size, step.token.as_deref()),
        (false, 1_048_576, 3_145_728, Some("abc"))
    );
    assert!(parse_page("nope").is_err());
}

/// A freshly generated RSA key signs a JWT whose three parts are
/// base64url and whose claims name the account and the scope; the token
/// answer parses with its lifetime.
#[test]
fn a_service_account_signs_a_jwt_and_a_token_parses() {
    use base64::Engine as _;
    // A 2048-bit RSA key in PKCS#8 PEM, generated for this test alone.
    let pem = include_str!("../fixtures/test-key.pem");
    let account = ServiceAccount::parse(&format!(
        r#"{{"type":"service_account","client_email":"rick@example.iam.gserviceaccount.com","private_key":{},"token_uri":"https://oauth2.example/token"}}"#,
        serde_json::to_string(pem).unwrap_or_default()
    ))
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(account.client_email, "rick@example.iam.gserviceaccount.com");
    let jwt = account
        .jwt(std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_hours(500_000))
        .unwrap_or_else(|e| panic!("{e}"));
    let parts: Vec<&str> = jwt.split('.').collect();
    assert_eq!(parts.len(), 3);
    let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[1])
        .unwrap_or_else(|e| panic!("{e}"));
    let claims: serde_json::Value =
        serde_json::from_slice(&claims).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(claims["iss"], "rick@example.iam.gserviceaccount.com");
    assert_eq!(claims["aud"], "https://oauth2.example/token");
    assert_eq!(claims["iat"], 1_800_000_000u64);
    assert_eq!(claims["exp"], 1_800_003_600u64);
    assert!(
        claims["scope"]
            .as_str()
            .is_some_and(|s| s.contains("devstorage"))
    );
    assert_eq!(
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(parts[2])
            .map(|s| s.len())
            .ok(),
        Some(256),
        "an RSA-2048 signature"
    );
    assert!(
        ServiceAccount::parse(r#"{"client_email":"x"}"#).is_err(),
        "a key is required"
    );
    let now = std::time::SystemTime::now();
    let token = parse_token(
        r#"{"access_token":"ya29.x","expires_in":3599,"token_type":"Bearer"}"#,
        now,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(token.access_token, "ya29.x");
    assert!(token.is_fresh(now));
    assert!(!token.is_fresh(now + std::time::Duration::from_hours(1)));
}

#[test]
fn the_schema_and_the_extension_register() {
    let schema = schema();
    assert_eq!(schema.title.key(), "gcs-google-cloud-storage");
    assert_eq!(schema.secret_fields().collect::<Vec<_>>(), ["key"]);
    assert!(schema.field("project").is_some_and(|f| f.required));
    assert!(matches!(
        schema.field("key").map(|f| &f.kind),
        Some(ConnectionFieldKind::Secret)
    ));
    let ext = GcsExtension::new();
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
            .capabilities(&Location::new(PROVIDER_ID, "/proj/photos/a"))
            .contains(Capabilities::NATIVE_TRANSFER | Capabilities::WRITE | Capabilities::REMOTE)
    );
}

#[test]
fn the_root_lists_the_projects_without_a_request() {
    use futures::StreamExt as _;
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let conns = Arc::new(GcsConnections::new(fake));
    let provider = GcsProvider::new(Arc::clone(&conns));
    conns.update(vec![ext::Connection {
        name: "proj".into(),
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
        assert_eq!(names, ["proj"]);
        assert!(matches!(
            conns.connect("proj").await,
            Err(types::Error::Extension { .. })
        ));
    });
}

/// The whole round trip against a real store (issue 21): `RICK_GCS_ENDPOINT`
/// names an emulator (fake-gcs-server in CI), `RICK_GCS_PROJECT` the
/// project; the test skips with a note without the endpoint, and fails
/// instead under `RICK_REQUIRE_CONTAINERS`.
#[test]
// One scenario against one store, start to finish.
#[allow(clippy::too_many_lines)]
fn against_a_store_list_upload_download_move_across_buckets_and_delete_round_trip() {
    use ext::{ListOptions, RemoveMode, TransferMode, WriteOptions};
    use futures::StreamExt as _;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    let Ok(endpoint) = std::env::var("RICK_GCS_ENDPOINT") else {
        eprintln!("RICK_GCS_ENDPOINT is not set: no object store to test against; skipped");
        assert!(
            std::env::var_os("RICK_REQUIRE_CONTAINERS").is_none(),
            "RICK_REQUIRE_CONTAINERS is set but RICK_GCS_ENDPOINT is not"
        );
        return;
    };
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let conns = Arc::new(GcsConnections::new(fake));
    let provider = GcsProvider::new(Arc::clone(&conns));
    let mut conn = ext::Connection {
        name: "test".into(),
        ..ext::Connection::default()
    };
    conn.settings.insert("endpoint".into(), endpoint);
    conn.settings.insert(
        "project".into(),
        std::env::var("RICK_GCS_PROJECT").unwrap_or_else(|_| "rick-test".into()),
    );
    conns.update(vec![conn]);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let bucket_a = format!("rick-test-a-{stamp}");
    let bucket_b = format!("rick-test-b-{stamp}");
    let at = |rest: &str| Location::new(PROVIDER_ID, format!("/test/{rest}"));
    let list = |provider: &GcsProvider, rest: &str| {
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

        provider
            .create_dir(&at(&format!("{bucket_a}/photos")))
            .await
            .unwrap_or_else(|e| panic!("mkdir: {e}"));
        let payload: Vec<u8> = (0..20 * 1024 * 1024u32).map(|i| (i % 239) as u8).collect();
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
        let stat = provider
            .stat(&at(&format!("{bucket_a}/photos/big.bin")))
            .await
            .unwrap_or_else(|e| panic!("stat: {e}"));
        assert_eq!(stat.kind, EntryKind::File);
        assert_eq!(stat.size, Some(payload.len() as u64));
        let mut back = Vec::new();
        provider
            .open_read(&at(&format!("{bucket_a}/photos/big.bin")))
            .await
            .unwrap_or_else(|e| panic!("open_read: {e}"))
            .read_to_end(&mut back)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
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
                .unwrap_or_else(|e| panic!("{e}"));
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
        assert_eq!(list(&provider, &bucket_a).await, ["archive"]);
        let mut small = String::new();
        provider
            .open_read(&at(&format!("{bucket_b}/archive/notes.txt")))
            .await
            .unwrap_or_else(|e| panic!("{e}"))
            .read_to_string(&mut small)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(small, "hello");

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
        assert!(!buckets.contains(&bucket_a) && !buckets.contains(&bucket_b));
    });
}

/// Issue 149: what a place offers. Nothing is created at a project — a
/// bucket needs more than the connection holds, and a file needs a bucket
/// — and each says why; a bucket and every folder below it take folders
/// and files, are searched beneath and are downloaded from.
#[test]
fn capabilities_say_what_each_place_offers() {
    ext::i18n::add_bundled("en", include_str!("../locales/en.ftl"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let provider = GcsProvider::new(Arc::new(GcsConnections::new(fake)));
    let at = |p: &str| Location::new(PROVIDER_ID, p);
    let creating = Capabilities::CREATE_DIR | Capabilities::CREATE_FILE;

    let project = provider.capabilities(&at("/proj"));
    assert!(!project.intersects(creating), "{project:?}");
    assert!(project.contains(Capabilities::SEARCH_SUBTREE | Capabilities::DOWNLOAD));
    assert_eq!(
        provider.unavailable_reason(&at("/proj"), Capabilities::CREATE_FILE),
        Some("Files live inside a bucket".to_owned())
    );
    assert_eq!(
        provider.unavailable_reason(&at("/proj"), Capabilities::CREATE_DIR),
        Some("Buckets are created in the provider's console".to_owned())
    );
    for inside in ["/proj/photos", "/proj/photos/2026/may"] {
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
        r#"{"kind":"storage#objects","items":[
            {"name":"2026/","size":"0"},
            {"name":"2026/a.jpg","size":"10"},
            {"name":"2026/may/trip/day 1/b.jpg","size":"20"},
            {"name":"2026/may/trip/day 1/c.jpg","size":"30"},
            {"name":"2026/.cache/thumb.db","size":"5"},
            {"name":"2026/empty/","size":"0"}]}"#,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let mut seen = std::collections::HashSet::new();
    let entries = GcsProvider::flat_entries("proj", "photos", "2026/", page, false, &mut seen);
    let listed: Vec<(String, EntryKind)> = entries
        .iter()
        .map(|e| (e.path.to_string(), e.kind.clone()))
        .collect();
    let dir = |p: &str| (format!("/proj/photos/2026/{p}"), EntryKind::Dir);
    let file = |p: &str| (format!("/proj/photos/2026/{p}"), EntryKind::File);
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

/// A local stand-in for the service and its token endpoint: `answer`
/// takes the method, the target and whether the request carried a bearer
/// token, and gives the status and the body. Returns the endpoint and the
/// requests seen, as `METHOD target`. No real account.
fn fake_service(
    answer: fn(&str, &str, bool) -> (u16, &'static str),
) -> (String, Arc<std::sync::Mutex<Vec<String>>>) {
    use std::io::{BufRead as _, BufReader, Read as _, Write as _};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap_or_else(|e| panic!("{e}"));
    let endpoint = format!(
        "http://{}",
        listener.local_addr().unwrap_or_else(|e| panic!("{e}"))
    );
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let log = Arc::clone(&log);
            std::thread::spawn(move || {
                let Ok(clone) = stream.try_clone() else {
                    return;
                };
                let mut reader = BufReader::new(clone);
                let mut request = String::new();
                if reader.read_line(&mut request).is_err() {
                    return;
                }
                let (mut length, mut bearer) = (0usize, false);
                let mut line = String::new();
                while reader.read_line(&mut line).is_ok_and(|n| n > 0) && line != "\r\n" {
                    let lower = line.to_ascii_lowercase();
                    if let Some(value) = lower.strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap_or(0);
                    }
                    bearer |= lower.starts_with("authorization: bearer ");
                    line.clear();
                }
                let mut body = vec![0u8; length];
                let _ = reader.read_exact(&mut body);
                let mut parts = request.split_whitespace();
                let method = parts.next().unwrap_or_default().to_owned();
                let target = parts.next().unwrap_or_default().to_owned();
                log.lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(format!("{method} {target}"));
                let (status, body) = answer(&method, &target, bearer);
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
            });
        }
    });
    (endpoint, seen)
}

/// A service account key whose token endpoint is `endpoint`'s `/token`,
/// signed with the key generated for the tests.
fn test_key(endpoint: &str) -> String {
    format!(
        r#"{{"type":"service_account","client_email":"rick@example.iam.gserviceaccount.com","private_key":{},"token_uri":"{endpoint}/token"}}"#,
        serde_json::to_string(include_str!("../fixtures/test-key.pem")).unwrap_or_default()
    )
}

const TOKEN: &str = r#"{"access_token":"ya29.test","expires_in":3600,"token_type":"Bearer"}"#;
const BUCKETS: &str = r#"{"kind":"storage#buckets","items":[]}"#;
const UNAUTHENTICATED: &str = r#"{"error":{"code":401,"message":"Anonymous caller does not have storage.buckets.list access."}}"#;

/// The outcomes and details of a connection test on `endpoint` with the
/// key as typed; an empty key is a connection without one.
fn tested(
    endpoint: &str,
    key: &str,
    limit: Duration,
    cancel: &types::CancelToken,
) -> (bool, Vec<(ext::CheckOutcome, String)>) {
    ext::i18n::add_bundled("en", include_str!("../locales/en.ftl"));
    let values = std::collections::BTreeMap::from([
        ("endpoint".to_owned(), endpoint.to_owned()),
        ("project".to_owned(), "rick-test".to_owned()),
    ]);
    let form = ext::ConnectionTestForm {
        name: None,
        values: &values,
    };
    let key = (!key.is_empty()).then(|| key.to_owned());
    let result = runtime::block_on(super::check::run(&form, Ok(key), limit, cancel));
    let labels: Vec<&str> = result.checks.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "The host is reachable",
            "The credentials are accepted",
            "The project's buckets can be listed"
        ]
    );
    (
        result.good,
        result
            .checks
            .into_iter()
            .map(|c| (c.outcome, c.detail))
            .collect(),
    )
}

fn test_of(endpoint: &str, key: &str) -> (bool, Vec<(ext::CheckOutcome, String)>) {
    tested(
        endpoint,
        key,
        Duration::from_secs(10),
        &types::CancelToken::new(),
    )
}

fn outcomes(lines: &[(ext::CheckOutcome, String)]) -> Vec<ext::CheckOutcome> {
    lines.iter().map(|(outcome, _)| *outcome).collect()
}

/// specs/s3-connection.md 3.6: a good test for Google Cloud Storage, with
/// a service account and, as an emulator takes it, without a key. It only
/// reads: the listings are `GET`s, and the one `POST` is the sign-in.
#[test]
fn a_connection_test_that_lists_the_buckets_is_good_and_only_reads() {
    use ext::CheckOutcome::Passed;
    let (endpoint, seen) = fake_service(|method, target, bearer| match (method, bearer) {
        ("POST", _) if target == "/token" => (200, TOKEN),
        ("GET", true) => (200, BUCKETS),
        _ => (401, UNAUTHENTICATED),
    });
    let (good, lines) = test_of(&endpoint, &test_key(&endpoint));
    assert!(good, "{lines:?}");
    assert_eq!(outcomes(&lines), [Passed, Passed, Passed]);
    let seen = seen
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert_eq!(seen.len(), 3, "{seen:?}");
    assert!(seen[0].starts_with("GET /storage/v1/b?"), "{seen:?}");
    assert!(seen[0].contains("maxResults=1"), "{seen:?}");
    assert_eq!(seen[1], "POST /token");
    assert_eq!(seen[2], seen[0]);

    let (open, seen) = fake_service(|_, _, _| (200, BUCKETS));
    let (good, lines) = test_of(&open, "");
    assert!(good, "{lines:?}");
    let seen = seen
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert_eq!(seen.len(), 1, "{seen:?}");
}

/// 3.6: a test that is not good, each outcome of each check.
#[test]
fn a_connection_test_that_is_not_good_says_which_check_and_why() {
    use ext::CheckOutcome::{Failed, NotRun, Passed};
    // The token endpoint refuses the key.
    let (endpoint, _) = fake_service(|method, _, _| match method {
        "POST" => (
            400,
            r#"{"error":"invalid_grant","error_description":"Invalid JWT Signature."}"#,
        ),
        _ => (401, UNAUTHENTICATED),
    });
    let (good, lines) = test_of(&endpoint, &test_key(&endpoint));
    assert!(!good);
    assert_eq!(outcomes(&lines), [Passed, Failed, NotRun]);
    assert!(lines[1].1.contains("invalid_grant"), "{lines:?}");

    // The service does not take the token.
    let (endpoint, _) = fake_service(|method, _, _| match method {
        "POST" => (200, TOKEN),
        _ => (
            401,
            r#"{"error":{"code":401,"message":"Invalid Credentials"}}"#,
        ),
    });
    let (good, lines) = test_of(&endpoint, &test_key(&endpoint));
    assert!(!good);
    assert_eq!(outcomes(&lines), [Passed, Failed, NotRun]);
    assert!(lines[1].1.contains("Invalid Credentials"), "{lines:?}");
    assert!(lines[1].1.contains("401"), "{lines:?}");

    // The account is known and may not list the project's buckets.
    let (endpoint, _) = fake_service(|method, _, bearer| match (method, bearer) {
        ("POST", _) => (200, TOKEN),
        (_, true) => (
            403,
            r#"{"error":{"code":403,"message":"rick@example does not have storage.buckets.list access to the Google Cloud project."}}"#,
        ),
        _ => (401, UNAUTHENTICATED),
    });
    let (good, lines) = test_of(&endpoint, &test_key(&endpoint));
    assert!(!good);
    assert_eq!(outcomes(&lines), [Passed, Passed, Failed]);
    assert!(lines[2].1.contains("storage.buckets.list"), "{lines:?}");
    assert!(lines[2].1.contains("403"), "{lines:?}");

    // No key where the service wants one, a key that is no key, and a key
    // file that is not there.
    let (good, lines) = test_of(&endpoint, "");
    assert!(!good);
    assert_eq!(outcomes(&lines), [Passed, Failed, NotRun]);
    assert!(lines[1].1.contains("Anonymous caller"), "{lines:?}");
    let (_, lines) = test_of(&endpoint, "{\"type\": \"nothing\"}");
    assert_eq!(outcomes(&lines), [Passed, Failed, NotRun]);
    let (_, lines) = test_of(&endpoint, "/nowhere/rick-key.json");
    assert_eq!(outcomes(&lines), [Passed, Failed, NotRun]);
    assert!(lines[1].1.contains("/nowhere/rick-key.json"), "{lines:?}");

    // A host that cannot be reached.
    let (good, lines) = test_of(&format!("http://{}", refusing_address()), "");
    assert!(!good);
    assert_eq!(outcomes(&lines), [Failed, NotRun, NotRun]);
    assert_ne!(lines[0].1, "");
}

/// specs/s3-connection.md 3.5: a check gives up after its limit, and a
/// cancelled test ends at once with nothing run.
#[test]
fn a_connection_test_gives_up_after_its_limit_and_can_be_cancelled() {
    use ext::CheckOutcome::{Failed, NotRun};
    let (endpoint, _) = fake_service(|_, _, _| {
        std::thread::sleep(Duration::from_secs(3));
        (200, BUCKETS)
    });
    let started = std::time::Instant::now();
    let (good, lines) = tested(
        &endpoint,
        "",
        Duration::from_millis(200),
        &types::CancelToken::new(),
    );
    assert!(!good);
    assert_eq!(outcomes(&lines), [Failed, NotRun, NotRun]);
    assert!(lines[0].1.contains("No answer within"), "{lines:?}");
    assert!(started.elapsed() < Duration::from_secs(2));

    let cancel = types::CancelToken::new();
    let later = cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        later.cancel();
    });
    let started = std::time::Instant::now();
    let (good, lines) = tested(&endpoint, "", Duration::from_secs(10), &cancel);
    assert!(!good);
    assert_eq!(outcomes(&lines), [NotRun, NotRun, NotRun]);
    assert!(started.elapsed() < Duration::from_secs(2));
}

/// 3.1 through the connector: in *Edit…* a key left empty is the
/// keychain's, and the test stores nothing.
#[test]
fn the_connector_tests_with_the_typed_key_or_the_keychain_s() {
    use ext::Connector as _;
    use platform::Secrets as _;
    ext::i18n::add_bundled("en", include_str!("../locales/en.ftl"));
    let (endpoint, _) = fake_service(|method, _, bearer| match (method, bearer) {
        ("POST", _) => (200, TOKEN),
        ("GET", true) => (200, BUCKETS),
        _ => (401, UNAUTHENTICATED),
    });
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let conns = GcsConnections::new(fake.clone());
    fake.store_secret(
        &ext::secret_key(PROVIDER_ID, "saved", "key"),
        &test_key(&endpoint),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(conns.checks().len(), 3);
    let run = |name: Option<&str>, key: &str| {
        let mut values = std::collections::BTreeMap::from([
            ("endpoint".to_owned(), endpoint.clone()),
            ("project".to_owned(), "rick-test".to_owned()),
        ]);
        if !key.is_empty() {
            values.insert("key".to_owned(), key.to_owned());
        }
        let form = ext::ConnectionTestForm {
            name,
            values: &values,
        };
        assert!(!format!("{form:?}").contains("PRIVATE KEY"));
        runtime::block_on(conns.test(&form, &types::CancelToken::new()))
    };
    assert!(run(Some("saved"), "").good);
    assert!(run(None, &test_key(&endpoint)).good);
    // Add without a key: the service wants one here.
    assert!(!run(None, "").good);
    assert_eq!(fake.secrets().len(), 1, "the test stored nothing");
}

/// Against the emulator the round trip uses (fake-gcs-server in CI): the
/// test is good, and a host that is not there is not.
#[test]
fn against_a_store_the_connection_test_is_good() {
    let Ok(endpoint) = std::env::var("RICK_GCS_ENDPOINT") else {
        eprintln!("RICK_GCS_ENDPOINT is not set: no object store to test against; skipped");
        assert!(
            std::env::var_os("RICK_REQUIRE_CONTAINERS").is_none(),
            "RICK_REQUIRE_CONTAINERS is set but RICK_GCS_ENDPOINT is not"
        );
        return;
    };
    let (good, lines) = test_of(&endpoint, "");
    assert!(good, "{lines:?}");
    let (good, lines) = test_of(&format!("http://{}", refusing_address()), "");
    assert!(!good, "{lines:?}");
}
