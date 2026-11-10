# ext-zfs — English, the fallback language.

zfs-name-empty = enter a name
zfs-name-character = “{ $character }” is not allowed in a snapshot name
zfs-name-too-long = the name is too long

zfs-versions-of = Versions of { $name }

zfs-zfs = ZFS
zfs-browse-snapshots = Browse snapshots
zfs-back-to-live-folder = Back to live folder
zfs-versions = Versions
zfs-no-snapshot-holds-folder = no snapshot contains this folder
zfs-select-one-file = select one file first
zfs-restore = Restore
zfs-select-items-to-restore = select what to restore first
zfs-restore-replaces = These live items will be moved to the trash and replaced by their snapshot version:
    { $items }
zfs-restoring = Restore { $count ->
    [one] one item
   *[other] { $count } items
} from a snapshot
zfs-not-restored = Not restored, the live item could not be moved to the trash or copied:
    { $items }
zfs-create-snapshot = Create snapshot
zfs-create-snapshot-of = Create a snapshot of { $dataset }
zfs-create = Create
zfs-snapshot-name = Snapshot name
zfs-include-children = Include child datasets
zfs-creating = Create snapshot { $name }
zfs-snapshot-exists = A snapshot named { $name } exists already; nothing was changed.
zfs-rename-snapshot = Rename snapshot
zfs-rename = Rename
zfs-renaming = Rename snapshot { $name }
zfs-select-one-snapshot = select one snapshot first
zfs-destroy-snapshot = Destroy snapshot
zfs-select-snapshots = select snapshots first
zfs-destroy-confirm = Destroy these snapshots? This cannot be undone.
    { $names }
zfs-destroying = Destroy { $count ->
    [one] one snapshot
   *[other] { $count } snapshots
}
zfs-snapshot-busy = { $name } is held or has dependent clones and was kept: { $reason }
zfs-run-as-administrator = Run as administrator?
zfs-run-as-administrator-ok = Run as administrator
zfs-permission-denied-ask = ZFS refused “{ $name }” for lack of permission. Run it once more as administrator?
zfs-as-administrator = { $name } (as administrator)
zfs-permission-denied = Permission denied; nothing was changed.
zfs-needs-permission = You lack the rights for this on { $dataset }, and no administrator prompt is available. An administrator can grant them with:
    { $command }
