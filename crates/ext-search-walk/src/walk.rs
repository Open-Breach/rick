//! The walk itself: the `ignore` crate's parallel walker over the query's
//! root, streaming the entries whose name contains the text.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ext::platform::fs::{RawEntry, RawKind};
use ext::runtime::mpsc;
use ext::{BoxStream, Filter, Query, QueryKind, SearchHit};
use futures::StreamExt;
use futures::stream;
use ignore::{WalkBuilder, WalkState};
use types::{CancelToken, Entry, EntryId, EntryKind, Error, VPath};

use crate::PROVIDER_ID;

/// Matches per batch before a thread hands them over. Each batch is merged
/// into the list's sorted view, so they are kept large; the flush below
/// keeps the first screen prompt when matches are sparse.
const BATCH: usize = 512;
/// A thread hands over what it has after this long, so sparse matches still
/// reach the list promptly.
const FLUSH_AFTER: Duration = Duration::from_millis(40);
/// Batches buffered between the walker threads and a slow consumer.
const CHANNEL_DEPTH: usize = 16;

/// What the walk looks for and skips, read off a [`Query`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Options {
    /// The name must contain this, case-insensitively. Empty matches
    /// nothing.
    pub text: String,
    /// Descend into and report hidden entries.
    pub hidden: bool,
    /// Directory and file names never descended into or reported.
    pub skip: Vec<String>,
    /// The remaining filters, checked against each match.
    pub narrow: Vec<Filter>,
}

impl Options {
    /// The options a query asks for.
    #[must_use]
    pub fn from_query(q: &Query) -> Self {
        let mut options = Self {
            text: q.text.clone(),
            hidden: true,
            skip: Vec::new(),
            narrow: Vec::new(),
        };
        for filter in &q.filters {
            match filter {
                Filter::NoHidden => options.hidden = false,
                Filter::SkipNames(names) => options.skip.extend(names.iter().cloned()),
                other => options.narrow.push(other.clone()),
            }
        }
        options
    }

    /// Whether an entry passes the narrowing filters.
    fn admits(&self, entry: &Entry) -> bool {
        self.narrow.iter().all(|filter| match filter {
            Filter::Extension(ext) => Path::new(&entry.name)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case(OsStr::new(ext))),
            Filter::MinSize(min) => entry.size.is_some_and(|s| s >= *min),
            Filter::MaxSize(max) => entry.size.is_some_and(|s| s <= *max),
            Filter::ModifiedAfter(after) => entry.modified.is_some_and(|m| m > *after),
            Filter::ModifiedBefore(before) => entry.modified.is_some_and(|m| m < *before),
            Filter::NoHidden | Filter::SkipNames(_) => true,
        })
    }
}

/// Streams the entries under the query's root whose name contains its
/// text, in the order the walk finds them, each with its full path.
///
/// The stream carries one error and ends when the query has no root, the
/// root is not a local location or does not exist, or the query asks for
/// file contents, which a walk over names cannot answer.
#[must_use = "the walk starts, but its matches are lost unless the stream is polled"]
pub fn search(q: &Query, cancel: CancelToken) -> BoxStream<'static, Result<Vec<SearchHit>, Error>> {
    let Some(root_loc) = &q.root else {
        return failed(Error::Unsupported("the subdirectory search needs a root"));
    };
    if root_loc.provider != PROVIDER_ID {
        return failed(Error::Unsupported(
            "the subdirectory search walks the local filesystem only",
        ));
    }
    if q.kind != QueryKind::Name {
        return failed(Error::Unsupported(
            "the subdirectory search matches names, not contents",
        ));
    }
    let root = match ext::platform::paths::to_path(&root_loc.path) {
        Ok(root) => root,
        Err(e) => return failed(e),
    };
    let opts = Options::from_query(q);
    if opts.text.is_empty() {
        return stream::empty().boxed();
    }
    let root_vpath = root_loc.path.clone();
    let (tx, rx) = mpsc::channel::<Result<Vec<SearchHit>, Error>>(CHANNEL_DEPTH);
    let walker_cancel = cancel.clone();
    ext::runtime::handle().spawn_blocking(move || {
        let span = tracing::info_span!("search_walk", path = %root.display(), text = %opts.text);
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

fn failed(error: Error) -> BoxStream<'static, Result<Vec<SearchHit>, Error>> {
    stream::once(async move { Err(error) }).boxed()
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

/// The entry at `path`, named by the location path `vpath`: what the
/// local provider would list for it, read through the platform's `stat`.
///
/// # Errors
///
/// The I/O error from examining the path.
pub fn entry_at(path: &Path, vpath: &VPath) -> std::io::Result<Entry> {
    let RawEntry {
        name,
        id,
        kind,
        size,
        created,
        modified,
        accessed,
        attrs,
    } = ext::platform::fs::stat(path)?;
    let kind = match kind {
        RawKind::File => EntryKind::File,
        RawKind::Dir => EntryKind::Dir,
        RawKind::Other => EntryKind::Other,
        RawKind::Symlink => EntryKind::Symlink {
            target: std::fs::read_link(path)
                .map(|target| ext::platform::paths::from_path(&target))
                .unwrap_or_default(),
            broken: std::fs::metadata(path).is_err(),
        },
    };
    Ok(Entry {
        id: EntryId::new(PROVIDER_ID, id),
        name: name.to_string_lossy().into_owned(),
        path: vpath.clone(),
        kind,
        size,
        created,
        modified,
        accessed,
        attrs,
        mime: None,
    })
}

/// One walker thread's pending matches.
struct Collector {
    root: PathBuf,
    root_vpath: VPath,
    opts: Options,
    tx: mpsc::Sender<Result<Vec<SearchHit>, Error>>,
    cancel: CancelToken,
    batch: Vec<SearchHit>,
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
        if contains_ignore_case(&name, &self.opts.text) {
            let relative = entry
                .path()
                .strip_prefix(&self.root)
                .unwrap_or(entry.path());
            let vpath = self
                .root_vpath
                .join(relative.as_os_str().as_encoded_bytes());
            if let Ok(found) = entry_at(entry.path(), &vpath)
                && self.opts.admits(&found)
            {
                self.batch.push(SearchHit {
                    entry: found,
                    snippet: None,
                });
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
    opts: &Options,
    cancel: &CancelToken,
    tx: &mpsc::Sender<Result<Vec<SearchHit>, Error>>,
) {
    if let Err(e) = std::fs::symlink_metadata(root) {
        let _ = tx.blocking_send(Err(Error::from_io(e)));
        return;
    }
    let skip: Vec<String> = opts.skip.clone();
    let threads = std::thread::available_parallelism().map_or(2, |n| n.get().min(8));
    let mut builder = WalkBuilder::new(root);
    builder
        .standard_filters(false)
        .hidden(!opts.hidden)
        .follow_links(false)
        .threads(threads)
        .filter_entry(move |entry| {
            entry.depth() == 0
                || !skip
                    .iter()
                    .any(|name| entry.file_name().to_string_lossy() == name.as_str())
        });
    let started = Instant::now();
    builder.build_parallel().run(|| {
        let mut collector = Collector {
            root: root.to_path_buf(),
            root_vpath: root_vpath.clone(),
            opts: opts.clone(),
            tx: tx.clone(),
            cancel: cancel.clone(),
            batch: Vec::with_capacity(BATCH),
            last_flush: Instant::now(),
        };
        Box::new(move |entry| collector.visit(entry))
    });
    tracing::debug!(took = ?started.elapsed(), "search walk finished");
}
