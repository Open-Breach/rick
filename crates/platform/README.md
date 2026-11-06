# platform

`platform` holds every line of per-OS code in Rick behind one trait set:
`Trash`, `OpenWith`, `Properties`, `ConfigDir`, `NativeDialogs`, `Clipboard`,
`Chrome` and `DragSource` (starting a drag other applications receive; drop
reception comes through the windowing backend), implemented
for Linux (XDG trash, `xdg-open`), macOS (`NSFileManager`, `open`) and Windows
(`IFileOperation`, `ShellExecute`), plus a `Fake` for tests. It also owns the
filesystem fast paths other crates program against: the bulk directory reader
(`statx`, `getattrlistbulk`, `GetFileInformationByHandleEx`), the native copy
(`FICLONE`/`copy_file_range`, `clonefile`, `CopyFileExW`), allocated sizes and
extent sharing, the `VPath` ↔ `Path` conversion and `file://` URIs. It is the only crate
allowed `unsafe` and `cfg(target_os)`. It depends on `types` and `runtime`;
`vfs`, `vfs-local` and the crates above them may depend on it.
