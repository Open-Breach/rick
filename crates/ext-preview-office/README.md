# ext-preview-office

`ext-preview-office` previews DOCX, XLSX, PPTX, ODT, ODS and ODP files. It
reads the metadata from the file's zip (title, author, dates, page, sheet
or slide count), looks for LibreOffice lazily on the first file shown (its
`soffice` setting in `preview-office.toml`, then beside the binary and the
plugins directory, the `PATH` and the install directories of the three
platforms, the first hit written back into the setting; no environment
variable) and, when it is there,
converts the file to PDF with `soffice --headless` inside the preview job,
cancellable and with a timeout, into a cache under the temporary directory
keyed by path, size and modification time (QUESTIONS.md 104); the pages then
render through pdfium like the PDF previewer's, found through the PDF
previewer's `library` setting and the same places. Without LibreOffice or
pdfium the preview pane shows the metadata, the install command for this
platform, *Browse…* and *Retry*; LibreOffice is never bundled. It is a plugin (`cdylib`, loaded from `plugins/preview-office/`)
and an `rlib` for the binary's opt-in `preview-office` feature. It depends
on `ext` and `types` only; `rick` (behind the feature) and `tests` link it,
and the plugin host loads it.
