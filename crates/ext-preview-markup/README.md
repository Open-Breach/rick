# ext-preview-markup

`ext-preview-markup` is one plugin with three previewers for the structured
text formats, each rendered by default with the *Rendered* toggle to the
source: Markdown (through `pulldown-cmark`) and HTML (a basic reader of
headings, paragraphs, lists, links, images and tables, no browser engine,
no JavaScript, no CSS fidelity) become an `ext::Document` the pane lays
out, with images read from the file's own folder or below; CSV and TSV
become an `ext::Grid` over an indexed window of the file, the delimiter
sniffed from the first lines and the first row taken as the header unless
it is all numbers, so a million-row file costs the rows on screen. The
source behind the toggle is the code previewer's highlighted text for
Markdown and HTML and the raw text for CSV; it is also what `render` gives
a pane that lays out no documents, and the text layer the pane's *Copy*
control reads. The previewers score above `ext-preview-code` for their
extensions, so disabling the plugin leaves the highlighted source. It
supersedes `ext-preview-csv` and the rendering that sat in
`ext-preview-code`. It is a plugin (`cdylib`, loaded from
`plugins/preview-markup/`) and an `rlib` for the binary's opt-in
`preview-markup` feature. It depends on `ext`, `types` and
`ext-preview-code` (for the source view); `rick` (behind the feature) and
`tests` link it, and the plugin host loads it.
