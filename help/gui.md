# hikasa gui

`hikasa gui` talks to the running window over a private local connection. Only `gui open` and `--gui`
start the window when it is not running.

| Command | Result |
|---|---|
| `hikasa gui open <specs…>` | opens a view (starts the window when needed) |
| `hikasa gui ping` | the window's name and protocol version |
| `hikasa gui list` | the open views: id, kind, title, specs |
| `hikasa gui close <id>` | closes a view |
| `hikasa gui quit` | closes every view and quits |
| `hikasa gui session <ref> [--wait]` | opens a named [session](topic:session) |
| `hikasa gui workspace <name>` | loads a [workspace](topic:workspace) (closes the open views) |

## Opening and waiting

- `--wait` blocks until the view closes and exits with its code: 1, 2, 11, 12 or 13 for file views,
  2, 12, 13 or 100 for folder views, 0 for a view without a verdict.
- `--no-launch` never starts the window; with `--wait` and no window the code is 102.
- 103 means no window executable was found next to `hikasa` or on `PATH`.
- `gui open` accepts the switches of `hikasa-gui` itself: `--title1` … `--title4`, `--ro`, `--filters`,
  `--expandall`, `--format`, `--iu`, `--stdin-name`; `hikasa gui open --help` lists them.

## One argument

`hikasa gui open <arg>` (and `hikasa-gui <arg>`) with one argument that is not an existing file or folder
opens the named session of that name (exactly, then in any case), else the workspace of that name. An existing
file ending in `.hikasa-pkg` is a settings package: with `--silent` everything in it is imported at once (as
`hikasa settings import --silent`); without it the import wizard of the window is needed, which comes in a later
version (exit 100). A window older than this command line answers that it does not support sessions: restart
it.

`hikasa text compare … --gui` and `hikasa folder compare … --gui` send the comparison to the window. See
[Exit codes](topic:exit-codes) and [The main window](topic:main-window).
