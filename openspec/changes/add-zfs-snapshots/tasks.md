# Tasks

## 1. Spec question and scaffolding

- [x] 1.1 Append a numbered question to QUESTIONS.md: SPEC.md puts ZFS snapshots in `ext-deco-fs` ("Not in 0.2"); this change builds a feature plugin `ext-zfs` now (design Decision 1), stating the assumption; verify the entry follows the file's numbering
- [x] 1.2 Create `crates/ext-zfs` as `cdylib` + `rlib` with `lib.rs` (manifest: id `zfs`, points Provider | Decorator | Action, `Platforms::LINUX`), the `ext::plugin!` entry point, empty `en.ftl`/`de.ftl`, and a one-paragraph README naming its responsibility and that it depends on `ext` and `types` only; verify `cargo build -p ext-zfs`
- [x] 1.3 Wire the crate into the workspace: members and workspace dependency, `zfs` feature on `rick`, dev-dependency of `tests`, `deny.toml` wrapper entries like `ext-svn`, plugin bundling into `plugins/zfs/`; verify `cargo deny check` and that `rick --list-plugins` lists `zfs`

## 2. Command-line layer

- [x] 2.1 Add the `Zfs` runner trait with the real implementation (`std::process`, `LC_ALL=C`, absolute path of `zfs` from PATH, `Unsupported` when missing) and a scripted fake; verify a unit test that the fake replays recorded output
- [x] 2.2 Parse `zfs list -H -p` output for file systems (name, mountpoint, mounted; skip `none`, `legacy`, unmounted) and snapshots (name, creation, used), from outputs recorded on OpenZFS 2.4; verify unit tests including names with spaces and malformed lines (skipped, never a panic)
- [x] 2.3 Classify failures from exit code and `LC_ALL=C` stderr into permission denied, already exists, held/has clones (destroy), and other (verbatim); verify unit tests per class
- [x] 2.4 Implement snapshot-name validation and the `rick-YYYY-MM-DD-HHMMSS` default (design Decision 9); verify unit tests for allowed characters, empty, too long, and the default format

## 3. Dataset cache and detection

- [x] 3.1 Implement `Datasets`: one file-system list, longest mount-point prefix on path components, per-dataset snapshot lists with a 30 s TTL and explicit invalidation (design Decisions 4 and 8); verify unit tests for nested datasets, a path outside every dataset, TTL expiry and invalidation
- [x] 3.2 Treat a missing `zfs` or a failed list as "no datasets", logged once at `debug`; verify a unit test with the fake returning `Unsupported` and one with a non-zero exit (spec: Dataset detection, No ZFS tooling)

## 4. Read-only `zfs` provider

- [x] 4.1 Implement path parsing for `/`, `/<dataset>`, `/<dataset>/@<snapshot>/<rel>` and `/~versions/<path>` (design Decision 3); verify unit tests including datasets with `/`, a path with a second `@` and that the parent of a snapshot is its dataset
- [x] 4.2 List the root (datasets with snapshots) and the dataset level (snapshots newest first, creation time as modified, used as size); verify unit tests against the fake (spec: Snapshot overview)
- [x] 4.3 List, `stat` and `open_read` inside a snapshot from a fixture tree `<mount>/.zfs/snapshot/<s>/…`, streamed in batches with the cancel token, entry ids from `(st_dev, st_ino)`; verify unit tests including hidden files and symlinks
- [x] 4.4 Return empty capabilities and `Unsupported` for every write and for `watch`; verify a unit test per write method and an integration test in `tests` that the context menu offers no rename, trash or new item inside a snapshot and that a confirmed permanent delete fails and changes nothing (spec: Browse a snapshot read-only)
- [x] 4.5 Verify in `tests` that copying a file from a `zfs` location into a live temp folder works through the normal copy job (spec: Copy out of a snapshot)

## 5. Navigation and versions

- [x] 5.1 Add the "ZFS" action group with *Browse snapshots* (newest snapshot containing the folder, `Disabled` with reason otherwise) and *Back to live folder* (nearest existing parent), both returning `ActionOutcome::Navigate`; verify unit tests for each scenario of the spec requirement "Open the snapshots of the current folder"
- [x] 5.2 List `/~versions/<path>`: walk snapshots newest first, keep a version when size or mtime changes, entry path is the snapshot location, display name `<snapshot> — <file name>`, streamed and cancellable (design Decision 5); verify unit tests for collapsed unchanged versions and a file deleted live
- [x] 5.3 Add the *Versions* action for one live or snapshot file, returning `ActionOutcome::Navigate` to its versions location (design Decision 10); verify unit tests for availability and the location it opens
- [x] 5.4 Add the action labels and messages of this group to `en.ftl` and `de.ftl`; verify the i18n test that every key exists in both files

## 6. Restore

- [ ] 6.1 Implement the restore job: live path from snapshot location, one confirmation listing live items to be replaced, trash them through `platform::Trash`, create missing parents, copy files, folders and symlinks with permissions and mtime, cancel token between files; a failed trash skips that item (design Decision 6); verify unit tests with `platform::Fake` for each scenario of the spec requirement "Restore from a snapshot"
- [ ] 6.2 Add the *Restore* action for entries in a snapshot or versions view and its locale strings; verify unit tests for availability and that a declined confirmation changes nothing

## 7. Snapshot management and elevation

- [ ] 7.1 Implement *Create snapshot* (form with name, preview row validation, "include child datasets" checkbox), run as a named job that invalidates the dataset's cache; verify unit tests for default name, invalid name and name taken (spec: Create a snapshot)
- [ ] 7.2 Implement *Rename snapshot* and *Destroy snapshot* (confirmation naming each snapshot and that it cannot be undone; held snapshot reported and kept); verify unit tests per spec scenario
- [ ] 7.3 Implement the permission rule: on a permission failure ask "Run as administrator?", then `Elevate::run_elevated` with the same arguments; `PermissionDenied` and refusal end in one notification, `Unsupported` in the message naming the needed `zfs allow` permissions (design Decision 7); verify unit tests with `Fake::set_elevate_runs` for delegated, accepted, dismissed and no-`pkexec` cases
- [ ] 7.4 Add the management strings to `en.ftl` and `de.ftl`; verify the i18n test

## 8. Info section

- [ ] 8.1 Read the fixed property list through `zfs get -H -p -o property,value,source` into the dataset cache, mapping sources and dropping `-` values and unknown properties with one retry (design Decision 11); verify unit tests on recorded output for an inherited value, an unencrypted dataset and an unknown-property error
- [ ] 8.2 Implement the decorator: `applies_to` from the dataset cache, no columns, `decorate` starting the background read when the cache is missing or stale, and a *ZFS* panel section with dataset, snapshot count, newest snapshot and date, then the properties with their source, or a *Loading…* row before the read arrives; verify unit tests for an entry inside and outside a dataset and before and after the read (spec: Dataset info section, Dataset properties shown)
- [ ] 8.3 Add the section's labels and source words to `en.ftl` and `de.ftl`; verify the i18n test

## 9. Integration and release notes

- [ ] 9.1 Add the opt-in real-pool test (`RICK_REQUIRE_ZFS=1`, dataset from `RICK_ZFS_TEST_DATASET`, skipped with a note otherwise) that creates, lists, browses, restores from, renames and destroys its own snapshot; verify it passes by hand on a ZFS machine and is skipped in CI
- [ ] 9.2 Add the plugin to CHANGELOG.md under the unreleased version; verify the entry names the plugin and its Linux-only scope
- [ ] 9.3 Run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --workspace`, `cargo deny check`, `cargo doc --no-deps`; verify all pass, and launch Rick on the ZFS machine to browse a snapshot, open a file's versions and restore a file by hand

## Workflow follow-up

- Open the pull request against `dev` (not `main`) with what was done, what was assumed (QUESTIONS.md entry from 1.1), which tests prove it, and what was left out (slider, rollback, holds, clones, send/receive, scheduling).
- After the maintainer's decision on the QUESTIONS.md entry and the merge, archive the change with `/opsx:archive` so `zfs-snapshots` becomes a living spec.
- Follow-up changes: `add-zfs-time-slider` (the timeline bar under the toolbar) and `add-zfs-property-editor` (editing selected properties), both building on this one.
