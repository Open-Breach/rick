//! The `rayon` pool for CPU-bound work: sorting, hashing, decoding.
//!
//! The runtime's worker threads are for I/O and coordination. Anything that
//! keeps a core busy for more than a moment runs here, either as one closure
//! through [`run`] or as a parallel iterator inside [`pool().install`](rayon::ThreadPool::install).
//!
//! ```
//! use rayon::prelude::*;
//!
//! let sum = runtime::block_on(runtime::compute::run(|| (1..=100u64).sum::<u64>()));
//! assert_eq!(sum, 5050);
//!
//! let max = runtime::compute::pool().install(|| (1..=100u64).into_par_iter().max());
//! assert_eq!(max, Some(100));
//! ```

use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::pin::Pin;
use std::sync::OnceLock;
use std::task::{Context, Poll};

use tokio::sync::oneshot;

static POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();

/// The process-wide compute pool, created on first use with one thread per
/// available core.
///
/// # Panics
///
/// Panics if the host refuses to start the pool's threads on first use, the
/// same way the tokio runtime does: Rick cannot run without either.
pub fn pool() -> &'static rayon::ThreadPool {
    POOL.get_or_init(|| {
        let built = rayon::ThreadPoolBuilder::new()
            .thread_name(|index| format!("rick-compute-{index}"))
            .panic_handler(|payload| {
                let message = payload
                    .downcast_ref::<&str>()
                    .map(ToString::to_string)
                    .or_else(|| payload.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "non-string panic payload".to_owned());
                tracing::error!(target: "runtime::compute", %message, "compute task panicked");
            })
            .build();
        // Same reasoning as the tokio runtime: a host that cannot start the
        // pool's threads cannot run Rick at all.
        #[allow(clippy::expect_used)]
        built.expect("the compute pool could not be started")
    })
}

/// Runs `f` on the compute pool and resolves with its result.
///
/// If `f` panics, the panic is re-raised in the awaiting task, exactly as
/// `rayon::join` and `std::thread::JoinHandle::join().unwrap()` would.
pub fn run<F, T>(f: F) -> Compute<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let (tx, rx) = oneshot::channel();
    pool().spawn(move || {
        let outcome = catch_unwind(AssertUnwindSafe(f));
        // The receiver may be gone: the awaiting job was cancelled and dropped
        // its handle. The work is done either way; nothing to report.
        let _ = tx.send(outcome);
    });
    Compute(rx)
}

/// Future returned by [`run`].
#[must_use = "futures do nothing unless polled"]
pub struct Compute<T>(oneshot::Receiver<std::thread::Result<T>>);

impl<T> Future for Compute<T> {
    type Output = T;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        match Pin::new(&mut self.0).poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Ok(Ok(value))) => Poll::Ready(value),
            Poll::Ready(Ok(Err(payload))) => resume_unwind(payload),
            // Only reachable if the pool dropped the closure without running
            // it, which rayon does not do for a live pool; surface it as the
            // bug it would be rather than hang forever.
            Poll::Ready(Err(_)) => resume_unwind(Box::new("compute task was dropped unrun")),
        }
    }
}

impl<T> std::fmt::Debug for Compute<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Compute")
    }
}

#[cfg(test)]
mod tests {
    use rayon::prelude::*;

    #[test]
    fn run_executes_off_the_runtime_threads() {
        let name = crate::block_on(crate::compute::run(|| {
            std::thread::current().name().map(ToOwned::to_owned)
        }));
        assert!(
            name.as_deref()
                .is_some_and(|n| n.starts_with("rick-compute-")),
            "{name:?}"
        );
    }

    #[test]
    fn pool_runs_parallel_iterators() {
        let total: u64 = crate::compute::pool().install(|| (0..10_000u64).into_par_iter().sum());
        assert_eq!(total, 49_995_000);
    }

    #[test]
    fn many_concurrent_runs_all_resolve() {
        let results = crate::block_on(async {
            let futures: Vec<_> = (0..256u32)
                .map(|i| crate::compute::run(move || i * i))
                .collect();
            let mut out = Vec::new();
            for f in futures {
                out.push(f.await);
            }
            out
        });
        assert_eq!(results, (0..256u32).map(|i| i * i).collect::<Vec<_>>());
    }

    #[test]
    fn panics_propagate_to_the_awaiting_task() {
        let caught = std::panic::catch_unwind(|| {
            crate::block_on(crate::compute::run(|| -> u32 { panic!("deliberate") }))
        });
        assert!(caught.is_err());
    }
}
