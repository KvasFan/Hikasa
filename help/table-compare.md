# hikasa table compare

`hikasa table compare <left> <right>` compares two tables cell by cell: every sheet of two workbooks (xlsx, xlsm,
xlsb, xls, ods), two delimited or fixed-width text files (CSV, TSV, …), or one of each (an export against its
source). Cells compare by their typed values: `1.50` equals `1.5`, `2024-01-31` equals a date cell of that day, and
a workbook number equals its CSV spelling at the 15 digits a workbook shows. The [exit code](topic:exit-codes) is
the verdict: 1 identical files, 2 same under the rules, 12 unimportant differences only, 13 important differences,
100 an error.

## Output

Without a report flag the output is one line per sheet pair, then the differing rows of each sheet (at most 20
unless `--display all`; `--max-rows <n>` sets the limit, 0 shows every row), then a summary line:

```
≠ Prices: 2 rows differ, 1 left only, 1 right only; cells: 1 important, 2 unimportant
  row 3/3: price: 2 → 2.5
  row 4/4: price: 0.75 → 0.76 (unimportant)
  row 5/-: left only: 4, fig, 3, 2024-02-03
= Notes → notes
< Old (left only)
important differences: 2 of 3 sheets differ; rows: 2 same, 2 changed, 3 left only, 1 right only
```

`=` same, `≠` important differences, `~` unimportant differences only, `<` a sheet on the left only, `>` on the
right only. Rows are labelled by their line (text tables) or sheet row (workbooks) on each side. `--display
all|differences|same|none` chooses the rows; columns of `--hide` are left out. With `--ignore-unimportant` a changed
row whose differences are all unimportant reads `row 2/2: same (unimportant differences)` (with `--display all`),
and the summary counts such rows apart (`…, 3 with unimportant differences only`). `--json` prints
`hikasa.table_compare.v1` ([JSON output](topic:json-output)), whose rows carry their differing cells, or every cell
with `--cells all` (also with `--report json`). A flag the chosen output does not use is refused (exit 100):
`--cells` without JSON, `--max-rows` with `--json` or a report, `--width` outside side-by-side and interleaved text,
`--line-numbers`/`--no-line-numbers` with summary or json, `--title` with json, `--display` with summary.

## Formats

Each side is read with the table format its name matches: *Comma Separated Values* (`*.csv`, `*.tsv` with tabs),
*Spreadsheet Workbooks* (`*.xls`, `*.xlsx`, `*.xlsm`, `*.xlsb`, `*.ods`), else the type is detected from the content.
A `.csv` that the comma does not split while `;`, a tab or `|` does is read with that delimiter (a German export
with `;` compares with its workbook as it is); `--delimiter` reads it as asked, and a file whose every row is then one
field warns `delimiter_mismatch` with the delimiter to try. Workbooks are recognised by their content whatever their
name. `--format <id|name>` gives both sides a format, `--left-format`/`--right-format` one side; the format must be
of the table kind ([hikasa format](topic:format)). The parser flags replace the formats' settings on both sides;
`--left-<flag>` and `--right-<flag>` (`--left-decimal ,`, `--right-date-order mdy`, …, every parser flag) on one
side, on top of the both-sides flag: a German CSV against an English one. See [Table formats](topic:table-formats).

## Sheets

- `--sheets by-name|left-name|right-name|position`: by name (the default) pairs equal names, then names that differ
  in case, then the rest by position; `left-name`/`right-name` pair by name only, in that side's order; `position`
  by position. A pair whose names differ is renamed, not a difference; a CSV pairs with the first sheet.
- `--sheet <L>=<R>` (repeatable) compares exactly these pairs: a sheet name or a 1-based number on each side,
  `<L>=` or `=<R>` for a sheet of one side only.

## Columns and rules

- `--columns auto|number|left-name|right-name`: auto pairs columns by their header names when both sides have them,
  else by number. `--align-column <L>=<R>` (repeatable) pairs two columns, the others follow by number.
- Columns are named by their header (`price`, `name:2024` for a header that looks like a number), their number
  (`3`, `#3`) or their letters (`C`); lists are comma-separated, `\,` is a comma in a name.
- `--key <cols>`: rows join by these columns (in priority order), as a database join; duplicate keys pair in
  their order and are counted. Without keys rows align like lines of text: `--algorithm
  unaligned|standard|myers|patience|histogram`, `--skew <n>`, `--closeness`, `--no-closeness`,
  `--closeness-threshold <‰>`, `--never-align`, `--sort-rows` (both sides sorted first).
- `--unimportant <cols>`: differences there are unimportant. `--text <cols>` compares display texts;
  `--number <cols>` compares as numbers, leading zeros included (`007` equals `7`; by default a column with leading
  zeros is text, an identifier).
- `--ignore-case [<cols>]`, `--ignore-whitespace [<cols>]` (every column without a value): such differences are
  unimportant.
- `--tolerance [<cols>=]<n>` or `<n>%` (relative), `--date-tolerance [<cols>=]<n>[ms|s|m|h|d]` (repeatable): values
  within the tolerance are unimportant differences, never same.
- `--exact-numbers [<cols>]`: workbook numbers compare by their stored binary value, not at 15 digits.
- `--formulas`: differing formula texts are important differences too (the cached values are always compared).
- `--ignore-hidden-rows`, `--ignore-hidden-columns`: rows and columns hidden in an xlsx or xlsm workbook are left
  out. `--hide <cols>` hides columns in the output only.
- The rules apply to every sheet pair. A column a sheet does not have is left out of that sheet with the warning
  `rule_not_applied`; keys that do not all resolve leave the sheet unkeyed with the warning `keys_not_applied`; a
  column that no sheet has is an error (exit 100). Rules for one sheet come from a Table Compare session
  ([hikasa session](topic:session), `[[rules.sheet]]`): the columns written there must exist in that sheet, the rules
  it takes over from every sheet's are left out where the sheet lacks their columns.
- `--ignore-unimportant` (`--iu`) treats unimportant differences as same: 12 becomes 2.

## Reports

`--report summary|side-by-side|interleaved|xml|json` (`--layout`) writes a report: summary, the sheets side by
side, or each changed row's left row above its right one, as text or HTML (`--html`, `--html-mono`); `--display`,
`--line-numbers`/`--no-line-numbers`, `--width`, `--title`, `--options`, `--output-options`; `--output-to
<file>|clipboard|printer|browser` (`--output <file>`), `--dry-run`. See [hikasa report](topic:report). A script
writes the same reports with `table-report` (`data-report`) ([hikasa script](topic:script)).

## Errors

A bad word, an unknown format, a column that names nothing, a sheet pair that names no sheet or `--type fixed`
without `--widths` is exit 100 (`invalid_input`); words are checked before anything is read. A password-protected
workbook is `unsupported` ("the workbook is password-protected; Hikasa does not decrypt workbooks"), a damaged one
`invalid_data`; the same holds when [text compare](topic:text-compare) reads a workbook as text. Two files with
identical bytes are identical in a folder comparison even when they cannot be read.

## In the window

`--gui` opens the comparison in the window's [Table Compare view](topic:view-table-compare) with the same words
(`--wait` waits until it closes and exits with its code, `--no-launch` does not start the window); the report flags
and `--cells` do not apply with it (exit 100). The window opens two table files in this view by itself, from the
command line, a drop or a folder row's Open. `hikasa quick <a> <b>` and the rules-based content test of [hikasa folder
compare](topic:folder-compare) compare table files with the same engine.
