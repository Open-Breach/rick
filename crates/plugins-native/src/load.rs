//! Opening a plugin library and calling its entry point.

use std::fmt;
use std::path::{Path, PathBuf};

use ext::plugin::{ENTRY_POINT, EntryPoint, Status};
use ext::{Extension, Manifest, Registry};
use tracing::{debug, warn};
use types::Error;

/// Why a library could not be loaded as a plugin.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    /// The dynamic loader refused the file: missing, not a library, or
    /// built against another kernel build whose libraries it cannot find.
    #[error("the library could not be loaded: {0}")]
    Open(#[source] libloading::Error),
    /// The library is not a Rick plugin.
    #[error("the library exports no `{ENTRY_POINT}`: {0}")]
    NoEntryPoint(#[source] libloading::Error),
    /// The registry refused the manifest or the extension's `register`
    /// failed.
    #[error("{0}")]
    Failed(String),
    /// The plugin panicked while registering; caught, the library unloaded.
    #[error("{0}")]
    Panicked(String),
    /// The entry point registered several extensions or none; a plugin is
    /// one extension.
    #[error("the entry point registered {0} extensions instead of one")]
    NotOne(usize),
}

/// A plugin library with its extension registered: what the application
/// loads like a built-in extension.
///
/// Keeps the library mapped for as long as it lives, because every
/// contribution in it points into the library's code; the host keeps every
/// loaded plugin for the process lifetime.
pub struct Loaded {
    manifest: Manifest,
    /// The plugin's contributions, as the entry point registered them.
    /// Declared before `library` so they drop first, while the code their
    /// `Drop` implementations live in is still mapped.
    contributions: Registry,
    path: PathBuf,
    _library: libloading::Library,
}

impl Loaded {
    /// The plugin's manifest.
    #[must_use]
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// The contributions the plugin registered.
    #[must_use]
    pub fn contributions(&self) -> &Registry {
        &self.contributions
    }

    /// The library file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Extension for Loaded {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        reg.absorb(&self.contributions);
        Ok(())
    }
}

impl fmt::Debug for Loaded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Loaded")
            .field("manifest", &self.manifest)
            .field("contributions", &self.contributions)
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

/// Opens the library at `path`, calls its entry point with a fresh
/// registry and returns the plugin with its contributions.
///
/// The caller has checked the plugin's manifest file first: the toolchain
/// and the API version are what make the library's entry point safe to
/// call. On any error the library is unloaded again before returning.
///
/// # Errors
///
/// [`LoadError`] says what went wrong; the reason a plugin gave for failing
/// or the message of the panic it raised is in the error's text.
pub fn load_library(path: &Path) -> Result<Loaded, LoadError> {
    debug!(path = %path.display(), "loading");
    // SAFETY: loading runs the library's initialisers. A Rust cdylib built
    // from this workspace has none beyond the runtime's own, and the caller
    // has checked the manifest that says it is one.
    let library = unsafe { libloading::Library::new(path) }.map_err(LoadError::Open)?;
    let mut registry = Registry::new();
    let status = {
        // SAFETY: `EntryPoint` is the signature `ext::plugin!` declares for
        // this symbol; the manifest checks (API version, toolchain) make
        // the library one built against the same `ext`.
        let entry: libloading::Symbol<EntryPoint> =
            unsafe { library.get(ENTRY_POINT.as_bytes()) }.map_err(LoadError::NoEntryPoint)?;
        // SAFETY: the entry point catches its own panics and only touches
        // the registry it is given, which outlives the call.
        unsafe { entry(&mut registry) }
    };
    let failure = || registry.failure().unwrap_or("no reason given").to_owned();
    let result = match status {
        Status::Registered => match registry.manifests() {
            [manifest] => Ok(manifest.clone()),
            other => Err(LoadError::NotOne(other.len())),
        },
        Status::Failed => Err(LoadError::Failed(failure())),
        Status::Panicked => Err(LoadError::Panicked(failure())),
    };
    match result {
        Ok(manifest) => Ok(Loaded {
            manifest,
            contributions: registry,
            path: path.to_owned(),
            _library: library,
        }),
        Err(error) => {
            warn!(path = %path.display(), %error, "plugin not loaded");
            // Everything that points into the library goes before the
            // library does.
            drop(registry);
            drop(library);
            Err(error)
        }
    }
}
