# ext-preview-csv

`ext-preview-csv` previews CSV and TSV files as a grid: it reads a bounded
window of the file, detects the encoding, sniffs the delimiter (comma,
semicolon, tab or pipe) from the first lines, takes the first row as the
header unless it is all numbers, indexes where every row starts and hands
the pane an `ext::Grid` that parses only the rows asked for, so a
million-row file costs the rows on screen. The status bar shows rows ×
columns, the delimiter and the encoding; `render` gives the raw text as the
fallback for a pane that shows no grids. It is a plugin (`cdylib`, loaded
from `plugins/preview-csv/`) and an `rlib` for the binary's opt-in
`preview-csv` feature. It depends on `ext` and `types` only; `rick` (behind
the feature) and `tests` link it, and the plugin host loads it.
