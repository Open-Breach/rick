//! The session file and the saved layouts: `session.toml` and
//! `layouts/<name>.toml` in the config directory.
//!
//! The session is written continuously, debounced to at most once per
//! [`SAVE_INTERVAL`], always through a temporary file renamed over the real
//! one, so a crash never leaves half a file. The exit hook writes once more,
//! synchronously, and stops the writer so an older snapshot in flight can
//! never land after it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use model::Session;
use runtime::mpsc;
use tracing::{debug, warn};
use types::Error;

/// The file name under the config directory.
pub const SESSION_FILE: &str = "session.toml";
/// The directory of saved layouts under the config directory.
pub const LAYOUTS_DIR: &str = "layouts";
/// How often the session is written at most while it keeps changing.
pub const SAVE_INTERVAL: Duration = Duration::from_secs(1);

/// Reads a session file.
///
/// # Errors
///
/// `Error::NotFound` when there is no such file, the I/O error, or
/// `Error::Io` with `InvalidData` when the TOML does not parse.
pub fn load(path: &Path) -> Result<Session, Error> {
    let text = std::fs::read_to_string(path).map_err(Error::from_io)?;
    toml::from_str(&text).map_err(|error| {
        Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            error.to_string(),
        ))
    })
}

/// Writes a session file: into a temporary file beside it, then renamed
/// over it, so the file is either the old one or the new one.
///
/// # Errors
///
/// The I/O error.
pub fn save(path: &Path, session: &Session) -> Result<(), Error> {
    let text = toml::to_string_pretty(session)
        .map_err(|error| Error::Io(std::io::Error::other(error.to_string())))?;
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    if !dir.as_os_str().is_empty() {
        std::fs::create_dir_all(&dir).map_err(Error::from_io)?;
    }
    let name = path.file_name().map_or_else(
        || "session".to_owned(),
        |n| n.to_string_lossy().into_owned(),
    );
    let temp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    std::fs::write(&temp, text).map_err(Error::from_io)?;
    if let Err(error) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(Error::from_io(error));
    }
    Ok(())
}

/// What the writer task and the exit hook share.
struct Shared {
    path: PathBuf,
    /// One write at a time, and the exit hook's last.
    write: Mutex<()>,
    /// Set by the exit hook; the task writes nothing after it.
    closed: AtomicBool,
    /// How many times the task wrote the file.
    writes: AtomicU64,
}

/// Saves the session off the loop's thread, at most once per
/// [`SAVE_INTERVAL`], always the latest snapshot sent.
pub struct Saver {
    tx: mpsc::UnboundedSender<Session>,
    shared: Arc<Shared>,
}

impl Saver {
    /// Starts the writer for `path`.
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        let shared = Arc::new(Shared {
            path,
            write: Mutex::new(()),
            closed: AtomicBool::new(false),
            writes: AtomicU64::new(0),
        });
        let (tx, mut rx) = mpsc::unbounded_channel::<Session>();
        let task = Arc::clone(&shared);
        runtime::handle().spawn(async move {
            let mut last_write: Option<Instant> = None;
            while let Some(mut session) = rx.recv().await {
                if let Some(at) = last_write {
                    let wait = SAVE_INTERVAL.saturating_sub(at.elapsed());
                    if !wait.is_zero() {
                        tokio::time::sleep(wait).await;
                    }
                }
                // Only the latest of what arrived meanwhile needs writing.
                while let Ok(newer) = rx.try_recv() {
                    session = newer;
                }
                if task.closed.load(Ordering::SeqCst) {
                    break;
                }
                let shared = Arc::clone(&task);
                let written = runtime::handle()
                    .spawn_blocking(move || {
                        let _guard = shared.write.lock().unwrap_or_else(PoisonError::into_inner);
                        if shared.closed.load(Ordering::SeqCst) {
                            return Ok(());
                        }
                        let result = save(&shared.path, &session);
                        if result.is_ok() {
                            shared.writes.fetch_add(1, Ordering::SeqCst);
                        }
                        result
                    })
                    .await;
                match written {
                    Ok(Ok(())) => debug!("session saved"),
                    Ok(Err(error)) => warn!(%error, "session not saved"),
                    Err(error) => warn!(%error, "session writer stopped"),
                }
                last_write = Some(Instant::now());
            }
        });
        Self { tx, shared }
    }

    /// Queues a snapshot; the latest one is written when the interval
    /// allows.
    pub fn send(&self, session: Session) {
        // The task lives as long as the runtime; a closed channel means the
        // process is going down.
        let _ = self.tx.send(session);
    }

    /// Writes `session` now, on the caller's thread, and stops the task so
    /// nothing older lands afterwards.
    ///
    /// # Errors
    ///
    /// The I/O error.
    pub fn flush(&self, session: &Session) -> Result<(), Error> {
        let _guard = self
            .shared
            .write
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        self.shared.closed.store(true, Ordering::SeqCst);
        save(&self.shared.path, session)
    }

    /// Where the file is.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.shared.path
    }

    /// How many times the task wrote the file so far; the exit hook's write
    /// is not counted.
    #[must_use]
    pub fn writes(&self) -> u64 {
        self.shared.writes.load(Ordering::SeqCst)
    }
}

/// The file of a named layout.
#[must_use]
pub fn layout_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{}.toml", file_stem(name)))
}

/// A layout name as a file stem: path separators and control characters
/// become `_`, so a name can never leave the layouts directory.
fn file_stem(name: &str) -> String {
    let stem: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect();
    if stem.is_empty() || stem == "." || stem == ".." {
        "_".to_owned()
    } else {
        stem
    }
}

/// The names of the saved layouts, sorted, case-insensitively.
#[must_use]
pub fn layouts(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "toml") {
                path.file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
            } else {
                None
            }
        })
        .collect();
    names.sort_by_key(|name| name.to_lowercase());
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_is_atomic_and_load_reports_absence_and_damage() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let path = dir.path().join("deep").join(SESSION_FILE);
        assert!(matches!(load(&path), Err(Error::NotFound)));
        let session = Session {
            focused_window: 0,
            windows: vec![model::WindowSession::default()],
        };
        save(&path, &session).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(load(&path).ok(), Some(session));
        let leftovers: Vec<_> = std::fs::read_dir(dir.path().join("deep"))
            .unwrap_or_else(|e| panic!("{e}"))
            .filter_map(Result::ok)
            .map(|e| e.file_name())
            .collect();
        assert_eq!(leftovers, [SESSION_FILE], "no temporary file stays");
        std::fs::write(&path, "windows = \n").unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(load(&path), Err(Error::Io(_))));
    }

    #[test]
    fn layout_names_become_safe_file_stems() {
        let dir = Path::new("/cfg/layouts");
        assert_eq!(layout_path(dir, "Review"), dir.join("Review.toml"));
        assert_eq!(layout_path(dir, "../x"), dir.join(".._x.toml"));
        assert_eq!(layout_path(dir, "a/b:c"), dir.join("a_b_c.toml"));
        assert_eq!(layout_path(dir, "  "), dir.join("_.toml"));
        assert_eq!(layout_path(dir, ".."), dir.join("_.toml"));
    }

    #[test]
    fn the_writer_debounces_and_the_flush_stops_it() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let path = dir.path().join(SESSION_FILE);
        let saver = Saver::new(path.clone());
        let mut session = Session::default();
        for i in 0..5 {
            session.focused_window = i;
            saver.send(session.clone());
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while saver.writes() == 0 {
            assert!(Instant::now() < deadline, "never written");
            std::thread::sleep(Duration::from_millis(20));
        }
        std::thread::sleep(Duration::from_millis(300));
        assert!(saver.writes() <= 2, "{} writes in 300 ms", saver.writes());
        session.focused_window = 9;
        saver.flush(&session).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(load(&path).map(|s| s.focused_window).ok(), Some(9));
        saver.send(Session::default());
        std::thread::sleep(Duration::from_millis(1300));
        assert_eq!(
            load(&path).map(|s| s.focused_window).ok(),
            Some(9),
            "nothing lands after the flush"
        );
    }
}
