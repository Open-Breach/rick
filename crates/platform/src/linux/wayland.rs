//! Wayland drag and drop over winit's own `wl_display`: a `wl_data_source`
//! carrying `text/uri-list` for drags out of Rick, and a `wl_data_device` of
//! Rick's own for drags in, since `winit` 0.30 has no drop target on
//! Wayland.
//!
//! The connection is `wayland-client`'s view of the display `winit` opened
//! (`Backend::from_foreign_display`), with an event queue of its own, so
//! `winit`'s dispatching is untouched. The queue is read on a blocking
//! thread of the runtime, next to `winit`'s reader (libwayland lets every
//! reader prepare, poll and read in turn), which wakes the UI when a drag
//! has news; the UI thread sends the requests and reads the shared state.
//! Rick binds its own `wl_seat` and `wl_pointer` to see the serial of the
//! button press a drag needs, which `winit` keeps to itself.

use std::collections::HashMap;
use std::io;
use std::os::fd::{AsFd as _, OwnedFd};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use raw_window_handle::{RawWindowHandle, WaylandDisplayHandle};
use types::Error;
use wayland_client::backend::{Backend, ObjectId};
use wayland_client::protocol::wl_data_device::{self, WlDataDevice};
use wayland_client::protocol::wl_data_device_manager::{DndAction, WlDataDeviceManager};
use wayland_client::protocol::wl_data_offer::{self, WlDataOffer};
use wayland_client::protocol::wl_data_source::{self, WlDataSource};
use wayland_client::protocol::wl_pointer::{self, ButtonState, WlPointer};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_seat::{self, WlSeat};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, delegate_noop, event_created_child,
};

use crate::dnd;
use crate::traits::{IncomingFiles, Point, WindowRef};

/// The one mime type Rick offers and takes.
const URI_LIST: &str = "text/uri-list";
/// The data device manager version Rick asks for: 3 brings `set_actions`,
/// `finish` and the drop-performed and finished events.
const MANAGER_VERSION: u32 = 3;
const SEAT_VERSION: u32 = 5;

type Wake = Arc<dyn Fn() + Send + Sync>;

/// What the reader thread and the UI thread share.
#[derive(Default)]
struct Shared {
    manager: Option<WlDataDeviceManager>,
    device: Option<WlDataDevice>,
    /// The serial of the last button press on Rick's windows: the implicit
    /// grab a drag starts from.
    press_serial: Option<u32>,
    /// The drag Rick started, until the compositor says it ended.
    outgoing: Option<Outgoing>,
    /// The drag from outside over one of Rick's surfaces.
    incoming: Option<Incoming>,
    /// Files dropped since the UI last asked.
    dropped: Vec<PathBuf>,
}

/// The reader thread's state: the shared part, the UI's waker, and what
/// only the dispatchers need.
struct State {
    shared: Arc<Mutex<Shared>>,
    wake: Wake,
    seat: Option<WlSeat>,
    pointer: Option<WlPointer>,
    /// Offers the compositor announced, with their mime types, until an
    /// enter or a selection says what they are for.
    offers: HashMap<ObjectId, Vec<String>>,
    /// A dispatched event changed something the UI should see.
    changed: bool,
}

struct Outgoing {
    source: WlDataSource,
    /// The URI list, shared with the blocking write that answers `send`.
    data: Arc<Vec<u8>>,
}

struct Incoming {
    offer: WlDataOffer,
    /// The surface the drag is over, to answer positions for that window.
    surface: ObjectId,
    position: Option<Point>,
    /// The offer carries a URI list Rick accepted.
    accepted: bool,
    /// The URI list being read from the source, through a pipe.
    reading: Option<Reading>,
    paths: Vec<PathBuf>,
    /// The drop happened; the files are reported once they are read.
    dropped: bool,
}

/// A pipe the source writes the URI list into, read without blocking.
struct Reading {
    fd: OwnedFd,
    buffer: Vec<u8>,
}

impl Reading {
    /// Reads what is there; `Some(bytes)` once the source closed its end.
    fn poll(&mut self) -> Option<Vec<u8>> {
        let mut chunk = [0u8; 4096];
        loop {
            match rustix::io::read(&self.fd, &mut chunk) {
                Ok(0) => return Some(std::mem::take(&mut self.buffer)),
                Ok(n) => self.buffer.extend_from_slice(&chunk[..n]),
                Err(rustix::io::Errno::AGAIN | rustix::io::Errno::INTR) => return None,
                Err(error) => {
                    tracing::debug!(%error, "reading the dropped URI list failed");
                    return Some(std::mem::take(&mut self.buffer));
                }
            }
        }
    }
}

fn lock(shared: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    // The state is plain data; a panic elsewhere leaves it usable.
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The UI thread's handle: the connection to send requests on and the
/// state the reader fills.
pub(super) struct Wayland {
    conn: Connection,
    qh: QueueHandle<State>,
    shared: Arc<Mutex<Shared>>,
}

impl Wayland {
    /// Joins winit's display, binds the seat, the pointer and the data
    /// device, and starts the reader; `None` when the display cannot be
    /// joined or has no seat or data device manager.
    pub(super) fn connect(display: WaylandDisplayHandle, wake: Wake) -> Option<Self> {
        // SAFETY: the pointer is the live `wl_display` winit gave out through
        // its raw display handle, which outlives every window of the process;
        // the backend does not take ownership (no disconnect on drop).
        let backend = unsafe { Backend::from_foreign_display(display.display.as_ptr().cast()) };
        let conn = Connection::from_backend(backend);
        let mut queue = conn.new_event_queue::<State>();
        let qh = queue.handle();
        let shared = Arc::new(Mutex::new(Shared::default()));
        let mut state = State {
            shared: Arc::clone(&shared),
            wake,
            seat: None,
            pointer: None,
            offers: HashMap::new(),
            changed: false,
        };
        let _registry = conn.display().get_registry(&qh, ());
        // Globals, then the seat's capabilities and the device.
        for _ in 0..2 {
            if let Err(error) = queue.roundtrip(&mut state) {
                tracing::warn!(%error, "wayland roundtrip failed; no native drag");
                return None;
            }
        }
        if lock(&shared).device.is_none() {
            tracing::warn!("no wl_data_device_manager or wl_seat; no native drag");
            return None;
        }
        runtime::handle().spawn_blocking(move || read_forever(queue, state));
        Some(Self { conn, qh, shared })
    }

    /// Reads any URI list under way and finishes a drop whose files are in.
    fn poll(&self) {
        let mut shared = lock(&self.shared);
        if let Some(incoming) = &mut shared.incoming
            && let Some(reading) = &mut incoming.reading
            && let Some(bytes) = reading.poll()
        {
            incoming.reading = None;
            incoming.paths = dnd::parse_uri_list(&bytes);
        }
        if shared
            .incoming
            .as_ref()
            .is_some_and(|i| i.dropped && i.reading.is_none())
            && let Some(incoming) = shared.incoming.take()
        {
            if incoming.accepted {
                if incoming.offer.version() >= 3 {
                    incoming.offer.finish();
                }
                shared.dropped.extend(incoming.paths);
            }
            incoming.offer.destroy();
            drop(shared);
            let _ = self.conn.flush();
        }
    }

    pub(super) fn start_drag(&self, window: &WindowRef, paths: &[PathBuf]) -> Result<(), Error> {
        let RawWindowHandle::Wayland(handle) = window.window else {
            return Err(Error::Unsupported("the window is not a Wayland surface"));
        };
        let origin = surface_of(&self.conn, handle.surface.as_ptr())
            .ok_or(Error::Unsupported("the window's wl_surface is not known"))?;
        let mut shared = lock(&self.shared);
        let (Some(manager), Some(device)) = (&shared.manager, &shared.device) else {
            return Err(Error::Unsupported("no Wayland data device"));
        };
        let serial = shared
            .press_serial
            .ok_or(Error::Unsupported("no button press seen on this seat"))?;
        let uris: Vec<PathBuf> = paths.iter().map(|p| dnd::absolute(p)).collect();
        let source = manager.create_data_source(&self.qh, ());
        source.offer(URI_LIST.to_owned());
        if source.version() >= 3 {
            source.set_actions(DndAction::Copy | DndAction::Move);
        }
        device.start_drag(Some(&source), &origin, None, serial);
        if let Some(previous) = shared.outgoing.replace(Outgoing {
            source,
            data: Arc::new(dnd::uri_list(&uris).into_bytes()),
        }) {
            previous.source.destroy();
        }
        drop(shared);
        self.conn
            .flush()
            .map_err(|error| Error::Io(io::Error::other(error.to_string())))?;
        tracing::debug!(serial, count = paths.len(), "wayland drag started");
        Ok(())
    }

    pub(super) fn drag_in_progress(&self) -> bool {
        lock(&self.shared).outgoing.is_some()
    }

    pub(super) fn incoming_files(&self) -> IncomingFiles {
        self.poll();
        let mut shared = lock(&self.shared);
        IncomingFiles {
            hovering: shared.incoming.is_some(),
            hovered: shared
                .incoming
                .as_ref()
                .map(|i| i.paths.clone())
                .unwrap_or_default(),
            dropped: std::mem::take(&mut shared.dropped),
        }
    }

    pub(super) fn drop_position(&self, window: &WindowRef) -> Option<Point> {
        let RawWindowHandle::Wayland(handle) = window.window else {
            return None;
        };
        let surface = surface_of(&self.conn, handle.surface.as_ptr())?;
        let shared = lock(&self.shared);
        let incoming = shared.incoming.as_ref()?;
        (incoming.surface.protocol_id() == surface.id().protocol_id())
            .then_some(incoming.position)
            .flatten()
    }
}

/// The reader: dispatches Rick's queue as events come, for the life of the
/// display, and wakes the UI when a drag has news.
fn read_forever(mut queue: EventQueue<State>, mut state: State) {
    loop {
        match queue.blocking_dispatch(&mut state) {
            Ok(_) => {
                if std::mem::take(&mut state.changed) {
                    (state.wake)();
                }
            }
            Err(error) => {
                tracing::warn!(%error, "wayland reader stopped");
                return;
            }
        }
    }
}

/// winit's `wl_surface` as a proxy Rick can name in requests.
fn surface_of(conn: &Connection, ptr: *mut std::ffi::c_void) -> Option<WlSurface> {
    // SAFETY: the pointer is the live `wl_surface` proxy winit gave out
    // through its raw window handle for a window being drawn this frame.
    let id = unsafe { ObjectId::from_ptr(WlSurface::interface(), ptr.cast()) }.ok()?;
    WlSurface::from_id(conn, id).ok()
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
            name,
            interface,
            version,
        } = event
        {
            let mut shared = lock(&state.shared);
            match interface.as_str() {
                "wl_seat" if state.seat.is_none() => {
                    state.seat = Some(registry.bind::<WlSeat, _, _>(
                        name,
                        version.min(SEAT_VERSION),
                        qh,
                        (),
                    ));
                }
                "wl_data_device_manager" if shared.manager.is_none() => {
                    shared.manager = Some(registry.bind::<WlDataDeviceManager, _, _>(
                        name,
                        version.min(MANAGER_VERSION),
                        qh,
                        (),
                    ));
                }
                _ => return,
            }
            if let (Some(seat), Some(manager), None) =
                (&state.seat, &shared.manager, &shared.device)
            {
                let device = manager.get_data_device(seat, qh, ());
                shared.device = Some(device);
            }
        }
    }
}

impl Dispatch<WlSeat, ()> for State {
    fn event(
        state: &mut Self,
        seat: &WlSeat,
        event: wl_seat::Event,
        (): &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(capabilities),
        } = event
        {
            let has_pointer = capabilities.contains(wl_seat::Capability::Pointer);
            match (&state.pointer, has_pointer) {
                (None, true) => state.pointer = Some(seat.get_pointer(qh, ())),
                (Some(pointer), false) => {
                    pointer.release();
                    state.pointer = None;
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<WlPointer, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlPointer,
        event: wl_pointer::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_pointer::Event::Button {
            serial,
            state: WEnum::Value(ButtonState::Pressed),
            ..
        } = event
        {
            lock(&state.shared).press_serial = Some(serial);
        }
    }
}

impl Dispatch<WlDataDevice, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlDataDevice,
        event: wl_data_device::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_device::Event::DataOffer { id } => {
                state.offers.insert(id.id(), Vec::new());
            }
            wl_data_device::Event::Enter {
                serial,
                surface,
                x,
                y,
                id: Some(offer),
            } => {
                let mimes = state.offers.remove(&offer.id()).unwrap_or_default();
                let accepted = mimes.iter().any(|m| m == URI_LIST);
                let reading = if accepted {
                    offer.accept(serial, Some(URI_LIST.to_owned()));
                    if offer.version() >= 3 {
                        offer.set_actions(DndAction::Copy | DndAction::Move, DndAction::Copy);
                    }
                    receive(&offer)
                } else {
                    offer.accept(serial, None);
                    None
                };
                let mut shared = lock(&state.shared);
                if let Some(previous) = shared.incoming.take() {
                    previous.offer.destroy();
                }
                shared.incoming = Some(Incoming {
                    offer,
                    surface: surface.id(),
                    position: Some(point(x, y)),
                    accepted,
                    reading,
                    paths: Vec::new(),
                    dropped: false,
                });
                state.changed = true;
            }
            wl_data_device::Event::Motion { x, y, .. } => {
                if let Some(incoming) = &mut lock(&state.shared).incoming {
                    incoming.position = Some(point(x, y));
                    state.changed = true;
                }
            }
            wl_data_device::Event::Leave => {
                if let Some(incoming) = lock(&state.shared).incoming.take() {
                    incoming.offer.destroy();
                    state.changed = true;
                }
            }
            wl_data_device::Event::Drop => {
                if let Some(incoming) = &mut lock(&state.shared).incoming {
                    incoming.dropped = true;
                    state.changed = true;
                }
            }
            wl_data_device::Event::Selection { id: Some(offer) } => {
                // Rick's file clipboard is its own; the system selection is
                // not taken this way.
                state.offers.remove(&offer.id());
                offer.destroy();
            }
            _ => {}
        }
    }

    event_created_child!(State, WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, ()),
    ]);
}

/// Asks the source for the URI list through a pipe Rick reads without
/// blocking; `None` when no pipe can be made.
fn receive(offer: &WlDataOffer) -> Option<Reading> {
    let (read, write) = rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC).ok()?;
    if let Err(error) = rustix::io::ioctl_fionbio(&read, true) {
        tracing::debug!(%error, "pipe cannot be made non-blocking");
        return None;
    }
    offer.receive(URI_LIST.to_owned(), write.as_fd());
    drop(write);
    Some(Reading {
        fd: read,
        buffer: Vec::new(),
    })
}

/// Surface coordinates are logical already, as egui's points are.
fn point(x: f64, y: f64) -> Point {
    // The window is far smaller than f32's range.
    #[allow(clippy::cast_possible_truncation)]
    Point {
        x: x as f32,
        y: y as f32,
    }
}

impl Dispatch<WlDataOffer, ()> for State {
    fn event(
        state: &mut Self,
        offer: &WlDataOffer,
        event: wl_data_offer::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_data_offer::Event::Offer { mime_type } = event
            && let Some(mimes) = state.offers.get_mut(&offer.id())
        {
            mimes.push(mime_type);
        }
    }
}

impl Dispatch<WlDataSource, ()> for State {
    fn event(
        state: &mut Self,
        source: &WlDataSource,
        event: wl_data_source::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let mut shared = lock(&state.shared);
        let ours = shared
            .outgoing
            .as_ref()
            .is_some_and(|o| o.source.id() == source.id());
        match event {
            wl_data_source::Event::Send { mime_type, fd } if ours => {
                if mime_type != URI_LIST {
                    return;
                }
                let data = shared
                    .outgoing
                    .as_ref()
                    .map(|o| Arc::clone(&o.data))
                    .unwrap_or_default();
                // The receiver reads the pipe at its own pace; the write
                // waits for it off this thread.
                runtime::handle().spawn_blocking(move || write_all(&fd, &data));
            }
            wl_data_source::Event::Cancelled | wl_data_source::Event::DndFinished => {
                if ours {
                    tracing::debug!(?event, "wayland drag ended");
                    if let Some(outgoing) = shared.outgoing.take() {
                        outgoing.source.destroy();
                    }
                    state.changed = true;
                } else {
                    source.destroy();
                }
            }
            _ => {}
        }
    }
}

fn write_all(fd: &OwnedFd, mut data: &[u8]) {
    while !data.is_empty() {
        match rustix::io::write(fd, data) {
            Ok(0) => return,
            Ok(n) => data = &data[n..],
            Err(rustix::io::Errno::INTR | rustix::io::Errno::AGAIN) => {
                std::thread::yield_now();
            }
            Err(error) => {
                tracing::debug!(%error, "writing the dragged URI list failed");
                return;
            }
        }
    }
}

delegate_noop!(State: ignore WlDataDeviceManager);
