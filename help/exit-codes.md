# Exit codes

Every subcommand ends with one of these codes:

| Code | Meaning |
|---|---|
| 0 | success (commands without a verdict) |
| 1 | binary same |
| 2 | rules-based same |
| 11 | binary differences |
| 12 | similar: only unimportant differences |
| 13 | rules-based differences |
| 100 | error (also a command line that does not parse, and a crash) |
| 102 | the command line cannot wait for the window |
| 103 | the window executable cannot be found or started |
| 107 | a base folder cannot be loaded |

## Rules

- `quick` and `text compare`: 1, 2, 11, 12 or 13; any error, including an input that cannot be read, is 100.
- `folder compare`: 13 when any important difference exists; else 100 when the result is incomplete or an item
  could not be compared; else 12 when only unimportant differences remain; else 2. A base folder that cannot
  be loaded is 107.
- Display filters never change the exit code; `--ignore-unimportant` does (12 becomes 2).
- A broken pipe on stdout stops the output and keeps the computed code.
- Ctrl-C during a comparison or a `vfs` operation cancels it and exits 100; a second Ctrl-C exits 100 at once
  ([Using the command line](topic:cli)).
- `gui open --wait` exits with the code of the view when it closes ([hikasa gui](topic:gui)).
- `report text`, `report folder` and the report flags: the comparison's code, as the compare subcommand; 100 for a
  refused word, combination or target (before anything is compared), a write that fails (the target keeps what it
  had), a stylesheet that cannot be read, a report too large for the clipboard, no application for the printer or the
  browser, a Ctrl-C. A file report of a pair that fails is a warning. See [hikasa report](topic:report).

- `folder op`: 0 when every item is done or skipped; 100 for a failed item, a stop, a row that failed in the comparison, or a
  command that plans nothing (a bad word, nothing selected, unanswered questions, a read-only side); 107 for a base that
  cannot be loaded. See [hikasa folder op](topic:folder-op).
- `folder sync`, `session run` of a folder-sync session: as `folder op`, and 100 for a row the comparison could not tell (also
  in a dry run), no preset, or an empty source; `--gui` exits 100 while the GUI cannot show a Folder Sync view. See
  [hikasa folder sync](topic:folder-sync).

See [hikasa quick](topic:quick), [hikasa folder compare](topic:folder-compare) and [Contents](topic:index).
