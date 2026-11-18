# Table formats

A table format (kind `table`, [hikasa format](topic:format)) tells [hikasa table compare](topic:table-compare) how
to read a file. Its `[table]` settings, and the flags of `table compare` that replace them for both sides (each flag
also as `--left-<flag>` and `--right-<flag>` for one side, on top of the both-sides flag):

| Setting | Flag | Values |
|---|---|---|
| `kind` | `--type` | `detect` (the content decides: delimiter, fixed widths, or one column), `delimited`, `fixed`, `spreadsheet` |
| `delimiter`, `delimiters` | `--delimiter <d>` (repeatable) | a character; several written together (`,;`); `tab`, `space`, `comma`, `semicolon`, `colon`, `pipe`, `u+XXXX` |
| `sniff_delimiter` | (`--delimiter` turns it off) | the delimiters are a preference: a file they do not split while `,`, `;`, a tab or `\|` does is read with that one (the *Comma Separated Values* preset sets it) |
| `qualifier` | `--qualifier <c>\|none` | the text qualifier (`"`); a doubled qualifier is one; delimiters and line breaks inside are data |
| `consecutive_as_one` | `--consecutive` | a run of delimiters separates once; leading and trailing runs separate nothing |
| `trim_whitespace` | `--trim` | spaces and tabs around fields and qualifiers are part of the delimiter |
| `widths` | `--widths <w;w;…>` | fixed column widths in characters; text past the last width is one more column |
| `first_line` | `--first-line` | `detect` (a vote of the columns), `names` (a header), `data` |
| `regional` | `--regional system\|default`, `--decimal <c>`, `--thousands <c>\|none`, `--date-order ymd\|dmy\|mdy`, `--date-sep <c>\|any` | how numbers and dates are written; `system` takes the locale's (below) |
| `ctrl_z_eof` | `--ctrl-z` | a Ctrl+Z (0x1A) ends the data; without it a Ctrl+Z at the very end is dropped |
| encoding | `--encoding <label>` | the text encoding; by default a BOM, valid UTF-8, else detected |

```toml
[table]
delimiter = ";"
qualifier = '"'
first_line = "names"

[table.regional]
decimal = ","
thousands = "."
date_order = "dmy"
date_sep = "."
```

- Numbers: an optional sign, digit groups of three when a thousands separator is set, one decimal separator, an
  exponent, a trailing `%`. Dates: ISO 8601 always (`2024-01-31`, `2024-01-31T12:30:00`), and the regional order
  with two-digit years (1930–2029), times with seconds, milliseconds and AM/PM.
- `--regional system`: numbers take the locale of `LC_ALL`, `LC_NUMERIC`, `LANG`, dates the one of `LC_ALL`,
  `LC_TIME`, `LANG` (the first set), through a table of languages and territories (`de_DE`: `,` decimal, `.`
  thousands, day.month.year). When none is set, Windows gives the user's regional settings (`HKCU\Control
  Panel\International`: the locale name, then the decimal and thousands separators and the order and separator of
  the short date) and macOS its `AppleLocale` (a region override `@rg=` gives the formats of that region); other
  systems, and a query that fails, give the defaults (`.`, `,`, year-month-day).
- The parser flags of a delimited table (`--qualifier`, `--consecutive`, `--trim`) need a delimited side: a side
  whose type is detected takes them with `--type delimited` or `--delimiter`. `--type fixed` needs `--widths` unless
  the format gives widths. Workbook sides ignore the parser flags.
- *Spreadsheet Workbooks* reads every sheet of xlsx, xlsm, xlsb, xls and ods files (hidden sheets are read and
  flagged; chart and macro sheets are skipped with the warning `sheet_skipped`). In text views, such as
  [hikasa text compare](topic:text-compare), a workbook reads as tab-separated text, a `--- <sheet> ---` line between
  sheets (the *Spreadsheet text* conversion; read-only). A Text format can convert with `[conversion] method =
  "spreadsheet"` too.

See [hikasa table compare](topic:table-compare).
