#[cfg(unix)]
use std::fs;
use std::path::Path;
use std::sync::Arc;

use ext::{Availability, Filter, Query, QueryKind, Registry, SearchHit, SearchIndex, SearchScope};
use futures::StreamExt;
use types::{CancelToken, Error, Location};

use super::service::{Command, Fake, Process, Service};
use super::*;

/// A name query, with a root when `root` is given.
fn query(text: &str, root: Option<&Path>) -> Query {
    Query {
        text: text.into(),
        root: root.map(|r| Location::new(hits::PROVIDER_ID, ext::platform::paths::from_path(r))),
        kind: QueryKind::Name,
        filters: Vec::new(),
    }
}

/// Every hit the index streams for `q`, or the first error.
fn collect(index: &TrackerIndex, q: &Query) -> Result<Vec<SearchHit>, Error> {
    ext::runtime::block_on(async {
        let mut stream = index.query(q, CancelToken::new());
        let mut found = Vec::new();
        while let Some(batch) = stream.next().await {
            found.extend(batch?);
        }
        Ok(found)
    })
}

/// The last command that was a query, not a probe: the probe loop runs
/// beside the tests and records its own commands whenever it fires.
fn last_query(fake: &Fake) -> Command {
    fake.commands()
        .into_iter()
        .rev()
        .find(|c| !backend::tests::is_probe(c))
        .unwrap_or_else(|| panic!("no query command"))
}

/// An index over a fake that reports the service present.
fn present() -> (Arc<Fake>, TrackerIndex) {
    let fake = Fake::new();
    backend::tests::make_present(&fake);
    let index = TrackerIndex::new(Arc::clone(&fake) as Arc<dyn Service>);
    assert!(ext::runtime::block_on(index.probe()));
    (fake, index)
}

#[test]
fn the_probe_decides_availability_and_follows_the_service() {
    let fake = Fake::new();
    let index = TrackerIndex::new(Arc::clone(&fake) as Arc<dyn Service>);
    // Nothing answers: not available, and said so once the probe ran.
    assert!(!ext::runtime::block_on(index.probe()));
    assert!(matches!(index.availability(), Availability::Disabled(_)));
    assert!(index.state().probed());
    backend::tests::make_present(&fake);
    assert!(ext::runtime::block_on(index.probe()));
    assert_eq!(index.availability(), Availability::Enabled);
    backend::tests::make_absent(&fake);
    assert!(!ext::runtime::block_on(index.probe()));
    assert!(matches!(index.availability(), Availability::Disabled(_)));
    assert_eq!(index.scope(), SearchScope::Everywhere);
    assert_eq!(index.id(), ID);
}

// The tool prints `/`-rooted paths, which a Windows path cannot stand in
// for; the connector never runs there.
#[cfg(unix)]
#[test]
fn a_query_runs_the_tool_and_streams_the_hits_that_are_on_disk() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let root = dir.path();
    fs::create_dir_all(root.join("sub/.hidden")).unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir_all(root.join("node_modules")).unwrap_or_else(|e| panic!("{e}"));
    for name in [
        "foo.txt",
        "sub/foo-deep.md",
        "sub/.hidden/foo-hidden.txt",
        "node_modules/foo-dep.js",
    ] {
        fs::write(root.join(name), b"hello").unwrap_or_else(|e| panic!("{e}"));
    }
    let (fake, index) = present();
    let paths = [
        root.join("foo.txt"),
        root.join("sub/foo-deep.md"),
        root.join("sub/.hidden/foo-hidden.txt"),
        root.join("node_modules/foo-dep.js"),
        root.join("gone/foo-gone.txt"),
    ];
    fake.prints(&backend::tests::output_for(&paths));

    let hits = collect(&index, &query("foo", None)).unwrap_or_else(|e| panic!("{e}"));
    let mut names: Vec<&str> = hits.iter().map(|h| h.entry.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        ["foo-deep.md", "foo-dep.js", "foo-hidden.txt", "foo.txt"],
        "every path on disk, the gone one skipped"
    );
    let deep = hits
        .iter()
        .find(|h| h.entry.name == "foo-deep.md")
        .unwrap_or_else(|| panic!("deep"));
    assert_eq!(
        deep.entry.path,
        ext::platform::paths::from_path(&root.join("sub/foo-deep.md"))
    );
    assert_eq!(deep.entry.id.provider, hits::PROVIDER_ID);
    assert_eq!(deep.entry.size, Some(5));
    assert!(hits.iter().all(|h| h.snippet.is_none()));

    // The command the tool was run with, as the backend builds it.
    backend::tests::check_name_command(&last_query(&fake), "foo", None);

    // A root narrows the hits to its subtree; the backend passes it on.
    fake.prints(&backend::tests::output_for(&paths));
    let under =
        collect(&index, &query("foo", Some(&root.join("sub")))).unwrap_or_else(|e| panic!("{e}"));
    let mut names: Vec<&str> = under.iter().map(|h| h.entry.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["foo-deep.md", "foo-hidden.txt"]);
    backend::tests::check_name_command(&last_query(&fake), "foo", Some(&root.join("sub")));

    // The filters narrow what the tool found; the hidden and skipped
    // names count below the root (the temporary directory is a dot-dir).
    fake.prints(&backend::tests::output_for(&paths));
    let mut filtered = query("foo", Some(root));
    filtered.filters = vec![
        Filter::NoHidden,
        Filter::SkipNames(vec!["node_modules".into()]),
    ];
    let hits = collect(&index, &filtered).unwrap_or_else(|e| panic!("{e}"));
    let mut names: Vec<&str> = hits.iter().map(|h| h.entry.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["foo-deep.md", "foo.txt"]);
    fake.prints(&backend::tests::output_for(&paths));
    let mut by_ext = query("foo", Some(root));
    by_ext.filters = vec![Filter::Extension("MD".into()), Filter::MinSize(1)];
    let hits = collect(&index, &by_ext).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].entry.name, "foo-deep.md");

    // Content and both: the backend's other two shapes.
    for kind in [QueryKind::Content, QueryKind::Both] {
        fake.prints(&[]);
        let mut q = query("foo bar", None);
        q.kind = kind;
        assert_eq!(
            collect(&index, &q).unwrap_or_else(|e| panic!("{e}")),
            Vec::<SearchHit>::new()
        );
        backend::tests::check_kind_command(&last_query(&fake), "foo bar", kind);
    }
}

#[test]
fn a_failing_tool_yields_one_error_and_noise_lines_are_ignored() {
    let (fake, index) = present();
    fake.fails("the service went away");
    let error = collect(&index, &query("foo", None))
        .err()
        .unwrap_or_else(|| panic!("no error"));
    assert!(
        matches!(&error, Error::Extension { ext, .. } if *ext == ID),
        "{error}"
    );
    assert!(
        error.to_string().contains("the service went away"),
        "{error}"
    );

    // What the tool prints besides paths is not a hit.
    fake.prints(&backend::tests::noise());
    assert_eq!(
        collect(&index, &query("foo", None)).unwrap_or_else(|e| panic!("{e}")),
        Vec::<SearchHit>::new()
    );

    // An empty text is still a command, answered by the tool.
    fake.prints(&[]);
    assert_eq!(
        collect(&index, &query("", None)).unwrap_or_else(|e| panic!("{e}")),
        Vec::<SearchHit>::new()
    );
}

#[cfg(unix)]
#[test]
fn the_hits_stop_at_the_cap() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let paths: Vec<std::path::PathBuf> = (0..MAX_HITS + 20)
        .map(|i| {
            let path = dir.path().join(format!("foo-{i:04}.txt"));
            fs::write(&path, b"x").unwrap_or_else(|e| panic!("{e}"));
            path
        })
        .collect();
    let (fake, index) = present();
    fake.prints(&backend::tests::output_for(&paths));
    let hits = collect(&index, &query("foo", None)).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(hits.len(), MAX_HITS);
}

#[test]
fn the_process_service_runs_a_real_command_and_streams_its_lines() {
    // The test binary itself lists its tests, one per line, on every OS.
    let exe = std::env::current_exe().unwrap_or_else(|e| panic!("{e}"));
    let exe = exe.to_string_lossy().into_owned();
    let command = Command::new(&exe, &["--list"]);
    let output = Process.output(&command).unwrap_or_else(|e| panic!("{e}"));
    assert!(
        output.contains("the_process_service_runs_a_real_command"),
        "{output}"
    );
    let lines: Vec<String> = ext::runtime::block_on(async {
        let mut stream = Process.stream(command, CancelToken::new());
        let mut lines = Vec::new();
        while let Some(batch) = stream.next().await {
            lines.extend(batch.unwrap_or_else(|e| panic!("{e}")));
        }
        lines
    });
    assert!(
        lines
            .iter()
            .any(|l| l.contains("the_process_service_runs_a_real_command")),
        "{lines:?}"
    );

    // A program that is not there: `output` says so, `stream` yields the
    // one error.
    let missing = Command::new("rick-no-such-tool-here", &[]);
    assert!(Process.output(&missing).is_err());
    let first =
        ext::runtime::block_on(async { Process.stream(missing, CancelToken::new()).next().await });
    assert!(
        matches!(first, Some(Err(Error::Extension { .. }))),
        "{first:?}"
    );

    // A cancelled stream ends.
    let cancel = CancelToken::new();
    cancel.cancel();
    let none = ext::runtime::block_on(async {
        Process
            .stream(Command::new(&exe, &["--list"]), cancel)
            .next()
            .await
    });
    assert!(none.is_none());
}

#[test]
fn the_extension_registers_one_index_for_its_platforms() {
    let mut registry = Registry::new();
    registry
        .load(&TrackerExtension::new())
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(registry.extensions(), &[ID]);
    assert_eq!(registry.points(), ext::Points::SEARCH_INDEX);
    assert_eq!(registry.manifests()[0].platforms, Platforms::LINUX);
    assert_eq!(registry.manifests()[0].name, "Tracker");
    let index = &registry.search_indexes()[0];
    assert_eq!(index.scope(), SearchScope::Everywhere);
    assert_eq!(index.id(), ID);
}
