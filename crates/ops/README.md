# ops

`ops` holds the file operations of version 0.1 as `Action`s registered through
`ext` from one built-in `Extension`: copy, cut and paste, duplicate, move to
trash, delete permanently, rename, new file, new folder, copy path, open with
the system default, open the containing folder, restore, restore trash items,
empty trash, pin to side panel and unpin. It also holds `transfer`, the
move-or-copy job a drag and drop runs; `Conflicts`, where a copy or move
that would land on an existing name asks — skip, overwrite, rename, apply to
all, cancel — and waits; and the `Scheduler`, which runs two jobs that share
a source or destination volume one after the other and jobs on disjoint
volumes in parallel, the queued one shown as *Waiting*. Every operation that
touches the filesystem runs as a `Job`; every reversible one (move, rename,
trash, restore, new file, new folder, pin, unpin) returns an `Undoable`
command for the undo log. In the virtual locations of `vfs-rick` the actions
that do not apply hide themselves; the trash actions show in the Trash view
only. The pinned list and the trash record belong to `app`, which hands the
actions a `PinStore` and a `TrashStore` view of them.
It depends on `types`, `runtime`, `platform`, `ext`, `vfs`, `vfs-local` and
`vfs-rick`;
`app` and `tests` may depend on it.
