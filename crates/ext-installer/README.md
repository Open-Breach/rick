# ext-installer

`ext-installer` is the tarball installer of `specs/ext-installer.md`: the
action *Install as application…* on a tar (gz, xz, bz2, zst) or zip archive
reads the archive's index and the headers of its candidate files to guess
whether it is a prebuilt GUI application or a command-line toolchain, its
main executable, icon, name and version, then shows the install form with
every guess editable and a summary of what will be written; the install
runs as a job that extracts into a staging folder beside the destination
under the archive-safety rules (no path escapes, no outside-pointing links,
no special entries, set-uid stripped, nothing executed), verifies the main
executable's ELF machine type against the running machine, renames the
staging folder into `~/.local/opt/<app-id>/<version>/`, writes
`rick-install.toml` beside it, flips the `current` symlink atomically and
writes the desktop entry, the icon into the hicolor theme and the command
symlinks in `~/.local/bin` (or the PATH-mode file `~/.config/rick/path.sh`
for toolchains), all pointing through `current`. An archive that matches an
installed app opens the same form in update mode; one previous version is
kept and *Revert* flips back to it; *Uninstall* removes exactly the files
the record lists and the app folder; *Repair* rewrites what a messy state
is missing. The Installed Apps view, `rick://apps`, lists every record with
its version, kind, scope, state and commands, browsable into the install
folder, with the per-row actions. System-wide installs are shown as
unavailable until the privileged helper of the spec's own milestone exists.
The plugin loads on every platform, and its actions are hidden and its view
answers "unsupported on this platform" anywhere but Linux. It depends on
`ext` and `types` only (the tar and zip crates are its own); `rick` (behind
`installer`) and `tests` link it, and the plugin host loads it.
