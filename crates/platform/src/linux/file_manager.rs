//! `org.freedesktop.FileManager1` on the session bus, through `zbus` on
//! the one runtime: the three methods of the interface, each handed to
//! the closure the caller serves with.

use types::Error;
use zbus::fdo::RequestNameFlags;

use crate::file_manager::{BUS_NAME, Call, OBJECT_PATH, OnCall, Show};

/// The connection that owns the name; closing with its last handle.
pub(crate) type Connection = zbus::Connection;

/// The object behind the interface.
struct FileManager {
    on_call: OnCall,
}

impl FileManager {
    fn call(&self, show: Show, uris: Vec<String>, startup_id: String) {
        tracing::debug!(?show, ?uris, "a call on org.freedesktop.FileManager1");
        (self.on_call)(Call {
            show,
            uris,
            startup_id,
        });
    }
}

// The interface as freedesktop.org documents it: three methods taking the
// URIs and the caller's startup id, answering nothing.
#[zbus::interface(name = "org.freedesktop.FileManager1")]
impl FileManager {
    fn show_folders(&self, uris: Vec<String>, startup_id: String) {
        self.call(Show::Folders, uris, startup_id);
    }

    fn show_items(&self, uris: Vec<String>, startup_id: String) {
        self.call(Show::Items, uris, startup_id);
    }

    fn show_item_properties(&self, uris: Vec<String>, startup_id: String) {
        self.call(Show::ItemProperties, uris, startup_id);
    }
}

fn bus_error(error: &zbus::Error) -> Error {
    Error::Io(std::io::Error::other(format!("the session bus: {error}")))
}

/// Connects, serves the object and asks for the name; queued behind
/// another owner rather than refused.
pub(crate) async fn serve(address: Option<&str>, on_call: OnCall) -> Result<Connection, Error> {
    let builder = match address {
        Some(address) => zbus::connection::Builder::address(address),
        None => zbus::connection::Builder::session(),
    }
    .map_err(|e| bus_error(&e))?;
    let connection = builder
        .serve_at(OBJECT_PATH, FileManager { on_call })
        .map_err(|e| bus_error(&e))?
        .build()
        .await
        .map_err(|e| bus_error(&e))?;
    // No flag at all, which this type's default is not, so the empty set
    // is spelled as two flags that share nothing: Rick keeps the name
    // while it runs, and when another file manager owns it Rick neither
    // fails nor takes it away but waits in the queue and gets the name
    // when that one lets go.
    let no_flags = RequestNameFlags::ReplaceExisting & RequestNameFlags::DoNotQueue;
    debug_assert!(no_flags.is_empty());
    let reply = connection
        .request_name_with_flags(BUS_NAME, no_flags)
        .await
        .map_err(|e| bus_error(&e))?;
    tracing::debug!(?reply, "asked for org.freedesktop.FileManager1");
    Ok(connection)
}
