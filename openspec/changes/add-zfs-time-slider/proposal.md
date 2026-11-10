# Proposal

## Why

`add-zfs-snapshots` makes snapshots browsable one at a time, but going back in time is still a hunt through a list of names. A time slider in a bar under the toolbar, as Time Machine has it, lets the user scrub through the states of the folder they are looking at and stop where the file they lost is still there. Rick's extensions never draw, so the bar has to be a kernel element fed by extensions; built that way, a later Btrfs, Time Machine or backup plugin gets the same slider for free, and several sources can feed one bar.

**Conflict with SPEC.md, decided.** SPEC.md's pane layout has no row between the toolbar and the file list, the Decorator signature in "Core types and extension traits" gains methods, and `Command` gains two variants; signatures change only through a spec change. Serge accepted the approach on 2026-11-10 (QUESTIONS.md 368) with five changes, which this revision carries: the timeline is read off the UI thread and only while the bar is visible, cached per source scope; the toggle is per pane; several sources merge into one list; no empty bar; and a design frame comes before the bar is built. It depends on `add-zfs-snapshots` and is applied after it.

## What Changes

- **Kernel, generic:** a *timeline* is the live location plus points in time, each with a time, a label, the source that offers it (*ZFS*) and the location that shows the folder at that time. A Decorator declares that it can offer timelines and answers for one location through a defaulted method that the application calls in a job, never on the UI thread, and only while the bar is shown. The application merges the points of every decorator that answers into one list sorted by time.
- **Timeline bar:** a one-row bar directly under the pane's toolbar with a time axis, one tick per point, a handle, the selected point's date, label and source, and *Now* at the right end. Dragging, clicking a tick, or Left/Right/Home/End take the pane to that point; *Now* returns to the live folder. At a past point the bar says that the pane shows the past, read-only.
- **Visibility:** a *Timeline* entry in the toolbar's view options menu and a palette command toggle the bar per pane. Both are absent where no decorator can offer a timeline for the location, and disabled with the reason once the timeline was read and turned out empty; the bar is never shown empty. The bar shows by itself while the pane is at a past point. The toggle is part of the pane's session state.
- **History:** a run of slider moves counts as one navigation, so one Back returns to where the scrubbing started.
- **Design first:** the bar's layout and its states (at *Now*, at a past point read-only, reading, focused) are drawn as a frame in the Figma file *Rick — FileExplorer* before the bar is built. This change carries a mock-up (`mockup.html`) as the proposal for that frame.
- **ZFS:** `ext-zfs` offers the timeline for live folders inside a dataset and for folders inside its snapshots: one point per snapshot of the dataset, from its snapshot list cached per dataset.
- **Out of scope:** dimming points where the folder does not exist (that needs a stat per snapshot), carrying the selection across points, a per-file timeline, a slider in the preview pane, a setting for the default visibility, other timeline sources than ZFS.

## Capabilities

### New Capabilities

- `location-timeline`: the kernel's timeline contract for extensions and the timeline bar under the toolbar: what it shows, how it navigates, when it is visible, how sources merge, and how it records history.

### Modified Capabilities

- `zfs-snapshots`: gains the requirement that ZFS offers a timeline for dataset and snapshot folders (introduced by `add-zfs-snapshots`; that change is archived first).

## Impact

- **`ext`**: `Timeline` and `TimePoint` types (with `source`), `Decorator::offers_timelines() -> bool` and `Decorator::timeline(&Location) -> Option<Timeline>`, both defaulted (`false`, `None`); worked example in the crate docs; `API_VERSION` bumped to 18, so every plugin is rebuilt.
- **`model`**: per-pane `timeline_shown` flag and the merged timeline of the pane's current location.
- **`app`**: `Command::ToggleTimeline(PaneId)` and `Command::TimeTravel { pane, to, first }`; a job that asks the decorators for the timeline while the bar is shown, merges and publishes it; session persistence of the flag.
- **`ui-egui`**: the bar in `pane.rs` below the toolbar, the view options menu entry, palette entry, keyboard focus and screenshots, built to the approved frame. The toolbar itself gets no new control (SPEC.md: "Other controls are not added to the toolbar").
- **`ext-zfs`**: implements both methods from its snapshot list cached per dataset.
- **Locales**: kernel strings in the core locale files, ZFS strings in the plugin's.
- **SPEC.md** (Serge): the pane layout names the bar, and "Core types and extension traits" the two Decorator methods.
