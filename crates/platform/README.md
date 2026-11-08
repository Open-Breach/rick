# platform

`platform` holds every line of per-OS code in Rick behind one trait set:
`Trash` (trash, restore, list, remove, empty; on macOS `list` is Rick's own
record of what it trashed, `trash.toml` in the config directory, since the
system keeps the original paths to itself), `OpenWith`, `Properties`,
`ConfigDir`, `NativeDialogs`, `Clipboard`, `Chrome`, `DragSource` (starting a
drag other applications receive, from the window `winit` describes through
its raw handles: an `NSDraggingSession`, OLE's `DoDragDrop` with a `CF_HDROP`,
a Wayland `wl_data_source`, an XDND source), `DropTarget` (where the cursor
of a drag from outside is, since the windowing backend reports its hovered
and dropped files without a position, and on Wayland, where `winit` 0.30
receives no drops at all, the files themselves through Rick's own
`wl_data_device`), `Volumes` (every mounted volume with its file system and space,
from `/proc/mounts` and sysfs, `NSFileManager.mountedVolumeURLs`, or
`GetLogicalDrives`, and ejecting removable media through `udisksctl`,
`diskutil` or `DeviceIoControl`), `StandardFolders` (XDG user-dirs, the known
folders, the fixed directories under the macOS home) and `RecentlyUsed` (the
XDG `recently-used.xbel`, Spotlight's last-used dates, the Recent folder's
shortcuts), implemented for Linux (XDG trash, `xdg-open`), macOS
(`NSFileManager`, `open`) and Windows (`IFileOperation`, `ShellExecute`), plus
a `Fake` for tests. It also owns the
filesystem fast paths other crates program against: the bulk directory reader
(`statx`, `getattrlistbulk`, `GetFileInformationByHandleEx`), the native copy
(`FICLONE`/`copy_file_range`, `clonefile`, `CopyFileExW`), allocated sizes and
extent sharing, the `VPath` ↔ `Path` conversion and `file://` URIs. It is the only crate
allowed `unsafe` and `cfg(target_os)`. It depends on `types` and `runtime`;
`vfs`, `vfs-local` and the crates above them may depend on it. It also uses
`raw-window-handle`, `wayland-client` and `x11rb` on Linux, `objc2-app-kit`
on macOS and the `windows` crate on Windows for the drag and drop halves the
backend leaves to it.
