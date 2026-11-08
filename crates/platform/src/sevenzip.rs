//! The 7-Zip library (`7z.so` on Linux and macOS, `7z.dll` on Windows;
//! LGPL), loaded dynamically with `libloading` and driven through its
//! COM-style interfaces, for the disk images the archive provider and
//! previewer browse (SPEC.md "Disk images"): the library parses the image
//! containers (VHD, VHDX, VMDK, VDI, QCOW2, DMG) and the file systems
//! inside them (FAT, NTFS, ext, HFS+, APFS, ISO), which no mature Rust
//! crate does. Every `unsafe` of the binding lives here, as the
//! Architecture rules want (QUESTIONS.md), behind a safe API:
//! [`SevenZip::load`] binds a library file and lists its [`Format`]s;
//! [`SevenZip::open_file`] and [`SevenZip::open_stream`] open an archive of
//! one format; an [`Archive`] lists its [`Item`]s, hands out the stream of
//! an item ([`Archive::sub_stream`], how a container's disk or a
//! partition is opened as an archive of its own, the way 7-Zip's own
//! browser descends) and extracts one into a writer. Nothing here knows
//! Rick's types; the plugin maps the errors.
//!
//! The library's interfaces are C++ virtual tables with the COM calling
//! convention and `IUnknown` first; the Rust side mirrors each table it
//! calls as a `#[repr(C)]` struct of function pointers, and implements the
//! three objects the library calls back into (an input stream over a
//! file, an open callback, an extract callback with its output stream) as
//! `#[repr(C)]` structs whose first field is the table pointer, with a
//! reference count behind it. Strings cross as `BSTR`s: a length prefix
//! in bytes, then `wchar_t`s (UTF-32 on Linux and macOS, UTF-16 on
//! Windows), freed with the C `free` the library allocated them with, or
//! `SysFreeString` on Windows.

use std::ffi::c_void;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime};

use libloading::Library;

/// The library's file name on this platform: 7-Zip names it `7z.so` on
/// macOS as well.
pub const LIBRARY_FILE: &str = if cfg!(windows) { "7z.dll" } else { "7z.so" };

/// Where the platform's usual installations put the library, in the
/// order they are tried: the distribution packages (`7zip`, `p7zip`) on
/// Linux, Homebrew and MacPorts on macOS, the 7-Zip installer on Windows.
#[must_use]
pub fn usual_locations() -> Vec<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        [
            "/usr/lib/7zip/7z.so",
            "/usr/lib/p7zip/7z.so",
            "/usr/libexec/p7zip/7z.so",
            "/usr/lib64/7zip/7z.so",
            "/usr/lib64/p7zip/7z.so",
            "/usr/lib/x86_64-linux-gnu/7zip/7z.so",
            "/usr/lib/aarch64-linux-gnu/7zip/7z.so",
            "/usr/local/lib/7zip/7z.so",
            "/usr/local/lib/p7zip/7z.so",
        ]
        .into_iter()
        .map(PathBuf::from)
        .collect()
    }
    #[cfg(target_os = "macos")]
    {
        [
            "/usr/local/lib/7zip/7z.so",
            "/usr/local/lib/p7zip/7z.so",
            "/opt/homebrew/lib/7zip/7z.so",
            "/opt/homebrew/lib/p7zip/7z.so",
            "/opt/local/lib/p7zip/7z.so",
        ]
        .into_iter()
        .map(PathBuf::from)
        .collect()
    }
    #[cfg(windows)]
    {
        ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"]
            .into_iter()
            .filter_map(std::env::var_os)
            .map(|dir| PathBuf::from(dir).join("7-Zip").join("7z.dll"))
            .collect()
    }
}

/// How the library is installed on this platform, for the message the
/// preview pane shows when it is missing.
#[must_use]
pub fn install_hint() -> &'static str {
    #[cfg(target_os = "linux")]
    {
        "install the 7zip package (`sudo apt install 7zip`, `sudo dnf install 7zip`), which \
         puts 7z.so under /usr/lib/7zip, or build 7z.so from the 7-Zip sources at \
         https://7-zip.org"
    }
    #[cfg(target_os = "macos")]
    {
        "build 7z.so from the 7-Zip sources at https://7-zip.org (`make -f \
         ../../cmpl_mac_arm64.mak` in CPP/7zip/Bundles/Format7zF) or install p7zip from \
         Homebrew"
    }
    #[cfg(windows)]
    {
        "install 7-Zip from https://7-zip.org, which puts 7z.dll in C:\\Program Files\\7-Zip"
    }
}

/// Why the library could not be used.
#[derive(Debug, thiserror::Error)]
pub enum SevenZipError {
    /// The file could not be loaded as a library.
    #[error("{path}: {source}")]
    Load {
        /// The library file.
        path: PathBuf,
        /// What the loader said.
        #[source]
        source: libloading::Error,
    },
    /// The library lacks an export every 7-Zip library has.
    #[error("{path} is not the 7-Zip library: no `{symbol}`")]
    NotSevenZip {
        /// The library file.
        path: PathBuf,
        /// The missing export.
        symbol: &'static str,
    },
    /// A call into the library failed.
    #[error("7-Zip: {call} failed with HRESULT {code:#010x}")]
    Call {
        /// The method.
        call: &'static str,
        /// The HRESULT, as an unsigned number.
        code: u32,
    },
    /// The library has no handler of that name.
    #[error("7-Zip: this library has no {0} format")]
    NoFormat(String),
    /// The handler did not recognise the data as its format.
    #[error("7-Zip: not a {0} archive")]
    NotArchive(String),
    /// The item was not extracted whole.
    #[error("7-Zip: extracting failed: {0}")]
    Extract(&'static str),
    /// The item has no stream of its own.
    #[error("7-Zip: the {0} handler gives item {1} no stream")]
    NoStream(String, u32),
    /// An I/O error on the file or the sink.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

type Hresult = i32;
const S_OK: Hresult = 0;
const S_FALSE: Hresult = 1;
const E_NOINTERFACE: Hresult = 0x8000_4002_u32.cast_signed();
const E_FAIL: Hresult = 0x8000_4005_u32.cast_signed();
const E_INVALIDARG: Hresult = 0x8007_0057_u32.cast_signed();

/// A COM interface or class id.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

impl Guid {
    /// A 7-Zip interface id: `23170F69-40C1-278A-0000-00gg00ss0000` for
    /// group `gg` and sub-id `ss`.
    const fn iface(group: u8, sub: u8) -> Self {
        Self {
            data1: 0x2317_0F69,
            data2: 0x40C1,
            data3: 0x278A,
            data4: [0, 0, 0, group, 0, sub, 0, 0],
        }
    }

    /// The id from its 16 bytes as a class id property carries them.
    fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != 16 {
            return None;
        }
        let mut data4 = [0u8; 8];
        data4.copy_from_slice(&bytes[8..16]);
        Some(Self {
            data1: u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            data2: u16::from_le_bytes([bytes[4], bytes[5]]),
            data3: u16::from_le_bytes([bytes[6], bytes[7]]),
            data4,
        })
    }
}

const IID_IUNKNOWN: Guid = Guid {
    data1: 0,
    data2: 0,
    data3: 0,
    data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46],
};
const IID_ISEQUENTIAL_IN_STREAM: Guid = Guid::iface(3, 0x01);
const IID_ISEQUENTIAL_OUT_STREAM: Guid = Guid::iface(3, 0x02);
const IID_IIN_STREAM: Guid = Guid::iface(3, 0x03);
const IID_IPROGRESS: Guid = Guid::iface(0, 0x05);
const IID_IARCHIVE_OPEN_CALLBACK: Guid = Guid::iface(6, 0x10);
const IID_IARCHIVE_EXTRACT_CALLBACK: Guid = Guid::iface(6, 0x20);
const IID_IIN_ARCHIVE_GET_STREAM: Guid = Guid::iface(6, 0x40);
const IID_IIN_ARCHIVE: Guid = Guid::iface(6, 0x60);

/// The property ids of `IInArchive::GetProperty` and
/// `GetArchiveProperty` that matter here (7-Zip's `PropID.h`).
pub mod prop {
    /// The index of the item a container stands for: the disk inside a
    /// VHDX, the main partition of a DMG.
    pub const MAIN_SUBFILE: u32 = 1;
    /// The path inside the archive.
    pub const PATH: u32 = 3;
    /// A directory.
    pub const IS_DIR: u32 = 6;
    /// The unpacked size.
    pub const SIZE: u32 = 7;
    /// The packed size.
    pub const PACK_SIZE: u32 = 8;
    /// Windows attributes, the Unix mode in the high 16 bits when bit 15
    /// is set.
    pub const ATTRIB: u32 = 9;
    /// The modification time.
    pub const MTIME: u32 = 12;
    /// The content is encrypted.
    pub const ENCRYPTED: u32 = 15;
    /// The file system of a partition or an image.
    pub const FILE_SYSTEM: u32 = 24;
    /// The cluster size of a file system.
    pub const CLUSTER_SIZE: u32 = 58;
    /// The Unix mode.
    pub const POSIX_ATTRIB: u32 = 53;
    /// A symbolic link's target.
    pub const SYMLINK: u32 = 54;
    /// The volume's label.
    pub const VOLUME_NAME: u32 = 59;
}

/// The handler properties of `GetHandlerProperty2`.
mod handler_prop {
    pub const NAME: u32 = 0;
    pub const CLASS_ID: u32 = 1;
    pub const EXTENSION: u32 = 2;
}

const VT_EMPTY: u16 = 0;
const VT_I2: u16 = 2;
const VT_I4: u16 = 3;
const VT_BSTR: u16 = 8;
const VT_BOOL: u16 = 11;
const VT_UI1: u16 = 17;
const VT_UI2: u16 = 18;
const VT_UI4: u16 = 19;
const VT_I8: u16 = 20;
const VT_UI8: u16 = 21;
const VT_FILETIME: u16 = 64;

/// 100-nanosecond ticks between 1601-01-01 and 1970-01-01.
const FILETIME_UNIX_EPOCH: u64 = 116_444_736_000_000_000;

/// A `PROPVARIANT`: the type tag, three reserved words and the value
/// union at offset 8. 24 bytes, the size of the Windows one; the library
/// on Linux and macOS writes a 16-byte layout with the same prefix.
#[repr(C)]
struct PropVariant {
    vt: u16,
    reserved: [u16; 3],
    data: [u64; 2],
}

/// A property's value, converted from the variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// A string.
    Text(String),
    /// Binary data (a class id, a signature).
    Bytes(Vec<u8>),
    /// A whole number.
    Int(u64),
    /// A flag.
    Bool(bool),
    /// A time.
    Time(SystemTime),
}

impl Value {
    /// The value as a string, when it is one.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            _ => None,
        }
    }

    /// The value as a number, when it is one.
    #[must_use]
    pub fn int(&self) -> Option<u64> {
        match self {
            Self::Int(n) => Some(*n),
            _ => None,
        }
    }

    /// The value as a flag, when it is one.
    #[must_use]
    pub fn bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// The value as a time, when it is one.
    #[must_use]
    pub fn time(&self) -> Option<SystemTime> {
        match self {
            Self::Time(t) => Some(*t),
            _ => None,
        }
    }
}

impl PropVariant {
    const fn empty() -> Self {
        Self {
            vt: VT_EMPTY,
            reserved: [0; 3],
            data: [0; 2],
        }
    }

    /// The `BSTR` the variant holds, when it is one and not null.
    fn bstr(&self) -> Option<*mut c_void> {
        if self.vt != VT_BSTR {
            return None;
        }
        // The union's first word is the pointer.
        let pointer = usize::try_from(self.data[0]).ok()? as *mut c_void;
        (!pointer.is_null()).then_some(pointer)
    }

    /// The bytes of a `BSTR`: its length prefix counts them.
    ///
    /// # Safety
    ///
    /// `bstr` is a `BSTR` the library allocated: a 4-byte length in bytes
    /// precedes the data.
    unsafe fn bstr_bytes(bstr: *mut c_void) -> Vec<u8> {
        // SAFETY: the caller's guarantee; the prefix lies 4 bytes before
        // the pointer and the data runs `len` bytes from it.
        unsafe {
            let len = bstr.cast::<u32>().sub(1).read_unaligned() as usize;
            std::slice::from_raw_parts(bstr.cast::<u8>(), len).to_vec()
        }
    }

    /// The value, with a copy of any string; the variant itself is
    /// cleared by `Drop`.
    fn value(&self) -> Option<Value> {
        match self.vt {
            VT_EMPTY => None,
            VT_BSTR => {
                let bstr = self.bstr()?;
                // SAFETY: `bstr` came from the library's `GetProperty`.
                let bytes = unsafe { Self::bstr_bytes(bstr) };
                Some(Value::Bytes(bytes))
            }
            VT_BOOL => Some(Value::Bool((self.data[0] & 0xFFFF) != 0)),
            VT_UI1 => Some(Value::Int(self.data[0] & 0xFF)),
            VT_UI2 | VT_I2 => Some(Value::Int(self.data[0] & 0xFFFF)),
            VT_UI4 | VT_I4 => Some(Value::Int(self.data[0] & 0xFFFF_FFFF)),
            VT_UI8 | VT_I8 => Some(Value::Int(self.data[0])),
            VT_FILETIME => {
                let ticks = self.data[0];
                let unix = ticks.checked_sub(FILETIME_UNIX_EPOCH)?;
                Some(Value::Time(
                    SystemTime::UNIX_EPOCH + Duration::from_nanos(unix.saturating_mul(100)),
                ))
            }
            _ => None,
        }
    }

    /// The value, a `BSTR` decoded as text.
    fn text_value(&self) -> Option<Value> {
        match self.value()? {
            Value::Bytes(bytes) => Some(Value::Text(decode_wide(&bytes))),
            other => Some(other),
        }
    }
}

impl Drop for PropVariant {
    fn drop(&mut self) {
        if let Some(bstr) = self.bstr() {
            // SAFETY: the string was allocated by the library's
            // `SysAllocString`, which is the platform's on Windows and
            // `malloc` with a 4-byte prefix elsewhere; it is freed once,
            // here, and the variant forgets it.
            unsafe { free_bstr(bstr) };
            self.vt = VT_EMPTY;
            self.data = [0; 2];
        }
    }
}

/// Frees a `BSTR` the library allocated.
///
/// # Safety
///
/// `bstr` is a live `BSTR` from the library, freed nowhere else.
#[cfg(windows)]
unsafe fn free_bstr(bstr: *mut c_void) {
    #[link(name = "oleaut32")]
    unsafe extern "system" {
        fn SysFreeString(bstr: *mut c_void);
    }
    // SAFETY: the caller's guarantee.
    unsafe { SysFreeString(bstr) }
}

/// Frees a `BSTR` the library allocated.
///
/// # Safety
///
/// `bstr` is a live `BSTR` from the library, freed nowhere else.
#[cfg(not(windows))]
unsafe fn free_bstr(bstr: *mut c_void) {
    // SAFETY: 7-Zip's `SysAllocString` on Unix is `malloc` of a 4-byte
    // length prefix plus the data, and its `SysFreeString` frees the
    // prefix's address.
    unsafe { libc::free(bstr.cast::<u32>().sub(1).cast::<c_void>()) }
}

/// A `wchar_t` string's bytes as text: UTF-16 on Windows, UTF-32 elsewhere.
fn decode_wide(bytes: &[u8]) -> String {
    if cfg!(windows) {
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_ne_bytes(*c))
            .collect();
        let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
        String::from_utf16_lossy(&units[..end])
    } else {
        bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| u32::from_ne_bytes(*c))
            .take_while(|&c| c != 0)
            .map(|c| char::from_u32(c).unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect()
    }
}

// The virtual tables of the interfaces this binding calls or implements,
// in the library's method order (`IArchive.h`, `IStream.h`,
// `IProgress.h`), `IUnknown` first.

#[repr(C)]
struct IUnknownVtbl {
    query_interface:
        unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> Hresult,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
}

#[repr(C)]
struct IInArchiveVtbl {
    base: IUnknownVtbl,
    open: unsafe extern "system" fn(*mut c_void, *mut c_void, *const u64, *mut c_void) -> Hresult,
    close: unsafe extern "system" fn(*mut c_void) -> Hresult,
    get_number_of_items: unsafe extern "system" fn(*mut c_void, *mut u32) -> Hresult,
    get_property: unsafe extern "system" fn(*mut c_void, u32, u32, *mut PropVariant) -> Hresult,
    extract: unsafe extern "system" fn(*mut c_void, *const u32, u32, i32, *mut c_void) -> Hresult,
    get_archive_property: unsafe extern "system" fn(*mut c_void, u32, *mut PropVariant) -> Hresult,
}

#[repr(C)]
struct IInArchiveGetStreamVtbl {
    base: IUnknownVtbl,
    get_stream: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> Hresult,
}

#[repr(C)]
struct ISequentialInStreamVtbl {
    base: IUnknownVtbl,
    read: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut u32) -> Hresult,
}

#[repr(C)]
struct IInStreamVtbl {
    sequential: ISequentialInStreamVtbl,
    seek: unsafe extern "system" fn(*mut c_void, i64, u32, *mut u64) -> Hresult,
}

#[repr(C)]
struct ISequentialOutStreamVtbl {
    base: IUnknownVtbl,
    write: unsafe extern "system" fn(*mut c_void, *const c_void, u32, *mut u32) -> Hresult,
}

#[repr(C)]
struct IArchiveOpenCallbackVtbl {
    base: IUnknownVtbl,
    set_total: unsafe extern "system" fn(*mut c_void, *const u64, *const u64) -> Hresult,
    set_completed: unsafe extern "system" fn(*mut c_void, *const u64, *const u64) -> Hresult,
}

#[repr(C)]
struct IArchiveExtractCallbackVtbl {
    base: IUnknownVtbl,
    set_total: unsafe extern "system" fn(*mut c_void, u64) -> Hresult,
    set_completed: unsafe extern "system" fn(*mut c_void, *const u64) -> Hresult,
    get_stream: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void, i32) -> Hresult,
    prepare_operation: unsafe extern "system" fn(*mut c_void, i32) -> Hresult,
    set_operation_result: unsafe extern "system" fn(*mut c_void, i32) -> Hresult,
}

/// Calls `AddRef` on a COM object.
///
/// # Safety
///
/// `object` is a live COM object.
unsafe fn add_ref(object: *mut c_void) {
    // SAFETY: a COM object's first word is its table, whose first entries
    // are `IUnknown`'s.
    unsafe {
        let vtbl = *object.cast::<*const IUnknownVtbl>();
        ((*vtbl).add_ref)(object);
    }
}

/// Calls `Release` on a COM object.
///
/// # Safety
///
/// `object` is a live COM object this caller holds a reference to.
unsafe fn release(object: *mut c_void) {
    // SAFETY: as for `add_ref`.
    unsafe {
        let vtbl = *object.cast::<*const IUnknownVtbl>();
        ((*vtbl).release)(object);
    }
}

/// Asks a COM object for one of its interfaces.
///
/// # Safety
///
/// `object` is a live COM object.
unsafe fn query_interface(object: *mut c_void, iid: &Guid) -> Option<*mut c_void> {
    let mut out: *mut c_void = std::ptr::null_mut();
    // SAFETY: as for `add_ref`; `out` is a valid out-pointer.
    let code = unsafe {
        let vtbl = *object.cast::<*const IUnknownVtbl>();
        ((*vtbl).query_interface)(object, iid, &raw mut out)
    };
    (code == S_OK && !out.is_null()).then_some(out)
}

/// A reference to a COM object, released when dropped.
struct ComPtr(*mut c_void);

impl Drop for ComPtr {
    fn drop(&mut self) {
        // SAFETY: the pointer was handed out by the library or one of the
        // objects below with a reference for us, released once here.
        unsafe { release(self.0) };
    }
}

// The objects the library calls back into. Each is `#[repr(C)]` with the
// table pointer first, as a C++ object with one virtual base is laid out,
// then a reference count; `Release` at zero drops the box. Every method
// catches panics, which must not unwind into C++, and answers `E_FAIL`.

/// The common `QueryInterface`: `self` for the ids in `supported`.
///
/// # Safety
///
/// `this` is one of this module's objects and `iid`, `out` are valid.
unsafe fn qi(
    this: *mut c_void,
    iid: *const Guid,
    out: *mut *mut c_void,
    supported: &[Guid],
) -> Hresult {
    if out.is_null() || iid.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's guarantee; the pointers were checked.
    unsafe {
        if supported.contains(&*iid) {
            add_ref(this);
            *out = this;
            S_OK
        } else {
            *out = std::ptr::null_mut();
            E_NOINTERFACE
        }
    }
}

/// Our input stream over a file: `ISequentialInStream` and `IInStream`.
#[repr(C)]
struct FileStream {
    vtbl: *const IInStreamVtbl,
    refs: AtomicU32,
    file: File,
}

static FILE_STREAM_VTBL: IInStreamVtbl = IInStreamVtbl {
    sequential: ISequentialInStreamVtbl {
        base: IUnknownVtbl {
            query_interface: file_stream_qi,
            add_ref: file_stream_add_ref,
            release: file_stream_release,
        },
        read: file_stream_read,
    },
    seek: file_stream_seek,
};

unsafe extern "system" fn file_stream_qi(
    this: *mut c_void,
    iid: *const Guid,
    out: *mut *mut c_void,
) -> Hresult {
    // SAFETY: `this` is a `FileStream` the library got from us.
    unsafe {
        qi(
            this,
            iid,
            out,
            &[IID_IUNKNOWN, IID_ISEQUENTIAL_IN_STREAM, IID_IIN_STREAM],
        )
    }
}

unsafe extern "system" fn file_stream_add_ref(this: *mut c_void) -> u32 {
    // SAFETY: `this` is a live `FileStream`.
    let stream = unsafe { &*this.cast::<FileStream>() };
    stream.refs.fetch_add(1, Ordering::AcqRel) + 1
}

unsafe extern "system" fn file_stream_release(this: *mut c_void) -> u32 {
    // SAFETY: `this` is a live `FileStream` with a reference to give back.
    let stream = unsafe { &*this.cast::<FileStream>() };
    let left = stream.refs.fetch_sub(1, Ordering::AcqRel) - 1;
    if left == 0 {
        // SAFETY: the last reference is gone; the box was leaked by
        // `FileStream::new` and nobody else points at it.
        drop(unsafe { Box::from_raw(this.cast::<FileStream>()) });
    }
    left
}

unsafe extern "system" fn file_stream_read(
    this: *mut c_void,
    data: *mut c_void,
    size: u32,
    processed: *mut u32,
) -> Hresult {
    let outcome = std::panic::catch_unwind(|| {
        // SAFETY: `this` is a live `FileStream`; `data` has `size` bytes
        // the library wants filled; `processed` is an out-pointer or null.
        unsafe {
            let stream = &mut *this.cast::<FileStream>();
            let buffer = std::slice::from_raw_parts_mut(data.cast::<u8>(), size as usize);
            let read = match stream.file.read(buffer) {
                Ok(read) => read,
                Err(_) => return E_FAIL,
            };
            if !processed.is_null() {
                *processed = u32::try_from(read).unwrap_or(u32::MAX);
            }
            S_OK
        }
    });
    outcome.unwrap_or(E_FAIL)
}

unsafe extern "system" fn file_stream_seek(
    this: *mut c_void,
    offset: i64,
    origin: u32,
    new_position: *mut u64,
) -> Hresult {
    let outcome = std::panic::catch_unwind(|| {
        let from = match origin {
            0 => SeekFrom::Start(u64::try_from(offset).unwrap_or(0)),
            1 => SeekFrom::Current(offset),
            2 => SeekFrom::End(offset),
            _ => return E_INVALIDARG,
        };
        // SAFETY: `this` is a live `FileStream`; `new_position` is an
        // out-pointer or null.
        unsafe {
            let stream = &mut *this.cast::<FileStream>();
            match stream.file.seek(from) {
                Ok(position) => {
                    if !new_position.is_null() {
                        *new_position = position;
                    }
                    S_OK
                }
                Err(_) => E_FAIL,
            }
        }
    });
    outcome.unwrap_or(E_FAIL)
}

impl FileStream {
    /// A stream over `file`, as a COM object with one reference.
    fn new(file: File) -> ComPtr {
        let stream = Box::new(Self {
            vtbl: &raw const FILE_STREAM_VTBL,
            refs: AtomicU32::new(1),
            file,
        });
        ComPtr(Box::into_raw(stream).cast::<c_void>())
    }
}

/// Our open callback: progress nobody reads, no password.
#[repr(C)]
struct OpenCallback {
    vtbl: *const IArchiveOpenCallbackVtbl,
    refs: AtomicU32,
}

static OPEN_CALLBACK_VTBL: IArchiveOpenCallbackVtbl = IArchiveOpenCallbackVtbl {
    base: IUnknownVtbl {
        query_interface: open_callback_qi,
        add_ref: open_callback_add_ref,
        release: open_callback_release,
    },
    set_total: open_callback_progress,
    set_completed: open_callback_progress,
};

unsafe extern "system" fn open_callback_qi(
    this: *mut c_void,
    iid: *const Guid,
    out: *mut *mut c_void,
) -> Hresult {
    // SAFETY: `this` is an `OpenCallback` the library got from us.
    unsafe { qi(this, iid, out, &[IID_IUNKNOWN, IID_IARCHIVE_OPEN_CALLBACK]) }
}

unsafe extern "system" fn open_callback_add_ref(this: *mut c_void) -> u32 {
    // SAFETY: `this` is a live `OpenCallback`.
    let callback = unsafe { &*this.cast::<OpenCallback>() };
    callback.refs.fetch_add(1, Ordering::AcqRel) + 1
}

unsafe extern "system" fn open_callback_release(this: *mut c_void) -> u32 {
    // SAFETY: `this` is a live `OpenCallback` with a reference to give back.
    let callback = unsafe { &*this.cast::<OpenCallback>() };
    let left = callback.refs.fetch_sub(1, Ordering::AcqRel) - 1;
    if left == 0 {
        // SAFETY: the last reference is gone; the box was leaked by
        // `OpenCallback::new`.
        drop(unsafe { Box::from_raw(this.cast::<OpenCallback>()) });
    }
    left
}

unsafe extern "system" fn open_callback_progress(
    _this: *mut c_void,
    _files: *const u64,
    _bytes: *const u64,
) -> Hresult {
    S_OK
}

impl OpenCallback {
    fn new() -> ComPtr {
        let callback = Box::new(Self {
            vtbl: &raw const OPEN_CALLBACK_VTBL,
            refs: AtomicU32::new(1),
        });
        ComPtr(Box::into_raw(callback).cast::<c_void>())
    }
}

/// Our output stream: writes into the sink of the extraction in progress.
#[repr(C)]
struct OutStream {
    vtbl: *const ISequentialOutStreamVtbl,
    refs: AtomicU32,
    /// The sink, for the duration of the `Extract` call; cleared before
    /// the call returns, so a late write fails instead of dangling.
    sink: Option<*mut (dyn Write + 'static)>,
    /// The first error the sink raised.
    error: Option<std::io::Error>,
}

static OUT_STREAM_VTBL: ISequentialOutStreamVtbl = ISequentialOutStreamVtbl {
    base: IUnknownVtbl {
        query_interface: out_stream_qi,
        add_ref: out_stream_add_ref,
        release: out_stream_release,
    },
    write: out_stream_write,
};

unsafe extern "system" fn out_stream_qi(
    this: *mut c_void,
    iid: *const Guid,
    out: *mut *mut c_void,
) -> Hresult {
    // SAFETY: `this` is an `OutStream` the library got from us.
    unsafe { qi(this, iid, out, &[IID_IUNKNOWN, IID_ISEQUENTIAL_OUT_STREAM]) }
}

unsafe extern "system" fn out_stream_add_ref(this: *mut c_void) -> u32 {
    // SAFETY: `this` is a live `OutStream`.
    let stream = unsafe { &*this.cast::<OutStream>() };
    stream.refs.fetch_add(1, Ordering::AcqRel) + 1
}

unsafe extern "system" fn out_stream_release(this: *mut c_void) -> u32 {
    // SAFETY: `this` is a live `OutStream` with a reference to give back.
    let stream = unsafe { &*this.cast::<OutStream>() };
    let left = stream.refs.fetch_sub(1, Ordering::AcqRel) - 1;
    if left == 0 {
        // SAFETY: the last reference is gone; the box was leaked by
        // `OutStream::new`.
        drop(unsafe { Box::from_raw(this.cast::<OutStream>()) });
    }
    left
}

unsafe extern "system" fn out_stream_write(
    this: *mut c_void,
    data: *const c_void,
    size: u32,
    processed: *mut u32,
) -> Hresult {
    let outcome = std::panic::catch_unwind(|| {
        // SAFETY: `this` is a live `OutStream`; `data` has `size` bytes;
        // `processed` is an out-pointer or null; the sink pointer, when
        // set, is the writer of the `Extract` call in progress.
        unsafe {
            let stream = &mut *this.cast::<OutStream>();
            let Some(sink) = stream.sink else {
                return E_FAIL;
            };
            let bytes = std::slice::from_raw_parts(data.cast::<u8>(), size as usize);
            match (*sink).write_all(bytes) {
                Ok(()) => {
                    if !processed.is_null() {
                        *processed = size;
                    }
                    S_OK
                }
                Err(error) => {
                    stream.error.get_or_insert(error);
                    E_FAIL
                }
            }
        }
    });
    outcome.unwrap_or(E_FAIL)
}

impl OutStream {
    fn new(sink: *mut (dyn Write + 'static)) -> ComPtr {
        let stream = Box::new(Self {
            vtbl: &raw const OUT_STREAM_VTBL,
            refs: AtomicU32::new(1),
            sink: Some(sink),
            error: None,
        });
        ComPtr(Box::into_raw(stream).cast::<c_void>())
    }
}

/// Our extract callback: hands the library our output stream for the one
/// item asked for and remembers how the operation ended.
#[repr(C)]
struct ExtractCallback {
    vtbl: *const IArchiveExtractCallbackVtbl,
    refs: AtomicU32,
    /// The item being extracted.
    index: u32,
    /// Our output stream, handed out with a reference per `GetStream`.
    out: *mut c_void,
    /// The operation result the library reported, `NExtract::NOperationResult`.
    result: Option<i32>,
}

static EXTRACT_CALLBACK_VTBL: IArchiveExtractCallbackVtbl = IArchiveExtractCallbackVtbl {
    base: IUnknownVtbl {
        query_interface: extract_callback_qi,
        add_ref: extract_callback_add_ref,
        release: extract_callback_release,
    },
    set_total: extract_callback_set_total,
    set_completed: extract_callback_set_completed,
    get_stream: extract_callback_get_stream,
    prepare_operation: extract_callback_prepare,
    set_operation_result: extract_callback_set_result,
};

unsafe extern "system" fn extract_callback_qi(
    this: *mut c_void,
    iid: *const Guid,
    out: *mut *mut c_void,
) -> Hresult {
    // SAFETY: `this` is an `ExtractCallback` the library got from us. The
    // password interfaces are left out on purpose: an encrypted image is
    // listed, not read.
    unsafe {
        qi(
            this,
            iid,
            out,
            &[IID_IUNKNOWN, IID_IPROGRESS, IID_IARCHIVE_EXTRACT_CALLBACK],
        )
    }
}

unsafe extern "system" fn extract_callback_add_ref(this: *mut c_void) -> u32 {
    // SAFETY: `this` is a live `ExtractCallback`.
    let callback = unsafe { &*this.cast::<ExtractCallback>() };
    callback.refs.fetch_add(1, Ordering::AcqRel) + 1
}

unsafe extern "system" fn extract_callback_release(this: *mut c_void) -> u32 {
    // SAFETY: `this` is a live `ExtractCallback` with a reference to give
    // back.
    let callback = unsafe { &*this.cast::<ExtractCallback>() };
    let left = callback.refs.fetch_sub(1, Ordering::AcqRel) - 1;
    if left == 0 {
        // SAFETY: the last reference is gone; the box was leaked by
        // `ExtractCallback::new`.
        drop(unsafe { Box::from_raw(this.cast::<ExtractCallback>()) });
    }
    left
}

unsafe extern "system" fn extract_callback_set_total(_this: *mut c_void, _total: u64) -> Hresult {
    S_OK
}

unsafe extern "system" fn extract_callback_set_completed(
    _this: *mut c_void,
    _completed: *const u64,
) -> Hresult {
    S_OK
}

/// `NExtract::NAskMode::kExtract`.
const ASK_EXTRACT: i32 = 0;

unsafe extern "system" fn extract_callback_get_stream(
    this: *mut c_void,
    index: u32,
    out_stream: *mut *mut c_void,
    ask_mode: i32,
) -> Hresult {
    if out_stream.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: `this` is a live `ExtractCallback`; `out_stream` is a valid
    // out-pointer; `out` is our live output stream.
    unsafe {
        let callback = &*this.cast::<ExtractCallback>();
        if index == callback.index && ask_mode == ASK_EXTRACT {
            add_ref(callback.out);
            *out_stream = callback.out;
        } else {
            // Not the item asked for: skip it.
            *out_stream = std::ptr::null_mut();
        }
    }
    S_OK
}

unsafe extern "system" fn extract_callback_prepare(_this: *mut c_void, _ask_mode: i32) -> Hresult {
    S_OK
}

unsafe extern "system" fn extract_callback_set_result(this: *mut c_void, result: i32) -> Hresult {
    // SAFETY: `this` is a live `ExtractCallback`.
    unsafe {
        let callback = &mut *this.cast::<ExtractCallback>();
        callback.result = Some(result);
    }
    S_OK
}

impl ExtractCallback {
    fn new(index: u32, out: *mut c_void) -> ComPtr {
        let callback = Box::new(Self {
            vtbl: &raw const EXTRACT_CALLBACK_VTBL,
            refs: AtomicU32::new(1),
            index,
            out,
            result: None,
        });
        ComPtr(Box::into_raw(callback).cast::<c_void>())
    }
}

/// The text of an `NExtract::NOperationResult`.
fn operation_result_text(result: i32) -> &'static str {
    match result {
        0 => "ok",
        1 => "unsupported method",
        2 => "data error",
        3 => "CRC error",
        4 => "data unavailable",
        5 => "unexpected end of data",
        6 => "data after the end",
        7 => "not an archive",
        8 => "headers error",
        9 => "wrong password",
        _ => "unknown error",
    }
}

type CreateObjectFn =
    unsafe extern "system" fn(*const Guid, *const Guid, *mut *mut c_void) -> Hresult;
type GetNumberOfFormatsFn = unsafe extern "system" fn(*mut u32) -> Hresult;
type GetHandlerProperty2Fn = unsafe extern "system" fn(u32, u32, *mut PropVariant) -> Hresult;

/// One archive format the library handles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Format {
    /// The handler's name: `zip`, `VHDX`, `QCOW`, `FAT`, `NTFS`, `GPT`, …
    pub name: String,
    /// The extensions the handler claims, lower case, as the library
    /// lists them.
    pub extensions: Vec<String>,
    class_id: Guid,
}

/// The loaded library.
pub struct SevenZip {
    path: PathBuf,
    create_object: CreateObjectFn,
    formats: Vec<Format>,
    /// Kept mapped for as long as any function pointer above may be
    /// called: the last field, dropped last.
    _library: Library,
}

impl std::fmt::Debug for SevenZip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SevenZip")
            .field("path", &self.path)
            .field("formats", &self.formats.len())
            .finish_non_exhaustive()
    }
}

// SAFETY: the library's exported functions are re-entrant across threads
// (7-Zip's own tools call them from several), the function pointers are
// plain data and the format list is immutable once loaded; an `Archive`
// is a separate object and not shared.
unsafe impl Send for SevenZip {}
// SAFETY: as above.
unsafe impl Sync for SevenZip {}

impl SevenZip {
    /// Loads the library at `path` and reads its format list.
    ///
    /// # Errors
    ///
    /// [`SevenZipError::Load`] when the file is not a loadable library,
    /// [`SevenZipError::NotSevenZip`] when it lacks the 7-Zip exports,
    /// [`SevenZipError::Call`] when listing the formats fails.
    pub fn load(path: &Path) -> Result<Arc<Self>, SevenZipError> {
        // SAFETY: loading a library runs its initialisers; the 7-Zip
        // library's set up tables and nothing else, and the caller names
        // the file (the bundled one, the setting's, a known install).
        let library = unsafe { Library::new(path) }.map_err(|source| SevenZipError::Load {
            path: path.to_path_buf(),
            source,
        })?;
        let not_sevenzip = |symbol| SevenZipError::NotSevenZip {
            path: path.to_path_buf(),
            symbol,
        };
        // SAFETY: the symbols are looked up by the names 7-Zip exports and
        // used with the signatures it declares (`IArchive.h`); a library
        // that happens to export the names with other signatures is not
        // one Rick ships or a user would point it at.
        let (create_object, number_of_formats, handler_property) = unsafe {
            let create_object = *library
                .get::<CreateObjectFn>(b"CreateObject\0")
                .map_err(|_| not_sevenzip("CreateObject"))?;
            let number_of_formats = *library
                .get::<GetNumberOfFormatsFn>(b"GetNumberOfFormats\0")
                .map_err(|_| not_sevenzip("GetNumberOfFormats"))?;
            let handler_property = *library
                .get::<GetHandlerProperty2Fn>(b"GetHandlerProperty2\0")
                .map_err(|_| not_sevenzip("GetHandlerProperty2"))?;
            (create_object, number_of_formats, handler_property)
        };
        let mut count: u32 = 0;
        // SAFETY: a valid out-pointer.
        check("GetNumberOfFormats", unsafe {
            number_of_formats(&raw mut count)
        })?;
        let mut formats = Vec::with_capacity(count as usize);
        for index in 0..count {
            let property = |id: u32| -> Result<Option<Value>, SevenZipError> {
                let mut variant = PropVariant::empty();
                // SAFETY: a valid out-variant, cleared by its `Drop`.
                check("GetHandlerProperty2", unsafe {
                    handler_property(index, id, &raw mut variant)
                })?;
                Ok(if id == handler_prop::CLASS_ID {
                    variant.value()
                } else {
                    variant.text_value()
                })
            };
            let name = property(handler_prop::NAME)?
                .and_then(|v| v.text().map(str::to_owned))
                .unwrap_or_default();
            let class_id = match property(handler_prop::CLASS_ID)? {
                Some(Value::Bytes(bytes)) => Guid::from_bytes(&bytes),
                _ => None,
            };
            let extensions = property(handler_prop::EXTENSION)?
                .and_then(|v| v.text().map(str::to_owned))
                .unwrap_or_default()
                .split_whitespace()
                .map(str::to_ascii_lowercase)
                .collect();
            if let Some(class_id) = class_id
                && !name.is_empty()
            {
                formats.push(Format {
                    name,
                    extensions,
                    class_id,
                });
            }
        }
        Ok(Arc::new(Self {
            path: path.to_path_buf(),
            create_object,
            formats,
            _library: library,
        }))
    }

    /// The library file this was loaded from.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Every format the library handles.
    #[must_use]
    pub fn formats(&self) -> &[Format] {
        &self.formats
    }

    /// The format called `name`, case ignored.
    #[must_use]
    pub fn format(&self, name: &str) -> Option<&Format> {
        self.formats
            .iter()
            .find(|f| f.name.eq_ignore_ascii_case(name))
    }

    /// A fresh handler object for `format`.
    fn handler(&self, format: &Format) -> Result<ComPtr, SevenZipError> {
        let mut object: *mut c_void = std::ptr::null_mut();
        let iid = IID_IIN_ARCHIVE;
        // SAFETY: the ids and the out-pointer are valid.
        check("CreateObject", unsafe {
            (self.create_object)(&raw const format.class_id, &raw const iid, &raw mut object)
        })?;
        if object.is_null() {
            return Err(SevenZipError::Call {
                call: "CreateObject",
                code: E_FAIL.cast_unsigned(),
            });
        }
        Ok(ComPtr(object))
    }

    /// Opens the file at `path` as an archive of `format`.
    ///
    /// # Errors
    ///
    /// [`SevenZipError::NotArchive`] when the handler does not recognise
    /// the file; the I/O error when it cannot be opened; a call error
    /// otherwise.
    pub fn open_file(
        self: &Arc<Self>,
        path: &Path,
        format: &Format,
    ) -> Result<Archive, SevenZipError> {
        let file = File::open(path)?;
        let stream = Stream(FileStream::new(file));
        self.open_stream(stream, format)
    }

    /// Opens `stream` (a file's, or an item's from [`Archive::sub_stream`])
    /// as an archive of `format`. The stream must start at the archive:
    /// no signature scan. It is rewound first, so one stream can be
    /// offered to several handlers in turn.
    ///
    /// # Errors
    ///
    /// [`SevenZipError::NotArchive`] when the handler does not recognise
    /// the data; a call error otherwise.
    pub fn open_stream(
        self: &Arc<Self>,
        stream: Stream,
        format: &Format,
    ) -> Result<Archive, SevenZipError> {
        stream.rewind()?;
        let handler = self.handler(format)?;
        let callback = OpenCallback::new();
        let max_check_start_position: u64 = 0;
        // SAFETY: `handler` is an `IInArchive`; the stream and callback are
        // live COM objects the handler may keep references to (it adds
        // its own); the position pointer outlives the call.
        let code = unsafe {
            let vtbl = *handler.0.cast::<*const IInArchiveVtbl>();
            ((*vtbl).open)(
                handler.0,
                stream.0.0,
                &raw const max_check_start_position,
                callback.0,
            )
        };
        match code {
            S_OK => {}
            S_FALSE => return Err(SevenZipError::NotArchive(format.name.clone())),
            other => {
                return Err(SevenZipError::Call {
                    call: "IInArchive::Open",
                    code: other.cast_unsigned(),
                });
            }
        }
        Ok(Archive {
            library: Arc::clone(self),
            format: format.name.clone(),
            handler,
            _source: stream,
        })
    }
}

/// Turns a failed HRESULT into an error.
fn check(call: &'static str, code: Hresult) -> Result<(), SevenZipError> {
    if code == S_OK {
        Ok(())
    } else {
        Err(SevenZipError::Call {
            call,
            code: code.cast_unsigned(),
        })
    }
}

/// A seekable input stream the library can open: a file of ours, or an
/// item's content handed out by a handler. Cloning adds a reference to
/// the same object, so one stream can be offered to several handlers in
/// turn; each seeks to the start itself.
pub struct Stream(ComPtr);

impl Clone for Stream {
    fn clone(&self) -> Self {
        // SAFETY: a live COM object this stream holds a reference to; the
        // clone's reference is released by its own `Drop`.
        unsafe { add_ref(self.0.0) };
        Self(ComPtr(self.0.0))
    }
}

impl Stream {
    /// A stream over the file at `path`.
    ///
    /// # Errors
    ///
    /// The I/O error from opening the file.
    pub fn file(path: &Path) -> Result<Self, SevenZipError> {
        Ok(Self(FileStream::new(File::open(path)?)))
    }

    /// Seeks to the start: what a handler expects when it is offered a
    /// stream another handler has read from.
    fn rewind(&self) -> Result<(), SevenZipError> {
        let mut position: u64 = 0;
        // SAFETY: a live `IInStream` and a valid out-pointer.
        check("IInStream::Seek", unsafe {
            let vtbl = *self.0.0.cast::<*const IInStreamVtbl>();
            ((*vtbl).seek)(self.0.0, 0, 0, &raw mut position)
        })
    }
}

/// One entry of an archive, as the handler describes it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Item {
    /// The path inside the archive, with the handler's separators.
    pub path: String,
    /// A directory.
    pub is_dir: bool,
    /// The unpacked size, when known.
    pub size: Option<u64>,
    /// The packed size, when the format has one.
    pub packed: Option<u64>,
    /// The modification time, when recorded.
    pub modified: Option<SystemTime>,
    /// The content is encrypted.
    pub encrypted: bool,
    /// A symbolic link's target.
    pub symlink: Option<String>,
    /// The Unix mode bits, when recorded (`kpidPosixAttrib`, or the high
    /// half of `kpidAttrib` when its Unix flag is set).
    pub mode: Option<u32>,
}

/// An opened archive: a handler with its stream.
pub struct Archive {
    library: Arc<SevenZip>,
    format: String,
    handler: ComPtr,
    /// The stream the handler reads; it holds its own reference, this one
    /// keeps the drop order plain.
    _source: Stream,
}

impl std::fmt::Debug for Archive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Archive")
            .field("format", &self.format)
            .finish_non_exhaustive()
    }
}

impl Drop for Archive {
    fn drop(&mut self) {
        // SAFETY: `handler` is a live `IInArchive`; `Close` before the
        // release is how the library's own tools end an archive.
        unsafe {
            let vtbl = *self.handler.0.cast::<*const IInArchiveVtbl>();
            let _ = ((*vtbl).close)(self.handler.0);
        }
    }
}

impl Archive {
    /// The handler's name: `VHDX`, `FAT`, …
    #[must_use]
    pub fn format(&self) -> &str {
        &self.format
    }

    /// The library this archive was opened with.
    #[must_use]
    pub fn library(&self) -> &Arc<SevenZip> {
        &self.library
    }

    /// The number of items.
    ///
    /// # Errors
    ///
    /// A call error.
    pub fn len(&self) -> Result<u32, SevenZipError> {
        let mut count: u32 = 0;
        // SAFETY: a live handler and a valid out-pointer.
        check("GetNumberOfItems", unsafe {
            let vtbl = *self.handler.0.cast::<*const IInArchiveVtbl>();
            ((*vtbl).get_number_of_items)(self.handler.0, &raw mut count)
        })?;
        Ok(count)
    }

    /// Whether the archive has no items.
    ///
    /// # Errors
    ///
    /// A call error.
    pub fn is_empty(&self) -> Result<bool, SevenZipError> {
        Ok(self.len()? == 0)
    }

    /// One property of an item, strings decoded.
    ///
    /// # Errors
    ///
    /// A call error.
    pub fn property(&self, index: u32, id: u32) -> Result<Option<Value>, SevenZipError> {
        let mut variant = PropVariant::empty();
        // SAFETY: a live handler and a valid out-variant, cleared by its
        // `Drop`.
        check("GetProperty", unsafe {
            let vtbl = *self.handler.0.cast::<*const IInArchiveVtbl>();
            ((*vtbl).get_property)(self.handler.0, index, id, &raw mut variant)
        })?;
        Ok(variant.text_value())
    }

    /// One property of the archive itself, strings decoded.
    ///
    /// # Errors
    ///
    /// A call error.
    pub fn archive_property(&self, id: u32) -> Result<Option<Value>, SevenZipError> {
        let mut variant = PropVariant::empty();
        // SAFETY: as for `property`.
        check("GetArchiveProperty", unsafe {
            let vtbl = *self.handler.0.cast::<*const IInArchiveVtbl>();
            ((*vtbl).get_archive_property)(self.handler.0, id, &raw mut variant)
        })?;
        Ok(variant.text_value())
    }

    /// The item a container stands for (`kpidMainSubfile`): the disk of a
    /// VHDX or QCOW2, the main partition of a DMG.
    ///
    /// # Errors
    ///
    /// A call error.
    pub fn main_subfile(&self) -> Result<Option<u32>, SevenZipError> {
        Ok(self
            .archive_property(prop::MAIN_SUBFILE)?
            .and_then(|v| v.int())
            .and_then(|n| u32::try_from(n).ok()))
    }

    /// The item at `index`.
    ///
    /// # Errors
    ///
    /// A call error.
    pub fn item(&self, index: u32) -> Result<Item, SevenZipError> {
        let text = |id| -> Result<Option<String>, SevenZipError> {
            Ok(self
                .property(index, id)?
                .and_then(|v| v.text().map(str::to_owned)))
        };
        let int = |id| -> Result<Option<u64>, SevenZipError> {
            Ok(self.property(index, id)?.and_then(|v| v.int()))
        };
        let flag = |id| -> Result<bool, SevenZipError> {
            Ok(self
                .property(index, id)?
                .and_then(|v| v.bool())
                .unwrap_or(false))
        };
        let attrib = int(prop::ATTRIB)?;
        let mode = match int(prop::POSIX_ATTRIB)? {
            Some(mode) => Some(u32::try_from(mode).unwrap_or(0)),
            None => attrib
                .filter(|a| a & 0x8000 != 0)
                .map(|a| u32::try_from(a >> 16).unwrap_or(0)),
        };
        Ok(Item {
            path: text(prop::PATH)?.unwrap_or_default(),
            is_dir: flag(prop::IS_DIR)?,
            size: int(prop::SIZE)?,
            packed: int(prop::PACK_SIZE)?,
            modified: self.property(index, prop::MTIME)?.and_then(|v| v.time()),
            encrypted: flag(prop::ENCRYPTED)?,
            symlink: text(prop::SYMLINK)?.filter(|s| !s.is_empty()),
            mode,
        })
    }

    /// Every item, in the handler's order.
    ///
    /// # Errors
    ///
    /// A call error.
    pub fn items(&self) -> Result<Vec<Item>, SevenZipError> {
        (0..self.len()?).map(|index| self.item(index)).collect()
    }

    /// The content of the item at `index` as a seekable stream, for a
    /// container's disk or a partition that is an archive of its own.
    ///
    /// # Errors
    ///
    /// [`SevenZipError::NoStream`] when the handler gives none; a call
    /// error otherwise.
    pub fn sub_stream(&self, index: u32) -> Result<Stream, SevenZipError> {
        // SAFETY: a live handler.
        let Some(get_stream) =
            (unsafe { query_interface(self.handler.0, &IID_IIN_ARCHIVE_GET_STREAM) })
        else {
            return Err(SevenZipError::NoStream(self.format.clone(), index));
        };
        let get_stream = ComPtr(get_stream);
        let mut sequential: *mut c_void = std::ptr::null_mut();
        // SAFETY: an `IInArchiveGetStream` and a valid out-pointer.
        let code = unsafe {
            let vtbl = *get_stream.0.cast::<*const IInArchiveGetStreamVtbl>();
            ((*vtbl).get_stream)(get_stream.0, index, &raw mut sequential)
        };
        if code != S_OK || sequential.is_null() {
            return Err(SevenZipError::NoStream(self.format.clone(), index));
        }
        let sequential = ComPtr(sequential);
        // SAFETY: a live stream object.
        let seekable = unsafe { query_interface(sequential.0, &IID_IIN_STREAM) }
            .ok_or_else(|| SevenZipError::NoStream(self.format.clone(), index))?;
        Ok(Stream(ComPtr(seekable)))
    }

    /// Writes the content of the item at `index` into `sink`.
    ///
    /// # Errors
    ///
    /// [`SevenZipError::Extract`] when the handler reports the item was
    /// not extracted whole, the sink's I/O error, or a call error.
    pub fn extract(&self, index: u32, sink: &mut dyn Write) -> Result<(), SevenZipError> {
        // SAFETY: the sink pointer is used only by the output stream's
        // `Write` during the `Extract` call below, which borrows `sink`
        // for its whole duration, and is cleared before this function
        // returns; the lifetime is erased for the `'static` object only.
        let erased: *mut (dyn Write + 'static) = unsafe {
            std::mem::transmute::<*mut (dyn Write + '_), *mut (dyn Write + 'static)>(
                std::ptr::from_mut::<dyn Write>(sink),
            )
        };
        let out = OutStream::new(erased);
        let callback = ExtractCallback::new(index, out.0);
        let indices = [index];
        // SAFETY: a live handler; the indices outlive the call; the
        // callback is a live COM object.
        let code = unsafe {
            let vtbl = *self.handler.0.cast::<*const IInArchiveVtbl>();
            ((*vtbl).extract)(self.handler.0, indices.as_ptr(), 1, 0, callback.0)
        };
        // SAFETY: both objects are ours and live; the library has returned
        // and may hold no reference that writes after this point, so the
        // sink is forgotten before `sink`'s borrow ends.
        let (result, error) = unsafe {
            let stream = &mut *out.0.cast::<OutStream>();
            stream.sink = None;
            let error = stream.error.take();
            let callback = &*callback.0.cast::<ExtractCallback>();
            (callback.result, error)
        };
        if let Some(error) = error {
            return Err(SevenZipError::Io(error));
        }
        check("IInArchive::Extract", code)?;
        match result {
            Some(0) => Ok(()),
            Some(other) => Err(SevenZipError::Extract(operation_result_text(other))),
            None => Err(SevenZipError::Extract("the item was not extracted")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The library for the tests that need it: the one CI put in
    /// `RICK_SEVENZIP_DIR` (test infrastructure), else the usual
    /// locations; `None` with a note otherwise.
    fn library() -> Option<Arc<SevenZip>> {
        let path = std::env::var_os("RICK_SEVENZIP_DIR")
            .map(|dir| PathBuf::from(dir).join(LIBRARY_FILE))
            .into_iter()
            .chain(usual_locations())
            .find(|p| p.is_file());
        let Some(path) = path else {
            eprintln!("no 7-Zip library on this machine; the binding test is skipped");
            assert!(
                std::env::var_os("RICK_REQUIRE_SEVENZIP").is_none(),
                "the 7-Zip library is required here but was not found"
            );
            return None;
        };
        Some(SevenZip::load(&path).unwrap_or_else(|e| panic!("{e}")))
    }

    /// A zip of two stored entries, written by hand.
    fn stored_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        fn crc32(bytes: &[u8]) -> u32 {
            let mut crc = 0xFFFF_FFFFu32;
            for &b in bytes {
                crc ^= u32::from(b);
                for _ in 0..8 {
                    crc = if crc & 1 == 1 {
                        (crc >> 1) ^ 0xEDB8_8320
                    } else {
                        crc >> 1
                    };
                }
            }
            !crc
        }
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data) in entries {
            let offset = u32::try_from(out.len()).unwrap_or(0);
            let crc = crc32(data);
            let len = u32::try_from(data.len()).unwrap_or(0);
            let name_len = u16::try_from(name.len()).unwrap_or(0);
            out.extend_from_slice(b"PK\x03\x04");
            out.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0x04, 0x39, 0xA6, 0x58]);
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(&name_len.to_le_bytes());
            out.extend_from_slice(&[0, 0]);
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(data);
            central.extend_from_slice(b"PK\x01\x02");
            central.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0x04, 0x39, 0xA6, 0x58]);
            central.extend_from_slice(&crc.to_le_bytes());
            central.extend_from_slice(&len.to_le_bytes());
            central.extend_from_slice(&len.to_le_bytes());
            central.extend_from_slice(&name_len.to_le_bytes());
            central.extend_from_slice(&[0; 8]);
            central.extend_from_slice(&[0, 0, 0, 0]);
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let central_offset = u32::try_from(out.len()).unwrap_or(0);
        let central_len = u32::try_from(central.len()).unwrap_or(0);
        let count = u16::try_from(entries.len()).unwrap_or(0);
        out.extend_from_slice(&central);
        out.extend_from_slice(b"PK\x05\x06");
        out.extend_from_slice(&[0, 0, 0, 0]);
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&central_len.to_le_bytes());
        out.extend_from_slice(&central_offset.to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        out
    }

    #[test]
    fn ids_and_variants_are_laid_out_as_the_library_expects() {
        assert_eq!(std::mem::size_of::<PropVariant>(), 24);
        assert_eq!(std::mem::size_of::<Guid>(), 16);
        assert_eq!(IID_IIN_ARCHIVE.data4, [0, 0, 0, 6, 0, 0x60, 0, 0]);
        let bytes: Vec<u8> = (0u8..16).collect();
        let guid = Guid::from_bytes(&bytes).unwrap_or_else(|| panic!("16 bytes"));
        assert_eq!(guid.data1, 0x0302_0100);
        assert_eq!(guid.data2, 0x0504);
        assert_eq!(guid.data4, [8, 9, 10, 11, 12, 13, 14, 15]);
        assert_eq!(Guid::from_bytes(&bytes[..5]), None);
        let time = PropVariant {
            vt: VT_FILETIME,
            reserved: [0; 3],
            data: [FILETIME_UNIX_EPOCH + 10_000_000, 0],
        };
        assert_eq!(
            time.value(),
            Some(Value::Time(SystemTime::UNIX_EPOCH + Duration::from_secs(1)))
        );
        let flag = PropVariant {
            vt: VT_BOOL,
            reserved: [0; 3],
            data: [0xFFFF, 0],
        };
        assert_eq!(flag.value(), Some(Value::Bool(true)));
        assert_eq!(PropVariant::empty().value(), None);
        assert_eq!(operation_result_text(2), "data error");
        assert_eq!(LIBRARY_FILE, if cfg!(windows) { "7z.dll" } else { "7z.so" });
        assert!(!install_hint().is_empty());
        assert!(!usual_locations().is_empty() || cfg!(windows));
    }

    #[test]
    fn a_missing_or_foreign_library_is_reported() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let missing = dir.path().join(LIBRARY_FILE);
        assert!(matches!(
            SevenZip::load(&missing),
            Err(SevenZipError::Load { path, .. }) if path == missing
        ));
        let text = dir.path().join("not-a-library.txt");
        std::fs::write(&text, b"hello").unwrap_or_else(|e| panic!("{e}"));
        assert!(SevenZip::load(&text).is_err());
    }

    #[test]
    fn the_library_lists_its_formats_and_opens_a_zip() {
        let Some(library) = library() else {
            return;
        };
        println!("7-Zip library: {}", library.path().display());
        let names: Vec<&str> = library.formats().iter().map(|f| f.name.as_str()).collect();
        println!("formats: {names:?}");
        for wanted in [
            "zip", "VHDX", "VHD", "QCOW", "VDI", "VMDK", "Dmg", "FAT", "NTFS", "Ext", "HFS",
            "APFS", "GPT", "MBR", "Iso", "Udf",
        ] {
            assert!(
                library.format(wanted).is_some(),
                "{wanted} missing from {names:?}"
            );
        }
        let zip = library.format("ZIP").unwrap_or_else(|| panic!("zip"));
        assert!(
            zip.extensions.iter().any(|e| e == "zip"),
            "{:?}",
            zip.extensions
        );
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let path = dir.path().join("two.zip");
        std::fs::write(
            &path,
            stored_zip(&[("docs/readme.txt", b"hello hello"), ("a.bin", &[1, 2, 3])]),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        let archive = library
            .open_file(&path, zip)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(archive.format(), "zip");
        assert_eq!(archive.len().ok(), Some(2));
        let items = archive.items().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(items[0].path.replace('\\', "/"), "docs/readme.txt");
        assert_eq!(items[0].size, Some(11));
        assert!(!items[0].is_dir);
        assert!(items[0].modified.is_some());
        assert_eq!(items[1].path, "a.bin");
        let mut out = Vec::new();
        archive
            .extract(0, &mut out)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(out, b"hello hello");
        out.clear();
        archive
            .extract(1, &mut out)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(out, [1, 2, 3]);
        assert!(
            archive
                .main_subfile()
                .unwrap_or_else(|e| panic!("{e}"))
                .is_none()
        );
        // A zip entry has no stream of its own to open as an archive.
        assert!(matches!(
            archive.sub_stream(0),
            Err(SevenZipError::NoStream(..))
        ));
        drop(archive);
        // Data that is not a zip is refused as such, not as an error.
        let text = dir.path().join("text.zip");
        std::fs::write(&text, b"just text, no archive").unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(
            library.open_file(&text, zip),
            Err(SevenZipError::NotArchive(name)) if name == "zip"
        ));
        assert!(format!("{library:?}").contains("formats"));
    }
}
