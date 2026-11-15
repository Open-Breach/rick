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
the Extensions dialogue is built from; `rick [<path>…]` opens each folder in a tab
of the most recently focused window (the working directory without any),
`rick reveal <file>…` opens the files' folders and selects the files,
`--new-window` puts them in a window of their own, `rick --activate` is
what the launcher entry runs — without a path it brings the window forward
and opens nothing, so a click on the launcher or on a dock favourite never
opens a tab; with paths it opens them — `rick <command>
<args…>` runs a plugin's subcommand, listed by `--help` with the core ones,
and a `rick:///<path>` URL (the scheme the desktop entry and `Info.plist`
register) opens an existing folder, reveals an existing file and is an
error toast otherwise.
One Rick runs per user session and config directory (`--config-dir <dir>`
picks another): a launch first hands its command line to the running
instance over the platform's single-instance endpoint, prints the reply
and exits with the instance's status within a second, and only binds the
endpoint and opens the window itself when nothing listens; the instance
runs every request on the UI thread between two frames, through the app's
external-work channel, and raises the window it changed through the
platform's activation. The command line and the instance are the crate's
library (`cli`, `instance`), so the integration tests drive them against
the built binary. When the integration dialog has installed Rick to the
standard location, the binary continues from that copy by the same rules
(`instance::relaunch`): it stops serving and frees the endpoint, starts the
installed binary on the same config directory, and exits once the new
process answers as the instance; should nothing answer within twenty
seconds it takes the endpoint back, stays and says so. On Linux the
instance also answers *"Show in folder"* from other applications: it
serves `org.freedesktop.FileManager1` on the session bus
(`platform::file_manager`) when that integration item is set up, and each
call runs like a second launch's command line (`instance::show`):
`ShowItems` is `reveal`, `ShowFolders` opens the folders, and
`ShowItemProperties` reveals and opens Properties. `rick --dbus-service`
is what the item's activation file runs when nobody owns the name: the
launch opens no folder of its own, and the instance, this one or the one
it hands over to, takes the name. It is the only crate that uses `anyhow`. Its second
binary, `rick-perms`, is the privileged helper: the one program Rick runs
as the administrator, through the platform's prompt, to apply a permission
change the user may not make by themselves (`platform::perms::helper`);
the bundle lays it beside `rick`. A third binary, `rick-zfs`, is the ZFS
plugin's privileged helper (`ext_zfs::helper`), which runs one checked
`zfs snapshot`, `rename` or `destroy` of a snapshot; the bundle lays it
beside `rick` where the ZFS plugin is bundled, so `ext-zfs` is always a
dependency and the `zfs` feature only compiles the plugin in. It depends on `types`, `runtime`,
`platform`, `vfs-local`, `model`, `app`, `ui-egui` and `plugins-native`;
nothing depends on it.
