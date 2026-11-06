# ops

`ops` holds the file operations of version 0.1 as `Action`s registered through
`ext` from one built-in `Extension`: copy, cut and paste, duplicate, move to
trash, delete permanently, rename, new file, new folder, copy path, open with
the system default and open the containing folder. Every operation that
touches the filesystem runs as a `Job`; every reversible one (move, rename,
trash, new file, new folder) returns an `Undoable` command for the undo log.
It depends on `types`, `runtime`, `platform`, `ext`, `vfs` and `vfs-local`;
`app` and `tests` may depend on it.
