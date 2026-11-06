# ops

`ops` holds the file operations of version 0.1 as `Action`s registered through
`ext` from one built-in `Extension`: copy, cut and paste, duplicate, move to
trash, delete permanently, rename, new file, new folder, copy path, open with
the system default, open the containing folder, restore, restore trash items,
empty trash, pin to side panel and unpin. It also holds `transfer`, the
move-or-copy job a drag and drop runs, and `Conflicts`, where a copy or move
that would land on an existing name asks — skip, overwrite, rename, apply to
all, cancel — and waits. Every operation that touches the filesystem runs as
a `Job`; every reversible one (move, rename, trash, restore, new file, new
folder, pin, unpin) returns an `Undoable` command for the undo log. The
pinned list and the trash record belong to `app`, which hands the actions a
`PinStore` and a `TrashStore` view of them.
It depends on `types`, `runtime`, `platform`, `ext`, `vfs` and `vfs-local`;
`app` and `tests` may depend on it.
