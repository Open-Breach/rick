//! Jobs: cancellable, progress-reporting futures with an id.

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, oneshot, watch};
use tracing::Instrument;
use types::{CancelToken, Error, JobId, Progress};

/// The least time between two progress events of one job. A job over
/// hundreds of thousands of entries reports each; the event sink, and so
/// the UI, sees at most this many reports a second per job, always the
/// latest, and the last one before the job ends. A `Progress::Message` is
/// a change of state and is published at once.
pub const PROGRESS_EVERY: Duration = Duration::from_millis(50);

/// What a runner tells its event sink about every job it runs, so the
/// application can follow jobs it did not spawn itself, such as those an
/// action started.
#[derive(Debug)]
pub enum JobEvent {
    /// A job was spawned.
    Started {
        /// The new job.
        id: JobId,
        /// Its name, empty for jobs spawned with [`JobRunner::spawn`].
        name: String,
        /// Whether the job honours [`JobRunner::pause`].
        pausable: bool,
    },
    /// A job published progress.
    Progress {
        /// The job.
        id: JobId,
        /// Its latest report.
        progress: Progress,
    },
    /// A job ended. The handle's receiver gets the value; this carries a
    /// [replica](Error::replicate) of the error.
    Finished {
        /// The job.
        id: JobId,
        /// How it ended.
        result: Result<(), Error>,
    },
}

/// What `runtime` hands back for every long operation.
///
/// The fields are public on purpose: `app` forwards progress into
/// `Event::JobProgress` and the result into `Event::JobDone` without any
/// wrapper. If the job's task is lost (a panic inside the job), `result`
/// resolves with a `RecvError` instead of a value.
#[derive(Debug)]
pub struct JobHandle<T> {
    /// The job's id, for [`JobRunner::cancel`] and the UI's job list.
    pub id: JobId,
    /// Cancels the job; the same token the job sees in its [`JobCtx`].
    pub cancel: CancelToken,
    /// Latest progress report.
    pub progress: watch::Receiver<Progress>,
    /// Resolves once with the job's outcome.
    pub result: oneshot::Receiver<Result<T, Error>>,
}

/// Where a job publishes its progress.
#[derive(Debug, Clone)]
pub struct ProgressSink(watch::Sender<Progress>);

impl ProgressSink {
    /// A sink with its receiver, for code that drives a sub-operation and
    /// forwards its progress into a job's own sink.
    #[must_use]
    pub fn channel() -> (Self, watch::Receiver<Progress>) {
        let (tx, rx) = watch::channel(Progress::Items {
            done: 0,
            total: None,
        });
        (Self(tx), rx)
    }

    /// Publishes a report, replacing the previous one. Never fails: a job
    /// nobody watches still runs.
    pub fn report(&self, progress: Progress) {
        self.0.send_replace(progress);
    }
}

/// What a job receives when it starts.
#[derive(Debug, Clone)]
pub struct JobCtx {
    /// This job's id.
    pub id: JobId,
    /// Cancelled when the caller or the runner cancels the job. The runner
    /// also drops the job's future on cancellation, so a job that never looks
    /// at the token still stops at its next await point.
    pub cancel: CancelToken,
    /// Progress output.
    pub progress: ProgressSink,
    /// Where a pausable job waits between steps while it is paused.
    pub pause: PauseGate,
}

/// The pause switch of a job, as the job sees it. A job that supports
/// pausing awaits [`wait`](Self::wait) between its steps (files, chunks);
/// one that does not never looks and runs through.
#[derive(Debug, Clone)]
pub struct PauseGate(watch::Receiver<bool>);

impl PauseGate {
    /// A gate that is never paused, for jobs run outside a runner.
    #[must_use]
    pub fn open() -> Self {
        let (_tx, rx) = watch::channel(false);
        Self(rx)
    }

    /// Whether the job is paused right now.
    #[must_use]
    pub fn is_paused(&self) -> bool {
        *self.0.borrow()
    }

    /// Returns at once while the job runs, and once it is resumed while it
    /// is paused. A runner that is gone counts as resumed.
    pub async fn wait(&self) {
        let mut rx = self.0.clone();
        while *rx.borrow_and_update() {
            if rx.changed().await.is_err() {
                return;
            }
        }
    }
}

/// Spawns jobs on the process-wide runtime and keeps track of the running
/// ones. Cloning gives another handle to the same runner.
#[derive(Clone, Default)]
pub struct JobRunner {
    inner: Arc<Inner>,
}

/// A running job's controls.
struct Running {
    cancel: CancelToken,
    pause: Option<watch::Sender<bool>>,
}

#[derive(Default)]
struct Inner {
    next_id: AtomicU64,
    running: Mutex<BTreeMap<JobId, Running>>,
    events: Option<mpsc::UnboundedSender<JobEvent>>,
}

impl Inner {
    fn running(&self) -> MutexGuard<'_, BTreeMap<JobId, Running>> {
        // The map holds plain tokens; a panic elsewhere cannot corrupt it.
        self.running.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn publish(&self, event: JobEvent) {
        if let Some(events) = &self.events {
            // A dropped receiver means nobody follows jobs any more.
            let _ = events.send(event);
        }
    }
}

/// Removes the job from the running list however its task ends.
struct Deregister {
    inner: Arc<Inner>,
    id: JobId,
}

impl Drop for Deregister {
    fn drop(&mut self) {
        self.inner.running().remove(&self.id);
    }
}

impl JobRunner {
    /// A runner with no jobs and no event sink.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A runner that reports every job's start, progress and end on the
    /// returned receiver, whoever spawned the job.
    #[must_use]
    pub fn with_events() -> (Self, mpsc::UnboundedReceiver<JobEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let runner = Self {
            inner: Arc::new(Inner {
                next_id: AtomicU64::new(0),
                running: Mutex::new(BTreeMap::new()),
                events: Some(tx),
            }),
        };
        (runner, rx)
    }

    /// Starts `job` as a task on the runtime and returns its handle.
    ///
    /// `job` is called immediately with the job's [`JobCtx`] to build the
    /// future, so it should only capture what it needs and return. The future
    /// runs until it completes or the job is cancelled, whichever is first; a
    /// cancelled job's result is `Err(Error::Cancelled)` even if its future
    /// never looked at the token.
    pub fn spawn<T, F, Fut>(&self, job: F) -> JobHandle<T>
    where
        T: Send + 'static,
        F: FnOnce(JobCtx) -> Fut,
        Fut: Future<Output = Result<T, Error>> + Send + 'static,
    {
        self.spawn_named("", job)
    }

    /// [`spawn`](Self::spawn) with a name for the job list and the event
    /// sink.
    pub fn spawn_named<T, F, Fut>(&self, name: impl Into<String>, job: F) -> JobHandle<T>
    where
        T: Send + 'static,
        F: FnOnce(JobCtx) -> Fut,
        Fut: Future<Output = Result<T, Error>> + Send + 'static,
    {
        self.spawn_with(name, false, job)
    }

    /// [`spawn_named`](Self::spawn_named) for a job that awaits its
    /// [`PauseGate`] between steps, so [`pause`](Self::pause) and
    /// [`resume`](Self::resume) work on it.
    pub fn spawn_pausable<T, F, Fut>(&self, name: impl Into<String>, job: F) -> JobHandle<T>
    where
        T: Send + 'static,
        F: FnOnce(JobCtx) -> Fut,
        Fut: Future<Output = Result<T, Error>> + Send + 'static,
    {
        self.spawn_with(name, true, job)
    }

    // The forwarding loop is one `select!`: the job, its cancellation and
    // the throttled progress belong together.
    #[allow(clippy::too_many_lines)]
    fn spawn_with<T, F, Fut>(&self, name: impl Into<String>, pausable: bool, job: F) -> JobHandle<T>
    where
        T: Send + 'static,
        F: FnOnce(JobCtx) -> Fut,
        Fut: Future<Output = Result<T, Error>> + Send + 'static,
    {
        let id = JobId(self.inner.next_id.fetch_add(1, Ordering::Relaxed) + 1);
        let cancel = CancelToken::new();
        let (progress_tx, progress_rx) = watch::channel(Progress::Items {
            done: 0,
            total: None,
        });
        let (result_tx, result_rx) = oneshot::channel();
        let (pause_tx, pause_rx) = watch::channel(false);

        self.inner.running().insert(
            id,
            Running {
                cancel: cancel.clone(),
                pause: pausable.then_some(pause_tx),
            },
        );
        self.inner.publish(JobEvent::Started {
            id,
            name: name.into(),
            pausable,
        });
        let deregister = Deregister {
            inner: Arc::clone(&self.inner),
            id,
        };

        let future = job(JobCtx {
            id,
            cancel: cancel.clone(),
            progress: ProgressSink(progress_tx),
            pause: PauseGate(pause_rx),
        });
        let cancelled = cancel.cancelled();
        let span = tracing::debug_span!("job", %id);
        let inner = Arc::clone(&self.inner);
        // Progress is forwarded from the job's own task, so the sink sees
        // the latest report before `Finished`; reports closer together than
        // `PROGRESS_EVERY` are coalesced into the one at the window's end.
        let mut progress = inner.events.as_ref().map(|_| progress_rx.clone());

        crate::handle().spawn(
            async move {
                let mut future = Box::pin(future);
                let mut cancelled = Box::pin(cancelled);
                let mut last_published: Option<Instant> = None;
                // A report arrived inside the window and waits for its end.
                let mut deferred = false;
                let window = tokio::time::sleep(Duration::ZERO);
                tokio::pin!(window);
                let result = loop {
                    tokio::select! {
                        biased;
                        () = &mut cancelled => break Err(Error::Cancelled),
                        result = &mut future => break result,
                        changed = async {
                            match progress.as_mut() {
                                Some(rx) => rx.changed().await.is_ok(),
                                None => std::future::pending().await,
                            }
                        } => {
                            if !changed {
                                // The job dropped its sink; nothing more to forward.
                                progress = None;
                                continue;
                            }
                            let latest = progress
                                .as_mut()
                                .map(|rx| rx.borrow_and_update().clone());
                            let Some(latest) = latest else {
                                continue;
                            };
                            // A message is a change of state (a conflict, a
                            // wait) and goes out at once; counts are throttled.
                            let is_message = matches!(latest, Progress::Message(_));
                            match last_published {
                                Some(at) if at.elapsed() < PROGRESS_EVERY && !is_message => {
                                    if !deferred {
                                        deferred = true;
                                        window.as_mut().reset((at + PROGRESS_EVERY).into());
                                    }
                                }
                                _ => {
                                    deferred = false;
                                    inner.publish(JobEvent::Progress { id, progress: latest });
                                    last_published = Some(Instant::now());
                                }
                            }
                        }
                        () = &mut window, if deferred => {
                            deferred = false;
                            let latest = progress
                                .as_mut()
                                .map(|rx| rx.borrow_and_update().clone());
                            if let Some(latest) = latest {
                                inner.publish(JobEvent::Progress { id, progress: latest });
                                last_published = Some(Instant::now());
                            }
                        }
                    }
                };
                // A report the job made just before returning, or one held
                // back by the window, is still unpublished; the sender being
                // gone does not matter.
                let last = progress.as_mut().and_then(|rx| {
                    let value = rx.borrow_and_update();
                    (deferred || value.has_changed()).then(|| value.clone())
                });
                if let Some(latest) = last {
                    inner.publish(JobEvent::Progress {
                        id,
                        progress: latest,
                    });
                }
                tracing::debug!(ok = result.is_ok(), "job finished");
                // Leave the running list before the caller can observe the
                // result, so `running()` right after `result.await` is exact.
                drop(deregister);
                inner.publish(JobEvent::Finished {
                    id,
                    result: result.as_ref().map(|_| ()).map_err(Error::replicate),
                });
                // The caller may have dropped its handle; the job is done
                // regardless.
                let _ = result_tx.send(result);
            }
            .instrument(span),
        );

        JobHandle {
            id,
            cancel,
            progress: progress_rx,
            result: result_rx,
        }
    }

    /// Ids of the jobs that have started and not yet finished, ascending.
    #[must_use]
    pub fn running(&self) -> Vec<JobId> {
        self.inner.running().keys().copied().collect()
    }

    /// Cancels a running job. Returns `false` when no job with that id is
    /// running, because it finished, was cancelled already, or never existed.
    // Cancelling is the point of the call; the flag is advisory.
    #[allow(clippy::must_use_candidate)]
    pub fn cancel(&self, id: JobId) -> bool {
        let token = self.inner.running().get(&id).map(|r| r.cancel.clone());
        match token {
            Some(token) => {
                token.cancel();
                true
            }
            None => false,
        }
    }

    /// Pauses a running job that supports it: the job stops at its next
    /// step and waits for [`resume`](Self::resume). Returns `false` when the
    /// job is not running or does not support pausing.
    #[allow(clippy::must_use_candidate)]
    pub fn pause(&self, id: JobId) -> bool {
        self.set_paused(id, true)
    }

    /// Resumes a paused job. Returns `false` when the job is not running or
    /// does not support pausing.
    #[allow(clippy::must_use_candidate)]
    pub fn resume(&self, id: JobId) -> bool {
        self.set_paused(id, false)
    }

    fn set_paused(&self, id: JobId, paused: bool) -> bool {
        let running = self.inner.running();
        match running.get(&id).and_then(|r| r.pause.as_ref()) {
            Some(gate) => {
                gate.send_replace(paused);
                true
            }
            None => false,
        }
    }

    /// Whether a running job supports pausing.
    #[must_use]
    pub fn is_pausable(&self, id: JobId) -> bool {
        self.inner
            .running()
            .get(&id)
            .is_some_and(|r| r.pause.is_some())
    }
}

impl fmt::Debug for JobRunner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JobRunner")
            .field("running", &self.running())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_standalone_sink_feeds_its_receiver() {
        let (sink, rx) = ProgressSink::channel();
        sink.report(Progress::Bytes {
            done: 1,
            total: None,
        });
        assert_eq!(
            *rx.borrow(),
            Progress::Bytes {
                done: 1,
                total: None
            }
        );
    }

    #[test]
    fn the_event_sink_sees_start_progress_and_end() {
        let (runner, mut events) = JobRunner::with_events();
        crate::block_on(async {
            let handle = runner.spawn_named("copy", |ctx| async move {
                ctx.progress.report(Progress::Bytes {
                    done: 5,
                    total: Some(5),
                });
                Err::<(), Error>(Error::NotFound)
            });
            assert!(matches!(handle.result.await, Ok(Err(Error::NotFound))));

            let mut seen = Vec::new();
            while let Ok(event) =
                tokio::time::timeout(std::time::Duration::from_millis(200), events.recv()).await
            {
                match event {
                    Some(event) => seen.push(event),
                    None => break,
                }
            }
            assert!(
                matches!(seen.first(), Some(JobEvent::Started { name, .. }) if name == "copy"),
                "{seen:?}"
            );
            assert!(
                seen.iter().any(|e| matches!(
                    e,
                    JobEvent::Progress {
                        progress: Progress::Bytes { done: 5, .. },
                        ..
                    }
                )),
                "{seen:?}"
            );
            assert!(
                matches!(
                    seen.last(),
                    Some(JobEvent::Finished {
                        result: Err(Error::NotFound),
                        ..
                    })
                ),
                "{seen:?}"
            );
        });
    }

    #[test]
    fn a_burst_of_reports_reaches_the_sink_throttled_with_the_last_one_intact() {
        let (runner, mut events) = JobRunner::with_events();
        crate::block_on(async {
            let handle = runner.spawn_named("count", |ctx| async move {
                for done in 1..=20_000u64 {
                    ctx.progress.report(Progress::Items {
                        done,
                        total: Some(20_000),
                    });
                    if done % 5_000 == 0 {
                        tokio::task::yield_now().await;
                    }
                }
                Ok::<(), Error>(())
            });
            assert!(matches!(handle.result.await, Ok(Ok(()))));
            let mut seen = Vec::new();
            while let Ok(Some(event)) =
                tokio::time::timeout(Duration::from_millis(200), events.recv()).await
            {
                seen.push(event);
            }
            let reports: Vec<u64> = seen
                .iter()
                .filter_map(|e| match e {
                    JobEvent::Progress {
                        progress: Progress::Items { done, .. },
                        ..
                    } => Some(*done),
                    _ => None,
                })
                .collect();
            assert!(
                reports.len() <= 8,
                "{} progress events for 20 000 reports",
                reports.len()
            );
            assert_eq!(reports.last(), Some(&20_000), "{reports:?}");
            assert!(matches!(seen.last(), Some(JobEvent::Finished { .. })));
        });
    }

    #[test]
    fn a_job_completes_with_its_value_and_leaves_the_list() {
        let runner = JobRunner::new();
        crate::block_on(async {
            let handle = runner.spawn(|_ctx| async { Ok::<_, Error>(41 + 1) });
            assert!(matches!(handle.result.await, Ok(Ok(42))));
            assert_eq!(runner.running(), [] as [JobId; 0]);
        });
    }

    #[test]
    fn ids_are_unique_and_ascending() {
        let runner = JobRunner::new();
        let a = runner.spawn(|_| async { Ok::<(), Error>(()) });
        let b = runner.spawn(|_| async { Ok::<(), Error>(()) });
        assert!(a.id < b.id);
        assert_ne!(
            JobRunner::new().spawn(|_| async { Ok::<(), Error>(()) }).id,
            JobId(0)
        );
    }

    #[test]
    fn cancel_by_id_ends_a_job_that_ignores_its_token() {
        let runner = JobRunner::new();
        crate::block_on(async {
            let handle = runner.spawn(|_ctx| async {
                std::future::pending::<()>().await;
                Ok::<(), Error>(())
            });
            assert_eq!(runner.running(), vec![handle.id]);
            assert!(runner.cancel(handle.id));
            assert!(matches!(handle.result.await, Ok(Err(Error::Cancelled))));
            assert_eq!(runner.running(), [] as [JobId; 0]);
            assert!(!runner.cancel(handle.id));
            assert!(handle.cancel.is_cancelled());
        });
    }

    #[test]
    fn the_handle_token_cancels_and_the_job_sees_it() {
        let runner = JobRunner::new();
        crate::block_on(async {
            let handle = runner.spawn(|ctx| async move {
                ctx.cancel.cancelled().await;
                // A cooperative job maps the token to the error itself.
                Err::<(), Error>(Error::Cancelled)
            });
            handle.cancel.cancel();
            assert!(matches!(handle.result.await, Ok(Err(Error::Cancelled))));
        });
    }

    #[test]
    fn a_pausable_job_waits_at_its_gate_until_resumed() {
        let runner = JobRunner::new();
        crate::block_on(async {
            let (step_tx, mut step_rx) = mpsc::unbounded_channel::<u8>();
            let handle = runner.spawn_pausable("copy", |ctx| async move {
                for step in 1..=3u8 {
                    ctx.pause.wait().await;
                    let _ = step_tx.send(step);
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                Ok::<(), Error>(())
            });
            assert!(runner.is_pausable(handle.id));
            assert_eq!(step_rx.recv().await, Some(1));
            assert!(runner.pause(handle.id));
            // Either the second step was already past its gate or it waits.
            let paused_at = tokio::time::timeout(Duration::from_millis(60), async {
                let mut last = 1;
                while let Some(step) = step_rx.recv().await {
                    last = step;
                }
                last
            })
            .await;
            assert!(paused_at.is_err(), "the job stopped at its gate");
            assert!(runner.resume(handle.id));
            assert!(matches!(handle.result.await, Ok(Ok(()))));
            let plain = runner.spawn(|_| async { Ok::<(), Error>(()) });
            assert!(!runner.is_pausable(plain.id));
            assert!(!runner.pause(JobId(9_999)));
        });
    }

    #[test]
    fn progress_reaches_the_watcher() {
        let runner = JobRunner::new();
        crate::block_on(async {
            let (go_tx, go_rx) = oneshot::channel::<()>();
            let mut handle = runner.spawn(|ctx| async move {
                ctx.progress.report(Progress::Bytes {
                    done: 10,
                    total: Some(100),
                });
                let _ = go_rx.await;
                ctx.progress.report(Progress::Message("done".into()));
                Ok::<(), Error>(())
            });
            assert!(handle.progress.changed().await.is_ok());
            assert_eq!(
                *handle.progress.borrow_and_update(),
                Progress::Bytes {
                    done: 10,
                    total: Some(100)
                }
            );
            let _ = go_tx.send(());
            assert!(matches!(handle.result.await, Ok(Ok(()))));
            assert_eq!(*handle.progress.borrow(), Progress::Message("done".into()));
        });
    }

    #[test]
    fn a_dropped_handle_does_not_stop_the_job() {
        let runner = JobRunner::new();
        let (done_tx, done_rx) = oneshot::channel::<u8>();
        crate::block_on(async {
            let handle = runner.spawn(|_ctx| async move {
                tokio::time::sleep(Duration::from_millis(20)).await;
                let _ = done_tx.send(7);
                Ok::<(), Error>(())
            });
            drop(handle);
            assert_eq!(done_rx.await, Ok(7));
        });
    }

    #[test]
    fn a_panicking_job_is_removed_and_its_result_is_lost() {
        let runner = JobRunner::new();
        crate::block_on(async {
            let handle = runner.spawn(|_ctx| async {
                assert!(!std::hint::black_box(true), "deliberate");
                Ok::<(), Error>(())
            });
            assert!(handle.result.await.is_err());
            assert_eq!(runner.running(), [] as [JobId; 0]);
        });
    }
}
