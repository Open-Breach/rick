# Rick — S3 connection, connection GUI and connection test

Status: binding from 2026-11-26, for release 0.6.1. Drafted on 2026-11-19 and deferred until 0.6.0 was out. It has three parts, each an issue against `dev`. The order of work is Part 1, the S3 connection (issue #309); then Part 3, the connection test (issue #310); then Part 2, the connection GUI (issue #311). Part 3 was added on 2026-11-26, and the parts kept their numbers.

Related: specs/scope.md, specs/gui-look.md, specs/gui-dialogues.md, specs/gui-side-panel.md, specs/gui-main-area.md, specs/plugins.md, specs/plugin-distribution.md, specs/ext-ssh.md

## Background

An S3 connection to Cloudflare R2 with a bucket-scoped token ("Object Read & Write" or "Object Read only") shows an empty root and a bare "permission denied". The investigation found:

- Credentials and signing are accepted. R2 answers `403 AccessDenied` to `ListBuckets`, because bucket-scoped tokens may work inside their bucket but may not list the account's buckets.
- Rick always fills the account root from `ListBuckets` and the connection has no bucket field, so such a token can never show anything.
- Every 403 is folded into the same "permission denied", whether the service said `AccessDenied`, `InvalidAccessKeyId` or `SignatureDoesNotMatch`, so the cause cannot be told from the message.
- A bucket name added to the end of the endpoint is silently dropped, so that does not get in either.

This is general S3 behaviour, not an R2 quirk: listing buckets is an account-level permission, separate from access inside one bucket.

Confirmed in the app on 2026-11-19: with the plain endpoint, typing `s3:/<connection>/<bucket>` in the address bar opens the bucket, so a bucket-scoped token works inside its bucket. With such a token that is the only way in before 0.6.1.

A second gap, named on 2026-11-26: a cloud connection is saved without any check. A wrong endpoint, a wrong key or a mistyped bucket name shows only later, when the connection is opened. Part 3 closes it.

## Part 1 — S3 connection

### 1.1 Buckets field

- New optional field *Buckets* on the S3 connection: one text field, bucket names separated by commas.
- Parsing: split on commas, trim whitespace, ignore empty entries, drop duplicates, keep the order typed.
- When the field has names, the connection root shows exactly those names as folders, as typed. Showing the root sends no request: Rick does not call `ListBuckets` for it and does not check the names there.
- When the field is empty, behaviour is as today: the root is filled from `ListBuckets`.
- The names are checked when the connection is added, saved or tested (Part 3). The rule of 2026-11-19, that a name is never checked up front, was revoked on 2026-11-26.
- A name that was saved all the same, through *Save anyway*, is reported when it is opened, with the service's real reason (see 1.2).
- The names are stored on the connection entry in `rick.toml` and are optional. They are kept as the text typed, through a text field of the connection: no new field kind and no TOML list. Existing connections without them load and behave unchanged.
- Going straight to a bucket through the address bar (`s3:/<connection>/<bucket>`) keeps working whether or not that bucket is in the list.
- *Connect* on a saved connection lists the buckets when *Buckets* is empty, as today, so wrong credentials are reported at once. With *Buckets* filled it sends no request.

Bucket path pasted onto the endpoint:

- Before 0.6.1 the client replaces the endpoint's path with its own on every request. A bucket name added to the endpoint is silently dropped, and Rick still calls `ListBuckets`.
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
- The fields of the dialogue, in order: Endpoint, the recognised provider, Region, Access key, Secret key, Buckets, Path-style addressing.

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

The style guide below is the project's own: *Look and feel* in `specs/gui-look.md` and the rules every dialogue follows at the top of *Dialogues* in `specs/gui-dialogues.md`. This part points at them and does not restate them. It is the last of the three parts to be built.

### 2.1 Connection dialogues

- All cloud provider and network provider connection dialogues are still off the style guide: cramped and not nice. Bring every one of them in line with it.
- *Add connection…* and *Edit…* are one dialogue, built from each plugin's fields, so it is brought on the guide once, for S3, Azure Blob, Google Cloud Storage and SSH. The S3 dialogue, with its recognised-provider label and its test, is the one to check first.
- The *Test* button and the list of its results (Part 3) belong to the dialogue and follow the guide like the rest.
- No behaviour of a connection changes in this part.
- The dialogue gets an entry of its own under *Dialogues* in `specs/gui-dialogues.md`, written by the task that builds it.

### 2.2 Cloud connections view

- The virtual view that lists all cloud connections, the Cloud overview at `rick://cloud`, does not follow the guidelines either. Bring it in line.
- Its columns behave like the normal details view (*File list* in `specs/gui-main-area.md`): resizable by dragging, and sizing themselves automatically to their content.
- Reuse the details view's column behaviour rather than building a second implementation.

### 2.3 Acceptance

- Every cloud and network connection dialogue has been checked against the style guide.
- Columns in the connections view resize by dragging and fit themselves to content, the same as in the details view.

## Part 3 — Connection test

Decided on 2026-11-26. It is built after Part 1, whose error codes it uses, and before Part 2.

### 3.1 The Test button

- The *Add connection…* and *Edit…* dialogues of the cloud providers (S3, Azure Blob, Google Cloud Storage) get a **Test** button.
- It tests what is in the form at that moment. Nothing is saved first. In *Edit*, a secret left empty is the one in the keychain.
- The result is a short list of checks in the dialogue, in the order they ran. Each is passed, failed or not run. A failed check shows the service's own code and reason, as 1.2 has it.
- A check that depends on a failed one is not run.
- SSH has no *Test* button for now, and a saved connection has no *Test* entry in its context menu.

### 3.2 The checks

| Provider | Checks, in order |
| --- | --- |
| S3 | 1. The host is reachable. 2. The credentials are accepted. 3. The key may list buckets. 4. Each name in *Buckets* can be opened: accessible, denied or missing, per name. |
| Azure Blob | 1. The host is reachable. 2. The credentials are accepted. 3. The account's containers can be listed. |
| Google Cloud Storage | 1. The host is reachable. 2. The credentials are accepted. 3. The project's buckets can be listed. |

The names in *Buckets* are checked whether or not the key may list buckets, so a mistyped name is caught either way.

### 3.3 When a test is good

- Azure Blob and Google Cloud Storage: all three checks passed.
- S3, always: the host is reachable and the credentials are accepted.
- S3 with names in *Buckets*: every name can be opened. Whether the key may list buckets is then shown as information and is not a failure.
- S3 with *Buckets* empty: the key may list buckets. If it may not, the test is not good, and the message is the one of 1.2 with the hint to add bucket names.

### 3.4 Saving

- *Add* and, in *Edit*, *Save* run the test first.
- A good test saves and closes, as before.
- A test that is not good keeps the dialogue open, shows the results with a warning, and offers **Save anyway** beside going back to the fields. Nothing is saved until the user chooses.
- An unreachable host is not good, like any other failure. *Save anyway* is how a connection is added while offline.

### 3.5 Limits

- The test only reads. It creates, changes and deletes nothing on the service.
- Each check gives up after 10 seconds, and the test can be cancelled. The dialogue stays usable while it runs.
- A secret typed in the form goes to the plugin's test and nowhere else: not into a file, not into a log.
- Browsing does not change. The root of an S3 connection with *Buckets* filled still shows the names as typed, without a request (1.1), and *Connect* on a saved connection stays as 1.1 has it.

### 3.6 Acceptance

- R2 with a bucket-scoped key and the bucket named: the test is good, and the list-buckets line shows as information.
- The same key with *Buckets* empty, a wrong secret, a mistyped bucket name and an unreachable host each give a test that is not good, with their own line and reason.
- *Add* with a test that is not good saves nothing until *Save anyway* is chosen, and saves nothing when the user goes back to the fields.
- An admin key with *Buckets* empty: the test is good.
- Azure Blob and Google Cloud Storage: a good test and a test that is not good, each.
- After a test, a typed secret is in no file and in no log line.

## Extension point

Parts 1 and 3 each need something the connection extension point does not have: the dialogue is one generic form built from a plugin's fixed list of fields, and a connection is saved without asking the plugin anything.

- Part 1: the dialogue asks the plugin after each edit, and the plugin may fill fields the user has not edited by hand and put a short note by a field. The recognition table lives in `ext-vfs-s3`, not in the dialogue.
- Part 3: each plugin says what its test checks and runs it on the values of the form. A plugin without a test shows no *Test* button and is saved as before.
- Each of the two changes raises the extension API by one: Part 1 makes it 24, Part 3 makes it 25. A plugin that uses neither changes only its API version.
- Decided on 2026-11-26: both changes ship in 0.6.1, a patch release. A plugin build fits one Rick release anyway (`specs/plugin-distribution.md`), so every plugin is published again with it.
- The shape of each change is proposed by the pull request that makes it. A docs pull request then records it here and where `specs/plugins.md` describes the versioned contract.

### API 24: a form that can react

Recorded from pull request #315 (issue #309, task 3), as built.

- `Connector` has one more method, with a default that does nothing: `react(&self, form: &ConnectionForm) -> ConnectionReaction`. A plugin that does not override it changes only its API version.
- The dialogue asks when it opens, after each edit, when a text field is left, and once more before it saves. `react` runs between two frames: it reads the form and answers, and does no I/O.
- `ConnectionForm` is the dialogue as it stands:
  - `values`: the fields by key, as typed. Secret fields are not in it; a secret reaches no reaction, and a reaction sets none.
  - `edited`: the keys of the fields the user edited by hand. In *Edit…* a value that was saved counts as edited; a field saved empty does not.
  - `changed`: the field just edited; none when the dialogue opens or is about to save.
  - `settled`: whether the input is whole. It is not while the user types in the field. It is after a paste, when the field is left, for a checkbox or a choice, when the dialogue opens and before it saves.
- `ConnectionReaction` is the plugin's answer, three lists by field key:
  - `fill`: values the plugin settles, such as the region of a recognised host. The dialogue takes one only for a field the user has not edited by hand. That is the rule of 1.3, and the dialogue enforces it, not the plugin. A filled field stays unedited, so a later reaction can settle it again.
  - `rewrite`: the user's own input moved into the fields it belongs to, such as a bucket address pasted as the endpoint (1.1). The dialogue takes it whatever the field's state, and the fields count as edited afterwards. A plugin rewrites only settled input: taking a half-typed address apart would move the text under the cursor.
  - `notes`: a short text by a field, already worded in the user's language, such as the recognised provider. The notes of one reaction replace those of the one before. A note has the row under its field.
- The S3 plugin's `react` is in `crates/ext-vfs-s3/src/connection.rs`, and its recognition table in `crates/ext-vfs-s3/src/recognition.rs`. The worked example is in the crate docs of `ext::connection`.

### API 25: a connection test

Recorded from pull request #320 (issue #310, task 1), as built. The S3 test is pull request #322, the Azure Blob and Google Cloud Storage tests #323.

- `Connector` has two more methods, both with defaults: `checks(&self) -> Vec<String>` and `async test(&self, form: &ConnectionTestForm, cancel: &CancelToken) -> ConnectionTest`. A plugin that overrides neither has no test: its dialogue shows no *Test* button and it is saved as before. It changes only its API version.
- `checks` says what the test checks, in order, in the user's language. The dialogue lists these while the test runs. A list that is not empty is what gives the dialogue its *Test* button.
- `ConnectionTestForm` is what is in the dialogue at that moment:
  - `name`: the saved connection, in *Edit…*. A secret field left empty then stands for the one in the keychain. None in *Add connection…*.
  - `values`: every field by key as typed, the secrets included. Its debug text names the fields and never a value.
- `ConnectionTest` is the result: `checks`, in the order they ran, and `good`, which the plugin decides, because the rule differs by provider (3.3).
- A `ConnectionCheck` has a `label`, an `outcome` and a `detail`. The outcome is passed, failed, information or not run. The detail is the service's own code and reason, as 1.2 has it; it is empty for a check that passed or was not run. Information is for a check that did not pass where that is not a failure, such as the list-buckets line of S3 with buckets named.
- The contract of `test`: it only reads; each check gives up after `CHECK_TIMEOUT`, ten seconds; a check that depends on a failed one is not run; it ends early when `cancel` is cancelled; a typed secret is written to no file, no keychain and no log.
- The application runs the test as a job (`Command::TestConnection`), which the dialogue or the operations area can cancel. The result comes back whole when the test ends, under the dialogue's request number. Nothing is saved by it.
- The requests behind the checks:
  - S3: `ListBuckets` answers checks 1 to 3. Each named bucket is checked with `ListObjectsV2` for a single key, which gives the service's code where a `HEAD` would give none.
  - Azure Blob: `List Containers` for a single name answers all three.
  - Google Cloud Storage: `buckets.list` for a single name without a token answers check 1; the sign-in at the token endpoint and the same request with the token answer checks 2 and 3.
- The worked example is in the crate docs of `ext::connection`.

## Code pointers

Not part of the decisions above: where each point lands in the code, read on `dev` at `f28c813` (2026-11-26, v0.6.0). Lines move.

- The account root: `list()` in `crates/ext-vfs-s3/src/provider.rs`, the `Place::Account` arm, which calls `list_buckets()` (line 479). The same file calls it once more for a listing of unlimited depth beneath an account (line 357); with *Buckets* filled that one goes through the names too.
- *Connect*: `connect()` in `crates/ext-vfs-s3/src/connection.rs` verifies with `list_buckets()` (line 196).
- The folded 403s: `failed()` in `crates/ext-vfs-s3/src/client.rs` (line 67).
- The dropped endpoint path: `target()` in the same file, `url.set_path(&path)` (line 412).
- The fields of the S3 connection: `schema()` in `crates/ext-vfs-s3/src/connection.rs`.
- The extension point: `Connector`, `ConnectionSchema` and `ConnectionFieldKind` in `crates/ext/src/connection.rs`; `API_VERSION` in `crates/ext/src/registry.rs`.
- The dialogue: `ConnectionDialog` and `connection_dialog()` in `crates/ui-egui/src/side_panel.rs`. It is one generic dialogue, and S3, Azure Blob, GCS and SSH all go through it. Saving is `Command::AddConnection` and `Command::EditConnection`, handled in `crates/app/src/lib.rs`; nothing is tested there before 0.6.1.
- The Cloud overview's table: `cloud_table()` in `crates/ui-egui/src/overview.rs`, which the connections of the Network overview share. The details view's columns are the shared table in `crates/ui-egui/src/table.rs`.
