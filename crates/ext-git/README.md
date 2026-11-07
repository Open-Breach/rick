# ext-git

`ext-git` is the Git extension, one manifest with three contributions: a
Decorator that shows status badges on entries and the branch with its
ahead/behind counts, read through gix and cached per work tree until the
`.git` directory or the work tree changes; the "Git" action group (Commit,
Stash, Stash pop, Pull, Push, Checkout branch, Discard changes, Show log,
Blame), available only inside a work tree and run through the `git`
command line; and an info panel section with the branch and the last commit.
It is the reference plugin: the crate builds as a `cdylib` that
`plugins-native` loads from `plugins/git/` through the entry point
`ext::plugin!` declares (`crates/plugins-native/README.md` walks through the
pattern), and as an `rlib` for the binary's opt-in `git` feature and the
tests. It depends on `ext` and `types` only; `rick` (behind `git`) and
`tests` link it, and the plugin host loads it.
