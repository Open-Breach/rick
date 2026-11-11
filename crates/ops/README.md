# ops

`ops` holds the file operations of version 0.1 as `Action`s registered through
`ext` from one built-in `Extension`: copy, cut and paste, duplicate, move to
trash, delete permanently, rename, new file, new folder, copy path, open with
the system default, open the containing folder, pin to side panel and unpin,
and Mount, a disk image handed to the operating system's own mount
mechanism through `platform::Mount`, shown only on the formats this OS
mounts (the mounted image is a removable volume whose Eject unmounts it);
each follows the provider's capabilities at the pane's location, so Paste,
Rename and the rest hide where nothing can be written and Move to trash
where there is no trash, and in the virtual locations of `vfs-rick` the
actions that do not apply hide themselves. New folder, New file and
Download… follow `CREATE_DIR`, `CREATE_FILE` and `DOWNLOAD`: where a
location that takes writes, or a remote one, lacks the capability the
action is disabled with the provider's reason, which the context menu shows
greyed with the reason as its tooltip, and a new folder's name is put to
the provider's `name_problem` before any request goes out. Open on a remote
file hands nothing to the system: it copies the file into the Downloads
folder as a job, and Download… (`download`) does the same for the whole
selection, folders included, into a folder asked for first. Open in
editor (F4) starts the command of the `editor` setting with the selected
local files appended, split at spaces outside double quotes, or the
system's default text editor where the setting is empty; the application
hands the setting over as `OpenSettings`. Compute hash… (`hash`) reads
every selected file once through its provider in 1 MiB chunks and feeds
each chunk to MD5, SHA-1, SHA-256 and BLAKE3 in one job with byte progress
and cancel, filling an `ext::Hashes` slot the Hash dialogue polls; the
module's pure functions render a digest in the remembered format (`[hash]
format` in `core.toml`), read a pasted value back and tell the algorithms
of its length apart for the Verify field. Restore, Restore trash items and
Empty trash belong to the trash provider plugin, `ext-vfs-trash`. It also
holds `transfer`, the move-or-copy job a drag and drop runs, which hands a
selection whose source and target share one provider instance with
`NATIVE_TRANSFER` to the provider's own `transfer` (a server-side copy, a
remote rename) and streams only what the provider cannot do itself;
`perms`, the permission jobs the Properties dialogue runs (owner, group,
mode bits and ACL on the selection and, when asked, on everything inside a
folder, every entry's previous state recorded so the job is one undo step,
the entries the user may not change handed to the privileged helper in one
prompt); `Conflicts`,
where a copy or move that would land on an existing name asks — skip,
replace (merge between two folders), keep both under the first free
numbered name, the same for the remaining, cancel — and waits, the job
having counted its conflicts before it brought anything; and the `Scheduler`,
which runs two jobs that share a source or destination volume one after the
other and jobs on disjoint volumes in parallel, the queued one shown as
*Waiting*. Every operation that touches the filesystem runs as a `Job`;
every reversible one (move, rename, trash, new file, new folder, pin,
unpin) returns an `Undoable` command for the undo log. The pinned list
belongs to `app`, which hands the actions a `PinStore` view of it.
It depends on `types`, `runtime`, `platform`, `ext`, `vfs`, `vfs-local` and
`vfs-rick`;
`app` and `tests` may depend on it.
