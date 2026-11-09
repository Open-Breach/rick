//! *"Show in folder" from other apps* (specs/system-integration.md §7.1):
//! the freedesktop file-manager interface, `org.freedesktop.FileManager1`,
//! which browsers, chat and download tools call to show a file in the
//! file manager. The running Rick owns the name on the session bus and
//! hands every call to the closure it serves with; the binary turns them
//! into the commands of the command line (§4: `ShowItems` is `reveal`,
//! `ShowFolders` is open, `ShowItemProperties` is open plus the
//! Properties dialog), so the bus is one more entry point and no second
//! code path.
//!
//! Linux only: the session bus through `zbus`, on the one runtime. On
//! macOS and Windows [`serve`] answers `Unsupported`.

use std::sync::Arc;

use types::Error;

/// The well-known name of the interface on the session bus.
pub const BUS_NAME: &str = "org.freedesktop.FileManager1";
/// The object the interface is served at.
pub const OBJECT_PATH: &str = "/org/freedesktop/FileManager1";
/// The argument the activation file starts Rick with, when another
/// application calls the interface and no Rick is running: the instance
/// starts for the bus and opens no folder of its own.
pub const SERVICE_ARGUMENT: &str = "--dbus-service";

/// What another application asks the file manager to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Show {
    /// `ShowItems`: open each item's folder and select the item.
    Items,
    /// `ShowFolders`: open each folder.
    Folders,
    /// `ShowItemProperties`: show each item and its properties.
    ItemProperties,
}

/// One call on the interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    /// Which method.
    pub show: Show,
    /// The `file://` URIs it names, as the caller sent them.
    pub uris: Vec<String>,
    /// The caller's startup id, an activation token for the window that
    /// shows the result; empty when the caller gave none.
    pub startup_id: String,
}

/// Who takes the calls: called on a task of the runtime, so it hands the
/// call on rather than acting on it.
pub type OnCall = Arc<dyn Fn(Call) + Send + Sync>;

/// The served interface: the bus name is Rick's for as long as this
/// lives, and released when it is dropped.
pub struct Service {
    #[cfg(target_os = "linux")]
    _connection: crate::linux::file_manager::Connection,
}

impl std::fmt::Debug for Service {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Service").finish_non_exhaustive()
    }
}

/// Serves the interface on the bus at `address` — the session bus of the
/// process's environment for `None` — and asks for the name. Another
/// file manager that owns the name keeps it; Rick is queued and takes
/// over when the name is released.
///
/// # Errors
///
/// `Error::Unsupported` off Linux; otherwise the error of reaching the
/// bus (no session bus in this environment) or of registering on it.
// Awaits nothing where there is no bus to reach.
#[cfg_attr(not(target_os = "linux"), allow(clippy::unused_async))]
pub async fn serve(address: Option<&str>, on_call: OnCall) -> Result<Service, Error> {
    #[cfg(target_os = "linux")]
    {
        let connection = crate::linux::file_manager::serve(address, on_call).await?;
        Ok(Service {
            _connection: connection,
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (address, on_call);
        Err(Error::Unsupported(
            "the freedesktop file-manager interface is a Linux session bus service",
        ))
    }
}
