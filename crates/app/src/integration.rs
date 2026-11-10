//! The integration check and what the dialog and Settings → Integration
//! ask for (specs/system-integration.md §6.1).
//!
//! The check is a job off the loop's thread: it asks the platform for its
//! items and for the actual state of each one the user has not declined,
//! reading only, and its answer is folded into the records of `rick.toml`
//! and into the model's [`Integration`](model::Integration). Only *Apply*,
//! *Add* and *Restore* write: each performs its rows one after the other
//! through the platform, records what every perform wrote beside the
//! item's state, reports a row that failed without stopping the others,
//! and ends with a check, so the rows show what is there.
//!
//! The install location is the exception: the installer plugin performs
//! it, through its registered command [`INSTALL_RICK_COMMAND`], as a job
//! of its own in the operations area. The rows after it wait for that
//! job and then act on the installed copy, and the apply's end names the
//! copy for the UI to relaunch from. Without the plugin the row is
//! unsupported and every other row works against the folder Rick runs
//! from.

use std::sync::Arc;
use std::time::SystemTime;

use std::path::PathBuf;

use ext::{ActionOutcome, CliCtx, CliOutcome, Level, t};
use model::{Applying, Integration, SettingsChange};
use platform::integration::{Actual, Check, Item, Items, Reason, Receipt, Request, Scope};
use types::{Error, JobId};

use crate::{App, Event, Msg};

/// The installer plugin's subcommand that performs the install location:
/// `rick install-rick <folder> <version>` (QUESTIONS.md, issue 120). The
/// core owns the row and the check; the plugin, when it is loaded, does
/// the install.
pub const INSTALL_RICK_COMMAND: &str = "install-rick";
/// The plugin's name, for the row that says it is not loaded.
const INSTALLER: &str = "Installer";
/// Rick's binary inside the folder it is installed to.
const BINARY: &str = "rick";

/// What the integration dialog and Settings → Integration ask of the
/// application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntegrationCommand {
    /// Compare the wanted state of every item with the actual one, off
    /// the UI thread; answered by [`Event::IntegrationChecked`]. `startup`
    /// is the check after the first frame, which *Check integration at
    /// startup* switches off and whose answer says whether to open the
    /// dialog; Settings' *Check again* sends `false`.
    Check {
        /// The check at startup.
        startup: bool,
    },
    /// The dialog's *Apply N items*: the switched-off rows are stored as
    /// declined, the switched-on rows are performed in the dialog's order;
    /// answered by [`Event::IntegrationApplied`].
    Apply {
        /// The dialog's *Install for*.
        scope: Scope,
        /// The rows switched on.
        perform: Vec<Item>,
        /// The rows switched off.
        decline: Vec<Item>,
        /// The rows whose files of another origin the user agreed to
        /// replace.
        replace: Vec<Item>,
    },
    /// Settings' *Add* on a skipped item: perform it, and *declined*
    /// becomes *set-up*.
    Add {
        /// The item.
        item: Item,
        /// Replace files of another origin in its way.
        replace: bool,
    },
    /// Settings' *Restore* on an item the system reset: perform it again.
    Restore {
        /// The item.
        item: Item,
        /// Replace files of another origin in its way.
        replace: bool,
    },
    /// *Reset choices*: clear every declined mark and let the dialog ask
    /// again.
    Reset,
}

/// The integration work in flight.
#[derive(Debug, Default)]
pub(crate) struct IntegrationRun {
    /// Numbers the checks, so a superseded one's answer is dropped.
    generation: u64,
    /// The running check.
    checking: Option<JobId>,
    /// The running apply.
    applying: Option<JobId>,
    /// The install the installer plugin runs for the install location,
    /// with the rows that wait for it.
    installing: Option<Installing>,
    /// The copy this process installed Rick to: what every other item
    /// points at from then on, and what a relaunch continues from.
    installed: Option<PathBuf>,
}

/// An apply waiting for the installer plugin's job.
#[derive(Debug)]
struct Installing {
    /// The plugin's install job.
    job: JobId,
    /// The dialog's *Install for*.
    scope: Scope,
    /// The rows to perform once Rick is installed, in order.
    rest: Vec<Item>,
    /// The rows whose foreign files may be replaced.
    replace: Vec<Item>,
    /// The installed binary, through the `current` link.
    binary: PathBuf,
}

/// An item's name in a notification: the title of its row in Settings.
fn item_name(item: Item) -> String {
    match item {
        Item::InstallLocation => t!("integration-item-install-location"),
        Item::LauncherEntry => t!("integration-item-launcher-entry"),
        Item::DefaultFileManager => t!("integration-item-default-file-manager"),
        Item::ShowInFolder => t!("integration-item-show-in-folder"),
        Item::RickCommand => t!("integration-item-rick-command"),
        Item::DockFavourite => t!("integration-item-dock-favourite"),
    }
}

impl App {
    /// Handles a [`Command::Integration`](crate::Command::Integration).
    pub(crate) fn integration(&mut self, command: IntegrationCommand, events: &mut Vec<Event>) {
        match command {
            IntegrationCommand::Check { startup } => self.check_integration(startup),
            IntegrationCommand::Apply {
                scope,
                perform,
                decline,
                replace,
            } => {
                if !decline.is_empty() {
                    let mut records = self.model.settings.integration.items.clone();
                    model::decline(&mut records, &decline, model::day_of(SystemTime::now()));
                    self.change_settings(SettingsChange::IntegrationRecords(records));
                }
                self.apply_integration(scope, &perform, &replace, events);
            }
            IntegrationCommand::Add { item, replace }
            | IntegrationCommand::Restore { item, replace } => {
                let replace = if replace { vec![item] } else { Vec::new() };
                self.apply_integration(Scope::User, &[item], &replace, events);
            }
            IntegrationCommand::Reset => {
                let mut records = self.model.settings.integration.items.clone();
                if model::reset_choices(&mut records) {
                    self.change_settings(SettingsChange::IntegrationRecords(records));
                }
                if !self.model.settings.integration.ask {
                    self.change_settings(SettingsChange::IntegrationAsk(true));
                }
                self.check_integration(false);
            }
        }
    }

    /// Starts a check, in place of one that is running.
    fn check_integration(&mut self, startup: bool) {
        if startup && !self.model.settings.integration.check_at_startup {
            return;
        }
        if let Some(running) = self.integration.checking.take() {
            self.jobs.cancel(running);
        }
        self.integration.generation += 1;
        let generation = self.integration.generation;
        let platform = Arc::clone(&self.platform);
        let records = self.model.settings.integration.clone();
        let installed = self.integration.installed.clone();
        let tx = self.mailbox.clone();
        self.model.integration.checking = true;
        let handle = self.jobs.spawn(move |job| async move {
            let cancel = job.cancel.clone();
            let answer = runtime::handle()
                .spawn_blocking(move || {
                    let listed = platform.integration_items();
                    let mut found: Vec<(Item, Check)> = Vec::new();
                    for item in &listed.items {
                        if cancel.is_cancelled() {
                            return Err(Error::Cancelled);
                        }
                        // Declined is not Rick's business: not even read.
                        if records.declined(*item) {
                            continue;
                        }
                        // The copy this process installed stands for the
                        // running one until the relaunch.
                        let request = Request {
                            binary: installed.clone(),
                            receipt: records.receipt(*item),
                            ..Request::default()
                        };
                        let mut check = platform.check_integration(*item, &request);
                        if *item == Item::InstallLocation && installed.is_some() {
                            check.actual = Actual::SetUp;
                        }
                        found.push((*item, check));
                    }
                    Ok((listed, found))
                })
                .await
                .unwrap_or_else(|join| Err(Error::Io(std::io::Error::other(join))));
            let (listed, found) = answer?;
            tx.send(Msg::IntegrationChecked {
                generation,
                startup,
                listed,
                found,
            });
            Ok(())
        });
        self.internal.insert(handle.id);
        self.integration.checking = Some(handle.id);
    }

    /// A check answered: its findings go into the records and the model.
    pub(crate) fn integration_checked(
        &mut self,
        generation: u64,
        startup: bool,
        listed: Items,
        found: &[(Item, Check)],
        events: &mut Vec<Event>,
    ) {
        if generation != self.integration.generation {
            return;
        }
        self.integration.checking = None;
        let now = SystemTime::now();
        // The install location is the installer plugin's to perform: with
        // the plugin not loaded the row is unsupported, and says why.
        let mut found = found.to_vec();
        if !self.can_install() {
            for (item, check) in &mut found {
                if *item == Item::InstallLocation && check.actual == Actual::Missing {
                    check.actual = Actual::Unsupported(Reason::Plugin(INSTALLER.to_owned()));
                }
            }
        }
        let settings = &self.model.settings.integration;
        let (records, rows) =
            model::fold_check(&settings.items, &listed.items, &found, model::day_of(now));
        let ask = startup && settings.ask && rows.iter().any(|row| row.status.asked());
        if records != settings.items {
            self.change_settings(SettingsChange::IntegrationRecords(records));
        }
        self.model.integration = Integration {
            system: listed.system,
            desktop: listed.desktop,
            items: rows,
            checked_at: Some(now),
            at_startup: startup,
            checking: false,
            applying: self.model.integration.applying.take(),
        };
        events.push(Event::IntegrationChecked { ask });
    }

    /// Whether the installer plugin's command is registered.
    fn can_install(&self) -> bool {
        self.registry
            .commands()
            .iter()
            .any(|command| command.name() == INSTALL_RICK_COMMAND)
    }

    /// Performs `perform`, in the order the platform lists the items, one
    /// after the other; a row that fails does not stop the next. The
    /// install location goes first, through the installer plugin's
    /// command, and the rows after it wait for its job: they then act on
    /// the installed copy.
    fn apply_integration(
        &mut self,
        scope: Scope,
        perform: &[Item],
        replace: &[Item],
        events: &mut Vec<Event>,
    ) {
        if self.integration.applying.is_some() || self.integration.installing.is_some() {
            return;
        }
        // The listed order first; a row asked for before any check keeps
        // the order it was given in.
        let mut order: Vec<Item> = self
            .model
            .integration
            .items
            .iter()
            .map(|row| row.item)
            .filter(|item| perform.contains(item))
            .collect();
        for item in perform {
            if !order.contains(item) {
                order.push(*item);
            }
        }
        if order.is_empty() {
            self.check_integration(false);
            return;
        }
        if let Some(running) = self.integration.checking.take() {
            self.jobs.cancel(running);
            self.integration.generation += 1;
        }
        let total = order.len();
        if let Some(at) = order.iter().position(|item| *item == Item::InstallLocation) {
            order.remove(at);
            self.model.integration.applying = Some(Applying {
                done: 0,
                total,
                current: Some(Item::InstallLocation),
            });
            if let Err(error) = self.start_install(scope, order, replace.to_vec()) {
                // Without the install the other rows would point at the
                // folder Rick was meant to leave: nothing else is done.
                self.integration_applied(vec![(Item::InstallLocation, Err(error))], None, events);
            }
            return;
        }
        let binary = self.integration.installed.clone();
        self.perform_rows(scope, &order, replace, binary.as_ref(), 0, total, None);
    }

    /// Starts the installer plugin's job for the folder Rick runs from,
    /// and parks the rows that follow it.
    fn start_install(
        &mut self,
        scope: Scope,
        rest: Vec<Item>,
        replace: Vec<Item>,
    ) -> Result<(), Error> {
        let row = self.model.integration.get(Item::InstallLocation);
        let folder = row.and_then(|row| row.check.target.clone());
        let place = row.and_then(|row| row.check.place.clone());
        let (Some(folder), Some(place)) = (folder, place) else {
            return Err(Error::Unsupported(
                "the check did not say where Rick runs from and where it goes",
            ));
        };
        let Some(command) = self
            .registry
            .commands()
            .iter()
            .find(|command| command.name() == INSTALL_RICK_COMMAND)
            .cloned()
        else {
            return Err(Error::Unsupported("the installer plugin is not loaded"));
        };
        let pane = self.model.focused;
        let ctx = CliCtx {
            args: vec![
                folder.to_string_lossy().into_owned(),
                env!("CARGO_PKG_VERSION").to_owned(),
            ],
            cwd: folder,
            pane,
            location: self.pane_location(pane),
        };
        match command.run(ctx, &self.jobs, &*self.ui)? {
            CliOutcome::Outcome(ActionOutcome::Started(job)) => {
                self.integration.installing = Some(Installing {
                    job,
                    scope,
                    rest,
                    replace,
                    binary: place.join("current").join(BINARY),
                });
                Ok(())
            }
            _ => Err(Error::Unsupported(
                "the installer plugin did not start its install",
            )),
        }
    }

    /// A job ended: when it is the installer plugin's install, the rows
    /// that waited for it are performed on the installed copy, or, the
    /// install having failed, left alone.
    pub(crate) fn integration_job_finished(
        &mut self,
        job: JobId,
        result: &Result<(), Error>,
        events: &mut Vec<Event>,
    ) {
        let Some(installing) = self.integration.installing.take_if(|i| i.job == job) else {
            return;
        };
        if let Err(error) = result {
            // The job's own failure was shown with its name already.
            tracing::warn!(%error, "Rick was not installed to the standard location");
            self.model.integration.applying = None;
            events.push(Event::IntegrationApplied {
                failed: vec![Item::InstallLocation],
                relaunch: None,
            });
            self.check_integration(false);
            return;
        }
        // Its record is the installer's own, in the app folder; here it
        // is only marked as reached.
        let mut records = self.model.settings.integration.items.clone();
        model::set_up(
            &mut records,
            Item::InstallLocation,
            Receipt::default(),
            model::day_of(SystemTime::now()),
        );
        self.change_settings(SettingsChange::IntegrationRecords(records));
        self.integration.installed = Some(installing.binary.clone());
        let total = installing.rest.len() + 1;
        if installing.rest.is_empty() {
            self.integration_applied(Vec::new(), Some(installing.binary), events);
            return;
        }
        self.perform_rows(
            installing.scope,
            &installing.rest,
            &installing.replace,
            Some(&installing.binary),
            1,
            total,
            Some(installing.binary.clone()),
        );
    }

    /// Performs `order` through the platform as one job off the loop's
    /// thread, each row for `binary` (the running Rick for `None`);
    /// `done` rows of `total` were performed before. `relaunch` travels
    /// to the apply's end.
    // The rows, how to perform them and where the apply stands.
    #[allow(clippy::too_many_arguments)]
    fn perform_rows(
        &mut self,
        scope: Scope,
        order: &[Item],
        replace: &[Item],
        binary: Option<&PathBuf>,
        done: usize,
        total: usize,
        relaunch: Option<PathBuf>,
    ) {
        let records = &self.model.settings.integration;
        let requests: Vec<(Item, Request)> = order
            .iter()
            .map(|item| {
                let request = Request {
                    scope,
                    binary: binary.cloned(),
                    receipt: records.receipt(*item),
                    replace: replace.contains(item),
                };
                (*item, request)
            })
            .collect();
        self.model.integration.applying = Some(Applying {
            done,
            total,
            current: order.first().copied(),
        });
        let platform = Arc::clone(&self.platform);
        let tx = self.mailbox.clone();
        let handle = self.jobs.spawn(move |job| async move {
            let mut results: Vec<(Item, Result<Receipt, Error>)> =
                Vec::with_capacity(requests.len());
            for (index, (item, request)) in requests.into_iter().enumerate() {
                // Cancelled between two rows: what is done stays done and
                // is recorded; the rest is not started.
                if job.cancel.is_cancelled() {
                    break;
                }
                tx.send(Msg::IntegrationProgress {
                    done: done + index,
                    total,
                    current: item,
                });
                let platform = Arc::clone(&platform);
                let result = runtime::handle()
                    .spawn_blocking(move || platform.perform_integration(item, &request))
                    .await
                    .unwrap_or_else(|join| Err(Error::Io(std::io::Error::other(join))));
                results.push((item, result));
            }
            tx.send(Msg::IntegrationApplied { results, relaunch });
            Ok(())
        });
        self.internal.insert(handle.id);
        self.integration.applying = Some(handle.id);
    }

    /// A row of the running apply is being set up.
    pub(crate) fn integration_progress(&mut self, done: usize, total: usize, current: Item) {
        if self.integration.applying.is_some() || self.integration.installing.is_some() {
            self.model.integration.applying = Some(Applying {
                done,
                total,
                current: Some(current),
            });
        }
    }

    /// The apply ended: what each row wrote is recorded, a row that failed
    /// is reported, and a check shows what is there now. `relaunch` is the
    /// copy Rick was installed to in this apply, for the UI to continue
    /// from.
    pub(crate) fn integration_applied(
        &mut self,
        results: Vec<(Item, Result<Receipt, Error>)>,
        relaunch: Option<PathBuf>,
        events: &mut Vec<Event>,
    ) {
        self.integration.applying = None;
        self.model.integration.applying = None;
        let day = model::day_of(SystemTime::now());
        let mut records = self.model.settings.integration.items.clone();
        let mut failed = Vec::new();
        for (item, result) in results {
            match result {
                Ok(receipt) => model::set_up(&mut records, item, receipt, day),
                Err(error) => {
                    tracing::warn!(item = item.key(), %error, "integration item not set up");
                    events.push(Event::Notify {
                        // Best effort: reported, never retried, never in
                        // the way of the other rows.
                        level: if item.best_effort() {
                            Level::Warning
                        } else {
                            Level::Error
                        },
                        text: t!(
                            "integration-failed",
                            item = item_name(item),
                            error = error.to_string()
                        ),
                    });
                    failed.push(item);
                }
            }
        }
        if records != self.model.settings.integration.items {
            self.change_settings(SettingsChange::IntegrationRecords(records));
        }
        events.push(Event::IntegrationApplied { failed, relaunch });
        self.check_integration(false);
    }
}
