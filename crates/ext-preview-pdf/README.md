# ext-preview-pdf

`ext-preview-pdf` previews PDF files through pdfium: it binds the pdfium
library from `RICK_PDFIUM_DIR`, the executable's directory or the system,
renders pages to the requested width and extracts each page's text as it is
shown. Without the library it says so instead of previewing. It depends on
`ext` and `types` only; `rick` and `tests` load it.
