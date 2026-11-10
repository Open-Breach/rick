# Rick — SSH Plugin (ext-ssh)

## Purpose

Most of what people browse remotely is a Linux server, a NAS, a Raspberry Pi, a container or a router, and all of those have one thing in common: a shell over SSH. `ext-ssh` lets Rick browse and operate on such a machine as if it were a folder, with the hosts the user already has in `~/.ssh/config` showing up without any setup, a console on the host one action away, and bastions (jump hosts) handled as a matter of course.

The plugin deliberately does **not** use the SFTP subsystem. Every operation is a command run on an SSH `exec` channel, so any host with a POSIX shell works, including the ones with no SFTP server at all. The price, parsing command output across GNU, BSD and BusyBox userlands, is paid once in a small set of command profiles.

This file is the binding specification for the plugin; where it disagrees with `SPEC.md`, the question goes into the pull request. It takes the place of `ext-vfs-sftp` in the Version 0.2 plan.

## Decisions

- One plugin crate, `ext-ssh`, a feature plugin like `ext-git`: one `Provider` (id `ssh`), the hosts in the side panel's Network section, and two Actions. Loadable plugin file; the binary ships without it and nothing breaks when it is removed.
- Shell commands over `exec` channels; no SFTP, not even as a fallback.
- Pure Rust: `russh` and `russh-keys`. No libssh2, no C dependency in the plugin file.
- Hosts come from the user's OpenSSH config plus Rick's own list; the OpenSSH entries are read-only in Rick.
- Authentication: agent, then identity files, then password. Secrets live only in the OS keychain, never in a TOML file.
- Host keys follow `known_hosts` with OpenSSH semantics; nothing is trusted silently.
- One connection pool per host, channels multiplexed, a second connection only at the server's session limit.
- Jump hosts (`ProxyJump`) are first-class, including chains.
- Same-host copy and move run on the host (`cp -a`, `mv`) through the `NATIVE_TRANSFER` hook; everything else streams through Rick.
- Previews: text-like previewers read a prefix and offer *Load more*; whole-file previewers are subject to the size limit of remote sources, with *Preview this file*.
- Unix-like hosts only in this version: Linux (glibc, musl, BusyBox), BSD, macOS. Windows SSH hosts are out of scope and are more likely to be served by an SMB provider later.

## Scope

**In:** browsing, stat, read, write, mkdir, rename, delete, symlink targets, same-host copy and move; hosts from `~/.ssh/config` and Rick-defined hosts; agent, key and password authentication with the OS keychain; `known_hosts` handling and the two host-key dialogues; jump hosts and chains; connection pooling; *Open console here* and *Open in external terminal*; prefix and size-capped previews; disk usage for the status bar.

**Out (this version):** Windows SSH hosts (cmd and PowerShell profiles); SFTP; port forwarding; SSHFS-style mounting; a remote search index; `chmod`/`chown` editing (permissions are shown read-only); a remote trash (delete is permanent, behind the confirm dialogue).

## Hosts

Hosts come from two sources, merged into one list and shown in the side panel's Network section.

1. **The user's OpenSSH config**, `~/.ssh/config` and the files its `Include` lines name. Every `Host` entry with a concrete name (no wildcards) appears automatically. Honoured directives: `HostName`, `User`, `Port`, `IdentityFile`, `IdentityAgent`, `ProxyJump`. Other directives are ignored without complaint. These entries are read-only in Rick; editing them means editing the file. The config is re-read when it changes on disk.
2. **Rick's own hosts**, stored in `rick.toml` as `[[ssh.hosts]]` (schema below). These are created and edited from the Network section.

The Network section lists each host as its own row (the `ssh` provider's root entries, the way the Cloud section lists a cloud provider's accounts), not one row for the provider; `vfs-rick`'s `NETWORK_PROVIDERS` names `ssh`. Rows follow the side-panel rules of `SPEC.md`: a Phosphor icon, click opens the host's default directory in the current tab, middle-click, Ctrl-click and *Open in new tab* open a new tab, no current-location highlight. The row's context menu offers *Connect*, *Disconnect*, *Open console*, *Open in external terminal*, and for Rick-defined hosts *Edit…* and *Remove*. The section's *Add…* opens the host editor.

**Host editor** fields: alias, host name, port (default 22), user, default directory (default: the remote home), identity file (optional), jump host (optional, picked from the host list), *Remember password in the OS keychain*, preview size cap override (optional), *Always preview* (off by default; ignores the cap on this host).

Locations are `Location { provider: "ssh", path: "/<alias>/<remote path>" }`; the provider's root lists the hosts as folders.

## Authentication

Order, stopping at the first success:

1. SSH agent: `SSH_AUTH_SOCK` on Linux and macOS, the OpenSSH agent named pipe on Windows.
2. Identity files: the host's `IdentityFile` entries, then the default `~/.ssh/id_*` set. An encrypted key prompts for its passphrase.
3. Keyboard-interactive, then password.

Passwords and passphrases are never written to any file of Rick's. If the user ticks *Remember*, the secret is stored in the OS keychain through a new `platform::Secrets` trait (`store`, `get`, `remove` by a stable key): Secret Service on Linux, Keychain on macOS, Credential Manager on Windows, a fake for tests. The trait is written so the cloud provider plugins reuse it. Removing a host removes its keychain entries.

**Host keys** follow `~/.ssh/known_hosts` with OpenSSH semantics (hashed entries, `@revoked`, port-qualified names). An unknown host shows a dialogue with the key type and SHA256 fingerprint and the choices *Trust and save*, *Trust once*, *Cancel*. A changed host key shows a distinct warning dialogue with *Cancel* as the default; accepting replaces the entry only after an explicit confirmation. Rick never trusts a key silently.

## Transport and command profiles

All operations run as commands on SSH `exec` channels. On the first connection to a host the plugin **probes** it once (`uname -s`, the presence and flavour of `stat`, `find -printf`, `dd`, `head`) and selects a **command profile**:

- `gnu` — GNU coreutils: `find -printf`, `stat --printf`.
- `bsd` — BSD and macOS userland: `stat -f`.
- `busybox` — reduced flag set, `ls -lan` where `find -printf` is missing.

The profile is cached per host and re-probed when a command fails in a way the profile does not expect. Rules for every profile:

- Output is **NUL-separated** wherever the tool allows it, so names with spaces, newlines and non-UTF-8 bytes survive. Where NUL is not available the profile escapes on the remote side.
- Names are byte strings end to end (`OsStr`, as everywhere in Rick); display decodes lossily with replacement characters.
- Every command is built from a fixed template with shell-quoted arguments; user input is never interpolated raw.
- Timestamps are read as epoch seconds, nanoseconds where the profile gives them.
- Rick treats the host as a remote-latency mount in the sense of `SPEC.md`'s Network file systems: no eager stat, no thumbnails by default. A new `Capabilities::REMOTE` flag says so to the kernel and to the preview frame; the cloud providers set it too.

## Provider operations

| Operation | Remote command (`gnu` profile) |
| --- | --- |
| list directory | `find <dir> -mindepth 1 -maxdepth 1 -printf '%y\0%s\0%T@\0%m\0%u\0%g\0%l\0%f\0'`, streamed in batches |
| stat | `stat --printf …` |
| read | `cat <file>`; a reader that stops early pulls only what it read (SSH flow control holds the rest) |
| read from an offset | `dd if=<file> bs=… skip=…` / `tail -c +N`; the provider sets `SEEK` |
| write | `cat > <file>` with stdin streamed, through a temporary name and a final `mv` |
| mkdir, rename, delete | `mkdir -p`, `mv`, `rm -rf` (delete goes through the confirm dialogue; no trash) |
| copy or move within the host | `cp -a` / `mv` as `transfer()` under `NATIVE_TRANSFER`; progress by polling the target's size |
| symlink target | from the listing (`%l`) |
| disk usage | `df -P` for the status bar |

Listings return name, kind, size, mtime, mode, owner, group and link target. Permissions are shown read-only; `PERMISSIONS` is not set. `WATCH` is not set either: a remote folder refreshes on the user's action and after Rick's own operations.

Cross-host copies and copies between a host and the local disk stream through Rick (the generic path in `vfs`); same-host copies and moves run on the host through `transfer()`, as the `NATIVE_TRANSFER` rule in `SPEC.md` describes for the cloud providers.

## Connections

- One **connection pool per host**. The first connection authenticates; operations open channels on it.
- A second connection opens only when the number of live channels reaches the server's session limit (assumed 10, OpenSSH's `MaxSessions` default; a refused channel open triggers the same).
- Idle connections close after a configurable timeout (default 10 minutes). A tab on a closed connection reconnects transparently on its next operation; if authentication now needs a prompt, the prompt appears then.
- Connection errors surface in the tab as a banner with *Retry*, never as a modal dialogue.
- Operations are jobs off the UI thread; listings stream so very large directories paint progressively, within the first-screen budget.

**Jump hosts (`ProxyJump`).** Bastions are first-class. For a host with `ProxyJump` in the OpenSSH config, or a jump host set in the editor, Rick connects to the jump host exactly like any other host (same authentication chain, same `known_hosts` check and dialogues), opens a `direct-tcpip` channel from it to the next hop and runs the next SSH session over that channel. Comma-separated chains repeat this per hop. Jump connections live in the same pool and are shared by every target behind the same bastion; the idle timeout applies per hop, and a bastion stays open while any target behind it is in use. A failure at any hop names the hop in the error banner.

## Console actions

Both actions appear in the context menu of a remote folder and of a host row in the Network section, in the Action group "SSH".

- **Open console here** (the default): opens Rick's embedded console pane attached to a PTY `shell` channel on the existing connection, `cd`'d into the browsed folder. No second login. The pane is the one `ext-action-terminal` introduces (Version 0.2, actions as plugins); it must accept an externally supplied PTY stream so this plugin can attach. If `ext-ssh` lands before that pane exists, it ships with the external action only and gains this one with the pane.
- **Open in external terminal**: launches the user's terminal with `ssh <alias>` (or `ssh -p <port> <user>@<host>`), which starts a fresh SSH session outside Rick. The launcher is a small `platform::Terminal` hook: `$TERMINAL`, then `x-terminal-emulator` and the common emulators on Linux; Terminal.app on macOS; Windows Terminal, then the console host, on Windows. `ext-action-terminal` reuses the same hook for the local *Open in terminal here*.

## Previews over SSH

Previewers declare whether they need the **whole file** or can work on a **prefix**.

- Prefix previewers (text, code, Markdown, CSV): the provider serves the first chunk (default 256 KiB) and the preview pane shows a *Load more* control at the end.
- Whole-file previewers (image, PDF, media): subject to a **size cap**, default 4 MiB, applied by the preview frame when the provider is `REMOTE`. Above the cap the pane shows the File info card with two buttons: *Preview this file*, for this file once, and *Always preview on <host>*, which sets the host's `preview_always` and switches the cap off for that host from then on. The cap is a global setting with a per-host size override for slow links; `preview_always` is the per-host switch that ignores both, reversible in the host editor and in the host row's context menu (see `SPEC.md`, Preview, *Remote sources*). Prefix previewers are unaffected by either.
- A file pulled for a preview goes to a temporary file that is removed when the tab closes.

## Configuration

```toml
[ssh]
read_openssh_config = true
idle_timeout_minutes = 10
preview_size_cap_mib = 4

[[ssh.hosts]]
alias = "nas"
host = "192.168.1.20"
port = 22
user = "serge"
default_dir = "/volume1"
identity_file = "~/.ssh/id_ed25519"
remember_password = false
preview_size_cap_mib = 1
preview_always = false

[[ssh.hosts]]
alias = "prod-db"
host = "10.0.3.12"
user = "ops"
jump = "bastion"   # alias of another host, Rick-defined or from ~/.ssh/config
```

## Dependencies

`russh` and `russh-keys` for the protocol; the `keyring` crate behind `platform::Secrets`; a small in-crate parser for the OpenSSH config. No C dependencies in the plugin file.

## Acceptance

Against Docker containers in CI: an OpenSSH server on Debian (`gnu`), on Alpine (`busybox`), and a two-container network where the target is reachable only through a bastion. The `bsd` profile runs against the macOS runner's own `sshd` where the runner allows enabling it, otherwise its selection and parsing are tested against recorded probe output.

1. Hosts from `~/.ssh/config` appear in Network without any setup; Rick-defined hosts can be added, edited and removed.
2. Agent, identity-file and password authentication each work; a remembered password survives a restart through the keychain and is absent from `rick.toml`.
3. Unknown and changed host keys produce their two dialogues; nothing is trusted silently.
4. Browsing, read, read from an offset, write, mkdir, rename, delete and same-host copy and move work against the `gnu` and `busybox` hosts with names containing spaces, newlines and invalid UTF-8.
5. A second tab on the same host does not prompt again; exceeding ten concurrent channels opens a second connection.
6. A host behind a bastion, and one behind a two-hop chain, browse and transfer exactly like a direct host; each hop gets its own host-key check; two targets behind one bastion share its connection.
7. A same-host move of a large tree produces no traffic through Rick (asserted by counting bytes through `open_read`); a copy from the host to the local disk streams with progress and cancels cleanly.
8. Both console actions open in the browsed folder, including on a jump-host target; *Open console here* is present only once the console pane exists.
9. A 1 GiB remote text file previews at once with *Load more*; a 50 MiB image shows the cap card and loads on *Preview this file*; *Always preview on <host>* loads it, writes `preview_always = true` for the host, and the next large image on that host previews without the message, while the same image on another host still shows it.
10. Removing `ext-ssh` from the plugins folder leaves Rick fully functional with an empty Network section.
