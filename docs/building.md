# Building Rick from source

```bash
git clone https://github.com/Open-Breach/rick.git
cd rick
cargo xtask bundle
./target/debug/bundle/rick
```

You need the stable Rust toolchain (`rust-toolchain.toml` pins it) and, on Linux, the usual Wayland/X11 development packages for an egui app.

`cargo xtask bundle` builds the workspace and lays out the binary, the shared libraries it links and every plugin in `target/debug/bundle/` (`--release` for `target/release/bundle/`). A release archive is that tree without the separate plugins (see below). Build and test the workspace as a whole (`cargo test --workspace`), since the kernel crates are shared libraries built once per feature set (QUESTIONS.md 81).

## Libraries a preview needs

- **PDF.** PDF previews need pdfium beside the binary, or wherever the PDF previewer's `library` setting points (`preview-pdf.toml` in the config directory, which Rick fills in itself when it finds the library and which the preview pane's *Browse…* sets). The bundle copies the library from that setting when it is set; a release archive brings it along.
- **Audio and video.** Media playback is built only with `--features ext-preview-media/gstreamer` (`cargo test --workspace --features ext-preview-media/gstreamer`, `cargo xtask bundle --features ext-preview-media/gstreamer`), which needs the GStreamer development files (`libgstreamer1.0-dev` and `libgstreamer-plugins-base1.0-dev` on Debian and Ubuntu, the official development package on macOS and Windows); without the feature the media previewer shows metadata only. `RICK_GSTREAMER_DIR` makes the bundle carry a GStreamer runtime on macOS and Windows, in the media plugin's folder `plugins/preview-media/`, where the plugin finds it wherever that folder lies (`crates/plugins-native/README.md`, *Where it goes*). On Windows a bundle built with the feature needs it: Rick looks for libraries beside the binary, in the folder of the plugin that links them and in the system's folder, not on the `PATH`, so a GStreamer that is installed and on the `PATH` is not found from there. On Linux Rick uses the system GStreamer: install `gstreamer1.0-plugins-good` and `gstreamer1.0-libav`, or your distribution's equivalents, for MP3 and H.264.
- **Office documents.** Office previews need LibreOffice installed; without it the preview shows the document's metadata and says so.

## What a release archive holds

Release archives are built for Linux (x64 and arm64), macOS (Apple silicon) and Windows (x64). Each holds:

- the `rick` binary and, beside it, the shared libraries it needs: Rick's kernel and the Rust standard library, which the plugins share with it, and the pdfium library;
- a `plugins/` folder with one folder per plugin of the **default set**: the trash and archive providers (the 7-Zip library for disk images beside the archive plugin, with its licence), the archive, markup, hex and icon previewers, the subdirectory search and the platform's search-index connector (Windows Search, Spotlight, or Tracker and Baloo on Linux), the compress, compare, console and bulk-rename actions and the tarball installer, and on Linux the media previewer, which uses the system GStreamer;
- on Windows, the Visual C++ runtime beside `rick.exe`;
- on Linux, the desktop entry and the icons in the XDG layout under `share/`; on macOS all of it inside `Rick.app`;
- the license, the README, the changelog and the third-party licenses: `THIRD-PARTY-LICENSES.md` lists the crates and, after them, what ships outside Cargo (pdfium, 7-Zip, GStreamer, the Rust standard library, the fonts and the icon font);
- a `licenses/` folder with the texts two of those bring with them: `licenses/pdfium/`, pdfium's licence and those of the libraries built into it, and `licenses/rust/COPYRIGHT-library.html`, the notices of the Rust standard library and of the crates it is built from.

Every file on the release page has a GitHub build attestation: a record, signed by GitHub, of the workflow that built the file and the commit it was built from. The Linux files, the packages and archives and the Linux plugin packages, are built and attested by Trebuchet's workflow; the macOS and Windows files and `SHA256SUMS.txt` by Rick's release workflow. To check a file with the GitHub CLI:

```bash
gh attestation verify rick_0.6.0_debian-13_amd64.deb --owner Open-Breach --signer-repo Open-Breach/trebuchet   # a Linux file
gh attestation verify rick-0.6.0-macos-arm64.zip --repo Open-Breach/rick                                      # a macOS or Windows file
```

The release workflow runs both lines itself, on a Linux package and on the macOS and the Windows archive, and checks that a file changed by one byte does not verify. It does so while the release is still a draft, and publishes only when that check has passed: where it fails, the draft stays a draft and nothing is public. An attestation proves where a file comes from, not that it is safe.

`SHA256SUMS.txt` on the release page has the checksums of the archives, the packages and the lists below. The Linux archives need glibc 2.36 or later (Debian 12, Ubuntu 24.04): they are built on the oldest distribution release the packages are made for. Each deb and rpm is built on its own release. Intel Macs are not covered yet.

The archives are not signed or notarised yet: macOS may refuse an app downloaded through a browser until you allow it under System Settings → Privacy & Security, or clear the quarantine flag with `xattr -dr com.apple.quarantine <folder>/Rick.app`; Windows SmartScreen may warn the first time.

## Linux packages

A release also makes a deb for each supported Debian and Ubuntu release and an rpm for each supported Fedora release, for x64 and arm64. They are built by Trebuchet's reusable workflow `linux-packages.yml` ([Open-Breach/trebuchet](https://github.com/Open-Breach/trebuchet), `docs/linux-packages.md`), which the release workflow calls. That workflow owns the list of releases; for each one it runs `.github/scripts/linux-package.sh` inside that release's container, packages the result with the metadata in `crates/rick/Cargo.toml`, installs the package in a fresh container and runs `rick --version` there. One build per architecture is also the Linux archive, with the plugin packages that fit it.

In that fresh container the workflow also runs `.github/scripts/linux-smoke.sh`, Rick's own part of the test, after the package is installed and once more for the unpacked archive:

- every plugin of the package loads, and no other plugin is in it;
- pdfium, from beside the binary, opens a PDF and renders a page;
- the media previewer plays two videos through the system's GStreamer, one in Motion JPEG and one in H.264;
- where the build is also the archive, each of its plugin packages is unpacked into the per-user folder and loads from there.

The container has the package's dependencies and nothing of the build, so a library missing from the dependency list fails the release there. The test programs it runs, `pdfium_check` and `preview_check`, and the two videos come from the same build, as one tar file in the workflow's folder for the smoke test, `TREBUCHET_SMOKE_FILES`. That folder goes from a build to its own smoke test and no further: nothing in it is attested or reaches a release. Two things a container cannot show: a window, with the libraries the user interface loads for X11, Wayland, Vulkan and EGL when it opens one, and sound. And one thing Ubuntu's container images leave out: they install nothing under `/usr/share/doc` but the `copyright` files, so for `THIRD-PARTY-LICENSES.md` the test there asks the package's list of files and not the disk.

What a package installs:

- `/usr/lib/rick/`: the binary with everything beside it, as in an archive: the shared libraries, `plugins/` with the default set, `third-party.json`, `licenses/` with the texts pdfium and the standard library bring, and `package`, a file holding `deb` or `rpm`, which tells Rick that its separate plugins come from the package manager;
- `/usr/bin/rick`: a script that starts the binary there;
- `/usr/share/applications/rick.desktop` and the icons under `/usr/share/icons/hicolor/`;
- the licence: `/usr/share/doc/rick/copyright` and `THIRD-PARTY-LICENSES.md` beside it in a deb, both under `/usr/share/licenses/rick/` in an rpm.

The call names the release tag, so Trebuchet's workflow attests the Linux files and attaches them to the release, as a draft where the tag has no release yet. Rick's release job then adds the other platforms' files and the checksums to that same draft and gives it its name and notes. The order from there is verify, then publish: the `verify` job checks the draft's files, and only after it the `publish` job makes a plain tag's release public. A pre-release and a dry run stay drafts. Last, for a published release, the workflow tells Trebuchet's publisher.

A package names every package it depends on, per distribution, in the workflow's call. Among them is a decoder for H.264, so that an MP4 plays with the package alone: `gstreamer1.0-libav` on Debian and Ubuntu, and on Fedora `gstreamer1-plugin-openh264` with `gstreamer1-plugins-bad-free`, whose parser that decoder needs. Fedora's plug-in is in Fedora's own repositories; the codec library it loads comes from Cisco's OpenH264 repository, which Fedora has switched on by default. Where someone has switched it off, Fedora installs a stand-in for the library and Rick says the plug-in for H.264 is missing. The packaging tools' own search for the libraries a binary links is off: Rick's binaries link the kernel libraries and the standard library that sit beside them, and that search does not get past them.

The separate plugins are not packaged yet; `rick-plugin-<id>` packages are Trebuchet's next step. After a published release the workflow tells Trebuchet's publisher, which puts the packages into the apt and dnf repositories on `get.openbreach.dev`.

## The separate plugins and their packages

Ten plugins are not in the archives: Git, SVN, ZFS (Linux), SSH, S3, Azure Blob, Google Cloud Storage, the Office and 3D previewers and, on macOS and Windows, the media previewer with its GStreamer runtime. That is nine on each platform. A release publishes each as a package of its own, `<id>-<version>-<platform>.rkp` (`git-0.6.0-linux-x64.rkp`), and one `plugins-<platform>.json` per platform that lists them for the plugin registry (specs/plugin-distribution.md §4). A package is a zip archive of the plugin's folder; *Install from file…* in the Extensions dialogue unpacks it into the per-user plugins folder, and the plugin loads at the next start. A package fits the Rick release it was built with and no other.

To make them from a bundle:

```bash
cargo xtask bundle --release
cargo xtask package --release
ls target/release/packages/
```

Which plugins are separate is said by each plugin's crate, under `[package.metadata.rick]` in its `Cargo.toml`. The same bundle gives the same bytes. A development build needs none of this: the bundle has every plugin.

## The README

`README.md` describes the latest release and is brought up to date in the release pull request only (specs/readme.md). `cargo xtask readme` does it: it writes the version, the Features list and the Install section from what the extensions register, and renders the two screenshots of `docs/screenshots/` afresh (it needs a wgpu adapter; a software one such as lavapipe will do). `cargo test --workspace` fails when the README of a release version is not what that command writes.

One part of the release pull request of 0.6.0 is written by hand: the privacy paragraph of specs/readme.md, section 9, word for word, as the first line under the rule above the closing links. The same test asks for it as soon as the status line names 0.6.0, and refuses it before. The same pull request brings the *Suggest plugins* setting into the README (specs/readme.md, *Rules for the release*).

## Writing a plugin

Start from `crates/plugins-native/README.md`. The numbers 0.2.0 was measured against are in [BENCHMARKS.md](../BENCHMARKS.md).
