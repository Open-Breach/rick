# ext-preview-hex

`ext-preview-hex` is the hex view: it shows a file's bytes as a hex dump
with an offset column and an ASCII column, sixteen bytes a row. It is the
fallback: it gives every file `ext::FALLBACK_SCORE`, a low score that is not
zero, so it takes whatever no other previewer recognises, while a file with a
recognised type still goes to its own previewer; it draws no thumbnails, and
it can be chosen per type in settings like any other previewer. It reads a bounded window from the start of the file and says
so in its status bar, renders the dump as text the pane lays out line by
line (so only the visible rows cost anything), and exposes the dump as the
text layer for find. It ships as a plugin: the crate builds as a `cdylib`
that `plugins-native` loads from `plugins/preview-hex/` through the entry
point `ext::plugin!` declares, and as an `rlib` for the binary's opt-in
`preview-hex` feature and the tests. It depends on `ext` and `types` only;
`rick` (behind `preview-hex`) and `tests` link it, and the plugin host loads
it.
