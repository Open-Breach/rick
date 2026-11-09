# ext-search-spotlight

`ext-search-spotlight` connects the title-row search box to Spotlight, the
macOS metadata index: one `SearchIndex` whose scope is the whole machine.
The probe runs `mdutil -s /` at start and every ten seconds, so the index
is available while indexing is enabled on the root volume; a query runs
`mdfind` (`-name` for a name query, the text itself for names and
contents, a `kMDItemTextContent` query for contents alone, `-onlyin` the
root when the query has one), stops at 500 paths, and the absolute paths
it prints are looked up on disk and narrowed by the query's filters
(QUESTIONS.md 120). macOS only. It depends on `ext` and `types` only;
`rick` (behind `search-spotlight`) and `tests` link it, and the plugin
host loads it.
