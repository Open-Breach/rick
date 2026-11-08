//! Windows drag and drop through OLE: `DoDragDrop` with an `IDataObject`
//! carrying the dragged files as a `CF_HDROP`, and the cursor position over
//! the window for drops winit reports without one.
//!
//! `DoDragDrop` runs the drag's own message loop on the calling thread and
//! returns when the drop has happened or the drag was cancelled, so the
//! frame that starts it ends when the drag does; winit buffers the window's
//! messages meanwhile. The thread already has OLE initialised: winit does
//! that for the thread that owns the window.

// The `implement` macro's generated glue trips these two pedantic lints.
#![allow(clippy::inline_always, clippy::ref_as_ptr)]

use std::path::PathBuf;

use raw_window_handle::RawWindowHandle;
use types::Error;
use windows::Win32::Foundation::{
    DATA_S_SAMEFORMATETC, DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS,
    DV_E_FORMATETC, E_NOTIMPL, HGLOBAL, HWND, OLE_E_ADVISENOTSUPPORTED, POINT, S_OK,
};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::Com::{
    DATADIR_GET, DVASPECT_CONTENT, FORMATETC, IAdviseSink, IDataObject, IDataObject_Impl,
    IEnumFORMATETC, IEnumSTATDATA, STGMEDIUM, STGMEDIUM_0, TYMED_HGLOBAL,
};
use windows::Win32::System::Memory::{
    GMEM_MOVEABLE, GMEM_ZEROINIT, GlobalAlloc, GlobalLock, GlobalUnlock,
};
use windows::Win32::System::Ole::{
    CF_HDROP, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE, DoDragDrop,
    IDropSource, IDropSource_Impl,
};
use windows::Win32::System::SystemServices::{MK_LBUTTON, MK_RBUTTON, MODIFIERKEYS_FLAGS};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Shell::{DROPFILES, SHCreateStdEnumFmtEtc};
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
use windows::core::{BOOL, HRESULT, Ref, Result as WinResult, implement};

use crate::traits::{Point, WindowRef};

/// The dragged files, as the one format the object serves: `CF_HDROP` in an
/// `HGLOBAL`.
#[implement(IDataObject)]
struct FileList {
    /// The `DROPFILES` block: the header, then every path in UTF-16, each
    /// NUL-terminated, then one more NUL.
    block: Vec<u8>,
}

impl FileList {
    fn new(paths: &[PathBuf]) -> Self {
        use std::os::windows::ffi::OsStrExt as _;
        let header = std::mem::size_of::<DROPFILES>();
        // The header is a handful of bytes; it fits the u32 the struct has.
        #[allow(clippy::cast_possible_truncation)]
        let dropfiles = DROPFILES {
            pFiles: header as u32,
            pt: POINT::default(),
            fNC: BOOL(0),
            fWide: BOOL(1),
        };
        // SAFETY: `DROPFILES` is plain data of `header` bytes, read as bytes
        // for as long as `dropfiles` lives.
        let header_bytes =
            unsafe { std::slice::from_raw_parts((&raw const dropfiles).cast::<u8>(), header) };
        let mut block = header_bytes.to_vec();
        for path in paths {
            for unit in crate::dnd::absolute(path).as_os_str().encode_wide() {
                block.extend_from_slice(&unit.to_ne_bytes());
            }
            block.extend_from_slice(&0u16.to_ne_bytes());
        }
        block.extend_from_slice(&0u16.to_ne_bytes());
        Self { block }
    }

    fn is_hdrop(format: &FORMATETC) -> bool {
        format.cfFormat == CF_HDROP.0
            && format.tymed & TYMED_HGLOBAL.0 as u32 != 0
            && format.dwAspect == DVASPECT_CONTENT.0
    }

    fn format() -> FORMATETC {
        FORMATETC {
            cfFormat: CF_HDROP.0,
            ptd: std::ptr::null_mut(),
            dwAspect: DVASPECT_CONTENT.0,
            lindex: -1,
            tymed: TYMED_HGLOBAL.0 as u32,
        }
    }

    /// A fresh global copy of the block, which the receiver frees.
    fn global(&self) -> WinResult<HGLOBAL> {
        let bytes = self.block.len();
        // SAFETY: a moveable, zeroed block of `bytes`, locked for the copy
        // and unlocked after; the receiver owns it from then on.
        unsafe {
            let global = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, bytes)?;
            let dest = GlobalLock(global);
            if dest.is_null() {
                return Err(windows::core::Error::from_thread());
            }
            std::ptr::copy_nonoverlapping(self.block.as_ptr(), dest.cast::<u8>(), bytes);
            let _ = GlobalUnlock(global);
            Ok(global)
        }
    }
}

impl IDataObject_Impl for FileList_Impl {
    fn GetData(&self, format: *const FORMATETC) -> WinResult<STGMEDIUM> {
        // SAFETY: OLE passes a valid `FORMATETC`.
        let format =
            unsafe { format.as_ref() }.ok_or_else(|| windows::core::Error::from(DV_E_FORMATETC))?;
        if !FileList::is_hdrop(format) {
            return Err(DV_E_FORMATETC.into());
        }
        let global = self.global()?;
        Ok(STGMEDIUM {
            tymed: TYMED_HGLOBAL.0 as u32,
            u: STGMEDIUM_0 { hGlobal: global },
            pUnkForRelease: std::mem::ManuallyDrop::new(None),
        })
    }

    fn GetDataHere(&self, _format: *const FORMATETC, _medium: *mut STGMEDIUM) -> WinResult<()> {
        Err(E_NOTIMPL.into())
    }

    fn QueryGetData(&self, format: *const FORMATETC) -> HRESULT {
        // SAFETY: OLE passes a valid `FORMATETC`.
        match unsafe { format.as_ref() } {
            Some(format) if FileList::is_hdrop(format) => S_OK,
            _ => DV_E_FORMATETC,
        }
    }

    fn GetCanonicalFormatEtc(&self, _in: *const FORMATETC, out: *mut FORMATETC) -> HRESULT {
        // SAFETY: OLE passes a valid out pointer.
        if let Some(out) = unsafe { out.as_mut() } {
            out.ptd = std::ptr::null_mut();
        }
        DATA_S_SAMEFORMATETC
    }

    fn SetData(
        &self,
        _format: *const FORMATETC,
        _medium: *const STGMEDIUM,
        _release: BOOL,
    ) -> WinResult<()> {
        Err(E_NOTIMPL.into())
    }

    fn EnumFormatEtc(&self, direction: u32) -> WinResult<IEnumFORMATETC> {
        if direction != DATADIR_GET.0 as u32 {
            return Err(E_NOTIMPL.into());
        }
        // SAFETY: a plain shell call over one format.
        unsafe { SHCreateStdEnumFmtEtc(&[FileList::format()]) }
    }

    fn DAdvise(
        &self,
        _format: *const FORMATETC,
        _advf: u32,
        _sink: Ref<'_, IAdviseSink>,
    ) -> WinResult<u32> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }

    fn DUnadvise(&self, _connection: u32) -> WinResult<()> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }

    fn EnumDAdvise(&self) -> WinResult<IEnumSTATDATA> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }
}

/// Ends the drag on Escape or when every button is up; the default cursors.
#[implement(IDropSource)]
struct Source;

impl IDropSource_Impl for Source_Impl {
    fn QueryContinueDrag(&self, escape_pressed: BOOL, key_state: MODIFIERKEYS_FLAGS) -> HRESULT {
        if escape_pressed.as_bool() {
            DRAGDROP_S_CANCEL
        } else if key_state & (MK_LBUTTON | MK_RBUTTON) == MODIFIERKEYS_FLAGS(0) {
            DRAGDROP_S_DROP
        } else {
            S_OK
        }
    }

    fn GiveFeedback(&self, _effect: DROPEFFECT) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

fn hwnd_of(window: &WindowRef) -> Result<HWND, Error> {
    match window.window {
        RawWindowHandle::Win32(handle) => Ok(HWND(handle.hwnd.get() as *mut std::ffi::c_void)),
        _ => Err(Error::Unsupported("the window is not a Win32 window")),
    }
}

/// Runs the drag to its end: `DoDragDrop` with the files as `CF_HDROP`,
/// offering copy, move and link; the receiver picks.
pub(super) fn start_drag(window: &WindowRef, paths: &[PathBuf]) -> Result<(), Error> {
    hwnd_of(window)?;
    if paths.is_empty() {
        return Err(Error::Unsupported("nothing to drag"));
    }
    let data: IDataObject = FileList::new(paths).into();
    let source: IDropSource = Source.into();
    let mut effect = DROPEFFECT(0);
    // SAFETY: both objects are live COM objects of this thread, which has
    // OLE initialised (winit did, for the window's thread); the out pointer
    // is a local.
    let result = unsafe {
        DoDragDrop(
            &data,
            &source,
            DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK,
            &raw mut effect,
        )
    };
    tracing::debug!(count = paths.len(), ?result, ?effect, "windows drag ended");
    if result == DRAGDROP_S_DROP || result == DRAGDROP_S_CANCEL || result == S_OK {
        Ok(())
    } else {
        Err(Error::Io(std::io::Error::other(format!(
            "DoDragDrop failed: {result}"
        ))))
    }
}

/// `DoDragDrop` returned before [`start_drag`] did: nothing runs on.
pub(super) fn drag_in_progress() -> bool {
    false
}

/// The cursor over the window's client area, in logical points of its DPI.
pub(super) fn drop_position(window: &WindowRef) -> Option<Point> {
    let hwnd = hwnd_of(window).ok()?;
    let mut point = POINT::default();
    // SAFETY: plain calls with a valid window handle and a local point.
    unsafe {
        GetCursorPos(&raw mut point).ok()?;
        if !ScreenToClient(hwnd, &raw mut point).as_bool() {
            return None;
        }
    }
    // SAFETY: a plain query on the window.
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let scale = if dpi == 0 {
        if window.scale > 0.0 {
            window.scale
        } else {
            1.0
        }
    } else {
        // 96 dpi is 1:1; the ratio is small.
        #[allow(clippy::cast_precision_loss)]
        {
            dpi as f32 / 96.0
        }
    };
    // Client coordinates of a window fit f32.
    #[allow(clippy::cast_precision_loss)]
    Some(Point {
        x: point.x as f32 / scale,
        y: point.y as f32 / scale,
    })
}
