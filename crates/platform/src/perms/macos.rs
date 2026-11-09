//! macOS: ownership and mode through the C library, the ACL through the
//! `acl_*` functions of libSystem in their text form (`acl_to_text`,
//! `acl_from_text`), the one `chmod +a` and `ls -le` speak as well.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::io;
use std::path::Path;

use types::Error;

use super::unix;
use super::{
    Acl, DarwinAce, FilePermissions, PermissionChange, Principal, PrincipalKind, Principals,
};

type AclT = *mut c_void;

/// `ACL_TYPE_EXTENDED`, the one ACL type macOS files have.
const ACL_TYPE_EXTENDED: u32 = 0x0000_0100;

unsafe extern "C" {
    fn acl_get_link_np(path: *const c_char, type_: u32) -> AclT;
    fn acl_set_link_np(path: *const c_char, type_: u32, acl: AclT) -> c_int;
    fn acl_to_text(acl: AclT, len_p: *mut isize) -> *mut c_char;
    fn acl_from_text(buf: *const c_char) -> AclT;
    fn acl_free(obj: *mut c_void) -> c_int;
    fn mbr_uid_to_uuid(id: u32, uu: *mut u8) -> c_int;
    fn mbr_gid_to_uuid(id: u32, uu: *mut u8) -> c_int;
}

pub(super) fn permissions(path: &Path) -> Result<FilePermissions, Error> {
    let stat = unix::stat(path)?;
    let acl = if stat.is_symlink {
        None
    } else {
        read_acl(path)?
    };
    Ok(FilePermissions {
        owner: unix::principal_of(stat.uid, PrincipalKind::User),
        group: unix::principal_of(stat.gid, PrincipalKind::Group),
        mode: (!stat.is_symlink).then_some(stat.mode),
        acl,
        is_dir: stat.is_dir,
    })
}

pub(super) fn set_permissions(path: &Path, change: &PermissionChange) -> Result<(), Error> {
    let stat = unix::stat(path)?;
    let uid = change
        .owner
        .as_ref()
        .map(|p| parse_id(p, PrincipalKind::User))
        .transpose()?;
    let gid = change
        .group
        .as_ref()
        .map(|p| parse_id(p, PrincipalKind::Group))
        .transpose()?;
    if let Some(mode) = change.mode
        && !stat.is_symlink
    {
        unix::set_mode(path, mode)?;
    }
    if let Some(acl) = &change.acl
        && !stat.is_symlink
    {
        write_acl(path, acl)?;
    }
    unix::set_owner(path, uid, gid)?;
    if let Some(mode) = change.mode
        && (uid.is_some() || gid.is_some())
        && !stat.is_symlink
        && mode & 0o6000 != 0
    {
        unix::set_mode(path, mode)?;
    }
    Ok(())
}

// The Windows side can fail, so the trait says `Result`.
#[allow(clippy::unnecessary_wraps)]
pub(super) fn principals() -> Result<Principals, Error> {
    Ok(unix::principals())
}

pub(super) fn principal(name: &str, kind: PrincipalKind) -> Result<Principal, Error> {
    unix::principal(name, kind)
}

fn parse_id(principal: &Principal, kind: PrincipalKind) -> Result<u32, Error> {
    if let Ok(id) = principal.id.trim().parse::<u32>() {
        return Ok(id);
    }
    let name = principal.name.as_deref().unwrap_or(&principal.id);
    unix::principal(name, kind)?
        .id
        .parse()
        .map_err(|_| Error::NotFound)
}

/// The entry's ACL as text, `None` when it has none, or when the volume
/// has no ACLs.
fn read_acl(path: &Path) -> Result<Option<Acl>, Error> {
    let c_path = unix::c_path(path).map_err(Error::from_io)?;
    // SAFETY: `c_path` is NUL-terminated; the handle is freed below.
    let acl = unsafe { acl_get_link_np(c_path.as_ptr(), ACL_TYPE_EXTENDED) };
    if acl.is_null() {
        let error = io::Error::last_os_error();
        return match error.raw_os_error() {
            // No ACL on the entry: an empty list the user can add to.
            Some(libc::ENOENT) => Ok(Some(Acl::Darwin(Vec::new()))),
            Some(libc::ENOTSUP | libc::EOPNOTSUPP | libc::EINVAL) => Ok(None),
            _ => Err(Error::from_io(error)),
        };
    }
    let mut len: isize = 0;
    // SAFETY: `acl` is a live handle; the text is freed below.
    let text = unsafe { acl_to_text(acl, &raw mut len) };
    let parsed = if text.is_null() {
        Err(Error::from_io(io::Error::last_os_error()))
    } else {
        // SAFETY: `acl_to_text` returns a NUL-terminated string.
        let owned = unsafe { CStr::from_ptr(text) }
            .to_string_lossy()
            .into_owned();
        // SAFETY: the text came from `acl_to_text` and is freed once.
        unsafe { acl_free(text.cast()) };
        text::parse(&owned)
    };
    // SAFETY: the handle came from `acl_get_link_np` and is freed once.
    unsafe { acl_free(acl) };
    parsed.map(|entries| Some(Acl::Darwin(entries)))
}

/// Replaces the entry's ACL.
fn write_acl(path: &Path, acl: &Acl) -> Result<(), Error> {
    let Acl::Darwin(entries) = acl else {
        return Err(Error::Unsupported(
            "this file system has macOS ACLs, not this kind",
        ));
    };
    let c_path = unix::c_path(path).map_err(Error::from_io)?;
    // A name typed in the dialogue is looked up here.
    let entries: Vec<DarwinAce> = entries
        .iter()
        .map(|entry| {
            let kind = if entry.group {
                PrincipalKind::Group
            } else {
                PrincipalKind::User
            };
            let id = parse_id(&entry.principal, kind)?;
            Ok(DarwinAce {
                principal: unix::principal_of(id, kind),
                ..entry.clone()
            })
        })
        .collect::<Result<_, Error>>()?;
    let text = text::format(&entries)?;
    let c_text = CString::new(text).map_err(|_| {
        Error::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "an ACL entry with a NUL byte",
        ))
    })?;
    // SAFETY: `c_text` is NUL-terminated; the handle is freed below.
    let acl = unsafe { acl_from_text(c_text.as_ptr()) };
    if acl.is_null() {
        return Err(Error::from_io(io::Error::last_os_error()));
    }
    // SAFETY: `c_path` is NUL-terminated and `acl` a live handle.
    let status = unsafe { acl_set_link_np(c_path.as_ptr(), ACL_TYPE_EXTENDED, acl) };
    let result = if status == 0 {
        Ok(())
    } else {
        Err(Error::from_io(io::Error::last_os_error()))
    };
    // SAFETY: the handle came from `acl_from_text` and is freed once.
    unsafe { acl_free(acl) };
    result
}

/// The UUID of a user or group as `acl_from_text` wants it.
fn uuid_of(id: u32, group: bool) -> Option<String> {
    let mut uuid = [0u8; 16];
    // SAFETY: `uuid` is 16 bytes, what the membership API writes.
    let status = unsafe {
        if group {
            mbr_gid_to_uuid(id, uuid.as_mut_ptr())
        } else {
            mbr_uid_to_uuid(id, uuid.as_mut_ptr())
        }
    };
    (status == 0).then(|| {
        format!(
            "{:02X}{:02X}{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
            uuid[0], uuid[1], uuid[2], uuid[3], uuid[4], uuid[5], uuid[6], uuid[7], uuid[8],
            uuid[9], uuid[10], uuid[11], uuid[12], uuid[13], uuid[14], uuid[15]
        )
    })
}

/// The text form: a `!#acl 1` header, then one entry per line as
/// `user|group:UUID:name:id:allow|deny:perm,perm,…`.
pub(super) mod text {
    use super::{DarwinAce, Error, Principal, io, uuid_of};

    const HEADER: &str = "!#acl 1";

    fn bad(what: &str) -> Error {
        Error::Io(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("an ACL whose text does not parse: {what}"),
        ))
    }

    pub(crate) fn parse(text: &str) -> Result<Vec<DarwinAce>, Error> {
        let mut entries = Vec::new();
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
            if line.starts_with("!#") {
                continue;
            }
            let fields: Vec<&str> = line.splitn(6, ':').collect();
            let [kind, _uuid, name, id, verdict, perms] = fields[..] else {
                return Err(bad("an entry with fewer than six fields"));
            };
            let group = match kind {
                "user" => false,
                "group" => true,
                _ => return Err(bad("an entry that is neither user nor group")),
            };
            let allow = match verdict {
                "allow" => true,
                "deny" => false,
                _ => return Err(bad("an entry that neither allows nor denies")),
            };
            let principal = Principal {
                id: id.to_owned(),
                name: (!name.is_empty()).then(|| name.to_owned()),
            };
            entries.push(DarwinAce {
                allow,
                group,
                principal,
                perms: perms
                    .split(',')
                    .filter(|p| !p.is_empty())
                    .map(str::to_owned)
                    .collect(),
            });
        }
        Ok(entries)
    }

    pub(crate) fn format(entries: &[DarwinAce]) -> Result<String, Error> {
        let mut out = String::from(HEADER);
        out.push('\n');
        for entry in entries {
            let id: u32 = entry
                .principal
                .id
                .trim()
                .parse()
                .map_err(|_| bad("a principal without a numeric id"))?;
            let uuid = uuid_of(id, entry.group).ok_or(Error::NotFound)?;
            let name = entry.principal.name.as_deref().unwrap_or("");
            if name.contains(':') || entry.perms.iter().any(|p| p.contains(':')) {
                return Err(bad("a colon in a name or permission"));
            }
            out.push_str(if entry.group { "group" } else { "user" });
            out.push(':');
            out.push_str(&uuid);
            out.push(':');
            out.push_str(name);
            out.push(':');
            out.push_str(&id.to_string());
            out.push(':');
            out.push_str(if entry.allow { "allow" } else { "deny" });
            out.push(':');
            out.push_str(&entry.perms.join(","));
            out.push('\n');
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_text_form_parses_entries_and_skips_the_header() {
        let entries = text::parse(
            "!#acl 1\nuser:FFFFEEEE-DDDD-CCCC-BBBB-AAAA000001F5:serge:501:allow:read,write,file_inherit\ngroup:ABCDEFAB-CDEF-ABCD-EFAB-CDEF00000014:staff:20:deny:write\n",
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(entries.len(), 2);
        assert!(entries[0].allow && !entries[0].group);
        assert_eq!(entries[0].principal, Principal::named("501", "serge"));
        assert_eq!(entries[0].perms, ["read", "write", "file_inherit"]);
        assert!(!entries[1].allow && entries[1].group);
        assert!(text::parse("user:x:y").is_err());
        assert!(text::parse("other:a:b:1:allow:read").is_err());
    }

    #[test]
    fn root_formats_with_its_uuid() {
        let entries = vec![DarwinAce {
            allow: true,
            group: false,
            principal: Principal::named("0", "root"),
            perms: vec!["read".to_owned(), "write".to_owned()],
        }];
        let text = text::format(&entries).unwrap_or_else(|e| panic!("{e}"));
        assert!(
            text.starts_with(
                "!#acl 1\nuser:FFFFEEEE-DDDD-CCCC-BBBB-AAAA00000000:root:0:allow:read,write\n"
            ),
            "{text}"
        );
    }
}
