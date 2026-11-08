# ext-vfs-azure

`ext-vfs-azure` is the Azure Blob Storage provider plugin: storage accounts
as locations under the `azure` provider id, one folder per account at the
root, which the side panel's Cloud section lists as its rows; the
account's containers are folders and the flat blob names show as folders
by prefix below them. Listings stream page by page, a download is a
streamed `GET`, an upload a block upload fed from the machine whose blocks
stay uncommitted when the writer is dropped before it is finished, so a
cancelled upload leaves no blob behind; a new folder is a zero-byte blob
ending in a slash; there is no rename. A move or copy within one account,
inside a container or across two, runs on the service as a *Copy Blob* per
blob, polled for its progress and aborted on cancel where the service still
allows, then a delete for a move, through `transfer()` under
`NATIVE_TRANSFER`, so nothing streams through the machine. Accounts are
made from the Cloud section's *Add connection…* dialogue, whose fields the
plugin's `Connector` declares (account and endpoint in `rick.toml`; the
account key in the platform keychain through `platform::Secrets`). Requests
carry the Shared Key signature and travel over rustls with ring. The store
is a remote-latency mount (`REMOTE`): no thumbnails, no watch, the kernel
polls.
It is a plugin (`plugins/vfs-azure/`) that depends on `ext` and `types`
only; `rick` (behind `vfs-azure`) and `tests` link it, and the plugin host
loads it.
