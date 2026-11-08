# ext-vfs-gcs

`ext-vfs-gcs` is the Google Cloud Storage provider plugin: projects as
locations under the `gcs` provider id, one folder per project at the root,
which the side panel's Cloud section lists as its rows; the project's
buckets are folders and the flat object names show as folders by prefix
below them. Listings stream page by page, a download is a streamed media
`GET`, an upload a resumable upload fed from the machine in chunks whose
session is abandoned when the writer is dropped before it is finished, so a
cancelled upload leaves no object behind; a new folder is a zero-byte
object ending in a slash; there is no rename. A move or copy within one
project, inside a bucket or across two, runs on the service as a `Rewrite`
per object, continued token by token with the bytes rewritten as the
progress, then a delete for a move, through `transfer()` under
`NATIVE_TRANSFER`, so nothing streams through the machine. Projects are
made from the Cloud section's *Add connection…* dialogue, whose fields the
plugin's `Connector` declares (project and endpoint in `rick.toml`; the
service account's key, the JSON file's contents or its path, in the
platform keychain through `platform::Secrets`; no key means
unauthenticated requests, which an emulator takes). The key signs a JWT
with ring's RSA that the token endpoint exchanges for a bearer token.
Requests travel over rustls with ring. The store is a remote-latency mount
(`REMOTE`): no thumbnails, no watch, the kernel polls.
It is a plugin (`plugins/vfs-gcs/`) that depends on `ext` and `types`
only; `rick` (behind `vfs-gcs`) and `tests` link it, and the plugin host
loads it.
