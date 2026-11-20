//! The client against a registry of the test's own: a server on this
//! machine and a key made here. No test asks get.openbreach.dev.

use std::time::{Duration, SystemTime};

use serde_json::json;

use super::fixtures::{INDEX_PATH, Server, Signer};
use super::*;

/// One build of a plugin, as the publisher writes it.
fn build(version: &str, rick: &str, platform: &str) -> serde_json::Value {
    json!({
        "version": version,
        "app_version": rick,
        "api_version": 23,
        "platform": platform,
        "url": format!("https://get.openbreach.dev/plugins/rick/{rick}/git-{version}-{platform}.rkp"),
        "size": 1_843_200,
        "sha256": "aa".repeat(32),
    })
}

/// An index of `schema` with Git and its `builds`, and S3 with one build
/// for Linux and Rick 0.6.0.
fn index(schema: u32, builds: &[serde_json::Value]) -> serde_json::Value {
    json!({
        "schema": schema,
        "generated": "2026-11-19T02:00:00Z",
        "apps": {
            "rick": {
                "latest": { "version": "0.6.0", "url": "https://example.invalid/v0.6.0" },
                "plugins": [
                    {
                        "id": "git",
                        "name": "Git",
                        "description": "Status badges and the Git actions of the context menu.",
                        "publisher": "Open-Breach",
                        "meta": {
                            "points": ["decorator", "action"],
                            "triggers": { "markers": [".git"] },
                        },
                        "builds": builds,
                    },
                    {
                        "id": "vfs-s3",
                        "name": "S3",
                        "description": "Amazon S3 and compatible stores as locations.",
                        "publisher": "Open-Breach",
                        "meta": {
                            "points": ["provider"],
                            "triggers": {
                                "connections": [{ "section": "cloud", "title": "Amazon S3" }],
                            },
                        },
                        "builds": [build("0.6.0", "0.6.0", "linux-x64")],
                    },
                ],
            },
            // Another application of the registry, in a shape of its own.
            "meeseeks": { "latest": { "version": "0.3.1", "url": "…" }, "plugins": "none" },
        },
        "a key of a later schema": [1, 2, 3],
    })
}

/// The index every test starts from: Git for Rick 0.6.0 on two platforms.
fn good() -> serde_json::Value {
    index(
        SCHEMA,
        &[
            build("0.6.0", "0.6.0", "linux-x64"),
            build("0.6.0", "0.6.0", "windows-x64"),
        ],
    )
}

/// The source `rick.toml` would name for `server` and `signer`.
fn source(server: &Server, signer: &Signer) -> Source {
    Source::new(Some(&server.index_url()), Some(&signer.public_key()))
}

fn parsed(index: &serde_json::Value) -> Index {
    Index::parse(index.to_string().as_bytes()).unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn a_good_index_is_fetched_verified_and_cached() {
    runtime::block_on(async {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let (server, signer) = (Server::start(), Signer::new(1));
        server.publish(&signer, &good());
        let source = source(&server, &signer);
        let cache = Cache::new(dir.path());
        assert_eq!(cache.fetched(), None, "nothing is cached before");

        let cached = refresh(&source, &cache, "0.6.0")
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(cached.index, parsed(&good()));
        assert_eq!(cached.index.generated, "2026-11-19T02:00:00Z");
        let ids: Vec<&str> = cached
            .index
            .plugins()
            .iter()
            .map(|p| p.id.as_str())
            .collect();
        assert_eq!(ids, ["git", "vfs-s3"]);

        // One GET for each of the two files, as Rick and as nobody in
        // particular.
        let requests = server.requests();
        let paths: Vec<&str> = requests.iter().map(|r| r.path.as_str()).collect();
        assert_eq!(paths, [INDEX_PATH, "registry/index.json.minisig"]);
        for request in &requests {
            assert_eq!(request.header("user-agent"), Some("rick/0.6.0"));
            for (name, _) in &request.headers {
                assert!(
                    ["accept", "host", "user-agent"].contains(&name.as_str()),
                    "{name} was sent with {}",
                    request.path
                );
            }
        }

        // The cache holds the two files as they were served.
        let folder = dir.path().join(CACHE_DIR);
        assert_eq!(
            std::fs::read(folder.join(INDEX_FILE)).ok(),
            Some(good().to_string().into_bytes())
        );
        assert!(folder.join(SIGNATURE_FILE).is_file());
        assert_eq!(cache.fetched(), Some(cached.fetched));
    });
}

#[test]
fn a_cached_index_is_read_back_and_verified_on_every_read() {
    runtime::block_on(async {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let (server, signer) = (Server::start(), Signer::new(1));
        server.publish(&signer, &good());
        let source = source(&server, &signer);
        let key = source.key.clone().unwrap_or_else(|| panic!("a key"));
        let cache = Cache::new(dir.path());
        assert!(matches!(cache.read(&key), Ok(None)), "nothing cached yet");

        let fetched = refresh(&source, &cache, "0.6.0")
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        let asked = server.requests().len();
        let read = cache
            .read(&key)
            .unwrap_or_else(|e| panic!("{e}"))
            .unwrap_or_else(|| panic!("an index is cached"));
        assert_eq!(read, fetched);
        assert_eq!(server.requests().len(), asked, "a read asks nobody");

        // Another key does not verify what this one signed.
        let other = Source::new(None, Some(&Signer::new(2).public_key()))
            .key
            .unwrap_or_else(|| panic!("a key"));
        assert!(matches!(cache.read(&other), Err(Error::Unverified)));

        // A cached index that was changed on the disk is not used.
        let path = dir.path().join(CACHE_DIR).join(INDEX_FILE);
        let changed = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{e}"))
            .replace("get.openbreach.dev", "example.invalid");
        std::fs::write(&path, changed).unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(cache.read(&key), Err(Error::Unverified)));

        // And neither is one whose signature is gone.
        std::fs::remove_file(dir.path().join(CACHE_DIR).join(SIGNATURE_FILE))
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(cache.read(&key), Err(Error::Unverified)));
    });
}

#[test]
fn a_bad_signature_is_not_used_and_the_last_good_index_stays() {
    runtime::block_on(async {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let (server, signer) = (Server::start(), Signer::new(1));
        server.publish(&signer, &good());
        let source = source(&server, &signer);
        let key = source.key.clone().unwrap_or_else(|| panic!("a key"));
        let cache = Cache::new(dir.path());
        let first = refresh(&source, &cache, "0.6.0")
            .await
            .unwrap_or_else(|e| panic!("{e}"));

        let newer = index(SCHEMA, &[build("0.7.0", "0.7.0", "linux-x64")]);
        let newer_bytes = newer.to_string().into_bytes();
        let signature_path = format!("{INDEX_PATH}.minisig");
        let cases: [(&str, String); 4] = [
            // The index changed and the signature is still the old one's.
            ("of other bytes", signer.sign(good().to_string().as_bytes())),
            // Signed, but not by the registry's key.
            ("of another key", Signer::new(2).sign(&newer_bytes)),
            ("no signature at all", "not a signature".to_owned()),
            // The right signature with one character of it changed.
            ("damaged", {
                let good = signer.sign(&newer_bytes);
                let at = good.find('\n').map_or(0, |at| at + 20);
                let swap = if good[at..].starts_with('A') {
                    "B"
                } else {
                    "A"
                };
                format!("{}{swap}{}", &good[..at], &good[at + 1..])
            }),
        ];
        for (case, signature) in cases {
            server.put(INDEX_PATH, newer_bytes.clone());
            server.put(&signature_path, signature);
            let refused = refresh(&source, &cache, "0.6.0").await;
            assert!(
                matches!(refused, Err(Error::Unverified)),
                "{case}: {refused:?}"
            );
            let kept = cache.read(&key).unwrap_or_else(|e| panic!("{case}: {e}"));
            assert_eq!(
                kept.as_ref(),
                Some(&first),
                "{case}: the last good one stays"
            );
        }

        // The same index with its own signature is taken.
        server.publish(&signer, &newer);
        let taken = refresh(&source, &cache, "0.6.0")
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(taken.index, parsed(&newer));
    });
}

#[test]
fn a_newer_schema_is_treated_like_a_failed_fetch() {
    runtime::block_on(async {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let (server, signer) = (Server::start(), Signer::new(1));
        server.publish(&signer, &good());
        let source = source(&server, &signer);
        let key = source.key.clone().unwrap_or_else(|| panic!("a key"));
        let cache = Cache::new(dir.path());
        let first = refresh(&source, &cache, "0.6.0")
            .await
            .unwrap_or_else(|e| panic!("{e}"));

        // Properly signed, and written to a schema this Rick does not read.
        let mut later = index(SCHEMA + 1, &[build("0.9.0", "0.9.0", "linux-x64")]);
        later["apps"]["rick"]["plugins"] = json!("another shape altogether");
        server.publish(&signer, &later);
        let refused = refresh(&source, &cache, "0.6.0").await;
        assert!(
            matches!(refused, Err(Error::NewerSchema(schema)) if schema == SCHEMA + 1),
            "{refused:?}"
        );
        let kept = cache.read(&key).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(kept, Some(first), "the last good one stays");
    });
}

#[test]
fn without_a_network_the_fetch_fails_and_the_cache_stays() {
    runtime::block_on(async {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let (server, signer) = (Server::start(), Signer::new(1));
        server.publish(&signer, &good());
        let source = source(&server, &signer);
        let key = source.key.clone().unwrap_or_else(|| panic!("a key"));
        let cache = Cache::new(dir.path());
        let first = refresh(&source, &cache, "0.6.0")
            .await
            .unwrap_or_else(|e| panic!("{e}"));

        // Nobody listens any more: the connection is refused.
        server.stop().await;
        let failed = refresh(&source, &cache, "0.6.0").await;
        assert!(matches!(failed, Err(Error::Fetch(_))), "{failed:?}");
        let kept = cache.read(&key).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(kept, Some(first));
    });
}

#[test]
fn a_file_that_is_not_served_is_a_failed_fetch() {
    runtime::block_on(async {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let (server, signer) = (Server::start(), Signer::new(1));
        let source = source(&server, &signer);
        let cache = Cache::new(dir.path());

        // Neither file.
        let failed = refresh(&source, &cache, "0.6.0").await;
        assert!(matches!(failed, Err(Error::Fetch(_))), "{failed:?}");
        // The index without its signature.
        server.put(INDEX_PATH, good().to_string());
        let failed = refresh(&source, &cache, "0.6.0").await;
        assert!(matches!(failed, Err(Error::Fetch(_))), "{failed:?}");
        // A signature no signature is as long as.
        server.put(&format!("{INDEX_PATH}.minisig"), vec![b'A'; 64 * 1024]);
        let failed = refresh(&source, &cache, "0.6.0").await;
        assert!(matches!(failed, Err(Error::Fetch(_))), "{failed:?}");
        assert_eq!(cache.fetched(), None, "nothing was cached");
    });
}

#[test]
fn without_a_key_nothing_is_fetched() {
    runtime::block_on(async {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let (server, signer) = (Server::start(), Signer::new(1));
        server.publish(&signer, &good());
        // A build whose key file is empty, pointed at an index all the same.
        let source = Source::with_compiled("", Some(&server.index_url()), None);
        assert_eq!(source.key, None);
        let refused = refresh(&source, &Cache::new(dir.path()), "0.6.0").await;
        assert!(matches!(refused, Err(Error::NotSetUp)), "{refused:?}");
        assert_eq!(server.requests(), [], "not one request");
    });
}

#[test]
fn the_source_is_this_build_s_unless_rick_toml_says_otherwise() {
    let key = Signer::new(1).public_key();
    let other = Signer::new(2).public_key();
    let compiled = format!("untrusted comment: minisign public key\n{key}\n");

    // The committed file is the registry's key, and this build reads the
    // registry with it.
    let committed = include_str!("../keys/registry.pub");
    let own = Source::new(None, None);
    assert_eq!(own.url, INDEX_URL);
    assert!(!own.custom);
    assert_eq!(
        own.key,
        Key::parse(committed).unwrap_or_else(|e| panic!("{e}"))
    );
    assert!(own.key.is_some(), "this build has the registry on");
    assert_eq!(own.host(), "get.openbreach.dev");
    assert_eq!(
        own.signature_url(),
        "https://get.openbreach.dev/registry/index.json.minisig"
    );

    // With a key compiled in and nothing set, the build's own.
    let built = Source::with_compiled(&compiled, None, Some("  "));
    assert_eq!(
        built.key,
        Key::parse(&key).unwrap_or_else(|e| panic!("{e}"))
    );
    assert!(built.key.is_some() && !built.custom);

    // Another address keeps the key; another key keeps the address.
    let elsewhere = Source::with_compiled(&compiled, Some("http://127.0.0.1:8123/i.json"), None);
    assert_eq!(elsewhere.url, "http://127.0.0.1:8123/i.json");
    assert_eq!(elsewhere.host(), "127.0.0.1:8123");
    assert_eq!(elsewhere.key, built.key);
    assert!(elsewhere.custom);
    let rekeyed = Source::with_compiled(&compiled, Some(""), Some(&other));
    assert_eq!(rekeyed.url, INDEX_URL);
    assert_ne!(rekeyed.key, built.key);
    assert!(rekeyed.key.is_some() && rekeyed.custom);

    // The source of an application that is not the released binary has
    // no key but the one rick.toml names, whatever this build carries.
    assert_eq!(Source::named(None, None).key, None);
    assert_eq!(
        Source::named(Some("http://127.0.0.1:1/i.json"), None).key,
        None
    );
    let named = Source::named(Some("http://127.0.0.1:1/i.json"), Some(&other));
    assert_eq!(named.key, rekeyed.key);
    assert!(named.custom);

    // A key that was asked for and is none: the compiled one does not
    // stand in for it.
    let broken = Source::with_compiled(&compiled, None, Some("not a key"));
    assert_eq!(broken.key, None);
    assert!(broken.custom);
}

/// Trebuchet's `keys/registry.pub` as it is on `main` of
/// Open-Breach/trebuchet, read at its commit `38164ff`: the SHA-256 of the
/// file, which `sha256sum keys/registry.pub` prints there.
const TREBUCHET_KEY_SHA256: &str =
    "e5d6bf252f53b832f3c874081b2d3e354b565104c6b2540db2b476a735da27dc";
/// The id minisign gave that key: the last word of the file's first line,
/// and what a signature names its key by.
const TREBUCHET_KEY_ID: &str = "197B77080066F36E";

/// The key compiled into Rick is a minisign public key, and it is
/// Trebuchet's file byte for byte.
///
/// Nothing is asked for the comparison: the test holds what Trebuchet's
/// file was when it was copied, its SHA-256 and the key's id, and the
/// committed file must hash to the one and carry the other. A file that
/// was edited here, or copied from elsewhere, fails. What the test cannot
/// see is a key Trebuchet replaces later: that is a new file there, a new
/// hash here and a release of Rick (Trebuchet's spec, 7.5).
#[test]
fn the_key_compiled_in_is_a_minisign_public_key_and_trebuchet_s_file() {
    use base64::Engine as _;
    use sha2::Digest as _;

    let committed = include_str!("../keys/registry.pub");

    // A `.pub` file as `minisign -G` writes it: the comment that names
    // the key's id, then one line of base64.
    let mut lines = committed.lines();
    assert_eq!(
        lines.next(),
        Some(format!("untrusted comment: minisign public key {TREBUCHET_KEY_ID}").as_str())
    );
    let line = lines.next().unwrap_or_else(|| panic!("no key line"));
    assert_eq!(lines.next(), None, "two lines and no more");
    // The line is the algorithm, Ed25519, the eight bytes of the key's
    // id and the 32 bytes of the key; the id is the comment's.
    let raw = base64::engine::general_purpose::STANDARD
        .decode(line)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(raw.len(), 2 + 8 + 32);
    assert_eq!(&raw[..2], b"Ed");
    let mut id = [0u8; 8];
    id.copy_from_slice(&raw[2..10]);
    assert_eq!(format!("{:016X}", u64::from_le_bytes(id)), TREBUCHET_KEY_ID);

    // The client reads it as its key, and it is the key of this build.
    let key = Key::parse(committed).unwrap_or_else(|e| panic!("{e}"));
    assert!(key.is_some());
    assert_eq!(Source::new(None, None).key, key);
    // A signature of another key is not this key's.
    let (other, bytes) = (Signer::new(1), b"an index");
    assert!(matches!(
        key.unwrap_or_else(|| panic!("no key"))
            .verify(bytes, &other.sign(bytes)),
        Err(Error::Unverified)
    ));

    // Trebuchet's file, byte for byte.
    assert_eq!(
        hex::encode(sha2::Sha256::digest(committed.as_bytes())),
        TREBUCHET_KEY_SHA256
    );
}

#[test]
fn a_public_key_is_one_base64_line_with_or_without_its_comment() {
    let key = Signer::new(7).public_key();
    let bare = Key::parse(&key).unwrap_or_else(|e| panic!("{e}"));
    assert!(bare.is_some());
    let file = format!("untrusted comment: minisign public key 0707070707070707\n{key}\n");
    assert_eq!(Key::parse(&file).unwrap_or_else(|e| panic!("{e}")), bare);
    assert_eq!(
        Key::parse(&format!("\n  {key}  \n\n")).unwrap_or_else(|e| panic!("{e}")),
        bare
    );

    for empty in ["", "\n", "  \n\t\n", "untrusted comment: no key yet\n"] {
        assert!(matches!(Key::parse(empty), Ok(None)), "{empty:?}");
    }
    for bad in ["not a key", "RWQ=", &format!("{key}\n{key}")] {
        assert!(matches!(Key::parse(bad), Err(Error::Key(_))), "{bad:?}");
    }
}

#[test]
fn a_build_fits_one_platform_and_one_rick_as_whole_strings() {
    let index = parsed(&index(
        SCHEMA,
        &[
            build("0.5.0", "0.5.0", "linux-x64"),
            build("0.6.0", "0.6.0", "linux-x64"),
            build("0.6.0", "0.6.0", "macos-arm64"),
            build("0.6.1", "0.6.1", "linux-x64"),
        ],
    ));
    let git = index.plugin("git").unwrap_or_else(|| panic!("git"));
    let fitting = |platform: &str, rick: &str| {
        git.fitting(platform, rick)
            .map(|build| (build.version.as_str(), build.platform.as_str()))
    };
    assert_eq!(fitting("linux-x64", "0.6.0"), Some(("0.6.0", "linux-x64")));
    assert_eq!(
        fitting("macos-arm64", "0.6.0"),
        Some(("0.6.0", "macos-arm64"))
    );
    assert_eq!(fitting("linux-x64", "0.6.1"), Some(("0.6.1", "linux-x64")));
    // No build for the platform, none for the release, and no version
    // that is nearly the same.
    assert_eq!(fitting("windows-x64", "0.6.0"), None);
    assert_eq!(fitting("macos-arm64", "0.6.1"), None);
    assert_eq!(fitting("linux-x64", "0.6"), None);
    assert_eq!(fitting("linux-x64", "0.6.0+local"), None);
    assert_eq!(fitting("linux", "0.6.0"), None);

    // The offer of a Rick: the plugins with such a build, in the index's
    // order, each with it.
    let Offer::Plugins(offered) = index.offer("linux-x64", "0.6.0") else {
        panic!("a release is offered a list");
    };
    let ids: Vec<(&str, &str)> = offered
        .iter()
        .map(|o| (o.plugin.id.as_str(), o.build.version.as_str()))
        .collect();
    assert_eq!(ids, [("git", "0.6.0"), ("vfs-s3", "0.6.0")]);
    let Offer::Plugins(offered) = index.offer("macos-arm64", "0.6.0") else {
        panic!("a release is offered a list");
    };
    assert_eq!(offered.len(), 1, "S3 has no build for macOS here");
    // A release the registry does not have, between two it has and after
    // the newest: an empty list, and nobody's fault.
    assert_eq!(
        index.offer("linux-x64", "0.5.5"),
        Offer::Plugins(Vec::new())
    );
    assert_eq!(
        index.offer("linux-x64", "0.7.0"),
        Offer::Plugins(Vec::new())
    );
}

#[test]
fn a_pre_release_fits_nothing_and_neither_does_a_rick_older_than_the_registry() {
    let index = parsed(&index(
        SCHEMA,
        &[
            build("0.6.0", "0.6.0", "linux-x64"),
            // Even a build that names the pre-release is not offered.
            build("0.7.0-dev", "0.7.0-dev", "linux-x64"),
        ],
    ));
    assert_eq!(index.offer("linux-x64", "0.7.0-dev"), Offer::PreRelease);
    assert_eq!(index.offer("linux-x64", "0.6.0-rc.1"), Offer::PreRelease);
    assert!(is_pre_release("0.6.0-dev") && is_pre_release("nightly-2026"));
    assert!(!is_pre_release("0.6.0") && !is_pre_release("0.6.0+build.5"));

    // The registry's oldest release is 0.6.0 (the pre-release build is
    // newer than it): a 0.5 is told its plugins are gone.
    assert_eq!(index.offer("linux-x64", "0.5.0"), Offer::TooOld);
    assert_eq!(index.offer("windows-x64", "0.5.9"), Offer::TooOld);
    assert!(matches!(index.offer("linux-x64", "0.6.0"), Offer::Plugins(list) if list.len() == 2));
    // An index without a build is too old for nobody.
    let empty = parsed(&json!({ "schema": 1, "apps": {} }));
    assert_eq!(
        empty.offer("linux-x64", "0.1.0"),
        Offer::Plugins(Vec::new())
    );
}

#[test]
fn of_two_builds_for_one_rick_the_higher_plugin_version_fits() {
    let index = parsed(&index(
        SCHEMA,
        &[
            build("0.6.9", "0.6.0", "linux-x64"),
            build("0.6.10", "0.6.0", "linux-x64"),
            build("0.6.2", "0.6.0", "linux-x64"),
        ],
    ));
    let git = index.plugin("git").unwrap_or_else(|| panic!("git"));
    assert_eq!(
        git.fitting("linux-x64", "0.6.0")
            .map(|b| b.version.as_str()),
        Some("0.6.10")
    );
}

#[test]
fn an_index_is_read_leniently_and_only_as_far_as_rick_s_entry() {
    // Unknown keys and the other applications are skipped: `good` has
    // both. What a plugin is for arrives whole.
    let index = parsed(&good());
    let git = index.plugin("git").unwrap_or_else(|| panic!("git"));
    assert_eq!(git.name, "Git");
    assert_eq!(git.publisher, "Open-Breach");
    assert_eq!(git.meta.points, ["decorator", "action"]);
    assert_eq!(git.meta.triggers.markers, [".git"]);
    assert_eq!(git.meta.triggers.previews, [] as [String; 0]);
    let s3 = index.plugin("vfs-s3").unwrap_or_else(|| panic!("vfs-s3"));
    assert_eq!(
        s3.meta.triggers.connections,
        [Connection {
            section: "cloud".to_owned(),
            title: "Amazon S3".to_owned(),
        }]
    );
    let build = &s3.builds[0];
    assert_eq!((build.api_version, build.size), (23, 1_843_200));
    assert_eq!(build.sha256, "aa".repeat(32));

    // An index without Rick, and one that says nothing at all.
    let other_apps_only = parsed(&json!({ "schema": 1, "apps": { "meeseeks": {} } }));
    assert_eq!(other_apps_only.plugins(), []);
    assert_eq!(parsed(&json!({})).plugins(), []);
    // What is no index is said to be none.
    for bad in ["", "[]", "{ \"schema\": \"one\" }", "{ \"apps\": 4 }"] {
        assert!(
            matches!(Index::parse(bad.as_bytes()), Err(Error::Unreadable(_))),
            "{bad:?}"
        );
    }
}

/// An index whose entry for Rick names `latest` and no plugin.
fn with_latest(latest: &serde_json::Value) -> Index {
    parsed(&json!({ "schema": 1, "apps": { "rick": { "latest": latest } } }))
}

#[test]
fn the_index_names_the_newest_release_of_rick() {
    // `good` names 0.6.0, as the publisher writes it on every release.
    let index = parsed(&good());
    let latest = index.latest().unwrap_or_else(|| panic!("latest"));
    assert_eq!(latest.version, "0.6.0");
    assert_eq!(latest.url, "https://example.invalid/v0.6.0");

    // Newer than the Rick that asks, and only then.
    for (rick, newer) in [
        ("0.5.0", true),
        ("0.5.9", true),
        ("0.6.0", false),
        ("0.6.1", false),
        ("0.10.0", false),
        ("1.0.0", false),
        // A pre-release Rick is told of nothing, not even of its own
        // release; what is no version is not compared.
        ("0.6.0-rc1", false),
        ("0.5.0-dev", false),
        ("six", false),
        ("", false),
    ] {
        assert_eq!(index.newer_than(rick).is_some(), newer, "{rick}");
    }
    assert_eq!(
        index
            .newer_than("0.5.0")
            .map(|latest| latest.version.as_str()),
        Some("0.6.0")
    );
    // Versions are compared as versions, not as text.
    let ten = with_latest(&json!({ "version": "0.10.0", "url": "https://example.org/r" }));
    assert!(ten.newer_than("0.9.0").is_some());
    assert!(ten.newer_than("0.10.0").is_none());

    // A pre-release is never announced as the newest, and what is no
    // version announces nothing.
    for version in ["0.7.0-rc1", "0.7.0-beta.2", "newest", "7"] {
        let index = with_latest(&json!({ "version": version, "url": "https://example.org/r" }));
        assert_eq!(index.newer_than("0.6.0"), None, "{version}");
    }
}

#[test]
fn an_entry_for_the_newest_release_that_is_none_costs_the_plugin_list_nothing() {
    for entry in [
        json!(null),
        json!("0.7.0"),
        json!(7),
        json!([]),
        json!({}),
        json!({ "url": "https://example.org/r" }),
        json!({ "version": 7 }),
        json!({ "version": "" }),
    ] {
        let index = parsed(&json!({
            "schema": 1,
            "apps": { "rick": { "latest": entry, "plugins": [{ "id": "git" }] } },
        }));
        assert_eq!(index.latest(), None, "{entry}");
        assert_eq!(index.newer_than("0.1.0"), None, "{entry}");
        assert!(index.plugin("git").is_some(), "{entry}");
    }
    // No entry at all, and no Rick at all.
    assert_eq!(parsed(&json!({ "schema": 1 })).latest(), None);
    // A version without an address is a release all the same.
    let bare = with_latest(&json!({ "version": "0.7.0" }));
    assert!(bare.newer_than("0.6.0").is_some());
    assert_eq!(bare.latest().and_then(Latest::page), None);
}

#[test]
fn only_an_https_address_is_a_release_page_to_open() {
    let page = |url: &str| {
        let latest = Latest {
            version: "0.7.0".to_owned(),
            url: url.to_owned(),
        };
        latest.page().map(str::to_owned)
    };
    for good in [
        "https://github.com/Open-Breach/rick/releases/tag/v0.7.0",
        "https://get.openbreach.dev/rick/0.7.0/",
    ] {
        assert_eq!(page(good).as_deref(), Some(good));
    }
    for bad in [
        "",
        "…",
        "http://github.com/Open-Breach/rick/releases/tag/v0.7.0",
        "http://127.0.0.1:8123/release",
        "file:///etc/passwd",
        "rick://open?path=/tmp",
        "javascript:alert(1)",
        "github.com/Open-Breach/rick",
        "https://",
    ] {
        assert_eq!(page(bad), None, "{bad}");
    }
}

#[test]
fn the_platforms_are_named_as_the_release_archives_are() {
    assert_eq!(platform_name("linux", "x86_64"), "linux-x64");
    assert_eq!(platform_name("linux", "aarch64"), "linux-arm64");
    assert_eq!(platform_name("macos", "aarch64"), "macos-arm64");
    assert_eq!(platform_name("windows", "x86_64"), "windows-x64");
    let own = platform();
    assert!(
        own.split_once('-')
            .is_some_and(|(os, arch)| os == std::env::consts::OS && !arch.is_empty()),
        "{own}"
    );
    assert_eq!(
        user_agent(RICK),
        format!("rick/{}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn the_background_check_is_due_once_a_day() {
    let now = SystemTime::now();
    let ago = |hours: u64| now - Duration::from_secs(hours * 60 * 60);
    assert!(due(None, now), "nothing cached");
    assert!(!due(Some(now), now));
    assert!(!due(Some(ago(23)), now));
    assert!(due(Some(ago(24)), now));
    assert!(due(Some(ago(24 * 30)), now));
    // A cached file dated after now is no reason to wait.
    assert!(due(Some(now + Duration::from_secs(3600)), now));
    // The same rule for a check at another interval.
    let hour = Duration::from_hours(1);
    assert!(due_after(None, now, hour));
    assert!(!due_after(Some(now - Duration::from_mins(59)), now, hour));
    assert!(due_after(Some(now - hour), now, hour));
    assert_eq!(CHECK_EVERY, Duration::from_hours(24));
}

/// The SHA-256 of `bytes`, as the index writes it.
fn sha256(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    hex::encode(sha2::Sha256::digest(bytes))
}

#[test]
fn a_package_is_downloaded_and_held_against_the_index() {
    runtime::block_on(async {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let server = Server::start();
        let package: Vec<u8> = (0..200_000u32).flat_map(u32::to_le_bytes).collect();
        server.put("plugins/git.rkp", package.clone());
        let url = server.url("plugins/git.rkp");
        let to = dir.path().join("git.rkp");
        let hash = sha256(&package);
        let expected = Expected {
            size: package.len() as u64,
            sha256: &hash,
        };

        let mut seen = Vec::new();
        let arrived = download(&url, &to, "0.6.0", Some(expected), &mut |done| {
            seen.push(done);
        })
        .await
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(arrived.size, package.len() as u64);
        assert_eq!(arrived.sha256, hash);
        assert_eq!(std::fs::read(&to).ok(), Some(package.clone()));
        assert_eq!(seen.last(), Some(&(package.len() as u64)));
        assert!(seen.is_sorted(), "the progress only grows");
        let requests = server.requests();
        assert_eq!(requests.len(), 1, "one GET");
        assert_eq!(requests[0].header("user-agent"), Some("rick/0.6.0"));

        // The index may write the hash in capitals.
        let capitals = hash.to_uppercase();
        let again = Expected {
            sha256: &capitals,
            ..expected
        };
        download(&url, &to, "0.6.0", Some(again), &mut |_| {})
            .await
            .unwrap_or_else(|e| panic!("{e}"));

        // Without an expectation nothing is checked, and what arrived is
        // said.
        let unchecked = download(&url, &to, "0.6.0", None, &mut |_| {})
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(unchecked, arrived);
    });
}

#[test]
fn a_package_that_is_another_one_is_deleted() {
    runtime::block_on(async {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let server = Server::start();
        let package = vec![7u8; 100_000];
        let hash = sha256(&package);
        let expected = Expected {
            size: package.len() as u64,
            sha256: &hash,
        };
        let url = server.url("plugins/git.rkp");
        let to = dir.path().join("git.rkp");

        let mut changed = package.clone();
        changed[50_000] = 8;
        let cases: [(&str, Vec<u8>); 3] = [
            ("one byte changed", changed),
            ("too short", package[..99_999].to_vec()),
            ("too long", vec![7u8; 400_000]),
        ];
        for (case, served) in cases {
            server.put("plugins/git.rkp", served);
            let refused = download(&url, &to, "0.6.0", Some(expected), &mut |_| {}).await;
            assert!(
                matches!(refused, Err(Error::Mismatch)),
                "{case}: {refused:?}"
            );
            assert!(!to.exists(), "{case}: the file is deleted");
        }

        // A package that is not there is a failed fetch, and leaves no
        // file either.
        let missing = server.url("plugins/none.rkp");
        let failed = download(&missing, &to, "0.6.0", Some(expected), &mut |_| {}).await;
        assert!(matches!(failed, Err(Error::Fetch(_))), "{failed:?}");
        assert!(!to.exists());
        assert_eq!(
            std::fs::read_dir(dir.path()).map(Iterator::count).ok(),
            Some(0)
        );
    });
}

#[test]
fn a_package_address_is_https_or_this_machine() {
    for good in [
        "https://example.org/plugin.rkp",
        "  https://get.openbreach.dev/plugins/rick/0.6.0/git-0.6.0-linux-x64.rkp ",
        "https://example.org:8443/download?file=plugin",
        "https://192.168.1.10/plugin.rkp",
        // Plain HTTP to this machine: the loopback addresses, however
        // they are spelt, and the one name.
        "http://127.0.0.1:8123/plugin.rkp",
        "http://127.8.9.10/plugin.rkp",
        "http://127.1:8123/plugin.rkp",
        "http://0x7f.0.0.1/plugin.rkp",
        "http://2130706433/plugin.rkp",
        "http://[::1]:9000/plugin.rkp",
        "http://[0:0:0:0:0:0:0:1]/plugin.rkp",
        "http://localhost/plugin.rkp",
        "HTTP://LOCALHOST:8123/plugin.rkp",
    ] {
        assert!(is_package_address(good), "{good}");
    }
    for bad in [
        "",
        "plugin.rkp",
        "ftp://example.org/plugin.rkp",
        "file:///tmp/plugin.rkp",
        "https://",
        // Plain HTTP to another machine, near or far.
        "http://example.org/plugin.rkp",
        "http://192.168.1.10/plugin.rkp",
        "http://10.0.0.1/plugin.rkp",
        "http://169.254.1.1/plugin.rkp",
        "http://[fe80::1]/plugin.rkp",
        "http://128.0.0.1/plugin.rkp",
        // Every address, which is not the loopback one.
        "http://0.0.0.0/plugin.rkp",
        "http://[::]/plugin.rkp",
        // An IPv4 address inside an IPv6 one is not taken apart.
        "http://[::ffff:127.0.0.1]/plugin.rkp",
        // No name but `localhost`, whatever it resolves to or looks like.
        "http://localhost./plugin.rkp",
        "http://sub.localhost/plugin.rkp",
        "http://localhost.example.org/plugin.rkp",
        "http://127.0.0.1.example.org/plugin.rkp",
        "http://localtest.me/plugin.rkp",
        // The host is what follows the last `@`, not what stands before.
        "http://127.0.0.1@example.org/plugin.rkp",
        "http://localhost:80@example.org/plugin.rkp",
        "http://example.org#@127.0.0.1/plugin.rkp",
    ] {
        assert!(!is_package_address(bad), "{bad}");
    }
}

/// A name for another machine, which no resolver is asked for: the
/// clients of [`elsewhere`] take it for the test's own server.
const ELSEWHERE: &str = "plugins.example";

/// Clients for which [`ELSEWHERE`] is `server`: a request to that name,
/// should one be made, arrives there and is counted. With the address of
/// `path` under that name.
fn elsewhere(server: &Server, path: &str) -> (crate::fetch::Clients, String) {
    let clients = crate::fetch::Clients::resolving("0.6.0", ELSEWHERE, server.address())
        .unwrap_or_else(|e| panic!("{e}"));
    let port = server.address().port();
    (clients, format!("http://{ELSEWHERE}:{port}/{path}"))
}

#[test]
fn plain_http_to_another_machine_is_refused_before_any_connection() {
    runtime::block_on(async {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let server = Server::start();
        server.put("plugins/git.rkp", vec![7u8; 1000]);
        let (clients, remote) = elsewhere(&server, "plugins/git.rkp");
        let to = dir.path().join("git.rkp");

        // The name does lead somewhere: asked past the rule, the package
        // is there, over one connection.
        let past = clients.local.get(&remote).send().await;
        assert_eq!(past.map(|answer| answer.status().as_u16()).ok(), Some(200));
        assert_eq!(server.connections(), 1);
        assert_eq!(server.requests().len(), 1);

        // Through the door it is not asked: refused as the address it
        // is, with no file.
        let refused =
            crate::download::download_with(&clients, &remote, &to, None, &mut |_| {}).await;
        assert!(
            matches!(
                &refused,
                Err(Error::Refused { url, redirected: false }) if *url == remote
            ),
            "{refused:?}"
        );
        assert!(!to.exists());

        // Neither is the index, nor its signature.
        let source = Source {
            url: remote.clone(),
            key: Source::named(None, Some(&Signer::new(1).public_key())).key,
            custom: true,
        };
        assert!(source.key.is_some());
        let refused = clients.get(&source.url).await;
        assert!(matches!(refused, Err(Error::Refused { .. })), "{refused:?}");
        let refused = fetch(&source, "0.6.0").await;
        assert!(matches!(refused, Err(Error::Refused { .. })), "{refused:?}");

        // A connection opened later is counted after every one before
        // it: there was none in between, and no request.
        let after = clients.local.get(server.url("after")).send().await;
        assert_eq!(after.map(|answer| answer.status().as_u16()).ok(), Some(404));
        assert_eq!(server.connections(), 2, "the two the test opened itself");
        let paths: Vec<String> = server.requests().into_iter().map(|r| r.path).collect();
        assert_eq!(paths, ["plugins/git.rkp", "after"]);
    });
}

#[test]
fn a_redirect_to_plain_http_on_another_machine_is_refused() {
    runtime::block_on(async {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let server = Server::start();
        let package = vec![7u8; 1000];
        server.put("plugins/git.rkp", package.clone());
        let (clients, remote) = elsewhere(&server, "plugins/git.rkp");
        // Straight to the other machine, and by way of this one.
        server.redirect("moved.rkp", &remote);
        server.redirect("moved-twice.rkp", "/moved.rkp");
        let to = dir.path().join("git.rkp");

        for (start, asked) in [
            ("moved.rkp", vec!["moved.rkp"]),
            ("moved-twice.rkp", vec!["moved-twice.rkp", "moved.rkp"]),
        ] {
            let before = server.requests().len();
            let refused = crate::download::download_with(
                &clients,
                &server.url(start),
                &to,
                None,
                &mut |_| {},
            )
            .await;
            assert!(
                matches!(
                    &refused,
                    Err(Error::Refused { url, redirected: true }) if *url == remote
                ),
                "{start}: {refused:?}"
            );
            assert!(!to.exists(), "{start}: no file");
            // Only this machine was asked, and the package never.
            let paths: Vec<String> = server.requests()[before..]
                .iter()
                .map(|r| r.path.clone())
                .collect();
            assert_eq!(paths, asked, "{start}");
        }
        // As many connections as requests: none was opened and left.
        let after = clients.local.get(server.url("after")).send().await;
        assert!(after.is_ok());
        assert_eq!(server.connections(), 4);
        assert_eq!(server.requests().len(), 4);

        // The same redirect, asked past the rule, does arrive: it is the
        // rule that stopped it, not the server.
        let past = clients.local.get(&remote).send().await;
        assert_eq!(past.map(|answer| answer.status().as_u16()).ok(), Some(200));
    });
}

/// The decision the redirect loop makes for each address it is sent on
/// to, asked directly. A server of the test's own cannot stand for
/// "another machine" here: the only addresses elsewhere the client asks
/// at all are `https://`, and the tests have no TLS server. The loop
/// itself is run by the tests around this one.
#[test]
fn another_machine_does_not_send_a_request_on_to_this_one() {
    use crate::fetch::{Way, onward};
    let url = |text: &str| reqwest::Url::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"));

    // From elsewhere to this machine: refused, whatever the scheme, and
    // however this machine is written.
    for (from, to) in [
        ("https://example.org/a.rkp", "http://127.0.0.1:8123/a.rkp"),
        ("https://example.org/a.rkp", "https://127.0.0.1:8443/a.rkp"),
        ("https://example.org/a.rkp", "http://localhost/a.rkp"),
        ("https://example.org/a.rkp", "https://localhost/a.rkp"),
        ("https://example.org/a.rkp", "http://[::1]:9000/a.rkp"),
        ("https://example.org/a.rkp", "https://[::1]/a.rkp"),
        ("https://example.org/a.rkp", "https://127.8.9.10/a.rkp"),
        ("https://192.168.1.10/a.rkp", "http://127.1/a.rkp"),
        (
            "https://get.openbreach.dev/registry/index.json",
            "http://localhost:8123/registry/index.json",
        ),
    ] {
        let refused = onward(&url(from), &url(to));
        assert!(
            matches!(&refused, Err(Error::Inward { url: named }) if *named == url(to).as_str()),
            "{from} to {to}: {refused:?}"
        );
    }

    // From this machine to this machine: followed, by the client of the
    // address's scheme.
    for (from, to, way) in [
        (
            "http://127.0.0.1:8123/a",
            "http://127.0.0.1:9000/b",
            Way::Local,
        ),
        (
            "http://127.0.0.1:8123/a",
            "http://localhost:8123/b",
            Way::Local,
        ),
        ("http://localhost/a", "http://[::1]/b", Way::Local),
        (
            "http://127.0.0.1/a",
            "https://localhost:8443/b",
            Way::Secure,
        ),
        ("https://localhost:8443/a", "http://127.0.0.1/b", Way::Local),
        // Outward, and from elsewhere to elsewhere: `https://` as ever.
        ("http://127.0.0.1/a", "https://example.org/b", Way::Secure),
        (
            "https://example.org/a",
            "https://cdn.example.org/b",
            Way::Secure,
        ),
    ] {
        let taken = onward(&url(from), &url(to));
        assert_eq!(taken.as_ref().ok(), Some(&way), "{from} to {to}: {taken:?}");
    }

    // Plain HTTP elsewhere stays what it was, from wherever.
    for (from, to) in [
        ("http://127.0.0.1/a", "http://example.org/b"),
        ("https://example.org/a", "http://cdn.example.org/b"),
        ("https://example.org/a", "ftp://example.org/b"),
    ] {
        let refused = onward(&url(from), &url(to));
        assert!(
            matches!(
                &refused,
                Err(Error::Refused { url: named, redirected: true }) if *named == url(to).as_str()
            ),
            "{from} to {to}: {refused:?}"
        );
    }
}

#[test]
fn a_redirect_within_the_rule_is_followed() {
    runtime::block_on(async {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let server = Server::start();
        let package = vec![7u8; 1000];
        server.put("plugins/git.rkp", package.clone());
        let port = server.address().port();
        // By a path, by the whole address, and by the name of this
        // machine, which is the loopback address without a lookup.
        server.redirect("by-path.rkp", "/plugins/git.rkp");
        server.redirect("by-address.rkp", &server.url("plugins/git.rkp"));
        server.redirect(
            "by-name.rkp",
            &format!("http://localhost:{port}/plugins/git.rkp"),
        );
        server.redirect("in-a-row.rkp", "by-address.rkp");
        let to = dir.path().join("git.rkp");
        for start in [
            "by-path.rkp",
            "by-address.rkp",
            "by-name.rkp",
            "in-a-row.rkp",
        ] {
            let arrived = download(&server.url(start), &to, "0.6.0", None, &mut |_| {})
                .await
                .unwrap_or_else(|e| panic!("{start}: {e}"));
            assert_eq!(arrived.size, 1000, "{start}");
            assert_eq!(std::fs::read(&to).ok(), Some(package.clone()), "{start}");
        }
        for request in server.requests() {
            assert_eq!(request.header("user-agent"), Some("rick/0.6.0"));
            assert_eq!(request.header("referer"), None, "{}", request.path);
        }

        // A redirect that leads back to itself ends, as a failed fetch.
        server.redirect("round.rkp", "/round.rkp");
        let before = server.requests().len();
        let to = dir.path().join("none.rkp");
        let failed = download(&server.url("round.rkp"), &to, "0.6.0", None, &mut |_| {}).await;
        assert!(matches!(failed, Err(Error::Fetch(_))), "{failed:?}");
        assert_eq!(server.requests().len() - before, 11);
        assert!(!to.exists());

        // One to what is no address at all is refused like any other.
        server.redirect("odd.rkp", "ftp://example.org/git.rkp");
        let refused = download(&server.url("odd.rkp"), &to, "0.6.0", None, &mut |_| {}).await;
        assert!(
            matches!(
                refused,
                Err(Error::Refused {
                    redirected: true,
                    ..
                })
            ),
            "{refused:?}"
        );
        assert!(!to.exists());
    });
}

#[test]
fn a_download_is_named_after_the_file_its_address_names() {
    for (url, name) in [
        (
            "https://example.org/git-0.6.0-linux-x64.rkp",
            "git-0.6.0-linux-x64.rkp",
        ),
        ("https://example.org/a/b/hello.rkp?token=1#top", "hello.rkp"),
        ("https://example.org/download/hello", "hello.rkp"),
        // Never a path: no separator survives, and no leading dot.
        (
            "https://example.org/he%20llo/..%2F..%2Fsly.rkp",
            "2F..2Fsly.rkp",
        ),
        ("https://example.org/", "plugin.rkp"),
        ("https://example.org", "plugin.rkp"),
        ("https://example.org/.rkp", "plugin.rkp"),
        ("not an address", "plugin.rkp"),
    ] {
        assert_eq!(package_name(url), name, "{url}");
    }
}
