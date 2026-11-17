# hikasa session

A session is everything a comparison needs: its type, the inputs (specs), the rules, a description and the
layout of its view. Named sessions live in folders below the settings folder (`sessions/<folder>/<name>.toml`);
each type has its own defaults (`sessions/_defaults/<type>.toml`), which every new session starts from; the
window keeps the last views as auto-saved sessions, grouped by day.

A session is named by its folders and name joined by `/`: `work/api-v2`. A name has 1–100 characters, none of
`/ \ : * ? " < > |`, does not start with `_` or end with a space or a dot, and is unique in its folder whatever
its case.

| Command | Result |
|---|---|
| `hikasa session list [--folder <folder>] [--autosaved] [--defaults]` | folders and sessions; with the flags also auto-saved sessions and saved defaults |
| `hikasa session show <ref>` / `show --defaults <type>` | the session file |
| `hikasa session new <ref> --type <type> [--left <spec>] [--right <spec>] [--description <text>]` | a new session from the type's defaults |
| `hikasa session edit <ref> (--set <key>=<value>… \| --unset <key>… \| --left \| --right \| --description) [--force]` | changes a session |
| `hikasa session edit --defaults <type> (--set <key>=<value>… \| --unset <key>…)` | changes the defaults of a type |
| `hikasa session rename <ref> <new ref> [--force]` | renames, or moves to another folder |
| `hikasa session lock <ref>` / `unlock <ref>` | a locked session is used but never saved, deleted or renamed without `--force` |
| `hikasa session delete <ref> [--force]` | deletes it |
| `hikasa session mkdir <folder>` / `rmdir <folder> [--recursive] [--force]` | session folders; a folder holding locked sessions needs `--force` |
| `hikasa session run <ref> [output flags]` | runs a Folder Compare, Folder Sync, Text Compare, Table Compare, Hex Compare or Picture Compare session here |

Types: `folder-compare`, `folder-sync`, `text-compare`, `text-edit`, `table-compare`, `hex-compare`,
`picture-compare`.

## Keys

`--set` keys are paths inside the session file, written like settings keys (see
[hikasa settings](topic:settings)): `rules.criteria.size=false`, `rules.alignment.skew_tolerance=4`,
`specs.format=c-cpp`, `view.split=0.25`. The rules are those of `--rules-file` of
[hikasa folder compare](topic:folder-compare) and [hikasa text compare](topic:text-compare). A file holds the
rules that differ from the type's defaults, so a later change of the defaults reaches every session that did not
change that rule itself. A rule you set stays in the file even when it equals the defaults, so it keeps its
value when the defaults change; `--unset <key>` removes it, and the session follows the defaults again
(`hikasa session edit work/api --unset rules.handling.flatten`). The size and date filters and the time offset
of a folder session may be written `false` to switch them off: `rules.filters.other.size=false`.

`edit`, `delete` and `rename` refuse a locked session unless `--force`. A settings import does not replace a
locked session, and restoring defaults does not delete one, unless `--force` (see
[hikasa settings](topic:settings)).

## Hex Compare and Picture Compare sessions

A `hex-compare` session holds the words of [hikasa hex compare](topic:hex-compare), a `picture-compare` session those
of [hikasa picture compare](topic:picture-compare); `session edit --set` checks each value with the same parsers (a
bad word is refused, exit 100), and keys a version does not know are kept. Both take `[specs] left`, `right`,
`format`, `left_format` and `right_format`.

```toml
# type = "hex-compare"
[rules.hex]
alignment = "fast"          # none | fast | complete
block = 8192                # bytes, a power of two, 1 KiB to 1 MiB
skew = 16777216             # bytes
window = 262144             # bytes, 4 KiB to 16 MiB
budget_mib = 1024
unlocked_max_mib = 16       # 0 to 1024
[view]
bytes_per_line = 16         # 1 to 256
group = 1                   # 1 | 2 | 4 | 8
chars = "ascii"             # ascii | latin1 | none
address = "hex"             # hex | decimal
layout = "side-by-side"     # side-by-side | over-under
context = 0                 # 0 to 64
[display]
filter = "differences"      # all | differences | same
```

```toml
# type = "picture-compare"
[rules.picture]
mode = "tolerance"          # tolerance | range | blend
tolerance = 0               # 0 to 255
blend = 50                  # 0 to 100 (read in blend mode)
auto_scale = false
tone_map = "clamp"          # clamp | reinhard
max_megapixels = 256        # 1 to 4096
ignore_unimportant = false
[view]
orientation_left = "none"   # none, cw, ccw, 180, flip-h, flip-v, transpose, transverse (a comma list)
orientation_right = "none"
offset = [0, 0]             # the right picture's origin in left coordinates
checkerboard = true
layout = "side-by-side"     # side-by-side | stacked
zoom = "fit"
```

`sessions/_defaults/hex-compare.toml` and `sessions/_defaults/picture-compare.toml` hold the defaults of new sessions;
the Picture Compare defaults also set the tolerance, Auto Scale, tone map and size limit with which `quick --rules` and
the folder content test compare pictures ([hikasa quick](topic:quick)). Swap Sides exchanges the specs and every
`_left`/`_right` key and negates `offset`. Only `[specs]` and `[rules]` make a session changed; the view's layout and
geometry are view state.

Standard input (`-`) and the clipboard are never stored in a session: that side stays empty, with the warning
`spec_not_saved`.

## Running a session

`hikasa session run <ref>` compares exactly as `hikasa folder compare`, `text compare`, `table compare`, `hex
compare` or `picture compare` would with the session's inputs and rules: the same output, the same JSON and the same
[exit codes](topic:exit-codes). It takes their report flags (`--report`, `--options`, `--output-options`,
`--display`, `--output-to`, `--html`, `--title`, …; see [hikasa report](topic:report)) and `--ignore-unimportant`; a
Hex Compare session also `--max-sections`, `--preview`, `--display` and `--context`; a Picture Compare session
`--diff-output`, `--diff-pane`, `--checkerboard` and `--probe` (each refused for the other types, exit 100); `--dry-run` is the report's for a
comparison and the sync's for a Folder Sync session. `hikasa report text|folder --session <ref>` reports a session too.
A Folder Sync session runs its sync (`[rules.sync]`) as [hikasa folder sync](topic:folder-sync) does, with
`--dry-run`, `--yes`, `--confirm` and `--allow-empty-source`; without a terminal and without `--yes` it writes nothing
and exits 100. Text Edit sessions cannot run here (exit 100, `unsupported`).

`specs.read_only_left` and `specs.read_only_right` (*Disable editing*) protect a side: no folder operation or sync of
the session writes it, and the window turns off the commands that would. The window keeps them when it saves the
session, and a side becomes writable again only by changing the key (`session edit --set
specs.read_only_right=false`).

`hikasa gui session <ref> [--wait]` opens it in the window, and so does `hikasa gui open <ref>` when no file of
that name exists (see [hikasa gui](topic:gui)). See also [hikasa workspace](topic:workspace) and
[Contents](topic:index).
