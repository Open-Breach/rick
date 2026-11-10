# Tasks

## 1. Preconditions

- [x] 1.1 Confirm `add-zfs-snapshots` is merged (applied on its branch instead, in the same pull request #156) (runner, cache with properties, elevation rule); verify `ext-zfs` builds on `dev` and its tests pass

## 2. Values and validation

- [x] 2.1 Implement the size parser (`none`, integer and decimal with K/M/G/T/P, optional `B`/`iB`, powers of 1024); verify unit tests for each unit, case, decimals, and rejects such as `fifty`, `-1G`, `1X` (spec: Size values)
- [x] 2.2 Implement the allow-list with each property's options, *Inherit* label from the parent's value, and the current value added when not standard (design Decision 2); verify unit tests for `zstd-3`, `256K`, and a pool root dataset without a parent
- [x] 2.3 Implement the preview rows: unchanged → no row, `old (source) → new`, quota below `used` / refquota below `referenced` as problems, the "nothing changed" row, and the "data written from now on" note; verify unit tests for each scenario of the spec requirement "Preview and validation"

## 3. Action and form

- [x] 3.1 Add *Edit ZFS properties…* to the ZFS group with availability on live folders in a dataset and dataset folders at the `zfs` root, hidden elsewhere; verify unit tests (spec: Where the editor is offered)
- [x] 3.2 Read fresh values in a job and open the form through `ActionOutcome::Later`; verify a test with the scripted runner that the form opens with the current values and the excluded properties are absent (spec: Editable properties and values)
- [x] 3.3 Add the action, field labels, options, note and messages to `en.ftl` and `de.ftl`; verify the i18n test

## 4. Apply

- [x] 4.1 Build the commands: one `zfs set` for explicit values, one `zfs inherit` per reset, then drop and re-read the cache entry; stop at the first failure and report the property and reason; verify unit tests for set + inherit and a partial failure (spec: Applying changes)
- [x] 4.2 Apply the permission rule: one question on the first permission failure, the failed and remaining commands through `Elevate`, dismissed prompt stops, no `pkexec` → delegation message with the property list; verify unit tests with `Fake::set_elevate_runs` (spec: Permission rule for property changes)

## 5. Integration

- [ ] 5.1 Extend the opt-in real-pool test (`RICK_REQUIRE_ZFS=1`) to set `compression` and `quota` on its test dataset, check the values, and inherit them back; verify by hand on a ZFS machine
- [x] 5.2 Run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --workspace`, `cargo deny check`, `cargo doc --no-deps`; verify all pass, and add the editor to CHANGELOG.md

## Workflow follow-up

- Open the pull request against `dev`, listing the excluded properties and the no-undo decision as deliberately left out.
- After merge, archive with `/opsx:archive` so `zfs-dataset-properties` becomes a living spec.
