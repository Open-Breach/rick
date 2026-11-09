# rick

`rick` is the binary: it sets up logging, loads the plugins found beside the
binary and in the config directory through `plugins-native` (minus the ids
`rick.toml` disables), builds the `app` with the native `platform`, the
local provider, the built-in extensions compiled in and the plugins, and
runs the `ui-egui` shell. The built-in extensions are Cargo features on this
crate, default-on: the text, code, image and PDF previewers; `git` and
`preview-hex` compile the Git and hex-preview plugins in instead of loading
them, default off. `rick --version`
prints the version and the git hash, `rick --list-plugins` the plugins table
the Extensions screen shows. It is the only crate that uses `anyhow`. Its second
binary, `rick-perms`, is the privileged helper: the one program Rick runs
as the administrator, through the platform's prompt, to apply a permission
change the user may not make by themselves (`platform::perms::helper`);
the bundle lays it beside `rick`. It depends on `types`, `platform`,
`vfs-local`, `app`, `ui-egui` and `plugins-native`; nothing depends on it.
