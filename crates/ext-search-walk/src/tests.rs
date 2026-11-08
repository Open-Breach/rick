use std::fs;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

use ext::{Filter, Query, QueryKind, Registry, SearchHit, SearchIndex, SearchScope};
use futures::StreamExt;
use types::{CancelToken, Error, Location, ProviderId};

use super::*;

fn nested(root: &Path, levels: usize) {
    let mut dir = root.to_path_buf();
    for level in 0..levels {
        dir.push(format!("level{level}"));
        fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        fs::write(dir.join(format!("foo{level}.txt")), b"x").unwrap_or_else(|e| panic!("{e}"));
        fs::write(dir.join(format!("bar{level}.txt")), b"x").unwrap_or_else(|e| panic!("{e}"));
    }
    fs::create_dir_all(root.join("node_modules")).unwrap_or_else(|e| panic!("{e}"));
    fs::write(root.join("node_modules/foo-ignored.js"), b"x").unwrap_or_else(|e| panic!("{e}"));
    fs::write(root.join(".foo-hidden"), b"x").unwrap_or_else(|e| panic!("{e}"));
}

fn loc(path: &Path) -> Location {
    Location::new(PROVIDER_ID, ext::platform::paths::from_path(path))
}

/// A name query under `root` with the pane's hidden toggle off and the
/// default ignore list.
fn query(root: &Path, text: &str) -> Query {
    Query {
        text: text.into(),
        root: Some(loc(root)),
        kind: QueryKind::Name,
        filters: vec![
            Filter::NoHidden,
            Filter::SkipNames(vec!["node_modules".into()]),
        ],
    }
}

/// Every hit of the query, and when the first batch arrived.
fn collect(q: &Query) -> (Option<Duration>, Vec<SearchHit>) {
    let started = Instant::now();
    ext::runtime::block_on(async {
        let mut stream = WalkIndex.query(q, CancelToken::new());
        let mut first_at = None;
        let mut found = Vec::new();
        while let Some(batch) = stream.next().await {
            first_at.get_or_insert_with(|| started.elapsed());
            found.extend(batch.unwrap_or_else(|e| panic!("{e}")));
        }
        (first_at, found)
    })
}

#[test]
fn matches_stream_with_their_full_path_and_the_ignore_list_holds() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    nested(dir.path(), 20);
    let root = loc(dir.path());
    let (first_at, found) = collect(&query(dir.path(), "FOO"));
    assert_eq!(found.len(), 20, "{found:?}");
    assert!(found.iter().all(|h| h.entry.name.starts_with("foo")));
    assert!(found.iter().all(|h| h.snippet.is_none()));
    assert!(
        found
            .iter()
            .all(|h| h.entry.path.as_bytes().starts_with(root.path.as_bytes()))
    );
    let deepest = found
        .iter()
        .find(|h| h.entry.name == "foo19.txt")
        .unwrap_or_else(|| panic!("deepest"));
    assert!(deepest.entry.path.to_string_lossy().contains("level19"));
    assert_eq!(deepest.entry.id.provider, PROVIDER_ID);
    assert!(
        first_at.is_some_and(|t| t < Duration::from_millis(100)),
        "first batch after {first_at:?}"
    );

    // Hidden on (no `NoHidden` filter), the dot-file joins; an empty text
    // matches nothing.
    let mut with_hidden = query(dir.path(), "foo");
    with_hidden.filters.retain(|f| *f != Filter::NoHidden);
    assert_eq!(collect(&with_hidden).1.len(), 21);
    let none = ext::runtime::block_on(async {
        WalkIndex
            .query(&query(dir.path(), ""), CancelToken::new())
            .next()
            .await
    });
    assert!(none.is_none());
}

#[test]
fn a_cancelled_search_ends_its_stream_and_a_missing_root_is_not_found() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    nested(dir.path(), 5);
    let cancel = CancelToken::new();
    cancel.cancel();
    let next = ext::runtime::block_on(async {
        WalkIndex
            .query(&query(dir.path(), "foo"), cancel)
            .next()
            .await
    });
    assert!(next.is_none());
    let missing = query(Path::new("/definitely/not/here"), "x");
    assert!(matches!(
        ext::runtime::block_on(async {
            WalkIndex.query(&missing, CancelToken::new()).next().await
        }),
        Some(Err(Error::NotFound))
    ));
}

#[test]
fn a_query_without_a_root_elsewhere_or_for_contents_is_refused() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let first = |q: &Query| {
        ext::runtime::block_on(async { WalkIndex.query(q, CancelToken::new()).next().await })
    };
    let mut no_root = query(dir.path(), "x");
    no_root.root = None;
    assert!(matches!(first(&no_root), Some(Err(Error::Unsupported(_)))));
    let mut elsewhere = query(dir.path(), "x");
    elsewhere.root = Some(Location::new(ProviderId("archive"), "/a.zip"));
    assert!(matches!(
        first(&elsewhere),
        Some(Err(Error::Unsupported(_)))
    ));
    let mut contents = query(dir.path(), "x");
    contents.kind = QueryKind::Content;
    assert!(matches!(first(&contents), Some(Err(Error::Unsupported(_)))));
}

#[test]
fn the_other_filters_narrow_what_the_walk_reports() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("a-small.txt"), b"x").unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("a-large.txt"), vec![0; 4096]).unwrap_or_else(|e| panic!("{e}"));
    fs::write(dir.path().join("a-other.md"), b"xx").unwrap_or_else(|e| panic!("{e}"));
    let names = |filters: Vec<Filter>| {
        let mut q = query(dir.path(), "a-");
        q.filters = filters;
        let mut names: Vec<String> = collect(&q).1.into_iter().map(|h| h.entry.name).collect();
        names.sort();
        names
    };
    assert_eq!(
        names(Vec::new()),
        ["a-large.txt", "a-other.md", "a-small.txt"]
    );
    assert_eq!(
        names(vec![Filter::Extension("TXT".into())]),
        ["a-large.txt", "a-small.txt"]
    );
    assert_eq!(names(vec![Filter::MinSize(100)]), ["a-large.txt"]);
    assert_eq!(
        names(vec![Filter::MaxSize(2)]),
        ["a-other.md", "a-small.txt"]
    );
    assert_eq!(
        names(vec![Filter::ModifiedAfter(SystemTime::UNIX_EPOCH)]).len(),
        3
    );
    assert_eq!(
        names(vec![Filter::ModifiedBefore(SystemTime::UNIX_EPOCH)]),
        [] as [String; 0]
    );
    assert_eq!(
        names(vec![
            Filter::Extension("txt".into()),
            Filter::MinSize(100),
            Filter::MaxSize(2)
        ]),
        [] as [&str; 0],
        "all filters must hold"
    );
}

#[test]
fn the_extension_registers_one_index_scoped_under_the_local_root() {
    let mut registry = Registry::new();
    registry
        .load(&WalkExtension::new())
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(registry.extensions(), &[ID]);
    assert_eq!(registry.points(), ext::Points::SEARCH_INDEX);
    let index = &registry.search_indexes()[0];
    assert_eq!(index.id(), ID);
    assert_eq!(index.availability(), ext::Availability::Enabled);
    let scope = index.scope();
    assert_eq!(
        scope,
        SearchScope::Under(Location::new(PROVIDER_ID, types::VPath::new()))
    );
    // The scope covers any local location and no other provider's.
    assert!(scope.covers(&Location::new(PROVIDER_ID, "/home/me")));
    assert!(scope.covers(&Location::new(PROVIDER_ID, "C:\\Users\\me")));
    assert!(!scope.covers(&Location::new(ProviderId("archive"), "/a.zip")));
    assert!(SearchScope::Everywhere.covers(&Location::new(ProviderId("archive"), "/a.zip")));
    let under_home = SearchScope::Under(Location::new(PROVIDER_ID, "/home/me"));
    assert!(under_home.covers(&Location::new(PROVIDER_ID, "/home/me/docs")));
    assert!(!under_home.covers(&Location::new(PROVIDER_ID, "/home/meow")));
    assert!(!under_home.covers(&Location::new(PROVIDER_ID, "/tmp")));
}
