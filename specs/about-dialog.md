# Rick — About dialog

## Purpose

The About dialog tells the user what they are running, where it came from and who made it, and satisfies the attribution duties of the licences Rick ships under: Apache-2.0 and MIT notices for the crates, the BSD notice for pdfium, the LGPL notices for GStreamer and 7-Zip, the font licences for the shipped fonts.

Design reference: Figma, section "About dialog". This text is binding where it and the frames disagree on behaviour; the frames are binding for layout, spacing and colour.

## About page

- Opened from the app menu, About Rick. A modal dialog in the settings window's chrome: `dialog/bg`, 14 px radius, close button top right; 520 pt wide, height from content.
- Hero, centred: the app symbol (`assets/icons/svg/rick-symbol.svg`) at 96 pt; "Rick" 26 pt medium; tagline "a god-like file manager" 14 pt in `text/secondary`; the version in a pill on `bg/selected-soft`; one monospace build line, 12 pt in `text/secondary`: `<short commit> · <OS> <arch> · plugin ABI <n> · built <YYYY-MM-DD>`. Commit, target and date are build-time environment values from `crates/rick/build.rs`; the ABI number is `ext::API_VERSION`. Nothing is read from git at runtime.
- Buttons under the hero: Copy details and What's new.
  - Copy details copies plain text: name and version; the build line; the toolchain (`RICK_TOOLCHAIN`); the OS and desktop (Linux: distribution and `XDG_CURRENT_DESKTOP`; macOS and Windows: the OS version); the loaded plugins, one `id version status` line each. `rick --version` prints the same text.
  - What's new shows the running version's section of `CHANGELOG.md` (embedded at build time) in a read-only text view inside the dialog; a `-dev` build shows the Unreleased section.
- Link card: Website, Source code, Report an issue. Each row opens the system browser. Report an issue opens the GitHub new-issue URL with the Copy-details text in the issue body.
- Licence card: Licence — "Apache-2.0", opens the embedded `LICENSE` text in the read-only view; Third-party licences — the component count as trailing value, opens the second page; Loaded plugins — "N plugins · M third-party", opens the Extensions screen. A plugin is third-party when it was loaded from outside the release's `plugins/` folder.
- Credits card: fixed rows first — Serge, "Specification, design and direction"; Claude (Anthropic), "Implementation through Claude Code, design and spec assistance" — then the GitHub contributors appended at build time (`git shortlog -sn --no-merges`, empty outside a checkout), each with the role from `CONTRIBUTORS.md` when a line matches the name, otherwise no role line. Avatars are the initials in a circle on `bg/control`; the dialog never loads images from the network.
- Footer: "Built with egui and eframe, wgpu, pdfium, GStreamer and Phosphor icons. Written largely by an AI coding agent from a human-written specification — use with care." in `text/secondary`, and "© 2026 Open-Breach contributors" in `text/placeholder`.

## Third-party licences page

- Reached from the Licence card; the header has a back button (returns to the About page), the title, and the close button. 520 × 640 pt, the list scrolls.
- Tools row: a search field filtering by component name, and a dropdown filtering by licence type (All licences plus one entry per licence id in the data).
- Sections: Bundled runtimes first — everything shipped outside Cargo: pdfium, GStreamer (bundled in the macOS and Windows archives), 7-Zip, the shipped fonts — listed by hand in `about/bundled.toml` with name, version, licence id, copyright, URL and licence text; then Rust crates, alphabetical, from the generated file.
- Row: name (medium), version (mono, `text/secondary`), licence pill (`trash/low-bg` / `trash/low-text`), chevron. Clicking a row expands it in place on `bg/selected-soft`: copyright holder, repository link, and the full licence text in a mono box on `bg/card`. One row is open at a time; clicking the open row closes it.
- Footer: "Generated at build time from Cargo.lock by cargo-about · N components, M licence types", and Export…, which writes the whole list — bundled runtimes and crates, each with its licence text — as one plain-text file through the save dialog.
- Plugins loaded from outside `plugins/` contribute their own entries from a `licenses.json` beside the plugin file (same shape as the generated file), shown under the plugin's name as a third section; no file, no section.

## Generation

- `cargo about generate` with the existing `about.toml` runs twice in the `licenses` job of `release.yml`: `about.hbs` for `THIRD-PARTY-LICENSES.md` as today, and `about-json.hbs` for `third-party.json`, an array of `{name, version, license, copyright, repository, text}`.
- The bundle puts `third-party.json` beside the binary (macOS: `Contents/Resources/`). The dialog reads it once at startup.
- A build without the file (`cargo run`, a dev build) shows the Third-party licences row as "not available in this build" and no count; the Bundled runtimes section still shows.
- The release workflow fails when the file is missing or when the set of crates in it differs from `Cargo.lock`.
- Build-time values: `RICK_GIT_HASH` (exists), `RICK_BUILD_DATE` (UTC date), `RICK_TARGET` (target triple), `RICK_CONTRIBUTORS` (newline-separated names) from `crates/rick/build.rs`; `RICK_TOOLCHAIN` (exists) from `crates/plugins-native/build.rs`.

## Strings

All visible text goes through `locales/en.ftl` and `locales/de.ftl`. Licence texts, crate names and the copyright lines are data and are not translated.

## Out of scope

Licence text translation, a donate link, update checks (Version 0.3), signing and notarisation.
