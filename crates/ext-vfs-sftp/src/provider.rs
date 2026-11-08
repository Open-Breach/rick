//! The provider: connections at the root, the remote host's tree below
//! each.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use ext::{
    Capabilities, ChangeStream, CopyOptions, EntryStream, LinkKind, ListOptions, ProgressSink,
    Provider, ReadHandle, RemoveMode, TransferMode, WriteHandle, WriteOptions, async_trait,
};
use futures::{StreamExt, stream};
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::{FileAttributes, OpenFlags, StatusCode};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::debug;
use types::{Attrs, CancelToken, Entry, EntryId, EntryKind, Error, Location, Progress, VPath};

use crate::connection::{Live, SftpConnections, ext_error};
use crate::{PROVIDER_ID, location, split};

/// Bytes per read when copying within a host through the machine.
const BUFFER: usize = 1 << 20;

/// What the provider can do below a connection.
const BELOW: Capabilities = Capabilities::REMOTE
    .union(Capabilities::WRITE)
    .union(Capabilities::RENAME)
    .union(Capabilities::SYMLINK)
    .union(Capabilities::SEEK)
    .union(Capabilities::NATIVE_TRANSFER);

/// FNV-1a over `bytes`: the key of an entry, stable across listings and
/// processes.
fn key(bytes: &[u8]) -> u128 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    u128::from(hash)
}

/// The kernel error for an SFTP error: the two status codes with a
/// variant of their own, the rest as the plugin's.
pub(crate) fn map(error: SftpError) -> Error {
    match &error {
        SftpError::Status(status) if status.status_code == StatusCode::NoSuchFile => {
            Error::NotFound
        }
        SftpError::Status(status) if status.status_code == StatusCode::PermissionDenied => {
            Error::PermissionDenied
        }
        _ => ext_error(error),
    }
}

/// Whether an SFTP error is the generic failure a version-3 server
/// answers when a name is taken.
fn is_failure(error: &SftpError) -> bool {
    matches!(error, SftpError::Status(status) if status.status_code == StatusCode::Failure)
}

fn already_exists() -> Error {
    Error::Io(std::io::Error::from(std::io::ErrorKind::AlreadyExists))
}

/// A time from the seconds the protocol carries.
fn time(seconds: Option<u32>) -> Option<SystemTime> {
    seconds.map(|s| SystemTime::UNIX_EPOCH + Duration::from_secs(u64::from(s)))
}

/// The entry for the remote file `rest` of the connection `name`, from
/// its attributes; a link carries its target.
pub(crate) fn entry(
    name: &str,
    rest: &str,
    attrs: &FileAttributes,
    link: Option<(String, bool)>,
) -> Entry {
    let loc = location(name, rest);
    let file_name = rest.rsplit('/').next().unwrap_or(rest).to_owned();
    let kind = match link {
        Some((target, broken)) => EntryKind::Symlink {
            target: VPath::from(target),
            broken,
        },
        None if attrs.is_dir() => EntryKind::Dir,
        None if attrs.is_regular() => EntryKind::File,
        None => EntryKind::Other,
    };
    let mut flags = Attrs::empty();
    if file_name.starts_with('.') {
        flags |= Attrs::HIDDEN;
    }
    if attrs.permissions.is_some_and(|mode| mode & 0o111 != 0) && kind == EntryKind::File {
        flags |= Attrs::EXECUTABLE;
    }
    if attrs.permissions.is_some_and(|mode| mode & 0o222 == 0) {
        flags |= Attrs::READONLY;
    }
    let size = if kind == EntryKind::Dir {
        None
    } else {
        attrs.size
    };
    Entry {
        id: EntryId::new(PROVIDER_ID, key(loc.path.as_bytes())),
        name: file_name,
        path: loc.path,
        kind,
        size,
        created: None,
        modified: time(attrs.mtime),
        accessed: time(attrs.atime),
        attrs: flags,
        mime: None,
    }
}

/// The folder entry of a connection at the provider's root, or of the
/// root itself.
fn folder(name: &str, rest: &str) -> Entry {
    let loc = location(name, rest);
    Entry {
        id: EntryId::new(PROVIDER_ID, key(loc.path.as_bytes())),
        name: if rest.is_empty() {
            name.to_owned()
        } else {
            rest.rsplit('/').next().unwrap_or(rest).to_owned()
        },
        path: loc.path,
        kind: EntryKind::Dir,
        size: None,
        created: None,
        modified: None,
        accessed: None,
        attrs: Attrs::empty(),
        mime: None,
    }
}

fn root_entry() -> Entry {
    Entry {
        id: EntryId::new(PROVIDER_ID, 0),
        name: "SFTP".to_owned(),
        path: VPath::from("/"),
        kind: EntryKind::Dir,
        size: None,
        created: None,
        modified: None,
        accessed: None,
        attrs: Attrs::empty(),
        mime: None,
    }
}

/// `rest/name` below a connection.
fn join(rest: &str, name: &str) -> String {
    if rest.is_empty() {
        name.to_owned()
    } else {
        format!("{rest}/{name}")
    }
}

/// An open directory handle of the raw session, closed on the server
/// when dropped, so a listing abandoned half-way leaves nothing open.
struct DirHandle {
    live: Arc<Live>,
    handle: Option<String>,
}

impl DirHandle {
    /// Closes the handle now, waiting for the server.
    async fn close(mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = self.live.raw.close(handle).await;
        }
    }
}

impl Drop for DirHandle {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            let live = Arc::clone(&self.live);
            ext::runtime::handle().spawn(async move {
                let _ = live.raw.close(handle).await;
            });
        }
    }
}

/// The states of a streamed listing.
enum Listing {
    Open {
        name: String,
        rest: String,
    },
    Read {
        dir: DirHandle,
        name: String,
        rest: String,
    },
    Done,
}

/// The `sftp` provider.
pub struct SftpProvider {
    conns: Arc<SftpConnections>,
}

impl std::fmt::Debug for SftpProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SftpProvider")
            .field("connections", &self.conns.names())
            .finish()
    }
}

impl SftpProvider {
    /// A provider over `conns`.
    #[must_use]
    pub fn new(conns: Arc<SftpConnections>) -> Self {
        Self { conns }
    }

    /// The session and the absolute remote path of a location below a
    /// connection.
    async fn resolve(&self, loc: &Location) -> Result<(Arc<Live>, String, String, String), Error> {
        let (name, rest) =
            split(loc).ok_or(Error::Unsupported("the root lists the connections"))?;
        let live = self.conns.session(&name).await?;
        let remote = live.remote(&rest);
        Ok((live, remote, name, rest))
    }

    /// One packet of a directory listing: the next batch, or the end.
    async fn read_batch(
        dir: DirHandle,
        name: String,
        rest: String,
        hidden: bool,
        cancel: &CancelToken,
    ) -> Option<(Result<Vec<Entry>, Error>, Listing)> {
        if cancel.is_cancelled() {
            dir.close().await;
            return Some((Err(Error::Cancelled), Listing::Done));
        }
        let handle = dir.handle.clone()?;
        match dir.live.raw.readdir(handle).await {
            Ok(packet) => {
                let entries: Vec<Entry> = packet
                    .files
                    .into_iter()
                    .filter(|f| f.filename != "." && f.filename != "..")
                    .filter(|f| hidden || !f.filename.starts_with('.'))
                    .map(|f| {
                        let link = f.attrs.is_symlink().then(|| (String::new(), false));
                        entry(&name, &join(&rest, &f.filename), &f.attrs, link)
                    })
                    .collect();
                Some((Ok(entries), Listing::Read { dir, name, rest }))
            }
            Err(SftpError::Status(status)) if status.status_code == StatusCode::Eof => {
                dir.close().await;
                None
            }
            Err(error) => {
                dir.close().await;
                Some((Err(map(error)), Listing::Done))
            }
        }
    }

    /// Removes a remote file or a whole folder.
    async fn remove_tree(live: &Arc<Live>, remote: &str) -> Result<(), Error> {
        let attrs = live.sftp.symlink_metadata(remote).await.map_err(map)?;
        if attrs.is_dir() {
            let children = live.sftp.read_dir(remote).await.map_err(map)?;
            for child in children {
                let path = format!("{remote}/{}", child.file_name());
                Box::pin(Self::remove_tree(live, &path)).await?;
            }
            live.sftp.remove_dir(remote).await.map_err(map)
        } else {
            live.sftp.remove_file(remote).await.map_err(map)
        }
    }
}

#[async_trait]
impl Provider for SftpProvider {
    fn id(&self) -> types::ProviderId {
        PROVIDER_ID
    }

    fn capabilities(&self, loc: &Location) -> Capabilities {
        if split(loc).is_some() {
            BELOW
        } else {
            Capabilities::REMOTE
        }
    }

    fn list(&self, loc: &Location, opts: ListOptions, cancel: CancelToken) -> EntryStream {
        let Some((name, rest)) = split(loc) else {
            let entries: Vec<Entry> = self
                .conns
                .names()
                .iter()
                .map(|name| folder(name, ""))
                .collect();
            return stream::once(async move { Ok(entries) }).boxed();
        };
        let conns = Arc::clone(&self.conns);
        let hidden = opts.hidden;
        stream::unfold(Listing::Open { name, rest }, move |state| {
            let conns = Arc::clone(&conns);
            let cancel = cancel.clone();
            async move {
                match state {
                    Listing::Done => None,
                    Listing::Open { name, rest } => {
                        if cancel.is_cancelled() {
                            return Some((Err(Error::Cancelled), Listing::Done));
                        }
                        let live = match conns.session(&name).await {
                            Ok(live) => live,
                            Err(error) => return Some((Err(error), Listing::Done)),
                        };
                        let remote = live.remote(&rest);
                        let handle = match live.raw.opendir(remote).await {
                            Ok(handle) => handle.handle,
                            Err(error) => return Some((Err(map(error)), Listing::Done)),
                        };
                        let dir = DirHandle {
                            live,
                            handle: Some(handle),
                        };
                        Self::read_batch(dir, name, rest, hidden, &cancel).await
                    }
                    Listing::Read { dir, name, rest } => {
                        Self::read_batch(dir, name, rest, hidden, &cancel).await
                    }
                }
            }
        })
        .boxed()
    }

    async fn stat(&self, loc: &Location) -> Result<Entry, Error> {
        let Some((name, rest)) = split(loc) else {
            return Ok(root_entry());
        };
        if rest.is_empty() {
            // The connection's own row: known without connecting.
            return if self.conns.names().contains(&name) {
                Ok(folder(&name, ""))
            } else {
                Err(Error::NotFound)
            };
        }
        let (live, remote, name, rest) = self.resolve(loc).await?;
        let attrs = live.sftp.symlink_metadata(&remote).await.map_err(map)?;
        let link = if attrs.is_symlink() {
            let target = live.sftp.read_link(&remote).await.map_err(map)?;
            let broken = live.sftp.metadata(&remote).await.is_err();
            Some((target, broken))
        } else {
            None
        };
        Ok(entry(&name, &rest, &attrs, link))
    }

    async fn open_read(&self, loc: &Location) -> Result<ReadHandle, Error> {
        let (live, remote, ..) = self.resolve(loc).await?;
        let file = live
            .sftp
            .open_with_flags(remote, OpenFlags::READ)
            .await
            .map_err(map)?;
        Ok(Box::new(file))
    }

    async fn open_write(&self, loc: &Location, opts: WriteOptions) -> Result<WriteHandle, Error> {
        let (live, remote, ..) = self.resolve(loc).await?;
        let mut flags = OpenFlags::CREATE | OpenFlags::WRITE | OpenFlags::TRUNCATE;
        if !opts.overwrite {
            flags |= OpenFlags::EXCLUDE;
        }
        match live.sftp.open_with_flags(&remote, flags).await {
            Ok(file) => Ok(Box::new(file)),
            Err(error) if !opts.overwrite && is_failure(&error) => {
                // Version 3 says "failure" for a taken name; make sure.
                if live.sftp.try_exists(&remote).await.unwrap_or(false) {
                    Err(already_exists())
                } else {
                    Err(map(error))
                }
            }
            Err(error) => Err(map(error)),
        }
    }

    async fn create_dir(&self, loc: &Location) -> Result<Entry, Error> {
        let (live, remote, name, rest) = self.resolve(loc).await?;
        if live.sftp.try_exists(&remote).await.map_err(map)? {
            return Err(already_exists());
        }
        live.sftp.create_dir(&remote).await.map_err(map)?;
        let attrs = live.sftp.metadata(&remote).await.map_err(map)?;
        Ok(entry(&name, &rest, &attrs, None))
    }

    async fn rename(&self, from: &Location, to: &Location) -> Result<Entry, Error> {
        let (live, source, name, _) = self.resolve(from).await?;
        let (to_name, to_rest) =
            split(to).ok_or(Error::Unsupported("cannot rename to the root"))?;
        if to_name != name {
            return Err(Error::Unsupported("a rename stays within one connection"));
        }
        let target = live.remote(&to_rest);
        live.sftp.rename(&source, &target).await.map_err(map)?;
        let attrs = live.sftp.symlink_metadata(&target).await.map_err(map)?;
        Ok(entry(&name, &to_rest, &attrs, None))
    }

    async fn remove(&self, loc: &Location, mode: RemoveMode) -> Result<(), Error> {
        if mode == RemoveMode::Trash {
            return Err(Error::Unsupported("an SFTP server has no trash"));
        }
        let (live, remote, ..) = self.resolve(loc).await?;
        Self::remove_tree(&live, &remote).await
    }

    async fn link(&self, target: &Location, at: &Location, kind: LinkKind) -> Result<Entry, Error> {
        let (live, link_path, name, rest) = self.resolve(at).await?;
        // The target as the link's text: a location of the same
        // connection becomes its remote path, anything else is used as
        // typed.
        let target_text = match split(target) {
            Some((t_name, t_rest)) if t_name == name => live.remote(&t_rest),
            _ => target.path.to_string_lossy().into_owned(),
        };
        match kind {
            LinkKind::Symbolic => {
                // OpenSSH's server reads SSH_FXP_SYMLINK's two paths the
                // other way round than the draft wrote them; russh-sftp
                // sends them in the draft's order, so swap for OpenSSH,
                // the server everyone runs.
                live.sftp
                    .symlink(&target_text, &link_path)
                    .await
                    .map_err(map)?;
            }
            LinkKind::Hard => {
                let done = live
                    .sftp
                    .hardlink(&target_text, &link_path)
                    .await
                    .map_err(map)?;
                if !done {
                    return Err(Error::Unsupported(
                        "the server has no hardlink@openssh.com extension",
                    ));
                }
            }
        }
        let attrs = live.sftp.symlink_metadata(&link_path).await.map_err(map)?;
        let link = attrs.is_symlink().then(|| (target_text.clone(), false));
        Ok(entry(&name, &rest, &attrs, link))
    }

    /// A copy within one host streams through the machine: the protocol
    /// has no copy of its own.
    async fn copy(
        &self,
        from: &Location,
        to: &Location,
        opts: CopyOptions,
        progress: ProgressSink,
        cancel: CancelToken,
    ) -> Result<(), Error> {
        let mut reader = self.open_read(from).await?;
        let mut writer = self
            .open_write(
                to,
                WriteOptions {
                    overwrite: opts.overwrite,
                },
            )
            .await?;
        let mut buffer = vec![0u8; BUFFER];
        let mut done = 0u64;
        loop {
            if cancel.is_cancelled() {
                return Err(Error::Cancelled);
            }
            let read = reader.read(&mut buffer).await.map_err(Error::from_io)?;
            if read == 0 {
                break;
            }
            writer
                .write_all(&buffer[..read])
                .await
                .map_err(Error::from_io)?;
            done += read as u64;
            progress.report(Progress::Bytes { done, total: None });
        }
        writer.shutdown().await.map_err(Error::from_io)?;
        Ok(())
    }

    /// A move within one host is a remote rename per item; a copy has no
    /// native path and is the kernel's.
    async fn transfer(
        &self,
        items: &[Location],
        to: &Location,
        mode: TransferMode,
        progress: ProgressSink,
        cancel: CancelToken,
    ) -> Result<(), Error> {
        if mode == TransferMode::Copy {
            return Err(Error::Unsupported(
                "an SFTP server copies nothing by itself",
            ));
        }
        let (live, dir, name, _) = self.resolve(to).await?;
        let total = items.len() as u64;
        for (done, item) in items.iter().enumerate() {
            if cancel.is_cancelled() {
                return Err(Error::Cancelled);
            }
            progress.report(Progress::Items {
                done: done as u64,
                total: Some(total),
            });
            let (item_name, item_rest) =
                split(item).ok_or(Error::Unsupported("the root cannot be moved"))?;
            if item_name != name {
                return Err(Error::Unsupported(
                    "a native move stays within one connection",
                ));
            }
            let file_name = item_rest
                .rsplit('/')
                .next()
                .filter(|n| !n.is_empty())
                .ok_or(Error::Unsupported("a connection cannot be moved"))?;
            let source = live.remote(&item_rest);
            let target = if dir == "/" {
                format!("/{file_name}")
            } else {
                format!("{dir}/{file_name}")
            };
            debug!(%source, %target, "remote rename");
            live.sftp.rename(&source, &target).await.map_err(map)?;
        }
        progress.report(Progress::Items {
            done: total,
            total: Some(total),
        });
        Ok(())
    }

    fn watch(&self, _loc: &Location) -> Result<ChangeStream, Error> {
        Err(Error::Unsupported(
            "an SFTP server sends no change notifications; the kernel polls",
        ))
    }
}
