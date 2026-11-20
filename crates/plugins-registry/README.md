# plugins-registry

`plugins-registry` is the client of the plugin registry
(specs/plugin-distribution.md §3 and §6): the signed list of the plugins a
release of Rick publishes, `docs/spec.md` 7.2 and 7.5 in
Open-Breach/trebuchet. It is the one place the kernel's side reaches the
network from, and all it asks for is two files and the packages the user
installs: the index, `https://get.openbreach.dev/registry/index.json`, and
its minisign signature beside it, one GET each with `rick/<version>` as
the user agent and nothing that tells one install from another. `Source`
says where the index is and which key signs it — the key compiled in from
`keys/registry.pub`, or what `[registry] url` and `public_key` of
`rick.toml` name; a build whose file is empty has no key, and without a
key nothing is fetched. `fetch` hands back an index only when its
signature verifies (`minisign-verify`, the legacy form refused) and its
`schema` is one this Rick reads; `Cache` keeps the last index that
verified with its signature in `registry/` of Rick's cache directory and
verifies it again on every read; `refresh` is a fetch written to the
cache, and `due` says whether the daily check is due (`due_after`
for a check at another interval). `Index::offer` is
the rule of what fits: a build is for one platform and one release of
Rick, compared as whole strings, a pre-release Rick fits nothing, and
neither does one older than the registry's oldest release.
`Index::newer_than` is the update notice for Rick itself: the index names
the newest release, and no request of its own is made for it. `download`
fetches a package to a file, hashing it as it is written, and holds it
against the size and the SHA-256 the index names: it stops as soon as more
bytes arrive than were named, and a package that is another one is
deleted. Without an expectation it checks nothing, which is *Install from
URL…*. `is_package_address` is the rule of which addresses are asked, and
it holds for every request of the crate: `https://` with any host, or
plain `http://` to this machine — a loopback address written as one, or
the name `localhost`, which the crate takes for the loopback address
without a lookup — so that no plain-HTTP request leaves the machine. Each
address a redirect names is held against it again before it is asked,
a redirect from another machine to this one is refused whatever its
scheme, plain HTTP never goes through a proxy, and an address that is
refused gets no request. When to ask, and whether the user allows it, is `app`'s. The `fixtures` feature adds what a test
needs to stand in for the registry — a key pair made on the spot that
signs as minisign does (`ring` and `blake2`), and a server on this machine
that counts what it is asked and can send a request on — and is the only
use the crate has for
`runtime`; no test asks the real registry. It depends on `reqwest` over
`rustls` with ring, as the cloud plugins do, `minisign-verify`, `semver`,
`sha2`, `serde` and `serde_json`; `app` and `tests` may depend on it.

What Rick suggests (specs/plugin-distribution.md §8) is read from the
cached index alone, by `app`: a suggestion calls nothing of this crate
that asks the network, and `[plugins] suggest` in `rick.toml` switches
the suggestions off without touching the registry's own switch,
`[registry] check`.

## The key

`keys/registry.pub` is Trebuchet's `keys/registry.pub`, the public half
of the key that signs the index, copied byte for byte: the comment line
that names the key's id, `197B77080066F36E`, and the base64 line. It is
compiled in, so a build has the registry on. Only the public half is
here; the secret one is a secret of Trebuchet's repository and nothing of
Rick needs it.

The test `the_key_compiled_in_is_a_minisign_public_key_and_trebuchet_s_file`
holds the file to that: it reads as a minisign public key whose id is the
comment's, and its SHA-256 is the one of Trebuchet's file, which the test
has written down with the commit it was read at. The comparison asks
nobody, so it runs with every other test; what it cannot see is a key
that Trebuchet replaces later.

A key that leaks is replaced there, and then here: the new file is copied
over this one, the test takes the new hash and id, and a release of Rick
carries it. The releases before keep refusing a list the new key signed
(Trebuchet's spec, 7.5).

A build whose `keys/registry.pub` is empty has no key and its registry is
off: the *Available* view says *The plugin list is not set up in this
build* and Rick makes no request.

## Another registry

For development, `rick.toml` points a Rick at another index and another
key; the Extensions dialogue then names the address in its footer:

```toml
[registry]
url = "http://127.0.0.1:8123/registry/index.json"
public_key = "RWQ…"   # the base64 line of a minisign public key
```

Either key alone works: another address with the build's key, or the
registry's address with another key. A `public_key` that is no key turns
the registry off; the build's own key never stands in for it. Both are
read at the start.

The tests work the same way, with the server and the key of the
`fixtures` module. The key compiled into a build is used by the binary
alone (`Source::new`, through `App::use_build_registry`); an application a
test makes knows only the key its `rick.toml` names (`Source::named`), so
no test asks the real registry, whatever `keys/registry.pub` holds.
