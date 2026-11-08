# ext-action-rename

`ext-action-rename` holds *Bulk rename* (Mod+F2, and *Rename* with
several entries selected): a form with a pattern (`{name}` the name
without its extension, `{ext}` the extension with its dot, `{n}` a counter
padded to the count's width or `{n:4}` to four digits, `{date}` the
modification date as `YYYY-MM-DD`), find and replace (plain text or a
regular expression with `$1` groups), a case change (lower, UPPER, Title),
the counter's start, and a live preview of every resulting name that marks
the problems: an empty name, a separator in it, two entries landing on one
name, or a name another entry of the folder already has. The form cannot be
confirmed while a problem remains. The renames run as one undoable job,
through temporary names so swaps and case-only changes on a
case-insensitive file system work, on local folders. It is a plugin
(`plugins/action-rename/`) that depends on `ext` and `types` only; `rick`
(behind `action-rename`) and `tests` link it, and the plugin host loads it.
