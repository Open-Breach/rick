# Spec Delta

## Purpose

Lets users change the few ZFS dataset properties that are safe to change on a running system, such as compression and quotas, from inside Rick, with the same permission rule as snapshot management.

## ADDED Requirements

### Requirement: Where the editor is offered
The system SHALL offer *Edit ZFS properties…* on a live folder inside a dataset, editing that folder's dataset, and on a dataset folder at the root of the ZFS location, editing that dataset. It SHALL NOT be offered inside a snapshot or outside a dataset.

#### Scenario: Live folder
- **WHEN** the user opens the context menu in `/tank/home/ann`
- **THEN** the ZFS group offers *Edit ZFS properties…* for `tank/home`

#### Scenario: Inside a snapshot
- **WHEN** the user is inside a snapshot of `tank/home`
- **THEN** the action is not offered

### Requirement: Editable properties and values
The editor SHALL offer exactly these properties, each opened with its current value and each also offering *Inherit*, labelled with the value it would inherit: `compression` (off, on, lz4, zstd, gzip, zle, and the current value when it is another), `atime` and `relatime` (on, off), `copies` (1, 2, 3), `recordsize` (powers of two from 4K to 1M, and the current value when it is another), `quota` and `refquota` (none or a size).

#### Scenario: Current values
- **WHEN** the editor opens for a dataset with `compression=zstd-3` set locally
- **THEN** the compression field shows `zstd-3` selected, among the standard options and *Inherit (lz4)*

#### Scenario: Excluded property
- **WHEN** the editor is open
- **THEN** no field for `mountpoint`, `readonly`, `dedup` or `encryption` exists

### Requirement: Size values
The quota fields SHALL accept `none` or a size as a number with an optional unit K, M, G, T or P (powers of 1024, case-insensitive, optional `B` or `iB` suffix).

#### Scenario: Unit parsed
- **WHEN** the user enters `50G` as `quota`
- **THEN** the preview shows the quota as 50 GiB

#### Scenario: Not a size
- **WHEN** the user enters `fifty` as `quota`
- **THEN** the preview marks the value as invalid and applying is disabled

### Requirement: Preview and validation
The editor SHALL show one preview row per changed property, old value with its source and new value, and SHALL disable applying while any row has a problem or nothing changed. A `quota` or `refquota` below the space the dataset already uses SHALL be a problem. For `compression`, `copies` and `recordsize` the editor SHALL state that new values apply to data written from now on.

#### Scenario: Change listed
- **WHEN** the user changes `compression` from `lz4 (inherited from tank)` to `zstd`
- **THEN** the preview shows one row `compression: lz4 (inherited from tank) → zstd`

#### Scenario: Quota too small
- **WHEN** the dataset uses 80 GiB and the user enters `50G` as `quota`
- **THEN** the row says the quota is below the space already used and applying is disabled

#### Scenario: Nothing changed
- **WHEN** the user opens the editor and changes nothing
- **THEN** applying is disabled

### Requirement: Applying changes
Applying SHALL run as one job that sets all explicit values, resets each property chosen as *Inherit* to inheritance, and then refreshes the dataset's cached properties. If one step fails, the steps before it SHALL stay applied and the message SHALL name the property that failed and ZFS's reason.

#### Scenario: Set and inherit
- **WHEN** the user sets `compression` to `zstd` and `atime` to *Inherit*, and applies
- **THEN** `compression` is `zstd` with source local, `atime` shows its inherited value, and the info section shows both

#### Scenario: Partial failure
- **WHEN** setting the values succeeds and resetting `atime` to inherit fails
- **THEN** the new values stay set and a notification names `atime` and the reason

### Requirement: Permission rule for property changes
Applying SHALL run with the user's own rights first; when ZFS refuses for lack of permission, the system SHALL ask once whether to run the changes as administrator and, on confirmation, run the remaining commands through the platform's elevation prompt. A dismissed prompt SHALL change nothing further and report permission denied; without an elevation prompt the message SHALL name the `zfs allow` permissions needed for the changed properties.

#### Scenario: Delegated
- **WHEN** the user holds `zfs allow` rights for `compression` on the dataset and changes it
- **THEN** it is applied without an elevation prompt

#### Scenario: Elevated
- **WHEN** the user has no rights, accepts running as administrator and authenticates
- **THEN** the changes are applied

#### Scenario: No elevation available
- **WHEN** `pkexec` is missing and ZFS refuses
- **THEN** the message names the permissions to delegate, for example `zfs allow ann compression,quota tank/home`
