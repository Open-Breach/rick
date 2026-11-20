//! The plugin registry as the application keeps it
//! (specs/plugin-distribution.md §6).
//!
//! The client is the crate `plugins-registry`; this module decides when it
//! is asked. Four occasions reach it as a [`RegistryCommand`]: the start,
//! once the first frame is drawn, the background check, the opening of
//! the Extensions dialogue's *Available* view, and *Check now*. Each runs
//! as a job off the loop's thread and answers with messages to the loop,
//! which folds them into [`PluginRegistry`], what the dialogue is drawn
//! from.
//!
//! - Without a public key the registry is off: nothing is read and nothing
//!   is fetched, whatever asks.
//! - The cached index is read once, at the start, and verified as every
//!   read of it is. The start asks nobody.
//! - The background check comes [`REGISTRY_AFTER_START`] after the start
//!   and from then on whenever the list becomes a day old, for as long as
//!   Rick runs: in a Rick that stays open, every 24 hours. With *Look for
//!   plugins and updates online* on (`[registry] check`) it fetches when
//!   the cached index is a day old or there is none; so does the opening
//!   of the view, whatever the list's age. With the switch off only
//!   *Check now* fetches.
//! - A fetch that fails leaves the last good index in place and says why.
//!   The background check tries again a day after the try that failed,
//!   never sooner by itself and never in a loop; the next start, the
//!   opening of the view and *Check now* are occasions of their own.
//! - A Rick with a pre-release part in its version fits nothing the
//!   registry has, and asks for nothing.
//!
//! What the index is used for is here too (§7). [`App::offering`] is the
//! *Available* view: why there is no list, or the plugins that fit this
//! Rick and are in neither plugins folder. **Install**
//! (`Command::InstallAvailable`) is a job of the operations area that
//! downloads a package, holds it against the size and the SHA-256 the
//! index names, unpacks it as *Install from file…* does and notes it in
//! the record. [`App::updates`] lists the plugins the registry installed
//! and has this Rick's build of; an update is the same install over the
//! old folder. *Install from URL…* (`Command::InstallPluginFrom`)
//! downloads a package from an address and checks nothing. A Rick a
//! package manager installed ([`App::packaging`]) shows that package
//! manager's command and downloads none.
//!
//! The key compiled into the build counts only once the binary has said
//! so ([`App::use_build_registry`]): an application made anywhere else
//! knows only the key its `rick.toml` names, which keeps every test away
//! from the real registry.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use ext::{Points, t};
use plugins_registry::{
    CHECK_EVERY, Cache, Cached, Expected, Index, Offer, Source, download, due_after,
    is_pre_release, refresh,
};
use types::{CancelToken, JobId, Progress};

use crate::plugins::{self, Origin};
use crate::{App, Event, Msg};

/// The folder of the config directory that is the cache of a Rick started
/// on a config directory of its own (`--config-dir`), and of every
/// application the binary did not name another one for.
pub const CACHE_DIR: &str = "cache";

/// The folder of the cache a package is downloaded to before it is
/// unpacked.
const DOWNLOADS_DIR: &str = "downloads";

/// The file beside the cached plugin list that holds the version of the
/// Rick whose updates were announced: the toast of the first start that
/// finds any is shown once for a release of Rick, not at every start.
const ANNOUNCED_FILE: &str = "updates-announced";

/// The file beside the cached plugin list that holds the version of the
/// newest release of Rick the user was told of: the toast that says a
/// newer Rick is available is shown once for a release, not at every
/// start.
const RELEASE_FILE: &str = "release-announced";

/// What the files of the cache say was announced before this run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Announced {
    /// The release of Rick whose plugin updates were said
    /// ([`ANNOUNCED_FILE`]).
    updates: Option<String>,
    /// The newest release of Rick that was said to be available
    /// ([`RELEASE_FILE`]).
    release: Option<String>,
}

/// How long after the start the background check first looks whether the
/// plugin list is a day old: not at the start itself, which has enough
/// to do and asks nobody.
pub const REGISTRY_AFTER_START: Duration = Duration::from_mins(1);

/// The longest the background check's timer sleeps at a stretch. A
/// machine that is suspended does not count the time for a timer on
/// every platform, so the timer looks at the clock again every hour
/// rather than once in a day; a look that finds the list younger than a
/// day asks nobody and costs nothing.
const LOOK_EVERY: Duration = Duration::from_hours(1);

/// When the background check runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Schedule {
    /// How long after the start it first looks.
    after_start: Duration,
    /// How old the list is when it is fetched again.
    every: Duration,
}

/// Whether `offered` is a higher version than `installed`, as versions
/// where both are, and never when either is none.
fn is_newer(offered: &str, installed: &str) -> bool {
    match (
        semver::Version::parse(offered),
        semver::Version::parse(installed),
    ) {
        (Ok(offered), Ok(installed)) => offered > installed,
        _ => false,
    }
}

/// What an install job fails with when the package did not arrive whole,
/// or was not asked for because its address, or one the download was
/// sent on to, is plain HTTP to another machine: the user's words for it.
fn failure(error: &plugins_registry::Error) -> types::Error {
    use plugins_registry::Error;
    plugins::failure(match error {
        Error::Mismatch => t!("plugins-download-mismatch"),
        Error::Fetch(reason) => t!("plugins-download-failed", reason = reason.as_str()),
        Error::Refused {
            url,
            redirected: false,
        } => t!("plugins-address-refused", url = url.as_str()),
        Error::Refused {
            url,
            redirected: true,
        } => t!("plugins-redirect-refused", url = url.as_str()),
        Error::Inward { url } => t!("plugins-redirect-inward", url = url.as_str()),
        other => other.to_string(),
    })
}

/// An occasion to read the plugin list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryCommand {
    /// The first frame is drawn: read what is cached, ask nobody, and
    /// set the background check for [`REGISTRY_AFTER_START`] from now.
    Start,
    /// The background check's time has come: a while after the start,
    /// and from then on whenever the list becomes a day old. With the
    /// switch on, fetch when the cached list is a day old or there is
    /// none. The application's own timer sends this; nothing of the
    /// interface does.
    Daily,
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
            // An index at an address that is not asked did not arrive.
            Error::Fetch(_) | Error::Refused { .. } | Error::Inward { .. } => Self::Offline,
            Error::NotSetUp
            | Error::Unreadable(_)
            | Error::Mismatch
            | Error::Key(_)
            | Error::Io { .. } => Self::Unreadable,
        }
    }
}

/// A release of Rick newer than the one that runs, as the plugin list
/// names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RickUpdate {
    /// Its version, `0.7.0`.
    pub version: String,
    /// The address of its release page, for the browser. `None` for a
    /// Rick a package manager installed, which is updated there, and
    /// when the list names no `https://` address.
    pub page: Option<String>,
    /// A package manager installed this Rick and updates it.
    pub packaged: bool,
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

/// A plugin of the *Available* view: one the registry has a build of
/// that fits this Rick, and that is in neither plugins folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Available {
    /// The extension id.
    pub id: String,
    /// The plugin's name; its id when the list gives none.
    pub name: String,
    /// What it does, in one line.
    pub description: String,
    /// The version of the build that fits.
    pub version: String,
    /// The extension points it fills.
    pub points: Points,
    /// The file name extensions its previewers claim.
    pub previews: Vec<String>,
    /// The names of an entry that make a folder its business.
    pub markers: Vec<String>,
    /// The titles of the kinds of connection it adds.
    pub connections: Vec<String>,
    /// The size of its package in bytes.
    pub size: u64,
    /// Its install is running.
    pub installing: bool,
}

/// A plugin of the per-user plugins folder that the registry has the
/// build for (specs/plugin-distribution.md §7, *Updates*): the record
/// marks it as the registry's, the list has a build that fits this Rick,
/// and the host refused the one on the disk — built for the Rick before —
/// or the list's build is of a higher plugin version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    /// The extension id.
    pub id: String,
    /// The plugin's folder, by which its row is known.
    pub path: PathBuf,
    /// The version of the build that fits.
    pub version: String,
    /// The plugin is refused as it is: the update is what makes it load.
    /// Otherwise it is loaded, and the update is a newer version of it.
    pub refused: bool,
    /// Its update is running.
    pub updating: bool,
}

/// What the *Available* view has to show (specs/plugin-distribution.md §3
/// and §6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Offering {
    /// No public key: *The plugin list is not set up in this build*.
    NotSetUp,
    /// This Rick's version has a pre-release part, and nothing fits it.
    PreRelease,
    /// This Rick is older than the registry's oldest release: its plugins
    /// are no longer published.
    TooOld,
    /// No plugin list yet: none was cached and none fetched. Why is in
    /// [`PluginRegistry::problem`], or it is the switch that is off.
    Nothing,
    /// The plugins on offer, in the list's order; empty when every plugin
    /// for this Rick is installed, or none is published for it.
    Plugins(Vec<Available>),
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
    /// The installs from the registry that are running, by plugin id.
    installing: BTreeMap<String, JobId>,
    /// The installs from an address that are running.
    from_address: BTreeSet<JobId>,
    /// The package manager this Rick was installed by, when it was: it
    /// then shows the command for a plugin and downloads none.
    pub(crate) packaging: Option<plugins::Packaging>,
    /// Where the index is and which key signs it, as `rick.toml` had it
    /// at the start: another address or key is taken up by the next start.
    source: Source,
    /// When the background check runs.
    schedule: Schedule,
    /// When the background check last asked for the list and did not get
    /// it. It does not ask again by itself until a day has passed since,
    /// however often its timer looks; `None` once a list has arrived.
    failed: Option<SystemTime>,
    /// Stops the timer of the background check that is waiting, if one
    /// is: there is never more than one.
    timer: CancelToken,
    /// The updates were said in this run of Rick, whatever the file of
    /// the cache says by now.
    said: bool,
    /// The newest release of Rick the user was told of, in this run or
    /// in one before it.
    release_said: Option<String>,
}

impl RegistryRun {
    /// The state of an application that has asked nothing yet, for the
    /// source `settings` name and the cache in `cache_dir`. The key
    /// compiled into this build is not part of it: only the binary turns
    /// the build's own registry on ([`App::use_build_registry`]), so an
    /// application made anywhere else — every test — has a registry only
    /// where its `rick.toml` names a key.
    pub(crate) fn new(settings: &model::RegistrySettings, cache_dir: PathBuf) -> Self {
        let source = Source::named(settings.url.as_deref(), settings.public_key.as_deref());
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
            installing: BTreeMap::new(),
            from_address: BTreeSet::new(),
            packaging: None,
            source,
            schedule: Schedule {
                after_start: REGISTRY_AFTER_START,
                every: CHECK_EVERY,
            },
            failed: None,
            timer: CancelToken::new(),
            said: false,
            release_said: None,
        }
    }

    /// Stops the background check's timer: the application is going.
    pub(crate) fn stop(&self) {
        self.timer.cancel();
    }
}

impl App {
    /// What the application knows of the plugin registry: whether it is
    /// set up, the last index that verified, and why there is none.
    #[must_use]
    pub fn plugin_registry(&self) -> &PluginRegistry {
        &self.plugin_registry.view
    }

    /// Turns on the registry of this build: the registry's address and
    /// the key compiled in, where `rick.toml` names no others. The binary
    /// calls it before the first frame. Without the call the application
    /// knows only the key its `rick.toml` names, and with none it asks
    /// nobody — which keeps every test away from the real registry.
    pub fn use_build_registry(&mut self) {
        let settings = &self.model.settings.registry;
        let source = Source::new(settings.url.as_deref(), settings.public_key.as_deref());
        let run = &mut self.plugin_registry;
        run.view.set_up = source.key.is_some();
        run.view.host = source.host();
        run.view.custom = source.custom.then(|| source.url.clone());
        run.source = source;
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

    /// Sets when the background check runs: `after_start` after the
    /// start, and from then on whenever the plugin list is `every` old.
    /// Rick's own are [`REGISTRY_AFTER_START`] and a day; a test that
    /// watches the check come round gives it shorter ones. Takes effect
    /// for the timers set from now on, so it is called before the start.
    pub fn set_registry_schedule(&mut self, after_start: Duration, every: Duration) {
        self.plugin_registry.schedule = Schedule { after_start, every };
    }

    /// Handles a [`Command::Registry`](crate::Command::Registry).
    pub(crate) fn registry_command(&mut self, command: RegistryCommand) {
        let source = self.plugin_registry.source.clone();
        let Some(key) = source.key.clone() else {
            // Off: nothing is read and nothing is fetched.
            return;
        };
        if is_pre_release(&self.plugin_registry.view.rick) {
            return;
        }
        let schedule = self.plugin_registry.schedule;
        let daily = command == RegistryCommand::Daily;
        if self.plugin_registry.job.is_some() {
            // A check is running: the background one looks again shortly.
            if daily {
                self.registry_arm(schedule.after_start);
            }
            return;
        }
        let allowed = self.model.settings.registry.check;
        // The background check asks only for a list that is a day old,
        // and that is known here without a look at the disk.
        let now = SystemTime::now();
        let old = due_after(self.plugin_registry.view.fetched, now, schedule.every);
        // And when its last try failed, only a day after that try: the
        // timer looks every hour, and a list that did not arrive is as
        // old at the next look.
        let waited = due_after(self.plugin_registry.failed, now, schedule.every);
        let fetch = match command {
            RegistryCommand::Start => false,
            RegistryCommand::Daily => allowed && old && waited,
            RegistryCommand::Opened => allowed,
            RegistryCommand::CheckNow => true,
        };
        let read = !self.plugin_registry.cache_read;
        if !read && !fetch {
            if daily {
                // Not yet a day old, or not allowed: look again later.
                self.registry_arm(self.registry_next());
            }
            return;
        }
        self.plugin_registry.cache_read = true;
        // The background check fetches only when the cache it has just
        // read is a day old as well; the two the user starts fetch for
        // certain, and say so while they do.
        let only_when_due = daily;
        // The start and the background check say what the list brought.
        let background = matches!(command, RegistryCommand::Start | RegistryCommand::Daily);
        self.plugin_registry.view.checking = fetch && !only_when_due;
        let cache = Cache::new(&self.plugin_registry.cache_dir);
        let rick = self.plugin_registry.view.rick.clone();
        let tx = self.mailbox.clone();
        let handle = self.jobs.spawn(move |_job| async move {
            // In the background: the release of Rick whose updates were
            // said.
            let announced = if background {
                let folder = cache.dir().to_owned();
                runtime::handle()
                    .spawn_blocking(move || {
                        let said = |file: &str| {
                            std::fs::read_to_string(folder.join(file))
                                .ok()
                                .map(|text| text.trim().to_owned())
                        };
                        Announced {
                            updates: said(ANNOUNCED_FILE),
                            release: said(RELEASE_FILE),
                        }
                    })
                    .await
                    .unwrap_or_default()
            } else {
                Announced::default()
            };
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
                    Ok(Some(cached))
                        if !due_after(Some(cached.fetched), SystemTime::now(), schedule.every)
                );
                tx.send(Msg::RegistryCached(cached));
                fresh
            } else {
                // The cache was read before: the application knew the
                // age of the list it holds and has decided.
                false
            };
            let fetched = if fetch && !(only_when_due && fresh) {
                tx.send(Msg::RegistryChecking);
                Some(refresh(&source, &cache, &rick).await)
            } else {
                None
            };
            tx.send(Msg::RegistryDone {
                fetched,
                command,
                announced,
            });
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

    /// A job ended, with the answer of its fetch when it made one. The
    /// start and the background check set the timer of the next look,
    /// and the first of them that finds updates says so, once for a
    /// release of Rick. Whatever asked, a list that names a newer Rick
    /// than this one says so, once for that release. `announced` is what
    /// was said before this run.
    pub(crate) fn registry_done(
        &mut self,
        fetched: Option<Result<Cached, plugins_registry::Error>>,
        command: RegistryCommand,
        announced: Announced,
        events: &mut Vec<Event>,
    ) {
        self.plugin_registry.job = None;
        self.plugin_registry.view.checking = false;
        match fetched {
            Some(Ok(cached)) => {
                self.plugin_registry.failed = None;
                self.registry_take(cached);
            }
            Some(Err(error)) => {
                tracing::warn!(%error, "the plugin list was not fetched");
                self.plugin_registry.view.problem = Some(RegistryProblem::from(&error));
                // The background check's own try: the next one is a day
                // from now. One the user started leaves its day alone.
                if command == RegistryCommand::Daily {
                    self.plugin_registry.failed = Some(SystemTime::now());
                }
            }
            None => {}
        }
        // The background check comes round: a while after the start, and
        // after each of its own looks. One the user started leaves the
        // timer as it stands; when that fires, the list is young, and
        // the look sets it for the day the list turns old.
        match command {
            RegistryCommand::Start => {
                self.registry_arm(self.plugin_registry.schedule.after_start);
            }
            RegistryCommand::Daily => self.registry_arm(self.registry_next()),
            RegistryCommand::Opened | RegistryCommand::CheckNow => {}
        }
        // What an earlier run said of a newer Rick, where this run has
        // said nothing yet.
        if self.plugin_registry.release_said.is_none() {
            self.plugin_registry.release_said = announced.release;
        }
        self.announce_release(events);
        if !matches!(command, RegistryCommand::Start | RegistryCommand::Daily) {
            return;
        }
        let count = self.updates().len();
        let rick = self.plugin_registry.view.rick.clone();
        if count == 0
            || self.plugin_registry.said
            || announced.updates.as_deref() == Some(rick.as_str())
        {
            return;
        }
        self.plugin_registry.said = true;
        events.push(Event::PluginUpdates { count });
        let folder = Cache::new(&self.plugin_registry.cache_dir).dir().to_owned();
        runtime::handle().spawn_blocking(move || {
            let written = std::fs::create_dir_all(&folder)
                .and_then(|()| std::fs::write(folder.join(ANNOUNCED_FILE), rick));
            if let Err(error) = written {
                tracing::warn!(%error, "the announced updates are not remembered");
            }
        });
    }

    /// The plugins of the per-user plugins folder that have an update:
    /// the record marks them as the registry's, the plugin list has a
    /// build that fits this Rick, and the host refused the one that is
    /// there or the list's build is of a higher version. Empty without a
    /// plugin list, for a plugin updated in this session, and for a Rick
    /// a package manager installed.
    #[must_use]
    pub fn updates(&self) -> Vec<Update> {
        let run = &self.plugin_registry;
        // A packaged Rick has no update of its own to offer: the package
        // manager moves its plugins with it.
        if run.packaging.is_some() {
            return Vec::new();
        }
        let Some(index) = &run.view.index else {
            return Vec::new();
        };
        self.plugins
            .iter()
            .filter(|row| !row.built_in && !row.removed)
            .filter(|row| {
                self.installed
                    .get(&row.path)
                    .is_some_and(|noted| noted.source == Origin::Registry)
            })
            .filter_map(|row| {
                let build = index
                    .plugin(&row.id)?
                    .fitting(&run.view.platform, &run.view.rick)?;
                let refused = matches!(row.status, crate::PluginStatus::Refused(_));
                let newer = row.status != crate::PluginStatus::Installed
                    && is_newer(&build.version, &row.version);
                (refused || newer).then(|| Update {
                    id: row.id.clone(),
                    path: row.path.clone(),
                    version: build.version.clone(),
                    refused,
                    updating: run.installing.contains_key(&row.id),
                })
            })
            .collect()
    }

    /// **Update all**: every update is installed, each as a job of its
    /// own, over the folder that is there.
    pub(crate) fn update_all(&mut self) {
        for update in self.updates() {
            self.install_available(&update.id);
        }
    }

    /// What the *Available* view shows now: why there is no list, or the
    /// plugins of the list that fit this Rick and are in neither plugins
    /// folder. A plugin removed in this session is on offer again.
    #[must_use]
    pub fn offering(&self) -> Offering {
        let run = &self.plugin_registry;
        if !run.view.set_up {
            return Offering::NotSetUp;
        }
        if is_pre_release(&run.view.rick) {
            return Offering::PreRelease;
        }
        let Some(index) = &run.view.index else {
            return Offering::Nothing;
        };
        match index.offer(&run.view.platform, &run.view.rick) {
            Offer::PreRelease => Offering::PreRelease,
            Offer::TooOld => Offering::TooOld,
            Offer::Plugins(offered) => Offering::Plugins(
                offered
                    .into_iter()
                    .filter(|offered| {
                        !self
                            .plugins
                            .iter()
                            .any(|row| row.id == offered.plugin.id && !row.removed)
                    })
                    .map(|offered| {
                        let plugin = offered.plugin;
                        let triggers = &plugin.meta.triggers;
                        Available {
                            id: plugin.id.clone(),
                            name: if plugin.name.is_empty() {
                                plugin.id.clone()
                            } else {
                                plugin.name.clone()
                            },
                            description: plugin.description.clone(),
                            version: offered.build.version.clone(),
                            points: plugin
                                .meta
                                .points
                                .iter()
                                .filter_map(|name| Points::named(name))
                                .fold(Points::empty(), |all, point| all | point),
                            previews: triggers.previews.clone(),
                            markers: triggers.markers.clone(),
                            connections: triggers
                                .connections
                                .iter()
                                .map(|connection| connection.title.clone())
                                .collect(),
                            size: offered.build.size,
                            installing: run.installing.contains_key(&plugin.id),
                        }
                    })
                    .collect(),
            ),
        }
    }

    /// **Install** on the available plugin `id`: a job that downloads the
    /// package of the build that fits into the cache, holds it against
    /// the size and the SHA-256 the list names, unpacks it into the
    /// per-user plugins folder as *Install from file…* does and notes it
    /// in the record as the registry's. The job is in the operations
    /// area, counts the bytes and can be cancelled; a package that is
    /// another one is deleted and the job fails, and nothing reaches the
    /// plugins folder.
    pub(crate) fn install_available(&mut self, id: &str) {
        let run = &self.plugin_registry;
        // A packaged Rick downloads nothing: its plugins are packages.
        if run.packaging.is_some() || run.installing.contains_key(id) {
            return;
        }
        let Some(index) = run.view.index.clone() else {
            return;
        };
        let Some(plugin) = index.plugin(id) else {
            return;
        };
        let Some(build) = plugin.fitting(&run.view.platform, &run.view.rick).cloned() else {
            return;
        };
        let name = if plugin.name.is_empty() {
            id.to_owned()
        } else {
            plugin.name.clone()
        };
        let rick = run.view.rick.clone();
        let downloads = run.cache_dir.join(DOWNLOADS_DIR);
        let package = downloads.join(format!(
            "{id}-{}.{}",
            build.version,
            plugins::PACKAGE_EXTENSION
        ));
        let dir = self.plugins_dir();
        let tx = self.mailbox.clone();
        let handle = self
            .jobs
            .spawn_named(format!("Install {name}"), move |job| async move {
                let total = Some(build.size);
                job.progress.report(Progress::Bytes { done: 0, total });
                std::fs::create_dir_all(&downloads)?;
                let expected = Expected {
                    size: build.size,
                    sha256: &build.sha256,
                };
                let progress = job.progress.clone();
                download(&build.url, &package, &rick, Some(expected), &mut |done| {
                    progress.report(Progress::Bytes { done, total });
                })
                .await
                .map_err(|error| failure(&error))?;
                let unpacked = {
                    let package = package.clone();
                    runtime::handle()
                        .spawn_blocking(move || {
                            let unpacked = plugins::unpack(&package, &dir, Origin::Registry, None);
                            // The package has done its work either way.
                            let _ = std::fs::remove_file(&package);
                            unpacked
                        })
                        .await
                };
                let (plugin, record) = unpacked
                    .map_err(|join| plugins::failure(join.to_string()))?
                    .map_err(|error| plugins::failure(error.to_string()))?;
                tx.send(Msg::PluginRecord(record));
                tx.send(Msg::PluginInstalled(Ok(Box::new(plugin))));
                Ok(())
            });
        self.plugin_registry
            .installing
            .insert(id.to_owned(), handle.id);
    }

    /// Whether the install of the plugin `id` from the registry is
    /// running.
    pub(crate) fn plugin_installing(&self, id: &str) -> bool {
        self.plugin_registry.installing.contains_key(id)
    }

    /// A job ended: when it was an install from the registry or from an
    /// address, that install is no longer running, and `true` says it
    /// was one.
    pub(crate) fn install_finished(&mut self, job: JobId) -> bool {
        let run = &mut self.plugin_registry;
        let before = run.installing.len();
        run.installing.retain(|_, running| *running != job);
        run.installing.len() < before || run.from_address.remove(&job)
    }

    /// *Install from URL…*: a job that downloads the plugin package at
    /// `url` into the cache and then handles it exactly as *Install from
    /// file…* handles a file — nothing is checked, a plugin of the same
    /// folder name is replaced — and notes it in the record with the
    /// address it came from. An address that is not `https://`, or plain
    /// `http://` to this machine, is no job and no request but an error
    /// notice that says `https://` is needed; a download that is sent on
    /// to such an address fails with the same words.
    pub(crate) fn install_from_address(&mut self, url: &str, events: &mut Vec<Event>) {
        let url = url.trim().to_owned();
        if !plugins::is_package_address(&url) {
            events.push(Event::Notify {
                level: ext::Level::Error,
                text: t!("plugins-address-refused", url = url.as_str()),
            });
            return;
        }
        let run = &self.plugin_registry;
        let name = plugins_registry::package_name(&url);
        let rick = run.view.rick.clone();
        let downloads = run.cache_dir.join(DOWNLOADS_DIR);
        let package = downloads.join(&name);
        let dir = self.plugins_dir();
        let tx = self.mailbox.clone();
        let handle = self
            .jobs
            .spawn_named(format!("Install {name}"), move |job| async move {
                job.progress.report(Progress::Bytes {
                    done: 0,
                    total: None,
                });
                std::fs::create_dir_all(&downloads)?;
                let progress = job.progress.clone();
                download(&url, &package, &rick, None, &mut |done| {
                    progress.report(Progress::Bytes { done, total: None });
                })
                .await
                .map_err(|error| failure(&error))?;
                let unpacked = {
                    let package = package.clone();
                    runtime::handle()
                        .spawn_blocking(move || {
                            let unpacked = plugins::unpack(&package, &dir, Origin::Url, Some(url));
                            let _ = std::fs::remove_file(&package);
                            unpacked
                        })
                        .await
                };
                let (plugin, record) = unpacked
                    .map_err(|join| plugins::failure(join.to_string()))?
                    .map_err(|error| plugins::failure(error.to_string()))?;
                tx.send(Msg::PluginRecord(record));
                tx.send(Msg::PluginInstalled(Ok(Box::new(plugin))));
                Ok(())
            });
        self.plugin_registry.from_address.insert(handle.id);
    }

    /// Says which package manager installed this Rick, when one did: the
    /// binary reads the file `package` beside itself. A packaged Rick
    /// shows the command for an available plugin and downloads none.
    pub fn set_packaging(&mut self, packaging: Option<plugins::Packaging>) {
        self.plugin_registry.packaging = packaging;
    }

    /// The package manager this Rick was installed by; `None` for an
    /// archive and for a development build.
    #[must_use]
    pub fn packaging(&self) -> Option<plugins::Packaging> {
        self.plugin_registry.packaging
    }

    /// The release of Rick that is newer than this one, when the plugin
    /// list names one (specs/plugin-distribution.md §6, *The update
    /// notice*): what the toast, the menu and the About dialog say. It is
    /// read from the list Rick holds and asks nobody. `None` with *Look
    /// for plugins and updates online* off, without a list, and while
    /// this Rick is that release or a newer one.
    #[must_use]
    pub fn rick_update(&self) -> Option<RickUpdate> {
        if !self.model.settings.registry.check {
            return None;
        }
        let run = &self.plugin_registry;
        let latest = run.view.index.as_ref()?.newer_than(&run.view.rick)?;
        let packaged = run.packaging.is_some();
        Some(RickUpdate {
            version: latest.version.clone(),
            // A packaged Rick is updated where it was installed.
            page: latest.page().filter(|_| !packaged).map(str::to_owned),
            packaged,
        })
    }

    /// Says that a newer Rick is available, once for a release: the
    /// release is remembered in the cache, beside the plugin list.
    fn announce_release(&mut self, events: &mut Vec<Event>) {
        let Some(update) = self.rick_update() else {
            return;
        };
        if self.plugin_registry.release_said.as_deref() == Some(update.version.as_str()) {
            return;
        }
        self.plugin_registry.release_said = Some(update.version.clone());
        let version = update.version;
        events.push(Event::RickUpdate {
            version: version.clone(),
        });
        let folder = Cache::new(&self.plugin_registry.cache_dir).dir().to_owned();
        runtime::handle().spawn_blocking(move || {
            let written = std::fs::create_dir_all(&folder)
                .and_then(|()| std::fs::write(folder.join(RELEASE_FILE), version));
            if let Err(error) = written {
                tracing::warn!(%error, "the announced release is not remembered");
            }
        });
    }

    /// How long until the background check is due again: until a day
    /// has passed since its try that failed, when its last one did, and
    /// else until the list that arrived last is a day old. A whole day
    /// when the list is that old already or there is none, which is while
    /// the switch is off. The timer sleeps an hour of it at most
    /// ([`LOOK_EVERY`]); the look that follows asks only when the time
    /// is up.
    fn registry_next(&self) -> Duration {
        let run = &self.plugin_registry;
        let every = run.schedule.every;
        let now = SystemTime::now();
        // What is left of the day that began at `since`.
        let left = |since: Option<SystemTime>| {
            since
                .and_then(|since| now.duration_since(since).ok())
                .and_then(|age| every.checked_sub(age))
                .filter(|left| !left.is_zero())
        };
        left(run.failed)
            .or_else(|| left(run.view.fetched))
            .unwrap_or(every)
    }

    /// Sets the background check's timer: [`RegistryCommand::Daily`]
    /// comes `after` from now, or after [`LOOK_EVERY`] when that is
    /// sooner, and finds out then whether the list is a day old. A timer
    /// that was waiting is dropped; there is one at most.
    fn registry_arm(&mut self, after: Duration) {
        self.plugin_registry.timer.cancel();
        self.plugin_registry.timer = CancelToken::new();
        let cancel = self.plugin_registry.timer.clone();
        let tx = self.mailbox.clone();
        let after = after.min(LOOK_EVERY);
        runtime::handle().spawn(async move {
            tokio::select! {
                () = cancel.cancelled() => {}
                () = tokio::time::sleep(after) => {
                    tx.send(Msg::RegistryDue);
                }
            }
        });
    }

    /// An index that verified is the one to show.
    fn registry_take(&mut self, cached: Cached) {
        let view = &mut self.plugin_registry.view;
        view.index = Some(Arc::new(cached.index));
        view.fetched = Some(cached.fetched);
        view.problem = None;
        // Another list, other markers: the folders the panes show are
        // looked at again.
        self.suggest_markers_changed();
    }
}
