# Rick — S3 connection and connection GUI

Status: drafted 2026-11-19, deferred. Not started and not binding yet: the work begins once the release now in progress, which reconfigures the plugins, is out. Until then this file is not in the `SPEC.md` index and no issue lists it. It has two parts, meant to be opened as two separate tasks against `dev`. The task that starts a part adds this file to the index and brings the sections it changes in line: *Object storage* in `specs/scope.md`, *Dialogues* in `specs/gui-dialogues.md`, and *Cloud* and *Virtual locations* in `specs/gui-side-panel.md`.

Related: specs/scope.md, specs/gui-look.md, specs/gui-dialogues.md, specs/gui-side-panel.md, specs/gui-main-area.md, specs/ext-ssh.md

## Background

An S3 connection to Cloudflare R2 with a bucket-scoped token ("Object Read & Write" or "Object Read only") shows an empty root and a bare "permission denied". The investigation found:

- Credentials and signing are accepted. R2 answers `403 AccessDenied` to `ListBuckets`, because bucket-scoped tokens may work inside their bucket but may not list the account's buckets.
- Rick always fills the account root from `ListBuckets` and the connection has no bucket field, so such a token can never show anything.
- Every 403 is folded into the same "permission denied", whether the service said `AccessDenied`, `InvalidAccessKeyId` or `SignatureDoesNotMatch`, so the cause cannot be told from the message.
- A bucket name added to the end of the endpoint is silently dropped, so that does not get in either.

This is general S3 behaviour, not an R2 quirk: listing buckets is an account-level permission, separate from access inside one bucket.

Confirmed in the app on 2026-11-19: with the plain endpoint, typing `s3:/<connection>/<bucket>` in the address bar opens the bucket, so a bucket-scoped token works inside its bucket. With such a token that is the only way in today.

## Part 1 — S3 connection

### 1.1 Buckets field

- New optional field *Buckets* on the S3 connection: one text field, bucket names separated by commas.
- Parsing: split on commas, trim whitespace, ignore empty entries, drop duplicates, keep the order typed.
- When the field has names, the connection root shows exactly those names as folders. Rick does not call `ListBuckets` and does not check the names up front.
- When the field is empty, behaviour is as today: the root is filled from `ListBuckets`.
- A mistyped or forbidden bucket is reported only when it is opened, with the server's real reason (see 1.2).
- The names are stored on the connection entry in `rick.toml` and are optional. Existing connections without them load and behave unchanged.
- Going straight to a bucket through the address bar (`s3:/<connection>/<bucket>`) keeps working whether or not that bucket is in the list.

Bucket path pasted onto the endpoint:

- Today the client replaces the endpoint's path with its own on every request. A bucket name added to the endpoint is silently dropped, and Rick still calls `ListBuckets`.
- New behaviour: when the endpoint carries a path, Rick takes the bucket name from it, adds it to *Buckets*, and keeps the endpoint without the path. Pasting the bucket address that Cloudflare's dashboard shows then just works.
- The dialogue shows this as it happens: the endpoint field is left clean and the name appears in *Buckets*. Nothing is dropped silently.
- A connection already saved with a path on its endpoint is treated the same way when it is loaded.

### 1.2 Error messages

Keep the service's error code instead of folding every 403 into "permission denied". The code appears in the message, and these cases get their own wording:

| Situation | The message says |
| --- | --- |
| `AccessDenied` on `ListBuckets` | This key is not allowed to list buckets. Add the bucket name or names in the connection's Buckets field. |
| `AccessDenied` anywhere else | Access to this bucket or object was denied. |
| `InvalidAccessKeyId` | The access key is not recognised. |
| `SignatureDoesNotMatch` | The signature was rejected, usually a wrong secret key. |
| `NoSuchBucket` | The bucket does not exist; check the name. |
| Anything else | The service's code and message as returned. |

### 1.3 Provider recognition from the endpoint

- No provider dropdown. Rick recognises the provider from the endpoint host as it is typed or pasted, and adjusts the form.
- A small label by the endpoint shows what was recognised, for example *Cloudflare R2*, so the behaviour is visible.
- Recognition only fills fields the user has not edited by hand. It never overwrites a typed value. Fields it settles, such as region and path style on R2, stay visible, pre-filled and editable; nothing is hidden.
- An empty endpoint means AWS, where only the region matters. An Amazon address pasted in is recognised as AWS too.
- An unknown host shows the full form as today (generic S3-compatible, such as MinIO or Ceph).
- Recognition is a small table: host pattern, provider name, region rule, path-style default. Adding a provider means adding a row.

First table; every provider in it is confirmed for the first cut. The patterns and defaults are written from memory and must be checked against each provider's documentation before they are hard-coded.

| Provider | Endpoint host pattern | Region | Path style |
| --- | --- | --- | --- |
| AWS | empty, or `s3.<region>.amazonaws.com` | entered by the user; read from the host when present | off |
| Cloudflare R2 | `<account-id>.r2.cloudflarestorage.com`, also `<account-id>.eu.r2.cloudflarestorage.com` | `auto` | on |
| Backblaze B2 | `s3.<region>.backblazeb2.com` | from the host | on |
| Wasabi | `s3.<region>.wasabisys.com` | from the host | on |
| DigitalOcean Spaces | `<region>.digitaloceanspaces.com` | from the host (verify) | on |
| Hetzner | `<location>.your-objectstorage.com` | from the host | on |
| Scaleway | `s3.<region>.scw.cloud` | from the host | on |
| OVH | `s3.<region>.io.cloud.ovh.net` | from the host | on |
| Google Cloud Storage, through its S3-compatible API (the native provider is `ext-vfs-gcs`) | `storage.googleapis.com` | `auto` | on |
| Unknown | anything else | entered by the user, default `us-east-1` | on |

### 1.4 Acceptance

- R2 connection with a bucket-scoped token and the bucket named in *Buckets*: the root shows the bucket, and opening it lists its objects.
- Same token with *Buckets* empty: the root shows the list-buckets message with the hint, not a bare "permission denied".
- A wrong access key, a wrong secret and a non-existent bucket each produce their own message with the service's code.
- Admin token with *Buckets* empty: the root lists all buckets as today.
- Pasting an R2 endpoint sets region `auto` and path style on and shows *Cloudflare R2*; a hand-typed region is left alone.
- Pasting an R2 endpoint ending in `/packages` leaves the endpoint clean, puts `packages` in *Buckets*, and the root opens showing that bucket.
- Existing connections in `rick.toml` open unchanged.

## Part 2 — Connection GUI tidy-up

The style guide below is the project's own: *Look and feel* in `specs/gui-look.md` and the rules every dialogue follows at the top of *Dialogues* in `specs/gui-dialogues.md`. This part points at them and does not restate them.

### 2.1 Connection dialogues

- All cloud provider and network provider connection dialogues are still off the style guide: cramped and not nice. Bring every one of them in line with it.
- The S3 dialogue is rebuilt in Part 1 anyway. Do it to the style guide first and use it as the model for the others.
- No functional change to the other dialogues in this task.

### 2.2 Cloud connections view

- The virtual view that lists all cloud connections, the Cloud overview at `rick://cloud`, does not follow the guidelines either. Bring it in line.
- Its columns behave like the normal details view (*File list* in `specs/gui-main-area.md`): resizable by dragging, and sizing themselves automatically to their content.
- Reuse the details view's column behaviour rather than building a second implementation.

### 2.3 Acceptance

- Every cloud and network connection dialogue has been checked against the style guide.
- Columns in the connections view resize by dragging and fit themselves to content, the same as in the details view.

## Code pointers

Not part of the decisions above: where each point lands in the code, read on `dev` at `f80ea84` (2026-11-19). The release in progress may move them.

- The account root: `list()` in `crates/ext-vfs-s3/src/provider.rs`, the `Place::Account` arm, which calls `list_buckets()` (lines 477–479).
- The folded 403s: `failed()` in `crates/ext-vfs-s3/src/client.rs` (lines 73–76).
- The dropped endpoint path: `target()` in the same file, `url.set_path(&path)` (line 411).
- The fields of the S3 connection: `schema()` in `crates/ext-vfs-s3/src/connection.rs`. A `Text` field keeps *Buckets* in `rick.toml` as the text typed; a TOML list would need a new field kind.
- *Add connection…* is one generic dialogue built from each plugin's `ConnectionSchema` (`crates/ext/src/connection.rs`), and S3, Azure Blob, GCS and SSH all go through it. The schema is a fixed list of fields, so the form reacting to the endpoint (1.3, and the pasted path in 1.1) needs something it does not have today. That changes an extension point, a major decision under `CLAUDE.md`: the task lists it under **Needs a decision**.
- The Cloud overview's table: `cloud_table()` in `crates/ui-egui/src/overview.rs`.
