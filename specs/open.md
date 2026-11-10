# Still open

Related: specs/system-integration.md

- Extension security model: what a WASM plugin may touch (capabilities per extension point). Native plugins are settled in `specs/system-integration.md` §9: declared permissions, release-signature verification for first-party plugins, trust on first use for third-party ones.
- Testing strategy for the performance targets: a generated 200k-entry fixture and a benchmark harness in CI.
- Accessibility: egui's AccessKit support is young; screen-reader support is a known weakness to state and track.
- Design, not drawn yet (issue 60): what happens when workspace tabs outgrow the title row; the context menus for a folder and for a multiple selection; the contents of *Open with*; the permission-denied state of a pane; the look of the preview pane's content; further colour schemes beyond Light and Dark; a customisable toolbar.
- Design, not drawn yet (issue 139): the paste-conflict dialogue between two folders; the File info card unfolded to all its fields; the search field behind *Other…* in the properties dialogue's dropdowns, and the ACL table for the lists that are not POSIX; the two fields of a *Custom* title bar colour; Dark for the properties, keyboard shortcuts and extensions dialogues, which follow the variables. Each is built from the rules of this document and asked about in the pull request where a rule had to be assumed.
