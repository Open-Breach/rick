# OpenSpec in Rick

OpenSpec adds a propose / apply / archive workflow. `SPEC.md` stays the
authoritative specification; `openspec/specs/` holds living specs for
capabilities introduced through changes. On conflict, `SPEC.md` wins until
the maintainer changes it.

## Pinned CLI version

Always run the CLI pinned, never `latest`:

    npx @fission-ai/openspec@1.15.0 <command>

Current pin: **1.15.0**. Why: it was the newest release that had been public
for more than two weeks when adopted (2026-11-09), so it has had time to
surface problems. Newer releases (1.16.0, 1.16.1) were deliberately skipped.

Bumping: change the version here and in `openspec/config.yaml` in a commit of
its own, run `openspec update`, and record below why the bump was made.

| Date | From | To | Reason |
| --- | --- | --- | --- |
| 2026-11-09 | — | 1.15.0 | Initial adoption. |

## What needs a change proposal

Proposal (`/opsx:propose`) for structural or pipeline changes:

- a new crate or extension (e.g. ZFS support),
- a new or changed `Command`, `Event`, trait or other extension point,
- an architecture, layering or dependency change,
- behaviour that changes what SPEC.md or an existing OpenSpec spec states.

No proposal; track in `TODO.md` or a GitHub issue:

- bug fixes, locale/translation work, icons and assets, docs,
- tuning and dependency bumps, small UI polish within existing behaviour.

If unsure, ask the maintainer.
