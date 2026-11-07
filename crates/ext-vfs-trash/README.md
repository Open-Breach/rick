# ext-vfs-trash

`ext-vfs-trash` makes the platform trash a browsable location: it registers
the view `trash` of the `rick` provider, so `rick://trash` lists what
`platform::Trash::list` gives (Rick's own record where the OS keeps the
original paths to itself) as entries named after the trashed items, with
their size and deletion date, a trashed folder browsable below it and
`open_read` for the preview; `remove(Permanent)` deletes a trashed item for
good, everything else is `Unsupported` and the capabilities say so. A
Decorator adds the *Original location* column to the view, and the "Trash"
action group holds Restore, Restore trash items and Empty trash, available
in the view only; Restore is undoable as a trash of the restored entries.
Nothing watches the trash: `watch` is `Unsupported` and the kernel polls.
It is a plugin (`plugins/vfs-trash/`) that depends on `ext` and `types`
only; `rick` (behind `vfs-trash`) and `tests` link it, and the plugin host
loads it.
