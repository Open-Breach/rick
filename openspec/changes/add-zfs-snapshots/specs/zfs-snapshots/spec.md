# Spec Delta

## Purpose

Lets users of OpenZFS on Linux see, browse and manage the snapshots of the datasets they work in, and get earlier versions of files back, without leaving Rick.

## ADDED Requirements

### Requirement: Dataset detection
The system SHALL treat a local location as inside a ZFS dataset when it lies at or below the mount point of a mounted ZFS file system, choosing the dataset with the longest matching mount point. When the `zfs` command is missing or fails, no location SHALL be treated as inside a dataset.

#### Scenario: Nested datasets
- **WHEN** datasets `tank` (mounted at `/tank`) and `tank/home` (mounted at `/tank/home`) exist and the user opens `/tank/home/ann/docs`
- **THEN** the location is treated as inside `tank/home`

#### Scenario: No ZFS tooling
- **WHEN** `zfs` is not installed
- **THEN** no ZFS action, column or panel section appears anywhere and no error is shown

#### Scenario: Not on ZFS
- **WHEN** the user opens a folder on an ext4 file system
- **THEN** the ZFS actions are hidden for that location

### Requirement: Linux only
The plugin SHALL declare Linux as its only platform and SHALL NOT be loaded on macOS or Windows.

#### Scenario: Other platform
- **WHEN** Rick starts on Windows with the ZFS plugin in the plugins directory
- **THEN** the Extensions screen lists the plugin as not supported on this platform and it is not loaded

### Requirement: Snapshot overview
The system SHALL offer a ZFS location whose root lists the mounted datasets that have snapshots, and whose dataset level lists that dataset's snapshots as folders, newest first, each with its creation time as the modification time and the space it holds as its size.

#### Scenario: Snapshots of a dataset
- **WHEN** the user opens the snapshot list of `tank/home`, which has snapshots `a` (Monday) and `b` (Tuesday)
- **THEN** two folders are listed, `b` before `a`, dated Tuesday and Monday

#### Scenario: Dataset without snapshots
- **WHEN** a mounted dataset has no snapshots
- **THEN** it is not listed at the root of the ZFS location

### Requirement: Browse a snapshot read-only
The system SHALL show a snapshot's files and folders as they were when it was taken, with preview, copy and the usual navigation, and SHALL refuse every change inside a snapshot: no write, rename, trash, new file or new folder is offered, and a permanent delete, which the core offers wherever there is a selection, fails with a message that snapshots are read-only.

#### Scenario: Preview a file in a snapshot
- **WHEN** the user selects a text file inside snapshot `b`
- **THEN** the preview shows the file's content as of snapshot `b`

#### Scenario: Copy out of a snapshot
- **WHEN** the user copies a file from snapshot `b` and pastes it into a live folder
- **THEN** the file is copied into the live folder as a normal copy job

#### Scenario: Change refused
- **WHEN** the user opens the context menu on an entry inside a snapshot
- **THEN** rename, move to trash and the new-item commands are not available

#### Scenario: Delete refused
- **WHEN** the user deletes an entry inside a snapshot permanently and confirms
- **THEN** the job fails with a message that snapshots are read-only and the entry is still there

### Requirement: Open the snapshots of the current folder
The system SHALL offer an action that takes the pane from a live folder in a dataset to the same folder inside the newest snapshot that contains it, and from there back to the live folder.

#### Scenario: Into the newest snapshot
- **WHEN** the user is in `/tank/home/ann/docs` and invokes *Browse snapshots*
- **THEN** the pane shows `ann/docs` inside the newest snapshot of `tank/home` that has that folder

#### Scenario: Folder in no snapshot
- **WHEN** the current folder was created after the newest snapshot
- **THEN** the action is disabled with the reason that no snapshot contains this folder

#### Scenario: Back to live
- **WHEN** the user is inside a snapshot and invokes *Back to live folder*
- **THEN** the pane shows the corresponding live folder, or its nearest existing parent if it no longer exists

### Requirement: File versions
The system SHALL offer, for one selected file in a dataset, a view listing each snapshot that holds a version of that file different from the next newer one listed (by size or modification time), newest first, labelled with the snapshot and its date. Each listed version SHALL be previewable and copyable like a file in a snapshot.

#### Scenario: Unchanged versions collapsed
- **WHEN** a file is identical in snapshots `a` and `b` and changed before snapshot `c`
- **THEN** the versions view lists `c` and `b` but not `a`

#### Scenario: Deleted file
- **WHEN** the selected location is a snapshot entry of a file that no longer exists live
- **THEN** its versions view still lists the snapshots that hold it

### Requirement: Restore from a snapshot
The system SHALL offer *Restore* for entries inside a snapshot or a versions view, copying each selected item back to its corresponding live location as a job. When a live item exists at that location, it SHALL first be moved to the trash, after the user confirmed the replacement; if trashing fails, that item SHALL NOT be restored.

#### Scenario: Restore a deleted file
- **WHEN** the user restores `report.odt` from snapshot `b` and no `report.odt` exists live
- **THEN** `report.odt` is copied to its live folder with its snapshot content

#### Scenario: Replace a live file
- **WHEN** the user restores `report.odt` from snapshot `b`, a live `report.odt` exists and the user confirms the replacement
- **THEN** the live file is in the trash and the snapshot version is at its place

#### Scenario: Replacement declined
- **WHEN** the user declines the replacement
- **THEN** nothing is changed

#### Scenario: Missing live parent
- **WHEN** the live parent folder of the restored item no longer exists
- **THEN** the missing folders are created before the item is restored

### Requirement: Create a snapshot
The system SHALL offer *Create snapshot* in a location inside a dataset, asking for a name prefilled with a timestamp (`rick-YYYY-MM-DD-HHMMSS`) and an option to include child datasets, and SHALL create the snapshot of the location's dataset as a job.

#### Scenario: Default name
- **WHEN** the user creates a snapshot of `tank/home` at 2026-11-09 14:30:05 and accepts the default
- **THEN** snapshot `tank/home@rick-2026-11-09-143005` exists

#### Scenario: Invalid name
- **WHEN** the user enters a name with a character ZFS does not allow in snapshot names
- **THEN** the dialogue marks the name as invalid and cannot be confirmed

#### Scenario: Name taken
- **WHEN** a snapshot of that name already exists
- **THEN** the job fails with a message naming the existing snapshot and nothing is changed

### Requirement: Rename a snapshot
The system SHALL offer *Rename snapshot* on one snapshot folder in the snapshot list, with the same name validation as creation.

#### Scenario: Rename
- **WHEN** the user renames `tank/home@a` to `before-upgrade`
- **THEN** the list shows `before-upgrade` instead of `a`

### Requirement: Destroy snapshots
The system SHALL offer *Destroy snapshot* on snapshot folders in the snapshot list, after a confirmation that names each snapshot and states that it cannot be undone. A snapshot ZFS refuses to destroy (held, or with dependent clones) SHALL be reported with ZFS's reason and left in place.

#### Scenario: Destroy confirmed
- **WHEN** the user destroys `tank/home@a` and confirms
- **THEN** `a` is gone from the list

#### Scenario: Held snapshot
- **WHEN** the user destroys a snapshot that has a hold
- **THEN** the job fails with a message that the snapshot is held, and the snapshot remains

### Requirement: Permission and elevation
Every create, rename and destroy SHALL run first with the user's own rights. When ZFS refuses it for lack of permission, the system SHALL ask whether to run it as administrator; on confirmation it SHALL run the same command once through the platform's elevation prompt, and on refusal or a dismissed prompt it SHALL report that permission was denied and change nothing.

#### Scenario: Delegated rights
- **WHEN** the user holds `zfs allow` rights for `snapshot` on `tank/home` and creates a snapshot
- **THEN** it is created without any elevation prompt

#### Scenario: Elevation accepted
- **WHEN** the user has no rights, accepts running as administrator and authenticates in the polkit prompt
- **THEN** the snapshot is created

#### Scenario: Elevation dismissed
- **WHEN** the user dismisses the polkit prompt
- **THEN** a notification says permission was denied and no snapshot exists

#### Scenario: No elevation available
- **WHEN** `pkexec` is not installed and ZFS refuses the command
- **THEN** the message names the missing rights and the `zfs allow` permission needed, and no elevation is offered

### Requirement: Snapshot lists stay current
The system SHALL refresh its knowledge of a dataset's snapshots after each create, rename or destroy it ran, and SHALL NOT show a cached snapshot list older than 30 seconds when a ZFS location is listed.

#### Scenario: Snapshot made outside Rick
- **WHEN** a snapshot is taken from a shell and the user refreshes the snapshot list more than 30 seconds later
- **THEN** the new snapshot is listed

### Requirement: Dataset info section
The system SHALL add a *ZFS* section to the info badges of an entry inside a dataset, showing the dataset name, the number of snapshots and the name and date of the newest snapshot, followed by the dataset's properties (Dataset properties shown).

#### Scenario: Info section
- **WHEN** the user hovers the ZFS info badge of a file in `tank/home`, which has 12 snapshots
- **THEN** the section shows `tank/home`, 12 snapshots, the newest snapshot's name and date, and the dataset properties

### Requirement: Dataset properties shown
The ZFS section SHALL list, read-only and in this order, the properties `guid`, `type`, `mountpoint`, `used`, `available`, `referenced`, `usedbysnapshots`, `quota`, `refquota`, `reservation`, `compression`, `compressratio`, `recordsize`, `dedup`, `copies`, `atime`, `encryption`, `keystatus` and `readonly`, sizes in human units, each value marked with its source (local, inherited from a named dataset, default or received). A property the installed ZFS does not report SHALL be left out.

#### Scenario: Inherited value
- **WHEN** `compression` is set to `zstd` on `tank` and not set on `tank/home`
- **THEN** the section shows `compression` as `zstd`, inherited from `tank`

#### Scenario: Unencrypted dataset
- **WHEN** the dataset is not encrypted
- **THEN** `encryption` reads `off` and `keystatus` is left out

#### Scenario: Copy a value
- **WHEN** the user copies the `guid` row
- **THEN** the clipboard holds the guid as a plain number
