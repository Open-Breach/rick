# Decisions

Related: specs/history.md, specs/open.md

The entries kept from the retired `QUESTIONS.md`, under their original numbers: the decision, or the assumption as it was implemented. A citation `QUESTIONS.md N` anywhere in the repository means entry `QN` here. Reference, not reading: `grep '^Q81\.' specs/decisions.md`. The full text of every entry is in the history: `git show 2102534:QUESTIONS.md`. *(provisional)* marks an entry that awaits a decision. Nothing is appended here.

Q5. The 30 s fixture budget is enforced on Linux only; Windows and macOS report their time.

Q7. Layer order: types > runtime > platform > ext > vfs > vfs-local > model > ops > app > ui-egui > rick, in deny.toml.

Q11. Restore in 0.1: macOS restores only what Rick trashed; a taken original path fails and is not resolved.

Q15. Trash on a local path goes through `platform::Trash` and is undoable; on another provider it is not.

Q18. `rick.toml`: absent keys take defaults; a file that does not parse warns and the last good settings stay.

Q19. Navigation is `Command`s bound by the UI's key map, not actions; names come through `UiHost::prompt`.

Q21. The shortcut table is the source of defaults; also bound: F11 Full screen, Mod+Q Quit, Mod+B side panel.

Q24. 60 fps is measured as egui's CPU frame: p95 under 16.6 ms over 240 scrolling frames of the 200k fixture.

Q26. `UiHost::prompt` and `confirm` answer in two passes: record the question, show a modal, run the action again.

Q28. Tab order is display state of the `egui_dock` tree, reported to the model (Q67).

Q30. Image budget: header facts at once, pixels as fast as the format allows; the fixture is a 48 MB BMP.

Q35. No grammar for TypeScript, F#, Visual Basic or TOML: TypeScript uses JavaScript's, the rest are plain text.

Q37. `ext-git` reads through gix; its actions run the `git` command line as jobs with `GIT_TERMINAL_PROMPT=0`.

Q40. Decorations merge in `Model::decorate`; the later decorator's badge and tint win; there is no priority.

Q47. Undo and redo have no button and no menu entry: shortcuts and command mode only.

Q59. `TransferMode::Ask` is answered by the UI (Move / Copy / Cancel at the cursor), never by the app.

Q62. Drops arrive through egui's dropped files; the drag source is the `platform::DragSource` hook.

Q63. Pins are stored as `provider:path` strings; a bare path is local.

Q66. Mod+N opens a new window; `core.new_file` has no default shortcut.

Q67. `model` mirrors the dock tree (splits, fractions, tab order) through `Command::TabOrder` and `Layout`.

Q68. Further windows are immediate viewports of the one `RickUi`; the root viewport moves when its window closes.

Q69. "Continue where I left off" is the session as read at start; a path argument opens a new tab in it.

Q70. `[session] start = "last" | "<layout>"` in `rick.toml`; the side panel's state is per-window session state.

Q72. Mod+Shift+F is the subtree search; Flatten subfolders has no default binding.

Q74. `Trash::remove(&TrashedItem)` joins the trait; the trash actions run on the `trash` pseudo-provider.

Q80. Timed tests on macOS and Windows assert the median of three runs; their target is median x 1.5.

Q81. Plugin ABI: Rust dylibs (`types`, `runtime`, `ext`) built with `-C prefer-dynamic`, not a C-ABI vtable.

Q82. `plugins-native` is the second crate allowed `unsafe`, for the loader calls only. (decided 2026-11-10)

Q83. `Manifest` gains `points` and `platforms`; `plugin.toml` is written by `cargo xtask bundle`. API 3.

Q84. `cargo xtask bundle` lays out `target/<profile>/bundle/`; the release archives are its contents.

Q85. A plugin library whose entry point ran is never unmapped, even when refused or after a panic.

Q86. The virtual locations are the built-in provider `rick` (crate `vfs-rick`), stored as `rick:trash`.

Q87. Provider ids `s3`, `azure`, `gcs` count as cloud and `sftp` as network, until a provider declares its kind.

Q88. `platform` gains `StandardFolders` and `RecentlyUsed`; Recent sorts by date accessed.

Q89. `ops::Scheduler` queues jobs per volume (device, or provider id); progress is coalesced to 50 ms.

Q90. Place icons are forty curated Phosphor icons, stored by name in `places.toml`; place edits are not undoable.

Q92. Trash view: Delete deletes permanently, Enter restores; virtual views take no writes and no drops.

Q93. A click on a side-panel row gives the panel the keyboard; Escape releases it.

Q94. The macOS trash record `trash.toml` lives in `platform`, behind `Trash::list`.

Q95. `Registry::view(name, provider)` lets an extension serve a `rick://` virtual location.

Q96. `ext` re-exports `platform` and `runtime`; an extension takes `Registry::platform()` while registering.

Q97. An archive location is the OS path plus the inner path (`archive:/a.zip/x`); a nested archive is a file.

Q99. The plugin entry symbol carries the crate name: `rick_plugin_register_<crate>`.

Q100. The trash actions are `ext-vfs-trash`'s: `trash.restore`, `trash.restore_items`, `trash.empty`.

Q101. Core actions follow capabilities: write actions hide without `WRITE`, Move to trash without `TRASH`.

Q102. zip, tar and 7z are writable by rewriting the whole archive; RAR, ISO, UDF and codecs are read-only.

Q103. `PreviewDoc::content()` returns `Listing`, `Grid` or `Document`; `Transport` gains volume, speed, loop. API 4.

Q104. `ext-preview-office` finds `soffice` itself, caches PDFs under `<temp>/rick/office/`, uses its own pdfium.

Q105. Encrypted archive entries are listed and marked; no password is asked; listing stops at 200 000 or 10 s.

Q106. GStreamer playback sits behind `ext-preview-media`'s `gstreamer` Cargo feature, off by default.

Q107. CSV: header unless all cells are numbers; delimiter sniffed over twenty records; 256 MiB indexed.

Q113. Find highlights text in place; in paged content it counts the matches and goes to their page.

Q115. Space toggles the selection; the preview pane opens with Mod+Shift+I, not with Space. (decided 2026-11-10)

Q118. The search box stays on every platform; an index adds a second group of rows; a hit is selected.

Q119. `Filter` gains `NoHidden` and `SkipNames`, `SearchScope` gains `covers`; `[search] ignore`. API 5.

Q120. Search connectors drive command lines: `mdfind`, `tracker3`, `baloosearch`, PowerShell for Windows.

Q121. A plugin not for this platform is left out of the bundle (`rick-plugin-manifest` exits with 3).

Q122. `ext-svn`: Update, Commit, Revert, Log, Add; Commit adds the selected unversioned files first.

Q123. `UiHost::form(Form) -> Option<Values>`: fields plus a live preview that can block. API 6.

Q124. `ActionOutcome::View` opens Compare and Console tabs (not in the session); `::Select` selects rows.

Q125. `ActionCtx::other` is the location of the window's first other explorer pane.

Q126. The `Conflicts` queue lives in `runtime`; every `JobCtx` carries it, so plugins ask the same dialogue.

Q127. `ext-action-compress` links `ext-vfs-archive`: the one plugin-to-plugin dependency. (decided 2026-11-10)

Q128. `LinkKind` gains `Junction` and `Shortcut`; the link actions ask the kind and are undoable.

Q129. `OpenWith` gains `applications` and `open_with`; the choice is kept in `core.toml` under `[open_with]`.

Q130. Extension settings live in `<config dir>/<extension id>.toml`; `platform::Terminal` opens a terminal.

Q131. Embedded console: the user's shell, `TERM=xterm-256color`, 10 000 lines; every key goes to the shell.

Q132. Bulk rename: `{name}`, `{ext}`, `{n}`, `{date}`; one undoable job through temporary names, local only.

Q133. Compare: local only; text up to 64 MiB, bytes otherwise; folders one level deep.

Q135. `ext-vfs-sftp` was built, then removed: `ext-ssh` replaces it (decided 2026-11-08).

Q136. `Connector` contract (API 7): `[[connections]]` in `rick.toml`, secrets in the keychain.

Q137. `Provider::transfer` defaults to `Unsupported`; `ops` then takes the kernel's path.

Q138. Object stores: a folder is a zero-byte key ending in `/`; no `RENAME`; uploads in 8 MiB parts.

Q140. `REMOTE` providers get no thumbnails; connection rows never connect until clicked.

Q142. `PreviewControl::Pointer` and `ControlValue::Delta` carry drag, wheel and double-click to a previewer.

Q143. `ext-preview-3d` reads `triangle_budget` (default 2 M) from `preview-3d.toml` at every open.

Q144. STL and OBJ are read as millimetres; Z is up; a 3MF's declared unit converts.

Q148. Uninstall asks first and is not undoable; Revert is the way back.

Q149. `platform::Elevate` runs a program as administrator (`pkexec`); system-wide installs wait for the helper.

Q150. `ActionOutcome::Navigate(Location)` (API 9); a `rick:apps` place can be added in `places.toml`.

Q151. Dropping an archive onto the Installed Apps view is not built.

Q157. The installer leaves hostile archive entries out and lists them; four cases refuse the whole install.

Q158. `ext-installer`'s manifest declares no permissions yet; that waits for the security model.

Q159. `Prompts` queue in `runtime` (`ask`, `choose`, `secret`), shown through `Event::Question`. API 10.

Q160. The terminal emulator is `ext::terminal`; the SSH console is a `View::Console` on the pooled connection.

Q161. `Previewer::reads()` is `Whole` or `Prefix`; remote prefixes come in 256 KiB chunks, the cap is 4 MiB.

Q162. `Connector::schema()` is owned; `Choice` fields; *Edit…* for Rick's own connections only.

Q163. SSH hosts are `[[connections]]` with `provider = "ssh"`; the plugin's settings are in `ssh.toml`.

Q164. `ssh:/<alias>` is the host's default directory; remote paths are absolute bytes.

Q165. SSH secrets are the keychain keys `ssh/<alias>/password` and `/passphrase`, only when remembered.

Q167. A listing error is a banner with *Retry* over the empty list, not a toast.

Q171. One `Acl` type with a flavour per file system, in `platform::Permissions`; no ACL library is linked.

Q172. Elevated permission changes run in the helper binary `rick-perms`, one prompt per job.

Q174. A permission job records the prior state and is one undo step; undo may prompt for elevation again.

Q177. The Permissions tab appears for local entries only.

Q178. i18n is `ext::i18n` and `ext::t!` over `fluent-bundle`; strings below `ext` stay English.

Q179. Labels handed to the registry are `ext::Text` (Fluent keys); `Action::label` returns a `String`. API 11.

Q180. Plugin translations: `crates/<plugin>/locales/<lang>.ftl`, handed to `Registry::translations`.

Q181. `check-strings.py` fails CI on a UI string literal outside `t!`; `// i18n: ok` exempts a line.

Q183. `language` in `rick.toml`; a `locales/<lang>.ftl` in the config directory overrides key by key.

Q186. Only the root window has a native drag source and a drop position.

Q188. macOS and Windows bench targets are median x 1.5 of five runs, never below the Linux target.

Q190. Compute hash shows only when every selected entry is a plain file; a symbolic link hides it.

Q191. `Manifest::previews` lists the extensions a plugin claims (`tar.gz` allowed), in `plugin.toml`. API 12.

Q192. Listing, grid and document count as `text` for the pinned card; a grid lays out 200 columns at most.

Q193. The registry lends a `SettingsStore` (`get` and `set` by extension id and key). API 12.

Q194. CI writes `library = "<path>"` into `preview-pdf.toml` and copies pdfium beside the test binaries.

Q195. *Browse…* uses `NativeDialogs::pick_files`, then `Command::ExtSetting` and `PreviewRetry`.

Q196. The Office plugin reads the PDF previewer's `library` setting; its own setting is `soffice`.

Q197. *Always preview* is the connection field `preview_always`, also for a host of `~/.ssh/config`.

Q198. *Copy* reads the file again as a job, up to 256 MiB, decoded by its BOM or as UTF-8.

Q202. A missing library is `Content::MissingLibrary`, carrying `ext::MissingLibrary`.

Q203. The `unsafe` for the 7-Zip library lives in `platform::sevenzip`, behind a safe API.

Q204. One setting, `sevenzip_library` in `vfs-archive.toml`; `ext-preview-archive` links the provider crate.

Q205. Disk images are walked as 7-Zip does: container, partition table, then file system handlers.

Q206. `Mount::mount_image` returns the `Volume`; Mount is one job; Linux sets up a loop device.

Q207. `ext-preview-markup` depends on `ext-preview-code` for the source view, as Q127 allows.

Q208. Bundled 7-Zip is built from the pinned 25.01 sources (Windows: the installer's DLL), SHA-256 checked.

Q210. `Action::app_menu()` puts an action in the burger menu, one submenu per extension. API 14.

Q211. `ActionOutcome::Later(FormLater)` and `UiHost::form_later` open a form when a job is done. API 14.

Q212. Form fields cannot react to each other; the installer derives its defaults from the chosen kind.

Q225. `density = "normal"` in an old `rick.toml` reads as Comfortable.

Q226. The accent-colour setting is withdrawn; an old `accent` key is ignored and dropped.

Q234. Settings pages: View (Files, Sorting, Side panel), Appearance, General, and one page per plugin.

Q242. Three token pairs miss WCAG contrast; the tokens stay and the contrast test names them. (provisional)

Q243. Mod+W on the last workspace tab of the last window does nothing.

Q244. *Duplicate tab* opens a new one-pane workspace at the focused folder, like Mod+T.

Q248. No Dual pane command and no F6; Split right is the way to a second pane (decided 2026-11-10).

Q249. Shown shortcuts are spelled as the key map spells them (`Mod+Shift+N`), the empty-folder hint too.

Q254. The footer takes the Figma height: the 26 px filter field plus `space/toolbar-y` above and below.

Q255. An SVG gets Fit, *Rendered* and *Copy*; no 1:1 button and no checkerboard yet. (decided 2026-11-10)

Q256. *Copy* inflates gzip first, so SVGZ and gzip-compressed text are copied as text.

Q257. Multi-variant icon containers and XPM get no 1:1 button; the hotspot toggle uses the `Tag` glyph.

Q258. `RICK_CONTRIBUTORS` names a file `build.rs` writes; release builds check out the whole history.

Q259. `about/bundled.toml` records the pdfium build of its day; `release.yml` downloads the latest.

Q261. `check-third-party.py` fails the `licenses` job when a release crate is missing from the file.

Q262. A contributor matching a fixed credits row is not repeated; `platform::os_description()` was added.

Q264. A launcher or dock click runs `rick --activate`: it only raises a running Rick; `rick` in a terminal opens the current directory (decided 2026-11-10).

Q265. A `rick://` URL takes an empty host and a local path only; `rick://apps` is an error.

Q266. The console sets `VTE_VERSION` so that shells report their directory through OSC 7.

Q267. Rick types `cd` into the console only where it can tell no job runs there (not on Windows).

Q268. One spring-loading delay, `drag.spring_delay_ms`, default 1000 ms, for tabs too. (decided 2026-11-10)

Q269. Tabs slide apart in 150 ms, eased out; at once under reduce motion.

Q270. "Exactly one pane" counts every pane of the workspace, the preview included.

Q272. A dragged tab rests on a neighbour from a quarter of overlap, short of the half where they swap.

Q273. A dragged tab's ghost is full size; it rides a title row, and elsewhere sits 8 px down and right.

Q274. A tab released neither on a tab row nor outside every window returns to its place.

Q276. Escape restores each window's workspace; *After a drag* still decides the folders drilled into.

Q277. The open hand shows over a tab's body; the closed hand stays for a whole pane or tab drag.

Q278. The core owns the integration check (`platform::Integration`); the install row is the plugin's.

Q279. Integration ships *Just me*; *All users* is disabled with *Not available yet*.

Q280. Each Linux integration item's *set up* state is read from files only.

Q281. The desktop entry and the ten hicolor icons are compiled into `platform`.

Q282. A dock favourite whose means are missing is *Unsupported*, not *missing*.

Q283. Integration record: `[integration.items.<item>]` with `state`, `checked`, `files`, `previous`.

Q284. `[integration]` keys: `ask`, `check_at_startup`, `shell_integration`.

Q285. *Reset choices* turns declined records into *missing* and sets `ask = true`.

Q286. *Apply N items* runs every row for itself; a failed row keeps its record; nothing is retried.

Q287. A differing file Rick did not write is *in the way* and is replaced only if the user agrees.

Q288. Default file manager: Rick edits `mimeapps.list` itself and records the previous values.

Q289. The `rick` shell function is a marked block in `path.sh`, which the installer carries over.

Q290. A failing `update-desktop-database` or `gtk-update-icon-cache` is logged, not an error.

Q293. The installer reads an unpacked folder (`Format::Folder`); Rick's own app id is `rick`.

Q294. Installer command `install-rick <folder> <version>`; the core runs it before the other rows.

Q295. *Install to the standard location* copies Rick and leaves the folder it ran from.

Q296. Relaunch: start `current/rick`, ask it for `--help` every 100 ms, give up after 20 s and stay.

Q297. Rick asks for `org.freedesktop.FileManager1` only once that item is set up or activated.

Q298. `rick --dbus-service` is the D-Bus activation entry; it opens no tab for the working directory.

Q299. Rick queues for `FileManager1` while another owns it and keeps the name once it has it.

Q300. `ShowItemProperties` reveals every item and opens Properties on the first folder's selection.

Q301. Dock favourite: Rick is appended; the undo removes only Rick from the list as it is by then.

Q302. Plasma: Rick is added to the first task manager, then the shell is asked to restart. (decided 2026-11-10)

Q303. COSMIC: `rick` is appended to the favourites file; a missing file stays *Unsupported*.

Q304. Every integration row starts switched on, the dock favourite included.

Q305. A row with a foreign file has a *Replace* checkbox; unticked, it is asked again at the next start.

Q306. *Don't ask again* is stored with either button; Escape is *Not now*.

Q307. The integration dialog's lines are worded from what the check found; the install row says *Copy*.

Q308. A dragged tab spring-loads only while held still, within 6 px (`drag::REST_SLOP`).

Q309. A cancelled drag undoes its workspace switches last first.

Q310. Drags between windows need screen positions; on Wayland a drag stays in its window. (decided 2026-11-10)

Q311. The window under the cursor paints the drag ghost; over the desktop only the closed hand shows.

Q313. Settings → Integration shows system, desktop and check time; there is no `rick://` links row.

Q314. Integration lines: *changed or removed since*, and *Skipped in the setup dialog on <day>*.

Q315. An undecided item has a grey *Not set up* pill and **Add**; a blocked one reads *Replace*.

Q316. `rick://` paths are canonicalised like command-line paths; the `\\?\` prefix on Windows is open.

Q317. A start with a path opens a workspace of its own; the session's Preview tab stays behind.

Q318. 0.3.0 shipped the design refresh and integration milestones 1 and 3; the other eight move on.

Q323. Issue 139 began from `dev` with the `0.4.0-dev` bump (#154) already made.

Q324. Issue 139 changed SPEC.md through a pull request that waited for Serge's review.

Q325. The Figma frame *List view mode* belongs to issue 149 and was not built in 139.

Q326. The app menu has 32 px entries; menus and cards keep 8 px corners, where Figma draws 10.

Q327. Extension submenus follow Layouts, with their group's icon or `PUZZLE_PIECE`.

Q328. A copy or move counts its top-level conflicts before it brings anything.

Q329. Two folders: *Merge folder?*, no *Keep both*, Enter is *Skip*; a file and a folder are asked as two files.

Q330. A file's Properties: *Size*, *On disk*, *Media type* under Location; *System properties…* at the left.

Q331. The *Hidden* switch of Properties shows the state and is not changed there.

Q332. The octal mode is an editable field; the special bits are set through its first digit.

Q333. With named ACL entries the grid's *Group* row is the mask; the owning group has a row of its own.

Q334. *Apply to enclosed items* is one switch for files and folders alike.

Q335. One File info card for every section; the pin is per previewer id; a click on the info button pins.

Q336. The card shows five fields as a popover and four when pinned; *Modified* follows *Size*.

Q337. *Open as hex* shows this file only; *plugin available* replaces the no-previewer text.

Q338. *Download* only opens the browser; the card shows the destination folder (decided 2026-11-10).

Q339. Preview tables: a 30 px `bg/panel` header, always striped, widths kept per previewer.

Q340. The remote limit stays 4 MiB; *Always preview* must stick, per source (decided 2026-11-10).

Q341. Shortcut categories: window commands under *Panes & tabs*, a *Search* category, one per plugin.

Q342. The actions are named *Git commit* and *SVN commit* everywhere (decided 2026-11-10).

Q343. The conflict chip is amber; a built-in action beats a plugin's; the keymap is `[shortcuts]` in `rick.toml`.

Q344. The plugin manifest gains a description; *Remove* is hidden for a built-in plugin.

Q345. An `.rkp` is a zip of the plugin's folder; install and remove ask nothing (decided 2026-11-10).

Q346. Inactive tab titles are `#2b2b2b` at 78 % on the light bands (decided 2026-11-10, for now).

Q347. Keys `titlebar_colour`, `titlebar_custom_light`, `titlebar_custom_dark`; hover on a dark band is 12 % white.

Q348. Window group: 16 px icons 12 px apart, each control answering over its 28 px step.

Q349. A console's tab reads *Console*; a failed job's toast stays until it is dismissed.

Q350. `Conflicts::ask` takes `ConflictDetails` (API 17); an extraction's dialogue has no "(n of m)".

Q351. Between two folders *Skip* is the filled button and *Merge* keeps the amber.

Q352. Type names: the format in capitals plus its family (*JPEG image*); else *BIN file*, or *File*.

Q353. *Contents* counts everything inside at any depth, links not followed; local folders only.

Q354. *Open target folder* follows the link's text; a link on a remote provider shows *Target* only.

Q355. *Media type* shows what the provider reports; the local provider reports none.

Q356. A new ACL entry gets read; the mask is recomputed as `setfacl` does; Escape closes the modal.

Q357. In the *No previewer* card a file in no family is a *Binary file*.

Q358. `PreviewWith` opens a file through one previewer until the selection changes.

Q359. The pin is `<previewer id> = true` in `[preview.pinned]`; *Destination*'s button creates the folder.

Q360. No GPS rows unless latitude and longitude are both numbers.

Q361. Preview column widths: `[preview.columns]` in `rick.toml`, per previewer id, by position.

Q362. The size-limit card is 440 px wide; download progress is reported every 128 KiB.

Q363. Shortcut rows follow the order of the spec's tables, which also resolves a conflict.

Q364. `.rkp` install: one top-level folder is the plugin; in and out by one rename; links are skipped.

Q365. *Custom* starts from Dusty rose and is kept; a bad value in `rick.toml` reads as the default.

Q366. The window's control is *Close window*; `Event::JobFailed` sends the notice to the job's window.
