# The README

Separate spec: `README.md`, Rick's front page, and how it is brought up to date with every release.

## Purpose

`README.md` is written for end users. A visitor understands in under a minute what Rick is, why it exists, what it does, what it looks like and how to run it. It is an overview, not documentation.

The README describes the latest release, never the state of `main` or `dev`. It is rewritten in the release pull request, the one that sets the version and writes the changelog section, so the tag is cut on a commit whose README describes that release and the archives carry that same text.

## Tone

- Terse. No padding, no marketing boilerplate, no sentence that says nothing. If a line can go without the reader missing it, it goes.
- Positive and exciting. Short, confident sentences that say what Rick does, not what it aims to do.
- No issue numbers, pull request numbers, commit hashes or changelog entries. Those are in `CHANGELOG.md` and on the release page.
- Nothing is listed that is not shipped and working in the release. Counts and lists come from the code, not from the spec or from memory.
- The whole README fits in roughly one scroll on GitHub. When it grows past that, cut. One exception: the list of previewed file types (Features, item 1) is always complete, however long it gets; it is folded instead of cut.

## Structure

The parts below, in this order. Sections 1, 2, 6, 7 and 8 are fixed text; 3, 4 and 5 are regenerated with every release.

### Title block

`# Rick`, then in this order:

1. The paragraph of section 1.
2. The **status line**, one line: the version, then `·`, then the status text, which says honestly how finished Rick is and asks for bug reports. Today the status text is:

   > Rick is early and not fully tested yet, so expect bugs. Tab navigation and some performance work are still unfinished. Please help us find what's broken and tell us what you see.

   So the line reads `v0.4.0 · Rick is early and not fully tested yet, so expect bugs. …`. The version follows the release. The status text is Serge's and stays word for word between releases. At every release the agent reads it against what the release actually ships; when a sentence looks no longer true (tab navigation is finished, say), it proposes new wording under **Needs a decision** and keeps the old text until Serge answers. It never rewrites the status text on its own.
3. The **call to action**, fixed wording with the version filled in:

   > Download v0.4.0 for Linux, macOS or Windows, try it, and tell us what breaks. Opinions welcome too, good or bad, as long as they're respectful.

   *Download* links to the Install section; *what breaks* and *Opinions* link to the issues page.

### 1. What Rick is (fixed)

Verbatim, never reworded:

> Rick is a fast, lean file manager for professionals. It runs on Linux, macOS and Windows from a single codebase and behaves the same on all three. A small core does the basics; everything else comes from an extensible plugin ecosystem.

### 2. Why Rick exists (fixed)

Heading `## Why Rick exists`. Verbatim, in the first person, Serge's voice, never reworded:

> I think AI is going to be the biggest accelerator open source has ever had. Open source always carried one handicap: it could never throw the resources at a project that a big company can. That is changing.
>
> I worked on Windows for years before moving to Linux. I expected to tinker and fight to make things work. Instead I found a polished experience that simply worked; it's a viable alternative today. But one thing from my earlier life I kept missing was Windows Explorer, a good file manager, even if there was room for improvement. Then I started using macOS more and more, and I don't like Finder either.
>
> I also believe more and more people will move to Linux, and those coming from Windows are used to the mouse and a graphical interface, not just the console. They'll need a powerful file manager. So I decided to build my own. That's how Rick came about.

### 3. Features (regenerated)

Heading `## Features`. An overview of six items, always in this order, one or two lines each. It is not a tour of every function.

Items 1 and 3 are **live**: they are generated from what the release's bundle registers, never typed by hand. Items 2, 4, 5 and 6 are **fixed wording**, touched only when what they describe changes.

1. **Previews** (live). One line with the real number of previewable file types: `Previews <n> file types, listed by kind below.` Every type is then listed by its short name, complete, grouped by kind (see *File types* below). The number and the list are what the previewers of the bundle, the core's and the plugins', declare.
2. **Interface** (fixed). `Multi-pane, multi-tab view: split panes side by side, drag them around, keep as many tabs as you like.`
3. **Storage and search** (live). The file systems and storage providers Rick browses, the cloud providers among them, and the platform search it queries on each OS. The lists are what the providers and search connectors of the bundle declare.
4. **Tool integration** (fixed wording, live names). `Works with the tools you already use, like Git: status badges in the file list and actions in the context menu.` A tool is named only when its plugin ships in the archives and loads.
5. **Tarball manager** (fixed). `Installs and uninstalls software distributed as tarballs, so a tar.gz download becomes a managed install instead of a folder you forget about.` Marked as Linux only for as long as `ext-installer` is.
6. **Convenience functions** (fixed). `Small things done right, such as computing a file's hash from the context menu.`

**File types** (live). Directly under the six items, the full list of item 1, one collapsible `<details>` block per kind, its `<summary>` naming the kind and its count, the types comma-separated inside. The kinds, in this order:

- **Media**: images, icons, audio and video.
- **Documents**: PDF, Office documents, eBooks, Markdown, HTML, CSV and plain text.
- **3D**: 3D and 3D-printing models (STL, OBJ, 3MF, …).
- **Programming**: source code, and data and config formats (JSON, YAML, TOML, …).
- **Archives and disk images**.

**Media** starts open (`<details open>`); the others start folded. A type belongs to exactly one kind. The kind is assigned by the previewer that declares the type, in the generator; a declared type without a kind fails the README test rather than landing in a catch-all. A new kind is added only when Serge agrees, proposed under **Needs a decision**. This list is exempt from the length rule under *Tone*: it is never shortened.

Under the file types a **Coming** list of three to five entries. The agent proposes it in the release pull request under **Needs a decision**, beside the previous list; Serge decides whether it stays, changes or grows. The agent never changes this list on its own.

### 4. Screenshot (regenerated)

One picture of the release's GUI, taken fresh for every release.

- It shows a typical two-pane window with the preview pane open on a sample 3D model of an animal: original, low-poly and cute, made for this repository and small enough to live in it. No third-party or licensed model. A 3D preview is the point: neither Explorer nor Finder does it.
- **A new animal every release.** The agent picks an animal at random, one not used in an earlier release, and generates its model with a script beside it, as `assets/readme/robot-dog.py` made `assets/readme/robot-dog.stl` for 0.4.0. The new model replaces the previous one in `assets/readme/`, so the folder holds the current release's animal only; the alt text of the picture names it. The animal is listed in the release pull request under **Decisions made**.
- Two renders of the same window, size and state: Light as `docs/screenshots/readme-light.png`, Dark as `docs/screenshots/readme-dark.png`. Both are committed.
- The README embeds them in one `<picture>` element with a `prefers-color-scheme: dark` source, so a visitor sees the one that matches their GitHub theme.
- They are rendered headlessly by the screenshot harness the repository already has (`crates/tests/tests/screenshots.rs`, feature `screenshots`), from a fixture folder and a fixture configuration at a fixed window size, so two runs on the same code give the same picture.

### 5. Install (regenerated)

Heading `## Install`. Three short blocks, **Linux**, **macOS**, **Windows**. Each has the direct link to the release's archive for every architecture built for that platform (`https://github.com/Open-Breach/rick/releases/download/v<version>/rick-<version>-<platform>.<ext>`) and the real steps to get Rick running, as they are in that release: two or three lines, not the reference. Today that is unpack and run; when an installer or a package arrives the steps change with it.

One line at the end says how plugins arrive: they ship in the archive's `plugins/` folder, and deleting a plugin's folder removes that plugin.

### 6. Warning (fixed)

The *Use with caution* block of the README as it stands, unchanged, until Serge takes it out.

### 7. Join in (fixed)

The *Join in* section of the README as it stands, unchanged.

### 8. Portal animation (fixed)

Directly after *Join in*, before the closing lines: Rick's portal animation, the looping portal burst (rings with folder and file icons spiralling out) from the design refresh, centred, with alt text that describes it. Serge commits the file himself under `assets/`; the agent never draws, generates or fetches it, and only embeds what is there, in a format GitHub plays inline in a README (animated GIF, WebP, PNG or SVG). If the file is not in `assets/` when the release is cut, the agent leaves this section out and says so under **Needs a decision**.

### Closing lines

- One line of links to what left the README: building from source, the command line, the architecture, the changelog.
- The licence line: Apache-2.0 with a link to `LICENSE`.
- The closing line about the name stays as it is.

## What leaves the README

The current README's tour (*See what's what*, *Then put it in order*, *One tool, your modules*, *Fast is a feature*), *Architecture in one breath*, the long archive description, the Install and Command line paragraphs, the version history and *To build from source* are not in the structure above. Nothing of it is deleted from the repository: what is still true and not already in a spec or in `CHANGELOG.md` moves to `docs/` (building from source, the command line, the architecture), and the closing links point there.

## Rules for the release

- The README update is part of the release pull request and of nothing else. Between releases the README is not touched, however far `dev` moves.
- Regenerated: the version in the status line and the call to action, feature items 1 and 3 with the grouped file types and the tool names of item 4, the animal and the two screenshots, the Install section.
- Left alone: sections 1, 2, 6, 7 and 8, the closing lines, the status text, and the fixed wording of feature items 2, 4, 5 and 6. A change to any of them is listed in the pull request under **Decisions made**.
- Proposed, not decided: the Coming list, and new wording for the status text when a sentence of it looks no longer true, both under **Needs a decision**.
- The README is checked, not trusted: a test fails when the live lists of items 1 and 3 differ from what the bundle registers, when a previewed type has no kind, and when the status text is not the spec's word for word; the release workflow fails when the version in the status line is not the tag's.
- The first release after this revision brings the generator (`crates/tests/src/readme.rs`, `cargo xtask readme`), the README test (`crates/tests/tests/readme.rs`) and the release check to this structure in the same release pull request. Until then the README stays as 0.4.0 left it.
- When the README cannot be made truthful (a type a previewer declares does not preview, an archive of the release is missing), the agent stops and says so under **Needs a decision**. It does not adjust the text to fit.
