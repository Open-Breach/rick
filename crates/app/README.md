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
the files once listed, and `Activate` asks the UI to raise the focused
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
file as a job and puts its text on the clipboard; a file no previewer
claims is reported with the disabled plugin whose manifest claims its type,
for the pane's placeholder; *Always preview on <source>* writes
`preview_always` into the source's connection and takes the size cap off
there. The extension settings store is shared with the plugins through the
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
the others, and clears the declined marks on *Reset choices*; the
*Shell integration* switch unties the console and the tab. It depends on `types`,
`runtime`, `platform`, `ext`, `vfs`, `vfs-local`, `vfs-rick`, `model` and
`ops` (the built-in operations it loads); only `ui-egui`, `rick` and
`tests` may depend on it.
