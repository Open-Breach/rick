# ext-action-compare

`ext-action-compare` holds the *Compare* action: with two files selected
it opens a tab with the two texts side by side, equal lines aligned, the
differing ones highlighted and one scroll for both; with two folders
selected, or with nothing selected in a window of two explorer panes, it
compares the two folders by name, size and modification time and lists
every entry as same, only on one side or differing, with a button to copy
each entry to the other folder through the kernel's copy (so the conflict
dialogue and the operations area apply). The comparison is a job on the
blocking pool that fills the `ext::Compare` the tab was opened with; the
UI draws it. Two files that are not text, or larger than 64 MiB, are
compared byte for byte and reported as identical or different; folders
are compared one level deep, a subfolder present on both sides counting as
the same. Local files and folders only. It is a plugin
(`plugins/action-compare/`) that depends on `ext` and `types` only; `rick`
(behind `action-compare`) and `tests` link it, and the plugin host loads it.
