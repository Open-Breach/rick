# Design

## Context

See proposal.md for the motivation and the conflict with SPEC.md's `ext-deco-fs`. Relevant current state:

- Plugins are `cdylib` + `rlib` crates that depend on `ext` and `types` only, register through one `Extension::register` and are loaded by `plugins-native` from `plugins/<name>/`. `ext-vfs-trash` (own provider + decorator + action group, uses `Registry::platform()`) and `ext-svn` (decorator + actions over a command line, run through `ext::blocking` inside named jobs) are the two closest models.
- `vfs-local` reports the same full capability set (`WRITE`, `TRASH`, `RENAME`, …) for every location; it does not know that `.zfs/snapshot/*` is read-only.
- `platform::Elevate::run_elevated(program, args)` exists (issue 44/24): `pkexec` on Linux, `PermissionDenied` when the prompt is dismissed, `Unsupported` without `pkexec`. `platform::Trash` and a `platform::Fake` with `set_elevate_runs` exist for tests.
- Actions can return `ActionOutcome::Navigate` (take the pane to a location); `ActionOutcome::View` opens only a comparison or a console tab, not a location; `UiHost` offers `confirm`, `prompt` and `form` with a live preview row that can disable the OK button.
- The architecture forbids `cfg(target_os)` outside `platform`; the plugin restricts itself through `Manifest::platforms = Platforms::LINUX`.
- This machine runs ZFS on root, so the design can be checked against a real pool by hand; CI has no ZFS.

## Goals / Non-Goals

**Goals:**
- Everything ZFS-specific lives in `ext-zfs`; no kernel crate changes.
- Snapshots behave like folders for every existing feature that only reads (tabs, history, preview, copy, search walk).
- The command line is the only way the plugin talks to ZFS, behind a seam that tests can replace.

**Non-Goals:**
- No new `platform` trait; no `libzfs` binding; no changes to `vfs-local` capabilities.
- No undo for snapshot management or restore (see Decisions 6 and 7).
- No slider or other new GUI element; everything uses existing views, actions, forms and info badges.

## Decisions

### 1. A feature plugin `ext-zfs`, not `ext-deco-fs`
The feature fills Provider, Decorator and Action, which SPEC.md's naming rule names after the feature (`ext-git`), and Btrfs and NTFS streams share nothing with it but the word "file system". *Alternative:* `ext-deco-fs` as SPEC.md lists it — one plugin for three unrelated features, enabled and disabled as one. Recorded as a numbered question; if the maintainer prefers `ext-deco-fs`, only the crate and plugin name change.

### 2. The `zfs` command line with `-H -p`, behind a `Zfs` runner trait
`zfs list -H -p -o name,mountpoint,mounted -t filesystem` and `zfs list -H -p -t snapshot -o name,creation,used -s creation -d 1 <dataset>` give tab-separated, unit-free output that is stable across OpenZFS 2.x; management is `zfs snapshot [-r]`, `zfs rename`, `zfs destroy`. Every call sets `LC_ALL=C` so error messages can be classified. The runner trait (`run(args) -> Output`) has the real implementation through `std::process` and a scripted fake for tests. *Alternative:* `libzfs_core` over FFI — no stable ABI across distributions, `unsafe` would have to move into `platform`, and the gain (no process spawn) does not matter at these call rates.

### 3. A read-only provider `zfs` instead of browsing `.zfs/snapshot` through `vfs-local`
Locations of provider `zfs`:

| Path | Lists |
| --- | --- |
| `/` | datasets that have snapshots, one folder each |
| `/<dataset>` (dataset name with its `/`s) | that dataset's snapshots, newest first |
| `/<dataset>/@<snapshot>/<rel>` | the tree under `<mountpoint>/.zfs/snapshot/<snapshot>/<rel>` |
| `/~versions/<live absolute path>` | the versions of one file (Decision 5) |

The snapshot is its own path component starting with `@`, the first such component (dataset names cannot contain `@`), so the parent of `/<dataset>/@<snapshot>` is the dataset's snapshot list and the breadcrumb and *Up* lead there; `~` cannot start a pool name, so `~versions` cannot clash. The provider reads with `std::fs` inside `ext::blocking`, returns an empty `Capabilities` set and `Unsupported` for every write; `watch` is `Unsupported` (snapshots never change; the overview levels are refreshed by Decision 8). Entry ids hash `(st_dev, st_ino)` of the snapshot file, which ZFS keeps distinct per snapshot. *Alternative:* navigate the local provider into `.zfs/snapshot/…` — zero provider code, but every write command would be offered and fail with `EROFS`, the breadcrumb would show `.zfs/snapshot`, and Restore would have to recognise snapshot paths by string. Copying out works through `vfs`'s cross-provider path (`open_read` → local `open_write`) with no extra code.

### 4. Dataset detection by longest mount-point prefix, cached
A `Datasets` cache holds the mounted file systems from one `zfs list` call (skipping `mountpoint` `none` and `legacy`, and `mounted` = `no`) and maps a live path to its dataset by the longest mount-point prefix on path components. `applies_to` and `availability` only read the cache; an empty or failed list (no `zfs`, exit ≠ 0) means "nowhere", logged once at `debug`. *Alternative:* the platform mount table (`fs_type == "zfs"`, device = dataset name) — it already filters bind mounts of the system disk, which would hide nested datasets on root-on-ZFS systems like the development machine.

### 5. Versions: walk the snapshots, keep a version when size or mtime changes
For a live path in dataset D with relative path `rel`, `stat` `.zfs/snapshot/<s>/<rel>` for every snapshot `s` newest first, and emit an entry when the file exists and `(size, mtime)` differs from the last emitted one. Each entry's `path` is its snapshot location (`/<D>/@<s>/<rel>`) so preview, copy and Restore use the normal snapshot path; its display name is `<snapshot> — <file name>`. The listing is streamed in batches and honours the cancel token, because each `stat` may auto-mount a snapshot. *Alternative:* `zfs diff` — needs `diff` permission, is slow on large datasets and answers a different question (what changed between two snapshots).

### 6. Restore copies inside the plugin and trashes what it replaces
The plugin cannot use `ops` (upward dependency), so Restore is its own job: for each selected snapshot entry compute the live path (mount point + `rel`), ask once for confirmation listing the live items that will be replaced, move those to the trash through `platform::Trash`, create missing parents, then copy recursively with `std::fs` (regular files, directories, symlinks as symlinks; permissions and modification time kept), checking the cancel token between files. A failed trash skips that item and reports it. Not undoable: the replaced version is in the trash and the restored one still in the snapshot. *Alternative:* emit a `Transfer` into the live folder — `Command` is not reachable from an `Action`, and it would apply the general conflict prompt instead of "trash the live version".

### 7. Management: own rights first, then offer elevation once
Each create/rename/destroy runs `zfs …` as the user. If the exit code is non-zero and stderr (in `LC_ALL=C`) matches `permission denied` or `insufficient privileges`, the action asks "Run as administrator?" and, on yes, calls `Elevate::run_elevated(<absolute zfs path>, same args)`. `PermissionDenied` from the prompt and a second refusal both end in the same notification; `Unsupported` (no `pkexec`) ends in the message naming the `zfs allow` permission (`snapshot`, `rename` + `mount` + `create`, `destroy` + `mount`). Arguments are built only from the cached dataset name and a validated snapshot name (Decision 9), never from free text, which is what makes running the system `zfs` binary elevated acceptable instead of a dedicated helper. Snapshot management is not undoable: destroy cannot be reversed, and offering undo for create alone would be inconsistent.

### 8. Freshness: invalidate on own actions, TTL of 30 s otherwise
Snapshot lists are cached per dataset with their read time; a listing of a `zfs` location or a decorator panel request re-reads a list older than 30 s, and every management job drops its dataset's entry when it finishes. Nothing watches `.zfs/snapshot` (it delivers no inotify events). The decorator's `invalidates_on` returns nothing.

### 9. Snapshot name validation in the form preview
Allowed: ASCII letters, digits, `_ - . :` and space, 1–255 bytes for `dataset@name` together; the form's preview row marks an invalid or empty name and disables OK. The default is `rick-YYYY-MM-DD-HHMMSS` in local time.

### 10. Actions and where they appear
One group `ActionGroup::Extension("ZFS")`: *Browse snapshots* and *Create snapshot* (live location in a dataset), *Back to live folder* and *Restore* (inside a snapshot or versions view), *Versions* (one live file or one snapshot file selected), *Rename snapshot* and *Destroy snapshot* (snapshot folders at the dataset level of the `zfs` provider). `availability` is `Hidden` outside those places, `Disabled(reason)` for the "no snapshot contains this folder" case. *Browse snapshots*, *Back to live folder* and *Versions* return `ActionOutcome::Navigate`: no action can open a location in a new tab without a kernel change, and the pane's history leads back.

### 11. Dataset properties in the info section, read through one cached `zfs get`
`zfs get -H -p -o property,value,source <list> <dataset>` with the fixed property list of the spec is read together with the snapshot list and cached under the same TTL (Decision 8). `Decorator::panel` must return at once, so it only reads the cache: `decorate` for a visible batch starts the background read when the dataset's entry is missing or stale, and a panel asked for before it arrives shows the dataset name and a *Loading…* row. Sources are mapped from ZFS's words (`local`, `inherited from <ds>`, `default`, `received`); `-` values for properties that do not apply (`keystatus` without encryption) and properties an older ZFS rejects as unknown are dropped, so one failing property never empties the section (on an unknown-property error the read is retried once without it). Sizes come as bytes (`-p`) and are formatted with the kernel's size formatter. *Alternative:* a separate *ZFS properties* dialogue — more UI for read-only data the info badge already has room for; the editor in change `add-zfs-property-editor` is where a dialogue earns its place.

### 12. Testing
Unit tests use the scripted runner (outputs recorded from OpenZFS 2.4 on the development machine) and a fixture tree that mimics `<mountpoint>/.zfs/snapshot/<s>/…` in a temp dir; the provider only reads paths, so no pool is needed. Elevation is tested through `platform::Fake::set_elevate_runs`. An opt-in test (`RICK_REQUIRE_ZFS=1`, skipped with a note otherwise, as `ext-svn` does) runs against a real pool given by `RICK_ZFS_TEST_DATASET`, creating and destroying its own snapshots there.

## Risks / Trade-offs

- [Auto-mount latency: the first access to a snapshot's directory mounts it, ~10–100 ms each] → versions and snapshot listings run as streamed, cancellable jobs; the dataset-level list uses `zfs list` and does not touch the directories.
- [Many snapshots (thousands, from auto-snapshot tools)] → lists stream in batches; versions collapse unchanged ones; no per-snapshot work happens for the info section beyond one cached `zfs list`.
- [Running the system `zfs` elevated with arguments from Rick] → arguments come only from cached dataset names and validated snapshot names; each command is one prompt for one action; no shell is involved.
- [Error classification by English stderr text] → `LC_ALL=C`; an unrecognised failure is reported verbatim and never offered elevation.
- [Restore copies with `std::fs` instead of `ops`, so it does not get reflink or the operations-area conflict policy] → acceptable for a restore; snapshot blocks are not reflinkable into the live dataset by `copy_file_range` before OpenZFS 2.2 anyway.
- [SPEC.md conflict] → the question is recorded first; the change can be renamed or deferred without touching other crates.

## Migration Plan

New plugin, off unless installed: it builds into `plugins/zfs/` with the workspace build and is listed in the Extensions screen; disabling it removes every trace. No data or settings migration. Rollback is disabling or removing the plugin.

## Open Questions

- Whether the `zfs` provider's root should also appear in the side panel's This Machine section; this change only reaches it through the actions, and adding a row later needs no change to the provider.
