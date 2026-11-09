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
the Extensions screen shows; `rick [<path>…]` opens each folder in a tab
of the most recently focused window (the working directory without any),
`rick reveal <file>…` opens the files' folders and selects the files,
`--new-window` puts them in a window of their own, and `rick <command>
<args…>` runs a plugin's subcommand, listed by `--help` with the core ones.
One Rick runs per user session and config directory (`--config-dir <dir>`
picks another): a launch first hands its command line to the running
instance over the platform's single-instance endpoint, prints the reply
and exits with the instance's status within a second, and only binds the
endpoint and opens the window itself when nothing listens; the instance
runs every request on the UI thread between two frames, through the app's
external-work channel, and raises the window it changed through the
platform's activation. The command line and the instance are the crate's
library (`cli`, `instance`), so the integration tests drive them against
the built binary. It is the only crate that uses `anyhow`. Its second
binary, `rick-perms`, is the privileged helper: the one program Rick runs
as the administrator, through the platform's prompt, to apply a permission
change the user may not make by themselves (`platform::perms::helper`);
the bundle lays it beside `rick`. It depends on `types`, `runtime`,
`platform`, `vfs-local`, `app`, `ui-egui` and `plugins-native`; nothing
depends on it.
