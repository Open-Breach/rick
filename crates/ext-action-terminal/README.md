# ext-action-terminal

`ext-action-terminal` holds *Open console* (Mod+`): a terminal at the
pane's location. By default it is an embedded console, a dock tab next to
the pane's active one: the user's shell runs in a pseudo-terminal
(`portable-pty`, so one code path serves Linux, macOS and Windows), its
output goes through the kernel's terminal emulator (`ext::terminal`: `vte`
parsing into a screen of cells with colours, attributes, a scroll region, an
alternate screen, tab stops and a scrollback of 10 000 lines, shared with
`ext-ssh`'s console over SSH) that the UI draws, and
what is typed in the tab goes to the shell; closing the tab ends the
shell. The setting `console` in `action-terminal.toml` (a choice in the
Settings window: `embedded` or `system`) sends the action to the user's
terminal emulator through `platform::Terminal` instead. *Open in system
file manager* stays a core action. It is a plugin
(`plugins/action-terminal/`) that depends on `ext` and `types` only and
takes the platform services from the registry; `rick` (behind
`action-terminal`) and `tests` link it, and the plugin host loads it.
