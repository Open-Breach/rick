# ext-preview-office

`ext-preview-office` previews DOCX, XLSX, PPTX, ODT, ODS and ODP files. It
reads the metadata from the file's zip (title, author, dates, page, sheet
or slide count), looks for LibreOffice (`RICK_SOFFICE`, the `PATH`, then
the install directories of the three platforms) and, when it is there,
converts the file to PDF with `soffice --headless` inside the preview job,
cancellable and with a timeout, into a cache under the temporary directory
keyed by path, size and modification time (QUESTIONS.md 95); the pages then
render through pdfium like the PDF previewer's. Without LibreOffice the
preview shows the metadata and a hint to install it; LibreOffice is never
bundled. It is a plugin (`cdylib`, loaded from `plugins/preview-office/`)
and an `rlib` for the binary's opt-in `preview-office` feature. It depends
on `ext` and `types` only; `rick` (behind the feature) and `tests` link it,
and the plugin host loads it.
