# Tasks

## 1. Preconditions

- [ ] 1.1 Confirm `add-zfs-snapshots` is merged and archived, so `openspec/specs/zfs-snapshots` exists; verify `openspec list --specs` lists it
- [ ] 1.2 Append a numbered question to QUESTIONS.md: the timeline bar adds a row to SPEC.md's pane layout, `Decorator` gains `timeline`, `Command` gains `ToggleTimeline` and `TimeTravel`; state the assumption (design Decisions 1, 3, 4); verify the numbering

## 2. Extension contract

- [ ] 2.1 Add `Timeline` and `TimePoint` to `ext` and `Decorator::timeline` with a default of `None`, with doc comments and a worked example in the decorator docs; bump `API_VERSION` to 16 with its history note; verify `cargo doc -p ext` and the doctest
- [ ] 2.2 Verify every existing decorator still compiles unchanged and the plugin host refuses a plugin built for API 15 (existing version-mismatch test)

## 3. Model and application

- [ ] 3.1 Add `timeline_shown` and the current `Option<Timeline>` to `model::Tab`; save and restore `timeline_shown` in the session with a default of `false`; verify a session round-trip test and loading an old session file
- [ ] 3.2 In `app`, ask the applicable decorators for a timeline after a listing completes and on `Event::Decorated`, first registered wins; verify app tests with two fake decorators (spec: Timeline offered by an extension)
- [ ] 3.3 Add `Command::ToggleTimeline(PaneId)`; verify an app test that it flips the flag and that the palette entry is `Disabled` without a timeline
- [ ] 3.4 Add `Command::TimeTravel { pane, to, first }` with one history entry per run and the nearest-parent fallback with a notice; verify app tests for Back after scrubbing and a missing folder (spec: Scrubbing is one history step; Folder missing at that time)

## 4. Timeline bar

- [ ] 4.1 Compute the axis: time-to-x mapping, tick merging below 3 px, nearest point to a position, index stepping; as pure functions in `ui-egui`; verify unit tests including thousands of points and a single point
- [ ] 4.2 Draw the bar under the toolbar with icon, label, axis, handle and *Now*; visibility by design Decision 4; verify screenshot tests for the bar at *Now* and at a past point, in light and dark theme
- [ ] 4.3 Wire input: click a tick, drag with the 200 ms rest rule, Left/Right/Home/End when focused, tooltip on hover; emitting `TimeTravel`; verify UI tests with egui's test harness for click, keyboard and drag-release
- [ ] 4.4 Add the *Timeline* checkbox entry to the view options menu (only with a timeline) and the palette command *Toggle timeline*; add strings to the core `en.ftl` and `de.ftl`; verify a screenshot test of the open menu and the i18n test

## 5. ZFS source

- [ ] 5.1 Implement `timeline` in `ext-zfs` from the dataset cache for live and snapshot folders, `None` while uncached with the read started from `decorate`; verify unit tests for each scenario of the spec requirement "ZFS timeline"
- [ ] 5.2 Verify by hand on the ZFS machine: toggle the bar in a home folder, scrub to an older snapshot, restore a file from there, press Back once and land in the live folder

## 6. Integration

- [ ] 6.1 Run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --workspace`, `cargo deny check`, `cargo doc --no-deps`; verify all pass, and add the bar to CHANGELOG.md

## Workflow follow-up

- Open the pull request against `dev` with the assumption from 1.2, the API bump, and what was left out (absent-point dimming, selection carry-over, per-file timeline).
- After merge, archive with `/opsx:archive` so `location-timeline` becomes a living spec and `zfs-snapshots` gains the timeline requirement.
