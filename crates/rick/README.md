# rick

`rick` is the binary: it sets up logging, builds the `app` with the native
`platform`, the local provider and the built-in extensions compiled in, and
runs the `ui-egui` shell. The built-in
extensions are Cargo features on this crate, default-on: the text, code,
image and PDF previewers. It is the only crate that
uses `anyhow`. It depends on `types`, `platform`, `vfs-local`, `app` and
`ui-egui`; nothing depends on it.
