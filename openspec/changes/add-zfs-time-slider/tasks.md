# Tasks

## 1. Preconditions

- [ ] 1.1 Confirm `add-zfs-snapshots` is merged and archived, so `openspec/specs/zfs-snapshots` exists; verify `openspec list --specs` lists it
- [x] 1.2 Record the decided question in QUESTIONS.md (368): the bar adds a row to SPEC.md's pane layout, `Decorator` gains `offers_timelines` and `timeline`, `Command` gains `ToggleTimeline` and `TimeTravel`; accepted with five changes (review of #156); verify the numbering
- [ ] 1.3 SPEC.md names the bar in the pane layout and the two Decorator methods in "Core types and extension traits" (Serge's edit, or his approval of the agent's pull request as in QUESTIONS.md 324); verify the text matches design Decisions 1, 5 and 8
- [x] 1.4 Propose the bar's frame as a mock-up (`mockup.html`): at *Now*, at a past point read-only, reading, focused, in Light and Dark, from SPEC.md's tokens; verify it opens in a browser and every state is shown in both schemes
- [ ] 1.5 The frame is drawn in the Figma file *Rick — FileExplorer*, page *Main view*, and approved by Serge; verify design Decisions 7 and 8 match it, updating them where the frame differs

## 2. Extension contract

- [ ] 2.1 Add `Timeline` and `TimePoint` (with `source`) to `ext`, and `Decorator::offers_timelines` (default `false`) and `Decorator::timeline` (default `None`), with doc comments that say `timeline` runs in a job and may block, and a worked example in the decorator docs; bump `API_VERSION` to 18 with its history note; verify `cargo doc -p ext` and the doctest
- [ ] 2.2 Verify every existing decorator still compiles unchanged and the plugin host refuses a plugin built for API 17 (existing version-mismatch test)

## 3. Model and application

- [ ] 3.1 Add `timeline_shown` to `model::Pane` and the merged timeline of the pane's location with its state (unknown, reading, empty, points); save and restore `timeline_shown` in the session with a default of `false`; verify a session round-trip test and loading an old session file
- [ ] 3.2 In `app`, while a pane's bar is shown, spawn a job per navigation that asks every decorator with `offers_timelines() && applies_to(loc)`, merges the points sorted by time and publishes `Event::Timeline`; cancel it on the next navigation; keep the result per location until `Event::Decorated` for it or 30 s; verify app tests with two fake decorators for merging and sources, one that blocks (the pane stays responsive, the state is reading), and one that counts calls (none while hidden, one per location while shown) (spec: Timeline offered by extensions)
- [ ] 3.3 Add `Command::ToggleTimeline(PaneId)`; verify an app test that it flips the pane's flag, that the palette entry is `Disabled` where no timeline source applies and after an empty answer, and absent from the menu where no source applies (spec: Timeline visibility)
- [ ] 3.4 Add `Command::TimeTravel { pane, to, first }` with one history entry per run and the nearest-parent fallback with a notice; verify app tests for Back after scrubbing and a missing folder (spec: Scrubbing is one history step; Folder missing at that time)

## 4. Timeline bar (after 1.5)

- [ ] 4.1 Compute the axis: time-to-x mapping, tick merging below 3 px, nearest point to a position, index stepping; as pure functions in `ui-egui`; verify unit tests including thousands of points and a single point
- [ ] 4.2 Draw the bar under the toolbar to the approved frame: icon, label with source, axis, handle, *Now*, the read-only state at a past point and the reading state; never drawn without points otherwise; verify screenshot tests for each state in Light and Dark
- [ ] 4.3 Wire input: click a tick, drag with the 200 ms rest rule, Left/Right/Home/End when focused, tooltip on hover with source; emitting `TimeTravel`; verify UI tests with egui's test harness for click, keyboard and drag-release
- [ ] 4.4 Add the *Timeline* checkbox entry to the view options menu (only where a source applies, disabled after an empty answer) and the palette command *Toggle timeline*; add strings to the core `en.ftl` and `de.ftl`; verify a screenshot test of the open menu and the i18n test

## 5. ZFS source

- [ ] 5.1 Implement `offers_timelines` and `timeline` in `ext-zfs` from `Datasets::snapshots`, with source "ZFS"; verify unit tests for each scenario of the spec requirement "ZFS timeline", including that three folders of one dataset read `zfs list` once
- [ ] 5.2 Verify by hand on the ZFS machine: toggle the bar in a folder of the test dataset, scrub to an older snapshot, restore a file from there, press Back once and land in the live folder

## 6. Integration

- [ ] 6.1 Run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --workspace`, `cargo deny check`, `cargo doc --no-deps`; verify all pass, and add the bar to CHANGELOG.md

## Workflow follow-up

- Open the pull request against `dev` with QUESTIONS.md 368, the API bump, and what was left out (absent-point dimming, selection carry-over, per-file timeline).
- After merge, archive with `/opsx:archive` so `location-timeline` becomes a living spec and `zfs-snapshots` gains the timeline requirement.
