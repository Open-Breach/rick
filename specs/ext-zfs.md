# Rick — ZFS Plugin (ext-zfs)

## Purpose

People who keep their home or their data on OpenZFS take snapshots, by hand or through a timer, and then rarely look at them, because getting a file back means a shell, the hidden `.zfs/snapshot` folder and the exact name of the snapshot. `ext-zfs` lets Rick show those snapshots as folders, list the versions of one file across them, put a file back where it was, create, rename and destroy snapshots, and change the few dataset properties that are safe to change on a running system, all without leaving the file manager.

This file is the binding specification for the plugin; where it disagrees with `SPEC.md`, the question goes into the pull request. It was placed in the review of #156: its own plugin, not part of `ext-deco-fs`.

## Decisions

- One plugin crate, `ext-zfs`, a feature plugin like `ext-git` and `ext-ssh`: one read-only `Provider` (id `zfs`), one `Decorator` for the Info panel section and the time slider's timeline, and the "ZFS" Action group. Loadable plugin file; the binary ships without it and nothing breaks when it is removed.
- Linux only (OpenZFS). The manifest declares Linux as its only platform; elsewhere the Extensions screen lists the plugin as not supported and it is not loaded.
- Everything goes through the `zfs` command line (`zfs list -H -p`, `zfs get -H -p`, `zfs snapshot`, `zfs rename`, `zfs destroy`, `zfs set`, `zfs inherit`) in the C locale. No libzfs, no C dependency in the plugin file.
- A snapshot's files are read from the dataset's `.zfs/snapshot/<name>/` directory with the platform's file system calls, so a snapshot opens, previews and copies out like any folder.
- Without `zfs`, or in a folder on another file system, the plugin shows nothing: no action, no column, no Info section, no error.
- Changes run with the user's own rights first (`zfs allow` delegation); only a change ZFS refuses for lack of permission is offered once more as administrator, through the privileged helper `rick-zfs`. Rick itself never runs as root.

## Scope

**In:** finding the dataset a folder lies in; the `zfs` location (datasets, snapshots, a snapshot's files, a file's versions); *Browse snapshots*, *Back to live folder*, *Versions*, *Restore*; *Create*, *Rename* and *Destroy snapshot*; *Edit ZFS properties…* for seven properties; the privileged helper and its polkit action; the Info panel section with the dataset's properties; the dataset's snapshots as the time slider's timeline.

**Out (this version):** macOS and FreeBSD ZFS; rollback; clones, holds and bookmarks as actions (a held snapshot is reported, not released); `zfs send` and `receive`; properties beyond the editor's seven, user properties, recursive property changes and undo of a property change; pool status and scrubs.

## Datasets

A local location is inside a dataset when it lies at or below the mount point of a mounted ZFS file system; with nested datasets the longest mount point wins, compared component by component. File systems with `mountpoint=none` or `legacy`, and unmounted ones, are left out. The list of file systems is read once per start; a dataset's snapshots and properties are cached for 30 seconds and dropped after every change Rick makes to that dataset.

## The zfs location

Paths of the `zfs` provider, where `<dataset>` is the dataset's full name and a snapshot is its own path component, so *Up* and the breadcrumb stay inside the dataset:

| Path | Lists |
|---|---|
| `/` | the mounted datasets that have snapshots |
| `/<dataset>` | the dataset's snapshots as folders, newest first; date is the creation time, size the space the snapshot holds |
| `/<dataset>/@<snapshot>/<path>` | the files and folders of `<path>` as they were in that snapshot |
| `/~versions/<live path>` | the versions of one file, see *Versions* |

Everything below `/` is read-only. The provider declares no write capability and answers every write, rename, trash, new file and new folder with "snapshots are read-only"; *Delete permanently*, which the core offers wherever there is a selection, fails the same way and changes nothing. Watching is not supported; the 30-second cache keeps the lists current.

## Actions

All in the "ZFS" group of the context menu and the palette, shown only where they apply.

- **Browse snapshots** — on a live folder in a dataset: the same folder inside the newest snapshot that has it. Disabled, with the reason, when no snapshot has the folder.
- **Back to live folder** — inside a snapshot: the corresponding live folder, or its nearest existing parent.
- **Versions** — on one live file, or one file inside a snapshot: navigates the pane to `/~versions/<live path>`, which lists, newest first, each snapshot whose copy of the file differs from the next newer one listed (size or modification time), named `<snapshot> — <file name>`. A file deleted live still has its versions. Each version previews and copies like a file in a snapshot.
- **Restore** — on entries inside a snapshot (below its root) or in a versions view: one confirmation that names the snapshot and the live paths the items go to, and, apart, the live items that will be replaced. Then one job: each live item in the way goes to the trash first, missing parent folders are created, and the snapshot's item is copied back with its permissions and modification times, folders and symbolic links included. An item whose live version cannot be trashed is skipped and reported. Hidden when the host lent no platform (no trash).
- **Create snapshot** — in a live folder of a dataset or in its snapshot list: a form with the name, prefilled `rick-YYYY-MM-DD-HHMMSS` in local time, and *Include child datasets* (`-r`). A name ZFS would refuse (empty, a character other than letters, digits and `_ - . : space`, longer than 255 with the dataset) is marked in the form, which then cannot be confirmed; a name that exists fails the job with the existing snapshot's name.
- **Rename snapshot** — on one snapshot in the snapshot list, with the same name check.
- **Destroy snapshot** — on snapshots in the snapshot list: one confirmation that names each snapshot (`dataset@name`) and says it cannot be undone, then one `zfs destroy` per snapshot. A snapshot ZFS refuses to destroy (held, dependent clones) is reported with ZFS's reason and stays.

- **Edit ZFS properties…** — on a live folder in a dataset, or on one dataset folder selected at the root of the `zfs` location; see *Property editor*.

Create, Rename, Destroy and the property editor run as named jobs and refresh the dataset's cached snapshot list and properties when they end.

## Property editor

The editor reads the dataset's values fresh in a job (not from the 30-second cache), then opens a form with one field per property, in this order:

| Property | Field | Values offered |
|---|---|---|
| `compression` | choice | *Inherit*, off, on, lz4, zstd, gzip, zle, and the current value when it is another (`zstd-3`) |
| `atime`, `relatime` | choice | *Inherit*, on, off |
| `copies` | choice | *Inherit*, 1, 2, 3 |
| `recordsize` | choice | *Inherit*, 4K to 1M in powers of two, and the current value when it is another |
| `quota`, `refquota` | text | `none`, a size, or empty for *Inherit* |

*Inherit* is labelled with the value it would give: the parent dataset's, or *default* for a pool's root dataset; the quota fields say it in their placeholder. A choice opens on the current value when it is set locally, else on *Inherit*; a quota opens with its local value, else empty. A size is a whole or decimal number with an optional unit K, M, G, T or P, powers of 1024, any case, optionally followed by `B` or `iB`.

The live preview has one row per changed property, `name: old (source)` → new, and the form cannot be confirmed while a row has a problem: a size that does not parse, a `quota` below `used` or a `refquota` below `referenced`, or no change at all. When `compression`, `copies` or `recordsize` changes, a row says that new values apply to data written from now on.

Applying is one job: one `zfs set p=v … <dataset>` for every explicit value, then one `zfs inherit <p> <dataset>` per property reset, stopping at the first failure; what ran before stays applied, and the message names the property and ZFS's reason. The permission rule is that of the snapshot commands, with the refused command and every one after it handed to the helper; without elevation, the message names the changed properties as the `zfs allow` permission set. No undo.

## Permissions and the privileged helper

1. Every change runs first as the user. With `zfs allow` rights (`snapshot`; `rename,mount,create`; `destroy,mount`) it simply works.
2. When ZFS refuses for lack of permission, Rick asks once whether to run the refused commands as administrator. On yes, each runs through the platform's elevation prompt (`platform::Elevate`, `pkexec` on Linux) as one call of `rick-zfs`; a dismissed prompt or a second refusal reports "Permission denied" and changes nothing more.
3. Without `pkexec`, or without the helper, no elevation is offered: the message names the `zfs allow` command that would give the user the right.

**The helper** `rick-zfs` is a small binary beside `rick` (`RICK_ZFS_HELPER` names it for packagers who keep it elsewhere), built from `ext_zfs::helper`. It takes the `zfs` arguments of exactly one change and accepts only:

- `snapshot [-r] <dataset>@<name>`
- `rename <dataset>@<old> <dataset>@<new>`, both of one dataset
- `destroy <dataset>@<name>`
- `set <property>=<value>… <dataset>` and `inherit <property> <dataset>`, for the editor's seven properties only, each value checked as the editor checks it

The snapshot commands need a snapshot's full name: a dataset name without `@` is refused, so the helper can never destroy or rename a dataset; the property commands need a dataset's name without `@`. No argument may start with `-` beyond the one `-r`, so no other option reaches `zfs`. It runs the system's `zfs` from `/usr/sbin`, `/sbin`, `/usr/bin` or `/bin`, never from the caller's `PATH`, with a cleared environment and the C locale, passes ZFS's output through and returns its exit code; refused arguments exit with 64, a missing `zfs` with 69.

**The polkit action** is `org.openbreach.rick.zfs` (`packaging/linux/org.openbreach.rick.zfs.policy`, in the Linux archive under `share/polkit-1/actions/`), administrator authentication every time. `pkexec` applies it when the policy is installed in `/usr/share/polkit-1/actions` and names the helper's path, which is where a system-wide install puts it (`/opt/rick/current/rick-zfs`); in a per-user install `pkexec` asks under its generic action instead.

## Info panel section

A *ZFS* section in the info badges of a local entry inside a dataset: the dataset's name; then, once read (*Loading…* until then), the number of snapshots, the newest snapshot's name and date, and the properties `guid`, `type`, `mountpoint`, `used`, `available`, `referenced`, `usedbysnapshots`, `quota`, `refquota`, `reservation`, `compression`, `compressratio`, `recordsize`, `dedup`, `copies`, `atime`, `encryption`, `keystatus` and `readonly`, in that order. Sizes are in human units, a quota or reservation of 0 reads *none*, `compressratio` carries an `x`. Each label carries the value's source: local, inherited from a named dataset, default or received. A property the installed ZFS does not report, or reports as `-`, is left out. Values copy as plain text.

## Timeline

The decorator offers the kernel's time slider a timeline for a live folder inside a dataset and for a folder inside one of its snapshots: one point per snapshot of that dataset, oldest first in the order ZFS took them, labelled with the snapshot's name, source *ZFS*, at its creation time, each pointing at the same relative folder inside that snapshot (`zfs:/<dataset>/@<snapshot>/<path>`), with the live folder as *Now*. It answers from the snapshot list cached per dataset, so the folders of one dataset read `zfs list` at most once in 30 seconds. A dataset without snapshots offers none. The kernel asks only while the pane's bar is shown, in a job.

## Configuration

None. The plugin has no settings.

## Dependencies

`chrono` for snapshot dates and the default name, `futures` and `tokio` (`fs`, `sync`) for the streamed listings, `thiserror`, `tracing`; at run time the `zfs` command line and, for elevation, `pkexec`. No C dependencies in the plugin file.

## Acceptance

Unit tests run against a scripted `zfs` runner and a temporary directory laid out as `.zfs/snapshot/<name>/`; the integration tests (`crates/tests/tests/zfs.rs`) run the plugin inside the application; the real-pool test runs by hand with `RICK_REQUIRE_ZFS=1` and `RICK_ZFS_TEST_DATASET` on a dataset with `zfs allow` delegation, and is skipped elsewhere.

1. Without `zfs`, and in a folder on another file system, no ZFS action, column or Info section appears and nothing is reported.
2. Nested datasets: a folder below `/tank/home` belongs to `tank/home`, not `tank`.
3. The snapshot list shows the snapshots newest first with creation date and size; a dataset without snapshots is not at the root.
4. Inside a snapshot, rename, trash and the new-item commands are not offered; a confirmed permanent delete fails as read-only and the file stays; a copy out of the snapshot arrives in the live folder.
5. *Browse snapshots* opens the newest snapshot with the folder; *Back to live folder* returns, to the nearest existing parent if needed.
6. *Versions* lists only the snapshots with a different copy, also for a file deleted live.
7. *Restore* always asks once, naming the snapshot; a deleted file comes back with its content and modification time; a live file in the way is in the trash afterwards; a declined question changes nothing; missing parents are created.
8. *Create* with the default name makes `rick-YYYY-MM-DD-HHMMSS`; an invalid name cannot be confirmed; a taken name fails and names it. *Rename* and *Destroy* work; a held snapshot stays and is reported.
9. A refused change offers elevation once; accepted, it runs `rick-zfs` with the same arguments; declined or dismissed, it reports "Permission denied". Without `pkexec` or the helper, the message names the `zfs allow` command.
10. `rick-zfs` refuses a dataset without `@` for the snapshot commands, any option but `snapshot -r`, a rename across datasets, a property outside the editor's seven or a value the editor would not send, and every other subcommand, and never starts `zfs` for them.
11. The property editor opens with the current values (`zstd-3` set locally is selected; *Inherit (lz4)* names the parent's value) and has no field for `mountpoint`, `readonly`, `dedup` or `encryption`; `50G` shows as 50 GiB, `fifty` is invalid; a quota below the space used, or no change, keeps it from applying; applying sets, then inherits, and a failed reset leaves the set values and names the property.
12. The real-pool test creates, lists, browses, restores from, renames and destroys its own snapshot, sets `compression` and `quota` and inherits them back, and leaves nothing behind.
13. Removing `ext-zfs` from the plugins folder leaves Rick fully functional.
14. The timeline of `/tank/home/ann/docs` has one point per snapshot of `tank/home` pointing at `ann/docs` inside it, the same from inside a snapshot; two snapshots of one second keep ZFS's order; three folders of a dataset read the list once.
