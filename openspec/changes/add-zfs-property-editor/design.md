# Design

## Context

See proposal.md for the motivation. This change lives entirely in `ext-zfs` and reuses what `add-zfs-snapshots` builds: the `Zfs` runner with `LC_ALL=C`, failure classification, the dataset cache with properties and sources (its Decision 11), and the "own rights, then elevate" rule (its Decision 7). `ext` already has what the UI needs: `Form` with `Text` and `Choice` fields, a live `Preview` of `PreviewRow { from, to, problem }` that disables OK while a row has a problem, and `ActionOutcome::Later` with `UiHost::form_later` for a form whose content a job reads first.

## Goals / Non-Goals

**Goals:**
- An editor built only from the existing form and preview contract; no kernel change.
- A fixed allow-list of properties; nothing outside it can be set through Rick.

**Non-Goals:**
- No generic property grid, no free-form `name=value` entry.
- No undo (Decision 5).

## Decisions

### 1. A form opened through `ActionOutcome::Later`
The action spawns a job that reads `zfs get -H -p -o property,value,source` for the allow-list plus `used` and `referenced` (fresh, not from the cache, because the user is about to change them), then hands over the form. *Alternative:* build the form from the cache synchronously — a value up to 30 s old as the "old" value in the preview would be misleading.

### 2. Choice fields with *Inherit* first, the current value always present
Each property is a `Choice`: `Inherit (<inherited value>)` first, then the standard values, plus the current value when it is not among them (`zstd-3`, `gzip-9`, `256K`). The quotas are `Text` fields (`none` or a size). The inherited value comes from `zfs get -s inherited,default` on the parent dataset (or the default for a pool's root dataset). *Alternative:* text fields everywhere — free text for enumerated values invites typos that only `zfs set` would catch.

### 3. Validation in the preview, sizes parsed by the plugin
Rows are computed on every change: unchanged properties produce no row; a quota that does not parse, or that is below `used` (`quota`) or `referenced` (`refquota`), gets a problem; no rows at all disables OK through an explanatory row. The size parser accepts `none`, integers and decimals with K/M/G/T/P and optional `B`/`iB`, as powers of 1024, matching how `zfs` reads them. The note about "data written from now on" is an extra preview row without a problem, shown only when `compression`, `copies` or `recordsize` changed.

### 4. Apply as at most 1 + n commands
One `zfs set p1=v1 p2=v2 … <dataset>` for all explicit values (ZFS applies a multi-property set atomically per dataset), then one `zfs inherit <p> <dataset>` per property reset to *Inherit* (the command takes one property). Then the dataset's cache entry is dropped and re-read so the info section updates. On a permission failure the user is asked once; on yes, the failed command and every remaining one run through `Elevate::run_elevated`, which may show one prompt per command — polkit's `org.freedesktop.policykit.exec` does not keep authorisation. *Since the review of #156:* what runs elevated is the helper `rick-zfs` of `add-zfs-snapshots`, which accepts `set` and `inherit` for the seven properties of Decision 2 only, each value checked as the editor checks it. *Alternative:* one elevated shell script — a shell under `pkexec` with composed arguments is exactly what the elevation rule avoids.

### 5. No undo
`UndoCommand::undo` gets no `UiHost`, so an undo that needs elevation could not ask; an undo that silently fails for half the users is worse than none. The preview shows old values before applying, and the info section shows the result after.

### 6. Delegation message
When elevation is unavailable, the message lists the changed properties as the `zfs allow` permission set (`zfs allow <user> compression,quota <dataset>`), with the user name from the environment, so it can be pasted into a root shell.

## Risks / Trade-offs

- [A quota just above `used` blocks writes soon after] → only "below used" is refused; near-full quotas are the user's call; the preview shows both numbers.
- [Several polkit prompts for one apply] → at most 1 + number of properties reset to inherit; usually one.
- [`compression` values differ between OpenZFS versions] → only values every 2.x knows are offered; the current value is always kept selectable; a value the system rejects fails with ZFS's message.
- [`recordsize` above 1M needs a module parameter] → the list stops at 1M; a larger current value is still shown.

## Migration Plan

None: a new action in an existing plugin; disabling the plugin removes it.
