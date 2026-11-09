# Design

## Context

See proposal.md for motivation and the SPEC.md conflict. Relevant current state:

- Extensions return data and jobs and never draw (SPEC.md, Extension points). The Decorator already answers per location (`applies_to`, `invalidates_on`), returns quickly from caches and pushes late results through `DecorateCtx::publish`, which the application turns into `Event::Decorated`.
- `ui-egui/src/pane.rs` draws each tab as a top panel (the toolbar), then the file view and the foot. Since the design refresh SPEC.md fixes the toolbar to the navigation pair, the breadcrumb and a right group of view options menu plus pane handle, and says other controls are not added to it; hidden files and refresh live in the view options menu.
- `app` already has a navigation that records history only on the first step: `Command::SpringLoad { pane, to, first }` for drags.
- `model::Tab` holds per-tab view state (view mode, hidden, flatten …) that `app::session` saves and restores.
- `API_VERSION` is 15 on `dev`; every change to an extension trait bumps it, and plugins built for another version are refused.
- `add-zfs-snapshots` gives `ext-zfs` a cached snapshot list per dataset and the `zfs` provider locations `/<dataset>@<snapshot>/<rel>`.

## Goals / Non-Goals

**Goals:**
- One generic timeline contract in `ext`; the bar knows nothing about ZFS.
- No `zfs` process runs while the user scrubs; the bar draws from data the app already holds.

**Non-Goals:**
- No new extension point; no drawing by extensions.
- No change to the preview pane or the side panel.
- No per-point existence check of the folder (see Decision 5).

## Decisions

### 1. A defaulted `Decorator::timeline`, not a sixth extension point
```rust
pub struct TimePoint { pub time: SystemTime, pub label: String, pub location: Location }
pub struct Timeline { pub live: Location, pub points: Vec<TimePoint> /* oldest first */, pub current: Option<usize> /* None = live */ }
fn timeline(&self, loc: &Location) -> Option<Timeline> { None }
```
A timeline is status of a directory, which is what SPEC.md says a Decorator answers; a defaulted method keeps every existing decorator source-compatible. *Alternative:* a sixth point `TimelineSource` with its own registry slot — cleaner in isolation, but SPEC.md fixes the count at five and the manifest's `points` flags, the Extensions screen and the plugin host would all grow for one method. *Alternative:* a Provider method — the live folder belongs to `vfs-local`, which cannot know about ZFS.

### 2. The app asks after listing and on `Decorated`
`app` calls `timeline(loc)` on the decorators whose `applies_to(loc)` is true after a listing completes and again on each `Event::Decorated` for that pane (that is when a decorator's cache has just been filled), stores the result in the tab, and the UI reads it from the model. `timeline` must return at once from a cache, like `panel`. *Alternative:* a polling call per frame from the UI — the UI must not call extensions.

### 3. `Command::TimeTravel { pane, to, first }`, modelled on `SpringLoad`
The bar emits `TimeTravel` with `first: true` on the first move after any other navigation and `false` after, so `app` pushes one history entry per run. When `to` does not exist, `app` walks up to the nearest existing parent inside the same point and publishes `Event::Notify` with the "did not exist then" text. *Alternative:* plain `Navigate` — a scrub through twenty points would fill the history with twenty entries.

### 4. Visibility: a per-tab flag plus "at a past point"
`model::Tab` gains `timeline_shown: bool` (saved in the session, default off); `Command::ToggleTimeline(PaneId)` flips it. The bar is drawn when `timeline.is_some() && (timeline_shown || timeline.current.is_some())`. The toggle is a checkbox entry *Timeline* in the view options menu, next to the hidden-files entry, present only when the tab has a timeline; no toolbar control is added. The palette lists *Toggle timeline* with `Disabled("No timeline for this folder")` otherwise. *Alternative:* show the bar whenever a timeline exists — on a root-on-ZFS machine that is every folder, a permanent row the user did not ask for.

### 5. Ticks by time, drawn merged, every point reachable
The axis maps time linearly from the oldest point to now; ticks closer than 3 logical pixels are drawn as one. The handle snaps to the nearest point by time, and Left/Right step by index, so every point is reachable even where ticks merge. While dragging, the label follows the handle at once but `TimeTravel` is sent only after the handle rests 200 ms or on release, so a fast scrub does not start a listing per point. *Alternative:* equal spacing per point — keeps ticks apart but hides the gaps that tell the user when nothing was snapshotted. Points are not checked for the folder's existence: with thousands of snapshots each `stat` may auto-mount one; the fallback to the nearest parent (Decision 3) covers the miss.

### 6. Bar layout
A top panel under the toolbar panel, as tall as the toolbar (`button/size` plus twice `space/toolbar-y`) with the same `space/toolbar-x` padding and design tokens: the `CLOCK_COUNTER_CLOCKWISE` icon, the current label (date, time and label, or "Now"), the axis filling the rest with the handle, and *Now* as the axis's right end. The bar takes keyboard focus on click or Tab; the focus ring follows the theme. Colours come from the design tokens already used for the operations area's progress bars and `accent/focus` for the focus ring.

### 7. ZFS implements `timeline` from its cache
`ext-zfs` maps a live path or a `zfs` snapshot location to `(dataset, rel)`, builds one point per cached snapshot (`/<dataset>@<s>/<rel>`, creation time, snapshot name) and sets `current` when the location is inside a snapshot. With no cached list it returns `None` and starts the read in `decorate`, whose `publish` makes the app ask again (Decision 2).

## Risks / Trade-offs

- [API version bump rebuilds every plugin] → all plugins are built from this workspace; the bump is in the same pull request as the trait change.
- [Thousands of snapshots make the axis unreadable] → merged ticks and keyboard stepping; a zoomable axis is a later improvement.
- [Landing on a point without the folder surprises the user] → the nearest-parent fallback plus a notice; dimming absent points stays out of scope.
- [Spec conflict on the pane layout and the trait] → the QUESTIONS.md entry comes first; the change touches no other GUI element.
- [Large listings per scrub step] → the 200 ms rest rule; listings of the previous point are cancelled when the pane moves on, as for every navigation.

## Migration Plan

Session files gain `timeline_shown` with a serde default of `false`, so older sessions load unchanged. Plugins built against API 15 are refused until rebuilt, which the workspace build does. Rollback is reverting the change and the version bump together.
