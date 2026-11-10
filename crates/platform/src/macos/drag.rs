//! macOS drag and drop through `AppKit`: an `NSDraggingSession` begun from
//! the window's content view with the dragged files as `NSURL` pasteboard
//! items, and the mouse location converted into the view for drops winit
//! reports without a position.
//!
//! `AppKit` runs the session itself once begun and tells the source when it
//! ended through `draggingSession:endedAtPoint:operation:`; the UI polls
//! for that, since the view never sees the mouse come up.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::ptr::NonNull;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{
    AnyThread as _, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
};
use objc2_app_kit::{
    NSApplication, NSDragOperation, NSDraggingContext, NSDraggingItem, NSDraggingSession,
    NSDraggingSource, NSEvent, NSEventType, NSView, NSWorkspace,
};
use objc2_foundation::{NSArray, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};
use raw_window_handle::RawWindowHandle;
use types::Error;

use crate::traits::{Point, WindowRef};

/// The edge of the icon shown under the cursor, in points.
const ICON_EDGE: f64 = 32.0;

/// What the source remembers of its session.
#[derive(Debug, Default)]
struct Ivars {
    /// `AppKit` said the session ended.
    ended: Cell<bool>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements and the class does
    // not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "RickDragSource"]
    #[ivars = Ivars]
    struct DragSource;

    unsafe impl NSObjectProtocol for DragSource {}

    unsafe impl NSDraggingSource for DragSource {
        #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
        fn operation_mask(
            &self,
            _session: &NSDraggingSession,
            _context: NSDraggingContext,
        ) -> NSDragOperation {
            // The receiver decides, as the spec's table says; Finder moves on
            // the same volume and copies across, like Rick itself.
            NSDragOperation::Copy
                | NSDragOperation::Move
                | NSDragOperation::Link
                | NSDragOperation::Generic
        }

        #[unsafe(method(draggingSession:endedAtPoint:operation:))]
        fn ended(
            &self,
            _session: &NSDraggingSession,
            _screen_point: NSPoint,
            operation: NSDragOperation,
        ) {
            tracing::debug!(?operation, "macos drag ended");
            self.ivars().ended.set(true);
        }
    }
);

impl DragSource {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars::default());
        // SAFETY: `init` of `NSObject` on a freshly allocated instance.
        unsafe { msg_send![super(this), init] }
    }
}

thread_local! {
    /// The source of the session in flight, kept alive until it ends.
    static SESSION: RefCell<Option<Retained<DragSource>>> = const { RefCell::new(None) };
}

/// The view of a window handle, for the main thread.
pub(super) fn view_of(
    window: &WindowRef,
    mtm: MainThreadMarker,
) -> Result<Retained<NSView>, Error> {
    let RawWindowHandle::AppKit(handle) = window.window else {
        return Err(Error::Unsupported("the window is not an AppKit view"));
    };
    let ptr: NonNull<NSView> = handle.ns_view.cast();
    let _ = mtm;
    // SAFETY: the pointer is the live `NSView` winit gave out through its
    // raw window handle for a window being drawn this frame, on the main
    // thread, which `mtm` proves.
    unsafe { Retained::retain(ptr.as_ptr()) }.ok_or(Error::Unsupported("the window's view is gone"))
}

/// Begins the session: one `NSDraggingItem` per path with the file's icon
/// under the cursor, the press event when `AppKit` still has it or a drag
/// event made for the current mouse location.
pub(super) fn start_drag(window: &WindowRef, paths: &[PathBuf]) -> Result<(), Error> {
    let mtm =
        MainThreadMarker::new().ok_or(Error::Unsupported("drags start on the main thread"))?;
    let view = view_of(window, mtm)?;
    let ns_window = view
        .window()
        .ok_or(Error::Unsupported("the view is not in a window"))?;
    let app = NSApplication::sharedApplication(mtm);
    let event = match app.currentEvent() {
        Some(event)
            if matches!(
                event.r#type(),
                NSEventType::LeftMouseDown
                    | NSEventType::LeftMouseDragged
                    | NSEventType::RightMouseDown
                    | NSEventType::RightMouseDragged
            ) =>
        {
            event
        }
        _ => {
            let location = ns_window.convertPointFromScreen(NSEvent::mouseLocation());
            // A drag event at the mouse, with the nil graphics context the
            // deprecated parameter asks for.
            NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                NSEventType::LeftMouseDragged,
                location,
                NSEvent::modifierFlags_class(),
                0.0,
                ns_window.windowNumber(),
                None,
                0,
                1,
                1.0,
            )
            .ok_or(Error::Unsupported("no drag event could be made"))?
        }
    };
    let in_view = view.convertPoint_fromView(event.locationInWindow(), None);
    let frame = NSRect::new(
        NSPoint::new(in_view.x - ICON_EDGE / 2.0, in_view.y - ICON_EDGE / 2.0),
        NSSize::new(ICON_EDGE, ICON_EDGE),
    );
    let workspace = NSWorkspace::sharedWorkspace();
    let mut items = Vec::with_capacity(paths.len());
    for (index, path) in paths.iter().enumerate() {
        let path = crate::dnd::absolute(path);
        let text = NSString::from_str(&path.to_string_lossy());
        let url = objc2_foundation::NSURL::fileURLWithPath(&text);
        let item = NSDraggingItem::initWithPasteboardWriter(
            NSDraggingItem::alloc(),
            ProtocolObject::from_ref(&*url),
        );
        // The first item carries the icon; the rest sit under it unseen.
        let icon = (index == 0).then(|| {
            let icon = workspace.iconForFile(&text);
            icon.setSize(NSSize::new(ICON_EDGE, ICON_EDGE));
            icon
        });
        let contents: Option<&AnyObject> = icon.as_deref().map(AsRef::as_ref);
        // SAFETY: the frame is in the view's coordinates and the contents,
        // when given, is an `NSImage`, which the method accepts.
        unsafe { item.setDraggingFrame_contents(frame, contents) };
        items.push(item);
    }
    if items.is_empty() {
        return Err(Error::Unsupported("nothing to drag"));
    }
    let items = NSArray::from_retained_slice(&items);
    let source = DragSource::new(mtm);
    // The source outlives the session through `SESSION`.
    let session = view.beginDraggingSessionWithItems_event_source(
        &items,
        &event,
        ProtocolObject::from_ref(&*source),
    );
    session.setAnimatesToStartingPositionsOnCancelOrFail(true);
    SESSION.with(|slot| *slot.borrow_mut() = Some(source));
    tracing::debug!(count = paths.len(), "macos drag started");
    Ok(())
}

/// Whether the session begun by [`start_drag`] is still running.
pub(super) fn drag_in_progress() -> bool {
    SESSION.with(|slot| {
        let mut slot = slot.borrow_mut();
        match slot.as_ref() {
            Some(source) if source.ivars().ended.get() => {
                *slot = None;
                false
            }
            Some(_) => true,
            None => false,
        }
    })
}

/// The mouse location in the view's coordinates, from the top-left corner
/// as winit's flipped view counts.
pub(super) fn drop_position(window: &WindowRef) -> Option<Point> {
    let mtm = MainThreadMarker::new()?;
    let view = view_of(window, mtm).ok()?;
    let ns_window = view.window()?;
    let in_window = ns_window.convertPointFromScreen(NSEvent::mouseLocation());
    let mut in_view = view.convertPoint_fromView(in_window, None);
    if !view.isFlipped() {
        in_view.y = view.bounds().size.height - in_view.y;
    }
    // Points fit f32 with room to spare.
    #[allow(clippy::cast_possible_truncation)]
    Some(Point {
        x: in_view.x as f32,
        y: in_view.y as f32,
    })
}
