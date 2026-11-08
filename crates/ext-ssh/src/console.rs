//! *Open console here*: a shell on the host in a PTY `shell` channel of the
//! existing connection, `cd`'d into the browsed folder, drawn by the
//! console pane through the kernel's terminal emulator. No second login.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use ext::terminal::Screen;
use ext::{Console, ConsoleWaker};
use russh::ChannelMsg;
use tokio::sync::mpsc;
use tracing::debug;
use types::Error;

use crate::connection::{Lease, Pool};
use crate::shell::quote;
use crate::ssh_error;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the console tab sends the channel: bytes typed, or a new size.
enum Input {
    Bytes(Vec<u8>),
    Resize(u16, u16),
}

/// A shell on a remote host, as the console pane draws it.
pub struct SshConsole {
    title: String,
    screen: Mutex<Screen>,
    input: mpsc::UnboundedSender<Input>,
    exit: Mutex<Option<i32>>,
    waker: Mutex<Option<ConsoleWaker>>,
    generation: AtomicU64,
}

impl std::fmt::Debug for SshConsole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SshConsole")
            .field("title", &self.title)
            .field("exit", &*lock(&self.exit))
            .finish_non_exhaustive()
    }
}

impl SshConsole {
    /// Starts a shell on `alias` in `dir` (the host's default directory
    /// when empty), on a `cols × rows` terminal, over a channel of the
    /// pool's connection. The console is handed back at once and connects
    /// on the runtime, so an action never blocks the application thread
    /// while a host-key or password dialogue waits for the user; a failure
    /// is printed on the screen and ends the console.
    #[must_use]
    pub fn start(pool: Arc<Pool>, alias: &str, dir: Vec<u8>, cols: u16, rows: u16) -> Arc<Self> {
        let (tx, mut rx) = mpsc::unbounded_channel::<Input>();
        let shown = if dir.is_empty() {
            alias.to_owned()
        } else {
            format!("{alias}: {}", String::from_utf8_lossy(&dir))
        };
        let console = Arc::new(Self {
            title: shown,
            screen: Mutex::new(Screen::new(cols, rows)),
            input: tx,
            exit: Mutex::new(None),
            waker: Mutex::new(None),
            generation: AtomicU64::new(0),
        });
        let weak = Arc::downgrade(&console);
        let alias = alias.to_owned();
        ext::runtime::handle().spawn(async move {
            let opened = Self::open(&pool, &alias, dir, cols, rows).await;
            let Lease { mut channel, token } = match opened {
                Ok(lease) => lease,
                Err(error) => {
                    if let Some(console) = weak.upgrade() {
                        {
                            let mut screen = lock(&console.screen);
                            let mut parser = ext::terminal::vte::Parser::new();
                            parser
                                .advance(&mut *screen, format!("{alias}: {error}\r\n").as_bytes());
                        }
                        *lock(&console.exit) = Some(-1);
                        console.changed();
                    }
                    return;
                }
            };
            let _token = token;
            let mut parser = ext::terminal::vte::Parser::new();
            let mut status: Option<i32> = None;
            loop {
                tokio::select! {
                    msg = channel.wait() => {
                        let Some(msg) = msg else { break };
                        let Some(console) = weak.upgrade() else { break };
                        match msg {
                            ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } => {
                                let responses = {
                                    let mut screen = lock(&console.screen);
                                    parser.advance(&mut *screen, &data);
                                    screen.take_responses()
                                };
                                if !responses.is_empty() {
                                    let _ = channel.data(&responses[..]).await;
                                }
                                console.changed();
                            }
                            ChannelMsg::ExitStatus { exit_status } => {
                                status = Some(i32::try_from(exit_status).unwrap_or(i32::MAX));
                            }
                            ChannelMsg::ExitSignal { .. } => status = Some(128),
                            ChannelMsg::Close => break,
                            _ => {}
                        }
                    }
                    input = rx.recv() => {
                        let Some(input) = input else {
                            // The console was dropped: end the shell.
                            let _ = channel.eof().await;
                            let _ = channel.close().await;
                            break;
                        };
                        match input {
                            Input::Bytes(bytes) => {
                                if channel.data(&bytes[..]).await.is_err() {
                                    debug!("console write failed; the shell is gone");
                                }
                            }
                            Input::Resize(cols, rows) => {
                                let _ = channel
                                    .window_change(u32::from(cols), u32::from(rows), 0, 0)
                                    .await;
                            }
                        }
                    }
                }
            }
            if let Some(console) = weak.upgrade() {
                *lock(&console.exit) = Some(status.unwrap_or(-1));
                console.changed();
            }
        });
        console
    }

    /// Opens the channel: a PTY, then the user's shell `cd`'d into the
    /// folder through `exec`, so it is one login and one shell.
    async fn open(
        pool: &Pool,
        alias: &str,
        dir: Vec<u8>,
        cols: u16,
        rows: u16,
    ) -> Result<Lease, Error> {
        let dir = if dir.is_empty() {
            pool.default_dir(alias).await?
        } else {
            dir
        };
        let lease = pool.lease(alias).await?;
        lease
            .channel
            .request_pty(
                true,
                "xterm-256color",
                u32::from(cols),
                u32::from(rows),
                0,
                0,
                &[],
            )
            .await
            .map_err(ssh_error)?;
        let mut command = b"cd ".to_vec();
        command.extend_from_slice(&quote(&dir));
        command.extend_from_slice(b" 2>/dev/null; exec \"${SHELL:-sh}\" -l");
        lease.channel.exec(true, command).await.map_err(ssh_error)?;
        Ok(lease)
    }

    fn changed(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        let waker = lock(&self.waker).clone();
        if let Some(wake) = waker {
            wake();
        }
    }

    /// The screen's text, for tests and logs.
    #[must_use]
    pub fn text(&self) -> String {
        lock(&self.screen).text()
    }
}

impl Console for SshConsole {
    fn title(&self) -> String {
        lock(&self.screen)
            .title()
            .map_or_else(|| self.title.clone(), str::to_owned)
    }

    fn screen(&self, scrolled_back: usize) -> ext::Screen {
        lock(&self.screen).snapshot(scrolled_back)
    }

    fn write(&self, bytes: &[u8]) {
        if lock(&self.exit).is_some() {
            return;
        }
        let _ = self.input.send(Input::Bytes(bytes.to_vec()));
    }

    fn resize(&self, cols: u16, rows: u16) {
        lock(&self.screen).resize(cols, rows);
        let _ = self.input.send(Input::Resize(cols, rows));
        self.changed();
    }

    fn exit_status(&self) -> Option<i32> {
        *lock(&self.exit)
    }

    fn set_waker(&self, waker: ConsoleWaker) {
        *lock(&self.waker) = Some(waker);
    }

    fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }
}
