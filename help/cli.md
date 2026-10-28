# Using the command line

`hikasa <subcommand> [flags] <arguments>` runs one tool and exits with a documented
[exit code](topic:exit-codes). It never needs the window; `hikasa gui …` talks to it (see [hikasa gui](topic:gui)).

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
  file format.
- `--output <file>` writes a report through a temporary file and a rename: on any error the file is untouched.
- Human output has one record per line where possible; control characters in names are escaped.
- Every failed command prints one `hikasa: error: …` line on stderr.

## See also

[Help topics](topic:help) · [Exit codes](topic:exit-codes) · [Contents](topic:index)
