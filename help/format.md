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

User formats live in the `formats` folder of the settings folder and shadow the built-in ones with the same
id. See [hikasa text compare](topic:text-compare) and [Contents](topic:index).
