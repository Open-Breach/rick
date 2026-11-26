# ext-vfs-s3

`ext-vfs-s3` is the S3 provider plugin: Amazon S3, Cloudflare R2, MinIO and
every other S3-compatible store as locations under the `s3` provider id,
one folder per account at the root, which the side panel's Cloud section
lists as its rows; the account's buckets are folders and the flat keys show
as folders by prefix below them. Listings stream page by page, a download
is a streamed `GET`, an upload a multipart upload fed from the machine and
aborted when the writer is dropped before it is finished, so a cancelled
upload leaves no object behind; a new folder is a zero-byte key ending in a
slash; there is no rename. A move or copy within one account, inside a
bucket or across two buckets, runs on the service as a `CopyObject` per
object (and a delete for a move) through `transfer()` under
`NATIVE_TRANSFER`, so nothing streams through the machine. Accounts are
made from the Cloud section's *Add connection…* dialogue, whose fields the
plugin's `Connector` declares (endpoint, region, access key, the optional
bucket names and the addressing style in `rick.toml`; the secret key in the
platform keychain through `platform::Secrets`). A connection that names its
buckets shows those names at its root without asking the service, which is
how a key that may not list buckets gets in. Requests carry Signature Version 4 and travel
over rustls with ring. The store is a remote-latency mount (`REMOTE`): no
thumbnails, no watch, the kernel polls.
It is a plugin (`plugins/vfs-s3/`) that depends on `ext` and `types` only;
`rick` (behind `vfs-s3`) and `tests` link it, and the plugin host loads it.
