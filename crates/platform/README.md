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
drag other applications receive; drop reception comes through the windowing
backend), `Volumes` (every mounted volume with its file system and space,
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
shortcuts) and `Secrets` (a secret by a stable key in the platform keychain
through the `keyring` crate: Secret Service, Keychain, Credential Manager;
the fake keeps them in memory) and the `agent` module that opens the SSH
agent's socket (`SSH_AUTH_SOCK`, or OpenSSH's named pipe on Windows),
implemented for Linux (XDG trash, `xdg-open`), macOS
(`NSFileManager`, `open`) and Windows (`IFileOperation`, `ShellExecute`), plus
a `Fake` for tests. It also owns the
filesystem fast paths other crates program against: the bulk directory reader
(`statx`, `getattrlistbulk`, `GetFileInformationByHandleEx`), the native copy
(`FICLONE`/`copy_file_range`, `clonefile`, `CopyFileExW`), allocated sizes and
extent sharing, the `VPath` ↔ `Path` conversion and `file://` URIs. It is the only crate
allowed `unsafe` and `cfg(target_os)`. It depends on `types` and `runtime`;
`vfs`, `vfs-local` and the crates above them may depend on it.
