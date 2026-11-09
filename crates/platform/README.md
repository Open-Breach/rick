# platform

`platform` holds every line of per-OS code in Rick behind one trait set
(`Permissions`: an entry's owner, group, mode bits and ACL — POSIX and
NFSv4 through extended attributes, macOS's own through `acl_to_text`,
NTFS through the security API — read and changed, with the users and
groups to choose from; `Elevate`: one command run as the administrator
through `pkexec`, Authorization Services or User Account Control, which
`perms::helper` uses for `rick-perms`, the helper that applies a permission
change the user may not make by themselves and reports what the entries
had):
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
`diskutil` or `DeviceIoControl`), `Mount` (which disk-image formats this
OS mounts natively and mounting one through the OS and nothing else:
`udisksctl loop-setup` for ISO and raw images on Linux, `hdiutil attach`
for DMG, ISO and raw images on macOS, `Mount-DiskImage` for ISO, VHD and
VHDX on Windows; the mounted image is a removable volume whose Eject
unmounts it), `StandardFolders` (XDG user-dirs, the known
folders, the fixed directories under the macOS home), `RecentlyUsed` (the
XDG `recently-used.xbel`, Spotlight's last-used dates, the Recent folder's
shortcuts), `Secrets` (a secret by a stable key in the platform keychain
through the `keyring` crate: Secret Service, Keychain, Credential Manager;
the fake keeps them in memory) and `SystemFonts` (the files of the desktop's
UI and monospace fonts: fontconfig's `sans-serif` and `monospace` matches
through `fc-match`, the San Francisco files, Segoe UI and Consolas; the
fake answers like the machine it runs on), and `Motion` (the system's
reduce-motion setting, read once from the desktop, which stills Rick's
loading animations) and the `agent` module that opens the SSH
agent's socket (`SSH_AUTH_SOCK`, or OpenSSH's named pipe on Windows),
implemented for Linux (XDG trash, `xdg-open`), macOS
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
