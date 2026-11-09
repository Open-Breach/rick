//! Work handed to the application from outside its windows: the command
//! line of a second launch, which the single-instance endpoint delivers on
//! a task of the runtime while the application lives on the UI thread
//! (specs/system-integration.md §3).
//!
//! The sender side gives the loop a closure over the application; the UI
//! drains the receiver between two frames, on its own thread, so the
//! closure sees the application as any command handler does and the
//! events it produces reach the windows like every other event. The waker
//! rings after every send, so a UI waiting for input repaints instead of
//! polling.

use std::sync::{Arc, Mutex, PoisonError};

use runtime::mpsc;

use crate::{App, Event, Waker};

/// Work for the application's thread: handed the application and the
/// events to fill, like a command handler.
pub type External = Box<dyn FnOnce(&mut App, &mut Vec<Event>) + Send>;

/// The sending half: cloned into whatever task delivers work.
#[derive(Clone)]
pub struct ExternalSender {
    tx: mpsc::UnboundedSender<External>,
    waker: Arc<Mutex<Option<Waker>>>,
}

impl std::fmt::Debug for ExternalSender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExternalSender").finish_non_exhaustive()
    }
}

impl ExternalSender {
    /// Queues `work` for the application's thread and rings the waker;
    /// `false` once the receiver is gone.
    pub fn send(&self, work: External) -> bool {
        if self.tx.send(work).is_err() {
            return false;
        }
        let waker = self
            .waker
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(wake) = waker {
            wake();
        }
        true
    }
}

/// The receiving half: the UI's, drained between frames.
pub struct ExternalReceiver {
    rx: mpsc::UnboundedReceiver<External>,
    waker: Arc<Mutex<Option<Waker>>>,
}

impl std::fmt::Debug for ExternalReceiver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExternalReceiver").finish_non_exhaustive()
    }
}

impl ExternalReceiver {
    /// Sets what rings after every send: the UI's repaint request.
    pub fn set_waker(&self, waker: Waker) {
        *self.waker.lock().unwrap_or_else(PoisonError::into_inner) = Some(waker);
    }

    /// Runs everything queued so far against `app`, in order, collecting
    /// the events into `events`; returns how much ran.
    pub fn drain(&mut self, app: &mut App, events: &mut Vec<Event>) -> usize {
        let mut ran = 0;
        while let Ok(work) = self.rx.try_recv() {
            work(app, events);
            ran += 1;
        }
        ran
    }
}

/// A channel for work from outside the windows.
#[must_use]
pub fn external_channel() -> (ExternalSender, ExternalReceiver) {
    let (tx, rx) = mpsc::unbounded_channel();
    let waker = Arc::new(Mutex::new(None));
    (
        ExternalSender {
            tx,
            waker: Arc::clone(&waker),
        },
        ExternalReceiver { rx, waker },
    )
}
