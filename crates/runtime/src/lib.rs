//! The job system: Rick's single `tokio` runtime, the [`JobRunner`] and the
//! [`compute`] pool.
//!
//! No other crate spawns threads. Every long operation is a job: a future
//! started through [`JobRunner::spawn`] that receives a [`JobCtx`] with its
//! [`CancelToken`](types::CancelToken) and a [`ProgressSink`], and whose
//! caller holds a [`JobHandle`] with the cancel token, a `watch` receiver for
//! progress and a `oneshot` receiver for the result. CPU-bound work inside a
//! job goes through [`compute::run`] so the runtime's worker threads stay free
//! for I/O.
//!
//! ```
//! use runtime::JobRunner;
//! use types::{Error, Progress};
//!
//! let runner = JobRunner::new();
//! let handle = runner.spawn(|ctx| async move {
//!     for done in 1..=3 {
//!         if ctx.cancel.is_cancelled() {
//!             return Err(Error::Cancelled);
//!         }
//!         let doubled = runtime::compute::run(move || done * 2).await;
//!         ctx.progress.report(Progress::Items { done, total: Some(3) });
//!         let _ = doubled;
//!     }
//!     Ok::<_, Error>("listed")
//! });
//! let result = runtime::block_on(handle.result);
//! assert!(matches!(result, Ok(Ok("listed"))));
//! ```

#![forbid(unsafe_code)]

use std::future::Future;
use std::sync::OnceLock;

pub mod compute;
mod job;

pub use job::{JobCtx, JobEvent, JobHandle, JobRunner, PROGRESS_EVERY, PauseGate, ProgressSink};
/// Re-exported so callers can name the channel types in [`JobHandle`] and
/// [`JobRunner::with_events`] without depending on `tokio` themselves.
pub use tokio::sync::{mpsc, oneshot, watch};

static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

fn runtime() -> &'static tokio::runtime::Runtime {
    RUNTIME.get_or_init(|| {
        // Time for the jobs' timeouts and polls; I/O for the network
        // providers (SFTP, object storage), whose sockets this runtime
        // drives as the one runtime of the process (QUESTIONS.md 81).
        let built = tokio::runtime::Builder::new_multi_thread()
            .thread_name("rick-rt")
            .enable_time()
            .enable_io()
            .build();
        // A process whose only runtime cannot start has nothing to run; this
        // is a startup failure of the host, not user data or filesystem
        // state, so it is the one place the kernel may stop.
        #[allow(clippy::expect_used)]
        built.expect("the tokio runtime could not be started")
    })
}

/// A handle to the process-wide `tokio` runtime, created on first use.
///
/// # Panics
///
/// Panics if the host refuses to start the runtime's threads on first use;
/// Rick cannot run without them.
#[must_use]
pub fn handle() -> tokio::runtime::Handle {
    runtime().handle().clone()
}

/// Runs a future to completion on the process-wide runtime, blocking the
/// calling thread.
///
/// # Panics
///
/// Panics when called from inside the runtime, that is from within a job or
/// another `block_on`; a job awaits instead.
pub fn block_on<F: Future>(future: F) -> F::Output {
    runtime().block_on(future)
}
