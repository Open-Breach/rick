//! The service behind the index: a command line run as a child process,
//! or, in the tests, a fake that plays back canned output.

use std::fmt;
use std::io::{BufRead as _, BufReader, Write as _};
use std::process::Stdio;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use ext::BoxStream;
use futures::StreamExt;
use futures::stream;
use types::{CancelToken, Error};

/// One invocation of a tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    /// The program, found on the `PATH`.
    pub program: String,
    /// Its arguments.
    pub args: Vec<String>,
    /// Text fed to its standard input, when it reads its request there.
    pub stdin: Option<String>,
}

impl Command {
    /// A command without input.
    #[must_use]
    pub fn new(program: &str, args: &[&str]) -> Self {
        Self {
            program: program.to_owned(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
            stdin: None,
        }
    }
}

impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.program)?;
        for arg in &self.args {
            write!(f, " {arg}")?;
        }
        Ok(())
    }
}

/// Lines per batch before they are handed on.
const BATCH: usize = 256;
/// A batch is handed on after this long even when it is short, so sparse
/// output still reaches the list promptly.
const FLUSH_AFTER: Duration = Duration::from_millis(40);

/// Runs the index's command line.
pub trait Service: Send + Sync + 'static {
    /// Runs `command` to its end: its standard output when it exited with
    /// success, what it wrote to standard error otherwise, or why it could
    /// not run. Called on the blocking pool.
    ///
    /// # Errors
    ///
    /// The tool's standard error, or the reason it could not start.
    fn output(&self, command: &Command) -> Result<String, String>;

    /// Runs `command` and streams the lines of its standard output in
    /// batches as they come. The stream ends with the tool, or with one
    /// error when the tool failed before printing anything; cancelling the
    /// token ends the tool.
    fn stream(
        &self,
        command: Command,
        cancel: CancelToken,
    ) -> BoxStream<'static, Result<Vec<String>, Error>>;
}

/// Wraps a tool's failure as the kernel's extension error.
#[must_use]
pub fn tool_error(text: impl Into<String>) -> Error {
    Error::Extension {
        ext: crate::ID,
        source: text.into().into(),
    }
}

/// The real service: child processes. The lines are read on the blocking
/// pool, since the shared runtime drives no I/O of its own; cancelling, or
/// dropping the stream, kills the child.
#[derive(Debug, Clone, Copy, Default)]
pub struct Process;

impl Service for Process {
    fn output(&self, command: &Command) -> Result<String, String> {
        let mut child = std::process::Command::new(&command.program)
            .args(&command.args)
            .stdin(if command.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("{} could not be run: {e}", command.program))?;
        if let (Some(input), Some(mut stdin)) = (&command.stdin, child.stdin.take()) {
            // A tool that stops reading is reported through its exit status.
            let _ = stdin.write_all(input.as_bytes());
        }
        let output = child
            .wait_with_output()
            .map_err(|e| format!("{}: {e}", command.program))?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            Err(if stderr.is_empty() { stdout } else { stderr })
        }
    }

    fn stream(
        &self,
        command: Command,
        cancel: CancelToken,
    ) -> BoxStream<'static, Result<Vec<String>, Error>> {
        let mut child = match std::process::Command::new(&command.program)
            .args(&command.args)
            .stdin(if command.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(error) => {
                let text = format!("{} could not be run: {error}", command.program);
                return stream::once(async move { Err(tool_error(text)) }).boxed();
            }
        };
        let stdin = child.stdin.take();
        let Some(stdout) = child.stdout.take() else {
            return stream::once(async { Err(tool_error("no standard output")) }).boxed();
        };
        let stderr = child.stderr.take();
        let child = Arc::new(Mutex::new(child));
        let (tx, rx) = ext::runtime::mpsc::channel::<Result<Vec<String>, Error>>(CHANNEL_DEPTH);
        let reader = Reader {
            child: Arc::clone(&child),
            program: command.program.clone(),
            tx,
            cancel: cancel.clone(),
        };
        ext::runtime::handle().spawn_blocking(move || {
            reader.run(command.stdin.zip(stdin), stdout, stderr);
        });
        let guard = KillOnDrop(child);
        stream::unfold((rx, cancel, guard), |(mut rx, cancel, guard)| async move {
            if cancel.is_cancelled() {
                return None;
            }
            tokio::select! {
                () = cancel.cancelled() => None,
                next = rx.recv() => next.map(|item| (item, (rx, cancel, guard))),
            }
        })
        .boxed()
    }
}

/// Batches buffered between the reading thread and a slow consumer.
const CHANNEL_DEPTH: usize = 16;

/// Kills the child when the stream is dropped, cancelled or exhausted.
struct KillOnDrop(Arc<Mutex<std::process::Child>>);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let mut child = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        // A child that already exited is fine; nothing else to do here.
        let _ = child.kill();
    }
}

/// Reads a child's output on the blocking pool.
struct Reader {
    child: Arc<Mutex<std::process::Child>>,
    program: String,
    tx: ext::runtime::mpsc::Sender<Result<Vec<String>, Error>>,
    cancel: CancelToken,
}

impl Reader {
    fn run(
        self,
        input: Option<(String, std::process::ChildStdin)>,
        stdout: std::process::ChildStdout,
        stderr: Option<std::process::ChildStderr>,
    ) {
        if let Some((text, mut stdin)) = input {
            // A tool that stops reading is reported through its exit status.
            let _ = stdin.write_all(text.as_bytes());
            drop(stdin);
        }
        let mut lines = BufReader::new(stdout).lines();
        let mut batch = Vec::new();
        let mut printed = false;
        let mut last_flush = Instant::now();
        let mut read_error = None;
        loop {
            if self.cancel.is_cancelled() {
                return;
            }
            match lines.next() {
                Some(Ok(line)) => {
                    printed = true;
                    batch.push(line);
                    if batch.len() >= BATCH || last_flush.elapsed() >= FLUSH_AFTER {
                        if self
                            .tx
                            .blocking_send(Ok(std::mem::take(&mut batch)))
                            .is_err()
                        {
                            return;
                        }
                        last_flush = Instant::now();
                    }
                }
                Some(Err(error)) => {
                    read_error = Some(error);
                    break;
                }
                None => break,
            }
        }
        if !batch.is_empty() && self.tx.blocking_send(Ok(batch)).is_err() {
            return;
        }
        if let Some(error) = read_error {
            let _ = self
                .tx
                .blocking_send(Err(tool_error(format!("{}: {error}", self.program))));
            return;
        }
        // The tool closed its output: its complaint, when it failed without
        // printing any path.
        let status = self
            .child
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .wait();
        match status {
            Ok(status) if status.success() || printed => {}
            Ok(status) => {
                let mut text = String::new();
                if let Some(mut stderr) = stderr {
                    let _ = std::io::Read::read_to_string(&mut stderr, &mut text);
                }
                let text = text.trim();
                let _ = self.tx.blocking_send(Err(tool_error(if text.is_empty() {
                    format!("{} exited with {status}", self.program)
                } else {
                    format!("{}: {text}", self.program)
                })));
            }
            Err(error) => {
                let _ = self
                    .tx
                    .blocking_send(Err(tool_error(format!("{}: {error}", self.program))));
            }
        }
    }
}

/// A service for the tests: answers every `output` from a script, every
/// `stream` from canned lines, and keeps the commands it was given.
#[derive(Debug)]
pub struct Fake {
    /// What `output` answers, by program name; a program not listed could
    /// not be run.
    outputs: Mutex<Vec<(String, Result<String, String>)>>,
    /// What `stream` prints, or the error it fails with.
    lines: Mutex<Result<Vec<String>, String>>,
    /// Every command run, in order.
    commands: Mutex<Vec<Command>>,
}

impl Fake {
    /// A fake that runs nothing yet.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            outputs: Mutex::new(Vec::new()),
            lines: Mutex::new(Ok(Vec::new())),
            commands: Mutex::new(Vec::new()),
        })
    }

    /// `program` answers `output` to `Service::output`.
    pub fn answers(&self, program: &str, output: Result<&str, &str>) {
        let mut outputs = self.outputs.lock().unwrap_or_else(PoisonError::into_inner);
        outputs.retain(|(p, _)| p != program);
        outputs.push((
            program.to_owned(),
            output.map(str::to_owned).map_err(str::to_owned),
        ));
    }

    /// `program` can no longer be run.
    pub fn forgets(&self, program: &str) {
        self.outputs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|(p, _)| p != program);
    }

    /// The next `stream` prints these lines.
    pub fn prints(&self, lines: &[String]) {
        *self.lines.lock().unwrap_or_else(PoisonError::into_inner) = Ok(lines.to_vec());
    }

    /// The next `stream` fails with this text.
    pub fn fails(&self, text: &str) {
        *self.lines.lock().unwrap_or_else(PoisonError::into_inner) = Err(text.to_owned());
    }

    /// The commands run so far.
    #[must_use]
    pub fn commands(&self) -> Vec<Command> {
        self.commands
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, command: &Command) {
        self.commands
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(command.clone());
    }
}

impl Service for Fake {
    fn output(&self, command: &Command) -> Result<String, String> {
        self.record(command);
        self.outputs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|(program, _)| *program == command.program)
            .map_or_else(
                || Err(format!("{} could not be run", command.program)),
                |(_, output)| output.clone(),
            )
    }

    fn stream(
        &self,
        command: Command,
        _cancel: CancelToken,
    ) -> BoxStream<'static, Result<Vec<String>, Error>> {
        self.record(&command);
        match self
            .lines
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
        {
            Ok(lines) => {
                stream::iter(lines.chunks(2).map(|c| Ok(c.to_vec())).collect::<Vec<_>>()).boxed()
            }
            Err(text) => stream::once(async move { Err(tool_error(text)) }).boxed(),
        }
    }
}
