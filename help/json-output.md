# JSON output

With `--json`, stdout holds exactly one JSON document, also when the command fails. Progress and logs always
go to stderr.

## The envelope

| Field | Meaning |
|---|---|
| `schema` | `hikasa.<command>.v<major>`, for example `hikasa.folder_compare.v1` |
| `hikasa_version` | the version that wrote the document |
| `ok` | false when the command failed |
| `exit_code` | the process [exit code](topic:exit-codes) |
| `result` | the result (`null` on error) |
| `warnings` | warnings (an array, possibly empty) |
| `error` | `{kind, message, spec, …}` on error, else `null` |

## Rules

- Field order is fixed; enum values are `snake_case` strings; absent values are `null`, never missing.
- Timestamps are RFC 3339 in UTC; CRC-32 values are 8 lowercase hex digits.
- A command line that does not parse gives `hikasa.usage_error.v1` (exit 100).
- `hikasa version --json` lists every schema of the build.
- `--help` stays text.

See [Using the command line](topic:cli) and [hikasa version](topic:version).
