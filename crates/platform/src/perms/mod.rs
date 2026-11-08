//! Ownership, permission bits and access control lists of a local entry:
//! what the Properties dialogue shows under *Permissions* and changes
//! through one [`Permissions`] trait, the same on every platform.
//!
//! A [`Principal`] is a user or group the way the platform names it (a
//! uid or gid on Unix, an SID on Windows), [`FilePermissions`] is what an
//! entry has now and [`PermissionChange`] what to set, every field optional
//! so one change can touch the owner alone or the mode alone. ACLs come in
//! the flavour of the file system ([`Acl`]): POSIX and `NFSv4` on Linux,
//! macOS's own on macOS, NTFS discretionary ACLs on Windows; where the
//! volume has none, `acl` is `None` and the dialogue hides the list.
//!
//! A change the current user may not make (another user's file) runs in
//! the privileged helper, `rick-perms`, through [`Elevate`]: [`helper`]
//! holds the protocol both sides speak, a JSON request file and a JSON
//! reply on standard output, so the helper reads a list of changes,
//! records what each entry had before (for undo) and applies them, and
//! Rick itself never runs elevated.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use types::Error;

use crate::Native;
use crate::traits::Elevate;

#[cfg(unix)]
mod unix;

#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod imp;
#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod imp;
#[cfg(windows)]
#[path = "windows.rs"]
mod imp;

pub mod helper;

/// A user or group as the platform identifies it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Principal {
    /// The platform's identifier: the uid or gid as decimal text on Unix,
    /// the SID (`S-1-5-…`) on Windows.
    pub id: String,
    /// The name the platform resolves the id to, when it knows one.
    pub name: Option<String>,
}

impl Principal {
    /// A principal by id, without a name.
    #[must_use]
    pub fn id(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: None,
        }
    }

    /// A principal with its name.
    #[must_use]
    pub fn named(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: Some(name.into()),
        }
    }

    /// The text for a label: the name, or the id when there is none.
    #[must_use]
    pub fn display(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.id)
    }
}

/// Which kind of principal a name is looked up as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrincipalKind {
    /// A user account.
    User,
    /// A group.
    Group,
}

/// The users and groups the platform knows, for the dialogue's lists.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Principals {
    /// The user accounts, sorted by name.
    pub users: Vec<Principal>,
    /// The groups, sorted by name.
    pub groups: Vec<Principal>,
}

/// The tag of a POSIX ACL entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PosixTag {
    /// The owner (`user::`).
    UserObj,
    /// A named user (`user:name:`).
    User,
    /// The owning group (`group::`).
    GroupObj,
    /// A named group (`group:name:`).
    Group,
    /// The mask that caps the named entries and the owning group.
    Mask,
    /// Everyone else (`other::`).
    Other,
}

/// One entry of a POSIX ACL: `rwx` bits for a tag, qualified by a user or
/// group for the named tags.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PosixAce {
    /// Whom the entry is for.
    pub tag: PosixTag,
    /// The user or group of a [`PosixTag::User`] or [`PosixTag::Group`]
    /// entry; `None` for the other tags.
    pub qualifier: Option<Principal>,
    /// The permission bits: read 4, write 2, execute 1.
    pub perms: u8,
}

/// What an `NFSv4` ACE does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Nfs4Kind {
    /// Grants the mask.
    Allow,
    /// Denies the mask.
    Deny,
    /// Logs access.
    Audit,
    /// Alarms on access.
    Alarm,
}

/// One access control entry of an `NFSv4` ACL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Nfs4Ace {
    /// Allow, deny, audit or alarm.
    pub kind: Nfs4Kind,
    /// The ACE flags (inheritance, group, identifier group).
    pub flags: u32,
    /// The access mask (`ACE4_READ_DATA` and the others).
    pub mask: u32,
    /// Who: `OWNER@`, `GROUP@`, `EVERYONE@` or a `user@domain` name.
    pub who: String,
}

/// One entry of a macOS ACL, as `chmod +a` writes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DarwinAce {
    /// Allow or deny.
    pub allow: bool,
    /// Whether the principal is a group.
    pub group: bool,
    /// The user or group: the uid or gid as the id, the name when known.
    pub principal: Principal,
    /// The permission names (`read`, `write`, `delete`, …) in the order
    /// `acl_to_text` lists them.
    pub perms: Vec<String>,
}

/// One entry of an NTFS discretionary ACL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NtAce {
    /// Access allowed, or denied.
    pub allow: bool,
    /// The trustee: the SID as the id, `DOMAIN\Name` as the name.
    pub trustee: Principal,
    /// The access mask (`FILE_GENERIC_READ` and the others).
    pub mask: u32,
    /// The ACE flags: object and container inheritance, inherit only, no
    /// propagate, and `INHERITED_ACE` for an entry that came from the
    /// parent, which a change leaves to the parent.
    pub flags: u8,
}

impl NtAce {
    /// `INHERITED_ACE`: the entry came from the parent folder.
    pub const INHERITED: u8 = 0x10;

    /// Whether the entry was inherited from the parent folder.
    #[must_use]
    pub fn inherited(&self) -> bool {
        self.flags & Self::INHERITED != 0
    }
}

/// An access control list in the flavour of the entry's file system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Acl {
    /// POSIX ACLs (ext4, XFS, Btrfs, tmpfs): the access list, and the
    /// default list a folder hands to new entries.
    Posix {
        /// The entries that apply to the entry itself.
        access: Vec<PosixAce>,
        /// A folder's default entries; empty for a file.
        default: Vec<PosixAce>,
    },
    /// An `NFSv4` ACL, as a mounted `NFSv4` export carries it.
    Nfs4(Vec<Nfs4Ace>),
    /// A macOS ACL.
    Darwin(Vec<DarwinAce>),
    /// An NTFS discretionary ACL.
    Nt {
        /// The entries, inherited ones included, in the list's order.
        entries: Vec<NtAce>,
        /// Whether inheritance from the parent is blocked.
        protected: bool,
    },
}

impl Acl {
    /// The flavour's name for a label.
    #[must_use]
    pub fn flavour(&self) -> &'static str {
        match self {
            Self::Posix { .. } => "POSIX ACL",
            Self::Nfs4(_) => "`NFSv4` ACL",
            Self::Darwin(_) => "ACL",
            Self::Nt { .. } => "NTFS permissions",
        }
    }
}

/// Everything an entry has: owner, group, mode bits and the ACL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilePermissions {
    /// The owner.
    pub owner: Principal,
    /// The owning group (the primary group on Windows).
    pub group: Principal,
    /// The mode bits, `0o7777` at most: `rwx` for user, group and other,
    /// setuid, setgid and sticky; `None` where the platform has none
    /// (Windows).
    pub mode: Option<u32>,
    /// The ACL, where the file system has one.
    pub acl: Option<Acl>,
    /// Whether the entry is a folder, which decides what a recursive change
    /// applies to it.
    pub is_dir: bool,
}

impl FilePermissions {
    /// A change that puts an entry back to exactly this state.
    #[must_use]
    pub fn restore(&self) -> PermissionChange {
        PermissionChange {
            owner: Some(self.owner.clone()),
            group: Some(self.group.clone()),
            mode: self.mode,
            acl: self.acl.clone(),
        }
    }
}

/// What to change on an entry; a `None` field is kept as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionChange {
    /// The new owner.
    pub owner: Option<Principal>,
    /// The new group.
    pub group: Option<Principal>,
    /// The new mode bits.
    pub mode: Option<u32>,
    /// The new ACL, replacing the whole list.
    pub acl: Option<Acl>,
}

impl PermissionChange {
    /// Whether the change changes nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.owner.is_none() && self.group.is_none() && self.mode.is_none() && self.acl.is_none()
    }
}

/// Reading and changing the ownership, mode bits and ACL of a local
/// entry.
pub trait Permissions: Send + Sync {
    /// What the entry at `path` has, without following a final link.
    ///
    /// # Errors
    ///
    /// `Error::NotFound` when there is no such entry; otherwise the I/O
    /// error from reading it.
    fn permissions(&self, path: &Path) -> Result<FilePermissions, Error>;

    /// Applies `change` to the entry at `path`, without following a final
    /// link.
    ///
    /// # Errors
    ///
    /// `Error::PermissionDenied` when the current user may not make the
    /// change (the caller then turns to the elevated helper);
    /// `Error::Unsupported` for an ACL flavour the file system does not
    /// have; otherwise the I/O error.
    fn set_permissions(&self, path: &Path, change: &PermissionChange) -> Result<(), Error>;

    /// The users and groups the platform knows, for the dialogue's lists.
    ///
    /// # Errors
    ///
    /// The I/O or platform error from enumerating them.
    fn principals(&self) -> Result<Principals, Error>;

    /// Resolves a typed name (or id) to a principal.
    ///
    /// # Errors
    ///
    /// `Error::NotFound` when no such user or group exists.
    fn principal(&self, name: &str, kind: PrincipalKind) -> Result<Principal, Error>;
}

impl Permissions for Native {
    fn permissions(&self, path: &Path) -> Result<FilePermissions, Error> {
        imp::permissions(path)
    }

    fn set_permissions(&self, path: &Path, change: &PermissionChange) -> Result<(), Error> {
        imp::set_permissions(path, change)
    }

    fn principals(&self) -> Result<Principals, Error> {
        imp::principals()
    }

    fn principal(&self, name: &str, kind: PrincipalKind) -> Result<Principal, Error> {
        imp::principal(name, kind)
    }
}

/// The environment variable that names the privileged helper, for tests
/// and packagers that keep it elsewhere than beside the binary.
pub const HELPER_ENV: &str = "RICK_PERMS_HELPER";

/// The privileged helper's file name.
pub const HELPER_NAME: &str = "rick-perms";

/// Where the privileged helper is: [`HELPER_ENV`] when set, else
/// `rick-perms` beside the running binary, or one directory up from it,
/// where a test binary in `target/<profile>/deps` finds the one `cargo`
/// built beside it.
#[must_use]
pub fn helper_path() -> PathBuf {
    if let Some(path) = std::env::var_os(HELPER_ENV).filter(|p| !p.is_empty()) {
        return PathBuf::from(path);
    }
    let name = if cfg!(windows) {
        format!("{HELPER_NAME}.exe")
    } else {
        HELPER_NAME.to_owned()
    };
    let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    else {
        return PathBuf::from(name);
    };
    let beside = dir.join(&name);
    if beside.is_file() {
        return beside;
    }
    match dir.parent().map(|up| up.join(&name)) {
        Some(up) if up.is_file() => up,
        _ => beside,
    }
}

/// Runs the privileged helper over `items` through `elevate`: the user sees
/// the platform's prompt once, and the reply says what each entry had
/// before and which ones failed.
///
/// # Errors
///
/// `Error::Unsupported` where the platform offers no prompt or the helper
/// is not installed; `Error::PermissionDenied` when the prompt was
/// dismissed; the helper's own failure as an I/O error.
pub fn apply_elevated(
    elevate: &dyn Elevate,
    items: &[helper::Item],
) -> Result<helper::Reply, Error> {
    apply_elevated_with(elevate, &helper_path(), items)
}

/// [`apply_elevated`] with the helper at `program`.
///
/// # Errors
///
/// As [`apply_elevated`].
pub fn apply_elevated_with(
    elevate: &dyn Elevate,
    program: &Path,
    items: &[helper::Item],
) -> Result<helper::Reply, Error> {
    if !program.is_file() {
        tracing::warn!(path = %program.display(), "the privileged helper is not installed");
        return Err(Error::Unsupported(
            "the privileged helper rick-perms is not installed beside the binary",
        ));
    }
    let request = helper::Request {
        items: items.to_vec(),
    };
    let file = helper::write_request(&request)?;
    let args: Vec<OsString> = vec!["apply".into(), file.path().as_os_str().to_owned()];
    let output = elevate.run_elevated(program, &args)?;
    if !output.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::Io(std::io::Error::other(format!(
            "the privileged helper failed ({}): {}",
            output
                .code
                .map_or_else(|| "killed".to_owned(), |c| c.to_string()),
            stderr.trim()
        ))));
    }
    helper::parse_reply(&output.stdout)
}
