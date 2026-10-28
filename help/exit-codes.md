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
- `gui open --wait` exits with the code of the view when it closes ([hikasa gui](topic:gui)).

See [hikasa quick](topic:quick), [hikasa folder compare](topic:folder-compare) and [Contents](topic:index).
