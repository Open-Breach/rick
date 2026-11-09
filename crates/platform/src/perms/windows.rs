//! Windows: the owner, the primary group and the NTFS discretionary ACL
//! of an entry through the security API; there are no mode bits. The users
//! and local groups come from the Net API, the well-known trustees
//! (Everyone, SYSTEM, Administrators, Users, Authenticated Users) from
//! their fixed SIDs, named as the system names them.

use std::ffi::c_void;
use std::io;
use std::path::Path;

use types::Error;
use windows::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, HLOCAL, LocalFree, WIN32_ERROR,
};
use windows::Win32::NetworkManagement::NetManagement::{
    FILTER_NORMAL_ACCOUNT, LOCALGROUP_INFO_0, MAX_PREFERRED_LENGTH, NERR_Success, NetApiBufferFree,
    NetLocalGroupEnum, NetUserEnum, USER_INFO_0,
};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSidToSidW, GetNamedSecurityInfoW, SE_FILE_OBJECT,
    SetNamedSecurityInfoW,
};
use windows::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACE_FLAGS, ACE_HEADER, ACL, ACL_REVISION, ACL_SIZE_INFORMATION,
    AclSizeInformation, AddAccessAllowedAceEx, AddAccessDeniedAceEx, DACL_SECURITY_INFORMATION,
    GROUP_SECURITY_INFORMATION, GetAce, GetAclInformation, GetLengthSid,
    GetSecurityDescriptorControl, InitializeAcl, LookupAccountNameW, LookupAccountSidW,
    OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
    SE_DACL_PROTECTED, SID_NAME_USE, UNPROTECTED_DACL_SECURITY_INFORMATION,
};
use windows::core::{PCWSTR, PWSTR};

use super::{Acl, FilePermissions, NtAce, PermissionChange, Principal, PrincipalKind, Principals};

/// `ACCESS_ALLOWED_ACE_TYPE`.
const ACE_ALLOWED: u8 = 0;
/// `ACCESS_DENIED_ACE_TYPE`.
const ACE_DENIED: u8 = 1;

/// The fixed SIDs every machine has, listed with the accounts.
const WELL_KNOWN: &[&str] = &[
    "S-1-1-0",      // Everyone
    "S-1-5-11",     // Authenticated Users
    "S-1-5-18",     // SYSTEM
    "S-1-5-32-544", // Administrators
    "S-1-5-32-545", // Users
];

/// A path as a NUL-terminated UTF-16 string.
fn wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt as _;
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

fn wide_str(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// A Win32 error as the kernel error.
fn win32(error: WIN32_ERROR) -> Error {
    match error {
        ERROR_ACCESS_DENIED => Error::PermissionDenied,
        ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => Error::NotFound,
        other => Error::Io(io::Error::from_raw_os_error(
            i32::try_from(other.0).unwrap_or(i32::MAX),
        )),
    }
}

fn core_error(error: &windows::core::Error) -> Error {
    win32(WIN32_ERROR::from_error(error).unwrap_or(WIN32_ERROR(u32::MAX)))
}

/// Memory the security API allocated with `LocalAlloc`, freed on drop.
struct Local(*mut c_void);

impl Drop for Local {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the pointer came from the security API, which allocates
            // with LocalAlloc, and is freed once.
            unsafe { LocalFree(Some(HLOCAL(self.0))) };
        }
    }
}

/// The security descriptor of an entry: owner, group and DACL.
struct Descriptor {
    owner: PSID,
    group: PSID,
    dacl: *mut ACL,
    _memory: Local,
    protected: bool,
}

fn descriptor(path: &Path) -> Result<Descriptor, Error> {
    let path = wide(path);
    let mut owner = PSID::default();
    let mut group = PSID::default();
    let mut dacl: *mut ACL = std::ptr::null_mut();
    let mut sd = PSECURITY_DESCRIPTOR::default();
    // SAFETY: every out-pointer names a local; the descriptor is freed by
    // `Local` when the `Descriptor` drops, and the owner, group and DACL
    // point into it.
    let status = unsafe {
        GetNamedSecurityInfoW(
            PCWSTR(path.as_ptr()),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            Some(&raw mut owner),
            Some(&raw mut group),
            Some(&raw mut dacl),
            None,
            &raw mut sd,
        )
    };
    if status != WIN32_ERROR(0) {
        return Err(win32(status));
    }
    let mut control: u16 = 0;
    let mut revision: u32 = 0;
    // SAFETY: `sd` is the live descriptor; the out-pointers name locals.
    let protected =
        unsafe { GetSecurityDescriptorControl(sd, &raw mut control, &raw mut revision) }.is_ok()
            && control & SE_DACL_PROTECTED.0 != 0;
    Ok(Descriptor {
        owner,
        group,
        dacl,
        _memory: Local(sd.0),
        protected,
    })
}

/// The SID as `S-1-…`.
fn sid_text(sid: PSID) -> Option<String> {
    if sid.0.is_null() {
        return None;
    }
    let mut text = PWSTR::null();
    // SAFETY: `sid` is a valid SID the caller owns; the string is freed
    // through `Local`.
    unsafe { ConvertSidToStringSidW(sid, &raw mut text) }.ok()?;
    let _free = Local(text.0.cast());
    // SAFETY: the string is NUL-terminated.
    Some(unsafe { text.to_string() }.unwrap_or_default())
}

/// `DOMAIN\Name` of a SID, when the system can resolve it.
fn account_name(sid: PSID) -> Option<String> {
    let mut name_len: u32 = 0;
    let mut domain_len: u32 = 0;
    let mut use_ = SID_NAME_USE::default();
    // SAFETY: a sizing call: the buffers are absent and the lengths come back.
    let _ = unsafe {
        LookupAccountSidW(
            PCWSTR::null(),
            sid,
            None,
            &raw mut name_len,
            None,
            &raw mut domain_len,
            &raw mut use_,
        )
    };
    if name_len == 0 {
        return None;
    }
    let mut name = vec![0u16; name_len as usize];
    let mut domain = vec![0u16; domain_len.max(1) as usize];
    // SAFETY: the buffers are as long as the sizing call asked for.
    unsafe {
        LookupAccountSidW(
            PCWSTR::null(),
            sid,
            Some(PWSTR(name.as_mut_ptr())),
            &raw mut name_len,
            Some(PWSTR(domain.as_mut_ptr())),
            &raw mut domain_len,
            &raw mut use_,
        )
    }
    .ok()?;
    let name = String::from_utf16_lossy(&name[..name.iter().position(|&c| c == 0).unwrap_or(0)]);
    let domain =
        String::from_utf16_lossy(&domain[..domain.iter().position(|&c| c == 0).unwrap_or(0)]);
    Some(if domain.is_empty() {
        name
    } else {
        format!("{domain}\\{name}")
    })
}

fn principal_of(sid: PSID) -> Principal {
    Principal {
        id: sid_text(sid).unwrap_or_default(),
        name: account_name(sid),
    }
}

/// The SID of a principal: its id when it is an SID, else its name looked
/// up. Freed through the returned `Local`.
fn sid_of(principal: &Principal) -> Result<(PSID, Local), Error> {
    let id = principal.id.trim();
    if id.starts_with("S-") {
        let text = wide_str(id);
        let mut sid = PSID::default();
        // SAFETY: `text` is NUL-terminated; the SID is freed through `Local`.
        unsafe { ConvertStringSidToSidW(PCWSTR(text.as_ptr()), &raw mut sid) }
            .map_err(|e| core_error(&e))?;
        return Ok((sid, Local(sid.0)));
    }
    let name = principal.name.as_deref().unwrap_or(id);
    lookup_name(name)
}

/// The SID of an account name.
fn lookup_name(name: &str) -> Result<(PSID, Local), Error> {
    let name = wide_str(name);
    let mut sid_len: u32 = 0;
    let mut domain_len: u32 = 0;
    let mut use_ = SID_NAME_USE::default();
    // SAFETY: a sizing call.
    let _ = unsafe {
        LookupAccountNameW(
            PCWSTR::null(),
            PCWSTR(name.as_ptr()),
            None,
            &raw mut sid_len,
            None,
            &raw mut domain_len,
            &raw mut use_,
        )
    };
    if sid_len == 0 {
        return Err(Error::NotFound);
    }
    // SAFETY: a plain allocation the size the sizing call asked for.
    let memory = unsafe {
        windows::Win32::System::Memory::LocalAlloc(
            windows::Win32::System::Memory::LPTR,
            sid_len as usize,
        )
    }
    .map_err(|e| core_error(&e))?;
    let local = Local(memory.0);
    let sid = PSID(memory.0);
    let mut domain = vec![0u16; domain_len.max(1) as usize];
    // SAFETY: the buffers are as long as the sizing call asked for.
    unsafe {
        LookupAccountNameW(
            PCWSTR::null(),
            PCWSTR(name.as_ptr()),
            Some(sid),
            &raw mut sid_len,
            Some(PWSTR(domain.as_mut_ptr())),
            &raw mut domain_len,
            &raw mut use_,
        )
    }
    .map_err(|_| Error::NotFound)?;
    Ok((sid, local))
}

/// The entries of a DACL.
fn aces(dacl: *mut ACL) -> Result<Vec<NtAce>, Error> {
    if dacl.is_null() {
        return Ok(Vec::new());
    }
    let mut info = ACL_SIZE_INFORMATION::default();
    // SAFETY: `dacl` is a live ACL and `info` the structure the class names.
    unsafe {
        GetAclInformation(
            dacl,
            (&raw mut info).cast(),
            u32::try_from(std::mem::size_of::<ACL_SIZE_INFORMATION>()).unwrap_or(0),
            AclSizeInformation,
        )
    }
    .map_err(|e| core_error(&e))?;
    let mut entries = Vec::with_capacity(info.AceCount as usize);
    for index in 0..info.AceCount {
        let mut ace: *mut c_void = std::ptr::null_mut();
        // SAFETY: `index` is below the count the ACL reported.
        unsafe { GetAce(dacl, index, &raw mut ace) }.map_err(|e| core_error(&e))?;
        // SAFETY: every ACE starts with an `ACE_HEADER`.
        let header = unsafe { *ace.cast::<ACE_HEADER>() };
        let allow = match header.AceType {
            ACE_ALLOWED => true,
            ACE_DENIED => false,
            // Object and callback ACEs do not apply to files; kept out.
            _ => continue,
        };
        // SAFETY: allowed and denied ACEs share the `ACCESS_ALLOWED_ACE`
        // layout, with the SID starting at `SidStart`.
        let typed = unsafe { &*ace.cast::<ACCESS_ALLOWED_ACE>() };
        let sid = PSID((&raw const typed.SidStart).cast_mut().cast());
        entries.push(NtAce {
            allow,
            trustee: principal_of(sid),
            mask: typed.Mask,
            flags: header.AceFlags,
        });
    }
    Ok(entries)
}

pub(super) fn permissions(path: &Path) -> Result<FilePermissions, Error> {
    let meta = std::fs::symlink_metadata(path).map_err(Error::from_io)?;
    let descriptor = descriptor(path)?;
    let acl = match aces(descriptor.dacl) {
        Ok(entries) => Some(Acl::Nt {
            entries,
            protected: descriptor.protected,
        }),
        Err(Error::Io(_)) => None,
        Err(error) => return Err(error),
    };
    Ok(FilePermissions {
        owner: principal_of(descriptor.owner),
        group: principal_of(descriptor.group),
        mode: None,
        acl,
        is_dir: meta.is_dir(),
    })
}

pub(super) fn set_permissions(path: &Path, change: &PermissionChange) -> Result<(), Error> {
    let wide_path = wide(path);
    if let Some(acl) = &change.acl {
        let Acl::Nt { entries, protected } = acl else {
            return Err(Error::Unsupported(
                "this file system has NTFS permissions, not this kind",
            ));
        };
        write_dacl(&wide_path, entries, *protected)?;
    }
    if change.owner.is_some() || change.group.is_some() {
        let owner = change.owner.as_ref().map(sid_of).transpose()?;
        let group = change.group.as_ref().map(sid_of).transpose()?;
        let mut what = windows::Win32::Security::OBJECT_SECURITY_INFORMATION(0);
        if owner.is_some() {
            what |= OWNER_SECURITY_INFORMATION;
        }
        if group.is_some() {
            what |= GROUP_SECURITY_INFORMATION;
        }
        // SAFETY: the SIDs live in their `Local`s until the end of the call.
        let status = unsafe {
            SetNamedSecurityInfoW(
                PCWSTR(wide_path.as_ptr()),
                SE_FILE_OBJECT,
                what,
                owner.as_ref().map(|(sid, _)| *sid),
                group.as_ref().map(|(sid, _)| *sid),
                None,
                None,
            )
        };
        if status != WIN32_ERROR(0) {
            return Err(win32(status));
        }
    }
    Ok(())
}

/// Replaces the DACL with the entries that are not inherited; the inherited
/// ones come back from the parent unless `protected`.
fn write_dacl(path: &[u16], entries: &[NtAce], protected: bool) -> Result<(), Error> {
    let own: Vec<&NtAce> = entries.iter().filter(|e| !e.inherited()).collect();
    let mut sids = Vec::with_capacity(own.len());
    let mut size = std::mem::size_of::<ACL>();
    for entry in &own {
        let (sid, local) = sid_of(&entry.trustee)?;
        // SAFETY: `sid` is a valid SID held by `local`.
        let length = unsafe { GetLengthSid(sid) } as usize;
        size += std::mem::size_of::<ACCESS_ALLOWED_ACE>() - std::mem::size_of::<u32>() + length;
        sids.push((sid, local));
    }
    // The ACL wants a size in whole DWORDs.
    let size = size.div_ceil(4) * 4;
    let mut buffer = vec![0u32; size / 4];
    let acl: *mut ACL = buffer.as_mut_ptr().cast();
    // SAFETY: `buffer` is `size` bytes, DWORD-aligned.
    unsafe { InitializeAcl(acl, u32::try_from(size).unwrap_or(u32::MAX), ACL_REVISION) }
        .map_err(|e| core_error(&e))?;
    for (entry, (sid, _)) in own.iter().zip(&sids) {
        let flags = ACE_FLAGS(u32::from(entry.flags & !NtAce::INHERITED));
        // SAFETY: `acl` was initialised with room for every entry; `sid` is
        // valid for the call.
        let added = unsafe {
            if entry.allow {
                AddAccessAllowedAceEx(acl, ACL_REVISION, flags, entry.mask, *sid)
            } else {
                AddAccessDeniedAceEx(acl, ACL_REVISION, flags, entry.mask, *sid)
            }
        };
        added.map_err(|e| core_error(&e))?;
    }
    let inheritance = if protected {
        PROTECTED_DACL_SECURITY_INFORMATION
    } else {
        UNPROTECTED_DACL_SECURITY_INFORMATION
    };
    // SAFETY: `path` is NUL-terminated and `acl` a valid ACL in `buffer`.
    let status = unsafe {
        SetNamedSecurityInfoW(
            PCWSTR(path.as_ptr()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | inheritance,
            None,
            None,
            Some(acl),
            None,
        )
    };
    if status != WIN32_ERROR(0) {
        return Err(win32(status));
    }
    Ok(())
}

/// The accounts the Net API lists at `level` 0, read from `buffer`.
unsafe fn names<T>(buffer: *mut u8, count: u32, name_of: impl Fn(&T) -> PWSTR) -> Vec<String> {
    let mut names = Vec::with_capacity(count as usize);
    for index in 0..count as usize {
        // SAFETY: the caller guarantees `count` records of `T` at `buffer`.
        let record = unsafe { &*buffer.cast::<T>().add(index) };
        let name = name_of(record);
        if !name.is_null() {
            // SAFETY: the Net API's strings are NUL-terminated.
            if let Ok(text) = unsafe { name.to_string() } {
                names.push(text);
            }
        }
    }
    names
}

fn local_users() -> Vec<String> {
    let mut buffer: *mut u8 = std::ptr::null_mut();
    let mut read: u32 = 0;
    let mut total: u32 = 0;
    // SAFETY: the out-pointers name locals; the buffer is freed below.
    let status = unsafe {
        NetUserEnum(
            PCWSTR::null(),
            0,
            FILTER_NORMAL_ACCOUNT,
            &raw mut buffer,
            MAX_PREFERRED_LENGTH,
            &raw mut read,
            &raw mut total,
            None,
        )
    };
    if status != NERR_Success || buffer.is_null() {
        return Vec::new();
    }
    // SAFETY: `read` records of `USER_INFO_0` are in `buffer`.
    let names = unsafe { names::<USER_INFO_0>(buffer, read, |u| u.usri0_name) };
    // SAFETY: the buffer came from the Net API and is freed once.
    unsafe { NetApiBufferFree(Some(buffer.cast())) };
    names
}

fn local_groups() -> Vec<String> {
    let mut buffer: *mut u8 = std::ptr::null_mut();
    let mut read: u32 = 0;
    let mut total: u32 = 0;
    // SAFETY: as in `local_users`.
    let status = unsafe {
        NetLocalGroupEnum(
            PCWSTR::null(),
            0,
            &raw mut buffer,
            MAX_PREFERRED_LENGTH,
            &raw mut read,
            &raw mut total,
            None,
        )
    };
    if status != NERR_Success || buffer.is_null() {
        return Vec::new();
    }
    // SAFETY: `read` records of `LOCALGROUP_INFO_0` are in `buffer`.
    let names = unsafe { names::<LOCALGROUP_INFO_0>(buffer, read, |g| g.lgrpi0_name) };
    // SAFETY: the buffer came from the Net API and is freed once.
    unsafe { NetApiBufferFree(Some(buffer.cast())) };
    names
}

// The same signature as the Linux and macOS versions, which can fail.
#[allow(clippy::unnecessary_wraps)]
pub(super) fn principals() -> Result<Principals, Error> {
    let mut users: Vec<Principal> = local_users()
        .iter()
        .filter_map(|name| lookup_name(name).ok())
        .map(|(sid, _local)| principal_of(sid))
        .collect();
    let mut groups: Vec<Principal> = local_groups()
        .iter()
        .filter_map(|name| lookup_name(name).ok())
        .map(|(sid, _local)| principal_of(sid))
        .collect();
    for text in WELL_KNOWN {
        if let Ok((sid, _local)) = sid_of(&Principal::id(*text)) {
            groups.push(principal_of(sid));
        }
    }
    users.sort_by(|a, b| a.display().cmp(b.display()));
    groups.sort_by(|a, b| a.display().cmp(b.display()));
    groups.dedup();
    Ok(Principals { users, groups })
}

pub(super) fn principal(name: &str, _kind: PrincipalKind) -> Result<Principal, Error> {
    let (sid, _local) = sid_of(&Principal::id(name.trim()))?;
    Ok(principal_of(sid))
}
