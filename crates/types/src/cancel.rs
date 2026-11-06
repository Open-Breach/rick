//! Cooperative cancellation.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll, Waker};

/// Cooperative cancellation shared between a job and whoever started it.
///
/// Cloning yields another handle to the same flag. Loops poll
/// [`is_cancelled`](Self::is_cancelled); async code awaits
/// [`cancelled`](Self::cancelled). Cancellation is permanent and idempotent.
///
/// ```
/// use types::CancelToken;
///
/// let token = CancelToken::new();
/// let worker = token.clone();
/// assert!(!worker.is_cancelled());
/// token.cancel();
/// assert!(worker.is_cancelled());
/// ```
#[derive(Clone, Default)]
pub struct CancelToken(Arc<Inner>);

#[derive(Default)]
struct Inner {
    cancelled: AtomicBool,
    wakers: Mutex<Vec<Waker>>,
}

impl CancelToken {
    /// A fresh, not yet cancelled token.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Cancels the token and wakes every task waiting on
    /// [`cancelled`](Self::cancelled). Calling it again does nothing.
    pub fn cancel(&self) {
        if self.0.cancelled.swap(true, Ordering::SeqCst) {
            return;
        }
        let wakers = std::mem::take(&mut *self.lock_wakers());
        for waker in wakers {
            waker.wake();
        }
    }

    /// `true` once [`cancel`](Self::cancel) has been called on any clone.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::SeqCst)
    }

    /// Resolves when the token is cancelled. The future owns a handle to the
    /// token, so it can be moved into a spawned task or a `select!`.
    pub fn cancelled(&self) -> Cancelled {
        Cancelled(self.clone())
    }

    fn lock_wakers(&self) -> std::sync::MutexGuard<'_, Vec<Waker>> {
        // A poisoned lock only means another thread panicked while holding a
        // plain `Vec<Waker>`; the vector itself is still usable.
        self.0.wakers.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl fmt::Debug for CancelToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CancelToken")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

/// Future returned by [`CancelToken::cancelled`].
#[must_use = "futures do nothing unless polled"]
pub struct Cancelled(CancelToken);

impl Future for Cancelled {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.0.is_cancelled() {
            return Poll::Ready(());
        }
        let mut wakers = self.0.lock_wakers();
        // Re-check under the lock: `cancel` sets the flag before draining the
        // list, so a cancellation that landed in between is seen here and one
        // that lands later finds our waker in the list.
        if self.0.is_cancelled() {
            return Poll::Ready(());
        }
        if !wakers.iter().any(|w| w.will_wake(cx.waker())) {
            wakers.push(cx.waker().clone());
        }
        Poll::Pending
    }
}

impl fmt::Debug for Cancelled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Cancelled").field(&self.0).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::task::Wake;

    struct CountingWaker(AtomicUsize);

    impl Wake for CountingWaker {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn cancel_is_visible_through_clones_and_idempotent() {
        let token = CancelToken::new();
        let clone = token.clone();
        assert!(!clone.is_cancelled());
        token.cancel();
        token.cancel();
        assert!(clone.is_cancelled());
        assert_eq!(format!("{token:?}"), "CancelToken { cancelled: true }");
    }

    #[test]
    fn cancelled_future_is_pending_then_ready_and_wakes_once() {
        let token = CancelToken::new();
        let waker_impl = Arc::new(CountingWaker(AtomicUsize::new(0)));
        let waker = Waker::from(waker_impl.clone());
        let mut cx = Context::from_waker(&waker);
        let mut fut = token.cancelled();

        assert_eq!(Pin::new(&mut fut).poll(&mut cx), Poll::Pending);
        assert_eq!(Pin::new(&mut fut).poll(&mut cx), Poll::Pending);
        assert_eq!(
            token.lock_wakers().len(),
            1,
            "same waker is not stored twice"
        );

        token.cancel();
        assert_eq!(waker_impl.0.load(Ordering::SeqCst), 1);
        assert_eq!(Pin::new(&mut fut).poll(&mut cx), Poll::Ready(()));
        assert!(token.lock_wakers().is_empty());
    }

    #[test]
    fn already_cancelled_token_resolves_immediately() {
        let token = CancelToken::new();
        token.cancel();
        let mut cx = Context::from_waker(Waker::noop());
        assert_eq!(
            Pin::new(&mut token.cancelled()).poll(&mut cx),
            Poll::Ready(())
        );
    }

    #[test]
    fn concurrent_cancel_wakes_waiters_on_other_threads() {
        let token = CancelToken::new();
        let waiters: Vec<_> = (0..8)
            .map(|_| {
                let token = token.clone();
                std::thread::spawn(move || {
                    let waker_impl = Arc::new(CountingWaker(AtomicUsize::new(0)));
                    let waker = Waker::from(waker_impl.clone());
                    let mut cx = Context::from_waker(&waker);
                    let mut fut = token.cancelled();
                    while Pin::new(&mut fut).poll(&mut cx).is_pending() {
                        std::thread::yield_now();
                    }
                    waker_impl.0.load(Ordering::SeqCst) <= 1
                })
            })
            .collect();
        std::thread::sleep(std::time::Duration::from_millis(10));
        token.cancel();
        for waiter in waiters {
            assert!(
                waiter
                    .join()
                    .is_ok_and(|woke_at_most_once| woke_at_most_once)
            );
        }
    }
}
