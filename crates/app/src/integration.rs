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

use std::sync::Arc;
use std::time::SystemTime;

use ext::{Level, t};
use model::{Applying, Integration, SettingsChange};
use platform::integration::{Check, Item, Items, Receipt, Request, Scope};
use types::{Error, JobId};

use crate::{App, Event, Msg};

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
    pub(crate) fn integration(&mut self, command: IntegrationCommand) {
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
                self.apply_integration(scope, &perform, &replace);
            }
            IntegrationCommand::Add { item, replace }
            | IntegrationCommand::Restore { item, replace } => {
                let replace = if replace { vec![item] } else { Vec::new() };
                self.apply_integration(Scope::User, &[item], &replace);
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
                        let request = Request {
                            receipt: records.receipt(*item),
                            ..Request::default()
                        };
                        found.push((*item, platform.check_integration(*item, &request)));
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
        let settings = &self.model.settings.integration;
        let (records, rows) =
            model::fold_check(&settings.items, &listed.items, found, model::day_of(now));
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

    /// Performs `perform`, in the order the platform lists the items, one
    /// after the other; a row that fails does not stop the next.
    fn apply_integration(&mut self, scope: Scope, perform: &[Item], replace: &[Item]) {
        if self.integration.applying.is_some() {
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
        let records = &self.model.settings.integration;
        let requests: Vec<(Item, Request)> = order
            .iter()
            .map(|item| {
                let request = Request {
                    scope,
                    binary: None,
                    receipt: records.receipt(*item),
                    replace: replace.contains(item),
                };
                (*item, request)
            })
            .collect();
        let total = requests.len();
        self.model.integration.applying = Some(Applying {
            done: 0,
            total,
            current: order.first().copied(),
        });
        let platform = Arc::clone(&self.platform);
        let tx = self.mailbox.clone();
        let handle = self.jobs.spawn(move |job| async move {
            let mut results: Vec<(Item, Result<Receipt, Error>)> = Vec::with_capacity(total);
            for (done, (item, request)) in requests.into_iter().enumerate() {
                // Cancelled between two rows: what is done stays done and
                // is recorded; the rest is not started.
                if job.cancel.is_cancelled() {
                    break;
                }
                tx.send(Msg::IntegrationProgress {
                    done,
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
            tx.send(Msg::IntegrationApplied { results });
            Ok(())
        });
        self.internal.insert(handle.id);
        self.integration.applying = Some(handle.id);
    }

    /// A row of the running apply is being set up.
    pub(crate) fn integration_progress(&mut self, done: usize, total: usize, current: Item) {
        if self.integration.applying.is_some() {
            self.model.integration.applying = Some(Applying {
                done,
                total,
                current: Some(current),
            });
        }
    }

    /// The apply ended: what each row wrote is recorded, a row that failed
    /// is reported, and a check shows what is there now.
    pub(crate) fn integration_applied(
        &mut self,
        results: Vec<(Item, Result<Receipt, Error>)>,
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
        events.push(Event::IntegrationApplied { failed });
        self.check_integration(false);
    }
}
