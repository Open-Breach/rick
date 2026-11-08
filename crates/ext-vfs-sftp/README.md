# ext-vfs-sftp

`ext-vfs-sftp` is the SFTP provider plugin: remote hosts over SSH as
locations under the `sftp` provider id, one folder per connection at the
root, which the side panel's Network section lists as its rows. Below a
connection the host's files appear relative to the connection's folder
(the login folder by default), listed packet by packet as the server
returns them, read and written through the SFTP subsystem's file handles,
created, renamed, removed and linked in place; a move within one host is a
remote rename through `transfer()` under `NATIVE_TRANSFER`, so nothing
streams through the machine, while a copy within a host streams, since the
protocol has no copy. Connections are made from the Network section's *Add
connection…* dialogue, whose fields the plugin's `Connector` declares
(host, port, user, key file, folder in `rick.toml`; password and key
passphrase in the platform keychain through `platform::Secrets`); one SSH
session per connection opens on first use and *Disconnect* closes it.
Host keys follow the user's `known_hosts` with OpenSSH semantics: an
unknown host is learnt, a changed key refused with its fingerprint. The
host is a remote-latency mount (`REMOTE`): no thumbnails, no watch, the
kernel polls. Pure Rust through `russh` and `russh-sftp` with `ring`.
It is a plugin (`plugins/vfs-sftp/`) that depends on `ext` and `types`
only; `rick` (behind `vfs-sftp`) and `tests` link it, and the plugin host
loads it.
