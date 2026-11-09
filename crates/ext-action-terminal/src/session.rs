//! A shell in a pseudo-terminal: the [`ext::Console`] the UI draws.
//!
//! The session opens a pty, starts the user's shell in the folder, reads
//! what the shell writes on the runtime's blocking pool and feeds it
//! through the `vte` parser into a [`Screen`]; the UI reads the screen and
//! writes what is typed. A second task of the pool waits for the shell to
//! end and records its exit status: on Windows the reader sees no end of
//! file while the pseudo console is open, so that task closes it once the
//! shell is gone, and the reader stops. The waker rings after every
//! change, so the tab repaints only then. Dropping the session ends the
//! shell.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use ext::terminal::Screen;
use ext::{Console, ConsoleWaker};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize};
use types::Error;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Plain state; a panic elsewhere leaves it usable.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A shell in a pseudo-terminal, with the screen of its output.
pub struct Session {
    screen: Mutex<Screen>,
    /// The pty, until the shell has ended; the waiter closes it then.
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    writer: Mutex<Box<dyn Write + Send>>,
    /// Ends the shell without the child, which the waiter owns.
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    exit: Mutex<Option<i32>>,
    waker: Mutex<Option<ConsoleWaker>>,
    generation: AtomicU64,
    folder: String,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("folder", &self.folder)
            .field("exit", &*lock(&self.exit))
            .finish_non_exhaustive()
    }
}

/// A pty library error as the kernel's: `portable-pty` reports through
/// `anyhow`, of which only the text is kept.
fn pty_error(error: &dyn std::fmt::Display) -> Error {
    Error::Extension {
        ext: crate::ID,
        source: error.to_string().into(),
    }
}

impl Session {
    /// Starts the user's shell in `dir` on a `cols × rows` terminal.
    ///
    /// # Errors
    ///
    /// `Error::Extension` when the pseudo-terminal cannot be opened or the
    /// shell not started.
    pub fn spawn(dir: &Path, cols: u16, rows: u16) -> Result<Arc<Self>, Error> {
        let dir = std::path::absolute(dir).map_err(Error::from_io)?;
        let system = portable_pty::native_pty_system();
        let pair = system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| pty_error(&e))?;
        let mut command = CommandBuilder::new_default_prog();
        command.cwd(&dir);
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|e| pty_error(&e))?;
        drop(pair.slave);
        let reader = pair.master.try_clone_reader().map_err(|e| pty_error(&e))?;
        let writer = pair.master.take_writer().map_err(|e| pty_error(&e))?;
        let folder = dir.file_name().map_or_else(
            || dir.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let session = Arc::new(Self {
            screen: Mutex::new(Screen::new(cols, rows)),
            master: Mutex::new(Some(pair.master)),
            writer: Mutex::new(writer),
            killer: Mutex::new(child.clone_killer()),
            exit: Mutex::new(None),
            waker: Mutex::new(None),
            generation: AtomicU64::new(0),
            folder,
        });
        session.start_reader(reader);
        session.start_waiter(child);
        tracing::info!(dir = %dir.display(), "console opened");
        Ok(session)
    }

    /// Waits for the shell to end, on the blocking pool, and records its
    /// exit status; the session is held weakly, so a closed tab (whose
    /// drop kills the shell) ends the wait without keeping the session.
    /// Closing the pty then stops the reader where the shell's end alone
    /// does not (Windows).
    fn start_waiter(self: &Arc<Self>, mut child: Box<dyn portable_pty::Child + Send + Sync>) {
        let weak = Arc::downgrade(self);
        ext::runtime::handle().spawn_blocking(move || {
            let status = child.wait().ok();
            let Some(session) = weak.upgrade() else {
                return;
            };
            let code = status.map_or(-1, |s| i32::try_from(s.exit_code()).unwrap_or(i32::MAX));
            *lock(&session.exit) = Some(code);
            drop(lock(&session.master).take());
            session.changed();
        });
    }

    /// Reads the shell's output until it ends, on the blocking pool; the
    /// session is held weakly so a closed tab ends the loop.
    fn start_reader(self: &Arc<Self>, mut reader: Box<dyn Read + Send>) {
        let weak = Arc::downgrade(self);
        ext::runtime::handle().spawn_blocking(move || {
            let mut parser = ext::terminal::vte::Parser::new();
            let mut buffer = vec![0u8; 64 * 1024];
            loop {
                let read = match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                let Some(session) = weak.upgrade() else {
                    return;
                };
                let responses = {
                    let mut screen = lock(&session.screen);
                    parser.advance(&mut *screen, &buffer[..read]);
                    screen.take_responses()
                };
                if !responses.is_empty() {
                    let _ = lock(&session.writer).write_all(&responses);
                }
                session.changed();
            }
            // The output has ended; the waiter records the exit status.
            if let Some(session) = weak.upgrade() {
                session.changed();
            }
        });
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

impl Drop for Session {
    fn drop(&mut self) {
        // Ending the shell ends the waiter; the pty closes with the
        // session, and the reader then stops.
        let _ = lock(&self.killer).kill();
    }
}

impl Console for Session {
    fn title(&self) -> String {
        lock(&self.screen)
            .title()
            .map_or_else(|| self.folder.clone(), str::to_owned)
    }

    fn screen(&self, scrolled_back: usize) -> ext::Screen {
        lock(&self.screen).snapshot(scrolled_back)
    }

    fn write(&self, bytes: &[u8]) {
        if lock(&self.exit).is_some() {
            return;
        }
        let mut writer = lock(&self.writer);
        if writer
            .write_all(bytes)
            .and_then(|()| writer.flush())
            .is_err()
        {
            tracing::debug!("console write failed; the shell is gone");
        }
    }

    fn resize(&self, cols: u16, rows: u16) {
        {
            let mut screen = lock(&self.screen);
            screen.resize(cols, rows);
        }
        if let Some(master) = lock(&self.master).as_ref() {
            let _ = master.resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
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
