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

## Hex and picture formats

A format of the *hex* kind (`--kind hex`) names files [hikasa hex compare](topic:hex-compare) and
[hikasa hex dump](topic:hex-dump) read through a load conversion: an *External* program (`[conversion]`) that turns the
file into the bytes to compare, run on at most 512 MiB of input, its output kept in the temporary folder while the
comparison lasts. Without a matching hex format the raw bytes are compared. No hex format is built in.

Formats of the *picture* kind say which files are pictures: the built-in *Picture Files*
(`*.png;*.apng;*.jpg;*.jpeg;*.jpe;*.jfif;*.gif;*.bmp;*.ico;*.tif;*.tiff;*.webp;*.tga;*.pnm;*.ppm;*.pgm;*.pbm;*.pam;
*.qoi;*.exr;*.hdr;*.dds`) makes [hikasa picture compare](topic:picture-compare), `quick --rules` and the rules-based
content test of folders compare such files by their pixels. The decoder is chosen by the file's content, not its name; DDS is in the mask but not decoded, so a DDS
pair is compared byte by byte in folders and `quick`. A picture format with an *External* load conversion is decoded
from the program's output (a format of a container the decoders do not read, converted to PNG, for example).
`--format`, `--left-format` and `--right-format` of both commands take a format of their own kind only.

In the window, Tools ▸ File Formats… does the same ([File Formats](topic:dialog-file-formats)), and the [format
editor](topic:dialog-format-editor) writes what `format edit` writes.

See [hikasa text compare](topic:text-compare), [hikasa settings](topic:settings) and [Contents](topic:index).
