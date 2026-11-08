//! The plugin boundary, proven with the fixture plugins (QUESTIONS.md 81):
//! the probe library registers a previewer whose `open` spawns a job, the
//! job runs on the host's runtime and its tracing event reaches the host's
//! subscriber; the panicking library is caught and the host carries on,
//! and loading the probe a second time in the same process is fine because
//! a library whose code ran is never unmapped (QUESTIONS.md 85).

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use ext::{Frame, RenderRequest};
use platform::Fake;
use plugins_native::{LoadError, load_library};
use types::{CancelToken, Location};
use vfs_local::LocalProvider;

/// The fixture library `name` beside the test binary: cargo builds a
/// dev-dependency's `cdylib` into the `deps` directory under its plain
/// name.
fn fixture(name: &str) -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_else(|e| panic!("{e}"));
    let deps = exe
        .parent()
        .unwrap_or_else(|| panic!("no parent of {}", exe.display()));
    let file = format!(
        "{}{name}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    let path = deps.join(file);
    assert!(path.is_file(), "fixture not built: {}", path.display());
    path
}

/// Everything the process logs, through the one global subscriber.
#[derive(Clone)]
struct Log(Arc<Mutex<Vec<u8>>>);

impl Write for Log {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn log() -> Log {
    static LOG: OnceLock<Log> = OnceLock::new();
    LOG.get_or_init(|| {
        let log = Log(Arc::default());
        let writer = log.clone();
        let _ = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(move || writer.clone())
            .try_init();
        log
    })
    .clone()
}

fn logged() -> String {
    String::from_utf8_lossy(&log().0.lock().unwrap_or_else(PoisonError::into_inner)).into_owned()
}

#[test]
fn the_probe_registers_and_its_job_runs_on_the_host_runtime() {
    let _ = log();
    let loaded = load_library(
        &fixture("plugin_probe"),
        Arc::new(platform::Native::new()),
        None,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(loaded.manifest().id.0, "probe");
    assert_eq!(loaded.manifest().name, "Probe");
    assert_eq!(loaded.contributions().previewers().len(), 1);
    assert_eq!(loaded.contributions().actions().len(), 1);
    assert!(logged().contains("probe: registering"), "{}", logged());

    // The application absorbs the plugin's contributions like a built-in's.
    let mut registry = ext::Registry::new();
    registry.load(&loaded).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(registry.extensions(), &[loaded.manifest().id]);
    assert_eq!(registry.previewers().len(), 1);

    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let file = dir.path().join("hello.probe");
    std::fs::write(&file, "hello from the host").unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let provider: Arc<dyn ext::Provider> = Arc::new(LocalProvider::new(fake));
    let loc = Location::new(vfs_local::PROVIDER_ID, platform::paths::from_path(&file));
    let previewer = Arc::clone(&registry.previewers()[0]);

    let (text, status) = runtime::block_on(async {
        let entry = provider.stat(&loc).await.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(previewer.score(&entry, b"hello"), 255);
        let mut doc = previewer
            .open(&loc, Arc::clone(&provider), CancelToken::new())
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        let frame = doc
            .render(
                0,
                RenderRequest {
                    width: 100,
                    height: 100,
                    scale: 1.0,
                    dark: false,
                },
            )
            .unwrap_or_else(|e| panic!("{e}"));
        let Frame::Text(block) = frame else {
            panic!("not a text frame: {frame:?}");
        };
        (block.text, doc.status())
    });
    assert_eq!(text, "hello from the host");
    // The job ran on a worker thread of the host's runtime, not on a
    // runtime of the plugin's own.
    assert_eq!(status, ["job ran on rick-rt"], "{status:?}");
    let logged = logged();
    assert!(logged.contains("probe: opening"), "{logged}");
    assert!(logged.contains("probe: read in a job"), "{logged}");
}

#[test]
fn a_plugin_that_panics_while_registering_is_caught_and_the_host_carries_on() {
    let _ = log();
    let error = match load_library(
        &fixture("plugin_panic"),
        Arc::new(platform::Native::new()),
        None,
    ) {
        Err(error) => error,
        Ok(loaded) => panic!("loaded {loaded:?}"),
    };
    assert!(
        matches!(&error, LoadError::Panicked(reason) if reason.contains("the panic fixture refuses to register")),
        "{error}"
    );
    // The host carries on: the next plugin loads.
    let loaded = load_library(
        &fixture("plugin_probe"),
        Arc::new(platform::Native::new()),
        None,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(loaded.manifest().id.0, "probe");
}

#[test]
fn a_missing_or_foreign_library_is_an_open_error() {
    let missing = std::env::temp_dir().join("no-such-plugin.so");
    assert!(matches!(
        load_library(&missing, Arc::new(platform::Native::new()), None),
        Err(LoadError::Open(_))
    ));
}
