# ext-preview-hex

`ext-preview-hex` is the fallback previewer: it claims every file at the
lowest score, so it draws only what no other previewer claims, and shows the
file's bytes as a hex dump with an offset column and an ASCII column, sixteen
bytes a row. It reads a bounded window from the start of the file and says
so in its status bar, renders the dump as text the pane lays out line by
line (so only the visible rows cost anything), and exposes the dump as the
text layer for find. It ships as a plugin: the crate builds as a `cdylib`
that `plugins-native` loads from `plugins/preview-hex/` through the entry
point `ext::plugin!` declares, and as an `rlib` for the binary's opt-in
`preview-hex` feature and the tests. It depends on `ext` and `types` only;
`rick` (behind `preview-hex`) and `tests` link it, and the plugin host loads
it.
