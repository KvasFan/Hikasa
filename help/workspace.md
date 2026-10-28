# hikasa workspace

A workspace is the set of open windows and tabs of the window: each window's size and place and its active
tab, and for each tab a named session or the session of an unsaved view, with its layout. The window saves
workspaces (Save Workspace As, and On exit, save workspace as); they live in `workspaces/<name>.toml` of the
settings folder.

| Command | Result |
|---|---|
| `hikasa workspace list` | name, windows, tabs and shortcut of each workspace |
| `hikasa workspace show <name>` | the workspace file |
| `hikasa workspace rename <name> <new name>` | renames it |
| `hikasa workspace delete <name>` | deletes it |
| `hikasa gui workspace <name>` | loads it in the window |

Loading a workspace closes the open views and opens its tabs. A tab whose named session no longer exists opens
the Home view instead, with the warning `session_missing` (`workspace show` reports it too).
`hikasa gui open <name>` loads a workspace when no file and no session has that name.

See [hikasa session](topic:session), [hikasa gui](topic:gui) and [Contents](topic:index).
