//! The *ZFS* section of the info panel: the dataset an entry lies in, its
//! snapshot count, the newest snapshot, then the dataset's properties with
//! where each value comes from. No columns, no badges.
//!
//! `panel` must answer at once, so it only reads the cache; `decorate`, for
//! a visible batch, starts the background read of the dataset's snapshot
//! list and properties when they are missing or stale. A panel asked for
//! before that read arrives shows the dataset and a *Loading…* row.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::SystemTime;

use ext::{ColumnDef, DecorateCtx, Decoration, Decorator, PanelSection, t};
use types::{Entry, EntryId, ExtId, Location};

use crate::actions::local_path;
use crate::datasets::Datasets;

/// The panel section and the reads behind it.
#[derive(Debug)]
pub struct ZfsDecorator {
    datasets: Arc<Datasets>,
    /// Datasets whose read is under way; one read serves every batch.
    reading: Arc<Mutex<HashSet<String>>>,
}

/// A date in local time, as the panel shows it.
fn date(at: SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(at)
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

impl ZfsDecorator {
    /// The decorator over the shared dataset cache.
    #[must_use]
    pub fn new(datasets: Arc<Datasets>) -> Self {
        Self {
            datasets,
            reading: Arc::default(),
        }
    }

    /// The dataset of a live location.
    fn dataset_of(&self, loc: &Location) -> Option<String> {
        let path = local_path(loc)?;
        self.datasets.dataset_of(&path).map(|fs| fs.name.clone())
    }

    /// Starts reading the snapshot list and properties of `dataset` unless
    /// both are fresh or a read is under way.
    fn refresh(&self, dataset: String, ctx: &DecorateCtx) {
        if self.datasets.cached(&dataset).is_some()
            && self.datasets.cached_properties(&dataset).is_some()
        {
            return;
        }
        if !self
            .reading
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(dataset.clone())
        {
            return;
        }
        let datasets = Arc::clone(&self.datasets);
        let reading = Arc::clone(&self.reading);
        ctx.jobs.spawn_named(t!("zfs-reading", dataset = dataset.clone()), move |_job| async move {
            let read = dataset.clone();
            let result = ext::blocking(move || {
                datasets.snapshots(&read)?;
                datasets.properties(&read).map(|_| ())
            })
            .await;
            reading
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&dataset);
            if let Err(error) = &result {
                tracing::debug!(%dataset, %error, "ZFS info not read");
            }
            result
        });
    }
}

impl Decorator for ZfsDecorator {
    fn id(&self) -> ExtId {
        crate::ID
    }

    fn applies_to(&self, loc: &Location) -> bool {
        self.dataset_of(loc).is_some()
    }

    fn columns(&self) -> &[ColumnDef] {
        &[]
    }

    fn decorate(&self, _entries: &[Entry], ctx: &DecorateCtx) -> Vec<(EntryId, Decoration)> {
        if let Some(dataset) = self.dataset_of(&ctx.location) {
            self.refresh(dataset, ctx);
        }
        Vec::new()
    }

    fn panel(&self, entry: &Entry) -> Option<PanelSection> {
        let path: PathBuf = ext::platform::paths::to_path(&entry.path).ok()?;
        if entry.id.provider.0 != "local" {
            return None;
        }
        let fs = self.datasets.dataset_of(&path)?;
        let mut rows = vec![(t!("zfs-dataset"), fs.name.clone())];
        let (Some(snapshots), Some(properties)) = (
            self.datasets.cached(&fs.name),
            self.datasets.cached_properties(&fs.name),
        ) else {
            rows.push((t!("zfs-loading"), String::new()));
            return Some(PanelSection {
                title: t!("zfs-zfs"),
                rows,
            });
        };
        rows.push((t!("zfs-snapshots"), snapshots.len().to_string()));
        if let Some(newest) = snapshots.first() {
            rows.push((
                t!("zfs-newest-snapshot"),
                format!("{} ({})", newest.name, date(newest.created)),
            ));
        }
        for property in properties.iter() {
            let source = property.source.text();
            let label = if source.is_empty() {
                property.name.clone()
            } else {
                format!("{} · {source}", property.name)
            };
            rows.push((label, property.shown()));
        }
        Some(PanelSection {
            title: t!("zfs-zfs"),
            rows,
        })
    }

    fn invalidates_on(&self, _loc: &Location) -> Vec<Location> {
        Vec::new()
    }

    fn invalidate(&self, _loc: &Location) {}
}
