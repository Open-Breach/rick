//! The previewer's test binary: proves pdfium loads and renders.
//!
//! It generates a three-page PDF, opens it through [`ext_preview_pdf::PdfDoc`]
//! and renders the first page, then prints the page count and the frame
//! size. pdfium is found the way the previewer finds it without a setting:
//! beside this executable, the plugins directory, the platform's library
//! directories, then the system loader. The release smoke test copies this
//! binary next to the unpacked `rick` and runs it, which shows the library
//! shipped in the archive is the one that loads.
//!
//! ```text
//! cargo run -p ext-preview-pdf --example pdfium_check
//! ```

use std::io::Write as _;
use std::process::ExitCode;

use ext::{Frame, PreviewDoc, RenderRequest};
use ext_preview_pdf::{Library, PdfDoc, sample_pdf};

fn run() -> Result<String, String> {
    let pdfium = Library::default()
        .bind()
        .map_err(|missing| format!("bind: {}", missing.detail))?;
    let mut doc = PdfDoc::new(pdfium, sample_pdf(3)).map_err(|e| format!("open: {e}"))?;
    let pages = doc.pages().ok_or("the PDF reports no pages")?;
    let frame = doc
        .render(
            0,
            RenderRequest {
                width: 400,
                height: 600,
                scale: 1.0,
                dark: false,
            },
        )
        .map_err(|e| format!("render: {e}"))?;
    match frame {
        Frame::Rgba {
            width,
            height,
            data,
        } if usize::try_from(width * height * 4).ok() == Some(data.len()) => Ok(format!(
            "pdfium ok: {pages} pages, first page rendered at {width}x{height}"
        )),
        other => Err(format!("unexpected frame: {other:?}")),
    }
}

fn main() -> ExitCode {
    // A test binary reports on stdout and stderr, like the tests do.
    match run() {
        Ok(report) => {
            let _ = writeln!(std::io::stdout().lock(), "{report}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            let _ = writeln!(std::io::stderr().lock(), "pdfium check failed: {error}");
            ExitCode::FAILURE
        }
    }
}
