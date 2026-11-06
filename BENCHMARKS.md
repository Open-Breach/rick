# Benchmarks

The acceptance benchmarks of version 0.1, measured on the GitHub-hosted CI
runners by the `bench` workflow (`.github/workflows/bench.yml`): every
benchmark five times in release on each runner, the median and the worst run
below. Each benchmark is an `#[ignore]`d test in `crates/tests/tests`; CI runs
each once per push on the three runners and asserts the target of that
runner's OS (`crates/tests/src/bench.rs`, whose table a unit test checks
against this file).

**Targets.** Linux: the spec's (Decisions table, QUESTIONS.md 5, 24 and 30).
macOS and Windows: the median × 1.5, rounded up to a round number, and never
below the Linux target, so a faster runner is not held to a tighter budget
than the spec asks. Where a runner cannot meet even that (the Windows fixture
under Defender, QUESTIONS.md 5), the target is what the runner does and the
row says so. Startup has no spec target; its three targets come from the
measurements.

Measured: run PENDING.

## Runners

The hardware line as the runner reports it (GitHub's image name and version,
then the CPU, core count and memory the OS sees).

### Linux — `ubuntu-latest`

PENDING

### macOS — `macos-latest`

PENDING

### Windows — `windows-latest`

PENDING

## Results

### `fixture` — generating the 200 000-entry fixture

Spec target: 30 s (Engineering guidelines → Tests; QUESTIONS.md 5). Test:
`two_hundred_thousand_entries_in_under_thirty_seconds` in `fixture.rs`.
Written to the runner's temp disk.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 30 s | — | — |
| macOS | 30 s | — | — |
| Windows | 30 s | — | — |

### `listing-first` — first batch of the 200 000-entry listing

Spec target: 50 ms (first rows within 100 ms of opening; the provider's half
of it, issue 2). Test:
`listing_200k_entries_first_batch_under_50ms_complete_under_400ms` in
`listing.rs`.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 50 ms | — | — |
| macOS | 50 ms | — | — |
| Windows | 50 ms | — | — |

### `listing-complete` — complete listing of 200 000 entries

Spec target: 400 ms (issue 2). Same test as `listing-first`.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 400 ms | — | — |
| macOS | 400 ms | — | — |
| Windows | 400 ms | — | — |

### `copy-sparse` — copying a 1 GiB sparse file

Spec target: 10 s through the fast path (issue 2). Test:
`copying_a_1gib_sparse_file_uses_the_fast_path` in `copy.rs`.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 10 s | — | — |
| macOS | 10 s | — | — |
| Windows | 10 s | — | — |

### `details-frame` — scrolling 200 000 entries in details view

Spec target: 60 fps, that is a 16.6 ms frame; the 95th percentile of 240
scrolling frames on egui's CPU frame (QUESTIONS.md 24). Test:
`scrolling_the_200k_fixture_in_details_view_holds_60_fps` in `ui.rs`.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 16.6 ms | — | — |
| macOS | 16.6 ms | — | — |
| Windows | 16.6 ms | — | — |

### `image-first` — first frame of the 48 MB bitmap

Spec target: 200 ms (QUESTIONS.md 30). Test:
`a_50_mb_image_and_a_300_page_pdf_show_a_first_frame_within_200_ms_without_blocking_the_ui`
in `preview.rs`.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 200 ms | — | — |
| macOS | 200 ms | — | — |
| Windows | 200 ms | — | — |

### `pdf-first` — first frame of the 300-page PDF

Spec target: 200 ms (QUESTIONS.md 30). Same test as `image-first`.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 200 ms | — | — |
| macOS | 200 ms | — | — |
| Windows | 200 ms | — | — |

### `preview-frame` — worst UI frame while the previews decode

Spec target: 16.6 ms (no UI frame over the budget while a decode runs
elsewhere, QUESTIONS.md 30). Same test as `image-first`.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 16.6 ms | — | — |
| macOS | 16.6 ms | — | — |
| Windows | 16.6 ms | — | — |

### `git-batch-cached` — decorating a visible batch with a cached status

Spec target: 16 ms for a batch of 100 entries of a 10 000-file repository
(issue 6). Test: `decorating_a_visible_batch_of_a_10k_repository_is_fast` in
`git.rs`.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 16 ms | — | — |
| macOS | 16 ms | — | — |
| Windows | 16 ms | — | — |

### `git-batch-pending` — decorating a visible batch without a cached status

Spec target: 16 ms; the batch is answered `Pending` at once (issue 6). Same
test as `git-batch-cached`.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 16 ms | — | — |
| macOS | 16 ms | — | — |
| Windows | 16 ms | — | — |

### `git-complete` — the real decorations of the batch published

Spec target: 500 ms after the uncached request (issue 6). Same test as
`git-batch-cached`.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 500 ms | — | — |
| macOS | 500 ms | — | — |
| Windows | 500 ms | — | — |

### `thumbnails` — the icon view's thumbnails of 40 pictures and PDFs

Spec target: 500 ms (GUI chrome round). Test:
`the_icon_view_thumbnails_arrive_within_500_ms_without_long_frames` in
`chrome.rs`.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 500 ms | — | — |
| macOS | 500 ms | — | — |
| Windows | 500 ms | — | — |

### `thumbnails-frame` — longest UI frame while the thumbnails arrive

Spec target: 16.6 ms. Same test as `thumbnails`.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 16.6 ms | — | — |
| macOS | 16.6 ms | — | — |
| Windows | 16.6 ms | — | — |

### `wrap-frame` — 200 000 wrapped lines in the preview pane

Spec target: 16.6 ms for every frame (preview pane round); the worst of
toggling wrap and three scroll frames. Test:
`two_hundred_thousand_wrapped_lines_keep_every_frame_under_the_budget` in
`preview_pane.rs`.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 16.6 ms | — | — |
| macOS | 16.6 ms | — | — |
| Windows | 16.6 ms | — | — |

### `startup` — startup to the first frame

No spec target; the targets are median × 1.5 on each OS. Headless: the app
built with every built-in extension on a 100-entry folder, then the shell's
frames in the `egui_kittest` harness until it is stable; window and GPU
creation are not included and the platform is the fake. Test:
`startup_to_the_first_frame` in `startup.rs`.

| OS | Target | Median | Worst |
| --- | --- | --- | --- |
| Linux | 1000 ms | — | — |
| macOS | 1000 ms | — | — |
| Windows | 1000 ms | — | — |
