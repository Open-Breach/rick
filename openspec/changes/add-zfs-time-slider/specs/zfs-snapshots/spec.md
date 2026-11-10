# Spec Delta

## ADDED Requirements

### Requirement: ZFS timeline
The ZFS plugin SHALL declare that it offers timelines and SHALL answer for a live folder inside a dataset that has snapshots and for a folder inside one of its snapshots: one point per snapshot of that dataset, labelled with the snapshot name, with the source "ZFS" and timed at its creation, each pointing at the same relative folder inside that snapshot (`/<dataset>/@<snapshot>/<path>`), with the live folder as *Now*. It SHALL answer from its snapshot list cached per dataset and read `zfs` only when that cache is cold or older than 30 seconds.

#### Scenario: Live folder
- **WHEN** the pane shows `/tank/home/ann/docs` and `tank/home` has snapshots `a` and `b`
- **THEN** the timeline has the points `a` and `b`, pointing at `ann/docs` inside each, and `/tank/home/ann/docs` as *Now*

#### Scenario: Inside a snapshot
- **WHEN** the pane shows `ann/docs` inside snapshot `a`
- **THEN** the timeline is the same as for the live folder and the pane is at point `a`

#### Scenario: Dataset without snapshots
- **WHEN** the dataset has no snapshots
- **THEN** no timeline is offered

#### Scenario: One read per dataset
- **WHEN** the bar is shown and the user opens three folders of `tank/home` within 30 seconds
- **THEN** `zfs list` for the snapshots of `tank/home` runs at most once
