# Engineering

Related: specs/architecture.md

## Engineering guidelines

This spec is written to be implemented by an AI coding agent as well as by people. The rules below are meant to be applied literally.

- **Toolchain.** Rust 2024 edition on the stable toolchain, pinned in `rust-toolchain.toml`. `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` with the pedantic group enabled must pass before any change is considered done.
- **Unsafe.** No `unsafe` outside the `platform` crate. Every `unsafe` block carries a `// SAFETY:` comment stating the invariant it relies on.
- **Errors.** Library crates define their error types with `thiserror`; only the `rick` binary uses `anyhow`. Never panic on user data, file content or filesystem state: `unwrap` and `expect` are allowed only where the invariant is provable and commented.
- **Crate discipline.** Every crate has a `README.md` of one paragraph stating its responsibility and the crates it may depend on. The layer order in the architecture section is enforced with `cargo-deny` bans and checked in CI; a dependency that would point upward is a design error, not a shortcut.
- **Logging.** `tracing` everywhere, `println!` and `eprintln!` nowhere outside tests. Spans around every filesystem operation so the performance targets can be measured.
- **Async and threads.** One `tokio` runtime owned by `runtime`; the UI thread never blocks on I/O. CPU-heavy work (sorting, hashing, decoding) goes through `rayon` or a dedicated pool, never on the runtime's worker threads.
- **Tests.** Unit tests next to the code; integration tests in a `tests` crate with a generated 200k-entry fixture that the performance targets are benchmarked against. A bug fix adds a test that fails before the fix.
- **Documentation.** Public items have doc comments; `cargo doc` builds without warnings. Each extension point has a worked example extension in its crate's docs.
- **Commits.** Small, one concern each, conventional-commit messages (`feat(vfs): …`, `fix(ui): …`). Each commit builds and passes tests on its own.
- **The spec wins.** When the code and this document disagree, the document is right until it is changed. Ambiguities or contradictions in the spec are raised as questions in the pull request, and never resolved silently in code.
- **Scope.** Implement what the spec says, nothing more. No speculative abstractions, no extra features, no configuration options the spec does not name.

## Non-goals & open questions

Rick does not implement file systems, mount network shares itself, or bundle a browser engine. Rick builds no file search index of its own. Where the platform has one (Windows Search, Spotlight, Tracker or Baloo on Linux) a search box appears on the toolbar and queries it; where none is present, there is no search box and only the live subtree walk described under the GUI. The items below need a call before they enter the plan.

All open questions from the first draft are settled (2026-11-05): iSCSI out of scope, RAR extract-only, LibreOffice used when installed and never bundled, TFVC deferred to a plugin, tar and zip read separately, GStreamer on every platform, and "supported file system" means correct basic behaviour in the core — file-system-specific features such as Btrfs subvolumes and ZFS snapshots come later as Decorator extensions. New questions go here as they come up.

**Configuration.** Settings are plain TOML files the user can read, edit and version: a main `rick.toml` plus one file per extension. They live in the platform's standard location — `$XDG_CONFIG_HOME/rick` (usually `~/.config/rick`) on Linux, `~/Library/Application Support/Rick` on macOS, `%APPDATA%\Rick` on Windows. The Settings screen writes the same files; the app watches them and reloads on change.

**License.** Rick is open source under Apache-2.0: permissive, with an explicit patent grant and contributor protection, and the usual choice in the Rust ecosystem, so plugins and embedders face no friction. Every crate in the workspace carries the same license; a `CONTRIBUTING` note states that contributions are accepted under it.

**Language.** English only for the first releases. User-facing strings still go through one `t!` lookup function from the start, so adding languages later is a translation file, not a code change.
