# ext-search-walk

`ext-search-walk` is the non-indexed subdirectory search of the spec's Main
area as a plugin: one `SearchIndex` whose scope is `Under` the local
provider's root, so it answers only queries that carry a root and streams
the entries below it whose name contains the text, found by the `ignore`
crate's parallel walker and handed over in batches as the walk proceeds,
with the pane's hidden-files toggle and the `[search] ignore` list of
`rick.toml` arriving as the query's `NoHidden` and `SkipNames` filters. The
filter bar's *Search in subdirectories* goes through it; the title-row
search box, which queries the platform indexes, leaves it alone
(QUESTIONS.md 118). In 0.1 the same walk lived in `vfs-local`. It depends
on `ext` and `types` only; `rick` (behind `search-walk`) and `tests` link
it, and the plugin host loads it.
