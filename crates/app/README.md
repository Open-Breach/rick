# app

`app` is the command loop: it receives `Command`s from the UI, mutates the
`model`, runs listings and watches as jobs, forwards job progress, and emits
`Event`s; the UI never touches the model directly. It owns the undo/redo log of
reversible commands, the extension `Registry` the actions come from, and the
TOML settings file with its watcher. A drag and drop's `Transfer`, `Receive`,
`SpringLoad`, `Pin` and `Unpin` are commands it handles itself; the pinned
list lives in its settings, and the pin actions in `ops` see it through the
`PinStore` bridge it hands them. It depends on `types`, `runtime`,
`platform`, `ext`, `vfs`, `vfs-local`, `model` and `ops` (the built-in
operations it loads); only `ui-egui`, `rick` and
`tests` may depend on it.
