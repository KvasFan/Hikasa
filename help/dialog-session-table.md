# Table rule tabs

The Session Settings dialog of a Table Compare session has four rule tabs after Specs. They keep the words of
`hikasa table compare` ([hikasa table compare](topic:table-compare)): what is typed here is checked as the command
line checks it, and a message under the tab keeps OK off until it is fixed. OK compares the view again. The scope at
the bottom applies the change to this view only or to the defaults of every new Table Compare session.

## Sheets

- **Pair the sheets**: by name (then the remaining sheets by position), by the left or the right sheets' names, or by
  position.
- **Custom pairs**: `left=right`, `left=` or `=right`, a sheet name or a 1-based number; given, only these pairs are
  compared. **Move Up**, **Move Down** order them, **Tidy** drops empty and repeated pairs.
- **Compare formulas**: a differing formula is an important difference even when the values are the same.

## Columns

- **Pair the columns**: automatic (by name when both sides have header names), by number, by the left or right names,
  or custom pairs (`left=right`, one a line).
- **Column rules**: the first line is every column's default (*Edit Default*): character case and whitespace
  exceptions, exact workbook numbers, a numeric tolerance (`0.01`, `0.5%`) and a date tolerance (`30s`, `5m`, `2h`,
  `1d`). Each further line names columns (a header name, letters such as `B`, a number, or `a,b`) and marks them
  unimportant, compared as text or as numbers, with exceptions, exact numbers, hidden, or with their own tolerances.
  **Add Rule** adds a line for the columns typed beside it.

## Rows

- **Key columns** pair the rows by their values, in priority order (`id, date`); without keys the rows are aligned
  like lines of text.
- **Alignment**: standard, Myers, Patience, Histogram or unaligned (row by row); **Skew tolerance**; **Closeness
  matching** and its threshold in ‰; **Never align differences**; **Sort rows before alignment**.
- **Leave hidden rows out**, **Leave hidden columns out**: hidden rows and columns of a workbook are not compared.

## Parsing

The parser words of the sides: the type (`detect`, `delimited`, `fixed`, `spreadsheet`), the delimiters, the
qualifier, consecutive delimiters, trimming, fixed widths, the first line (`detect`, `names`, `data`), the regional
settings (decimal and thousands separators, date order and separator, or `system`), the encoding and Ctrl+Z. Empty
fields keep the formats' own settings ([Table formats](topic:table-formats)).
