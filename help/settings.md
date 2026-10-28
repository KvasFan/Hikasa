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
| `archives` | archive type masks (`archives.toml`) |

The path is the TOML key path: `options.text_editing.auto_indent`, `openwith.app.0.command`; a segment that is
not a plain name is written in double quotes, an array position as a number.

| Command | Result |
|---|---|
| `hikasa settings get <key>` | the value (a table as TOML) |
| `hikasa settings set <key> <value>` | changes the value; `(file unchanged)` when nothing was written |
| `hikasa settings unset <key>` | puts the value back to its default |
| `hikasa settings list [<key>] [--changed]` | every key with its value; `# default: …` after changed ones |
| `hikasa settings restore-defaults (--all \| --file <file>…) (--dry-run \| --yes)` | deletes settings files |

A value is TOML (`true`, `5`, `"text"`, `["Ctrl+Q", "F2"]`); a text value may be given without quotes
(`hikasa settings set options.appearance.theme_mode dark`). A value equal to its default is removed from the
file, so a later change of the default reaches you. Keys Hikasa does not know are kept in the file; a wrong
value is refused with a message.

## Repairing a file

Every other command stops when a settings file holds a value of the wrong type or out of range. The `settings`
commands report it as the warning `invalid_config_value` and use the default instead, so
`hikasa settings set <key> <value>` repairs the file. An Open With application or a filter preset that is
incomplete (an empty command, an empty mask) is kept as it is, at its position, with the warning.

## Changes from elsewhere

`set` refuses to overwrite a file that changed since it was read (for example by the window); run it again, or
add `--force`. `restore-defaults` deletes files only with `--yes`; `--dry-run` prints the plan. Sessions,
workspaces and file formats are not touched (see [hikasa format](topic:format)).

See [Using the command line](topic:cli), [JSON output](topic:json-output) and [Contents](topic:index).
