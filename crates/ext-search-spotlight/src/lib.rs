//! The Spotlight search index connector: the macOS metadata index, queried through `mdfind`.
//!
//! `mdutil -s /` is the probe: the index is available while indexing is
//! enabled on the root volume. A query runs `mdfind`, with `-name` for a
//! name query, the text as a plain query for names and contents together
//! and a `kMDItemTextContent` query for contents alone, under the root
//! with `-onlyin`, and reads the absolute paths it prints (QUESTIONS.md
//! 120).
//!
//! The crate has one shape with the other three connectors (issue 22): a
//! [`Service`] runs the platform's tool, the real one through a child
//! process and the tests' [`Fake`](service::Fake) from canned output; a
//! probe asks the service whether the index is there at start and every
//! [`PROBE_EVERY`] after, so `availability` follows the service as it
//! appears and disappears; and `query` turns a [`Query`] into one command,
//! streams the paths the tool prints as the tool prints them, looks each up
//! on disk for its entry and applies the query's root and filters to what
//! the tool could not (QUESTIONS.md 120).

// `deny`, not `forbid`: the entry point's `#[no_mangle]` carries its own
// allowance (QUESTIONS.md 82).
#![deny(unsafe_code)]

pub mod backend;
pub mod hits;
pub mod service;

use ext::t;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use ext::{
    API_VERSION, Availability, BoxStream, Capabilities, Extension, Manifest, Platforms, Points,
    Query, Registry, SearchHit, SearchIndex, SearchScope, Version,
};
use futures::StreamExt;
use futures::stream;
use types::{CancelToken, Error, ExtId};

pub use service::{Command, Process, Service};

/// The extension id.
pub const ID: ExtId = ExtId("search-spotlight");

/// How often the service is asked whether it is still there, so the
/// title-row search box follows the index as it appears or disappears.
pub const PROBE_EVERY: Duration = Duration::from_secs(10);

/// The most hits one query reports: enough for a dropdown, and a bound on
/// a tool that would otherwise list the whole machine.
pub const MAX_HITS: usize = 500;

/// The extension: registers the one index over the real service.
#[derive(Debug)]
pub struct SpotlightExtension {
    manifest: Manifest,
}

impl Default for SpotlightExtension {
    fn default() -> Self {
        Self::new()
    }
}

impl SpotlightExtension {
    /// The extension, with the crate's version in its manifest.
    #[must_use]
    pub fn new() -> Self {
        Self {
            manifest: Manifest {
                id: ID,
                name: "Spotlight",
                version: Version::parse(env!("CARGO_PKG_VERSION"))
                    .unwrap_or_else(|_| Version::new(0, 0, 0)),
                api_version: API_VERSION,
                requires: Capabilities::empty(),
                points: Points::SEARCH_INDEX,
                platforms: Platforms::MACOS,
                previews: &[],
                commands: &[],
            },
        }
    }
}

impl Extension for SpotlightExtension {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        reg.translations(&[("en", include_str!("../locales/en.ftl"))]);
        reg.search_index(Arc::new(SpotlightIndex::new(Arc::new(Process))));
        Ok(())
    }
}

ext::plugin!(|| Arc::new(SpotlightExtension::new()) as Arc<dyn Extension>);

/// What the probe found out, shared with the loop that keeps asking.
#[derive(Debug, Default)]
pub struct State {
    /// The service answered the last probe.
    available: AtomicBool,
    /// The probe ran at least once.
    probed: AtomicBool,
    /// What the probe learnt about the tools, for the query command.
    tools: std::sync::Mutex<backend::Tools>,
}

impl State {
    /// Whether the service was there at the last probe.
    #[must_use]
    pub fn available(&self) -> bool {
        self.available.load(Ordering::SeqCst)
    }

    /// Whether a probe has answered yet.
    #[must_use]
    pub fn probed(&self) -> bool {
        self.probed.load(Ordering::SeqCst)
    }

    /// What the probe learnt about the tools.
    #[must_use]
    pub fn tools(&self) -> backend::Tools {
        self.tools
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn record(&self, probe: Option<backend::Tools>) {
        let available = probe.is_some();
        if let Some(tools) = probe {
            *self
                .tools
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = tools;
        }
        self.available.store(available, Ordering::SeqCst);
        self.probed.store(true, Ordering::SeqCst);
    }
}

/// Spotlight as a search index.
#[derive(Clone)]
pub struct SpotlightIndex {
    service: Arc<dyn Service>,
    state: Arc<State>,
}

impl std::fmt::Debug for SpotlightIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpotlightIndex")
            .field("available", &self.state.available())
            .finish_non_exhaustive()
    }
}

impl SpotlightIndex {
    /// An index over `service`, probing it now and every [`PROBE_EVERY`]
    /// for as long as the index lives.
    #[must_use]
    pub fn new(service: Arc<dyn Service>) -> Self {
        let state = Arc::new(State::default());
        ext::runtime::handle().spawn(probe_loop(Arc::downgrade(&state), Arc::clone(&service)));
        Self { service, state }
    }

    /// What the probe found out.
    #[must_use]
    pub fn state(&self) -> &Arc<State> {
        &self.state
    }

    /// Asks the service whether it is there, now, and records the answer;
    /// the loop does the same every [`PROBE_EVERY`].
    pub async fn probe(&self) -> bool {
        probe_once(&self.state, &self.service).await
    }
}

/// One probe: the backend's commands run on the blocking pool.
async fn probe_once(state: &State, service: &Arc<dyn Service>) -> bool {
    let service = Arc::clone(service);
    let found = ext::blocking(move || Ok(backend::probe(&*service)))
        .await
        .unwrap_or_default();
    state.record(found);
    state.available()
}

/// Probes until the index is dropped.
async fn probe_loop(state: Weak<State>, service: Arc<dyn Service>) {
    loop {
        let Some(state) = state.upgrade() else {
            return;
        };
        let was = state.available();
        let now = probe_once(&state, &service).await;
        if was != now {
            tracing::info!(
                available = now,
                "Spotlight index {}",
                if now { "appeared" } else { "disappeared" }
            );
        }
        drop(state);
        tokio::time::sleep(PROBE_EVERY).await;
    }
}

impl SearchIndex for SpotlightIndex {
    fn id(&self) -> ExtId {
        ID
    }

    fn availability(&self) -> Availability {
        if self.state.available() {
            Availability::Enabled
        } else if self.state.probed() {
            Availability::Disabled(t!("search-spotlight-spotlight-is-not-running"))
        } else {
            Availability::Disabled(t!("search-spotlight-spotlight-has-not-answered-yet"))
        }
    }

    fn scope(&self) -> SearchScope {
        SearchScope::Everywhere
    }

    fn query(
        &self,
        q: &Query,
        cancel: CancelToken,
    ) -> BoxStream<'static, Result<Vec<SearchHit>, Error>> {
        let command = match backend::query_command(q, &self.state.tools()) {
            Ok(command) => command,
            Err(error) => return stream::once(async move { Err(error) }).boxed(),
        };
        tracing::debug!(?command, "Spotlight query");
        let lines = self.service.stream(command, cancel);
        let query = q.clone();
        // Each batch of lines becomes entries on the blocking pool, since
        // every path is looked up on disk; the count stops the tool at
        // `MAX_HITS`.
        let hits = lines
            .then(move |batch| {
                let query = query.clone();
                async move {
                    match batch {
                        Ok(lines) => {
                            ext::blocking(move || {
                                Ok(hits::for_lines(&lines, &query, backend::path_of_line))
                            })
                            .await
                        }
                        Err(error) => Err(error),
                    }
                }
            })
            .filter(|batch| futures::future::ready(!matches!(batch, Ok(hits) if hits.is_empty())));
        stream::unfold((Box::pin(hits), 0usize), |(mut hits, count)| async move {
            if count >= MAX_HITS {
                return None;
            }
            let mut batch = hits.next().await?;
            if let Ok(entries) = &mut batch {
                entries.truncate(MAX_HITS - count);
            }
            let count = count + batch.as_ref().map_or(0, Vec::len);
            Some((batch, (hits, count)))
        })
        .boxed()
    }
}

#[cfg(test)]
mod tests;
