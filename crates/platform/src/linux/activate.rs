//! Raising a window on Linux (specs/system-integration.md §3): on Wayland
//! through `xdg_activation_v1` with the activation token the launcher
//! gave the second launch, on X11 through `_NET_ACTIVE_WINDOW`.
//!
//! The Wayland request goes over winit's own `wl_display`
//! (`Backend::from_foreign_display`, as the drag does) with an event queue
//! of its own, just long enough to bind the activation global and send
//! the request; without a token the compositor would ignore the request,
//! so none is sent and the caller hears why. The X11 message goes over a
//! connection of Rick's own to the root window, as the EWMH asks, with
//! the source marked as an application.

use std::io;

use raw_window_handle::{RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle};
use types::Error;
use wayland_client::backend::{Backend, ObjectId};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, delegate_noop};
use wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::XdgActivationV1;
use x11rb::connection::Connection as _;
use x11rb::protocol::xproto::{ClientMessageEvent, ConnectionExt as _, EventMask};
use x11rb::rust_connection::RustConnection;

use crate::traits::WindowRef;

/// Raises and focuses `window`, with the launcher's `token` where the
/// display is Wayland.
pub(super) fn activate(window: &WindowRef, token: Option<&str>) -> Result<(), Error> {
    match (window.display, window.window) {
        (RawDisplayHandle::Wayland(display), RawWindowHandle::Wayland(surface)) => {
            wayland(display, surface.surface.as_ptr(), token)
        }
        (
            RawDisplayHandle::Xlib(_) | RawDisplayHandle::Xcb(_),
            RawWindowHandle::Xlib(_) | RawWindowHandle::Xcb(_),
        ) => x11(window),
        _ => Err(Error::Unsupported(
            "the window is neither a Wayland surface nor an X11 window",
        )),
    }
}

/// The one global the activation needs.
struct State {
    activation: Option<XdgActivationV1>,
}

impl Dispatch<WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        (): &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name, interface, ..
        } = event
            && interface == "xdg_activation_v1"
            && state.activation.is_none()
        {
            state.activation = Some(registry.bind::<XdgActivationV1, _, _>(name, 1, qh, ()));
        }
    }
}

delegate_noop!(State: ignore XdgActivationV1);

fn wayland_error(error: impl std::fmt::Display) -> Error {
    Error::Io(io::Error::other(error.to_string()))
}

fn wayland(
    display: WaylandDisplayHandle,
    surface: *mut std::ffi::c_void,
    token: Option<&str>,
) -> Result<(), Error> {
    let Some(token) = token else {
        return Err(Error::Unsupported(
            "no activation token: on Wayland only the launcher's token raises a window",
        ));
    };
    // SAFETY: the pointer is the live `wl_display` winit gave out through
    // its raw display handle, which outlives every window of the process;
    // the backend does not take ownership (no disconnect on drop).
    let backend = unsafe { Backend::from_foreign_display(display.display.as_ptr().cast()) };
    let conn = Connection::from_backend(backend);
    let mut queue = conn.new_event_queue::<State>();
    let qh = queue.handle();
    let _registry = conn.display().get_registry(&qh, ());
    let mut state = State { activation: None };
    queue.roundtrip(&mut state).map_err(wayland_error)?;
    let activation = state.activation.ok_or(Error::Unsupported(
        "the compositor has no xdg_activation_v1",
    ))?;
    // SAFETY: the pointer is the live `wl_surface` proxy winit gave out
    // through its raw window handle for a window being drawn this frame.
    let id = unsafe { ObjectId::from_ptr(WlSurface::interface(), surface.cast()) }
        .map_err(wayland_error)?;
    let surface = WlSurface::from_id(&conn, id).map_err(wayland_error)?;
    activation.activate(token.to_owned(), &surface);
    activation.destroy();
    conn.flush().map_err(wayland_error)?;
    Ok(())
}

fn x11_error(error: impl std::fmt::Display) -> Error {
    Error::Io(io::Error::other(error.to_string()))
}

fn x11(window: &WindowRef) -> Result<(), Error> {
    let id = match window.window {
        RawWindowHandle::Xlib(handle) => u32::try_from(handle.window).ok(),
        RawWindowHandle::Xcb(handle) => Some(handle.window.get()),
        _ => None,
    }
    .ok_or(Error::Unsupported("the window is not an X11 window"))?;
    let (conn, screen) = RustConnection::connect(None).map_err(x11_error)?;
    let root = conn
        .setup()
        .roots
        .get(screen)
        .ok_or(Error::Unsupported("the X11 display has no screen"))?
        .root;
    let atom = conn
        .intern_atom(false, b"_NET_ACTIVE_WINDOW")
        .map_err(x11_error)?
        .reply()
        .map_err(x11_error)?
        .atom;
    // Source indication 1: an application; no timestamp; no window that
    // is active now.
    let event = ClientMessageEvent::new(32, id, atom, [1, 0, 0, 0, 0]);
    conn.send_event(
        false,
        root,
        EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
        event,
    )
    .map_err(x11_error)?;
    conn.flush().map_err(x11_error)?;
    Ok(())
}
