# hikasa settings

`hikasa settings` reads and changes the settings files of the settings folder (`--config-dir`, or the
per-user folder). Every value has a key `<file>.<path>`:

| File | Holds |
|---|---|
| `options` | the program options (`options.toml`) |
| `state` | recent items, the last window, first-run flags (`state.toml`) |
| `commands` | shortcuts, menus and toolbars per view (`commands.toml`) |
| `theme` | the colour scheme of light and dark mode, the fonts (`theme.toml`) |
| `themes.<scheme>` | one colour scheme (`themes/<scheme>.toml`; `light` and `dark` are built in) |
| `filters` | name filter presets (`filters.toml`) |
| `openwith` | Open With applications (`openwith.toml`) |
| `archives` | archive type masks and [limits](topic:archives) (`archives.toml`) |

The path is the TOML key path: `options.text_editing.auto_indent`, `openwith.app.0.command`; a segment that is
not a plain name is written in double quotes, an array position as a number.

| Command | Result |
|---|---|
| `hikasa settings get <key>` | the value (a table as TOML) |
| `hikasa settings set <key> <value>` | changes the value; `(file unchanged)` when nothing was written |
| `hikasa settings unset <key>` | puts the value back to its default; a key Hikasa keeps without knowing it (a view or command of a newer version) leaves the file |
| `hikasa settings list [<key>] [--changed]` | every key with its value; `# default: …` after changed ones |
| `hikasa settings restore-defaults (--all \| --file <file>…) (--dry-run \| --yes)` | deletes settings files |
| `hikasa settings restore-defaults --category <cat>… [--delete-sessions [--force]] [--delete-workspaces] (--dry-run \| --yes)` | restores factory defaults by category |
| `hikasa settings export -o <file.hikasa-pkg> [--category <cat>…] [--item <cat>:<name>…]` | writes a settings package |
| `hikasa settings import <file> [--category <cat>…] [--item <cat>:<name>…] [--delete-existing <cat>…] [--force] (--dry-run \| --yes \| --silent)` | imports one |

A value is TOML (`true`, `5`, `"text"`, `["Ctrl+Q", "F2"]`); a text value may be given without quotes
(`hikasa settings set options.appearance.theme_mode dark`). A value equal to its default is removed from the
file, so a later change of the default reaches you. Keys Hikasa does not know are kept in the file; a wrong
value is refused with a message.

## Repairing a file

Every other command stops when a settings file holds a value of the wrong type or out of range. The `settings`
commands report it as the warning `invalid_config_value` and use the default instead, so
`hikasa settings set <key> <value>` repairs the file. An Open With application or a filter preset that is
incomplete (an empty command, an empty mask) is kept as it is, at its position, with the warning.

## Settings packages

A settings package (`.hikasa-pkg`) is a zip file with `manifest.toml` and the settings files at their places
in the settings folder. Categories: `options` (`options.toml`, `filters.toml`, `openwith.toml`,
`archives.toml`), `theme` (`theme.toml`, `themes/*.toml`), `commands`, `state`, `sessions` (named sessions
and the defaults of each type, never auto-saved ones), `formats` (file formats and their order), `workspaces`,
and `system_info` (a `system-info.txt` for support: version, platform, settings folder and counts; exported only
on request, never imported). Export takes every category but `system_info` unless `--category` or `--item`
narrow it; items are named as in the manifest (`sessions:work/api-v2`, `formats:my-c`).

Import first checks the whole package (paths, sizes, every file against its format, the rules of every
session, nothing newer than this version) and writes nothing when anything is wrong. `--dry-run` prints the
plan: `delete <path>`, then `add <path>` or `replace <path>` (a file that exists, whatever the case of its name,
is replaced and keeps its spelling), then one summary line per category.
`--delete-existing sessions|formats|workspaces` deletes the existing items of that category first. `--silent`
imports every category and item without deleting anything.

A locked session is neither replaced nor deleted: the plan says `skip (locked) <path>` (`skip_locked` in JSON)
and the warning `session_locked` names it. Add `--force` to replace or delete locked sessions too.

`restore-defaults --category` deletes the files of those categories so that the built-in defaults apply again:
the settings files, the type defaults of sessions, the format files and `order.toml`. Named sessions and
workspaces are deleted only with `--delete-sessions` and `--delete-workspaces`; a locked session is kept, with
the warning `session_locked`, unless `--force` is added too. See
[hikasa session](topic:session) and [hikasa workspace](topic:workspace).

## Changes from elsewhere

`set` refuses to overwrite a file that changed since it was read (for example by the window); run it again, or
add `--force`. `restore-defaults` deletes files only with `--yes`; `--dry-run` prints the plan. With `--all` or
`--file`, sessions, workspaces and file formats are not touched (see [hikasa format](topic:format)).

See [Using the command line](topic:cli), [JSON output](topic:json-output) and [Contents](topic:index).
