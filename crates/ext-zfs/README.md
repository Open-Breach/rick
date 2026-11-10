# ext-zfs

`ext-zfs` brings the snapshots of OpenZFS datasets into Rick on Linux. It
finds the dataset a local folder lies in through the `zfs` command line
(longest mount point wins; without `zfs` or outside a dataset it shows
nothing), and registers a read-only provider `zfs` whose root lists the
datasets with snapshots, each dataset its snapshots newest first, and each
snapshot its file tree as it was, read from `.zfs/snapshot/` under the
mount point; a `/~versions/<path>` location lists the snapshots that hold a
different version of one file. The "ZFS" action group opens the snapshots
of the current folder and goes back to the live one, opens a file's
versions, restores entries from a snapshot (a live item it replaces goes
to the trash first), and creates, renames and destroys snapshots as jobs,
offering to run a command once more through the platform's elevation
prompt when ZFS refuses it for lack of permission. A Decorator adds a ZFS
section with the dataset, its snapshots and its main properties to the
info panel. Every call to ZFS goes through the `Zfs` runner trait, which
tests replace with a scripted fake. It is a plugin (`plugins/zfs/`,
manifest platform Linux) that depends on `ext` and `types` only; `rick`
(behind `zfs`) and `tests` link it, and the plugin host loads it.
