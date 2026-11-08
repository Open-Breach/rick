//! Ownership and mode bits on Linux and macOS, and the user and group
//! databases through the C library, so names resolve the way the system
//! resolves them (local files, LDAP, Open Directory alike).

use std::ffi::{CStr, CString};
use std::io;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::path::Path;
use std::sync::Mutex;

use types::Error;

use super::{Principal, PrincipalKind, Principals};

/// What `lstat` says about an entry's identity and mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Stat {
    pub(super) uid: u32,
    pub(super) gid: u32,
    /// The permission bits, special bits included.
    pub(super) mode: u32,
    pub(super) is_dir: bool,
    pub(super) is_symlink: bool,
}

pub(super) fn stat(path: &Path) -> Result<Stat, Error> {
    let meta = std::fs::symlink_metadata(path).map_err(Error::from_io)?;
    Ok(Stat {
        uid: meta.uid(),
        gid: meta.gid(),
        mode: meta.mode() & 0o7777,
        is_dir: meta.is_dir(),
        is_symlink: meta.file_type().is_symlink(),
    })
}

/// Changes the owner and group of the entry itself (a link is not
/// followed); `None` keeps.
pub(super) fn set_owner(path: &Path, uid: Option<u32>, gid: Option<u32>) -> Result<(), Error> {
    if uid.is_none() && gid.is_none() {
        return Ok(());
    }
    std::os::unix::fs::lchown(path, uid, gid).map_err(Error::from_io)
}

/// Sets the mode bits of a file or folder (never a link, which has none
/// of its own on Linux).
pub(super) fn set_mode(path: &Path, mode: u32) -> Result<(), Error> {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode & 0o7777))
        .map_err(Error::from_io)
}

/// A principal for a uid or gid, named when the database knows it.
pub(super) fn principal_of(id: u32, kind: PrincipalKind) -> Principal {
    let name = match kind {
        PrincipalKind::User => user_name(id),
        PrincipalKind::Group => group_name(id),
    };
    Principal {
        id: id.to_string(),
        name,
    }
}

/// The principal for a typed name, or a decimal id.
pub(super) fn principal(name: &str, kind: PrincipalKind) -> Result<Principal, Error> {
    let name = name.trim();
    if let Ok(id) = name.parse::<u32>() {
        return Ok(principal_of(id, kind));
    }
    let id = match kind {
        PrincipalKind::User => user_id(name),
        PrincipalKind::Group => group_id(name),
    };
    id.map(|id| Principal::named(id.to_string(), name))
        .ok_or(Error::NotFound)
}

/// The users and groups of the system, sorted by name.
pub(super) fn principals() -> Principals {
    let mut users = list_users();
    let mut groups = list_groups();
    users.sort_by(|a, b| a.display().cmp(b.display()));
    groups.sort_by(|a, b| a.display().cmp(b.display()));
    Principals { users, groups }
}

/// The buffer size the reentrant lookups want.
fn lookup_buffer() -> Vec<u8> {
    // SAFETY: `sysconf` reads a constant; no pointers are involved.
    let hint = unsafe { libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX) };
    let size = usize::try_from(hint)
        .ok()
        .filter(|&n| n > 0)
        .unwrap_or(16 * 1024);
    vec![0u8; size]
}

/// The text at `ptr`, which the C library owns for the duration of the
/// call that produced it.
unsafe fn text(ptr: *const libc::c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: the caller guarantees `ptr` is a NUL-terminated string that
    // outlives this call.
    Some(
        unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned(),
    )
}

/// The name of user `uid`.
pub(super) fn user_name(uid: u32) -> Option<String> {
    let mut buffer = lookup_buffer();
    loop {
        let mut record = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut found: *mut libc::passwd = std::ptr::null_mut();
        // SAFETY: every pointer names storage that outlives the call;
        // `buffer` is the scratch space the record's strings go to.
        let status = unsafe {
            libc::getpwuid_r(
                uid,
                record.as_mut_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &raw mut found,
            )
        };
        if status == libc::ERANGE {
            buffer.resize(buffer.len() * 2, 0);
            continue;
        }
        if status != 0 || found.is_null() {
            return None;
        }
        // SAFETY: `found` points at `record`, which the call filled in.
        return unsafe { text((*found).pw_name) };
    }
}

/// The name of group `gid`.
pub(super) fn group_name(gid: u32) -> Option<String> {
    let mut buffer = lookup_buffer();
    loop {
        let mut record = std::mem::MaybeUninit::<libc::group>::uninit();
        let mut found: *mut libc::group = std::ptr::null_mut();
        // SAFETY: as in `user_name`.
        let status = unsafe {
            libc::getgrgid_r(
                gid,
                record.as_mut_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &raw mut found,
            )
        };
        if status == libc::ERANGE {
            buffer.resize(buffer.len() * 2, 0);
            continue;
        }
        if status != 0 || found.is_null() {
            return None;
        }
        // SAFETY: `found` points at `record`, which the call filled in.
        return unsafe { text((*found).gr_name) };
    }
}

/// The uid of the user named `name`.
pub(super) fn user_id(name: &str) -> Option<u32> {
    let name = CString::new(name).ok()?;
    let mut buffer = lookup_buffer();
    loop {
        let mut record = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut found: *mut libc::passwd = std::ptr::null_mut();
        // SAFETY: as in `user_name`; `name` is NUL-terminated.
        let status = unsafe {
            libc::getpwnam_r(
                name.as_ptr(),
                record.as_mut_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &raw mut found,
            )
        };
        if status == libc::ERANGE {
            buffer.resize(buffer.len() * 2, 0);
            continue;
        }
        if status != 0 || found.is_null() {
            return None;
        }
        // SAFETY: `found` points at `record`, which the call filled in.
        return Some(unsafe { (*found).pw_uid });
    }
}

/// The gid of the group named `name`.
pub(super) fn group_id(name: &str) -> Option<u32> {
    let name = CString::new(name).ok()?;
    let mut buffer = lookup_buffer();
    loop {
        let mut record = std::mem::MaybeUninit::<libc::group>::uninit();
        let mut found: *mut libc::group = std::ptr::null_mut();
        // SAFETY: as in `user_name`; `name` is NUL-terminated.
        let status = unsafe {
            libc::getgrnam_r(
                name.as_ptr(),
                record.as_mut_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &raw mut found,
            )
        };
        if status == libc::ERANGE {
            buffer.resize(buffer.len() * 2, 0);
            continue;
        }
        if status != 0 || found.is_null() {
            return None;
        }
        // SAFETY: `found` points at `record`, which the call filled in.
        return Some(unsafe { (*found).gr_gid });
    }
}

/// The enumerations of the user database are not reentrant; one at a time.
static ENUMERATING: Mutex<()> = Mutex::new(());

/// Every user account.
#[cfg(target_os = "macos")]
pub(super) fn list_users() -> Vec<Principal> {
    let _guard = ENUMERATING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut users = Vec::new();
    // SAFETY: the enumeration runs under the lock; each record the library
    // hands back is read before the next call.
    unsafe {
        libc::setpwent();
        loop {
            let record = libc::getpwent();
            if record.is_null() {
                break;
            }
            if let Some(name) = text((*record).pw_name)
                && !name.starts_with('_')
            {
                users.push(Principal::named((*record).pw_uid.to_string(), name));
            }
        }
        libc::endpwent();
    }
    users
}

/// Every group.
#[cfg(target_os = "macos")]
pub(super) fn list_groups() -> Vec<Principal> {
    let _guard = ENUMERATING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut groups = Vec::new();
    // SAFETY: as in `list_users`.
    unsafe {
        libc::setgrent();
        loop {
            let record = libc::getgrent();
            if record.is_null() {
                break;
            }
            if let Some(name) = text((*record).gr_name)
                && !name.starts_with('_')
            {
                groups.push(Principal::named((*record).gr_gid.to_string(), name));
            }
        }
        libc::endgrent();
    }
    groups
}

/// Every user account of `/etc/passwd`.
#[cfg(not(target_os = "macos"))]
pub(super) fn list_users() -> Vec<Principal> {
    let _guard = ENUMERATING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    database("/etc/passwd")
}

/// Every group of `/etc/group`.
#[cfg(not(target_os = "macos"))]
pub(super) fn list_groups() -> Vec<Principal> {
    let _guard = ENUMERATING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    database("/etc/group")
}

/// The `name:x:id:` records of a passwd-style file.
#[cfg(not(target_os = "macos"))]
fn database(path: &str) -> Vec<Principal> {
    std::fs::read_to_string(path)
        .map(|text| parse_database(&text))
        .unwrap_or_default()
}

/// The names and ids of the records in a passwd-style text.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub(super) fn parse_database(text: &str) -> Vec<Principal> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split(':');
            let name = fields.next()?;
            let id: u32 = fields.nth(1)?.trim().parse().ok()?;
            (!name.is_empty() && !name.starts_with('#'))
                .then(|| Principal::named(id.to_string(), name))
        })
        .collect()
}

/// The path as the C library wants it.
pub(super) fn c_path(path: &Path) -> io::Result<CString> {
    CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "a path with a NUL byte"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_database_parser_takes_names_and_ids() {
        let users = parse_database(
            "root:x:0:0:root:/root:/bin/bash\n# a comment\n\nme:x:1000:1000::/home/me:/bin/sh\nbroken:x:abc:1\n",
        );
        assert_eq!(
            users,
            [
                Principal::named("0", "root"),
                Principal::named("1000", "me")
            ]
        );
    }

    #[test]
    fn root_resolves_both_ways() {
        assert_eq!(user_name(0).as_deref(), Some("root"));
        assert_eq!(user_id("root"), Some(0));
        assert!(group_name(0).is_some());
        assert_eq!(
            principal("0", PrincipalKind::User).unwrap_or_else(|e| panic!("{e}")),
            Principal::named("0", "root")
        );
        assert!(matches!(
            principal("no-such-user-here", PrincipalKind::User),
            Err(Error::NotFound)
        ));
    }
}
