# The command line

One Rick runs per user session and config directory: a second `rick` hands its command line to the running one over a local socket (a named pipe on Windows) and exits with its status within a second, so a terminal, a launcher or a link always reaches the window you have open, and the window comes forward.

| Command | What it does |
| --- | --- |
| `rick [<path>…]` | Opens each folder in a tab of the most recently focused window; the current directory without any path |
| `rick reveal <file>…` | Opens the files' folders and selects the files |
| `rick --new-window [<path>…]` | Puts the folders in a window of their own |
| `rick --activate` | What the launcher entry runs: brings the window forward and opens no tab |
| `rick --version` | Prints the build's details |
| `rick --list-plugins` | Prints the plugins Rick found and whether each loaded |
| `rick --config-dir <dir> …` | Runs a Rick of its own on another config directory |
| `rick --dbus-service` | What the session bus runs for *“Show in folder”* when no Rick is up; opens no folder of its own |

Plugins add subcommands of their own, which `rick --help` lists beside the core ones: `rick install <archive>` opens the tarball installer's form on an archive (Linux).

## Links

A `rick:///<path>` link from a document or a chat opens an existing folder or reveals an existing file, and says so in a toast otherwise; the desktop entry and `Info.plist` register the scheme.

## The console

The embedded console follows the tab it was opened from and the tab follows the console's `cd` (the shell reports it through OSC 7 where it knows how: bash and zsh with the distribution's vte.sh, fish). The tarball installer's `path.sh` defines a `rick` function for login shells that lack `~/.local/bin` on their PATH.

## Where plugins live

Plugins live in the `plugins/` folder beside the binary and in a per-user one in the config directory (`~/.config/rick/plugins` on Linux, `~/Library/Application Support/Rick/plugins` on macOS, `%APPDATA%\Rick\plugins` on Windows). The Extensions screen in the burger menu lists them and switches each on or off for the next start.

## Installing into the system

On Linux, the first start checks what of the system Rick is not set up in yet and shows *Set up Rick on this system*, one row with a switch per item: the standard location (`~/.local/opt/rick/<version>` with a `current` link), the app launcher entry, the default file manager, *“Show in folder”* from other apps, the `rick` command and the dock favourite. Nothing needs a password, a file Rick did not write is never replaced unless you tick *Replace* on its row, and Settings → Integration shows each item afterwards. By hand it still works: the archive carries the desktop entry and the icons in the XDG layout under `share/`, so copy `share/` into `~/.local/share/` and put `rick` on the PATH; or open the tarball in Rick and choose *Install as application*.

On macOS the archive is `Rick.app` zipped: drag it to Applications. On Windows, unpack and run `rick.exe`, which carries its icon and version.
