# Proposal

## Why

`add-zfs-snapshots` makes snapshots browsable one at a time, but going back in time is still a hunt through a list of names. A time slider in a bar under the toolbar, as Time Machine has it, lets the user scrub through the states of the folder they are looking at and stop where the file they lost is still there. Rick's extensions never draw, so the bar has to be a kernel element fed by extensions; built that way, a later Btrfs, Time Machine or backup plugin gets the same slider for free.

**Conflict with SPEC.md.** SPEC.md's pane layout has no row between the toolbar and the file list, the Decorator signature in "Core types and extension traits" gains a method, and `Command` gains two variants; signatures change only through a spec change. This change needs the maintainer's decision and records it as a numbered question in QUESTIONS.md first. It also depends on `add-zfs-snapshots` and is applied after it.

## What Changes

- **Kernel, generic:** a *timeline* is an ordered list of points in time for a location, each with a label and the location that shows the folder at that time, plus the live location. A Decorator may offer one for a location through a new, defaulted method; the first decorator that offers one wins.
- **Timeline bar:** a one-row bar directly under the pane's toolbar with a time axis, one tick per point, a handle, the selected point's date and label, and *Now* at the right end. Dragging, clicking a tick, or Left/Right/Home/End take the pane to that point; *Now* returns to the live folder.
- **Visibility:** a *Timeline* entry in the toolbar's view options menu (only where a timeline exists) and a palette command toggle the bar per tab; the bar shows by itself while the tab is at one of the timeline's past points. The toggle is part of the tab's session state.
- **History:** a run of slider moves counts as one navigation, so one Back returns to where the scrubbing started.
- **ZFS:** `ext-zfs` offers the timeline for live folders inside a dataset and for folders inside its snapshots: one point per snapshot of the dataset, oldest to newest.
- **Out of scope:** dimming points where the folder does not exist (that needs a stat per snapshot), carrying the selection across points, a per-file timeline, a slider in the preview pane, a setting for the default visibility, other timeline sources than ZFS.

## Capabilities

### New Capabilities

- `location-timeline`: the kernel's timeline contract for extensions and the timeline bar under the toolbar: what it shows, how it navigates, when it is visible, and how it records history.

### Modified Capabilities

- `zfs-snapshots`: gains the requirement that ZFS offers a timeline for dataset and snapshot folders (introduced by `add-zfs-snapshots`; that change is archived first).

## Impact

- **`ext`**: `Timeline` and `TimePoint` types, `Decorator::timeline(&Location) -> Option<Timeline>` with a default of `None`; worked example in the crate docs; `API_VERSION` bumped (16), so every plugin is rebuilt.
- **`model`**: per-tab `timeline_shown` flag and the timeline of the current location.
- **`app`**: `Command::ToggleTimeline(PaneId)` and `Command::TimeTravel { pane, to, first }`; asks the decorators for the timeline after a listing and on `Decorated` events; session persistence of the flag.
- **`ui-egui`**: the bar in `pane.rs` below the toolbar, the view options menu entry, palette entry, keyboard focus and screenshots. The toolbar itself gets no new control (SPEC.md: "Other controls are not added to the toolbar").
- **`ext-zfs`**: implements `timeline` from its dataset cache.
- **Locales**: kernel strings in the core locale files, ZFS strings in the plugin's.
