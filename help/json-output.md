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
  itself goes to its target. See [hikasa report](topic:report).
- `hikasa script run|check` prints `hikasa.script.v1`: the mode, the script, its arguments, where it stopped, the check's
  errors (`line`, `column`, `code`, `message`), and per command its line, words, expanded text, status (`checked`, `ok`,
  `failed`, `refused`, `not_run`) and the `result` of the matching command's document (`hikasa.folder_op.v1`, …), item
  lists capped at 100,000 (`items_omitted`). A check that fails is `ok: false` with every error in `result`; a run is
  `ok: true` with its exit code. See [hikasa script](topic:script).
- `hikasa gui script` (and `script run --gui`, `gui open @script`) prints `hikasa.gui_script.v1`: the script as sent,
  its arguments, `waited`, and with `--wait` the script's `exit_code` and `summary` (`commands`, `ok`, `failed`,
  `refused`, `not_run`, `stopped_at`); without it both are `null`. With `--silent` (`gui script --silent`, `gui open
  @script --silent`) the script runs here, headless, and the document is `hikasa.script.v1`, as for `hikasa script run
  --silent`. The build lists 43 schemas.
- The warnings of scripts and operations: `nothing_selected` (a `select` selected nothing), `attrib_unsupported`
  (`attrib` letters the back-end does not have: those items are skipped), `log_not_written` (a script's log file not
  written: a dry run, or a write that failed).
- The warnings of reports: `report_option_unused` (a word the layout does not use), `report_folder_renamed` (the files
  folder was not this report's), `report_old_folder_kept` (the earlier files folder could not be removed),
  `file_report_failed` (a pair report failed; the folder report stands), `source_date_epoch_invalid`
  (`SOURCE_DATE_EPOCH` ignored).
- A folder operation prints `hikasa.folder_op.v1` (with `--dry-run` too: `status` says `planned`), a sync
  `hikasa.folder_sync.v1`, a content test or CRC `hikasa.folder_contents.v1`. A failed item is no failed command: the
  document is `ok: true` with exit code 100, and the item carries its `error`; see [hikasa folder op](topic:folder-op)
  and [hikasa folder sync](topic:folder-sync).

- `hikasa text convert` prints `hikasa.text_convert.v1`: the target (`in_place`, `output`, `stdout`), per file its
  format, encoding, BOM and line endings before and after, the conversions with the lines each changed, its status
  (`planned`, `done`, `skipped` with `no_change` or `declined`, `failed`, `not_run`), error, questions and warnings; the
  save plan (`write` items with `backup`, `expect`, `via_link`, `encoding`, `bom`, `bytes`, `spliced_bytes`,
  `encoded_lines`); the counts. As for folder operations, a run that wrote with failed files is `ok: true` with exit
  code 100; see [hikasa text convert](topic:text-convert).
- `hikasa text find` prints `hikasa.text_find.v1`: the query (`pattern`, `regex`, `match_case`, `whole_word`), the
  roots, `mask`, `recursive`, `include_hidden`, `archives`; each file with a hit (`spec`, `encoding`, `hits` with `line`,
  `column` and `len` in characters, the `preview` of the line, `preview_range` of the hit in it, `truncated`;
  `hit_count`; `stopped` when the file or the search stopped at a limit); the files and folders not searched (`spec`,
  `reason`: `binary`, `too_large`, `unreadable`, `converted`, `filtered`; `error`); `counts` (`files_searched`,
  `files_matched`, `hits`, `skipped`), `limit_reached` (warning `find_limit_reached`), and `replace`: `null`, or with
  `--replace` the text, `dry_run`, the save `plan` as above, one `results` entry per file with a hit (`replaced`,
  `status`, `skip`, `error`) and the `summary` (`replaced`, `done`, `skipped`, `failed`, `not_run`). Exit code 0 with
  or without hits; a replace with a failed file is `ok: true` with exit code 100; see
  [hikasa text find](topic:text-find).
- `hikasa table compare` prints `hikasa.table_compare.v1`: each side (`spec`, `size`, `mtime`, `source`: `delimited`,
  `fixed`, `xlsx`, `xlsm`, `xlsb`, `xls`, `ods`; `sheets`, `encoding`, the Hikasa `format`), the effective table
  `format` of each side (`kind`, `delimiters`, `qualifier`, `widths`, `first_line`, `regional`, `encoding`,
  `detected`), the `options`, `ignore_unimportant`, the `verdict`, the `stats` (sheet counts and `totals`) and per
  sheet pair its `name`, sides, `renamed`, `state`, `alignment` (`keyed` with its keys, `engine` with its algorithm,
  `orphan`), `columns` with their rules and detected kinds, `stats`, `duplicates`, the `header` cells, the `display`
  and the `rows` the display shows (`state`, `importance`, the row labels, the `cells` with `value` (the display
  text), `type` and `formula`); `--display` and `--cells differing|all` choose rows and cells. The JSON Schema is
  `docs/schemas/table_compare.v1.json`. Its warnings: `duplicate_keys`, `unterminated_qualifier`,
  `stray_after_qualifier`, `sheet_skipped`, `formula_without_value`, `rule_not_applied`, `keys_not_applied` (a
  sheet compared without keys), `delimiter_mismatch` (every row one field while another delimiter fits), `lossy_decode`,
  `alignment_degraded`, and with `--verbose` `merged_cells`. `left.format` and `right.format` name the side's format
  of the table kind, else `null`.
- The warnings of saves and conversions: `source_touched` (the file has a new time but the content it was read
  from), `save_reencoded` (every line is encoded: a new encoding, no source to copy unchanged lines from, a changed
  file), `lossy_line_rewritten` (converted lines held bytes that could not be decoded), `unmappable_replaced`
  (characters the encoding cannot write were written as `?`).

See [Using the command line](topic:cli) and [hikasa version](topic:version).
