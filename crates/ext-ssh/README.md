# ext-ssh

`ext-ssh` is the SSH plugin, one manifest with several contributions
(`specs/ext-ssh.md`): the `ssh` provider, whose root lists the hosts of
`~/.ssh/config` (with its `Include`s, read again when the file changes) and
Rick's own, kept as `[[connections]]` tables of `rick.toml` with
`provider = "ssh"` and edited from the Network section's host editor, each
host a row of that section that opens its default directory; the "SSH"
action group, *Open console here* (a PTY `shell` channel on the existing
connection in the console pane, through the kernel's terminal emulator) and
*Open in external terminal* (`ssh <alias>` through `platform::Terminal`);
and three settings in `ssh.toml`. Every operation is a command on an `exec`
channel through `russh`, never SFTP: the host is probed once and a command
profile chosen (`gnu`, `bsd`, `busybox`), listings stream NUL-separated
records so names with spaces, newlines and bytes that are no UTF-8 survive,
reads are `cat` (`tail -c +N` from an offset, `SEEK`), writes go through a
temporary name and `mv`, and a same-host copy or move is `cp -a` or `mv`
under `NATIVE_TRANSFER` with the target's size as progress. Connections
pool per host with channels multiplexed and a second connection at the
session limit, close after the idle timeout and come back on the next
operation; jump hosts connect like any host and the next hop runs over a
`direct-tcpip` channel, chains hop by hop, a bastion shared by every target
behind it. Authentication tries the agent, the identity files (a passphrase
is asked for) and then keyboard-interactive and password; secrets live in
the platform keychain through `platform::Secrets` and every question, the
two host-key dialogues over `known_hosts` included, goes through the
`Prompts` queue the host lends, so nothing is trusted silently. The host is
`REMOTE`: no thumbnails, no watch, previews prefix-limited or size-capped.
It is a plugin (`plugins/ssh/`) that depends on `ext` and `types` only;
`rick` (behind `ssh`) and `tests` link it, and the plugin host loads it.
