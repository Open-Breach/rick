# platform

`platform` holds every line of per-OS code in Rick behind one trait set:
`Trash`, `OpenWith`, `Properties`, `ConfigDir` and `NativeDialogs`, implemented
for Linux (XDG trash, `xdg-open`), macOS (`NSFileManager`, `open`) and Windows
(`IFileOperation`, `ShellExecute`), plus a `Fake` for tests. It also owns the
filesystem fast paths other crates program against: the bulk directory reader
(`statx`, `getattrlistbulk`, `GetFileInformationByHandleEx`), the native copy
(`FICLONE`/`copy_file_range`, `clonefile`, `CopyFileExW`), allocated sizes and
extent sharing, and the `VPath` ↔ `Path` conversion. It is the only crate
allowed `unsafe` and `cfg(target_os)`. It depends on `types` and `runtime`;
`vfs`, `vfs-local` and the crates above them may depend on it.
