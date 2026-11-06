# app

`app` is the command loop: it receives `Command`s from the UI, mutates the
`model`, runs listings and watches as jobs, forwards job progress, and emits
`Event`s; the UI never touches the model directly. It owns the undo/redo log of
reversible commands, the extension `Registry` the actions come from, and the
TOML settings file with its watcher. A drag and drop's `Transfer`, `Receive`,
`SpringLoad`, `Pin` and `Unpin` are commands it handles itself; the pinned
list lives in its settings, and the pin actions in `ops` see it through the
`PinStore` bridge it hands them. It feeds the side panel — the Recent list
from every change watch plus a bounded scan at start (`recent.toml`), the
mounted volumes polled through the platform, the trash after every operation
with Rick's own record (`trash.toml`) where the platform cannot list it, and
the folder tree following the focused pane — runs the subtree search as a
visible job, lists the children of folders expanded in Tree view, and routes
the name conflicts copy and move jobs ask about to the UI and their answers
back. It depends on `types`, `runtime`,
`platform`, `ext`, `vfs`, `vfs-local`, `model` and `ops` (the built-in
operations it loads); only `ui-egui`, `rick` and
`tests` may depend on it.
