//! The Linux drag source and drop target: Wayland or X11, chosen by the
//! display the window is on. Both are driven from the UI thread, which
//! polls them every frame while a drag is in flight; the Wayland reader is
//! the one thread of its own, a blocking thread of the runtime that wakes
//! the UI when a drag has news.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Arc;

use raw_window_handle::RawDisplayHandle;
use types::Error;

use super::{wayland, x11};
use crate::traits::{IncomingFiles, Point, WindowRef};

/// One of the two display servers, connected on first use.
enum Backend {
    Wayland(wayland::Wayland),
    /// Boxed: the X11 connection is large next to the Wayland handle.
    X11(Box<x11::X11>),
}

thread_local! {
    /// The connection of the UI thread, the only thread that drags.
    static BACKEND: RefCell<Option<Backend>> = const { RefCell::new(None) };
}

/// Runs `f` on the connection made by [`attach`], if there is one.
fn with_backend<T>(f: impl FnOnce(&mut Backend) -> T) -> Option<T> {
    BACKEND.with(|cell| cell.borrow_mut().as_mut().map(f))
}

/// Connects to the display `window` is on, once; `wake` is the UI's waker
/// for the Wayland reader.
pub(super) fn attach(window: &WindowRef, wake: Arc<dyn Fn() + Send + Sync>) {
    BACKEND.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_some() {
            return;
        }
        *slot = match window.display {
            RawDisplayHandle::Wayland(display) => {
                wayland::Wayland::connect(display, wake).map(Backend::Wayland)
            }
            RawDisplayHandle::Xlib(_) | RawDisplayHandle::Xcb(_) => {
                x11::X11::connect().map(|x11| Backend::X11(Box::new(x11)))
            }
            _ => None,
        };
        tracing::debug!(connected = slot.is_some(), "drag backend attached");
    });
}

pub(super) fn start_drag(window: &WindowRef, paths: &[PathBuf]) -> Result<(), Error> {
    with_backend(|backend| match backend {
        Backend::Wayland(wayland) => wayland.start_drag(window, paths),
        Backend::X11(x11) => x11.start_drag(paths),
    })
    .unwrap_or(Err(Error::Unsupported(
        "no Wayland or X11 display connection for the window; was it attached?",
    )))
}

pub(super) fn drag_in_progress() -> bool {
    with_backend(|backend| match backend {
        Backend::Wayland(wayland) => wayland.drag_in_progress(),
        Backend::X11(x11) => x11.drag_in_progress(),
    })
    .unwrap_or(false)
}

pub(super) fn incoming_files() -> IncomingFiles {
    with_backend(|backend| match backend {
        Backend::Wayland(wayland) => wayland.incoming_files(),
        Backend::X11(_) => IncomingFiles::default(),
    })
    .unwrap_or_default()
}

pub(super) fn drop_position(window: &WindowRef) -> Option<Point> {
    with_backend(|backend| match backend {
        Backend::Wayland(wayland) => wayland.drop_position(window),
        Backend::X11(x11) => x11.drop_position(window),
    })
    .flatten()
}
