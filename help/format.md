# hikasa format

File formats tell Hikasa how to read a file: its mask, encoding, grammar and which parts are unimportant.

| Command | Result |
|---|---|
| `hikasa format list` | every format in detection order: position, id, status, name, mask |
| `hikasa format show <id|name>` | one format as TOML (`--json`: the same as JSON) |
| `hikasa format detect <file>` | the format a file name selects, and why |
| `hikasa format tokens <file>` | the grammar tokens of each line |

`detect` and `tokens` accept `--stdin-name <name>` for `-`; `tokens` also accepts `--format`, `--encoding`
and `--lines <first>-<last>`.

## Changing formats

| Command | Result |
|---|---|
| `hikasa format new <id> [--from <id>] [--name <text>] [--mask <mask>] [--kind <kind>]` | a new user format, empty or a copy |
| `hikasa format edit <id> (--from-file <path> \| --set <key=value>…)` | changes a format; a built-in gets an override file |
| `hikasa format delete <id>` | deletes a user format |
| `hikasa format import <path> [--replace]` | copies a format file into the formats folder |
| `hikasa format export <id> [-o <path>]` | the format as TOML on stdout or in a file |
| `hikasa format enable <id>`, `hikasa format disable <id>` | turns detection by the format on or off |
| `hikasa format move <id> (--up \| --down \| --top \| --bottom \| --before <id> \| --after <id>)` | changes the detection order |
| `hikasa format restore (<id>… \| --all) (--dry-run \| --yes)` | removes files that override built-in formats |

User formats live in the `formats` folder of the settings folder, one file per format (`<id>.toml`), with the
order and the enabled flags in `order.toml`. A command writes only the files it changes; the built-in formats
are never copied there. A built-in cannot be deleted: disable it, and `format restore` removes the file that
overrides it (`--all` removes every user format and the order). `--set` takes a TOML key path of the format
(`mask=*.x;*.y`, `text.tab_size=8`); a change that does not compile is refused and nothing is written. The id
`order` is reserved (`order.toml` holds the order).

See [hikasa text compare](topic:text-compare), [hikasa settings](topic:settings) and [Contents](topic:index).
