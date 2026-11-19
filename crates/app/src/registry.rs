//! The plugin registry as the application keeps it
//! (specs/plugin-distribution.md §6).
//!
//! The client is the crate `plugins-registry`; this module decides when it
//! is asked. Three occasions reach it as a [`RegistryCommand`]: the start,
//! once the first frame is drawn, the opening of the Extensions dialogue's
//! *Available* view, and *Check now*. Each runs as a job off the loop's
//! thread and answers with messages to the loop, which folds them into
//! [`PluginRegistry`], what the dialogue is drawn from.
//!
//! - Without a public key the registry is off: nothing is read and nothing
//!   is fetched, whatever asks.
//! - The cached index is read once, at the first occasion, and verified as
//!   every read of it is.
//! - With *Look for plugins and updates online* on (`[registry] check`),
//!   the start fetches when the cached index is a day old or there is
//!   none, and the opening of the view fetches. With it off only *Check
//!   now* does.
//! - A fetch that fails leaves the last good index in place and says why;
//!   it is tried again at the next occasion, never in a loop.
//! - A Rick with a pre-release part in its version fits nothing the
//!   registry has, and asks for nothing.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use plugins_registry::{Cache, Cached, Index, Source, due, is_pre_release, refresh};
use types::JobId;

use crate::{App, Msg};

/// The folder of the config directory that is the cache of a Rick started
/// on a config directory of its own (`--config-dir`), and of every
/// application the binary did not name another one for.
pub const CACHE_DIR: &str = "cache";

/// An occasion to read the plugin list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryCommand {
    /// The first frame is drawn: read what is cached and, with the switch
    /// on, fetch when the last fetch is a day old.
    Start,
    /// The *Available* view was opened: fetch, with the switch on.
    Opened,
    /// *Check now*: fetch, whatever the switch says.
    CheckNow,
}

/// Why the last read of the cache or the last fetch gave no index. The
/// last good index, when there is one, is still in
/// [`PluginRegistry::index`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryProblem {
    /// The index did not arrive: no network, or the registry did not
    /// answer with it.
    Offline,
    /// The index's signature is not the registry's: *The plugin list
    /// could not be verified*.
    Unverified,
    /// The index is written to a schema above this Rick's: *This list
    /// needs a newer Rick*.
    NewerRick,
    /// The index verified and could not be read all the same, or the
    /// cache could not.
    Unreadable,
}

impl From<&plugins_registry::Error> for RegistryProblem {
    fn from(error: &plugins_registry::Error) -> Self {
        use plugins_registry::Error;
        match error {
            Error::Unverified => Self::Unverified,
            Error::NewerSchema(_) => Self::NewerRick,
            Error::Fetch(_) => Self::Offline,
            Error::NotSetUp | Error::Unreadable(_) | Error::Key(_) | Error::Io { .. } => {
                Self::Unreadable
            }
        }
    }
}

/// What the application knows of the plugin registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginRegistry {
    /// There is a public key to verify an index with. Without one the
    /// registry is off: *The plugin list is not set up in this build*.
    pub set_up: bool,
    /// The registry's host, the *Source* of an available plugin.
    pub host: String,
    /// The address of the index while `rick.toml` names another address
    /// or another key than this build's own; the dialogue's footer names
    /// it.
    pub custom: Option<String>,
    /// The version of this Rick, which a build must have been made for.
    pub rick: String,
    /// The platform of this Rick, named as a build names its own.
    pub platform: String,
    /// A fetch is running.
    pub checking: bool,
    /// The last index that verified, fetched in this session or read from
    /// the cache.
    pub index: Option<Arc<Index>>,
    /// When that index was fetched.
    pub fetched: Option<SystemTime>,
    /// Why the last read or fetch gave none; `None` after one that did.
    pub problem: Option<RegistryProblem>,
}

/// The registry's state and the work in flight.
#[derive(Debug)]
pub(crate) struct RegistryRun {
    /// What the UI reads.
    pub(crate) view: PluginRegistry,
    /// Rick's cache directory.
    pub(crate) cache_dir: PathBuf,
    /// The cache was read in this session.
    cache_read: bool,
    /// The running job; a second one is never started beside it.
    job: Option<JobId>,
    /// Where the index is and which key signs it, as `rick.toml` had it
    /// at the start: another address or key is taken up by the next start.
    source: Source,
}

impl RegistryRun {
    /// The state of a Rick that has asked nothing yet, for the source
    /// `settings` name and the cache in `cache_dir`.
    pub(crate) fn new(settings: &model::RegistrySettings, cache_dir: PathBuf) -> Self {
        let source = source_of(settings);
        Self {
            view: PluginRegistry {
                set_up: source.key.is_some(),
                host: source.host(),
                custom: source.custom.then(|| source.url.clone()),
                rick: plugins_registry::RICK.to_owned(),
                platform: plugins_registry::platform(),
                checking: false,
                index: None,
                fetched: None,
                problem: None,
            },
            cache_dir,
            cache_read: false,
            job: None,
            source,
        }
    }
}

/// The source `rick.toml` names, or this build's own.
fn source_of(settings: &model::RegistrySettings) -> Source {
    Source::new(settings.url.as_deref(), settings.public_key.as_deref())
}

impl App {
    /// What the application knows of the plugin registry: whether it is
    /// set up, the last index that verified, and why there is none.
    #[must_use]
    pub fn plugin_registry(&self) -> &PluginRegistry {
        &self.plugin_registry.view
    }

    /// Names Rick's cache directory, where the plugin list is kept. The
    /// binary hands over the platform's own when Rick runs on the
    /// platform's config directory; without the call the cache is the
    /// folder [`CACHE_DIR`] of the config directory, so that a Rick on a
    /// config directory of its own shares nothing.
    pub fn set_cache_dir(&mut self, dir: PathBuf) {
        self.plugin_registry.cache_dir = dir;
    }

    /// Makes the registry answer as for a Rick of another version than
    /// this build's: which builds fit, and what the requests call it.
    pub fn set_rick_version(&mut self, version: &str) {
        version.clone_into(&mut self.plugin_registry.view.rick);
    }

    /// Handles a [`Command::Registry`](crate::Command::Registry).
    pub(crate) fn registry_command(&mut self, command: RegistryCommand) {
        let source = self.plugin_registry.source.clone();
        let Some(key) = source.key.clone() else {
            // Off: nothing is read and nothing is fetched.
            return;
        };
        if is_pre_release(&self.plugin_registry.view.rick) || self.plugin_registry.job.is_some() {
            return;
        }
        let allowed = self.model.settings.registry.check;
        let fetch = match command {
            RegistryCommand::Start | RegistryCommand::Opened => allowed,
            RegistryCommand::CheckNow => true,
        };
        let read = !self.plugin_registry.cache_read;
        if !read && !fetch {
            return;
        }
        self.plugin_registry.cache_read = true;
        // The start waits for the cache's age before it is known whether
        // it fetches; the two others fetch for certain.
        let only_when_due = command == RegistryCommand::Start;
        self.plugin_registry.view.checking = fetch && !only_when_due;
        let cache = Cache::new(&self.plugin_registry.cache_dir);
        let rick = self.plugin_registry.view.rick.clone();
        let tx = self.mailbox.clone();
        let handle = self.jobs.spawn(move |_job| async move {
            // Fresh is an index that can be used and is not a day old: a
            // cached one that does not verify is no reason to wait.
            let fresh = if read {
                let cached = {
                    let cache = cache.clone();
                    runtime::handle()
                        .spawn_blocking(move || cache.read(&key))
                        .await
                        .unwrap_or_else(|join| {
                            Err(plugins_registry::Error::Unreadable(join.to_string()))
                        })
                };
                let fresh = matches!(
                    &cached,
                    Ok(Some(cached)) if !due(Some(cached.fetched), SystemTime::now())
                );
                tx.send(Msg::RegistryCached(cached));
                fresh
            } else {
                !due(cache.fetched(), SystemTime::now())
            };
            let fetched = if fetch && !(only_when_due && fresh) {
                tx.send(Msg::RegistryChecking);
                Some(refresh(&source, &cache, &rick).await)
            } else {
                None
            };
            tx.send(Msg::RegistryDone(fetched));
            Ok(())
        });
        self.internal.insert(handle.id);
        self.plugin_registry.job = Some(handle.id);
    }

    /// The cache was read.
    pub(crate) fn registry_cached(
        &mut self,
        cached: Result<Option<Cached>, plugins_registry::Error>,
    ) {
        match cached {
            Ok(Some(cached)) => self.registry_take(cached),
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(%error, "the cached plugin list is not used");
                self.plugin_registry.view.problem = Some(RegistryProblem::from(&error));
            }
        }
    }

    /// A job began to fetch.
    pub(crate) fn registry_checking(&mut self) {
        self.plugin_registry.view.checking = true;
    }

    /// A job ended, with the answer of its fetch when it made one.
    pub(crate) fn registry_done(
        &mut self,
        fetched: Option<Result<Cached, plugins_registry::Error>>,
    ) {
        self.plugin_registry.job = None;
        self.plugin_registry.view.checking = false;
        match fetched {
            Some(Ok(cached)) => self.registry_take(cached),
            Some(Err(error)) => {
                tracing::warn!(%error, "the plugin list was not fetched");
                self.plugin_registry.view.problem = Some(RegistryProblem::from(&error));
            }
            None => {}
        }
    }

    /// An index that verified is the one to show.
    fn registry_take(&mut self, cached: Cached) {
        let view = &mut self.plugin_registry.view;
        view.index = Some(Arc::new(cached.index));
        view.fetched = Some(cached.fetched);
        view.problem = None;
    }
}
