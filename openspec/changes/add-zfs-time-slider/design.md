# Design

## Context

See proposal.md for motivation and the SPEC.md conflict, decided in the review of #156. Relevant current state:

- Extensions return data and jobs and never draw (SPEC.md, Extension points). The Decorator already answers per location (`applies_to`, `invalidates_on`), returns quickly from caches and pushes late results through `DecorateCtx::publish`, which the application turns into `Event::Decorated`.
- `ui-egui/src/pane.rs` draws each pane as a top panel (the toolbar), then the file view and the foot. Since the design refresh SPEC.md fixes the toolbar to the navigation pair, the breadcrumb and a right group of view options menu plus pane handle, and says other controls are not added to it; hidden files and refresh live in the view options menu.
- A workspace tab is a whole layout of panes; each `model::Pane` holds its own explorer tabs. The timeline belongs to what a pane shows, so its toggle is per pane (review point 2).
- `app` already has a navigation that records history only on the first step: `Command::SpringLoad { pane, to, first }` for drags.
- `API_VERSION` is 18 on `dev`; every change to an extension trait bumps it, and plugins built for another version are refused.
- `add-zfs-snapshots` gives `ext-zfs` a snapshot list cached per dataset (30 s) and the `zfs` provider locations `/<dataset>/@<snapshot>/<rel>`.

## Goals / Non-Goals

**Goals:**
- One generic timeline contract in `ext`; the bar knows nothing about ZFS.
- No extension call for a timeline on the UI thread, and none while the bar is hidden (review point 1).
- Several sources for one folder end up in one bar (review point 3).
- No `zfs` process runs while the user scrubs; the bar draws from data the app already holds.

**Non-Goals:**
- No new extension point; no drawing by extensions.
- No change to the preview pane or the side panel.
- No per-point existence check of the folder (see Decision 6).

## Decisions

### 1. Two defaulted Decorator methods, not a sixth extension point
```rust
pub struct TimePoint {
    pub time: SystemTime,
    pub label: String,
    /// The source's name as the bar shows it ("ZFS").
    pub source: String,
    pub location: Location,
}
pub struct Timeline { pub live: Location, pub points: Vec<TimePoint> }

/// Whether this decorator can offer timelines at all; static, read once.
fn offers_timelines(&self) -> bool { false }
/// The timeline of `loc`; may block (it runs in a job). None when there is none.
fn timeline(&self, loc: &Location) -> Option<Timeline> { None }
```
A timeline is status of a directory, which is what SPEC.md says a Decorator answers, and the Decorator is the one point that annotates locations it does not own (Serge's review). Defaulted methods keep every existing decorator source-compatible. `offers_timelines` lets the application decide without calling into the extension whether the *Timeline* entry can exist for a location: it is there when a decorator that offers timelines `applies_to` the location. *Alternative:* a sixth point `TimelineSource` — SPEC.md fixes the count at five, and the manifest, the Extensions screen and the plugin host would all grow for one method. *Alternative:* a Provider method — the live folder belongs to `vfs-local`, which cannot know about ZFS.

### 2. Read in a job, only while the bar is shown, cached per location (review point 1)
`timeline()` may run `zfs` on a cold cache, and a pool with thousands of snapshots makes even a warm answer non-trivial. So the application never calls it from `handle` or the UI: while a pane's bar is shown (toggle on, or the pane at a past point), every navigation of that pane spawns a job that calls `timeline(loc)` on each decorator with `offers_timelines() && applies_to(loc)`, merges the answers (Decision 3) and publishes `Event::Timeline { pane, location, timeline }`; the model stores it for the pane's current location. A newer navigation cancels the job. The merged result is kept per location until the location is refreshed, a change its decorators watch arrives for it, or 30 seconds have passed, the age the decorators already use, so returning to a folder or toggling the bar again does not ask again. (Built: not on `Event::Decorated`, which fires for every listing batch and would make the cache useless.) The source caches what it reads per its own scope: `ext-zfs` answers from the snapshot list cached per dataset, so every folder of one dataset costs at most one `zfs list`. While the job runs the bar shows its *reading* state (Decision 5).

### 3. Several sources merge into one list (review point 3)
Every answering decorator contributes its points; the application concatenates them, sorts by time (stable, so equal times keep registration order) and keeps the first answer's `live`. Each point carries its `source`, which the tooltip and the label show ("b · ZFS · 12 Nov 14:30"), so a ZFS snapshot and a backup of the same minute stay distinguishable. The pane's current point is the one whose location equals the pane's location; at the live location it is *Now*. *Alternative:* first registered wins — hides a second source completely.

### 4. `Command::TimeTravel { pane, to, first }`, modelled on `SpringLoad`
The bar emits `TimeTravel` with `first: true` on the first move after any other navigation and `false` after, so `app` pushes one history entry per run. When `to` does not exist, `app` walks up to the nearest existing parent inside the same point and publishes `Event::Notify` with the "did not exist then" text. *Alternative:* plain `Navigate` — a scrub through twenty points would fill the history with twenty entries.

### 5. Visibility: a per-pane flag, never an empty bar (review points 2 and 4)
`model::Pane` gains `timeline_shown: bool` (saved in the session, default off); `Command::ToggleTimeline(PaneId)` flips it. The *Timeline* checkbox entry in the view options menu and the palette command *Toggle timeline* exist only where a decorator that offers timelines applies to the location; elsewhere the entry is absent and the palette lists the command `Disabled("No timeline for this folder")`. Once a read for the location came back empty, both stay but are disabled with "No earlier states of this folder". The bar is drawn only with a non-empty timeline, or in its *reading* state for at most the time of the first read after the toggle (a thin bar with the reading text, no axis); a read that comes back empty removes it again and leaves the entry disabled. The bar also shows, toggle or not, while the pane is at a past point of the timeline it holds: reached along the bar, or switched off there; it goes when the pane is back at *Now*. A folder inside a snapshot opened another way (*Browse snapshots*) shows the bar when the toggle is on; with the toggle off nobody is asked, as review point 1 wants, so no bar. A move along the bar keeps the timeline the pane holds, so scrubbing asks no source. No toolbar control is added.

### 6. Points evenly spaced, drawn as dots, every point reachable
*Revised in the hands-on test on a real dataset:* the axis spaces the points evenly by index, oldest at the left end, the newest one step left of *Now*, and draws each as a dot; dots closer than a dot's width and a pixel are drawn as one pill. Spacing by time, the first design, hid points on a real dataset: two snapshots in the same minute merged into one thin tick at the left end, and one taken minutes ago sat under *Now*'s handle three days of axis away. The cost is that the axis no longer shows the gaps when nothing was snapshotted; the tooltip and the label give each point's date and time. The handle snaps to the nearest point, and Left/Right step by index, so every point is reachable even where dots merge. While dragging, the label follows the handle at once but `TimeTravel` is sent only after the handle rests 200 ms or on release, so a fast scrub does not start a listing per point. Points are not checked for the folder's existence: with thousands of snapshots each `stat` may auto-mount one; the fallback to the nearest parent (Decision 4) covers the miss.

### 7. Design first (review point 5)
The bar is a visible layout change, so its design is approved before the pull request. There is no access to the Figma file *Rick — FileExplorer*, so the approval is given on screenshots instead of a frame: `mockup.html` was the proposal, and the bar built to it is rendered headlessly in every state, in Light and Dark (`screenshots_of_the_time_slider`: at *Now*, at a past point read-only, reading, focused with a tick's tooltip, and the view options menu), plus the same states against a real ZFS dataset. The read-only state at a past point: the row takes `bg/selected-soft`, the label reads the point's date, label and source followed by a *Read-only* pill, and a *Back to now* button sits at the right end. Where the review asks for changes, the bar and this design are updated before the pull request; a Figma frame can be drawn from the screenshots later.

### 8. Bar layout (as proposed in the mock-up)
A top panel under the toolbar panel, as tall as the toolbar (`button/size` plus twice `space/toolbar-y`) with the same `space/toolbar-x` padding and design tokens: the `CLOCK_COUNTER_CLOCKWISE` icon, the current label (*Now* with the count of earlier states; or date and time, label and source with the *Read-only* pill), the axis filling the rest with the handle, and *Now* at the axis's right end or the *Back to now* button. The label column is as wide as the longest thing it may say for this timeline (at most 45 % of the row) and the right end always keeps the button's width, so the axis does not move when the pane goes into the past: a drag would otherwise shift the ticks under the pointer (found by the drag test). The bar takes keyboard focus on click; the focus ring is `accent/focus`, and while it is focused the arrows, Home and End are the bar's, not the list's. Colours: `bg/track` and `text/secondary` dots at *Now*, `text/on-selected` on `bg/selected-soft` in the past, `accent/focus` for the handle. The font set has no bold face, so the date and *Now* are set apart by colour only.

### 9. ZFS implements both methods from its cache
`ext-zfs` returns `true` from `offers_timelines` and, in `timeline`, maps a live path or a `zfs` snapshot location to `(dataset, rel)` and builds one point per snapshot from `Datasets::snapshots(dataset)`, which reads `zfs list` only when the per-dataset cache is cold or older than 30 s: `/<dataset>/@<s>/<rel>`, creation time, snapshot name, source "ZFS". A dataset without snapshots answers `None`.

## Risks / Trade-offs

- [API version bump rebuilds every plugin] → all plugins are built from this workspace and ship bundled; the bump is in the same pull request as the trait change.
- [Thousands of snapshots make the axis unreadable] → merged ticks and keyboard stepping; a zoomable axis is a later improvement.
- [Landing on a point without the folder surprises the user] → the nearest-parent fallback plus a notice; dimming absent points stays out of scope.
- [The first read after toggling takes a moment on a cold cache] → the *reading* state instead of an empty bar; later navigations in the same dataset are served from the source's cache.
- [Two sources disagree about *Now*] → the first answer's `live` is used; every source in this workspace answers with the pane's own live folder.
- [No Figma frame] → the design is reviewed on screenshots of the built bar before the pull request; changes asked for there are made before it opens.
- [Large listings per scrub step] → the 200 ms rest rule; listings of the previous point are cancelled when the pane moves on, as for every navigation.

## Migration Plan

Session files gain the pane's `timeline_shown` with a serde default of `false`, so older sessions load unchanged. Plugins built against API 18 are refused until rebuilt, which the workspace build does. Rollback is reverting the change and the version bump together.
