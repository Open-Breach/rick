//! The Tracker 3 search index connector: the GNOME file index, queried with SPARQL over D-Bus.
//!
//! The miner (`org.freedesktop.Tracker3.Miner.Files`) is a SPARQL endpoint
//! on the session bus; the `tracker3` command line is its D-Bus client,
//! so the connector sends its SPARQL through `tracker3 sparql
//! --dbus-service` and reads the `file://` URIs that come back
//! (QUESTIONS.md 120). The probe asks the bus whether the miner owns its
//! name, through `gdbus` or `busctl`, so a Tracker that is installed but
//! not running stays out of the box rather than being started by the first
//! query.
//!
//! The indexer has two names. Since 2024 GNOME calls it `LocalSearch`: its
//! command line is `localsearch`, its endpoint
//! `org.freedesktop.LocalSearch3`, and the SPARQL goes through `tinysparql
//! query`, the command line of the library that was Tracker's; older
//! distributions and Ubuntu have `tracker3`. The connector looks for both
//! at start ([`backend::installed`]) and queries through the one it finds.
//! With neither it disables itself quietly: `availability` gives the
//! reason, the host is told through [`ext::attention`], which the
//! Extensions dialogue shows, and nothing is ever run or shown in the
//! search box (issue 149).
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
pub const ID: ExtId = ExtId("search-tracker");

/// How often the service is asked whether it is still there, so the
/// title-row search box follows the index as it appears or disappears.
pub const PROBE_EVERY: Duration = Duration::from_secs(10);

/// The most hits one query reports: enough for a dropdown, and a bound on
/// a tool that would otherwise list the whole machine.
pub const MAX_HITS: usize = 500;

/// The extension: registers the one index over the real service.
#[derive(Debug)]
pub struct TrackerExtension {
    manifest: Manifest,
}

impl Default for TrackerExtension {
    fn default() -> Self {
        Self::new()
    }
}

impl TrackerExtension {
    /// The extension, with the crate's version in its manifest.
    #[must_use]
    pub fn new() -> Self {
        Self {
            manifest: Manifest {
                id: ID,
                name: "Tracker",
                description: "Searches the GNOME file index (Tracker 3) from the search box.",
                version: Version::parse(env!("CARGO_PKG_VERSION"))
                    .unwrap_or_else(|_| Version::new(0, 0, 0)),
                api_version: API_VERSION,
                requires: Capabilities::empty(),
                points: Points::SEARCH_INDEX,
                platforms: Platforms::LINUX,
                previews: &[],
                commands: &[],
            },
        }
    }
}

impl Extension for TrackerExtension {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        reg.translations(&[("en", include_str!("../locales/en.ftl"))]);
        reg.search_index(Arc::new(TrackerIndex::new(Arc::new(Process))));
        Ok(())
    }
}

ext::plugin!(|| Arc::new(TrackerExtension::new()) as Arc<dyn Extension>);

/// What the probe found out, shared with the loop that keeps asking.
#[derive(Debug, Default)]
pub struct State {
    /// The service answered the last probe.
    available: AtomicBool,
    /// The probe ran at least once.
    probed: AtomicBool,
    /// What the probe learnt about the tools, for the query command.
    tools: std::sync::Mutex<backend::Tools>,
    /// Which of the indexer's command lines is installed, looked up once:
    /// `Some(None)` is neither.
    installed: std::sync::OnceLock<Option<backend::Tools>>,
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

    /// Whether the indexer is not on this machine at all: neither
    /// `localsearch` nor `tracker3` was found, and the connector has
    /// disabled itself.
    #[must_use]
    pub fn missing(&self) -> bool {
        self.installed.get() == Some(&None)
    }

    /// What the probe learnt about the tools.
    #[must_use]
    pub fn tools(&self) -> backend::Tools {
        *self
            .tools
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
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

/// Tracker as a search index.
#[derive(Clone)]
pub struct TrackerIndex {
    service: Arc<dyn Service>,
    state: Arc<State>,
}

impl std::fmt::Debug for TrackerIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrackerIndex")
            .field("available", &self.state.available())
            .finish_non_exhaustive()
    }
}

impl TrackerIndex {
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

/// One probe: the backend's commands run on the blocking pool. The first
/// one looks for the indexer's command line, under both of its names;
/// with neither the connector disables itself and tells the host why,
/// quietly: the Extensions dialogue shows it, the search box nothing.
async fn probe_once(state: &State, service: &Arc<dyn Service>) -> bool {
    let service = Arc::clone(service);
    let known = state.installed.get().copied();
    let answer = ext::blocking(move || {
        let installed = known.unwrap_or_else(|| backend::installed(&*service));
        let found = installed.and_then(|tools| backend::probe(&*service, tools));
        Ok((installed, found))
    })
    .await;
    let Ok((installed, found)) = answer else {
        state.record(None);
        return false;
    };
    if state.installed.set(installed).is_ok() {
        if let Some(tools) = installed {
            tracing::info!(?tools, "the indexer's command line");
            ext::attention::clear(ID);
        } else {
            tracing::info!("neither localsearch nor tracker3 is installed");
            ext::attention::report(ID, t!("search-tracker-neither-found"));
        }
    }
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
                "Tracker index {}",
                if now { "appeared" } else { "disappeared" }
            );
        }
        drop(state);
        tokio::time::sleep(PROBE_EVERY).await;
    }
}

impl SearchIndex for TrackerIndex {
    fn id(&self) -> ExtId {
        ID
    }

    fn availability(&self) -> Availability {
        if self.state.available() {
            Availability::Enabled
        } else if self.state.missing() {
            Availability::Disabled(t!("search-tracker-neither-found"))
        } else if self.state.probed() {
            Availability::Disabled(t!("search-tracker-tracker-is-not-running"))
        } else {
            Availability::Disabled(t!("search-tracker-tracker-has-not-answered-yet"))
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
        tracing::debug!(?command, "Tracker query");
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
