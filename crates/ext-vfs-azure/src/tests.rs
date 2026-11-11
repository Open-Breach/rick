use std::sync::Arc;

use ext::{Capabilities, ConnectionFieldKind, Registry};
use platform::Fake;
use types::{EntryKind, Location};

use super::client::{Unsigned, parse_containers, parse_page};
use super::*;

/// The plugin's English strings, which a registry loads when the extension
/// registers.
fn english() {
    ext::i18n::add_bundled("en", include_str!("../locales/en.ftl"));
}

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
    assert_eq!(schema.title.key(), "azure-azure-blob");
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

/// Issue 149: what a place offers. An account takes a folder, which is a
/// container, and no file, and says why; a container and every folder
/// below it take both, are searched beneath and are downloaded from.
#[test]
fn capabilities_say_what_each_place_offers() {
    english();
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let provider = AzureProvider::new(Arc::new(AzureConnections::new(fake)));
    let at = |p: &str| Location::new(PROVIDER_ID, p);
    let creating = Capabilities::CREATE_DIR | Capabilities::CREATE_FILE;

    let account = provider.capabilities(&at("/work"));
    assert!(account.contains(Capabilities::CREATE_DIR));
    assert!(!account.contains(Capabilities::CREATE_FILE));
    assert_eq!(
        provider.unavailable_reason(&at("/work"), Capabilities::CREATE_FILE),
        Some("Files live inside a container".to_owned())
    );
    for inside in ["/work/photos", "/work/photos/2026", "/work/photos/2026/may"] {
        let caps = provider.capabilities(&at(inside));
        assert!(
            caps.contains(creating | Capabilities::SEARCH_SUBTREE | Capabilities::DOWNLOAD),
            "{inside}: {caps:?}"
        );
        assert_eq!(provider.unavailable_reason(&at(inside), creating), None);
    }
    assert!(!provider.capabilities(&at("/")).intersects(creating));
    assert!(
        provider
            .unavailable_reason(&at("/"), Capabilities::CREATE_DIR)
            .is_some()
    );
}

/// Issue 149: a container's name is checked against the service's rules
/// before any request goes out, and the rule it breaks is named. The
/// default *New folder* is such a name.
#[test]
fn a_container_name_is_checked_against_the_naming_rules() {
    use crate::provider::container_name_problem as problem;
    english();
    for good in ["abc", "photos", "photos-2026", "a1b", &"a".repeat(63)] {
        assert_eq!(problem(good), None, "{good}");
    }
    let length = Some("A container name is 3 to 63 characters long".to_owned());
    assert_eq!(problem("ab"), length);
    assert_eq!(problem(&"a".repeat(64)), length);
    let characters =
        Some("A container name has lowercase letters, digits and hyphens only".to_owned());
    assert_eq!(problem("New folder"), characters);
    assert_eq!(problem("Photos"), characters);
    assert_eq!(problem("my_photos"), characters);
    let edges = Some("A container name starts and ends with a letter or a digit".to_owned());
    assert_eq!(problem("-photos"), edges);
    assert_eq!(problem("photos-"), edges);
    assert_eq!(
        problem("my--photos"),
        Some("A container name has no two hyphens in a row".to_owned())
    );

    // The provider answers for the place a folder would be made at: a
    // container at an account, nothing below one.
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let provider = AzureProvider::new(Arc::new(AzureConnections::new(fake)));
    let at = |p: &str| Location::new(PROVIDER_ID, p);
    assert_eq!(provider.name_problem(&at("/work/New folder")), characters);
    assert_eq!(provider.name_problem(&at("/work/photos")), None);
    assert_eq!(provider.name_problem(&at("/work/photos/New folder")), None);
    // And `create_dir` refuses the name itself, without a connection: no
    // request was made.
    let error = runtime::block_on(provider.create_dir(&at("/work/New folder")))
        .err()
        .map(|e| e.to_string());
    assert_eq!(
        error,
        Some(
            "extension vfs-azure: A container name has lowercase letters, digits and hyphens only"
                .to_owned()
        )
    );
}

/// Issue 149, subfolder search: a listing without a delimiter answers for
/// everything beneath a prefix. Every blob is a file, every folder on the
/// way to one is reported once, a placeholder is its folder and never an
/// entry of its own, and what lies under a hidden folder is left out
/// with it.
#[test]
fn a_flat_listing_gives_the_files_and_each_folder_once() {
    let page = parse_page(
        r#"<?xml version="1.0" encoding="utf-8"?>
<EnumerationResults ContainerName="photos">
  <Prefix>2026/</Prefix>
  <Blobs>
    <Blob><Name>2026/</Name><Properties><Content-Length>0</Content-Length></Properties></Blob>
    <Blob><Name>2026/a.jpg</Name><Properties><Content-Length>10</Content-Length></Properties></Blob>
    <Blob><Name>2026/may/</Name><Properties><Content-Length>0</Content-Length></Properties></Blob>
    <Blob><Name>2026/may/trip/day 1/b.jpg</Name><Properties><Content-Length>20</Content-Length></Properties></Blob>
    <Blob><Name>2026/may/trip/day 1/c.jpg</Name><Properties><Content-Length>30</Content-Length></Properties></Blob>
    <Blob><Name>2026/.cache/thumb.db</Name><Properties><Content-Length>5</Content-Length></Properties></Blob>
    <Blob><Name>2026/empty/</Name><Properties><Content-Length>0</Content-Length></Properties></Blob>
  </Blobs>
  <NextMarker/>
</EnumerationResults>"#,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let mut seen = std::collections::HashSet::new();
    let entries =
        AzureProvider::flat_entries("work", "photos", "2026/", page.clone(), false, &mut seen);
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
    assert_eq!(entries[3].name, "day 1");
    assert_eq!(entries[4].size, Some(20));
    // With hidden entries shown, the hidden folder and its file come too.
    let mut seen = std::collections::HashSet::new();
    let all = AzureProvider::flat_entries("work", "photos", "2026/", page, true, &mut seen);
    assert_eq!(all.len(), 9);
}

/// What a request looked like on the wire.
#[derive(Debug, Clone)]
struct Seen {
    method: String,
    /// The request target as sent: the path and the query.
    target: String,
    headers: Vec<(String, String)>,
}

impl Seen {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// A server that records every request and answers each with `answer`,
/// which takes the method and the target and gives the status line and
/// the body. It stands in for the service where the test is about what
/// the client sends.
fn recording_server(
    answer: fn(&str, &str) -> (&'static str, &'static str),
) -> (String, Arc<std::sync::Mutex<Vec<Seen>>>) {
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
            let Ok(mut stream) = stream else { return };
            let log = Arc::clone(&log);
            std::thread::spawn(move || {
                let Ok(clone) = stream.try_clone() else {
                    return;
                };
                let mut reader = BufReader::new(clone);
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    let mut parts = line.split_whitespace();
                    let method = parts.next().unwrap_or_default().to_owned();
                    let target = parts.next().unwrap_or_default().to_owned();
                    let mut headers = Vec::new();
                    loop {
                        let mut header = String::new();
                        if reader.read_line(&mut header).unwrap_or(0) == 0 {
                            return;
                        }
                        let header = header.trim_end();
                        if header.is_empty() {
                            break;
                        }
                        if let Some((k, v)) = header.split_once(':') {
                            headers.push((k.trim().to_owned(), v.trim().to_owned()));
                        }
                    }
                    let length = headers
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                        .and_then(|(_, v)| v.parse::<usize>().ok())
                        .unwrap_or(0);
                    let mut body = vec![0u8; length];
                    if reader.read_exact(&mut body).is_err() {
                        return;
                    }
                    let (status, body) = answer(&method, &target);
                    log.lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push(Seen {
                            method: method.clone(),
                            target,
                            headers,
                        });
                    let body = if method == "HEAD" { "" } else { body };
                    let reply = format!(
                        "HTTP/1.1 {status}\r\ncontent-length: {}\r\ncontent-type: application/xml\r\n\r\n{body}",
                        body.len()
                    );
                    if stream.write_all(reply.as_bytes()).is_err() {
                        return;
                    }
                }
            });
        }
    });
    (endpoint, seen)
}

/// A provider with one connection, `test`, to the server at `endpoint`.
fn provider_at(endpoint: &str) -> AzureProvider {
    use platform::Secrets as _;
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let conns = Arc::new(AzureConnections::new(fake.clone()));
    let mut conn = ext::Connection {
        name: "test".into(),
        ..ext::Connection::default()
    };
    conn.settings.insert("endpoint".into(), endpoint.to_owned());
    conn.settings
        .insert("account".into(), "devstoreaccount1".into());
    fake.store_secret(
        &ext::secret_key(PROVIDER_ID, "test", "key"),
        "Eby8vdM02xNOcqFlqUwJPLlmEtlCDXJ1OUzFT50uSRZ6IFsuFq2UVErCz4I6tq/K1SZFPTOtr/KBHBeksoGMGw==",
    )
    .unwrap_or_else(|e| panic!("{e}"));
    conns.update(vec![conn]);
    AzureProvider::new(conns)
}

const EMPTY_LIST: &str = r#"<?xml version="1.0" encoding="utf-8"?><EnumerationResults><Blobs/><NextMarker/></EnumerationResults>"#;

/// Issue 149: the service wants a `Content-Length` on every `PUT`, and
/// answers one without with `MissingContentLengthHeader`. Creating a
/// container is a `PUT` without a body, and sent none.
#[test]
fn a_put_without_a_body_carries_a_content_length() {
    fn answer(method: &str, _target: &str) -> (&'static str, &'static str) {
        match method {
            // The container is not there yet.
            "HEAD" => ("404 Not Found", ""),
            _ => ("201 Created", ""),
        }
    }
    let (endpoint, seen) = recording_server(answer);
    let provider = provider_at(&endpoint);
    runtime::block_on(provider.create_dir(&Location::new(PROVIDER_ID, "/test/photos")))
        .unwrap_or_else(|e| panic!("create container: {e}"));
    let seen = seen
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let put = seen
        .iter()
        .find(|r| r.method == "PUT")
        .unwrap_or_else(|| panic!("no PUT among {seen:?}"));
    assert_eq!(put.target, "/devstoreaccount1/photos?restype=container");
    assert_eq!(put.header("content-length"), Some("0"), "{put:?}");
}

/// A store that does not answer is a lost connection, which keeps the pane
/// where it is, and no error of the plugin's own (SPEC.md *Removed and
/// unreachable locations*).
#[test]
fn a_store_that_refuses_the_connection_is_a_lost_connection() {
    use futures::StreamExt as _;
    // A port that was free a moment ago and has no listener.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap_or_else(|e| panic!("{e}"));
    let endpoint = format!(
        "http://{}",
        listener.local_addr().unwrap_or_else(|e| panic!("{e}"))
    );
    drop(listener);
    let provider = provider_at(&endpoint);
    let first = runtime::block_on(async {
        provider
            .list(
                &Location::new(PROVIDER_ID, "/test/photos"),
                ext::ListOptions::default(),
                types::CancelToken::new(),
            )
            .next()
            .await
    });
    match first {
        Some(Err(error)) => assert!(error.is_unreachable(), "{error}"),
        other => panic!("the listing went through: {other:?}"),
    }
}

/// Issue 149: a new folder and a new file inside a container, two levels
/// deep, under the names the inline edit suggests, which have a space. A
/// folder is a zero-byte blob with a trailing slash and a file an empty
/// blob; a space travels as `%20` in the path and in a query value alike,
/// never as a `+`, which is not what was signed.
#[test]
fn a_folder_and_a_file_with_a_space_are_put_two_levels_deep() {
    use tokio::io::AsyncWriteExt as _;
    fn answer(method: &str, target: &str) -> (&'static str, &'static str) {
        match method {
            "PUT" => ("201 Created", ""),
            "GET" if target.contains("comp=list") => ("200 OK", EMPTY_LIST),
            // Nothing is there yet.
            _ => ("404 Not Found", ""),
        }
    }
    let (endpoint, seen) = recording_server(answer);
    let provider = provider_at(&endpoint);
    let at = |rest: &str| Location::new(PROVIDER_ID, format!("/test/photos/{rest}"));
    runtime::block_on(async {
        provider
            .create_dir(&at("2026/may/New folder"))
            .await
            .unwrap_or_else(|e| panic!("new folder: {e}"));
        let mut writer = provider
            .open_write(
                &at("2026/may/New file.txt"),
                ext::WriteOptions { overwrite: false },
            )
            .await
            .unwrap_or_else(|e| panic!("new file: {e}"));
        writer
            .shutdown()
            .await
            .unwrap_or_else(|e| panic!("new file: {e}"));
    });
    let seen = seen
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let puts: Vec<&Seen> = seen.iter().filter(|r| r.method == "PUT").collect();
    let targets: Vec<&str> = puts.iter().map(|r| r.target.as_str()).collect();
    assert_eq!(
        targets,
        [
            "/devstoreaccount1/photos/2026/may/New%20folder/",
            "/devstoreaccount1/photos/2026/may/New%20file.txt",
        ]
    );
    for put in &puts {
        assert_eq!(put.header("content-length"), Some("0"), "{put:?}");
        assert_eq!(put.header("x-ms-blob-type"), Some("BlockBlob"), "{put:?}");
    }
    // The look for an existing folder lists by prefix: the space and the
    // slash are percent-encoded in the query.
    let list = seen
        .iter()
        .find(|r| r.method == "GET" && r.target.contains("comp=list"))
        .unwrap_or_else(|| panic!("no listing among {seen:?}"));
    assert!(
        list.target.contains("prefix=2026%2Fmay%2FNew%20folder%2F"),
        "{}",
        list.target
    );
    assert!(!list.target.contains('+'), "{}", list.target);
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

/// A provider connected to the store `RICK_AZURE_ENDPOINT` names, as
/// `test`; `None`, with a note, without the endpoint, which is a failure
/// under `RICK_REQUIRE_CONTAINERS`.
fn store_provider() -> Option<(AzureProvider, Arc<AzureConnections>)> {
    use platform::Secrets as _;
    let Ok(endpoint) = std::env::var("RICK_AZURE_ENDPOINT") else {
        eprintln!("RICK_AZURE_ENDPOINT is not set: no blob store to test against; skipped");
        assert!(
            std::env::var_os("RICK_REQUIRE_CONTAINERS").is_none(),
            "RICK_REQUIRE_CONTAINERS is set but RICK_AZURE_ENDPOINT is not"
        );
        return None;
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
    Some((provider, conns))
}

/// Acceptance (issue 149) against a real store: a container is created at
/// the account's root under a name the rules take, and a name they do not
/// take is refused before the store is asked; inside the container a new
/// folder and a new file, under the names the inline edit suggests,
/// appear two levels deep and are there again after a refresh; and a
/// listing of everything beneath finds a blob three levels below the
/// container, and below the account.
#[test]
// One scenario against one store, start to finish.
#[allow(clippy::too_many_lines)]
fn against_a_store_new_folders_and_files_appear_at_depth_and_are_found_beneath() {
    use ext::{Depth, ListOptions, RemoveMode, WriteOptions};
    use futures::StreamExt as _;
    use tokio::io::AsyncWriteExt as _;

    let Some((provider, conns)) = store_provider() else {
        return;
    };
    english();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let container = format!("rick-test-depth-{stamp}");
    let at = |rest: &str| Location::new(PROVIDER_ID, format!("/test/{rest}"));
    let list = |provider: &AzureProvider, rest: &str, depth: Depth| {
        let stream = provider.list(
            &at(rest),
            ListOptions {
                hidden: true,
                depth,
                ..ListOptions::default()
            },
            types::CancelToken::new(),
        );
        async move {
            let batches: Vec<_> = stream.collect().await;
            let mut found: Vec<(String, EntryKind)> = batches
                .into_iter()
                .flat_map(|b| b.unwrap_or_else(|e| panic!("{e}")))
                .map(|e| (e.path.to_string(), e.kind))
                .collect();
            found.sort_by(|a, b| a.0.cmp(&b.0));
            found
        }
    };
    runtime::block_on(async {
        conns
            .connect("test")
            .await
            .unwrap_or_else(|e| panic!("connect: {e}"));
        // At the root: a name the rules refuse never reaches the store.
        assert!(
            provider
                .create_dir(&at("New folder"))
                .await
                .is_err_and(|e| e.to_string().contains("container name"))
        );
        provider
            .create_dir(&at(&container))
            .await
            .unwrap_or_else(|e| panic!("create container: {e}"));

        // Two levels deep, with the suggested names.
        for folder in ["one", "one/two", "one/two/New folder"] {
            provider
                .create_dir(&at(&format!("{container}/{folder}")))
                .await
                .unwrap_or_else(|e| panic!("new folder {folder}: {e}"));
        }
        let mut writer = provider
            .open_write(
                &at(&format!("{container}/one/two/New file.txt")),
                WriteOptions { overwrite: false },
            )
            .await
            .unwrap_or_else(|e| panic!("new file: {e}"));
        writer
            .shutdown()
            .await
            .unwrap_or_else(|e| panic!("new file: {e}"));

        // Listed again, which is what a refresh does, they are there, and
        // a folder that holds nothing but its placeholder lists as empty.
        let dir = |p: &str| (format!("/test/{container}/{p}"), EntryKind::Dir);
        let file = |p: &str| (format!("/test/{container}/{p}"), EntryKind::File);
        for _ in 0..2 {
            assert_eq!(list(&provider, &container, Depth::One).await, [dir("one")]);
            assert_eq!(
                list(&provider, &format!("{container}/one/two"), Depth::One).await,
                [file("one/two/New file.txt"), dir("one/two/New folder")]
            );
            assert_eq!(
                list(
                    &provider,
                    &format!("{container}/one/two/New folder"),
                    Depth::One
                )
                .await,
                Vec::new()
            );
        }
        let stat = provider
            .stat(&at(&format!("{container}/one/two/New file.txt")))
            .await
            .unwrap_or_else(|e| panic!("stat: {e}"));
        assert_eq!((stat.kind, stat.size), (EntryKind::File, Some(0)));

        // Everything beneath the container, in one listing: the blob three
        // levels down is among it, and each folder comes once.
        assert_eq!(
            list(&provider, &container, Depth::Unlimited).await,
            [
                dir("one"),
                dir("one/two"),
                file("one/two/New file.txt"),
                dir("one/two/New folder"),
            ]
        );
        assert_eq!(
            list(&provider, &format!("{container}/one"), Depth::Unlimited).await,
            [
                dir("one/two"),
                file("one/two/New file.txt"),
                dir("one/two/New folder"),
            ]
        );
        // And beneath the account, whatever else its other containers hold.
        let below_account = list(&provider, "", Depth::Unlimited).await;
        assert!(
            below_account.contains(&file("one/two/New file.txt"))
                && below_account.contains(&(format!("/test/{container}"), EntryKind::Dir)),
            "{below_account:?}"
        );

        provider
            .remove(&at(&container), RemoveMode::Permanent)
            .await
            .unwrap_or_else(|e| panic!("remove container: {e}"));
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
