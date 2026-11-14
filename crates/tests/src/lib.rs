//! Integration tests and the generated fixture tree for Rick.
//!
//! The crate exposes [`fixture`], which generates a deterministic directory
//! tree of N entries for tests and benchmarks, [`media`], which writes a
//! photo with EXIF and a PDF of any page size, [`mem`], an in-memory
//! provider that stands in for a cloud provider, [`mod@bench`], the
//! targets of the acceptance benchmarks per operating system, and
//! [`readme`], the parts of `README.md` that are regenerated with every
//! release from what the extensions register. The integration tests
//! themselves live under `tests/`, as one test binary (`tests/main.rs`),
//! so they share a process; [`alone`] gives a test a process of its own.

#![forbid(unsafe_code)]

pub mod bench;
pub mod fixture;
pub mod media;
pub mod mem;
pub mod readme;

/// A local location for an OS path.
#[must_use]
pub fn local(path: &std::path::Path) -> types::Location {
    types::Location::new(vfs_local::PROVIDER_ID, platform::paths::from_path(path))
}

/// Runs the test `name` in a process of its own, and says whether the
/// caller is that process.
///
/// The integration tests are one binary and run side by side in one
/// process. A test that changes what the process has once, the interface
/// language say, would change it under every other test, and they under
/// it. Such a test calls this first, with its name as the test harness
/// prints it, and returns when the answer is `false`:
///
/// ```no_run
/// fn switching_the_language() {
///     if !tests::alone("locales::switching_the_language") {
///         return;
///     }
///     // The test itself.
/// }
/// ```
///
/// In the run of the whole binary the call starts the binary again for
/// this one test, waits for it and passes its failure on with its output;
/// in that second process the call says `true` and the test runs, alone.
///
/// # Panics
///
/// When the second process cannot be started, when the test fails in it,
/// or when it did not run exactly this one test (a name that is not the
/// test's).
#[must_use]
pub fn alone(name: &str) -> bool {
    /// Names the test of the second process, in its environment.
    const MARK: &str = "RICK_TEST_ALONE";
    if std::env::var(MARK).is_ok_and(|test| test == name) {
        return true;
    }
    let exe = std::env::current_exe().unwrap_or_else(|e| panic!("{e}"));
    let output = std::process::Command::new(&exe)
        .args([name, "--exact", "--include-ignored", "--nocapture"])
        .env(MARK, name)
        .output()
        .unwrap_or_else(|e| panic!("{}: {e}", exe.display()));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success() && stdout.contains("test result: ok. 1 passed;"),
        "{name} in a process of its own ({}):\n{stdout}\n{stderr}",
        output.status
    );
    false
}

/// The text of the file at `path` once `done` holds for it, or as it is
/// after ten seconds. The settings file is written by a task of the
/// runtime after the command that changed it, which a test's `settle` does
/// not wait for, so a test that looks at what a command wrote reads the
/// file this way rather than once: the Linux arm64 runner read `rick.toml`
/// before an edit had reached it (full.yml run 42522726087).
///
/// # Panics
///
/// When the file cannot be read for a reason other than not existing yet.
#[must_use]
pub fn read_until(path: &std::path::Path, done: impl Fn(&str) -> bool) -> String {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => panic!("{}: {e}", path.display()),
        };
        if done(&text) || std::time::Instant::now() >= deadline {
            return text;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}
