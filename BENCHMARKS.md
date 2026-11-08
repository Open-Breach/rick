# Benchmarks

The acceptance benchmarks of version 0.2: the fifteen of 0.1, the five the
0.1 close-out added and the nine of the 0.2 milestones, measured on the
GitHub-hosted CI runners by the `bench` workflow
(`.github/workflows/bench.yml`): every benchmark five times in release on
each runner, the median and the worst run below. Each benchmark is an
`#[ignore]`d test in `crates/tests/tests`, timed against the target of the
runner's OS in `crates/tests/src/bench.rs`, whose table a unit test checks
against this file. Since issue 35 CI measures the targets without asserting
them: the `bench` workflow runs by hand, on release tags and nightly on
Linux x64 from `dev`, its nightly summaries on the `benchmark-history`
branch; a release run without `RICK_BENCH_RECORD` asserts them.

**Targets.** Linux: the spec's (Decisions table, QUESTIONS.md 5, 24 and
30), asserted on a single run. macOS and Windows: the median × 1.5, rounded
up to a round number, never below the Linux target, and past the worst run
where that sits higher; where that lands at or under the spec's number, the
spec's number is the target (QUESTIONS.md 188). On macOS and Windows a test
repeats its measured part three times and asserts the median of the three
against the target (`tests::bench::runs`, QUESTIONS.md 80): one run on
those shared runners swings too much for a 1.5 × budget. The tables below
keep the single-run medians and worsts of the measurement run. Startup,
cancel and session start have no spec number, so their targets on every OS
come from the measurements: median × 2 for startup and session start, with a
5 ms floor, median × 1.5 for cancel. The first run of `pdf-first` on a
runner pays the cold start (the library and the file in no cache yet); it
shows in the Worst column where it did.

Measured: bench run
[42488206033](https://github.com/Open-Breach/rick/actions/runs/42488206033),
2026-11-08, commit `d140302`, five runs per benchmark on every runner
(issue 25); `3d-first`, which landed on `dev` after that run, waits for the
next one. It supersedes the 0.1 runs of 2026-11-06 and 2026-11-07
([42311657502](https://github.com/Open-Breach/rick/actions/runs/42311657502),
[42328854042](https://github.com/Open-Breach/rick/actions/runs/42328854042)),
whose numbers are quoted below where a target was set from them. The
artifacts `bench-Linux`, `bench-macOS` and `bench-Windows` hold the five
raw logs, the hardware line and the summary of each runner.

**What changed on macOS since 0.1.** 0.1 shipped four macOS targets set to
what the runner did rather than the spec: the listing's first batch (400 ms)
and complete listing (3 s), the PDF's first frame (1.2 s) and the tree view's
first children (100 ms). Re-measured, the runner meets the spec on the
listing's first batch (11.7 ms, was 154 ms) and the PDF's first frame
(3.4 ms where 0.1's warm runs measured 482 to 761 ms), whose targets are
the spec's 50 ms and 200 ms again; the complete listing is faster (1024 ms,
was 1303 ms) and its target
tightens to 1.6 s; the tree view's first children measure 71.1 ms (worst
79.2 ms), so its 100 ms target stays, the rule's 107 ms being no tighter.
The fixture's 40 s tightens to 35 s (22.9 s, was 26.0 s). Startup fell from
63.4 to 12.2 ms (target 25 ms, was 150 ms). One macOS target had to
loosen: cancelling the sparse copy took 53.6 ms in the median where 0.1's
runs had mostly cancelled a job that was already done, so its target is
80 ms (was 50 ms). Elsewhere the same run tightened Windows' first search
batch to the spec's 100 ms (was 150 ms) and startup on every OS to 25 ms,
set the provisional macOS and Windows targets of `plugin-load` and
`archive-zip-first` to the Linux 50 ms, and loosened the Windows fixture
to 45 s, where that runner now sits just under the spec's 30 s.

## Runners

The hardware line as the runner reports it (GitHub's image name and version,
then the CPU, core count and memory the OS sees).

### Linux — `ubuntu-latest`

- Runner image: ubuntu24 20261101.360.1
- CPU: AMD EPYC 7763 64-Core Processor, 4 vCPU
- Memory: 15 GB

### macOS — `macos-latest`

- Runner image: macos26 20261102.0473.1
- CPU: Apple M1 (Virtual), 3 vCPU
- Memory: 7 GB

### Windows — `windows-latest`

- Runner image: win25-vs2026 20261103.298.1
- CPU: AMD EPYC 7763 64-Core Processor, 4 vCPU
- Memory: 16 GB

## Results

### `fixture` — generating the 200 000-entry fixture

Spec target: 30 s (Engineering guidelines → Tests; QUESTIONS.md 5). Test:
`two_hundred_thousand_entries_in_under_thirty_seconds` in `fixture.rs`.
Written to the runner's temp disk. macOS: 22.9 s × 1.5 = 34 s, target 35 s
(was 40 s from 26.0 s); every run was under the spec's 30 s, the rule's
headroom is not. Windows: 27.3 s × 1.5 = 41 s, target 45 s (was the
spec's 30 s): the 0.1 run measured 17.1 s and the issue 35 run 14.5 s on
other CPUs of the pool, this runner's five runs met the 30 s by under 3 s,
the Defender case QUESTIONS.md 5 excepted from the budget.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 30 s | 1888 ms | 2183 ms |
| macOS | 35 s | 22.9 s | 25.5 s |
| Windows | 45 s | 27.3 s | 28.1 s |

### `listing-first` — first batch of the 200 000-entry listing

Spec target: 50 ms (first rows within 100 ms of opening; the provider's half
of it, issue 2). Test:
`listing_200k_entries_first_batch_under_50ms_complete_under_400ms` in
`listing.rs`. macOS: 11.7 ms × 1.5 = 18 ms, under the spec's 50 ms, which
is the target again (0.1 measured 154 ms and set 400 ms).

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 50 ms | 2.0 ms | 2.0 ms |
| macOS | 50 ms | 11.7 ms | 14.2 ms |
| Windows | 50 ms | 1.6 ms | 1.6 ms |

### `listing-complete` — complete listing of 200 000 entries

Spec target: 400 ms (issue 2). Same test as `listing-first`. macOS:
1024 ms × 1.5 = 1536 ms, target 1.6 s (0.1 measured 1303 ms and set 3 s
after CI runs of 2469 ms).

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 400 ms | 232 ms | 234 ms |
| macOS | 1.6 s | 1024 ms | 1058 ms |
| Windows | 400 ms | 150 ms | 155 ms |

### `copy-sparse` — copying a 1 GiB sparse file

Spec target: 10 s through the fast path (issue 2). Test:
`copying_a_1gib_sparse_file_uses_the_fast_path` in `copy.rs`. Windows copies
the holes too (`CopyFileEx`); 1.5 × its median stays under the spec.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 10 s | 3.2 ms | 3.4 ms |
| macOS | 10 s | 0.4 ms | 1.0 ms |
| Windows | 10 s | 318 ms | 321 ms |

### `details-frame` — scrolling 200 000 entries in details view

Spec target: 60 fps, that is a 16.6 ms frame; the 95th percentile of 240
scrolling frames on egui's CPU frame (QUESTIONS.md 24). Test:
`scrolling_the_200k_fixture_in_details_view_holds_60_fps` in `ui.rs`.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 16.6 ms | 3.1 ms | 3.1 ms |
| macOS | 16.6 ms | 6.9 ms | 9.0 ms |
| Windows | 16.6 ms | 1.3 ms | 1.6 ms |

### `image-first` — first frame of the 48 MB bitmap

Spec target: 200 ms (QUESTIONS.md 30). Test:
`a_50_mb_image_and_a_300_page_pdf_show_a_first_frame_within_200_ms_without_blocking_the_ui`
in `preview.rs`.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 200 ms | 123 ms | 126 ms |
| macOS | 200 ms | 80.5 ms | 92.0 ms |
| Windows | 200 ms | 98.0 ms | 107 ms |

### `pdf-first` — first frame of the 300-page PDF

Spec target: 200 ms (QUESTIONS.md 30). Same test as `image-first`. The
first run of the five on Linux (84.7 ms) carries the cold start and is the
Worst; macOS measures 3.4 ms where 0.1's warm runs took 482 to 761 ms
(median 726 ms, target 1.2 s), so 3.4 ms × 1.5 = 5 ms sits under the
spec's 200 ms, which is the target again.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 200 ms | 10.7 ms | 84.7 ms |
| macOS | 200 ms | 3.4 ms | 5.7 ms |
| Windows | 200 ms | 6.1 ms | 6.5 ms |

### `preview-frame` — worst UI frame while the previews decode

Spec target: 16.6 ms (no UI frame over the budget while a decode runs
elsewhere, QUESTIONS.md 30). Same test as `image-first`.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 16.6 ms | 5.9 ms | 6.5 ms |
| macOS | 16.6 ms | 5.3 ms | 5.8 ms |
| Windows | 16.6 ms | 3.0 ms | 3.5 ms |

### `git-batch-cached` — decorating a visible batch with a cached status

Spec target: 16 ms for a batch of 100 entries of a 10 000-file repository
(issue 6). Test: `decorating_a_visible_batch_of_a_10k_repository_is_fast` in
`git.rs`.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 16 ms | 0.1 ms | 0.1 ms |
| macOS | 16 ms | 0.0 ms | 0.1 ms |
| Windows | 16 ms | 0.1 ms | 0.1 ms |

### `git-batch-pending` — decorating a visible batch without a cached status

Spec target: 16 ms; the batch is answered `Pending` at once (issue 6). Same
test as `git-batch-cached`.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 16 ms | 0.0 ms | 0.0 ms |
| macOS | 16 ms | 0.1 ms | 0.1 ms |
| Windows | 16 ms | 0.1 ms | 0.1 ms |

### `git-complete` — the real decorations of the batch published

Spec target: 500 ms after the uncached request (issue 6). Same test as
`git-batch-cached`.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 500 ms | 45.9 ms | 47.4 ms |
| macOS | 500 ms | 37.2 ms | 59.7 ms |
| Windows | 500 ms | 27.4 ms | 29.6 ms |

### `thumbnails` — the icon view's thumbnails of 40 pictures and PDFs

Spec target: 500 ms (GUI chrome round). Test:
`the_icon_view_thumbnails_arrive_within_500_ms_without_long_frames` in
`chrome.rs`. Three of the five Windows runs found every thumbnail already
there when the view opened and measured nothing.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 500 ms | 12.7 ms | 15.1 ms |
| macOS | 500 ms | 37.7 ms | 52.8 ms |
| Windows | 500 ms | 0.0 ms | 7.6 ms |

### `thumbnails-frame` — longest UI frame while the thumbnails arrive

Spec target: 16.6 ms. Same test as `thumbnails`.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 16.6 ms | 2.5 ms | 4.8 ms |
| macOS | 16.6 ms | 3.2 ms | 8.1 ms |
| Windows | 16.6 ms | 0.0 ms | 2.6 ms |

### `wrap-frame` — 200 000 wrapped lines in the preview pane

Spec target: 16.6 ms for every frame (preview pane round); the worst of
toggling wrap and three scroll frames. Test:
`two_hundred_thousand_wrapped_lines_keep_every_frame_under_the_budget` in
`preview_pane.rs`.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 16.6 ms | 1.0 ms | 1.1 ms |
| macOS | 16.6 ms | 0.8 ms | 2.0 ms |
| Windows | 16.6 ms | 1.3 ms | 1.4 ms |

### `viewer-frame` — stepping through 50 images in the viewer

Spec target: 16.6 ms for every frame (Viewer, milestone 3 of 0.2); the
worst UI frame while the viewer steps through 50 images with Right, the
decodes running off the UI thread. Test:
`stepping_through_fifty_images_keeps_every_frame_under_the_budget` in
`viewer.rs`. First measured by the bench workflow in the 0.2 run.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 16.6 ms | 1.2 ms | 1.3 ms |
| macOS | 16.6 ms | 2.5 ms | 11.0 ms |
| Windows | 16.6 ms | 1.2 ms | 1.8 ms |

### `startup` — startup to the first frame

No spec target; the targets are median × 2 on each OS, rounded up: 10.4,
12.2 and 10.6 ms give 25 ms on all three. Headless: the app built with
every built-in extension on a 100-entry folder, then the shell's frames in
the `egui_kittest` harness until it is stable; window and GPU creation are
not included and the platform is the fake. Test:
`startup_to_the_first_frame` in `startup.rs`. 0.1 measured 7.6, 63.4 and
16.8 ms (targets 20, 150 and 40 ms); macOS fell by five times, the others
moved with their runs.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 25 ms | 10.4 ms | 10.9 ms |
| macOS | 25 ms | 12.2 ms | 20.7 ms |
| Windows | 25 ms | 10.6 ms | 11.6 ms |

### `plugin-load` — loading the Git plugin at start

Issue 16: the plugin host scanning a plugins directory with the Git plugin,
checking its manifest, loading the library and letting it register, which is
what the plugin build adds to startup over the compiled-in build; the issue
asks for 50 ms on CI Linux. Test:
`loading_the_git_plugin_adds_at_most_50_ms_to_startup` in `plugins.rs`. macOS and Windows measure under a millisecond, so the
provisional 100 ms gives way to the Linux 50 ms, the rule's floor.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 50 ms | 0.4 ms | 0.5 ms |
| macOS | 50 ms | 0.3 ms | 0.4 ms |
| Windows | 50 ms | 0.2 ms | 0.3 ms |

### `archive-zip-first` — the first batch of a 10 000-entry zip

Issue 20: the archive provider reading a 10 000-entry zip's central
directory, building its index and handing the root listing's first batch to
the stream, measured from `list` to the batch's arrival with a cold cache;
the issue asks for 50 ms. Test: `a_10_000_entry_zip_lists_its_first_batch_within_50_ms`
in `providers.rs`. macOS: 21.7 ms × 1.5 = 33 ms and Windows 30.1 ms × 1.5 =
45 ms, both under the Linux 50 ms, which is their target; the provisional
100 ms targets go.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 50 ms | 22.2 ms | 22.8 ms |
| macOS | 50 ms | 21.7 ms | 29.9 ms |
| Windows | 50 ms | 30.1 ms | 30.4 ms |

### `tree-first` — the tree view's first children on the 200k fixture

Spec target: 50 ms for the first screen after expanding a folder (close-out
A). Test: `tree_view_on_the_200k_fixture_expands_within_the_budget` in
`closeout.rs`. macOS: 71.1 ms × 1.5 = 107 ms, no tighter than the 100 ms
0.1 set from CI runs of 65.6 and 80.8 ms, and every run of the five was
under 100 ms, so the target stays.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 50 ms | 3.9 ms | 4.3 ms |
| macOS | 100 ms | 71.1 ms | 79.2 ms |
| Windows | 50 ms | 4.3 ms | 4.5 ms |

### `search-first` — first batch of a subtree search

Spec target: 100 ms for the first matches of `foo` over 20 nested levels and
a 12 000-file folder (close-out A). Test:
`the_first_search_batch_arrives_within_100_ms` in `closeout.rs`. macOS:
75.6 ms in the median, 87.0 ms at worst, both under the spec's 100 ms,
which stands as in 0.1 (50.6 ms then). Windows: 60.7 ms × 1.5 = 91 ms,
under the spec's 100 ms, which is the target again (0.1 measured 85.1 ms
and set 150 ms).

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 100 ms | 29.9 ms | 31.2 ms |
| macOS | 100 ms | 75.6 ms | 87.0 ms |
| Windows | 100 ms | 60.7 ms | 91.0 ms |

### `cancel` — cancelling the 1 GiB sparse copy from the operations area

No spec number (the job ends "within one batch"); the targets come from
the measurements: Linux 11.2 ms × 1.5 = 17 → 20 ms and Windows 6.5 ms ×
1.5 = 10 → 20 ms (not below Linux), both as in 0.1; macOS 53.6 ms × 1.5 =
80 ms (was 50 ms: three of 0.1's four runs cancelled a job that had already
finished, now four of five caught it running). Test:
`a_1gib_sparse_copy_shows_progress_pauses_and_cancels` in `closeout.rs`.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 20 ms | 11.2 ms | 11.7 ms |
| macOS | 80 ms | 53.6 ms | 56.0 ms |
| Windows | 20 ms | 6.5 ms | 11.7 ms |

### `session-start` — starting a three-window, twenty-tab session

No spec number; the test asserts the big session starts within 300 ms of a
one-tab one, and the absolute time of the big one is recorded here with
targets from the measurements: median × 2 as for startup, with a 5 ms floor
since the Linux and Windows medians sit near the jitter of a shared runner;
macOS 5.6 ms × 2 = 11.2 → 12 ms (was 15 ms). `App::new` with the session
file, headless. Test: `a_big_session_starts_about_as_fast_as_a_small_one`
in `session.rs`, which also runs in the ordinary test suite.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 5 ms | 0.6 ms | 0.7 ms |
| macOS | 12 ms | 5.6 ms | 6.9 ms |
| Windows | 5 ms | 1.5 ms | 1.6 ms |

### `office-convert` — a three-page DOCX converted by LibreOffice and its first page rendered

Issue 18's target: 5 s when LibreOffice is present. Test:
`a_docx_converts_within_5_s_when_libreoffice_is_present` in
`previewers.rs`, measured on Linux only (CI installs LibreOffice there;
macOS and Windows skip with a note, so their rows stay empty). The first
conversion of a fresh LibreOffice profile is paid once before the runs.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 5 s | 1055 ms | 1055 ms |
| macOS | 5 s | — | — |
| Windows | 5 s | — | — |

### `waveform` — a 3-minute MP3 opened and its waveform drawn

Issue 18's target: 500 ms, from opening the file to the waveform picture
(the Discoverer, the preroll and the decode of the whole file to peaks).
Test: `an_mp4_plays_and_seeks_and_a_3_minute_mp3_gets_its_waveform_within_500_ms`
in `previewers.rs`, with the `gstreamer` feature (Linux CI only, so the
other rows stay empty). The worst of the five runs sat 10 ms under the
target; the median has 170 ms to spare.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 500 ms | 332 ms | 490 ms |
| macOS | 500 ms | — | — |
| Windows | 500 ms | — | — |

### `zip-first` — the first screen of a 10 000-entry zip

Issue 18's target: 100 ms from opening the file to the first rows of the
listing. Test: `a_10000_entry_zip_lists_its_first_screen_within_100_ms` in
`previewers.rs`. macOS: 42.5 ms × 1.5 = 64 ms and Windows 43.8 ms × 1.5 =
66 ms, both under the 100 ms, which stands.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 100 ms | 39.6 ms | 39.8 ms |
| macOS | 100 ms | 42.5 ms | 56.3 ms |
| Windows | 100 ms | 43.8 ms | 46.6 ms |

### `csv-window` — a screen of rows anywhere in a 1 000 000-row CSV

Issue 18 asks the CSV grid to scroll at the frame budget; what the grid
costs a scrolling frame is the window of rows it asks for, so the worst of
fifty 60-row windows over the file is held to 16.6 ms. Test:
`a_million_row_csv_answers_any_window_within_a_frame` in `previewers.rs`,
which also prints the time to open and index the file.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 16.6 ms | 0.1 ms | 0.1 ms |
| macOS | 16.6 ms | 0.1 ms | 0.7 ms |
| Windows | 16.6 ms | 0.1 ms | 0.1 ms |

### `previewers-frame` — worst UI frame while the previewer plugins decode

Spec target: 16.6 ms (no UI frame over the budget while a decode runs
elsewhere). Test: `no_ui_frame_over_16_ms_while_the_previewers_decode` in
`previewers.rs`: every fixture of the milestone is selected in turn in the
preview pane, LibreOffice and GStreamer included where they are.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 16.6 ms | 6.4 ms | 6.5 ms |
| macOS | 16.6 ms | 4.4 ms | 5.4 ms |
| Windows | 16.6 ms | 1.8 ms | 1.9 ms |

### `3d-first` — a 500 000-triangle binary STL opened and its first frame rendered

Issue 19's target: 300 ms from opening the file to the first frame, for
each of its fixtures (a 12-triangle STL, a 500 000-triangle binary STL, an
OBJ with three objects, a 3MF in millimetres); the heaviest, the binary
STL, is the one recorded, the other three are held to the same 300 ms in
the test. Test: `each_fixture_shows_a_frame_within_300_ms` in
`preview_3d.rs`, rendered at 800 × 600 through wgpu on the runner's
software adapter (lavapipe on Linux, WARP on Windows; macOS skips with a
note when it has no adapter), the device opened once before the runs. Not
yet measured by the bench workflow; the local development machine opened
and rendered the fixture in 261 ms on lavapipe (the other three fixtures
in 8–24 ms).

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 300 ms | — | — |
| macOS | 300 ms | — | — |
| Windows | 300 ms | — | — |
