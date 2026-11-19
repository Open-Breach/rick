# app

`app` is the command loop: it receives `Command`s from the UI, mutates the
`model`, runs listings and watches as jobs, forwards job progress, and emits
`Event`s; the UI never touches the model directly. It opens the preview of
the followed explorer's selection through the best-scoring previewer, the
preview pane being Rick's only viewer. It owns the undo/redo log of
reversible commands, the extension `Registry` the actions come from, the
TOML settings file with its watcher (the `language` setting among them: it
registers the kernel's `locales/en.ftl` and `de.ftl` with the `t!` lookup,
reads the overrides in the configuration directory's `locales/`, and applies
a language change live), and the session: `session.toml`, written
after every change at most once per second through a temporary file and once
more on exit, restored at start (or the start layout the settings name, with
a vanished folder's tab opened at its nearest ancestor), and the saved
layouts in `layouts/`. Windows open, close and exchange workspace tabs and
panes through it;
closing the last one asks the UI to exit. A second launch's command line
reaches it through the external-work channel: `NewWindowAt` opens a
window of its own, `Reveal` opens the files' folders as tabs and selects
the files once listed (and, asked to, opens Properties on them), and
`Activate` asks the UI to raise the focused
window with the launcher's token (`Event::Activate`). A console a
tab opened follows that tab and the tab follows it: the shell's working
directory (OSC 7, through `Console::cwd`) moves the explorer tab, and the
explorer's navigation types `cd` into the console while the shell is at
its prompt (`Console::foreground_job`). A drag and drop's `Transfer`, `Receive`,
`SpringLoad`, `Pin` and `Unpin` are commands it handles itself, and so is
`Open`: a folder navigates, a local file is offered to the archive provider
and enters as a location when it browses it (unless the setting
`archives.on_open` says `system`), anything else opens with the system
default; nothing opens a viewer. The preview pane's *Copy* reads the whole
file as a job and puts its text on the clipboard; a preview is reported
with the previewer that opened it, and a file no previewer claims with the
disabled plugin whose manifest claims its type and the file's permission
line, for the pane's File info card (`Command::PreviewWith` shows one file
through a named previewer whatever the scores); *Always preview on <source>* writes
`preview_always` into the source's connection and takes the size limit off
there, whichever remote provider keeps the source; a whole file fetched
from a remote source for a preview is counted as it arrives
(`Event::PreviewProgress`), and `Command::PreviewCancel` stops the fetch
and brings the size-limit card back. For the Extensions dialogue it keeps the
plugins' rows with what their manifests say, which schemes a plugin's
providers serve, where its settings file is and whether its previewer
reported a third-party library missing; `Command::InstallPlugin` unpacks a
plugin package — an `.rkp` file, a zip archive of a plugin's folder — into
the per-user plugins folder and `Command::RemovePlugin` deletes a user's
plugin there (`plugins`), each as a job, checking and asking nothing and
never writing outside that folder. `Command::InstallAvailable` is
**Install** on a plugin of the registry: a job of the operations area that
downloads the package of the build that fits into the cache, holds it
against the size and the SHA-256 the plugin list names, and unpacks it the
same way; a package that is another one is deleted and the job fails with
*The download did not match the plugin list*. `App::offering` is the
*Available* view's list: why there is none, or the plugins of the list
that fit this Rick and are in neither plugins folder. Every unpack is
noted in `installed.toml` in the per-user plugins folder (`plugins::Record`:
per folder the origin — `registry`, `file` or `url` — the version and the
package's SHA-256); *Remove* drops the entry, and a start drops the
entries whose folder is gone. `App::updates` lists the plugins of the
per-user folder that the record marks as the registry's, that the plugin
list has this Rick's build of, and that the host refused — built for the
Rick before — or that the list has a higher version of; **Update** is an
install over the old folder and `Command::UpdateAll` one for each. The
start that first finds any for a release of Rick says so
(`Event::PluginUpdates`) and remembers the release in
`registry/updates-announced` of the cache. `Command::InstallPluginFrom` is
*Install from URL…*: the package at an `https://` address is downloaded
and handled exactly as a file, the record noting `url` and the address. A
Rick a package manager installed says so in the file `package` beside
its binary (`plugins::Packaging`, which the binary hands over): it shows
the command for an available plugin, `sudo apt install rick-plugin-git`,
downloads none and has no update of its own to offer. The extension settings store is shared with the plugins through the
registry, so a plugin that found its third-party library writes the path
into its own setting. The pinned list lives in its settings, and the pin actions
in `ops` see it through the `PinStore` bridge it hands them. It feeds the
side panel — the Places list from `places.toml`, seeded from the standard
folders on the first start and edited through `PlaceAdd`, `PlaceRemove`,
`PlaceMove` and `PlaceEdit`; the mounted volumes polled through the
platform, with `Eject` as a job; the cloud and network providers' rows, one
per connection; the connections themselves (`AddConnection`, `Connection`
with connect, disconnect and remove), whose non-secret fields are
`[[connections]]` tables of `rick.toml` and whose secrets go to the
platform keychain as a job before the table is written, the provider's
`Connector` told after every change; and the trash's
count after every operation — mounts the `rick://` provider of `vfs-rick`
over every other provider, hands it the views extensions registered (the
Trash view of `ext-vfs-trash`) and tells it when a view's contents changed,
polls a location its provider cannot watch (`stat` every three seconds and
after every operation), runs the subtree search as a visible job, lists the
children of folders expanded in Tree view, routes the name conflicts copy
and move jobs ask about to the UI and their answers back, and shows a job
queued behind another on its volume as *Waiting*. It runs the integration
check of specs/system-integration.md §6 (`IntegrationCommand::Check`, a job
off the loop's thread that reads what the platform finds of every item the
user has not declined and folds it into `rick.toml`'s `[integration]`
records and the model; the check at startup obeys *Check integration at
startup* and says whether the dialog should open), performs the dialog's
*Apply* and Settings' *Add* and *Restore* row by row through the platform,
recording what each wrote and reporting a row that failed without stopping
the others, and clears the declined marks on *Reset choices*. The install
location alone is not the platform's: the installer plugin performs it
through its registered command `install-rick`, as a job of its own, the
rows after it waiting for that job and then acting on the installed copy,
and the apply's end names the copy for the UI to relaunch from; without the
plugin the row is *Unsupported* and the others work against the folder
Rick runs from. The
*Shell integration* switch unties the console and the tab. It decides when
the plugin registry is asked (`registry`, specs/plugin-distribution.md §6):
`RegistryCommand::Start`, once the first frame is drawn, reads the cached
plugin list and fetches it when it is a day old, `Opened` fetches when the
*Available* view is opened, both only while `[registry] check` is on, and
`CheckNow` fetches whatever the switch says; each is a job off the loop's
thread through the `plugins-registry` crate, and `App::plugin_registry` is
what came of it: whether a key is there at all, the last index that
verified, when it was fetched, and why the last attempt gave none. A Rick
without a key or with a pre-release version asks nothing. The key compiled
into the build counts only once the binary has said so
(`App::use_build_registry`): an application made anywhere else, as every
test is, knows only a key its `rick.toml` names, and so never asks the
real registry. The cache is
`cache/` in the config directory unless the binary names the platform's
(`App::set_cache_dir`). It depends on `types`,
`runtime`, `platform`, `ext`, `vfs`, `vfs-local`, `vfs-rick`, `model`,
`ops` (the built-in operations it loads) and `plugins-registry`; only
`ui-egui`, `rick` and `tests` may depend on it.
