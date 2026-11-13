//! The integration items as the application keeps them
//! (specs/system-integration.md §6.1): the records of `rick.toml`'s
//! `[integration]` table, one per item with its state and the date of the
//! last check, and what the last check found, for the dialog and
//! Settings → Integration.
//!
//! The wanted state is *set-up* for every item unless it was declined. A
//! check compares that with what the platform finds, item by item:
//! [`fold_check`] turns the findings into the new records and the
//! [`ItemStatus`] of each item. An item that was set up and is no longer
//! is *drifted*: its record keeps saying *set-up*, which is how the next
//! start still knows. A declined item is never checked, so it is never
//! listed and never watched for drift.

use std::collections::BTreeMap;
use std::time::SystemTime;

use platform::integration::{Actual, Check, Desktop, Item, Reason, Receipt};
use serde::{Deserialize, Serialize};
use toml::value::{Date, Datetime};

/// An item's state in `rick.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ItemState {
    /// Rick found it set up, or set it up.
    SetUp,
    /// The user switched it off: not Rick's business any more.
    Declined,
    /// It cannot be set up on this system.
    Unsupported,
    /// It is not set up, and the user has not said no.
    Missing,
}

/// One item's record in `[integration.items]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemRecord {
    /// Set up, declined, unsupported or missing.
    pub state: ItemState,
    /// The date of the last check, the calendar day in UTC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked: Option<Datetime>,
    /// The files and links Rick created for the item, `~` for the home
    /// directory: what an undo removes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    /// The values Rick replaced for the item, as they were: what an undo
    /// puts back.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub previous: BTreeMap<String, String>,
}

impl ItemRecord {
    /// A record in `state` without a check or anything written.
    #[must_use]
    pub fn new(state: ItemState) -> Self {
        Self {
            state,
            checked: None,
            files: Vec::new(),
            previous: BTreeMap::new(),
        }
    }

    /// What Rick wrote for the item, as the platform takes it.
    #[must_use]
    pub fn receipt(&self) -> Receipt {
        Receipt {
            files: self.files.clone(),
            previous: self.previous.clone(),
        }
    }

    /// Records what a perform wrote.
    pub fn set_receipt(&mut self, receipt: Receipt) {
        self.files = receipt.files;
        self.previous = receipt.previous;
    }
}

/// The `[integration]` table of `rick.toml`.
// Three switches of the spec, each one key.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct IntegrationSettings {
    /// Open the dialog when the startup check finds something missing or
    /// drifted; `false` is the dialog's *Don't ask again*.
    pub ask: bool,
    /// *Check integration at startup*.
    pub check_at_startup: bool,
    /// *Shell integration*: the embedded console follows the tab's
    /// folder, and a `cd` in the console moves the tab.
    pub shell_integration: bool,
    /// One record per item, by the item's key.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub items: BTreeMap<String, ItemRecord>,
}

impl Default for IntegrationSettings {
    fn default() -> Self {
        Self {
            ask: true,
            check_at_startup: true,
            shell_integration: true,
            items: BTreeMap::new(),
        }
    }
}

impl IntegrationSettings {
    /// The record of `item`, if it has one.
    #[must_use]
    pub fn record(&self, item: Item) -> Option<&ItemRecord> {
        self.items.get(item.key())
    }

    /// What Rick wrote for `item` so far.
    #[must_use]
    pub fn receipt(&self, item: Item) -> Receipt {
        self.record(item).map_or_default(ItemRecord::receipt)
    }

    /// Whether the user declined `item`.
    #[must_use]
    pub fn declined(&self, item: Item) -> bool {
        self.record(item)
            .is_some_and(|record| record.state == ItemState::Declined)
    }
}

/// The calendar day of `time` in UTC, as a record's `checked` holds it;
/// `None` for a time before 1970.
#[must_use]
pub fn day_of(time: SystemTime) -> Option<Datetime> {
    let text = platform::iso8601(time);
    let mut parts = text.get(..10)?.split('-');
    let date = Date {
        year: parts.next()?.parse().ok()?,
        month: parts.next()?.parse().ok()?,
        day: parts.next()?.parse().ok()?,
    };
    Some(Datetime {
        date: Some(date),
        time: None,
        offset: None,
    })
}

/// What the dialog and Settings → Integration show of an item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemStatus {
    /// *Set up*.
    SetUp,
    /// Not set up and not declined: the dialog asks.
    Missing,
    /// *Reset by system*: it was set up and is no longer; the dialog asks
    /// and Settings offers *Restore*.
    Drifted,
    /// *Skipped*: switched off by the user; Settings offers *Add*.
    Declined,
    /// *Unsupported*, with the reason.
    Unsupported(Reason),
}

impl ItemStatus {
    /// Whether the dialog lists the item: missing or drifted.
    #[must_use]
    pub fn asked(&self) -> bool {
        matches!(self, Self::Missing | Self::Drifted)
    }
}

/// One item as the last check left it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrationItem {
    /// Which.
    pub item: Item,
    /// What the row shows.
    pub status: ItemStatus,
    /// The facts of the check: where the item is or goes, what is in its
    /// way. Empty for a declined item, which is not checked.
    pub check: Check,
}

/// The rows being set up right now, for the progress line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applying {
    /// The rows finished.
    pub done: usize,
    /// The rows to set up in all.
    pub total: usize,
    /// The row being set up.
    pub current: Option<Item>,
}

/// What the application knows about the integration items: the outcome of
/// the last check and whether one, or an apply, is running.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Integration {
    /// The line naming the operating system and the desktop.
    pub system: String,
    /// The desktop, when it is one of the four with first-class support.
    pub desktop: Option<Desktop>,
    /// Every item of the platform in the dialog's order; empty before the
    /// first check and on a platform without items.
    pub items: Vec<IntegrationItem>,
    /// When the last check finished.
    pub checked_at: Option<SystemTime>,
    /// Whether the last check was the one at startup.
    pub at_startup: bool,
    /// A check is running.
    pub checking: bool,
    /// Rows are being set up.
    pub applying: Option<Applying>,
}

impl Integration {
    /// The row of `item`.
    #[must_use]
    pub fn get(&self, item: Item) -> Option<&IntegrationItem> {
        self.items.iter().find(|row| row.item == item)
    }

    /// The rows the dialog asks about: missing or drifted.
    pub fn asked(&self) -> impl Iterator<Item = &IntegrationItem> {
        self.items.iter().filter(|row| row.status.asked())
    }
}

/// Folds what a check found into the records: the records afterwards and
/// every listed item's row. `found` has the check of each listed item that
/// is not declined; `day` is the date the records take.
///
/// A set-up item found missing is *drifted* and its record keeps saying
/// *set-up*; a declined item stays declined whatever is on the disk; a
/// record of an item this platform does not list is left alone.
#[must_use]
pub fn fold_check(
    records: &BTreeMap<String, ItemRecord>,
    listed: &[Item],
    found: &[(Item, Check)],
    day: Option<Datetime>,
) -> (BTreeMap<String, ItemRecord>, Vec<IntegrationItem>) {
    let mut records = records.clone();
    let mut rows = Vec::with_capacity(listed.len());
    for item in listed {
        let before = records.get(item.key()).map(|record| record.state);
        if before == Some(ItemState::Declined) {
            rows.push(IntegrationItem {
                item: *item,
                status: ItemStatus::Declined,
                check: Check::default(),
            });
            continue;
        }
        let Some((_, check)) = found.iter().find(|(found, _)| found == item) else {
            continue;
        };
        let (state, status) = match &check.actual {
            Actual::SetUp => (ItemState::SetUp, ItemStatus::SetUp),
            Actual::Missing if before == Some(ItemState::SetUp) => {
                (ItemState::SetUp, ItemStatus::Drifted)
            }
            Actual::Missing => (ItemState::Missing, ItemStatus::Missing),
            Actual::Unsupported(reason) => (
                ItemState::Unsupported,
                ItemStatus::Unsupported(reason.clone()),
            ),
        };
        let record = records
            .entry(item.key().to_owned())
            .or_insert_with(|| ItemRecord::new(state));
        record.state = state;
        record.checked = day;
        rows.push(IntegrationItem {
            item: *item,
            status,
            check: check.clone(),
        });
    }
    (records, rows)
}

/// Marks `items` as declined: what the dialog's switched-off rows become.
/// What Rick wrote for an item before stays recorded.
pub fn decline(records: &mut BTreeMap<String, ItemRecord>, items: &[Item], day: Option<Datetime>) {
    for item in items {
        let record = records
            .entry(item.key().to_owned())
            .or_insert_with(|| ItemRecord::new(ItemState::Declined));
        record.state = ItemState::Declined;
        record.checked = day;
    }
}

/// Records that `item` was set up, with what the perform wrote.
pub fn set_up(
    records: &mut BTreeMap<String, ItemRecord>,
    item: Item,
    receipt: Receipt,
    day: Option<Datetime>,
) {
    let record = records
        .entry(item.key().to_owned())
        .or_insert_with(|| ItemRecord::new(ItemState::SetUp));
    record.state = ItemState::SetUp;
    record.checked = day;
    record.set_receipt(receipt);
}

/// *Reset choices*: every declined mark is cleared, so the item is
/// *missing* again and the dialog asks once more. Returns whether any
/// record changed.
pub fn reset_choices(records: &mut BTreeMap<String, ItemRecord>) -> bool {
    let mut changed = false;
    for record in records.values_mut() {
        if record.state == ItemState::Declined {
            record.state = ItemState::Missing;
            changed = true;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn day(text: &str) -> Option<Datetime> {
        text.parse().ok()
    }

    #[test]
    fn the_table_reads_and_writes_as_the_spec_names_it() {
        let defaults = IntegrationSettings::default();
        assert!(defaults.ask && defaults.check_at_startup && defaults.shell_integration);
        let mut settings = IntegrationSettings::default();
        let mut record = ItemRecord::new(ItemState::SetUp);
        record.checked = day("2026-11-09");
        record.set_receipt(Receipt {
            files: vec!["~/.local/bin/rick".to_owned()],
            previous: BTreeMap::new(),
        });
        settings
            .items
            .insert(Item::RickCommand.key().to_owned(), record);
        settings.items.insert(
            Item::DockFavourite.key().to_owned(),
            ItemRecord::new(ItemState::Declined),
        );
        settings.ask = false;
        let text = toml::to_string(&settings).unwrap_or_else(|e| panic!("{e}"));
        assert!(text.contains("ask = false"), "{text}");
        assert!(text.contains("[items.rick-command]"), "{text}");
        assert!(text.contains("state = \"set-up\""), "{text}");
        assert!(text.contains("checked = 2026-11-09"), "{text}");
        assert!(text.contains("state = \"declined\""), "{text}");
        let back: IntegrationSettings = toml::from_str(&text).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back, settings);
        assert!(back.declined(Item::DockFavourite));
        assert_eq!(back.receipt(Item::RickCommand).files, ["~/.local/bin/rick"]);
        assert!(back.receipt(Item::LauncherEntry).is_empty());
        // The four states, as the spec spells them.
        for (state, name) in [
            (ItemState::SetUp, "set-up"),
            (ItemState::Declined, "declined"),
            (ItemState::Unsupported, "unsupported"),
            (ItemState::Missing, "missing"),
        ] {
            let text = toml::to_string(&ItemRecord::new(state)).unwrap_or_default();
            assert_eq!(text.trim(), format!("state = \"{name}\""));
        }
        // An empty table is the defaults.
        let empty: IntegrationSettings = toml::from_str("").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(empty, IntegrationSettings::default());
    }

    #[test]
    fn the_day_of_a_time_is_its_utc_date() {
        let time = SystemTime::UNIX_EPOCH + Duration::from_secs(1_762_300_801);
        assert_eq!(day_of(time), day("2025-11-05"));
        assert_eq!(
            day_of(time).map(|d| d.to_string()).as_deref(),
            Some("2025-11-05")
        );
    }

    #[test]
    fn a_check_sorts_the_items_into_their_states() {
        let listed = Item::ALL;
        let mut records = BTreeMap::new();
        records.insert(
            Item::DefaultFileManager.key().to_owned(),
            ItemRecord::new(ItemState::SetUp),
        );
        records.insert(
            Item::DockFavourite.key().to_owned(),
            ItemRecord::new(ItemState::Declined),
        );
        records.insert("context-menu".to_owned(), ItemRecord::new(ItemState::SetUp));
        // The dock is declined, so it is not among the findings.
        let found = vec![
            (Item::InstallLocation, Check::missing()),
            (Item::LauncherEntry, Check::set_up()),
            (Item::DefaultFileManager, Check::missing()),
            (Item::ShowInFolder, Check::unsupported(Reason::AllUsers)),
            (Item::RickCommand, Check::missing()),
        ];
        let today = day("2026-11-09");
        let (after, rows) = fold_check(&records, &listed, &found, today);
        let status = |item: Item| {
            rows.iter()
                .find(|row| row.item == item)
                .map(|row| row.status.clone())
        };
        assert_eq!(status(Item::InstallLocation), Some(ItemStatus::Missing));
        assert_eq!(status(Item::LauncherEntry), Some(ItemStatus::SetUp));
        assert_eq!(status(Item::DefaultFileManager), Some(ItemStatus::Drifted));
        assert_eq!(
            status(Item::ShowInFolder),
            Some(ItemStatus::Unsupported(Reason::AllUsers))
        );
        assert_eq!(status(Item::DockFavourite), Some(ItemStatus::Declined));
        assert_eq!(rows.len(), 6);
        let state = |item: Item| after.get(item.key()).map(|record| record.state);
        assert_eq!(state(Item::InstallLocation), Some(ItemState::Missing));
        assert_eq!(state(Item::LauncherEntry), Some(ItemState::SetUp));
        // Drifted: the record still says what was wanted and reached.
        assert_eq!(state(Item::DefaultFileManager), Some(ItemState::SetUp));
        assert_eq!(state(Item::ShowInFolder), Some(ItemState::Unsupported));
        assert_eq!(state(Item::DockFavourite), Some(ItemState::Declined));
        assert_eq!(
            after.get(Item::LauncherEntry.key()).and_then(|r| r.checked),
            today
        );
        assert_eq!(
            after.get(Item::DockFavourite.key()).and_then(|r| r.checked),
            None,
            "a declined item is not checked"
        );
        // Another platform's record travels along untouched.
        assert_eq!(
            after.get("context-menu"),
            Some(&ItemRecord::new(ItemState::SetUp))
        );
        // Asked about: the missing and the drifted ones.
        let integration = Integration {
            items: rows,
            ..Integration::default()
        };
        let asked: Vec<Item> = integration.asked().map(|row| row.item).collect();
        assert_eq!(
            asked,
            [
                Item::InstallLocation,
                Item::DefaultFileManager,
                Item::RickCommand
            ]
        );
        assert!(integration.get(Item::DockFavourite).is_some());
    }

    #[test]
    fn declining_setting_up_and_resetting_change_the_records() {
        let mut records = BTreeMap::new();
        let today = day("2026-11-09");
        set_up(
            &mut records,
            Item::LauncherEntry,
            Receipt {
                files: vec!["~/.local/share/applications/rick.desktop".to_owned()],
                previous: BTreeMap::new(),
            },
            today,
        );
        decline(
            &mut records,
            &[Item::DockFavourite, Item::LauncherEntry],
            today,
        );
        let launcher = records.get(Item::LauncherEntry.key());
        assert_eq!(launcher.map(|r| r.state), Some(ItemState::Declined));
        assert_eq!(
            launcher.map(|r| r.files.len()),
            Some(1),
            "what Rick wrote stays on record"
        );
        assert!(reset_choices(&mut records));
        assert!(
            records
                .values()
                .all(|record| record.state == ItemState::Missing)
        );
        assert!(!reset_choices(&mut records), "nothing left to reset");
        // Add: declined becomes set-up, and drift is reported from then on.
        decline(&mut records, &[Item::DockFavourite], today);
        set_up(&mut records, Item::DockFavourite, Receipt::default(), today);
        let (_, rows) = fold_check(
            &records,
            &[Item::DockFavourite],
            &[(Item::DockFavourite, Check::missing())],
            today,
        );
        assert_eq!(rows[0].status, ItemStatus::Drifted);
    }
}
