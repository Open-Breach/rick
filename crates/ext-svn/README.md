# ext-svn

`ext-svn` is the Subversion extension, the same shape as `ext-git`: a
Decorator that shows status badges on entries with an SVN column next to
them, a roll-up mark on folders with changes inside, and the working copy's
revision, URL and last commit in the info panel, all read through `svn
status --xml` and `svn info --xml` once per working copy and cached until
the `.svn` directory or the working copy changes; and the "SVN" action
group (Update, Commit, Revert, Log, Add), available only inside a working
copy and run through the `svn` command line as jobs that drop the cache
when they finish. The crate builds as a `cdylib` that `plugins-native`
loads from `plugins/svn/` through the entry point `ext::plugin!` declares,
and as an `rlib` for the binary's opt-in `svn` feature and the tests, which
drive a working copy generated with `svnadmin` and `svn` and skip with a
note where the command line is missing (`RICK_REQUIRE_SVN=1` makes them
fail instead). It depends on `ext` and `types` only; `rick` (behind `svn`)
and `tests` link it, and the plugin host loads it.
