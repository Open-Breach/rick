//! Linux: ownership and mode through the C library, the ACLs through the
//! extended attributes the kernel exposes them as, `system.posix_acl_access`
//! and `system.posix_acl_default` for POSIX ACLs and `system.nfs4_acl` on
//! an `NFSv4` mount, so no ACL library needs linking.

use std::io;
use std::path::Path;

use types::Error;

use super::unix::{self, Stat};
use super::{
    Acl, FilePermissions, Nfs4Ace, Nfs4Kind, PermissionChange, PosixAce, PosixTag, Principal,
    PrincipalKind, Principals,
};

const POSIX_ACCESS: &str = "system.posix_acl_access";
const POSIX_DEFAULT: &str = "system.posix_acl_default";
const NFS4_ACL: &str = "system.nfs4_acl";

pub(super) fn permissions(path: &Path) -> Result<FilePermissions, Error> {
    let stat = unix::stat(path)?;
    let acl = if stat.is_symlink {
        None
    } else {
        read_acl(path, &stat)?
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
    // The mode first: a POSIX ACL set afterwards decides the group bits.
    if let Some(mode) = change.mode
        && !stat.is_symlink
    {
        unix::set_mode(path, mode)?;
    }
    if let Some(acl) = &change.acl
        && !stat.is_symlink
    {
        write_acl(path, &stat, acl)?;
    }
    // The owner last: a chown clears setuid and setgid, so the mode above
    // would not survive it the other way round for a file of another user.
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

/// The numeric id of a principal: its id when numeric, else its name
/// looked up.
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

/// The entry's ACL: `NFSv4` where the mount has one, else POSIX where the
/// file system supports them (the minimal list derived from the mode when
/// none is stored), else `None`.
fn read_acl(path: &Path, stat: &Stat) -> Result<Option<Acl>, Error> {
    if let Ok(Some(data)) = xattr_get(path, NFS4_ACL) {
        return Ok(Some(Acl::Nfs4(nfs4::parse(&data)?)));
    }
    let access = match xattr_get(path, POSIX_ACCESS) {
        Ok(Some(data)) => posix::parse(&data)?,
        Ok(None) => posix::minimal(stat.mode),
        Err(error) if unsupported(&error) => return Ok(None),
        Err(error) => return Err(Error::from_io(error)),
    };
    let default = if stat.is_dir {
        match xattr_get(path, POSIX_DEFAULT) {
            Ok(Some(data)) => posix::parse(&data)?,
            Ok(None) => Vec::new(),
            Err(error) if unsupported(&error) => Vec::new(),
            Err(error) => return Err(Error::from_io(error)),
        }
    } else {
        Vec::new()
    };
    Ok(Some(Acl::Posix { access, default }))
}

/// Replaces the entry's ACL with `acl`.
fn write_acl(path: &Path, stat: &Stat, acl: &Acl) -> Result<(), Error> {
    match acl {
        Acl::Posix { access, default } => {
            let access = &resolve(access)?;
            let default = &resolve(default)?;
            if posix::is_minimal(access) {
                // The three base entries live in the mode; no attribute.
                if let Some(mode) = posix::mode_of(access) {
                    unix::set_mode(path, (stat.mode & 0o7000) | mode)?;
                }
                xattr_remove(path, POSIX_ACCESS).map_err(Error::from_io)?;
            } else {
                xattr_set(path, POSIX_ACCESS, &posix::encode(access)).map_err(Error::from_io)?;
            }
            if stat.is_dir {
                if default.is_empty() {
                    xattr_remove(path, POSIX_DEFAULT).map_err(Error::from_io)?;
                } else {
                    xattr_set(path, POSIX_DEFAULT, &posix::encode(default))
                        .map_err(Error::from_io)?;
                }
            }
            Ok(())
        }
        Acl::Nfs4(entries) => {
            xattr_set(path, NFS4_ACL, &nfs4::encode(entries)).map_err(Error::from_io)
        }
        Acl::Darwin(_) | Acl::Nt { .. } => Err(Error::Unsupported(
            "this file system has POSIX or `NFSv4` ACLs, not this kind",
        )),
    }
}

/// The entries with every named qualifier's id numeric: a name typed in
/// the dialogue is looked up here.
fn resolve(entries: &[PosixAce]) -> Result<Vec<PosixAce>, Error> {
    entries
        .iter()
        .map(|entry| {
            let kind = match entry.tag {
                PosixTag::User => PrincipalKind::User,
                PosixTag::Group => PrincipalKind::Group,
                _ => return Ok(entry.clone()),
            };
            let Some(qualifier) = &entry.qualifier else {
                return Err(Error::Io(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "a named ACL entry without a user or group",
                )));
            };
            let id = parse_id(qualifier, kind)?;
            Ok(PosixAce {
                tag: entry.tag,
                qualifier: Some(unix::principal_of(id, kind)),
                perms: entry.perms,
            })
        })
        .collect()
}

/// Whether an extended attribute error says the file system has no such
/// attributes at all.
fn unsupported(error: &io::Error) -> bool {
    matches!(error.raw_os_error(), Some(libc::ENOTSUP | libc::ENOSYS))
}

/// The value of the attribute `name` on the entry itself; `None` when the
/// entry has no such attribute.
fn xattr_get(path: &Path, name: &str) -> io::Result<Option<Vec<u8>>> {
    let c_path = unix::c_path(path)?;
    let c_name = std::ffi::CString::new(name)?;
    let mut buffer = vec![0u8; 1024];
    loop {
        // SAFETY: both strings are NUL-terminated and `buffer` is as long
        // as the length passed.
        let got = unsafe {
            libc::lgetxattr(
                c_path.as_ptr(),
                c_name.as_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
            )
        };
        if got >= 0 {
            buffer.truncate(usize::try_from(got).unwrap_or(0));
            return Ok(Some(buffer));
        }
        let error = io::Error::last_os_error();
        match error.raw_os_error() {
            Some(libc::ERANGE) => buffer.resize(buffer.len() * 4, 0),
            Some(libc::ENODATA) => return Ok(None),
            _ => return Err(error),
        }
    }
}

fn xattr_set(path: &Path, name: &str, value: &[u8]) -> io::Result<()> {
    let c_path = unix::c_path(path)?;
    let c_name = std::ffi::CString::new(name)?;
    // SAFETY: the strings are NUL-terminated and `value` is as long as the
    // length passed.
    let status = unsafe {
        libc::lsetxattr(
            c_path.as_ptr(),
            c_name.as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
        )
    };
    if status == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Removes the attribute; an attribute that is not there is fine.
fn xattr_remove(path: &Path, name: &str) -> io::Result<()> {
    let c_path = unix::c_path(path)?;
    let c_name = std::ffi::CString::new(name)?;
    // SAFETY: both strings are NUL-terminated.
    let status = unsafe { libc::lremovexattr(c_path.as_ptr(), c_name.as_ptr()) };
    if status == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::ENODATA | libc::ENOTSUP) => Ok(()),
        _ => Err(error),
    }
}

/// POSIX ACLs as the kernel stores them: a little-endian version word
/// (2), then entries of a 16-bit tag, 16-bit permissions and a 32-bit id,
/// `0xFFFF_FFFF` for the entries without one.
pub(super) mod posix {
    use std::io;

    use types::Error;

    use super::super::unix;
    use super::{PosixAce, PosixTag, PrincipalKind};

    const VERSION: u32 = 2;
    const UNDEFINED_ID: u32 = u32::MAX;
    const TAG_USER_OBJ: u16 = 0x01;
    const TAG_USER: u16 = 0x02;
    const TAG_GROUP_OBJ: u16 = 0x04;
    const TAG_GROUP: u16 = 0x08;
    const TAG_MASK: u16 = 0x10;
    const TAG_OTHER: u16 = 0x20;

    fn bad(what: &str) -> Error {
        Error::Io(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("a POSIX ACL the kernel should not have stored: {what}"),
        ))
    }

    pub(crate) fn parse(data: &[u8]) -> Result<Vec<PosixAce>, Error> {
        let Some((header, rest)) = data.split_first_chunk::<4>() else {
            return Err(bad("shorter than its header"));
        };
        if u32::from_le_bytes(*header) != VERSION {
            return Err(bad("another version"));
        }
        if rest.len() % 8 != 0 {
            return Err(bad("a partial entry"));
        }
        rest.chunks_exact(8)
            .map(|entry| {
                let tag = u16::from_le_bytes([entry[0], entry[1]]);
                let perms = u16::from_le_bytes([entry[2], entry[3]]);
                let id = u32::from_le_bytes([entry[4], entry[5], entry[6], entry[7]]);
                let (tag, qualifier) = match tag {
                    TAG_USER_OBJ => (PosixTag::UserObj, None),
                    TAG_USER => (
                        PosixTag::User,
                        Some(unix::principal_of(id, PrincipalKind::User)),
                    ),
                    TAG_GROUP_OBJ => (PosixTag::GroupObj, None),
                    TAG_GROUP => (
                        PosixTag::Group,
                        Some(unix::principal_of(id, PrincipalKind::Group)),
                    ),
                    TAG_MASK => (PosixTag::Mask, None),
                    TAG_OTHER => (PosixTag::Other, None),
                    _ => return Err(bad("an unknown tag")),
                };
                Ok(PosixAce {
                    tag,
                    qualifier,
                    perms: u8::try_from(perms & 0o7).unwrap_or(0),
                })
            })
            .collect()
    }

    pub(crate) fn encode(entries: &[PosixAce]) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 + entries.len() * 8);
        out.extend_from_slice(&VERSION.to_le_bytes());
        // The kernel wants the entries sorted by tag, named ones by id.
        let mut sorted: Vec<&PosixAce> = entries.iter().collect();
        sorted.sort_by_key(|e| (tag_code(e.tag), qualifier_id(e)));
        for entry in sorted {
            out.extend_from_slice(&tag_code(entry.tag).to_le_bytes());
            out.extend_from_slice(&u16::from(entry.perms & 0o7).to_le_bytes());
            out.extend_from_slice(&qualifier_id(entry).to_le_bytes());
        }
        out
    }

    fn tag_code(tag: PosixTag) -> u16 {
        match tag {
            PosixTag::UserObj => TAG_USER_OBJ,
            PosixTag::User => TAG_USER,
            PosixTag::GroupObj => TAG_GROUP_OBJ,
            PosixTag::Group => TAG_GROUP,
            PosixTag::Mask => TAG_MASK,
            PosixTag::Other => TAG_OTHER,
        }
    }

    fn qualifier_id(entry: &PosixAce) -> u32 {
        match (entry.tag, &entry.qualifier) {
            (PosixTag::User | PosixTag::Group, Some(principal)) => {
                principal.id.trim().parse().unwrap_or(UNDEFINED_ID)
            }
            _ => UNDEFINED_ID,
        }
    }

    /// The list a bare mode stands for.
    pub(crate) fn minimal(mode: u32) -> Vec<PosixAce> {
        let bits = |shift: u32| u8::try_from((mode >> shift) & 0o7).unwrap_or(0);
        vec![
            PosixAce {
                tag: PosixTag::UserObj,
                qualifier: None,
                perms: bits(6),
            },
            PosixAce {
                tag: PosixTag::GroupObj,
                qualifier: None,
                perms: bits(3),
            },
            PosixAce {
                tag: PosixTag::Other,
                qualifier: None,
                perms: bits(0),
            },
        ]
    }

    /// Whether the list is nothing but the three base entries, which the
    /// mode carries by itself.
    pub(crate) fn is_minimal(entries: &[PosixAce]) -> bool {
        entries.iter().all(|e| {
            matches!(
                e.tag,
                PosixTag::UserObj | PosixTag::GroupObj | PosixTag::Other
            )
        })
    }

    /// The `rwx` bits of the three base entries as a mode, when all three
    /// are there.
    pub(crate) fn mode_of(entries: &[PosixAce]) -> Option<u32> {
        let bits = |tag: PosixTag| {
            entries
                .iter()
                .find(|e| e.tag == tag)
                .map(|e| u32::from(e.perms & 0o7))
        };
        Some(
            (bits(PosixTag::UserObj)? << 6)
                | (bits(PosixTag::GroupObj)? << 3)
                | bits(PosixTag::Other)?,
        )
    }
}

/// `NFSv4` ACLs as the kernel's NFS client exposes them: XDR, big-endian, a
/// count then entries of type, flag, access mask and a length-prefixed
/// `who` string padded to four bytes.
pub(super) mod nfs4 {
    use std::io;

    use types::Error;

    use super::{Nfs4Ace, Nfs4Kind};

    fn bad(what: &str) -> Error {
        Error::Io(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("an `NFSv4` ACL that does not decode: {what}"),
        ))
    }

    fn word(data: &[u8], at: &mut usize) -> Result<u32, Error> {
        let bytes = data.get(*at..*at + 4).ok_or_else(|| bad("truncated"))?;
        *at += 4;
        Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    pub(crate) fn parse(data: &[u8]) -> Result<Vec<Nfs4Ace>, Error> {
        let mut at = 0;
        let count = word(data, &mut at)?;
        let mut entries = Vec::with_capacity(count.min(1024) as usize);
        for _ in 0..count {
            let kind = match word(data, &mut at)? {
                0 => Nfs4Kind::Allow,
                1 => Nfs4Kind::Deny,
                2 => Nfs4Kind::Audit,
                3 => Nfs4Kind::Alarm,
                _ => return Err(bad("an unknown type")),
            };
            let flags = word(data, &mut at)?;
            let mask = word(data, &mut at)?;
            let len = usize::try_from(word(data, &mut at)?).map_err(|_| bad("a huge who"))?;
            let who = data
                .get(at..at + len)
                .ok_or_else(|| bad("a truncated who"))?;
            at += len.div_ceil(4) * 4;
            entries.push(Nfs4Ace {
                kind,
                flags,
                mask,
                who: String::from_utf8_lossy(who).into_owned(),
            });
        }
        Ok(entries)
    }

    pub(crate) fn encode(entries: &[Nfs4Ace]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&u32::try_from(entries.len()).unwrap_or(0).to_be_bytes());
        for entry in entries {
            let kind: u32 = match entry.kind {
                Nfs4Kind::Allow => 0,
                Nfs4Kind::Deny => 1,
                Nfs4Kind::Audit => 2,
                Nfs4Kind::Alarm => 3,
            };
            out.extend_from_slice(&kind.to_be_bytes());
            out.extend_from_slice(&entry.flags.to_be_bytes());
            out.extend_from_slice(&entry.mask.to_be_bytes());
            let who = entry.who.as_bytes();
            out.extend_from_slice(&u32::try_from(who.len()).unwrap_or(0).to_be_bytes());
            out.extend_from_slice(who);
            out.resize(out.len().div_ceil(4) * 4, 0);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ace(tag: PosixTag, id: Option<u32>, perms: u8) -> PosixAce {
        PosixAce {
            tag,
            qualifier: id.map(|id| Principal::id(id.to_string())),
            perms,
        }
    }

    #[test]
    fn posix_acls_encode_sorted_and_parse_back() {
        let entries = vec![
            ace(PosixTag::Other, None, 0),
            ace(PosixTag::User, Some(1000), 6),
            ace(PosixTag::UserObj, None, 7),
            ace(PosixTag::Mask, None, 6),
            ace(PosixTag::GroupObj, None, 4),
            ace(PosixTag::Group, Some(50), 4),
        ];
        let bytes = posix::encode(&entries);
        assert_eq!(bytes.len(), 4 + 6 * 8);
        assert_eq!(&bytes[..4], &2u32.to_le_bytes());
        let parsed = posix::parse(&bytes).unwrap_or_else(|e| panic!("{e}"));
        let tags: Vec<PosixTag> = parsed.iter().map(|e| e.tag).collect();
        assert_eq!(
            tags,
            [
                PosixTag::UserObj,
                PosixTag::User,
                PosixTag::GroupObj,
                PosixTag::Group,
                PosixTag::Mask,
                PosixTag::Other
            ]
        );
        assert_eq!(
            parsed[1].qualifier.as_ref().map(|p| p.id.as_str()),
            Some("1000")
        );
        assert_eq!(parsed[1].perms, 6);
        assert!(!posix::is_minimal(&entries));
        assert!(posix::is_minimal(&posix::minimal(0o750)));
        assert_eq!(posix::mode_of(&posix::minimal(0o750)), Some(0o750));
        assert!(posix::parse(&[1, 0, 0, 0]).is_err());
        assert!(posix::parse(&[2, 0, 0, 0, 1]).is_err());
    }

    #[test]
    fn nfs4_acls_round_trip_with_padding() {
        let entries = vec![
            Nfs4Ace {
                kind: Nfs4Kind::Allow,
                flags: 0,
                mask: 0x0016_01bf,
                who: "OWNER@".to_owned(),
            },
            Nfs4Ace {
                kind: Nfs4Kind::Deny,
                flags: 0x40,
                mask: 0x2,
                who: "alice@example.org".to_owned(),
            },
        ];
        let bytes = nfs4::encode(&entries);
        assert_eq!(bytes.len() % 4, 0);
        assert_eq!(
            nfs4::parse(&bytes).unwrap_or_else(|e| panic!("{e}")),
            entries
        );
        // Cut inside the last who: the padding alone would still parse.
        assert!(nfs4::parse(&bytes[..bytes.len() - 6]).is_err());
    }

    #[test]
    fn a_file_reports_its_mode_and_owner_and_takes_a_new_mode() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let file = dir.path().join("f");
        std::fs::write(&file, b"x").unwrap_or_else(|e| panic!("{e}"));
        unix::set_mode(&file, 0o640).unwrap_or_else(|e| panic!("{e}"));
        let perms = permissions(&file).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(perms.mode, Some(0o640));
        assert!(!perms.is_dir);
        assert!(perms.owner.name.is_some());
        set_permissions(
            &file,
            &PermissionChange {
                mode: Some(0o4755),
                ..PermissionChange::default()
            },
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            permissions(&file).unwrap_or_else(|e| panic!("{e}")).mode,
            Some(0o4755)
        );
        // A link has no mode of its own and no ACL.
        let link = dir.path().join("l");
        std::os::unix::fs::symlink(&file, &link).unwrap_or_else(|e| panic!("{e}"));
        let perms = permissions(&link).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(perms.mode, None);
        assert!(perms.acl.is_none());
    }

    #[test]
    fn a_posix_acl_is_set_read_back_and_reduced_to_the_mode() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let file = dir.path().join("f");
        std::fs::write(&file, b"x").unwrap_or_else(|e| panic!("{e}"));
        unix::set_mode(&file, 0o640).unwrap_or_else(|e| panic!("{e}"));
        let perms = permissions(&file).unwrap_or_else(|e| panic!("{e}"));
        let Some(Acl::Posix { access, default }) = perms.acl else {
            eprintln!("no POSIX ACLs on this file system; the round trip is not checked here");
            return;
        };
        assert_eq!(access, posix::minimal(0o640));
        assert!(default.is_empty());
        let mut extended = access.clone();
        extended.push(ace(PosixTag::User, Some(0), 4));
        extended.push(ace(PosixTag::Mask, None, 4));
        let change = PermissionChange {
            acl: Some(Acl::Posix {
                access: extended.clone(),
                default: Vec::new(),
            }),
            ..PermissionChange::default()
        };
        match set_permissions(&file, &change) {
            Ok(()) => {}
            Err(Error::Unsupported(_) | Error::Io(_)) => {
                eprintln!("the file system refused the ACL; the round trip is not checked here");
                return;
            }
            Err(error) => panic!("{error}"),
        }
        let perms = permissions(&file).unwrap_or_else(|e| panic!("{e}"));
        let Some(Acl::Posix { access, .. }) = perms.acl else {
            panic!("the ACL is gone");
        };
        assert!(
            access
                .iter()
                .any(|e| e.tag == PosixTag::User && e.perms == 4)
        );
        assert!(access.iter().any(|e| e.tag == PosixTag::Mask));
        // Back to the base entries: the attribute goes, the mode follows.
        set_permissions(
            &file,
            &PermissionChange {
                acl: Some(Acl::Posix {
                    access: posix::minimal(0o600),
                    default: Vec::new(),
                }),
                ..PermissionChange::default()
            },
        )
        .unwrap_or_else(|e| panic!("{e}"));
        let perms = permissions(&file).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(perms.mode, Some(0o600));
        assert_eq!(
            xattr_get(&file, POSIX_ACCESS).unwrap_or_else(|e| panic!("{e}")),
            None
        );
    }
}
