# ext-search-windows

`ext-search-windows` connects the title-row search box to Windows Search,
the system index: one `SearchIndex` whose scope is the whole machine. The
probe runs `sc.exe query WSearch` at start and every ten seconds, so the
index is available while the service runs; a query becomes Windows Search
SQL (`System.FileName LIKE '%text%'` for a name query, `CONTAINS` for
contents, `SCOPE='file:…'` for the root, `TOP 500`) that a PowerShell
script read from its standard input runs through an ADODB connection to
the `Search.CollatorDSO` OLE DB provider, and the paths it prints are
looked up on disk and narrowed by the query's filters; the COM interfaces
themselves are `unsafe` calls a plugin may not make (QUESTIONS.md 120).
Windows only. It depends on `ext` and `types` only; `rick` (behind
`search-windows`) and `tests` link it, and the plugin host loads it.
