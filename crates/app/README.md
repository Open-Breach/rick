# app

`app` is the command loop: it receives `Command`s from the UI, mutates the
`model`, runs listings and watches as jobs, forwards job progress, and emits
`Event`s; the UI never touches the model directly. It opens the preview of
the followed explorer's selection and every viewer tab's file through the
best-scoring previewer, and decides what `Open` does with a file (a viewer
tab when a previewer claims it, the system application otherwise or when the
`[viewer] external` setting names its type) by reading its first bytes as a
job. It owns the undo/redo log of
reversible commands, the extension `Registry` the actions come from, the
TOML settings file with its watcher, and the session: `session.toml`, written
after every change at most once per second through a temporary file and once
more on exit, restored at start (or the start layout the settings name, with
a vanished folder's tab opened at its nearest ancestor), and the saved
layouts in `layouts/`. Windows open, close and exchange tabs through it;
closing the last one asks the UI to exit. A drag and drop's `Transfer`, `Receive`,
`SpringLoad`, `Pin` and `Unpin` are commands it handles itself; the pinned
list lives in its settings, and the pin actions in `ops` see it through the
`PinStore` bridge it hands them. It feeds the side panel — the Places list from `places.toml`, seeded from
the standard folders on the first start and edited through `PlaceAdd`,
`PlaceRemove`, `PlaceMove` and `PlaceEdit`; the mounted volumes polled
through the platform, with `Eject` as a job; the cloud providers' rows; and
the trash after every operation with Rick's own record (`trash.toml`) where
the platform cannot list it — mounts the `rick://` provider of `vfs-rick`
over every other provider and tells it when a view's contents changed, runs
the subtree search as a visible job, lists the children of folders expanded
in Tree view, routes the name conflicts copy and move jobs ask about to the
UI and their answers back, and shows a job queued behind another on its
volume as *Waiting*. It depends on `types`, `runtime`,
`platform`, `ext`, `vfs`, `vfs-local`, `vfs-rick`, `model` and `ops` (the
built-in operations it loads); only `ui-egui`, `rick` and
`tests` may depend on it.
