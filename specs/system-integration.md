# Rick — System integration & distribution

Status: decided 2026-11-07 and 2026-11-09; part of Version 0.3 (see SPEC.md, *Version 0.3*). Binding in the same way as SPEC.md; where the two disagree, the question goes into the pull request. Design reference for the dialogs: the Figma file *Rick - FileExplorer*, section *System integration* (frames *Integration dialog — first run (Linux)* and *Settings — Integration*, plus the yellow *Spec notes — integration* frame).

## 1. Purpose and scope

System integration is everything that makes Rick behave like the file manager of the machine rather than a window on it: being opened by the system and other programs, exchanging files with them, appearing where the user looks for apps, and arriving on the machine through a channel the platform trusts. Distribution is how a release reaches users on each platform and how they can verify where it came from.

In scope: single instance and the command line, the `rick://` scheme, clipboard interoperability, the integration check and its dialog, the per-platform integration items, notifications and progress, signing and provenance, plugin load rules, the release channels per platform, update checks, uninstall, and the release matrix.

Out of scope, by decision: replacing the system file chooser (the portal `FileChooser` backend — a different GUI, not planned), a plugin store, the WASM host, mounting remote providers as local paths (GVFS-style), the Windows 11 compact context menu (needs a packaged shell extension; later), Flatpak and Snap, the Mac App Store, the Microsoft Store, official distribution inclusion before 1.0.

## 2. Decisions

1. **Linux:** Rick can be set as the default file manager. It ships a desktop entry and the `org.freedesktop.FileManager1` D-Bus service.
2. **macOS:** Rick runs alongside Finder and never claims to replace it.
3. **Windows:** alongside Explorer by default; *Make Rick the default* is an opt-in setting with a warning, a repair action and a one-click revert.
4. **Distribution:** native packages through the Open Build Service plus the AUR plus an AppImage on Linux; a notarised DMG plus a Homebrew cask on macOS; a signed installer plus winget on Windows.
5. **Provenance:** every release artifact, including every plugin file, carries a Sigstore signature and a GitHub build attestation.
6. **Plugin permissions** are declarative, not enforced, until a WASM host exists; the Extensions screen says so.
7. **Single instance:** one Rick process per user session; a second launch hands its arguments to the running instance and exits.
8. **Clipboard:** system clipboard only; the internal in-memory file clipboard is dropped.
9. **Integration check:** one startup check per platform and one dialog listing what is missing, each item with its own switch; a switched-off item is *declined* and is never asked again.
10. **Linux desktops with first-class support:** GNOME, KDE Plasma, COSMIC and Cinnamon. All others get the shared freedesktop parts.

## 3. Single instance

- One Rick process per user session. The first process binds a local endpoint — a Unix socket in `$XDG_RUNTIME_DIR` (Linux), in the user's temporary directory (macOS), a named pipe under `\\.\pipe\` (Windows) — named after the user and the config directory, so two installs with different config directories can coexist.
- A second launch connects, sends its command (see §4) as one message, waits for the reply, prints it if any, and exits with the instance's status. The reply arrives within a second or the newcomer reports the instance as unresponsive and exits non-zero; it never starts a second GUI unless `--new-window` was given and the endpoint cannot be reached at all (a stale socket from a crash is removed and the newcomer becomes the instance).
- Reasons, for the record: one clipboard, one job queue, one session and one set of watchers; a context-menu click opens a tab in a warm instance instead of cold-starting a process; the D-Bus name has one owner anyway.
- A path handed to the instance opens as a new tab in the **most recently focused window**; `--new-window` opens a new window with that tab. The window is raised and focused through the platform's activation mechanism (`xdg_activation` token or `_NET_ACTIVE_WINDOW` on Linux, `NSApp.activate` on macOS, `SetForegroundWindow` with the newcomer's `AllowSetForegroundWindow` on Windows).
- When nothing is running, the same binary becomes the instance; every entry point below works identically whether Rick was running or not.

## 4. Command line and `rick://`

- The command line gains subcommands. **Core:** `rick [<path>…]` (no argument: the current directory; several paths: one tab each), `rick reveal <file>…` (opens the folder and selects the files), `rick --new-window [<path>…]`, and the existing `rick --version` and `rick --list-plugins`. Paths are resolved against the caller's working directory before they are sent.
- **Plugins register their own subcommands** through the plugin manifest (`[[commands]]` with name and help line); the host dispatches `rick <command> <args>` to the plugin, e.g. `rick install <archive>` from `ext-installer`. Unknown commands print the help and exit 2.
- Over the socket this is one message type, *run command with arguments*, with the caller's working directory and environment subset (`DISPLAY`/`WAYLAND_DISPLAY`, the activation token). The same message is what every platform entry point sends.
- **Thin wrappers:** the D-Bus `ShowItems` call maps to `reveal`, `ShowFolders` to open, `ShowItemProperties` to open plus the Properties dialog; the Windows registry verb runs `rick "%1"`; the macOS Apple Event for opened folders and the Finder extension run the same open command.
- **`rick://<path>` URL scheme** on all three platforms, for links from documents, wikis and chat: `rick:///home/serge/Apps` opens the folder, `rick:///home/serge/Apps/x.txt` reveals the file. Registered through the desktop entry (`x-scheme-handler/rick`), `Info.plist` (`CFBundleURLTypes`) and the registry (`HKCU\Software\Classes\rick`). Paths are percent-decoded; anything that is not an existing local path shows an error toast and opens nothing.
- **Shell integration:** the embedded console follows the tab's folder and a `cd` in the console moves the tab (the console reports its working directory through OSC 7; Rick emits `cd` on tab navigation only while the console has no foreground job). A shell function installed by the integration dialog makes `rick` with no argument open the shell's current directory — the binary already does that; the function only exists so `rick` is on the PATH of login shells that do not read `~/.local/bin`.

## 5. Clipboard

- **System clipboard only.** The internal file clipboard goes away: there is one state, the system's.
- On *Copy* or *Cut* Rick writes, in one clipboard transaction: the native file-list formats where they exist — Windows `CF_HDROP` plus `Preferred DropEffect` (`DROPEFFECT_COPY` or `DROPEFFECT_MOVE`); Linux `text/uri-list` plus `x-special/gnome-copied-files` (`copy` or `cut` on the first line, also read by KDE); macOS `NSPasteboard` file URLs (`public.file-url`) — plus **Rick's own type** `application/x-rick-entries` (one UTI and registry format of the same name) carrying the full entries: provider id, path or key within the provider, and the copy-or-move mode.
- **Non-local items** (SSH, archives, cloud) have no native file representation. Rick still writes something native alongside: at least `text/plain` with one path or URL per line, so a paste into a text field yields the paths. Other file managers see the paths as text, not as files; that is the platform's limit.
- **Paste** reads Rick's type first and uses it when present; otherwise the native file list. Cut-paste inside Rick moves on every platform, macOS included (the mode travels in Rick's type). Cut in Rick, paste in Finder is a copy — Finder has no cut. After a cut-paste Rick clears the clipboard.
- The SPEC.md sentence that deferred clipboard interoperability to "later" is superseded by this section.

## 6. Integration check and dialog

### 6.1 Model

- Rick keeps, in `rick.toml` under `[integration]`, one record per **integration item** (§7): `state = "set-up" | "declined" | "unsupported" | "missing"` and the date of the last check. The wanted state is "set-up" for every item unless declined.
- **At startup** (after the first frame, off the UI thread) Rick compares the wanted state with the actual one, item by item, for the current platform and desktop. The check reads files and registry keys only; it changes nothing.
- An item that is **missing** or **drifted** (was set-up, is no longer — e.g. Windows restored Explorer after an update) is listed in the **integration dialog**, each row with its own switch (on by default for missing, on for drifted). A row switched **off** is stored as *declined*. **Apply N items** performs the switched-on rows and shows a progress line; **Not now** closes without changes and asks again at the next start; the footer checkbox **Don't ask again** silences the whole dialog (stored as `[integration] ask = false`).
- **Declined** is treated like done: the check never lists a declined item again, and a declined item the user later set up by hand outside Rick is **not watched for drift** — declined means "not Rick's business".
- **Settings → Integration** (a new category after *Appearance*) shows every item with a status pill — *Set up* (green), *Reset by system* (amber, with **Restore**), *Skipped* (grey, with **Add**), *Unsupported* (grey, no action) — the OS, desktop and time of the last check, a **Check again** button, the preference switches (shell integration, *Check integration at startup*), and **Reset choices**, which clears every declined mark so the dialog asks once more. **Add** performs the item and replaces *declined* by *set-up*, after which drift is reported like any other item.
- Items that need **elevation** say so in their description (on Linux only *All users* asks for a password, through `pkexec` and the installer's helper; on Windows a UAC prompt for `HKLM`; on macOS none). The dialog never asks for a password itself.
- **Linux, started from an archive** outside an install location: the first row is *Install to the standard location*, which moves Rick to `~/.local/opt/rick` (or `/opt/rick` for *All users*) through `ext-installer` and relaunches from there; the other rows then act on the installed copy. The *Install for* segmented control (Just me / All users) at the top of the dialog sets the scope for every row.

### 6.2 Dialog

Per the Figma frame: header with the app icon, title *Set up Rick on this system* and a subtitle naming OS and desktop; the explanatory line; *Install for*; the items card (icon, title, one-line description, optional *Best effort* pill, switch); an info line about passwords; footer with *Don't ask again*, *Not now*, *Apply N items*. Settings window chrome and tokens; nothing new.

## 7. Integration items

### 7.1 Linux

- **Install location** — the binary, plugins and `share/` under `~/.local/opt/rick/<version>` with a `current` link, or `/opt/rick`; via `ext-installer`.
- **App launcher entry** — `packaging/linux/rick.desktop` and the icons installed under `share/applications` and `share/icons/hicolor` of the chosen scope; `update-desktop-database` and `gtk-update-icon-cache` run when present.
- **Default file manager** — `xdg-mime default rick.desktop inode/directory` and `x-scheme-handler/rick` for the scope's `mimeapps.list`.
- **"Show in folder" from other apps** — the D-Bus service `org.freedesktop.FileManager1` (`ShowItems`, `ShowFolders`, `ShowItemProperties`) with a service file for D-Bus activation, installed in `share/dbus-1/services` of the scope (user and system installs alike). The running instance owns the name; activation starts Rick when it is not running.
- **`rick` command** — a symlink in `~/.local/bin` or `/usr/local/bin`, plus the shell function of §4 in Rick's `path.sh` (the one the installer plugin sources from the profile).
- **Dock / launcher favourite** — GNOME: `org.gnome.shell favorite-apps` through gsettings; Plasma: the task-manager launcher list in `plasma-org.kde.plasma.desktop-appletsrc`; COSMIC: the dock's configuration under `~/.config/cosmic`; Cinnamon: `org.cinnamon favorite-apps`. Other desktops: *Unsupported*. Detected from `XDG_CURRENT_DESKTOP`. Marked **Best effort**: it never blocks the rest and a failure is reported, not retried.
- Also honoured, not items: the Trash spec (already), the thumbnail spec (`~/.cache/thumbnails`, read and write), `recently-used.xbel`, `mimeapps.list` for *Open with*, udisks2 for the drive list, `xdg-terminal-exec` where present for *Open terminal here*.

### 7.2 Windows

- **Context menu** — *Open in Rick* on `Directory`, `Drive` and `Directory\Background` under `HKCU\Software\Classes` (classic menu; the Windows 11 compact menu is out of scope).
- **Start menu shortcut** — `Rick.lnk` in the user's Programs folder with the icon and `AppUserModelID` `OpenBreach.Rick`.
- **Taskbar pin** — **Best effort**: Windows has no public API; Rick tries the documented shell verb and reports failure without retrying.
- **`rick` on PATH** — the install folder appended to the user `PATH` (`HKCU\Environment`), broadcast with `WM_SETTINGCHANGE`.
- **App registration for Open With** — `HKCU\Software\Classes\Applications\rick.exe` with `SupportedTypes` empty and `FriendlyAppName`, plus `RegisteredApplications` so Rick appears in *Default apps*.
- **Default file manager** (opt-in) — rewrites the `open` verb of `Folder` and `Directory` and the Explorer shell-open command to `rick "%1"`, after storing the previous values in `rick.toml` for **Revert**. Off by default; the row's description warns that Windows updates may reset it; the drift check shows the *Reset by system* pill with **Restore**. The uninstaller reverts it.
- Manifest `longPathAware`; `\\?\` prefix for long paths; drive letters and UNC paths first-class in the local provider.

### 7.3 macOS

- **Folder and `rick://` registration** — `Info.plist` declares `public.folder` in `CFBundleDocumentTypes` and the `rick` scheme in `CFBundleURLTypes`; Launch Services registers on first launch from `/Applications`. The item is *Set up* when the bundle is registered and lives under `/Applications` or `~/Applications`, else it offers to move it there.
- **Finder extension** — a Finder Sync extension bundled in `Rick.app` adding *Open in Rick* to the folder context menu; the item opens *System Settings → Extensions* when the extension is not enabled (there is no API to enable it). **Best effort**.
- **Services** — *Open in Rick* under the Services menu via `NSServices` in `Info.plist`; needs no action, listed as *Set up* once registered.
- **Dock** — the app added to `com.apple.dock persistent-apps` and the Dock restarted; **Best effort**.
- **`rick` on PATH** — a symlink in `/usr/local/bin` (asks once through the system authorization dialog) or `~/.local/bin` plus the shell function.
- Also honoured, not items: `NSFileManager.trashItem` for the trash (already), Quick Look as previewer of last resort, Finder tags via extended attributes, the Keychain for secrets, Launch Services for *Open with*, Core Spotlight for `ext-search-spotlight`.

## 8. Watching, notifications, progress

- **File watching** (already in `vfs-local`): only folders currently shown in a tab plus the side panel's drive list; never recursive (inotify watch limits); events are coalesced per folder and the view refreshes at most every 200 ms.
- **Notifications:** a job that ran longer than five seconds and finishes while no Rick window is focused posts one platform notification (`org.freedesktop.Notifications`, `UNUserNotificationCenter`, Windows toast) naming the job and its result; clicking it focuses the window.
- **Progress:** while jobs run, the overall progress is shown on the taskbar button (Windows `ITaskbarList3`) and as a Dock badge with the overall percentage (macOS); Linux shells that support the `com.canonical.Unity.LauncherEntry` progress bar get it too.

## 9. Signing, provenance and plugin security

- **Signing.** Every file in a release — the archives, installers, DMG, AppImage, and every plugin file inside them — gets a Sigstore `cosign` keyless signature tied to the release workflow's GitHub Actions identity, and a GitHub build attestation (`actions/attest-build-provenance`) in the same job. Verification instructions (`gh attestation verify … --owner Open-Breach`) go into the README's download section.
- Linux repositories are GPG-signed by OBS; a self-hosted repository would sign with a key kept as a CI secret with an expiry and a rotation plan.
- macOS: Developer ID signing and notarisation of `Rick.app` and the DMG (`notarytool`, API key as CI secrets). Windows: Authenticode through Azure Trusted Signing on `rick.exe`, every plugin DLL and the installer.
- **The README states plainly that signing proves origin, not safety.**
- **Plugin permissions** are a declaration: the manifest lists what the plugin uses (`permissions = ["network", "filesystem", "credentials", …]`); the Extensions screen shows the list under the plugin with the sentence *Declared by the plugin — Rick cannot enforce this for native plugins.* Enforcement arrives with the WASM host.
- **First-party plugins** (shipped in the release's `plugins/` folder) are verified at load against the release's signature bundle (`plugins/signatures.json`, produced by the release job); a mismatch refuses the plugin with the reason in the Extensions screen.
- **Third-party plugins** (from the per-user plugins folder) are **trust on first use**: the first load shows an approval dialog naming the file, its SHA-256 and its declared permissions; approval pins the hash in `rick.toml`; a changed hash prompts again. Nothing is loaded without approval.
- **Load rules:** plugins load only from the install's `plugins/` folder and the per-user plugins folder (already the case); never from the working directory, from a world-writable directory, or from a path given on the command line. The user folder must be owned by the user and not world-writable, or its plugins are refused with a message.
- **Security hygiene:** `SECURITY.md` with a disclosure contact and a response target; `cargo deny` (already) plus `cargo audit` in the quick checks; the privacy statement in the README: *Rick sends nothing anywhere. No telemetry, no crash reports; the only network traffic is the one you start — a remote provider you added, or the update check you switched on.*

## 10. Distribution

### 10.1 Linux

- **Open Build Service** project (Serge's home project, e.g. `home:<user>:rick`) building a `.deb` for Debian stable and Ubuntu LTS and an `.rpm` for Fedora (current and previous) and openSUSE Tumbleweed, from the release tarball; Cargo dependencies vendored by the OBS `cargo_vendor` source service; OBS signs and hosts the repositories. Packaging sources live in `packaging/obs/` (spec file, `debian/` folder, `_service`).
- **AUR:** `rick-bin` from the release archives (`packaging/aur/PKGBUILD`), pushed by the release workflow to the AUR over SSH; a from-source `rick` package optional, later.
- **AppImage** built in the release workflow (`appimagetool`), no integration of its own; the integration dialog's *Install to the standard location* row is how an AppImage user gets integrated.
- **Uninstall:** the packages' post-remove scripts delete the desktop entry, icons, D-Bus service file and `mimeapps.list` default; the installer-plugin path removes the same through its record.
- Official distribution inclusion is not pursued before 1.0: `cargo-debstatus` shows a large share of the ~540 crates missing from Debian, mostly the egui/wgpu stack.

### 10.2 macOS

- A signed and notarised `Rick.app` in a DMG (`create-dmg`), built in the release workflow with the existing Apple Developer account; **universal binary** (arm64 + x86_64) from 0.3.0 — Intel Macs are covered.
- **Sparkle** for in-app updates: EdDSA-signed appcast published as a release asset; the About dialog and *Settings → General* expose the update check.
- **Homebrew cask:** own tap `Open-Breach/homebrew-rick` first (`packaging/homebrew/rick.rb`, bumped by the release workflow), pull request to `homebrew-cask` once stable; `livecheck` points at GitHub releases.

### 10.3 Windows

- **Installer:** WiX (`packaging/windows/rick.wxs`, built with the `wix` .NET tool in the release workflow), an MSI — per-user by default, per-machine on request — installing the binary and the plugins, with the integration items of §7.2 as optional features the user ticks in the setup dialog; Authenticode-signed, the MSI and the binaries inside it. Also a portable zip without integration.
- **Uninstall**, through the MSI from *Installed apps* or `msiexec /x`, removes the context-menu keys, the Start-menu shortcut, the PATH entry and the app registration, and a custom action **reverts *Make Rick the default*** when it was switched on.
- **winget:** manifest in `microsoft/winget-pkgs` (`packaging/winget/`, installer type `wix`), submitted by the release workflow with `wingetcreate` from the Open-Breach fork; Scoop or Chocolatey only on request.
- Decided 2026-11-09: WiX. Inno Setup was the assumption before the decision.

### 10.4 Update checks

- Package-manager installs update themselves. AppImage, the portable zip and the installer get a **new version available** check against the GitHub releases API: once a day at most, opt-out in *Settings → General*, never automatic download; the result is a line in the About dialog and a one-time toast. The DMG uses Sparkle.

## 11. Operator checklist

Things only Serge can do: accounts, identities, certificates and secrets. Secrets go into GitHub Actions secrets on `Open-Breach/rick`. The release workflow skips a channel whose secrets are missing and says so in the job summary, so a release never fails on a missing account.

- Once: the publisher identity shown by code signing on every platform (Open-Breach or personal name) — consistent across Apple, Azure Trusted Signing and the Linux repository key; a reverse-DNS identifier (`org.open-breach.rick`, already in use); `id-token: write` and `attestations: write` permissions for the release workflow.
- Linux: an account at build.opensuse.org and a home project for Rick; `osc` installed and logged in once; the OBS targets enabled; an AUR account with an SSH key (`AUR_SSH_KEY` secret).
- macOS: a *Developer ID Application* certificate exported as `.p12` (`MACOS_CERT_P12`, `MACOS_CERT_PASSWORD`); an App Store Connect API key for `notarytool` (`NOTARY_KEY_ID`, `NOTARY_ISSUER_ID`, `NOTARY_KEY_P8`); the bundle identifier registered; the repository `Open-Breach/homebrew-rick`; a Sparkle EdDSA key pair (`SPARKLE_PRIVATE_KEY`).
- Windows: an Azure Trusted Signing account and certificate profile (identity validation takes days — start early); an Entra app registration federated to GitHub Actions with the *Trusted Signing Certificate Profile Signer* role (`AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_SUBSCRIPTION_ID`); a fork of `microsoft/winget-pkgs` under Open-Breach.

## 12. Milestones

Issues in this order, each merged into `dev` when green; the design refresh of issue 60 runs in parallel and ships in the same release.

1. Single instance, CLI subcommands, `rick://`, shell integration (§3, §4).
2. Clipboard interoperability (§5).
3. Integration check, dialog and *Settings → Integration*, with the Linux items (§6, §7.1).
4. Windows integration items (§7.2).
5. macOS integration items and the universal binary (§7.3, §10.2 first bullet).
6. Notifications and progress; watcher rules verified (§8).
7. Signing, provenance, plugin security and hygiene (§9).
8. Linux distribution: OBS, AUR, AppImage, uninstall (§10.1).
9. macOS distribution: DMG, notarisation, Sparkle, Homebrew (§10.2).
10. Windows distribution: installer, Authenticode, winget (§10.3), and update checks (§10.4).

Milestones 8–10 need the operator checklist (§11) done for their platform; the release workflow degrades gracefully until then.
