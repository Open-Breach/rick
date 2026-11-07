//! Search in subdirectories: the `ignore` crate's parallel walker over a
//! local folder, streaming the entries whose name contains the text.
//!
//! This is the kernel-side walk the spec's Main area describes; the
//! `ext-search-walk` plugin of 0.2 may wrap it. Matches arrive in batches as
//! the walk proceeds, so the first screen shows before the walk finishes;
//! cancelling the token ends the walk within one directory per thread.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ext::EntryStream;
use futures::StreamExt;
use futures::stream;
use ignore::{WalkBuilder, WalkState};
use tokio::sync::mpsc;
use types::{CancelToken, Entry, Error, Location, VPath};

use crate::{entry_at, path_of};

/// What a subtree search looks for and skips.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SearchOptions {
    /// The name must contain this, case-insensitively. Empty matches nothing.
    pub text: String,
    /// Descend into and report hidden entries.
    pub hidden: bool,
    /// Directory and file names never descended into or reported
    /// (`.git`, `node_modules`, `target`).
    pub ignore: Vec<String>,
}

/// Matches per batch before a thread hands them over. Each batch is merged
/// into the list's sorted view, so they are kept large; the flush below
/// keeps the first screen prompt when matches are sparse.
const BATCH: usize = 512;
/// A thread hands over what it has after this long, so sparse matches still
/// reach the list promptly.
const FLUSH_AFTER: Duration = Duration::from_millis(40);
/// Batches buffered between the walker threads and a slow consumer.
const CHANNEL_DEPTH: usize = 16;

/// Streams the entries under `loc` whose name contains `opts.text`, in the
/// order the walk finds them, each with its full path.
#[must_use]
pub fn search(loc: &Location, opts: SearchOptions, cancel: CancelToken) -> EntryStream {
    let root = match path_of(loc) {
        Ok(root) => root,
        Err(e) => return stream::once(async move { Err(e) }).boxed(),
    };
    if opts.text.is_empty() {
        return stream::empty().boxed();
    }
    let root_vpath = loc.path.clone();
    let (tx, rx) = mpsc::channel::<Result<Vec<Entry>, Error>>(CHANNEL_DEPTH);
    let walker_cancel = cancel.clone();
    runtime::handle().spawn_blocking(move || {
        let span =
            tracing::info_span!("vfs_local.search", path = %root.display(), text = %opts.text);
        let _enter = span.enter();
        walk(&root, &root_vpath, &opts, &walker_cancel, &tx);
    });
    stream::unfold((rx, cancel), |(mut rx, cancel)| async move {
        if cancel.is_cancelled() {
            return None;
        }
        rx.recv().await.map(|item| (item, (rx, cancel)))
    })
    .boxed()
}

/// `haystack` contains `needle` ignoring case.
fn contains_ignore_case(haystack: &str, needle: &str) -> bool {
    if needle.is_ascii() && haystack.is_ascii() {
        let needle = needle.as_bytes();
        return haystack
            .as_bytes()
            .windows(needle.len())
            .any(|window| window.eq_ignore_ascii_case(needle));
    }
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

/// One walker thread's pending matches.
struct Collector {
    root: PathBuf,
    root_vpath: VPath,
    text: String,
    tx: mpsc::Sender<Result<Vec<Entry>, Error>>,
    cancel: CancelToken,
    batch: Vec<Entry>,
    last_flush: Instant,
}

impl Collector {
    fn flush(&mut self) -> bool {
        self.last_flush = Instant::now();
        if self.batch.is_empty() {
            return true;
        }
        let batch = std::mem::take(&mut self.batch);
        self.tx.blocking_send(Ok(batch)).is_ok()
    }

    fn visit(&mut self, entry: Result<ignore::DirEntry, ignore::Error>) -> WalkState {
        if self.cancel.is_cancelled() {
            return WalkState::Quit;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                tracing::debug!(%error, "skipped while searching");
                return WalkState::Continue;
            }
        };
        if entry.depth() == 0 {
            return WalkState::Continue;
        }
        let name = entry.file_name().to_string_lossy();
        if contains_ignore_case(&name, &self.text) {
            let relative = entry
                .path()
                .strip_prefix(&self.root)
                .unwrap_or(entry.path());
            let vpath = self
                .root_vpath
                .join(relative.as_os_str().as_encoded_bytes());
            if let Ok(found) = entry_at(entry.path(), &vpath) {
                self.batch.push(found);
            }
        }
        if (self.batch.len() >= BATCH || self.last_flush.elapsed() >= FLUSH_AFTER) && !self.flush()
        {
            return WalkState::Quit;
        }
        WalkState::Continue
    }
}

impl Drop for Collector {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

fn walk(
    root: &Path,
    root_vpath: &VPath,
    opts: &SearchOptions,
    cancel: &CancelToken,
    tx: &mpsc::Sender<Result<Vec<Entry>, Error>>,
) {
    if let Err(e) = std::fs::symlink_metadata(root) {
        let _ = tx.blocking_send(Err(Error::from_io(e)));
        return;
    }
    let ignore: Vec<String> = opts.ignore.clone();
    let threads = std::thread::available_parallelism().map_or(2, |n| n.get().min(8));
    let mut builder = WalkBuilder::new(root);
    builder
        .standard_filters(false)
        .hidden(!opts.hidden)
        .follow_links(false)
        .threads(threads)
        .filter_entry(move |entry| {
            entry.depth() == 0
                || !ignore
                    .iter()
                    .any(|skip| entry.file_name().to_string_lossy() == skip.as_str())
        });
    builder.build_parallel().run(|| {
        let mut collector = Collector {
            root: root.to_path_buf(),
            root_vpath: root_vpath.clone(),
            text: opts.text.clone(),
            tx: tx.clone(),
            cancel: cancel.clone(),
            batch: Vec::with_capacity(BATCH),
            last_flush: Instant::now(),
        };
        Box::new(move |entry| collector.visit(entry))
    });
    tracing::debug!("search walk finished");
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::Instant;

    use super::*;
    use crate::PROVIDER_ID;

    fn nested(root: &std::path::Path, levels: usize) {
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

    #[test]
    fn matches_stream_with_their_full_path_and_the_ignore_list_holds() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        nested(dir.path(), 20);
        let loc = Location::new(PROVIDER_ID, platform::paths::from_path(dir.path()));
        let opts = SearchOptions {
            text: "FOO".into(),
            hidden: false,
            ignore: vec!["node_modules".into()],
        };
        let started = Instant::now();
        let (first_at, found) = runtime::block_on(async {
            let mut stream = search(&loc, opts, CancelToken::new());
            let mut first_at = None;
            let mut found = Vec::new();
            while let Some(batch) = stream.next().await {
                first_at.get_or_insert_with(|| started.elapsed());
                found.extend(batch.unwrap_or_else(|e| panic!("{e}")));
            }
            (first_at, found)
        });
        assert_eq!(found.len(), 20, "{found:?}");
        assert!(found.iter().all(|e| e.name.starts_with("foo")));
        assert!(
            found
                .iter()
                .all(|e| e.path.as_bytes().starts_with(loc.path.as_bytes()))
        );
        let deepest = found
            .iter()
            .find(|e| e.name == "foo19.txt")
            .unwrap_or_else(|| panic!("deepest"));
        assert!(deepest.path.to_string_lossy().contains("level19"));
        assert!(
            first_at.is_some_and(|t| t < Duration::from_millis(100)),
            "first batch after {first_at:?}"
        );

        // Hidden on, the dot-file joins; an empty text matches nothing.
        let with_hidden = runtime::block_on(async {
            let mut stream = search(
                &loc,
                SearchOptions {
                    text: "foo".into(),
                    hidden: true,
                    ignore: vec!["node_modules".into()],
                },
                CancelToken::new(),
            );
            let mut n = 0;
            while let Some(batch) = stream.next().await {
                n += batch.map_or(0, |b| b.len());
            }
            n
        });
        assert_eq!(with_hidden, 21);
        let none = runtime::block_on(async {
            search(&loc, SearchOptions::default(), CancelToken::new())
                .next()
                .await
        });
        assert!(none.is_none());
    }

    #[test]
    fn a_cancelled_search_ends_its_stream() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        nested(dir.path(), 5);
        let loc = Location::new(PROVIDER_ID, platform::paths::from_path(dir.path()));
        let cancel = CancelToken::new();
        cancel.cancel();
        let next = runtime::block_on(async {
            search(
                &loc,
                SearchOptions {
                    text: "foo".into(),
                    ..SearchOptions::default()
                },
                cancel,
            )
            .next()
            .await
        });
        assert!(next.is_none());
        assert!(matches!(
            runtime::block_on(async {
                search(
                    &Location::new(PROVIDER_ID, "/definitely/not/here"),
                    SearchOptions {
                        text: "x".into(),
                        ..SearchOptions::default()
                    },
                    CancelToken::new(),
                )
                .next()
                .await
            }),
            Some(Err(Error::NotFound))
        ));
    }
}
