//! Per-volume job scheduling: two jobs that share a source or destination
//! volume run one after the other, jobs on disjoint volumes run in
//! parallel (SPEC.md "Operations area", Concurrency).
//!
//! A copy, move or duplicate job works out the [`VolumeKey`]s it touches
//! and [`acquire`](Scheduler::acquire)s them before its first step. While
//! another running job holds one of them the job waits, queued first in
//! first out, and the application shows its row as *Waiting* with the name
//! of the job it waits for. The [`Lease`] the job gets releases the keys
//! when it is dropped, which a cancelled job does too, since cancelling
//! drops its future; the next queued job whose keys are all free then
//! starts.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use runtime::{JobCtx, oneshot};
use tracing::debug;
use types::{JobId, Location, ProviderId};
use vfs::Mounts;

/// Called whenever a job starts waiting or stops, so the application can
/// show the *Waiting* state.
pub type ScheduleWaker = Arc<dyn Fn() + Send + Sync>;

/// What a job must not share with another running job.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum VolumeKey {
    /// A local file system, by the device half of its entry ids.
    Local(u64),
    /// Another provider as a whole: an object store, an SFTP connection.
    Provider(ProviderId),
}

/// A job queued for its volumes.
struct Waiter {
    id: JobId,
    name: String,
    keys: Vec<VolumeKey>,
    wake: oneshot::Sender<()>,
}

#[derive(Default)]
struct Inner {
    /// The jobs holding their volumes, with their names.
    running: HashMap<JobId, (String, Vec<VolumeKey>)>,
    /// The jobs waiting, oldest first.
    queue: Vec<Waiter>,
}

impl Inner {
    /// The name of the running job that holds one of `keys`, if any.
    fn blocker(&self, keys: &[VolumeKey]) -> Option<(JobId, String)> {
        self.running
            .iter()
            .filter(|(_, (_, held))| held.iter().any(|k| keys.contains(k)))
            .min_by_key(|(id, _)| **id)
            .map(|(id, (name, _))| (*id, name.clone()))
    }

    /// Starts every queued job whose keys are free, oldest first; a job
    /// started in this pass holds its keys against the ones after it.
    fn start_ready(&mut self) {
        let mut index = 0;
        while index < self.queue.len() {
            if self.queue[index].wake.is_closed() {
                // The waiting job was cancelled before its turn.
                let gone = self.queue.remove(index);
                debug!(%gone.id, "waiting job gone");
                continue;
            }
            if self.blocker(&self.queue[index].keys).is_some() {
                index += 1;
                continue;
            }
            let waiter = self.queue.remove(index);
            debug!(%waiter.id, "job starts");
            self.running.insert(waiter.id, (waiter.name, waiter.keys));
            let _ = waiter.wake.send(());
        }
    }
}

/// The per-volume queue the copy and move jobs go through.
#[derive(Default)]
pub struct Scheduler {
    inner: Mutex<Inner>,
    waker: Mutex<Option<ScheduleWaker>>,
}

impl std::fmt::Debug for Scheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.lock();
        f.debug_struct("Scheduler")
            .field("running", &inner.running.len())
            .field("waiting", &inner.queue.len())
            .finish()
    }
}

impl Scheduler {
    /// A scheduler with nothing running.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // Plain data; a panic elsewhere leaves it usable.
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Sets the function called when a job starts waiting or stops.
    pub fn set_waker(&self, waker: ScheduleWaker) {
        *self.waker.lock().unwrap_or_else(PoisonError::into_inner) = Some(waker);
    }

    fn wake(&self) {
        let waker = self
            .waker
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(wake) = waker {
            wake();
        }
    }

    /// Every job waiting right now with the name of the job it waits for,
    /// oldest first.
    #[must_use]
    pub fn waiting(&self) -> Vec<(JobId, String)> {
        let inner = self.lock();
        inner
            .queue
            .iter()
            .filter(|w| !w.wake.is_closed())
            .map(|w| {
                let for_name = inner
                    .blocker(&w.keys)
                    .map_or_else(|| "another job".to_owned(), |(_, name)| name);
                (w.id, for_name)
            })
            .collect()
    }

    /// The jobs holding volumes right now, ascending.
    #[must_use]
    pub fn running(&self) -> Vec<JobId> {
        let mut ids: Vec<JobId> = self.lock().running.keys().copied().collect();
        ids.sort();
        ids
    }

    /// Waits until no other running job holds one of `keys`, then holds
    /// them for `job` until the returned lease is dropped. A job with no
    /// keys runs at once.
    pub async fn acquire(
        self: &Arc<Self>,
        job: &JobCtx,
        name: &str,
        keys: Vec<VolumeKey>,
    ) -> Lease {
        let rx = {
            let mut inner = self.lock();
            match inner.blocker(&keys) {
                None => {
                    inner.running.insert(job.id, (name.to_owned(), keys));
                    None
                }
                Some((blocker, blocker_name)) => {
                    debug!(id = %job.id, %blocker, "job waits for its volume");
                    let (tx, rx) = oneshot::channel();
                    inner.queue.push(Waiter {
                        id: job.id,
                        name: name.to_owned(),
                        keys,
                        wake: tx,
                    });
                    drop(blocker_name);
                    Some(rx)
                }
            }
        };
        if let Some(rx) = rx {
            self.wake();
            // A dropped receiver (the job was cancelled) is noticed by the
            // next pass over the queue; `rx` ending in an error means the
            // scheduler went away, in which case the job just runs.
            let _ = rx.await;
            self.wake();
        }
        Lease {
            scheduler: Arc::clone(self),
            id: job.id,
        }
    }

    /// Releases a job's keys and starts what can start.
    fn release(&self, id: JobId) {
        let mut inner = self.lock();
        if inner.running.remove(&id).is_some() {
            inner.start_ready();
        }
        drop(inner);
        self.wake();
    }
}

/// A job's hold on its volumes; dropping it lets the next queued job start.
#[must_use = "the keys are released when the lease is dropped"]
pub struct Lease {
    scheduler: Arc<Scheduler>,
    id: JobId,
}

impl std::fmt::Debug for Lease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lease")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.scheduler.release(self.id);
    }
}

/// The device of a local path, or of its nearest existing ancestor: the
/// device half of the local entry id.
fn local_device(path: &Path) -> Option<u64> {
    let mut current = Some(path);
    while let Some(p) = current {
        if let Ok(raw) = platform::fs::stat(p) {
            // The device is the high half of the local id.
            #[allow(clippy::cast_possible_truncation)]
            return Some((raw.id >> 64) as u64);
        }
        current = p.parent();
    }
    None
}

/// The volume a location is on: the device of a local path (or of its
/// nearest existing ancestor, for a target that does not exist yet), the
/// provider for anything else.
pub async fn volume_key(loc: &Location) -> VolumeKey {
    if loc.provider != vfs_local::PROVIDER_ID {
        return VolumeKey::Provider(loc.provider);
    }
    let Ok(path) = platform::paths::to_path(&loc.path) else {
        return VolumeKey::Provider(loc.provider);
    };
    let device = runtime::handle()
        .spawn_blocking(move || local_device(&path))
        .await
        .ok()
        .flatten();
    device.map_or(VolumeKey::Provider(loc.provider), VolumeKey::Local)
}

/// The volumes a set of source-to-target pairs touches, each once.
pub async fn volume_keys(_mounts: &Mounts, pairs: &[(Location, Location)]) -> Vec<VolumeKey> {
    let mut keys: Vec<VolumeKey> = Vec::new();
    for (from, to) in pairs {
        for loc in [from, to] {
            let key = volume_key(loc).await;
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
    }
    keys
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use runtime::JobRunner;
    use types::Error;

    use super::*;

    fn key(n: u64) -> VolumeKey {
        VolumeKey::Local(n)
    }

    #[test]
    fn jobs_on_one_volume_run_in_turn_and_disjoint_ones_together() {
        let scheduler = Scheduler::new();
        let runner = JobRunner::new();
        runtime::block_on(async {
            let (first_go_tx, first_go_rx) = oneshot::channel::<()>();
            let s = Arc::clone(&scheduler);
            let first = runner.spawn_named("first", move |job| async move {
                let _lease = s.acquire(&job, "first", vec![key(1)]).await;
                let _ = first_go_rx.await;
                Ok::<(), Error>(())
            });
            tokio::time::sleep(Duration::from_millis(20)).await;
            assert_eq!(scheduler.running(), [first.id]);

            // The second shares the volume: it waits for the first by name.
            let (second_started_tx, mut second_started_rx) = oneshot::channel::<()>();
            let s = Arc::clone(&scheduler);
            let second = runner.spawn_named("second", move |job| async move {
                let _lease = s.acquire(&job, "second", vec![key(1), key(2)]).await;
                let _ = second_started_tx.send(());
                tokio::time::sleep(Duration::from_millis(30)).await;
                Ok::<(), Error>(())
            });
            tokio::time::sleep(Duration::from_millis(20)).await;
            assert_eq!(
                scheduler.waiting(),
                [(second.id, "first".to_owned())],
                "the second waits for the first"
            );
            assert!(second_started_rx.try_recv().is_err());

            // A third on another volume runs at once.
            let s = Arc::clone(&scheduler);
            let third = runner.spawn_named("third", move |job| async move {
                let _lease = s.acquire(&job, "third", vec![key(3)]).await;
                Ok::<(), Error>(())
            });
            assert!(matches!(third.result.await, Ok(Ok(()))));
            assert_eq!(scheduler.waiting().len(), 1, "the third never waited");

            // The first ends: the second starts.
            let _ = first_go_tx.send(());
            assert!(matches!(first.result.await, Ok(Ok(()))));
            assert!(
                tokio::time::timeout(Duration::from_secs(2), &mut second_started_rx)
                    .await
                    .is_ok()
            );
            assert!(scheduler.waiting().is_empty());
            assert!(matches!(second.result.await, Ok(Ok(()))));
            assert!(scheduler.running().is_empty());
        });
    }

    #[test]
    fn cancelling_the_running_job_starts_the_next_and_a_cancelled_waiter_is_skipped() {
        let scheduler = Scheduler::new();
        let runner = JobRunner::new();
        runtime::block_on(async {
            let s = Arc::clone(&scheduler);
            let first = runner.spawn_named("first", move |job| async move {
                let _lease = s.acquire(&job, "first", vec![key(1)]).await;
                std::future::pending::<()>().await;
                Ok::<(), Error>(())
            });
            tokio::time::sleep(Duration::from_millis(20)).await;
            let s = Arc::clone(&scheduler);
            let second = runner.spawn_named("second", move |job| async move {
                let _lease = s.acquire(&job, "second", vec![key(1)]).await;
                std::future::pending::<()>().await;
                Ok::<(), Error>(())
            });
            let (third_started_tx, third_started_rx) = oneshot::channel::<()>();
            let s = Arc::clone(&scheduler);
            let third = runner.spawn_named("third", move |job| async move {
                let _lease = s.acquire(&job, "third", vec![key(1)]).await;
                let _ = third_started_tx.send(());
                Ok::<(), Error>(())
            });
            tokio::time::sleep(Duration::from_millis(20)).await;
            assert_eq!(scheduler.waiting().len(), 2);

            // The waiting second is cancelled: it leaves the queue.
            assert!(runner.cancel(second.id));
            assert!(matches!(second.result.await, Ok(Err(Error::Cancelled))));
            assert_eq!(
                scheduler.waiting(),
                [(third.id, "first".to_owned())],
                "the cancelled waiter is gone"
            );

            // Cancelling the running first starts the third.
            assert!(runner.cancel(first.id));
            assert!(matches!(first.result.await, Ok(Err(Error::Cancelled))));
            assert!(
                tokio::time::timeout(Duration::from_secs(2), third_started_rx)
                    .await
                    .is_ok()
            );
            assert!(matches!(third.result.await, Ok(Ok(()))));
            assert!(scheduler.running().is_empty());
            assert!(scheduler.waiting().is_empty());
        });
    }

    #[test]
    fn volume_keys_tell_local_devices_from_providers() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let here = Location::new(
            vfs_local::PROVIDER_ID,
            platform::paths::from_path(dir.path()),
        );
        let new_name = Location::new(
            vfs_local::PROVIDER_ID,
            platform::paths::from_path(&dir.path().join("not-yet")),
        );
        let remote = Location::new(ProviderId("s3"), "/bucket");
        runtime::block_on(async {
            let a = volume_key(&here).await;
            let b = volume_key(&new_name).await;
            assert!(matches!(a, VolumeKey::Local(_)));
            assert_eq!(
                a, b,
                "a target that does not exist yet is on its parent's volume"
            );
            assert_eq!(
                volume_key(&remote).await,
                VolumeKey::Provider(ProviderId("s3"))
            );
            let mounts = Mounts::new();
            let keys =
                volume_keys(&mounts, &[(here.clone(), remote.clone()), (here, new_name)]).await;
            assert_eq!(keys.len(), 2, "{keys:?}");
        });
    }
}
