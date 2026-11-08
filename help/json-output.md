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
- `hikasa report` prints `hikasa.report.v1` with `--json`: the report's type, layout, format, target and file, the
  bytes written, the comparison's verdict, the canonical words and the unused ones, and for links the files folder
  (`new` or `replace_ours`, `renamed`) and each pair's status (`planned`, `written`, `skipped`, `failed`); the report
  itself goes to its target. The build lists 39 schemas. See [hikasa report](topic:report).
- The warnings of reports: `report_option_unused` (a word the layout does not use), `report_folder_renamed` (the files
  folder was not this report's), `report_old_folder_kept` (the earlier files folder could not be removed),
  `file_report_failed` (a pair report failed; the folder report stands), `source_date_epoch_invalid`
  (`SOURCE_DATE_EPOCH` ignored).
- A folder operation prints `hikasa.folder_op.v1` (with `--dry-run` too: `status` says `planned`), a sync
  `hikasa.folder_sync.v1`, a content test or CRC `hikasa.folder_contents.v1`. A failed item is no failed command: the
  document is `ok: true` with exit code 100, and the item carries its `error`; see [hikasa folder op](topic:folder-op)
  and [hikasa folder sync](topic:folder-sync).

See [Using the command line](topic:cli) and [hikasa version](topic:version).
