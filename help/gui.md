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
| `hikasa gui script <file> [<arg>…]` | runs a [script](topic:script) in the window, with its [status window](topic:dialog-script) |

## Opening and waiting

- `--wait` blocks until the view closes and exits with its code: 1, 2, 11, 12 or 13 for file views,
  2, 12, 13 or 100 for folder views, 0 for a view without a verdict.
- The view is chosen by the files' formats: two folders or archives open a Folder Compare view, two tables a Table
  Compare view, two files of a Hex format a [Hex Compare view](topic:view-hex-compare), two pictures a
  [Picture Compare view](topic:view-picture-compare), other files a Text Compare view; `--fv` names the view
  (`--fv="Hex Compare"`, `--fv="Picture Compare"`, `--fv="Table Compare"`, `--fv="Text Edit"`). The window serves
  Folder Compare, Folder Sync, Text Compare, Text Edit, Table Compare, Hex Compare and Picture Compare views; another
  kind exits 100 (`unsupported_view`).
- `--no-launch` never starts the window; with `--wait` and no window the code is 102.
- `gui open <left> <right> --wait` is the diff tool of version control systems: [Using Hikasa with version
  control](topic:version-control).
- 103 means no window executable was found next to `hikasa` or on `PATH`.
- `gui open clipboard:// <file>` (and `--gui` with `clipboard://`) compares the clipboard: the window reads
  its own clipboard when it opens the view, so this works even when `hikasa` runs without a display. No or an
  empty clipboard in the window exits 100. A view keeps what it read.
- `gui open` accepts the switches of `hikasa-gui` itself: `--title1` … `--title4`, `--ro`, `--filters`,
  `--expandall`, `--format`, `--iu`, `--stdin-name`, `--edit`, `--savetarget`, `--nobackups`; `hikasa gui open
  --help` lists them.

## Editing switches

- `--edit` opens each file in a Text Edit view: two files open two views (`--wait` waits for both and exits
  with the higher code: 0, or 100 when a file could not be loaded). With `--fv` or `--sync`, the last of them
  decides the view. Each view is a [Text Edit view](topic:view-text-edit); `--ro`, `--savetarget` and
  `--nobackups` apply to it.
- `--ro` makes every pane read-only, `--ro1` the left pane (the file of a Text Edit view) and `--ro2` the right
  one.
- `--savetarget=<file>` makes the saves of the view write that file instead: the right pane of a Text Compare
  (the left pane still saves in place), the file of a Text Edit view. A relative path is taken from the
  current folder of this command; `-` and `clipboard://` are refused (exit 100).
- `--nobackups` keeps no backup on any save or operation of that view.

## One argument

`hikasa gui open <arg>` (and `hikasa-gui <arg>`) with one argument that is not an existing file or folder
opens the named session of that name (exactly, then in any case), else the workspace of that name. An existing
file ending in `.hikasa-pkg` is a settings package: with `--silent` everything in it is imported at once (as
`hikasa settings import --silent`); without it the window shows the [import wizard](topic:wizard-import) with the
path filled in (exit 0). A window older than this command line answers that it does not support sessions: restart
it.

## Scripts

`hikasa gui script <file> [<arg>…] [--wait] [--no-launch] [--stop-on-error] [--confirm <mode>] [--closescript]
[--dry-run] [--silent]` finds the script as `hikasa script run` does and checks it here: a script that cannot be read exits 105, one
that does not check 106 with its errors, before the window is asked anything. Then the window runs it with this
terminal's current folder and the environment variables the script names; an argument, the folder or a value that is
not valid UTF-8 cannot be sent (exit 100: use `hikasa script run`). Exit 0 once it started; with `--wait` the script's
own exit code (102 when the window goes away first). `--closescript` closes the status window at the end, whatever the
code. `--dry-run` runs it in the window as `hikasa script run --dry-run` does (no plan executed, no file written); a
window too old for that refuses it before anything is sent (exit 100: restart it). `--json` prints
`hikasa.gui_script.v1` ([JSON output](topic:json-output)). `hikasa script run <file> … --gui` is the same command. A
window that already runs a script refuses another (exit 100); an older window answers that it does not support
scripts: restart it.

`@<file>` names a script on the command line of `hikasa gui open` and `hikasa-gui`: `hikasa gui open @s.txt a b
--wait` is `hikasa gui script s.txt a b --wait`; `hikasa-gui @s.txt a b` starts the window with the script, or sends
it to the window that runs (exit 0). The other arguments are the script's `%1`…, never a comparison. A file whose name
starts with `@` is written `./@name`; `@` alone is an error. A switch of a view (`--ro`, `--sync`, `--nobackups`, a
title, …) given with a script warns `switch_ignored`: a script opens no view. With `--silent` the script never goes
through a window: it runs headless where it was started, as `hikasa script run --silent` (Ctrl-C included), and the
command exits with its code; `--json` then prints `hikasa.script.v1`. `--silent` with neither a script nor a settings
package, and `--closescript` without a script, warn `switch_ignored`.

`hikasa text compare … --gui` and `hikasa folder compare … --gui` send the comparison to the window; `gui open … --sync`
and `hikasa folder sync … --gui` open a [Folder Sync view](topic:view-folder-sync) (`gui list` names its kind
`folder_sync`; `--wait` exits with the code of its last Run). See
[Exit codes](topic:exit-codes) and [The main window](topic:main-window).

## Quick Compare at startup

With Options ▸ Startup ▸ **Show the quick compare dialog when starting with a file comparison**, an open of two files
whose view kind is chosen by their formats (`hikasa a b`, `hikasa gui open a b`) shows the [Quick Compare
dialog](topic:dialog-quick-compare-startup) first; `--wait` then gets the quick comparison's exit code when the dialog
is closed.
