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
| `hikasa session run <ref> [output flags]` | runs a Folder Compare or Text Compare session here |

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

Standard input (`-`) and the clipboard are never stored in a session: that side stays empty, with the warning
`spec_not_saved`.

## Running a session

`hikasa session run <ref>` compares exactly as `hikasa folder compare` or `hikasa text compare` would with the
session's inputs and rules: the same output, the same JSON and the same [exit codes](topic:exit-codes). It
takes their output flags (`--report`, `--display`, `--output`, `--html`, `--title`, `--ignore-unimportant`, …).
Other session types cannot run here yet (exit 100, `unsupported`).

`hikasa gui session <ref> [--wait]` opens it in the window, and so does `hikasa gui open <ref>` when no file of
that name exists (see [hikasa gui](topic:gui)). See also [hikasa workspace](topic:workspace) and
[Contents](topic:index).
