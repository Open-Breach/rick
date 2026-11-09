# Proposal

## Why

`add-zfs-snapshots` shows a dataset's properties in the info section, but changing one, such as turning on `zstd` compression for a photo dataset or setting a quota on a shared folder, still needs a shell and the exact `zfs set` syntax. A small editor for the properties that are safe to change while the system runs closes that gap without turning Rick into a pool administration tool.

## What Changes

- New action *Edit ZFS properties…* in the "ZFS" group of `ext-zfs`, on a live folder inside a dataset (editing that dataset) and on a dataset folder at the root of the `zfs` location.
- A form with one field per editable property, opened with the current values: `compression`, `atime`, `relatime`, `copies`, `recordsize`, `quota`, `refquota`. Every field offers *Inherit* (showing the value it would inherit) besides the explicit values.
- The form's preview lists each change as old → new with the old value's source, and marks invalid values (a size that does not parse, a quota below the space already used), which disables applying.
- Applying runs as one job: all explicit values in one `zfs set`, each property set back to *Inherit* through `zfs inherit`, then the dataset's cached properties are refreshed so the info section shows the new values.
- Permissions follow the rule of `add-zfs-snapshots`: own rights first, then one offer to run the same commands elevated.
- A note in the form that new values apply to data written from now on, for `compression`, `copies` and `recordsize`.
- **Out of scope:** properties that can break a running system or need planning (`mountpoint`, `canmount`, `readonly`, `sharenfs`, `sharesmb`, `dedup`, `encryption` and key handling, `reservation`, `refreservation`, `volsize`, `sync`, `xattr`, `acltype`, `casesensitivity`), user properties (`module:name`), recursive changes to child datasets, undo, pool properties, rewriting existing data with the new settings.

## Capabilities

### New Capabilities

- `zfs-dataset-properties`: editing the safe subset of a ZFS dataset's properties: which properties, which values, validation, applying with inherit, and the permission rule.

### Modified Capabilities

None. The read-only display stays as `zfs-snapshots` specifies it; this change only refreshes it after an edit, which that requirement already covers.

## Impact

- **`ext-zfs` only**: one action, the form and its preview, the apply job, size parsing, new locale strings. Uses `UiHost::form_later` / `ActionOutcome::Later` (the form needs values read by a job), `Elevate`, and the dataset cache from `add-zfs-snapshots`.
- No kernel crate, trait, `Command` or API version changes.
- Depends on `add-zfs-snapshots` (cache, runner, error classification, elevation rule); independent of `add-zfs-time-slider`.
