# ext-search-tracker

`ext-search-tracker` connects the title-row search box to Tracker 3, the
GNOME file index: one `SearchIndex` whose scope is the whole machine. The
probe asks the session bus, through `gdbus` or `busctl`, whether the miner
`org.freedesktop.Tracker3.Miner.Files` owns its name, at start and every
ten seconds, so the index is available only while Tracker runs; a query
becomes SPARQL (the file name containing the text, the full-text index for
contents, under the root's `file://` URI when the query has one) sent to
the miner over D-Bus by the `tracker3 sparql` command line, whose
`file://` URIs are decoded, looked up on disk and narrowed by the query's
filters (QUESTIONS.md 120). The indexer is looked for at start under both of
its names: `localsearch`, as GNOME calls it since 2024, whose endpoint is
`org.freedesktop.LocalSearch3` and whose SPARQL goes through `tinysparql
query`, and `tracker3`. With neither the connector disables itself and tells
the host why through `ext::attention`, so the Extensions dialogue marks it
and the search box shows nothing. Linux only. It depends on `ext` and `types`
only; `rick` (behind `search-tracker`) and `tests` link it, and the plugin
host loads it.
