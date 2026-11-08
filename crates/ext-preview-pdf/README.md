# ext-preview-pdf

`ext-preview-pdf` previews PDF files through pdfium: it binds the pdfium
library lazily, on the first PDF shown, from its `library` setting
(`preview-pdf.toml`) when set, then beside the binary and in the plugins
directory, then the platform's library directories and the system loader,
and writes the first hit back into the setting (SPEC.md, *Third-party
libraries*; no environment variable). It renders pages to the requested
width and extracts each page's text as it is shown. Without the library the
preview pane shows the file's facts, the download for this platform,
*Browse…* and *Retry*. It depends on `ext` and `types` only; `rick` and
`tests` load it.
