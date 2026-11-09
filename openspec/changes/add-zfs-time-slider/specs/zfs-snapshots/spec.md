# Spec Delta

## ADDED Requirements

### Requirement: ZFS timeline
The ZFS plugin SHALL offer a timeline for a live folder inside a dataset that has snapshots and for a folder inside one of its snapshots: one point per snapshot of that dataset, oldest first, labelled with the snapshot name and timed at its creation, each pointing at the same relative folder inside that snapshot, with the live folder as *Now*. It SHALL answer from its cached snapshot list without running `zfs`.

#### Scenario: Live folder
- **WHEN** the pane shows `/tank/home/ann/docs` and `tank/home` has snapshots `a` and `b`
- **THEN** the timeline has the points `a` and `b`, pointing at `ann/docs` inside each, and `/tank/home/ann/docs` as *Now*

#### Scenario: Inside a snapshot
- **WHEN** the pane shows `ann/docs` inside snapshot `a`
- **THEN** the timeline is the same as for the live folder and the current point is `a`

#### Scenario: Dataset without snapshots
- **WHEN** the dataset has no snapshots
- **THEN** no timeline is offered

#### Scenario: Snapshot list not cached yet
- **WHEN** the timeline is asked for before the dataset's snapshot list has been read
- **THEN** no timeline is offered at first and the bar's controls appear once the list has been read
