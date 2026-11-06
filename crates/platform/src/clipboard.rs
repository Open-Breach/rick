//! The system clipboard through `arboard`: X11 and Wayland on Linux, the
//! pasteboard on macOS, the Win32 clipboard on Windows.

use types::Error;

use crate::Native;
use crate::traits::Clipboard;

impl Clipboard for Native {
    fn set_text(&self, text: &str) -> Result<(), Error> {
        let mut clipboard = arboard::Clipboard::new().map_err(convert)?;
        clipboard.set_text(text).map_err(convert)
    }
}

fn convert(error: arboard::Error) -> Error {
    match error {
        arboard::Error::ClipboardNotSupported => {
            Error::Unsupported("no clipboard is reachable from this session")
        }
        other => Error::Io(std::io::Error::other(other)),
    }
}
