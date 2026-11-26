# Rick — Plugin distribution

Status: decided 2026-11-18. Binding in the same way as SPEC.md; where the two disagree, the question goes into the pull request. The other half — the registry's format, its signing and how a release's plugins are published — is `docs/spec.md` in `Open-Breach/trebuchet`, sections 4.3 and 7, and is not repeated here.

Related: specs/plugins.md, specs/gui-dialogues.md, specs/gui-preview.md, specs/gui-side-panel.md, specs/core-types.md, specs/system-integration.md

## 1. Purpose and scope

Rick ships with the plugins a file manager cannot do without, and the rest become things a user fetches one by one. A release publishes each of those as a package of its own; Rick reads a signed list of them, installs the ones the user asks for, and says so when a file, a folder or a connection would be served by a plugin that is not there.

In scope: which plugins ship inside every install and which are separate, the plugin package as a release artefact, what a manifest says a plugin is for, the registry client, the *Available* view and updates in the Extensions dialogue, *Install from URL…*, and the suggestions.

Out of scope, by decision or for now: plugins built outside this workspace (§3), a plugin made active without a restart, permissions that are enforced, ratings, search across descriptions, any payment. This file is the later decision where `specs/system-integration.md` §1 calls a plugin store out of scope and where `specs/plugins.md` says *Extension store. Later.*

## 2. Decisions

1. **A default set ships inside.** Sixteen plugins are part of every install, in the archives and in the `rick` package; ten are separate (§4). Rick downloads nothing at its first start: what it needs to be a file manager is on the disk when it is unpacked, offline and behind a proxy alike.
2. **One package per separate plugin.** The package is the `.rkp` that *Install from file…* already reads. A release attaches one per separate plugin and platform.
3. **A signed registry.** Rick reads one index, `https://get.openbreach.dev/registry/index.json`, signed with a key whose public half is compiled into Rick. Open-Breach's own plugins only.
4. **A build fits one Rick.** The plugin boundary stays as it is, so a plugin loads only in the Rick release it was built with (§3). The registry names that release per build, and Rick offers a build only when it is its own.
5. **Linux packages are their own channel.** A Rick installed from a deb or an rpm takes its separate plugins from the `rick-plugin-<id>` packages; Rick shows the command and downloads nothing.
6. **Suggestions come from the cached index.** Rick never goes online because a file was selected.
7. **Nothing is installed unasked.** Every install and every update is a click.

## 3. What fits

- The plugin boundary is unchanged: a plugin shares the kernel crates with the binary as Rust dynamic libraries (`QUESTIONS.md 81`), and the host refuses one whose `api_version` or `toolchain` is not its own. In practice a build of a plugin loads in exactly one build of Rick, which is why every release publishes all of them.
- **The host says so.** `plugin.toml` gains `rick`, the version of Rick the plugin was built with, written by the bundler like the rest of the file. The host refuses a plugin whose `rick` is not its own version, before it loads the library and with the reason *Built for Rick 0.5.0*, so a library of another release is never mapped.
- **The rule.** A build of the registry fits when its `platform` is this platform and its `app_version` is this Rick's version, compared as whole strings. `api_version` is carried and shown; it decides nothing while the rule above is stricter.
- A Rick whose version has a pre-release part (`0.6.0-dev`) fits nothing, and the *Available* view says so.
- A Rick older than the registry's oldest release fits nothing either: *The plugins for Rick 0.5.0 are no longer published. Update Rick to install plugins.*
- Decided on 2026-11-18: the boundary stays. A C ABI was considered that evening and put aside, because it rewrites the `ext` boundary and every plugin while the API still changes every few days. A boundary that lets one build serve several Rick releases — a C ABI, or the WASM host — would turn the rule above into a comparison of `api_version` and nothing else here would change; it is a later possibility (*Still open*).

## 4. The default set, the separate plugins and their packages

**The default set** is in every install. Besides what is compiled into the binary — the local file system and the text, code, image and PDF previewers — it is these sixteen plugins, each where it is built at all:

| | Plugins |
| --- | --- |
| Providers | `vfs-trash`, `vfs-archive` |
| Previewers | `preview-archive`, `preview-hex`, `preview-icons`, `preview-markup` |
| Search | `search-walk`, and the indexers of the platform: `search-baloo` and `search-tracker` on Linux, `search-spotlight` on macOS, `search-windows` on Windows |
| Actions | `action-rename`, `action-compress`, `action-compare`, `action-terminal`, `installer` |

**The separate plugins** are these ten: `git`, `svn`, `zfs` (Linux), `ssh`, `vfs-s3`, `vfs-azure`, `vfs-gcs`, `preview-office`, `preview-3d` and `preview-media`.

- **Media is split by platform.** `preview-media` belongs to the default set on Linux, where GStreamer is the system's and the plugin is small, and is separate on macOS and Windows, where the runtime travels with it. That is nine separate plugins on each platform.
- **Where the list lives.** Each plugin crate says it in its own `Cargo.toml`, under `[package.metadata.rick]`: `separate = true`, or `separate = ["macos", "windows"]` for a split by platform. A crate that says nothing is in the default set.
- **The archives and the `rick` package carry the default set**, in `plugins/` beside the binary as today, and no separate plugin. `cargo xtask bundle` still lays out every plugin, so a development build has them all with nothing installed by hand; the release takes the separate ones out of the tree it archives.
- **The GStreamer runtime moves with the media plugin** on macOS and Windows: out of its place beside the binary and into the plugin's folder, so the package holds it and the plugin finds it there, as the archive plugin finds the 7-Zip library. The archives of those two platforms lose it.
- A **plugin package** is an `.rkp`: a zip archive holding the plugin's folder, `<id>/`, exactly as `cargo xtask bundle` lays it out under `plugins/` — `plugin.toml`, the library, and what the plugin bundles beside it, such as the 7-Zip library of `vfs-archive`. The name is `<id>-<version>-<platform>.rkp`, `git-0.6.0-linux-x64.rkp`; the platforms are those of the release archives: `linux-x64`, `linux-arm64`, `macos-arm64`, `windows-x64`.
- `cargo xtask package [--release]` makes them from the bundle: one package per separate plugin of the bundle in `target/<profile>/packages/`, and beside them `plugins-<platform>.json`, the manifest Trebuchet's publisher reads (its spec, 7.4). The entries of an archive are written in sorted order with a fixed timestamp, so the same bundle gives the same file and the same hash.
- The manifest's `meta` is Rick's: `points` (the extension points, as in `plugin.toml`) and `triggers` (§5). Trebuchet copies it without reading it.
- The release workflow runs the task on each of the four platforms and attaches the packages and the four manifests to the release.
- A release made before the registry client exists has its separate plugins on the release page only; *Install from file…* installs them.

## 5. What a plugin is for

A manifest already lists `previews`, the file name extensions its previewers claim. It gains two lists, and the three together are the plugin's **triggers**: what Rick can know about a plugin without having it.

- `markers` — names of an entry directly inside a folder that make the folder the plugin's business: `.git` for Git, `.svn` for Subversion. Compared as whole names, case as written.
- `connections` — what the plugin adds to *Add…* in the side panel: the section, `network` or `cloud`, and the title of the kind of connection, *Amazon S3*.

The lists are part of `ext::Manifest`, land in `plugin.toml` and in the package manifest's `meta.triggers`, and are empty for a plugin that has none. The versioned contract changes with them, so the API version goes up by one.

## 6. The registry client

- **What is read.** The index and its signature, `index.json.minisig`, from the address of decision 3. The public key is Trebuchet's `keys/registry.pub`, committed here as a file, byte for byte, and compiled in: a build has the registry on. A Rick without a key — a build whose file is empty, or a `[registry] public_key` in `rick.toml` that is no key — has it off: nothing is fetched, and the *Available* view says *The plugin list is not set up in this build*.
- **When.** When the *Available* view is opened, and in the background once a day, off the UI thread. The start itself asks nobody: once the first frame is drawn it reads the cached list. The background check comes one minute after the start, when the cached list is more than a day old or there is none, and again every 24 hours for as long as Rick stays open: whenever the list it holds becomes a day old. A list the user fetched in between counts, and the day starts over from it. A background check that fails is tried again 24 hours later, not sooner and never in a loop; the next start, the opening of the *Available* view and **Check now** are occasions of their own.
- **The switch.** *Look for plugins and updates online*, on the Behaviour page of the settings, on by default, `[registry] check` in `rick.toml`. Off, nothing is fetched by itself; the *Available* view then shows what was cached, its date, and **Check now**.
- **What is sent.** One GET for each of the two files, with `rick/<version>` as the user agent and nothing else: no identifier, no list of installed plugins. The README's privacy paragraph names this check (`specs/readme.md` §9), and it is the only one: the update notice for Rick itself, below, reads the same list and makes no request of its own.
- **Verification.** The signature is checked on every read, of a fetched index and of the cached one. An index that does not verify is dropped and the last good one stays; the view says *The plugin list could not be verified* and offers **Check now**. A `schema` above the one Rick knows is treated like a failed fetch, with *This list needs a newer Rick*.
- **The cache** is `registry/index.json` and its signature in Rick's cache directory. It is the only thing the suggestions read.
- **Another registry.** `[registry] url` and `[registry] public_key` in `rick.toml` point Rick at another index and key, for development and for the tests. While either is set, the dialogue's footer names the address.
- **The update notice for Rick itself.** The index names the newest release of Rick under `apps.rick.latest`, its version and the address of its release page (`docs/spec.md` 7.6 in `Open-Breach/trebuchet`). Rick reads it from the list it already holds, the fetched one or the cached one: no request is made for it, and there is no in-app installer. It is under the same switch: with *Look for plugins and updates online* off there is no notice, whatever a cached list says. When that version is newer than Rick's own:
  - **A toast**, once for a release: *Rick 0.7.0 is available*, with **Download**, which opens the release page in the browser. Which release was said is remembered in the cache beside the list (`registry/release-announced`), the way the plugin updates' notice is, so the next start says nothing of it and the release after it is said again.
  - **The menu.** The menu button carries the attention dot and the application menu has *Update available* (`specs/gui-title-row.md`, *Menu*), until Rick is that release or a newer one.
  - **The About dialog** has the same line (`specs/about-dialog.md`, *About page*).
  - **A packaged Rick** (the file `package` of §7) is updated by its package manager: the toast and the About line read *Rick 0.7.0 is available. Update it through your package manager.*, and nothing offers a download.
  - **Versions and addresses.** Both versions are read as versions and compared as such. A Rick with a pre-release part in its version is told of nothing, as it is offered nothing; a `latest` that is a pre-release, or no version at all, announces nothing. Only an `https://` address is opened: a release whose entry names another address, or none, is announced without **Download**. An entry that cannot be read costs the plugin list nothing.
- **Dependencies.** `reqwest` with `rustls`, as the cloud plugins use them, `sha2` and `zip` are in the workspace; signatures are verified with `minisign-verify`. Where the client lives among the crates is the implementer's proposal, in the pull request.

## 7. The Extensions dialogue

What `specs/gui-dialogues.md` says of the dialogue stays. It gains a second list.

- **Installed · Available.** A segmented control above the search field. *Installed* is the list as it is. *Available* lists the plugins of the registry that have a build that fits and that are in neither plugins folder, enabled or not, under the same headings; a row has the icon of its kind, the name and, where the switch is, the download size. The control shows the count, *Available 7*.
- **Details of an available plugin.** The head as for an installed one — tile, name, `git · 0.6.0 · Action` — with the chip *Available* and, in place of the switch, **Install** (`DOWNLOAD_SIMPLE`, the primary button). Then the description and an info card with *Contributes* (from the triggers: the file types, the markers, the connections), *Size* and *Source*, the registry's host.
- **Install.** A job: it downloads the package, compares its size and SHA-256 with the index, and unpacks it into the per-user plugins folder the way *Install from file…* does. A package that fails the comparison is deleted and the job fails with *The download did not match the plugin list*. The plugin then stands under *Installed* with the *restart* chip, as after *Install from file…*: it is loaded at the next start.
- **Restart now.** While any row has the *restart* chip, the footer shows **Restart now**, which closes Rick and starts it again with its session.
- **The record.** `installed.toml` in the per-user plugins folder notes, per id, where a plugin came from — `registry`, `file` or `url` — its version, the SHA-256 of its package and, for `url`, the address. An entry whose folder is gone is dropped at start. A plugin put into the folder by hand has no entry.
- **Updates.** After Rick itself was updated, the plugins of the per-user folder were built for the Rick before and the host refuses them. One that the record marks `registry` and for which the index has a build that fits is an **update**: its attention card reads *Built for Rick 0.5.0* and has **Update**, the header's secondary line counts them, *2 updates*, and the footer shows **Update all**. At the first start that finds any, a toast says *2 plugins need their build for Rick 0.6.0* with **Update all**. An update is an install over the old folder. The same holds for a build with a higher plugin version for the same Rick, should a release ever publish one.
- **Remove** works as today and drops the record's entry. A plugin removed is available again.
- **Install from URL…** stands beside *Install from file…* in the footer: a field for an `https://` address of an `.rkp`, which is downloaded and then handled exactly as a file is. Nothing is checked, as for a file; the record notes `url`.
- **Which addresses are asked.** `https://` is accepted for any host. Plain `http://` is accepted only when the host is this machine, so that no plain-HTTP request for a plugin ever leaves it. This machine is a loopback address written as one, `127.0.0.0/8` or `::1`, or the name `localhost`, which Rick takes for the loopback address itself, without a lookup; no other name counts, whatever it resolves to. The same holds after a redirect: every address a download is sent on to is held against the rule before it is asked, whatever address the download started from. And a redirect from a host that is not this machine to this machine is refused, whatever the scheme of either address, so that no server elsewhere has Rick ask a service of this machine; from this machine to this machine a redirect is followed. An address that is refused gets no request at all, and the message says that `https://` is needed. Plain HTTP never goes through a proxy. The rule holds for every request of the registry client: *Install from URL…*, a package of the list, and the index and its signature at an address `[registry] url` names.
- **A packaged Rick.** The deb and the rpm install a file named `package` beside the binary, holding `deb` or `rpm`; the archives have none. Where it is found, an available plugin has no **Install**: the details show the command, `sudo apt install rick-plugin-git` or `sudo dnf install rick-plugin-git`, monospace with a copy button, under the line *Installed through your package manager*. Plugins beside the binary are *Built-in plugin* as today and have no update of their own; the package manager moves them with Rick.
- **Offline, or the switch off and nothing cached:** *Available* shows one line saying so and **Check now**. Where the list did not arrive, the line is *The plugin list could not be fetched.* and names no reason: Rick may be offline, or the registry may not have answered with the list.

## 8. Suggestions

A suggestion says that a plugin exists for what the user is looking at. It is made from the cached index and the manifests in the plugins folders, never from a request.

- **Who is suggested.** A plugin of the registry with a build that fits, in neither plugins folder, whose triggers match. A plugin that is installed and disabled is suggested the same way from its own manifest, with **Enable** where the other has **Install**; that is today's *plugin available* line, which now has a button.
- **Three places.**
  1. *The preview pane.* The selected file's extension is in a plugin's `previews`, and no loaded previewer claims the file but the one of last resort. On the *No previewer* card the head keeps *plugin available* and the action row gains **Install 3D model preview** after *Open with…*: the button names the plugin as its manifest does. Where the hex view shows the file, the suggestion is the bar below, at the top of the pane: under the pane's toolbar, over the pinned File info card and the content.
  2. *The file list.* A pane enters a folder of the local file system whose listing — the one the pane has loaded, hidden entries included — holds an entry named in a plugin's `markers`. The suggestion is the bar, between the toolbar and the list of that pane. Under the toolbar the order is: the time slider when it is on, the bar, the banner of a listing that could not be read or of a path that *does not exist here*, the list. The bar follows the loaded listing for as long as the pane stays in the folder: `git init` beside the pane brings it and removing `.git` takes it away, without leaving and entering.
  3. *Adding a connection.* Where the side panel lists the kinds of connection that can be added — the *Add…* row of an empty section and the menu of a section's header — the kinds in the `connections` of suggested plugins follow the ones that are there, each with `DOWNLOAD_SIMPLE` and the plugin's size; choosing one installs the plugin.
     - *What an entry reads.* A suggested kind reads its title alone, *S3*, where a loaded kind reads *Add S3 connection…*: choosing it adds no connection and opens no dialogue. It stands under a hairline when loaded kinds stand above it, and its tooltip is the bar's text.
     - *A disabled plugin.* The kinds of a plugin that is on the disk and disabled are listed too, with `PUZZLE_PIECE` and *Enable* in the size's place; choosing one switches the plugin on.
     - *No click that names no plugin installs one.* With one loaded kind and nothing suggested the row reads *Add connection…* and opens that kind's dialogue at once. With a suggested kind beside it, or with suggested kinds alone, the row reads *Add…* and opens the list, even when the list has one entry.
     - *The Cloud section*, hidden while no cloud plugin is loaded, shows with *Add…* while a cloud plugin is suggested: one the registry offers, or one that is on the disk and disabled.
- **The bar.** One line on `bg/subtle`: `PUZZLE_PIECE`, the text, then **Install**, **Not now** and **Don't suggest again**. One bar per pane at a time; it takes no focus and no key, and leaving the file or the folder takes it away.
  - *The text* is the plugin's name and its description as the manifest writes it, *Git plugin — Shows Git status badges, the branch and how far it is ahead or behind, and adds the Git actions: …*, cut with an ellipsis where the line ends and whole in the tooltip. A plugin whose manifest has no description has its name alone, *3D model preview plugin*. `preview-3d` keeps its name, *3D model preview*, and gets no description.
  - *A narrow pane.* Where the text has no room beside the buttons — less than it needs, and less than 160 px — the buttons go under the text, on as many lines as they need. Wherever there is that room the bar is one line.
- **After Install and after Enable.** The plugin is on the disk, or switched on, and waits for the next start; it is suggested no longer. The place that offered it says so.
  - *The bar.* Every bar that showed that plugin's offer at the time reads *Git is installed. Restart Rick to use it.* with **Restart now** and no other button, and after **Enable** *Git is enabled. Restart Rick to use it.* It reads so until its pane leaves the file or the folder. The next folder with the marker has no bar: the Extensions dialogue has the *restart* chip and **Restart now** (§7).
  - *The card.* The *No previewer* card that had **Install …** or **Enable …** has **Restart now** in that button's place, and its head reads *plugin installed, restart Rick to use it*, or *plugin enabled, restart Rick to use it*, where it read *plugin available*, until the selection moves on.
  - *Add…* closes with the choice, so a toast says it once the plugin is on the disk or switched on: *S3 is installed. Restart Rick to use it.*, or *S3 is enabled. Restart Rick to use it.*, with **Restart now**. It stays until the button is pressed or the toast is dismissed. A bar and a card say it themselves and raise no toast. The toast tells of an install the user asked for; it is not a suggestion, and a suggestion is never a toast.
- **Not now** silences that plugin until Rick is started again. **Don't suggest again** writes its id to `[plugins] no_suggest` in `rick.toml`. *Suggest plugins*, on the Behaviour page and on by default (`[plugins] suggest`), switches all of it off: the bars, the kinds in *Add…* and, on the *No previewer* card, the button and the line *plugin available* with it (`specs/gui-preview.md`, *No previewer*, where a note on this stands).
- **Not suggested.** In the Extensions dialogue the details of a silenced plugin read *Not suggested* with **Suggest again**, which takes back **Not now** and **Don't suggest again** alike. It is one line under the description, the words and then the button, in the details of an available plugin and of an installed plugin that is switched off. An installed plugin that is switched on is never suggested and has no such line.
- **A packaged Rick.** **Install** opens the Extensions dialogue on the plugin, where the command stands.
- A path typed into the path bar is not a trigger: the path bar takes paths, and a remote location is reached through its connection.

## 9. Trust

- A plugin installed from the registry was checked twice: the index against the compiled-in key, the package against the index. Its hash is in the record.
- When the trust check of `specs/system-integration.md` §9 arrives, a plugin the record marks `registry` whose folder still matches is not asked about; one from a file, from an address or put there by hand is, as that section says.
- The build attestations of that section stay a matter of the release. The registry does not depend on them.

## 10. Milestones

Issues in this order, each merged into `dev` when green.

1. The split, packages and triggers (§3 *The host says so*, §4, §5): `rick` in `plugin.toml`, the two manifest lists, the separate plugins marked, `cargo xtask package`, the archives with the default set, the release workflow.
2. In Trebuchet: the publisher's side and the plugin packages. Needs the signing key from Serge.
3. The registry client and the Extensions dialogue (§3, §6, §7, §9).
4. Suggestions (§8).
