# ext-search-baloo

`ext-search-baloo` connects the title-row search box to Baloo, the KDE
file index: one `SearchIndex` whose scope is the whole machine. The probe
runs `balooctl6 status`, or `balooctl status` on KDE Frameworks 5, at
start and every ten seconds, so the index is available only while the file
indexer runs, and remembers which generation answered; a query runs
`baloosearch` (or `baloosearch6`) with `filename:` terms for a name query
and the text itself for contents, under the root with `-d`, capped at 500
results, and the absolute paths it prints are looked up on disk and
narrowed by the query's filters (QUESTIONS.md 120). Linux only. It depends
on `ext` and `types` only; `rick` (behind `search-baloo`) and `tests` link
it, and the plugin host loads it.
