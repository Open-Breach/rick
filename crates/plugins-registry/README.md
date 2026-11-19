# plugins-registry

`plugins-registry` is the client of the plugin registry
(specs/plugin-distribution.md §3 and §6): the signed list of the plugins a
release of Rick publishes, `docs/spec.md` 7.2 and 7.5 in
Open-Breach/trebuchet. It is the one place the kernel's side reaches the
network from, and all it asks for is two files and the packages the user
installs: the index, `https://get.openbreach.dev/registry/index.json`, and
its minisign signature beside it, one GET each with `rick/<version>` as
the user agent and nothing that tells one install from another. `Source` says where the
index is and which key signs it — the key compiled in from
`keys/registry.pub`, or what `[registry] url` and `public_key` of
`rick.toml` name; while that file is empty there is no key, and without a
key nothing is fetched. `fetch` hands back an index only when its signature
verifies (`minisign-verify`, the legacy form refused) and its `schema` is
one this Rick reads; `Cache` keeps the last index that verified with its
signature in `registry/` of Rick's cache directory and verifies it again on
every read; `refresh` is a fetch written to the cache, and `due` says
whether the daily check is due. `Index::offer` is the rule of what fits: a
build is for one platform and one release of Rick, compared as whole
strings, a pre-release Rick fits nothing, and neither does one older than
the registry's oldest release. `download` fetches a package to a file,
hashing it as it is written, and holds it against the size and the SHA-256
the index names: it stops as soon as more bytes arrive than were named,
and a package that is another one is deleted. Without an expectation it
checks nothing, which is *Install from URL…*: `is_package_address` is the
rule for such an address, `https://` with a host or plain `http://` on
this machine. When to ask, and whether the user allows it, is `app`'s. The `fixtures` feature adds what a test needs to stand in for
the registry — a key pair made on the spot that signs as minisign does
(`ring` and `blake2`), and a server on this machine that counts what it is
asked — and is the only use the crate has for `runtime`; no test asks the
real registry. It depends on `reqwest` over `rustls` with ring, as the
cloud plugins do, `minisign-verify`, `semver`, `sha2`, `serde` and
`serde_json`;
`app` and `tests` may depend on it.
