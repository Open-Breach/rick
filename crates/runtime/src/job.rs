//! Jobs: cancellable, progress-reporting futures with an id.

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::sync::{oneshot, watch};
use tracing::Instrument;
use types::{CancelToken, Error, JobId, Progress};

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
}

/// Spawns jobs on the process-wide runtime and keeps track of the running
/// ones. Cloning gives another handle to the same runner.
#[derive(Clone, Default)]
pub struct JobRunner {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    next_id: AtomicU64,
    running: Mutex<BTreeMap<JobId, CancelToken>>,
}

impl Inner {
    fn running(&self) -> MutexGuard<'_, BTreeMap<JobId, CancelToken>> {
        // The map holds plain tokens; a panic elsewhere cannot corrupt it.
        self.running.lock().unwrap_or_else(PoisonError::into_inner)
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
    /// A runner with no jobs.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
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
        let id = JobId(self.inner.next_id.fetch_add(1, Ordering::Relaxed) + 1);
        let cancel = CancelToken::new();
        let (progress_tx, progress_rx) = watch::channel(Progress::Items {
            done: 0,
            total: None,
        });
        let (result_tx, result_rx) = oneshot::channel();

        self.inner.running().insert(id, cancel.clone());
        let deregister = Deregister {
            inner: Arc::clone(&self.inner),
            id,
        };

        let future = job(JobCtx {
            id,
            cancel: cancel.clone(),
            progress: ProgressSink(progress_tx),
        });
        let cancelled = cancel.cancelled();
        let span = tracing::debug_span!("job", %id);

        crate::handle().spawn(
            async move {
                let result = tokio::select! {
                    biased;
                    () = cancelled => Err(Error::Cancelled),
                    result = future => result,
                };
                tracing::debug!(ok = result.is_ok(), "job finished");
                // Leave the running list before the caller can observe the
                // result, so `running()` right after `result.await` is exact.
                drop(deregister);
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
        let token = self.inner.running().get(&id).cloned();
        match token {
            Some(token) => {
                token.cancel();
                true
            }
            None => false,
        }
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
    fn a_job_completes_with_its_value_and_leaves_the_list() {
        let runner = JobRunner::new();
        crate::block_on(async {
            let handle = runner.spawn(|_ctx| async { Ok::<_, Error>(41 + 1) });
            assert!(matches!(handle.result.await, Ok(Ok(42))));
            assert!(runner.running().is_empty());
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
            assert!(runner.running().is_empty());
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
            assert!(runner.running().is_empty());
        });
    }
}
