# Using the command line

`hikasa <subcommand> [flags] <arguments>` runs one tool and exits with a documented
[exit code](topic:exit-codes). It never needs the window; `hikasa gui …` talks to it (see [hikasa gui](topic:gui)),
and `hikasa gui script` runs a [script](topic:script) in it.

## Global flags

These flags are accepted before or after the subcommand:

| Flag | Meaning |
|---|---|
| `--json` | one JSON document on stdout, also on error ([JSON output](topic:json-output)) |
| `--quiet`, `-q` | no progress, no warnings, no other non-error stderr output |
| `--log <file>` | append diagnostic logs to the file |
| `--verbose` | debug-level logs |
| `--config-dir <dir>` | use this settings folder (portable behaviour) |
| `--no-color` | no ANSI colours (also when `NO_COLOR` is set or stdout is not a terminal) |
| `--help`, `-h` / `--version`, `-V` | usage / the version (exit 0) |

A flag that takes a value may be repeated only with the same value.

## Inputs and outputs

- `-` names standard input. It is read once; `--stdin-name <name>` gives it a file name, which picks the
  file format (also a table format: `hikasa table compare - b.csv --stdin-name a.csv`).
- `clipboard://` names the clipboard: its text, or an image as a PNG named `clipboard.png` (which
  [hikasa picture compare](topic:picture-compare) compares like any picture; `--diff-output clipboard://` places an
  image there). It is read once per
  command, only when a `clipboard://` spec is given. No clipboard (no display, a headless machine) is an
  `unsupported` error, an empty clipboard `not_found`; both exit 100. `HIKASA_CLIPBOARD_FILE=<file>` makes the
  file's text stand in for the clipboard (scripts, headless machines). `HIKASA_CLIPBOARD_HELPER_FILE=<file>` makes
  the helper that keeps a report on the Linux clipboard write it to that file instead (scripts, headless machines,
  tests).
- `--output <file>` writes a report through a temporary file and a rename: on any error the file is untouched.
  `--output-to` also takes `clipboard`, `printer` and `browser` ([hikasa report](topic:report)).
- `SOURCE_DATE_EPOCH=<seconds>` (a decimal count of seconds since 1970) is the time a report states, in UTC, so a
  report is the same bytes on every run; a value that is not such a count is ignored with the warning
  `source_date_epoch_invalid`. Only reports read it.
- Human output has one record per line where possible; control characters in names are escaped.
- Every failed command prints one `hikasa: error: …` line on stderr.

## Ctrl-C

The first Ctrl-C cancels a comparison or a `vfs` operation: an archive being rewritten stays as it was and its temporary
and lock files go, the file of `--output` keeps what it had, and the command exits 100. A second Ctrl-C exits 100 at once,
as a kill would; the next rewrite of that archive removes what is left. In `text convert` a file being written stays as
it was and the files not reached are not written; `text find` stops its search, and with `--replace` a file being
written stays as it was. Any other command ends at the first Ctrl-C. A
command started with Ctrl-C ignored stays that way. In `script run` the first Ctrl-C stops the current command as that command stops, the
rest of the script is not run, and the script exits 100 (`Script stopped at line <n>`).

## Comparing files

`hikasa quick` gives a verdict as an exit code only ([hikasa quick](topic:quick)); `hikasa text compare`, `hikasa table
compare`, [hikasa hex compare](topic:hex-compare) and [hikasa picture compare](topic:picture-compare) show the
differences of text, tables, bytes and pictures. `hikasa hex dump` and `hikasa hex find` read and search the bytes of
any file ([hikasa hex dump](topic:hex-dump), [hikasa hex find](topic:hex-find)).

## Scripts

`hikasa script run <file> [<arg>…]` runs a batch of commands over one folder comparison, headless; `hikasa script check`
checks one without running it ([hikasa script](topic:script)). Its log goes to stderr; a report with `output-to:-` to
stdout.

## See also

[Help topics](topic:help) · [Exit codes](topic:exit-codes) · [Contents](topic:index)
