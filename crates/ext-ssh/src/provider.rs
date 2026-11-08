//! The `ssh` provider: the hosts at the root, the host's tree below each,
//! every operation a command on an `exec` channel of the pool.

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};
use std::time::{Duration, SystemTime};

use ext::{
    Capabilities, ChangeStream, CopyOptions, EntryStream, LinkKind, ListOptions, ProgressSink,
    Provider, ReadHandle, RemoveMode, TransferMode, WriteHandle, WriteOptions, async_trait,
};
use futures::future::BoxFuture;
use futures::{StreamExt, stream};
use russh::client::Msg;
use russh::{Channel, ChannelMsg};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tracing::{debug, warn};
use types::{Attrs, CancelToken, Entry, EntryId, EntryKind, Error, Location, Progress, VPath};

use crate::connection::{Lease, Output, Pool, Token};
use crate::profile::{self, Parser, RawEntry, RawKind};
use crate::shell::{self, Command};
use crate::{PROVIDER_ID, location, message, split};

/// What the provider can do below a host.
const BELOW: Capabilities = Capabilities::REMOTE
    .union(Capabilities::WRITE)
    .union(Capabilities::RENAME)
    .union(Capabilities::SYMLINK)
    .union(Capabilities::SEEK)
    .union(Capabilities::NATIVE_TRANSFER);

/// Bytes per buffer when a copy streams through the machine.
const BUFFER: usize = 1 << 20;
/// How often a native transfer's target is measured for progress.
const POLL_PROGRESS: Duration = Duration::from_millis(500);

/// Numbers the temporary names of uploads within the process.
static UPLOADS: AtomicU64 = AtomicU64::new(0);

/// FNV-1a over the alias and the path: the key of an entry, stable across
/// listings and processes.
fn key(alias: &str, path: &[u8]) -> u128 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in alias
        .bytes()
        .chain(std::iter::once(0))
        .chain(path.iter().copied())
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    u128::from(hash)
}

fn already_exists() -> Error {
    Error::Io(std::io::Error::from(std::io::ErrorKind::AlreadyExists))
}

/// A time from epoch seconds and nanoseconds.
fn time(mtime: Option<(i64, u32)>) -> Option<SystemTime> {
    let (secs, nanos) = mtime?;
    let duration = Duration::new(secs.unsigned_abs(), nanos);
    if secs >= 0 {
        SystemTime::UNIX_EPOCH.checked_add(duration)
    } else {
        SystemTime::UNIX_EPOCH.checked_sub(duration)
    }
}

/// The entry for the remote `path` on `alias`, from a record.
pub(crate) fn entry_at(alias: &str, path: &[u8], raw: &RawEntry) -> Entry {
    let loc = location(alias, path);
    let name = String::from_utf8_lossy(shell::file_name(path)).into_owned();
    let kind = match raw.kind {
        RawKind::File => EntryKind::File,
        RawKind::Dir => EntryKind::Dir,
        RawKind::Link => EntryKind::Symlink {
            target: VPath::from(raw.link.clone().unwrap_or_default()),
            // Whether the target exists is not in the listing; a stat
            // says so when asked.
            broken: false,
        },
        RawKind::Other => EntryKind::Other,
    };
    let is_dir = kind == EntryKind::Dir;
    let mut attrs = Attrs::empty();
    if name.starts_with('.') {
        attrs |= Attrs::HIDDEN;
    }
    if kind == EntryKind::File && raw.mode & 0o111 != 0 {
        attrs |= Attrs::EXECUTABLE;
    }
    if raw.mode & 0o222 == 0 {
        attrs |= Attrs::READONLY;
    }
    Entry {
        id: EntryId::new(PROVIDER_ID, key(alias, loc.path.as_bytes())),
        name,
        path: loc.path,
        kind,
        size: (!is_dir).then_some(raw.size),
        created: None,
        modified: time(raw.mtime),
        accessed: None,
        attrs,
        mime: None,
    }
}

/// The folder entry of a host at the root, known without connecting.
fn host_entry(alias: &str) -> Entry {
    let loc = location(alias, b"");
    Entry {
        id: EntryId::new(PROVIDER_ID, key(alias, loc.path.as_bytes())),
        name: alias.to_owned(),
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
        name: "SSH".to_owned(),
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

/// Reads a command's standard output as the file's bytes, reporting the
/// command's failure as the read error when it printed nothing or ended
/// badly: `cat` of a missing file is `NotFound`, not an empty file.
struct ChannelReader {
    channel: Option<Channel<Msg>>,
    pending: Option<BoxFuture<'static, (Option<ChannelMsg>, Channel<Msg>)>>,
    buffered: Option<(bytes::Bytes, usize)>,
    stderr: Vec<u8>,
    status: Option<u32>,
    done: bool,
    _token: Token,
}

impl ChannelReader {
    fn new(lease: Lease) -> Self {
        Self {
            channel: Some(lease.channel),
            pending: None,
            buffered: None,
            stderr: Vec::new(),
            status: None,
            done: false,
            _token: lease.token,
        }
    }

    /// The error the command's end stands for, if it failed.
    fn failure(&self) -> Option<std::io::Error> {
        let status = self.status?;
        if status == 0 {
            return None;
        }
        let output = Output {
            status,
            stdout: Vec::new(),
            stderr: self.stderr.clone(),
        };
        Some(match output.error() {
            Error::NotFound => std::io::Error::from(std::io::ErrorKind::NotFound),
            Error::PermissionDenied => std::io::Error::from(std::io::ErrorKind::PermissionDenied),
            other => std::io::Error::other(other.to_string()),
        })
    }
}

impl AsyncRead for ChannelReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        loop {
            if let Some((data, offset)) = self.buffered.take() {
                let take = buf.remaining().min(data.len() - offset);
                buf.put_slice(&data[offset..offset + take]);
                if offset + take < data.len() {
                    self.buffered = Some((data, offset + take));
                }
                return Poll::Ready(Ok(()));
            }
            if self.done {
                return Poll::Ready(self.failure().map_or(Ok(()), Err));
            }
            if self.pending.is_none() {
                let Some(mut channel) = self.channel.take() else {
                    self.done = true;
                    continue;
                };
                self.pending = Some(Box::pin(async move {
                    let msg = channel.wait().await;
                    (msg, channel)
                }));
            }
            let Some(future) = self.pending.as_mut() else {
                continue;
            };
            let (msg, channel) = futures::ready!(future.as_mut().poll(cx));
            self.pending = None;
            self.channel = Some(channel);
            match msg {
                Some(ChannelMsg::Data { data }) => {
                    if !data.is_empty() {
                        self.buffered = Some((data, 0));
                    }
                }
                Some(ChannelMsg::ExtendedData { data, .. }) => self.stderr.extend_from_slice(&data),
                Some(ChannelMsg::ExitStatus { exit_status }) => self.status = Some(exit_status),
                Some(ChannelMsg::ExitSignal { .. }) => self.status = Some(128),
                Some(ChannelMsg::Eof | ChannelMsg::Close) | None => self.done = true,
                Some(_) => {}
            }
        }
    }
}

/// Writes a file through `cat > <temporary name>`; `shutdown` ends the
/// command and moves the temporary name onto the target, so a reader
/// never sees a half-written file. Dropped before `shutdown`, it removes
/// the temporary file.
struct ChannelWriter {
    writer: Pin<Box<dyn AsyncWrite + Send>>,
    channel: Option<Channel<Msg>>,
    finishing: Option<BoxFuture<'static, std::io::Result<()>>>,
    finished: bool,
    pool: Arc<Pool>,
    alias: String,
    temporary: Vec<u8>,
    target: Vec<u8>,
    _token: Token,
}

impl AsyncWrite for ChannelWriter {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        self.writer.as_mut().poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        self.writer.as_mut().poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        if self.finished {
            return Poll::Ready(Ok(()));
        }
        if self.finishing.is_none() {
            let Some(mut channel) = self.channel.take() else {
                return Poll::Ready(Err(std::io::Error::other("the upload channel is gone")));
            };
            let pool = Arc::clone(&self.pool);
            let alias = self.alias.clone();
            let temporary = self.temporary.clone();
            let target = self.target.clone();
            self.finishing = Some(Box::pin(async move {
                channel
                    .eof()
                    .await
                    .map_err(|e| std::io::Error::other(e.to_string()))?;
                let mut stderr = Vec::new();
                let mut status = None;
                while let Some(msg) = channel.wait().await {
                    match msg {
                        ChannelMsg::ExtendedData { data, .. } => stderr.extend_from_slice(&data),
                        ChannelMsg::ExitStatus { exit_status } => status = Some(exit_status),
                        ChannelMsg::Close => break,
                        _ => {}
                    }
                }
                let output = Output {
                    status: status.unwrap_or(255),
                    stdout: Vec::new(),
                    stderr,
                };
                if output.status != 0 {
                    let _ = pool
                        .run(
                            &alias,
                            Command::new("rm").raw(" -f --").arg(&temporary).as_bytes(),
                        )
                        .await;
                    return Err(std::io::Error::other(output.error().to_string()));
                }
                pool.run_ok(
                    &alias,
                    Command::new("mv")
                        .raw(" -f --")
                        .arg(&temporary)
                        .arg(&target)
                        .as_bytes(),
                )
                .await
                .map(drop)
                .map_err(|e| std::io::Error::other(e.to_string()))
            }));
        }
        let Some(future) = self.finishing.as_mut() else {
            return Poll::Ready(Ok(()));
        };
        let result = futures::ready!(future.as_mut().poll(cx));
        self.finishing = None;
        self.finished = true;
        Poll::Ready(result)
    }
}

impl Drop for ChannelWriter {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        // Dropped before shutdown: the half-written temporary goes.
        let pool = Arc::clone(&self.pool);
        let alias = self.alias.clone();
        let temporary = self.temporary.clone();
        let channel = self.channel.take();
        ext::runtime::handle().spawn(async move {
            if let Some(channel) = channel {
                let _ = channel.eof().await;
                let _ = channel.close().await;
            }
            let _ = pool
                .run(
                    &alias,
                    Command::new("rm").raw(" -f --").arg(&temporary).as_bytes(),
                )
                .await;
        });
    }
}

/// The states of a streamed listing.
enum Listing {
    Open {
        alias: String,
        remote: Vec<u8>,
    },
    Read {
        alias: String,
        dir: Vec<u8>,
        channel: Channel<Msg>,
        token: Token,
        parser: Parser,
        stderr: Vec<u8>,
        status: Option<u32>,
        any: bool,
    },
    Done,
}

/// The `ssh` provider.
pub struct SshProvider {
    pool: Arc<Pool>,
}

impl std::fmt::Debug for SshProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SshProvider")
            .field("pool", &self.pool)
            .finish()
    }
}

impl SshProvider {
    /// A provider over `pool`.
    #[must_use]
    pub fn new(pool: Arc<Pool>) -> Self {
        Self { pool }
    }

    /// The pool, for the tests.
    #[must_use]
    pub fn pool(&self) -> &Arc<Pool> {
        &self.pool
    }

    /// The alias and the absolute remote path of a location below a host;
    /// the default directory for a bare alias.
    async fn resolve(&self, loc: &Location) -> Result<(String, Vec<u8>), Error> {
        let (alias, remote) = split(loc).ok_or(Error::Unsupported("the root lists the hosts"))?;
        if remote.is_empty() {
            let dir = self.pool.default_dir(&alias).await?;
            return Ok((alias, dir));
        }
        Ok((alias, remote))
    }

    /// The record of one remote path.
    async fn stat_remote(&self, alias: &str, path: &[u8]) -> Result<RawEntry, Error> {
        let profile = self.pool.profile(alias).await?;
        let output = self
            .pool
            .run(alias, &profile::stat_command(profile, path))
            .await?;
        if output.status == 2 {
            return Err(Error::NotFound);
        }
        if output.status != 0 {
            return Err(output.error());
        }
        let mut parser = Parser::for_stat();
        let mut records = parser.push(&output.stdout);
        if records.len() != 1 || !parser.remainder().is_empty() {
            warn!(alias, %profile, "stat printed no record; the host is probed again");
            self.pool.reprobe(alias);
            return Err(message(format!(
                "{alias}: stat of {} printed nothing the {profile} profile reads",
                String::from_utf8_lossy(path)
            )));
        }
        Ok(records.remove(0))
    }

    /// Whether `path` exists on the host (a link counts).
    async fn exists(&self, alias: &str, path: &[u8]) -> Result<bool, Error> {
        let mut command = b"[ -e ".to_vec();
        command.extend_from_slice(&shell::quote(path));
        command.extend_from_slice(b" ] || [ -L ");
        command.extend_from_slice(&shell::quote(path));
        command.extend_from_slice(b" ]");
        Ok(self.pool.run(alias, &command).await?.status == 0)
    }

    /// Runs the listing stream's next step.
    // One listing from the channel open to its close, batch by batch.
    #[allow(clippy::too_many_lines)]
    async fn list_step(
        pool: Arc<Pool>,
        state: Listing,
        hidden: bool,
        cancel: CancelToken,
    ) -> Option<(Result<Vec<Entry>, Error>, Listing)> {
        match state {
            Listing::Done => None,
            Listing::Open { alias, remote } => {
                if cancel.is_cancelled() {
                    return Some((Err(Error::Cancelled), Listing::Done));
                }
                let dir = if remote.is_empty() {
                    match pool.default_dir(&alias).await {
                        Ok(dir) => dir,
                        Err(error) => return Some((Err(error), Listing::Done)),
                    }
                } else {
                    remote
                };
                let profile = match pool.profile(&alias).await {
                    Ok(profile) => profile,
                    Err(error) => return Some((Err(error), Listing::Done)),
                };
                let Lease { channel, token } = match pool.lease(&alias).await {
                    Ok(lease) => lease,
                    Err(error) => return Some((Err(error), Listing::Done)),
                };
                if let Err(error) = channel
                    .exec(true, profile::list_command(profile, &dir))
                    .await
                {
                    return Some((Err(crate::ssh_error(error)), Listing::Done));
                }
                let state = Listing::Read {
                    alias,
                    dir,
                    channel,
                    token,
                    parser: Parser::new(profile),
                    stderr: Vec::new(),
                    status: None,
                    any: false,
                };
                Box::pin(Self::list_step(pool, state, hidden, cancel)).await
            }
            Listing::Read {
                alias,
                dir,
                mut channel,
                token,
                mut parser,
                mut stderr,
                mut status,
                mut any,
            } => {
                loop {
                    if cancel.is_cancelled() {
                        return Some((Err(Error::Cancelled), Listing::Done));
                    }
                    let Some(msg) = channel.wait().await else {
                        break;
                    };
                    match msg {
                        ChannelMsg::Data { data } => {
                            let batch: Vec<Entry> = parser
                                .push(&data)
                                .iter()
                                .filter(|raw| hidden || !raw.name.starts_with(b"."))
                                .map(|raw| entry_at(&alias, &shell::join(&dir, &raw.name), raw))
                                .collect();
                            if !batch.is_empty() {
                                any = true;
                                let state = Listing::Read {
                                    alias,
                                    dir,
                                    channel,
                                    token,
                                    parser,
                                    stderr,
                                    status,
                                    any,
                                };
                                return Some((Ok(batch), state));
                            }
                        }
                        ChannelMsg::ExtendedData { data, .. } => stderr.extend_from_slice(&data),
                        ChannelMsg::ExitStatus { exit_status } => status = Some(exit_status),
                        ChannelMsg::Close => break,
                        _ => {}
                    }
                }
                drop(token);
                if !parser.remainder().is_empty() {
                    warn!(
                        alias,
                        "listing left bytes the profile does not read; probing again"
                    );
                    pool.reprobe(&alias);
                }
                let output = Output {
                    status: status.unwrap_or(0),
                    stdout: Vec::new(),
                    stderr,
                };
                if output.status != 0 && !any {
                    return Some((Err(output.error()), Listing::Done));
                }
                None
            }
        }
    }

    /// The total and free bytes of the filesystem behind a location, from
    /// `df -P -k`: what a status bar would show (`specs/ext-ssh.md`).
    ///
    /// # Errors
    ///
    /// The connection error, or the command's.
    pub async fn disk_usage(&self, loc: &Location) -> Result<Option<(u64, u64)>, Error> {
        let (alias, path) = self.resolve(loc).await?;
        let output = self
            .pool
            .run_ok(&alias, &profile::df_command(&path))
            .await?;
        Ok(profile::parse_df(&output))
    }

    /// Streams one file from a host to a location, both on this provider,
    /// through the machine: a copy across two hosts.
    async fn stream_copy(
        &self,
        from: &Location,
        to: &Location,
        opts: CopyOptions,
        progress: &ProgressSink,
        cancel: &CancelToken,
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

    /// Moves or copies one item into `dir` on the host with `mv` or
    /// `cp -a`, reporting the target's size while the copy runs; a cancel
    /// kills the copy and removes what it left.
    #[allow(clippy::too_many_arguments)]
    async fn transfer_one(
        &self,
        alias: &str,
        item: &[u8],
        dir: &[u8],
        mode: TransferMode,
        total: Option<u64>,
        progress: &ProgressSink,
        cancel: &CancelToken,
    ) -> Result<(), Error> {
        let name = shell::file_name(item);
        if name.is_empty() {
            return Err(Error::Unsupported("a root cannot be moved or copied"));
        }
        let target = shell::join(dir, name);
        if self.exists(alias, &target).await? {
            return Err(already_exists());
        }
        let program = match mode {
            TransferMode::Move => "mv --",
            TransferMode::Copy => "cp -a --",
        };
        // The copy runs in the background of its shell, which prints the
        // pid first and waits for it, so a cancel can kill it by number.
        let mut command = Command::new(program)
            .arg(item)
            .arg(dir)
            .raw(" & pid=$!; printf '%s\\n' \"$pid\"; wait \"$pid\"")
            .into_bytes();
        command.insert(0, b' ');
        let Lease { mut channel, token } = self.pool.lease(alias).await?;
        channel
            .exec(true, command)
            .await
            .map_err(crate::ssh_error)?;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut status = None;
        let mut pid: Option<String> = None;
        let mut poll = tokio::time::interval(POLL_PROGRESS);
        poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                msg = channel.wait() => {
                    let Some(msg) = msg else { break };
                    match msg {
                        ChannelMsg::Data { data } => {
                            stdout.extend_from_slice(&data);
                            if pid.is_none() && let Some(end) = stdout.iter().position(|b| *b == b'\n') {
                                pid = Some(String::from_utf8_lossy(&stdout[..end]).trim().to_owned());
                            }
                        }
                        ChannelMsg::ExtendedData { data, .. } => stderr.extend_from_slice(&data),
                        ChannelMsg::ExitStatus { exit_status } => status = Some(exit_status),
                        ChannelMsg::Close => break,
                        _ => {}
                    }
                }
                _ = poll.tick() => {
                    if cancel.is_cancelled() {
                        if let Some(pid) = pid.as_deref().filter(|p| p.chars().all(|c| c.is_ascii_digit())) {
                            let _ = self.pool.run(alias, format!("kill {pid}").as_bytes()).await;
                        }
                        drop(token);
                        if mode == TransferMode::Copy {
                            let _ = self
                                .pool
                                .run(alias, Command::new("rm").raw(" -rf --").arg(&target).as_bytes())
                                .await;
                        }
                        return Err(Error::Cancelled);
                    }
                    if mode == TransferMode::Copy
                        && let Ok(out) = self
                            .pool
                            .run(alias, Command::new("du").raw(" -sk --").arg(&target).as_bytes())
                            .await
                        && out.status == 0
                        && let Some(kib) = String::from_utf8_lossy(&out.stdout)
                            .split_whitespace()
                            .next()
                            .and_then(|k| k.parse::<u64>().ok())
                    {
                        progress.report(Progress::Bytes { done: kib * 1024, total });
                    }
                }
            }
        }
        drop(token);
        let output = Output {
            status: status.unwrap_or(255),
            stdout: Vec::new(),
            stderr,
        };
        if output.status != 0 {
            return Err(output.error());
        }
        Ok(())
    }
}

#[async_trait]
impl Provider for SshProvider {
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
        let Some((alias, remote)) = split(loc) else {
            let entries: Vec<Entry> = self
                .pool
                .hosts()
                .all()
                .iter()
                .map(|host| host_entry(&host.alias))
                .collect();
            return stream::once(async move { Ok(entries) }).boxed();
        };
        let pool = Arc::clone(&self.pool);
        let hidden = opts.hidden;
        stream::unfold(Listing::Open { alias, remote }, move |state| {
            let pool = Arc::clone(&pool);
            let cancel = cancel.clone();
            Self::list_step(pool, state, hidden, cancel)
        })
        .boxed()
    }

    async fn stat(&self, loc: &Location) -> Result<Entry, Error> {
        let Some((alias, remote)) = split(loc) else {
            return Ok(root_entry());
        };
        if remote.is_empty() {
            // The host's own row: known without connecting.
            return if self.pool.hosts().get(&alias).is_some() {
                Ok(host_entry(&alias))
            } else {
                Err(Error::NotFound)
            };
        }
        let raw = self.stat_remote(&alias, &remote).await?;
        let mut entry = entry_at(&alias, &remote, &raw);
        if let EntryKind::Symlink { target, broken } = &mut entry.kind {
            let resolved = if target.as_bytes().starts_with(b"/") {
                target.as_bytes().to_vec()
            } else {
                shell::join(shell::parent(&remote), target.as_bytes())
            };
            *broken = !self.exists(&alias, &resolved).await.unwrap_or(false);
        }
        Ok(entry)
    }

    async fn open_read(&self, loc: &Location) -> Result<ReadHandle, Error> {
        self.open_read_at(loc, 0).await
    }

    async fn open_read_at(&self, loc: &Location, offset: u64) -> Result<ReadHandle, Error> {
        let (alias, path) = self.resolve(loc).await?;
        let lease = self.pool.lease(&alias).await?;
        lease
            .channel
            .exec(true, profile::read_from_command(&path, offset))
            .await
            .map_err(crate::ssh_error)?;
        Ok(Box::new(ChannelReader::new(lease)))
    }

    async fn open_write(&self, loc: &Location, opts: WriteOptions) -> Result<WriteHandle, Error> {
        let (alias, path) = self.resolve(loc).await?;
        if !opts.overwrite && self.exists(&alias, &path).await? {
            return Err(already_exists());
        }
        let dir = shell::parent(&path);
        let number = UPLOADS.fetch_add(1, Ordering::Relaxed);
        let temporary = shell::join(
            dir,
            format!(".rick-upload-{}-{number}.part", std::process::id()).as_bytes(),
        );
        let Lease { channel, token } = self.pool.lease(&alias).await?;
        channel
            .exec(
                true,
                Command::new("cat").raw(" >").arg(&temporary).into_bytes(),
            )
            .await
            .map_err(crate::ssh_error)?;
        let writer = Box::pin(channel.make_writer());
        Ok(Box::new(ChannelWriter {
            writer,
            channel: Some(channel),
            finishing: None,
            finished: false,
            pool: Arc::clone(&self.pool),
            alias,
            temporary,
            target: path,
            _token: token,
        }))
    }

    async fn create_dir(&self, loc: &Location) -> Result<Entry, Error> {
        let (alias, path) = self.resolve(loc).await?;
        let output = self
            .pool
            .run(
                &alias,
                Command::new("mkdir").raw(" --").arg(&path).as_bytes(),
            )
            .await?;
        if output.status != 0 {
            if output.stderr_text().to_ascii_lowercase().contains("exist") {
                return Err(already_exists());
            }
            return Err(output.error());
        }
        let raw = self.stat_remote(&alias, &path).await?;
        Ok(entry_at(&alias, &path, &raw))
    }

    async fn rename(&self, from: &Location, to: &Location) -> Result<Entry, Error> {
        let (alias, source) = self.resolve(from).await?;
        let (to_alias, target) = self.resolve(to).await?;
        if to_alias != alias {
            return Err(Error::Unsupported("a rename stays on one host"));
        }
        if self.exists(&alias, &target).await? {
            return Err(already_exists());
        }
        self.pool
            .run_ok(
                &alias,
                Command::new("mv")
                    .raw(" --")
                    .arg(&source)
                    .arg(&target)
                    .as_bytes(),
            )
            .await?;
        let raw = self.stat_remote(&alias, &target).await?;
        Ok(entry_at(&alias, &target, &raw))
    }

    async fn remove(&self, loc: &Location, mode: RemoveMode) -> Result<(), Error> {
        if mode == RemoveMode::Trash {
            return Err(Error::Unsupported("a remote host has no trash"));
        }
        let (alias, path) = self.resolve(loc).await?;
        if path.is_empty() || path == b"/" {
            return Err(Error::Unsupported("the root of a host cannot be removed"));
        }
        if !self.exists(&alias, &path).await? {
            return Err(Error::NotFound);
        }
        self.pool
            .run_ok(
                &alias,
                Command::new("rm").raw(" -rf --").arg(&path).as_bytes(),
            )
            .await
            .map(drop)
    }

    async fn link(&self, target: &Location, at: &Location, kind: LinkKind) -> Result<Entry, Error> {
        let (alias, link_path) = self.resolve(at).await?;
        // The target as the link's text: a path of the same host as its
        // remote path, anything else as typed.
        let target_text = match split(target) {
            Some((t_alias, t_remote)) if t_alias == alias => {
                if t_remote.is_empty() {
                    self.pool.default_dir(&alias).await?
                } else {
                    t_remote
                }
            }
            _ => target.path.as_bytes().to_vec(),
        };
        let flag = match kind {
            LinkKind::Symbolic => " -s --",
            LinkKind::Hard => " --",
            LinkKind::Junction | LinkKind::Shortcut => {
                return Err(Error::Unsupported(
                    "junctions and shortcuts are Windows links; a Unix host has none",
                ));
            }
        };
        self.pool
            .run_ok(
                &alias,
                Command::new("ln")
                    .raw(flag)
                    .arg(&target_text)
                    .arg(&link_path)
                    .as_bytes(),
            )
            .await?;
        let raw = self.stat_remote(&alias, &link_path).await?;
        Ok(entry_at(&alias, &link_path, &raw))
    }

    /// One file copied on the host with `cp -p`; across two hosts the bytes
    /// stream through the machine.
    async fn copy(
        &self,
        from: &Location,
        to: &Location,
        opts: CopyOptions,
        progress: ProgressSink,
        cancel: CancelToken,
    ) -> Result<(), Error> {
        let (alias, source) = self.resolve(from).await?;
        let (to_alias, target) = self.resolve(to).await?;
        if to_alias != alias {
            return self.stream_copy(from, to, opts, &progress, &cancel).await;
        }
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        if !opts.overwrite && self.exists(&alias, &target).await? {
            return Err(already_exists());
        }
        let raw = self.stat_remote(&alias, &source).await?;
        self.pool
            .run_ok(
                &alias,
                Command::new("cp")
                    .raw(" -p --")
                    .arg(&source)
                    .arg(&target)
                    .as_bytes(),
            )
            .await?;
        progress.report(Progress::Bytes {
            done: raw.size,
            total: Some(raw.size),
        });
        Ok(())
    }

    /// A move or copy within one host runs on the host: `mv`, or `cp -a`
    /// with the target's size as the progress.
    async fn transfer(
        &self,
        items: &[Location],
        to: &Location,
        mode: TransferMode,
        progress: ProgressSink,
        cancel: CancelToken,
    ) -> Result<(), Error> {
        let (alias, dir) = self.resolve(to).await?;
        let total = items.len() as u64;
        for (done, item) in items.iter().enumerate() {
            if cancel.is_cancelled() {
                return Err(Error::Cancelled);
            }
            progress.report(Progress::Items {
                done: done as u64,
                total: Some(total),
            });
            let (item_alias, item_path) = self.resolve(item).await?;
            if item_alias != alias {
                return Err(Error::Unsupported("a native transfer stays on one host"));
            }
            debug!(alias, item = %String::from_utf8_lossy(&item_path), dir = %String::from_utf8_lossy(&dir), ?mode, "native transfer");
            let size = match mode {
                TransferMode::Copy => self
                    .stat_remote(&alias, &item_path)
                    .await
                    .ok()
                    .map(|r| r.size),
                TransferMode::Move => None,
            };
            self.transfer_one(&alias, &item_path, &dir, mode, size, &progress, &cancel)
                .await?;
        }
        progress.report(Progress::Items {
            done: total,
            total: Some(total),
        });
        Ok(())
    }

    fn remote_preview_cap(&self, loc: &Location) -> Option<u64> {
        let (alias, _) = split(loc)?;
        let host = self.pool.hosts().get(&alias)?;
        Some(
            host.preview_cap
                .unwrap_or_else(|| self.pool.hosts().settings().preview_cap),
        )
    }

    fn watch(&self, _loc: &Location) -> Result<ChangeStream, Error> {
        Err(Error::Unsupported(
            "a remote host sends no change notifications; the folder refreshes on request",
        ))
    }
}
