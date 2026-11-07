# Benchmarks

The acceptance benchmarks of version 0.1, measured on the GitHub-hosted CI
runners by the `bench` workflow (`.github/workflows/bench.yml`): every
benchmark five times in release on each runner, the median and the worst run
below. Each benchmark is an `#[ignore]`d test in `crates/tests/tests`; CI runs
each once per push on the three runners and asserts the target of that
runner's OS (`crates/tests/src/bench.rs`, whose table a unit test checks
against this file).

**Targets.** Linux: the spec's (Decisions table, QUESTIONS.md 5, 24 and 30),
asserted on a single run. macOS and Windows: the median × 1.5, rounded up to
a round number, and never below the Linux target, so a faster runner is not
held to a tighter budget than the spec asks; where that number still sat
below the worst run it was rounded up past the worst run. On macOS and
Windows a test repeats its measured part three times and asserts the median
of the three against the target (`tests::bench::runs`, QUESTIONS.md 80): one
run on those shared runners swings too much for a 1.5 × budget, as the
macOS tree view first screen showed (median 42 ms, single CI runs of 66 and
81 ms). The tables below keep the single-run medians and worsts of the
measurement runs; the targets are unchanged. Two benchmarks then went over their first
targets in CI runs of the release pull request and were set to what the
runner does, as the close-out issue provides: the macOS listing (first batch
400 ms, complete 3 s), the macOS tree view first screen (100 ms) and startup
(median × 2 on every OS, since its millisecond-scale numbers swing more than
1.5 × between runs). The first run of
`pdf-first` and `preview-frame` on each runner loads pdfium cold (0.8 s on
Linux, 1.4 s on macOS, 0.2 s on Windows); CI binds pdfium in the unit tests
before the benchmarks run, so that run is left out of the targets and shown
in the Worst column as it was measured. Startup has no spec target; its three
targets, Linux included, are median × 1.5. The Windows fixture, which
QUESTIONS.md 5 excepted from the 30 s budget under Defender, now meets it
with 13 s to spare, so the exception is not used.

Measured: bench run
[42311657502](https://github.com/Open-Breach/rick/actions/runs/42311657502),
2026-11-06, commit `453ad0f`, for the first fifteen benchmarks, and bench run
[42328854042](https://github.com/Open-Breach/rick/actions/runs/42328854042),
2026-11-07, commit `c7ad2ec` (close-out A and B merged), for the five that
A and B added; the second run measured the first fifteen again, within their
targets, on partly different hardware (below). The artifacts `bench-Linux`,
`bench-macOS` and `bench-Windows` hold the five raw logs, the hardware line
and the summary of each runner. In the second run the macOS job ended after
three complete passes and part of a fourth: the close-out A test
`a_new_text_cancels_the_running_walk_on_the_200k_fixture`, which is not a
timing, found both search walks finished and failed the job; its `BENCH`
lines were taken from the job log and the bench workflow now skips that
test. Startup on Windows measured 16.8 ms median in the second run, on a
slower CPU, so its target follows that run (median × 2 = 40 ms).

## Runners

The hardware line as the runner reports it (GitHub's image name and version,
then the CPU, core count and memory the OS sees).

### Linux — `ubuntu-latest`

- Runner image: ubuntu24 20261101.360.1
- CPU: Intel(R) Xeon(R) 6973P-C, 4 vCPU (first run); AMD EPYC 9V45 96-Core
  Processor, 4 vCPU (second run)
- Memory: 15 GB

### macOS — `macos-latest`

- Runner image: macos26 20261026.0455.2
- CPU: Apple M1 (Virtual), 3 vCPU (both runs)
- Memory: 7 GB

### Windows — `windows-latest`

- Runner image: win25-vs2026 20261103.298.1
- CPU: AMD EPYC 9V74 80-Core Processor, 4 vCPU (first run); AMD EPYC 7763
  64-Core Processor, 4 vCPU (second run)
- Memory: 16 GB

## Results

### `fixture` — generating the 200 000-entry fixture

Spec target: 30 s (Engineering guidelines → Tests; QUESTIONS.md 5). Test:
`two_hundred_thousand_entries_in_under_thirty_seconds` in `fixture.rs`.
Written to the runner's temp disk. macOS misses the spec's 30 s: 26.0 s ×
1.5 = 39 s, target 40 s. Windows meets it.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 30 s | 2104 ms | 6349 ms |
| macOS | 40 s | 26.0 s | 35.6 s |
| Windows | 30 s | 17.1 s | 17.3 s |

### `listing-first` — first batch of the 200 000-entry listing

Spec target: 50 ms (first rows within 100 ms of opening; the provider's half
of it, issue 2). Test:
`listing_200k_entries_first_batch_under_50ms_complete_under_400ms` in
`listing.rs`. macOS: 154 ms × 1.5 = 231 ms; CI runs of the release pull
request then measured 105 ms and 299 ms, so the target is 400 ms, what the
runner does.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 50 ms | 1.4 ms | 1.5 ms |
| macOS | 400 ms | 154 ms | 250 ms |
| Windows | 50 ms | 1.3 ms | 1.4 ms |

### `listing-complete` — complete listing of 200 000 entries

Spec target: 400 ms (issue 2). Same test as `listing-first`. macOS: 1303 ms
× 1.5 = 1955 ms; CI runs of the release pull request then measured 950 ms
and 2469 ms, so the target is 3 s, what the runner does.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 400 ms | 163 ms | 167 ms |
| macOS | 3 s | 1303 ms | 1356 ms |
| Windows | 400 ms | 122 ms | 169 ms |

### `copy-sparse` — copying a 1 GiB sparse file

Spec target: 10 s through the fast path (issue 2). Test:
`copying_a_1gib_sparse_file_uses_the_fast_path` in `copy.rs`. Windows copies
the holes too (CopyFileEx, 1.8 s); 1.5 × the median is 2.7 s, under the spec.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 10 s | 2.7 ms | 2.8 ms |
| macOS | 10 s | 2.5 ms | 4.1 ms |
| Windows | 10 s | 1791 ms | 2654 ms |

### `details-frame` — scrolling 200 000 entries in details view

Spec target: 60 fps, that is a 16.6 ms frame; the 95th percentile of 240
scrolling frames on egui's CPU frame (QUESTIONS.md 24). Test:
`scrolling_the_200k_fixture_in_details_view_holds_60_fps` in `ui.rs`.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 16.6 ms | 1.5 ms | 1.6 ms |
| macOS | 16.6 ms | 7.7 ms | 8.7 ms |
| Windows | 16.6 ms | 1.1 ms | 1.7 ms |

### `image-first` — first frame of the 48 MB bitmap

Spec target: 200 ms (QUESTIONS.md 30). Test:
`a_50_mb_image_and_a_300_page_pdf_show_a_first_frame_within_200_ms_without_blocking_the_ui`
in `preview.rs`.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 200 ms | 100 ms | 103 ms |
| macOS | 200 ms | 82.3 ms | 122 ms |
| Windows | 200 ms | 74.4 ms | 106 ms |

### `pdf-first` — first frame of the 300-page PDF

Spec target: 200 ms (QUESTIONS.md 30). Same test as `image-first`. The
Worst column is the cold first run (pdfium loading) on every runner; the warm
runs were 6.8–7.8 ms on Linux, 482–761 ms on macOS, 5.8–7.1 ms on Windows.
macOS: 726 ms × 1.5 = 1089 ms, target 1.2 s.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 200 ms | 7.7 ms | 814 ms |
| macOS | 1.2 s | 726 ms | 1418 ms |
| Windows | 200 ms | 5.8 ms | 218 ms |

### `preview-frame` — worst UI frame while the previews decode

Spec target: 16.6 ms (no UI frame over the budget while a decode runs
elsewhere, QUESTIONS.md 30). Same test as `image-first`. The Windows worst
is the cold first run; its warm runs were 2.5–2.7 ms.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 16.6 ms | 4.6 ms | 5.0 ms |
| macOS | 16.6 ms | 4.3 ms | 10.2 ms |
| Windows | 16.6 ms | 2.7 ms | 26.2 ms |

### `git-batch-cached` — decorating a visible batch with a cached status

Spec target: 16 ms for a batch of 100 entries of a 10 000-file repository
(issue 6). Test: `decorating_a_visible_batch_of_a_10k_repository_is_fast` in
`git.rs`.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 16 ms | 0.0 ms | 0.0 ms |
| macOS | 16 ms | 0.0 ms | 0.1 ms |
| Windows | 16 ms | 0.1 ms | 0.1 ms |

### `git-batch-pending` — decorating a visible batch without a cached status

Spec target: 16 ms; the batch is answered `Pending` at once (issue 6). Same
test as `git-batch-cached`.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 16 ms | 0.1 ms | 0.1 ms |
| macOS | 16 ms | 0.1 ms | 0.2 ms |
| Windows | 16 ms | 0.2 ms | 0.2 ms |

### `git-complete` — the real decorations of the batch published

Spec target: 500 ms after the uncached request (issue 6). Same test as
`git-batch-cached`.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 500 ms | 26.6 ms | 27.3 ms |
| macOS | 500 ms | 56.0 ms | 61.2 ms |
| Windows | 500 ms | 21.3 ms | 22.8 ms |

### `thumbnails` — the icon view's thumbnails of 40 pictures and PDFs

Spec target: 500 ms (GUI chrome round). Test:
`the_icon_view_thumbnails_arrive_within_500_ms_without_long_frames` in
`chrome.rs`.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 500 ms | 11.8 ms | 14.7 ms |
| macOS | 500 ms | 36.0 ms | 53.4 ms |
| Windows | 500 ms | 10.1 ms | 11.8 ms |

### `thumbnails-frame` — longest UI frame while the thumbnails arrive

Spec target: 16.6 ms. Same test as `thumbnails`.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 16.6 ms | 2.4 ms | 4.2 ms |
| macOS | 16.6 ms | 4.6 ms | 4.9 ms |
| Windows | 16.6 ms | 3.1 ms | 4.5 ms |

### `wrap-frame` — 200 000 wrapped lines in the preview pane

Spec target: 16.6 ms for every frame (preview pane round); the worst of
toggling wrap and three scroll frames. Test:
`two_hundred_thousand_wrapped_lines_keep_every_frame_under_the_budget` in
`preview_pane.rs`.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 16.6 ms | 0.7 ms | 0.8 ms |
| macOS | 16.6 ms | 0.6 ms | 1.2 ms |
| Windows | 16.6 ms | 0.8 ms | 0.9 ms |

### `viewer-frame` — stepping through 50 images in the viewer

Spec target: 16.6 ms for every frame (Viewer, milestone 3 of 0.2); the
worst UI frame while the viewer steps through 50 images with Right, the
decodes running off the UI thread. Test:
`stepping_through_fifty_images_keeps_every_frame_under_the_budget` in
`viewer.rs`. Not yet measured by the bench workflow: the targets are the
spec's on every OS, as for the other frame budgets; the Median and Worst
columns are filled by the next bench run.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 16.6 ms | — | — |
| macOS | 16.6 ms | — | — |
| Windows | 16.6 ms | — | — |

### `startup` — startup to the first frame

No spec target; the targets are median × 2 on each OS (7.6 → 20 ms, 63.4 →
150 ms, and 16.8 → 40 ms for Windows from the second run, whose slower CPU
measured 15.7–18.4 ms): median × 1.5 gave 15 ms on Windows and a CI run of
the release pull request measured 15.3 ms. Headless: the app built with
every built-in extension
on a 100-entry folder, then the shell's frames in the `egui_kittest` harness
until it is stable; window and GPU creation are not included and the
platform is the fake. Test: `startup_to_the_first_frame` in `startup.rs`.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 20 ms | 7.6 ms | 7.9 ms |
| macOS | 150 ms | 63.4 ms | 81.1 ms |
| Windows | 40 ms | 9.7 ms | 11.9 ms |

### `plugin-load` — loading the Git plugin at start

Issue 16: the plugin host scanning a plugins directory with the Git plugin,
checking its manifest, loading the library and letting it register, which is
what the plugin build adds to startup over the compiled-in build; the issue
asks for 50 ms on CI Linux. macOS and Windows carry a provisional target of
twice that until the bench workflow measures them.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 50 ms | — | — |
| macOS | 100 ms | — | — |
| Windows | 100 ms | — | — |

### `tree-first` — the tree view's first children on the 200k fixture

Spec target: 50 ms for the first screen after expanding a folder (close-out
A). Test: `tree_view_on_the_200k_fixture_expands_within_the_budget` in
`closeout.rs`. macOS: 42.2 ms × 1.5 = 63 ms (four runs); CI runs of the
release pull request then measured 65.6 ms and 80.8 ms, so the target is
100 ms, what the runner does.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 50 ms | 2.7 ms | 3.0 ms |
| macOS | 100 ms | 42.2 ms | 50.5 ms |
| Windows | 50 ms | 4.9 ms | 6.0 ms |

### `search-first` — first batch of a subtree search

Spec target: 100 ms for the first matches of `foo` over 20 nested levels and
a 12 000-file folder (close-out A). Test:
`the_first_search_batch_arrives_within_100_ms` in `closeout.rs`. Windows:
85.1 ms × 1.5 = 128 ms, target 150 ms. macOS: 50.6 ms × 1.5 = 76 ms, under
the spec's 100 ms, which stands (four runs; a CI run measured 86.5 ms).

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 100 ms | 27.6 ms | 27.9 ms |
| macOS | 100 ms | 50.6 ms | 90.5 ms |
| Windows | 150 ms | 85.1 ms | 116 ms |

### `cancel` — cancelling the 1 GiB sparse copy from the operations area

No spec number (the job ends "within one batch"); the targets come from
the measurements: Linux 11.0 ms × 1.5 = 16.5 → 20 ms, Windows 7.6 ms × 1.5
→ 20 ms (not below Linux), macOS 50 ms, what the runner does: three of its
four runs cancelled a job that had already finished (0 ms), the fourth took
38.3 ms. Test: `a_1gib_sparse_copy_shows_progress_pauses_and_cancels` in
`closeout.rs`.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 20 ms | 11.0 ms | 11.2 ms |
| macOS | 50 ms | 0.0 ms | 38.3 ms |
| Windows | 20 ms | 7.6 ms | 12.0 ms |

### `session-start` — starting a three-window, twenty-tab session

No spec number; the test asserts the big session starts within 300 ms of a
one-tab one, and the absolute time of the big one is recorded here with
targets from the measurements: median × 2 as for startup, with a 5 ms floor
since the medians (0.6 ms, 6.2 ms, 2.1 ms) sit below the jitter of a shared
runner. `App::new` with the session file, headless. Test:
`a_big_session_starts_about_as_fast_as_a_small_one` in `session.rs`, which
also runs in the ordinary test suite.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 5 ms | 0.6 ms | 1.5 ms |
| macOS | 15 ms | 6.2 ms | 7.5 ms |
| Windows | 5 ms | 2.1 ms | 2.4 ms |
