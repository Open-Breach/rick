# Proposal

## Why

Rick runs on ZFS machines but treats ZFS as plain POSIX: the user cannot see, create or browse snapshots, and the most common reason to have them, getting back a deleted or overwritten file, means leaving Rick for a shell. A `zfs` command line is on every OpenZFS system, and each dataset already exposes its snapshots as read-only trees under `.zfs/snapshot/`, so the feature needs no kernel change, only a plugin.

**Conflict with SPEC.md.** SPEC.md places ZFS snapshots in `ext-deco-fs` ("later", explicitly "Not in 0.2", shared with Btrfs subvolumes and NTFS streams). This change instead proposes a feature plugin `ext-zfs` now, following SPEC.md's naming rule for a feature that fills several extension points (like `ext-git`). That needs the maintainer's decision; the first task records it as a numbered question in QUESTIONS.md.

## What Changes

- New plugin crate `ext-zfs` (Linux only, `plugins/zfs/`), one manifest filling Provider, Decorator and Action.
- Detects whether a local location lies in a mounted ZFS dataset through the `zfs` command line; without `zfs` or outside a dataset the plugin contributes nothing visible.
- A read-only `zfs` provider: an overview of datasets, each dataset's snapshots as folders, and each snapshot's file tree, so snapshots open in tabs, the breadcrumb, the preview and the history like any folder; copying out of a snapshot works through the normal copy path.
- A *Versions* view for one file: every snapshot that holds a different version of it, newest first, each previewable and copyable.
- *Restore*: put a file or folder from a snapshot back at its live location; a live item it would replace goes to the trash first.
- Snapshot management: create (with a name defaulting to a timestamp, optionally recursive), rename and destroy, as jobs.
- Permissions: each management command runs first as the user (whatever `zfs allow` delegates); when ZFS refuses it for lack of permission, Rick offers to run it once more elevated through the platform's prompt (polkit on Linux).
- An info-panel section with the entry's dataset, its snapshot count, the newest snapshot, and the dataset's main properties read-only (guid, space, compression, record size, encryption and more), each with its source (local, inherited, default).
- **Out of scope:** the time-machine slider (change `add-zfs-time-slider`), rollback, clones, holds, send/receive, scheduled or automatic snapshots, pool and dataset administration and editing properties (change `add-zfs-property-editor`), `zfs diff` badges, Btrfs and other copy-on-write file systems, macOS and Windows OpenZFS ports.

## Capabilities

### New Capabilities

- `zfs-snapshots`: detecting ZFS datasets, browsing snapshots read-only, file versions, restoring from a snapshot, the dataset info section with its properties, and creating, renaming and destroying snapshots with the permission and elevation rule.

### Modified Capabilities

None (no existing OpenSpec capability; SPEC.md is not edited by this change).

## Impact

- **New crate** `crates/ext-zfs` depending on `ext` and `types` only (plus `thiserror`, `tracing`, `futures`, `async-trait` as the other plugins do); dependencies point downward only.
- **Workspace wiring**: `Cargo.toml` members, a `zfs` feature on `rick`, `tests` dev-dependency, `deny.toml` wrapper entries, plugin bundling as for `ext-svn`.
- **Uses existing kernel surfaces** without changing them: `Registry::provider`, `decorator`, `action`, `platform()` for `Trash` and `Elevate`, `ActionOutcome::View`/navigate, `UiHost` forms and confirmations.
- **External tool**: `zfs` (OpenZFS ≥ 2.0) at run time; `pkexec` for the elevated path. CI has neither ZFS nor a pool, so tests run against a fake `zfs` and a fixture tree; real-pool tests are opt-in.
- **Locales**: new `en.ftl` and `de.ftl` for the plugin.
