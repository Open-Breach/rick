# rick

`rick` is the binary: it sets up logging, builds the `app` with the native
`platform`, the local provider and the built-in extensions compiled in, and
runs the `ui-egui` shell. Optional extensions are Cargo features on this
crate, default-on for built-ins; none exist yet. It is the only crate that
uses `anyhow`. It depends on `types`, `platform`, `vfs-local`, `app` and
`ui-egui`; nothing depends on it.
