//! X11 drag and drop on a connection of Rick's own: an XDND source for
//! drags out of Rick, moved along every frame from the UI thread, and the
//! pointer's position over a window for drops in, which `winit` receives
//! (as hovered and dropped files) but reports without a position.
//!
//! The source side of XDND (version 5): Rick's hidden window owns the
//! `XdndSelection`; while the button is down the pointer is queried, the
//! `XdndAware` window under it gets `XdndEnter` and `XdndPosition`
//! messages and answers `XdndStatus`; on release an accepting target gets
//! `XdndDrop`, serves itself the `text/uri-list` through a selection
//! request, and ends the drag with `XdndFinished`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use raw_window_handle::RawWindowHandle;
use types::Error;
use x11rb::connection::Connection as _;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ClientMessageEvent, ConnectionExt as _, CreateWindowAux, EventMask, KeyButMask,
    PropMode, SELECTION_NOTIFY_EVENT, SelectionNotifyEvent, SelectionRequestEvent, Window,
    WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

use crate::dnd;
use crate::traits::{Point, WindowRef};

/// The XDND protocol version Rick speaks.
const XDND_VERSION: u32 = 5;
/// How long a target gets to answer an `XdndDrop` with `XdndFinished`.
const FINISH_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a target gets to answer an `XdndPosition` before the next one
/// is sent anyway.
const STATUS_TIMEOUT: Duration = Duration::from_millis(400);

x11rb::atom_manager! {
    Atoms: AtomsCookie {
        XdndAware,
        XdndEnter,
        XdndPosition,
        XdndStatus,
        XdndLeave,
        XdndDrop,
        XdndFinished,
        XdndSelection,
        XdndTypeList,
        XdndProxy,
        XdndActionCopy,
        TARGETS,
        URI_LIST: b"text/uri-list",
    }
}

/// The drag Rick is running.
struct Drag {
    /// The URI list the target reads through the selection.
    data: Vec<u8>,
    /// The `XdndAware` window under the pointer, and the XDND version it
    /// speaks.
    target: Option<(Window, u32)>,
    /// The target said it will take the drop.
    accepted: bool,
    /// An `XdndPosition` is out and its `XdndStatus` not back yet.
    status_due: Option<Instant>,
    /// The last root position sent.
    sent: Option<(i16, i16)>,
    /// The button came up and `XdndDrop` went out; waiting for
    /// `XdndFinished` until then.
    finish_by: Option<Instant>,
}

/// The connection and the hidden source window.
pub(super) struct X11 {
    conn: RustConnection,
    root: Window,
    source: Window,
    atoms: Atoms,
    drag: Option<Drag>,
}

impl X11 {
    /// Opens a connection of Rick's own to `$DISPLAY` and the hidden window
    /// drags are sent from; `None` when the server cannot be reached.
    pub(super) fn connect() -> Option<Self> {
        let (conn, screen) = RustConnection::connect(None)
            .map_err(|error| tracing::warn!(%error, "no X11 connection; no native drag"))
            .ok()?;
        let root = conn.setup().roots.get(screen)?.root;
        let atoms = Atoms::new(&conn).ok()?.reply().ok()?;
        let source = conn.generate_id().ok()?;
        conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            source,
            root,
            -10,
            -10,
            1,
            1,
            0,
            WindowClass::INPUT_ONLY,
            x11rb::COPY_FROM_PARENT,
            &CreateWindowAux::new().override_redirect(1),
        )
        .ok()?;
        conn.change_property32(
            PropMode::REPLACE,
            source,
            atoms.XdndAware,
            AtomEnum::ATOM,
            &[XDND_VERSION],
        )
        .ok()?;
        conn.flush().ok()?;
        Some(Self {
            conn,
            root,
            source,
            atoms,
            drag: None,
        })
    }

    pub(super) fn start_drag(&mut self, paths: &[PathBuf]) -> Result<(), Error> {
        let uris: Vec<PathBuf> = paths.iter().map(|p| dnd::absolute(p)).collect();
        let data = dnd::uri_list(&uris).into_bytes();
        self.conn
            .set_selection_owner(self.source, self.atoms.XdndSelection, x11rb::CURRENT_TIME)
            .and_then(|_| self.conn.flush())
            .map_err(|error| Error::Io(std::io::Error::other(error.to_string())))?;
        self.drag = Some(Drag {
            data,
            target: None,
            accepted: false,
            status_due: None,
            sent: None,
            finish_by: None,
        });
        tracing::debug!(count = paths.len(), "xdnd drag started");
        Ok(())
    }

    /// Serves the selection and moves the drag along; whether it goes on.
    pub(super) fn drag_in_progress(&mut self) -> bool {
        if self.drag.is_none() {
            return false;
        }
        self.pump_events();
        self.tick();
        let _ = self.conn.flush();
        self.drag.is_some()
    }

    /// Where the pointer is over `window`, in its logical coordinates.
    pub(super) fn drop_position(&self, window: &WindowRef) -> Option<Point> {
        let id = match window.window {
            RawWindowHandle::Xlib(handle) => u32::try_from(handle.window).ok()?,
            RawWindowHandle::Xcb(handle) => handle.window.get(),
            _ => return None,
        };
        let reply = self.conn.query_pointer(id).ok()?.reply().ok()?;
        if !reply.same_screen {
            return None;
        }
        let scale = if window.scale > 0.0 {
            window.scale
        } else {
            1.0
        };
        Some(Point {
            x: f32::from(reply.win_x) / scale,
            y: f32::from(reply.win_y) / scale,
        })
    }

    /// Handles what the server and the target sent since the last frame.
    fn pump_events(&mut self) {
        while let Ok(Some(event)) = self.conn.poll_for_event() {
            match event {
                Event::ClientMessage(message) if message.type_ == self.atoms.XdndStatus => {
                    let data = message.data.as_data32();
                    if let Some(drag) = &mut self.drag
                        && drag.target.is_some_and(|(w, _)| w == data[0])
                    {
                        drag.accepted = data[1] & 1 == 1;
                        drag.status_due = None;
                    }
                }
                Event::ClientMessage(message) if message.type_ == self.atoms.XdndFinished => {
                    let data = message.data.as_data32();
                    if self.drag.as_ref().is_some_and(|d| {
                        d.finish_by.is_some() && d.target.is_some_and(|(w, _)| w == data[0])
                    }) {
                        tracing::debug!(accepted = data[1] & 1 == 1, "xdnd drop finished");
                        self.drag = None;
                    }
                }
                Event::SelectionRequest(request) => self.serve(&request),
                _ => {}
            }
        }
    }

    /// Answers the target's request for the dragged data.
    fn serve(&self, request: &SelectionRequestEvent) {
        let property = if request.property == x11rb::NONE {
            request.target
        } else {
            request.property
        };
        let written = if request.target == self.atoms.URI_LIST {
            self.drag.as_ref().is_some_and(|drag| {
                self.conn
                    .change_property8(
                        PropMode::REPLACE,
                        request.requestor,
                        property,
                        self.atoms.URI_LIST,
                        &drag.data,
                    )
                    .is_ok()
            })
        } else if request.target == self.atoms.TARGETS {
            self.conn
                .change_property32(
                    PropMode::REPLACE,
                    request.requestor,
                    property,
                    AtomEnum::ATOM,
                    &[self.atoms.TARGETS, self.atoms.URI_LIST],
                )
                .is_ok()
        } else {
            false
        };
        let notify = SelectionNotifyEvent {
            response_type: SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: request.time,
            requestor: request.requestor,
            selection: request.selection,
            target: request.target,
            property: if written { property } else { x11rb::NONE },
        };
        let _ = self
            .conn
            .send_event(false, request.requestor, EventMask::NO_EVENT, notify);
    }

    /// One step of the source: follow the pointer, tell the target, drop on
    /// release.
    fn tick(&mut self) {
        let Some(drag) = &mut self.drag else {
            return;
        };
        if let Some(deadline) = drag.finish_by {
            if Instant::now() >= deadline {
                tracing::debug!("xdnd target never finished the drop");
                self.drag = None;
            }
            return;
        }
        let Some(pointer) = self
            .conn
            .query_pointer(self.root)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
        else {
            self.drag = None;
            return;
        };
        let held = pointer
            .mask
            .intersects(KeyButMask::BUTTON1 | KeyButMask::BUTTON3);
        let position = (pointer.root_x, pointer.root_y);
        let under = if held {
            find_target(&self.conn, &self.atoms, self.root, pointer.child, position)
        } else {
            drag.target
        };
        let (source, atoms) = (self.source, &self.atoms);
        if under.map(|(w, _)| w) != drag.target.map(|(w, _)| w) {
            if let Some((old, _)) = drag.target {
                send(&self.conn, old, atoms.XdndLeave, [source, 0, 0, 0, 0]);
            }
            drag.target = under;
            drag.accepted = false;
            drag.status_due = None;
            drag.sent = None;
            if let Some((new, version)) = under {
                let flags = version.min(XDND_VERSION) << 24;
                send(
                    &self.conn,
                    new,
                    atoms.XdndEnter,
                    [source, flags, atoms.URI_LIST, 0, 0],
                );
            }
        }
        if !held {
            // Released: drop on a willing target, leave the rest.
            if let Some((target, _)) = drag.target {
                if drag.accepted {
                    send(
                        &self.conn,
                        target,
                        atoms.XdndDrop,
                        [source, 0, x11rb::CURRENT_TIME, 0, 0],
                    );
                    drag.finish_by = Some(Instant::now() + FINISH_TIMEOUT);
                    return;
                }
                send(&self.conn, target, atoms.XdndLeave, [source, 0, 0, 0, 0]);
            }
            tracing::debug!("xdnd drag released without a target");
            self.drag = None;
            return;
        }
        let Some((target, _)) = drag.target else {
            return;
        };
        let waiting = drag.status_due.is_some_and(|due| Instant::now() < due);
        if drag.sent != Some(position) && !waiting {
            // Root coordinates packed as the protocol wants them.
            #[allow(clippy::cast_sign_loss)]
            let packed = ((position.0 as u32) << 16) | (position.1 as u32 & 0xFFFF);
            send(
                &self.conn,
                target,
                atoms.XdndPosition,
                [source, 0, packed, x11rb::CURRENT_TIME, atoms.XdndActionCopy],
            );
            drag.sent = Some(position);
            drag.status_due = Some(Instant::now() + STATUS_TIMEOUT);
        }
    }
}

/// Sends an XDND client message to `window`.
fn send(conn: &RustConnection, window: Window, kind: Atom, data: [u32; 5]) {
    let event = ClientMessageEvent::new(32, window, kind, data);
    if let Err(error) = conn.send_event(false, window, EventMask::NO_EVENT, event) {
        tracing::debug!(%error, "xdnd message not sent");
    }
}

/// The `XdndAware` window under the pointer and the version it speaks,
/// walking down from the root's child at `position` and following an
/// `XdndProxy` where one is set.
fn find_target(
    conn: &RustConnection,
    atoms: &Atoms,
    root: Window,
    child: Window,
    position: (i16, i16),
) -> Option<(Window, u32)> {
    let mut window = child;
    for _ in 0..64 {
        if window == x11rb::NONE {
            return None;
        }
        let target = proxy_of(conn, atoms, window).unwrap_or(window);
        if let Some(version) = aware_version(conn, atoms, target) {
            return Some((target, version));
        }
        let translated = conn
            .translate_coordinates(root, window, position.0, position.1)
            .ok()?
            .reply()
            .ok()?;
        window = translated.child;
    }
    None
}

/// The XDND version a window advertises, if it is `XdndAware`.
fn aware_version(conn: &RustConnection, atoms: &Atoms, window: Window) -> Option<u32> {
    let reply = conn
        .get_property(false, window, atoms.XdndAware, AtomEnum::ATOM, 0, 1)
        .ok()?
        .reply()
        .ok()?;
    reply.value32()?.next()
}

/// The window that handles XDND for `window`, when it names one.
fn proxy_of(conn: &RustConnection, atoms: &Atoms, window: Window) -> Option<Window> {
    let reply = conn
        .get_property(false, window, atoms.XdndProxy, AtomEnum::WINDOW, 0, 1)
        .ok()?
        .reply()
        .ok()?;
    let proxy = reply.value32()?.next()?;
    // A proxy names itself as its own proxy, says the protocol.
    let check = conn
        .get_property(false, proxy, atoms.XdndProxy, AtomEnum::WINDOW, 0, 1)
        .ok()?
        .reply()
        .ok()?;
    (check.value32()?.next() == Some(proxy)).then_some(proxy)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use raw_window_handle::{
        RawDisplayHandle, RawWindowHandle, XlibDisplayHandle, XlibWindowHandle,
    };
    use x11rb::connection::Connection as _;
    use x11rb::protocol::xproto::{ConnectionExt as _, CreateWindowAux, WindowClass};

    use super::X11;
    use crate::traits::WindowRef;

    /// Against a real X server, when `DISPLAY` names one (`xvfb-run` does):
    /// the connection, the hidden source window owning the selection, a
    /// drag that ends at once since no button is held, and the pointer
    /// position over a window of the test's own.
    #[test]
    fn with_a_display_the_source_connects_and_the_pointer_is_queried() {
        if std::env::var_os("DISPLAY").is_none() {
            return;
        }
        let mut x11 = X11::connect().unwrap_or_else(|| panic!("an X11 connection"));
        x11.start_drag(&[PathBuf::from("/tmp/x")])
            .unwrap_or_else(|e| panic!("{e}"));
        let owner = x11
            .conn
            .get_selection_owner(x11.atoms.XdndSelection)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .unwrap_or_else(|| panic!("the selection owner"));
        assert_eq!(owner.owner, x11.source);
        // No button is down: the first tick ends the drag.
        assert!(!x11.drag_in_progress());

        let window = x11.conn.generate_id().unwrap_or_else(|e| panic!("{e}"));
        x11.conn
            .create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                window,
                x11.root,
                0,
                0,
                200,
                100,
                0,
                WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT,
                &CreateWindowAux::new(),
            )
            .unwrap_or_else(|e| panic!("{e}"));
        x11.conn.flush().unwrap_or_else(|e| panic!("{e}"));
        let window_ref = WindowRef {
            window: RawWindowHandle::Xlib(XlibWindowHandle::new(u64::from(window))),
            display: RawDisplayHandle::Xlib(XlibDisplayHandle::new(None, 0)),
            scale: 2.0,
        };
        let position = x11
            .drop_position(&window_ref)
            .unwrap_or_else(|| panic!("a pointer position"));
        assert!(position.x.is_finite() && position.y.is_finite());
    }
}
