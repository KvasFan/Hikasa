# Table Compare

Compares tabular data (CSV/TSV/fixed-width text, spreadsheets, HTML tables)
cell-by-cell in two grids that scroll together, side-by-side or over-under.

*M2 status (Table Compare contract `design/m2-table-contract.md`):* the engine (phase 1) and its integration
(phase 2) are done: `hikasa table compare` with every rule of §2 as a flag, the JSON document
`hikasa.table_compare.v1`, the summary, side-by-side and interleaved reports (text and HTML) with the XML and JSON
dumps, `table-compare` sessions and `session run`, the script command `table-report`, `quick --rules` and the
rules-based content test of folder comparisons. The grid of §3 is phase 3 (Wave 13); until then `--gui` is refused.

## 1. Input and parsing (Table file formats)

- **TB-001** (P0): A table format (`10-file-formats.md`) defines: mask, conversion
  (plain text, external program, built-in spreadsheet extraction, built-in HTML-table
  extraction), encoding, Ctrl+Z EOF marker, **Type** (Detect / Delimited / Fixed), and
  **Regional** settings (use system, or explicit decimal separator, thousands
  separator, date order and separator).
  *M2 notes (§2, §6.1, T-19, T-22, T-28):* the format's `[table]` settings are `kind` (`detect`, `delimited`,
  `fixed`, `spreadsheet`), `delimiter`/`delimiters`, `qualifier`, `consecutive_as_one`, `trim_whitespace`,
  `widths`, `first_line`, `regional` (`decimal`, `thousands`, `date_order`, `date_sep`, `use_system`: the locale's,
  through the A16 table of §6.4) and `ctrl_z_eof`; `table compare` replaces each with a flag (`--type`,
  `--delimiter`, …, `--regional system`). Built-in spreadsheet extraction is the conversion `spreadsheet`: Text
  Compare reads a workbook as tab-separated text, one block per sheet; HTML-table extraction stays M3.
- **TB-002** (P0): Delimited: delimiter set (comma, semicolon, tab, space, other),
  text qualifier (`"`), *treat consecutive delimiters as one*, *treat surrounding
  whitespace as part of delimiter*; quoted fields may contain delimiters and doubled
  qualifiers. Fixed: list of column widths (`10;20;8`). Detect: sniff delimiter and
  qualifier from the first lines.
  *M2 notes (§2.2, §2.3, T-18):* the in-house parser; sniffing over the first 64 records (`,` `;` tab `|` `:` space,
  the most consistent wins; aligned blank columns make a fixed-width table; else one column).
- **TB-003** (P0): *First line contains*: Detect / Column names / Cell data; per-pane
  override from the File Info header.
  *M2 notes (§2.5):* Detect is a vote of the columns of the first record over up to 50 body records; `--first-line`
  sets it for both sides (the per-pane override is the grid's, phase 3).
- **TB-004** (P1): Spreadsheet conversion loads **all sheets** (xlsx, xlsm, xls, ods)
  through `calamine`; each sheet becomes a comparison **sheet** (tab at the bottom);
  HTML conversion extracts each `<table>` as a sheet. Colour spots on sheet tabs show
  the overall state of each sheet (red important / blue unimportant).
  *M2 notes (T-1, §3):* **moved to M2 (P0 for xlsx)**: all sheets of xlsx, xlsm, xlsb, xls and ods through
  `calamine` 0.38, recognised by content; hidden sheets loaded and flagged, chart and macro sheets skipped
  (`sheet_skipped`); a password-protected workbook is refused (`unsupported`). The *Spreadsheet Workbooks* preset
  (`*.xls;*.xlsx;*.xlsm;*.xlsb;*.ods`) is of type `spreadsheet`. HTML tables stay M3.
- **TB-005** (P0): Cell values are typed: General (auto-detect per column: number,
  date/time, text) or Text (forced), using the regional settings for parsing.
  *M2 notes (T-5, T-6, T-23…T-25):* typed cells (number, text, bool, date-time, duration, error); a column with
  leading zeros or more than 15 digits is text (an identifier), `--number` makes it numeric; workbook numbers
  compare at 15 significant digits unless `--exact-numbers`.

## 2. Session rules

### 2.1 Sheets tab

- **TB-010** (P1): Sheet alignment: Unaligned (by position), Align by left name, Align
  by right name, Custom (Move Up/Down to pair sheets manually; Tidy removes blank rows).
  Comparing two sheets of one file: open the file on both sides and pair sheets manually.
  *M2 notes (T-10, §6.5):* `--sheets by-name|left-name|right-name|position` (by name: equal names, then
  case-insensitive, then by position) and `--sheet <L>=<R>` for custom pairs; a renamed pair is informational.

### 2.2 Columns tab (per sheet)

- **TB-011** (P0): Each comparison column shows left column letter + sample, right
  column letter + sample, and handling summary. Column alignment: Unaligned, Align by
  left name, Align by right name, Custom (Move Up/Down, Tidy). Context menu in the grid
  header: Align Columns by Number / by Left Name / by Right Name, Align Left/Right
  Column With…, Hide Column, Unhide Column ▸ (list / All), Resize Column…, Key Column,
  Unimportant Column, Standard Column, Column Handling…
  *M2 notes (T-11, T-12):* `--columns auto|number|left-name|right-name`, `--align-column <L>=<R>`; columns are
  named by header, number (`3`, `#3`) or letters (`C`).
- **TB-012** (P0): **Column Handling** dialog (per column, or "Edit Default"): **Key**
  (participates in row alignment and sort; priority = list order), **Use default**,
  **Type** General/Text, **Unimportant** (differences in this column are unimportant),
  *Text important except for*: Character case, Whitespace; **Date tolerance** (N
  seconds/minutes/hours/days), **Numeric tolerance** (absolute or relative %).
  *M2 notes (T-7, T-30):* `--key`, `--unimportant`, `--text`, `--number`, `--ignore-case`, `--ignore-whitespace`,
  `--tolerance [<cols>=]<n>[%]`, `--date-tolerance [<cols>=]<n>[ms|s|m|h|d]`, `--exact-numbers`, `--hide`,
  `--formulas`; a value within a tolerance is an unimportant difference. The flags apply to every sheet: a column a
  sheet lacks is left out of it (`rule_not_applied`), one no sheet has is an error. Per-sheet rules are a session's
  `[[rules.sheet]]`.

### 2.3 Rows tab (per sheet)

- **TB-013** (P0): If key columns exist, rows align by key (sorted by key, like a
  database join; duplicate keys align in order). Otherwise: Unaligned / Standard /
  Myers / Patience alignment with Skew tolerance, closeness matching and *Never align
  differences* (same engines as text, operating on row content); *Sort rows before
  alignment*; *Use default*; "Edit Default…".
  *M2 notes (T-8, §5.4):* `--algorithm`, `--skew`, `--closeness`, `--no-closeness`, `--closeness-threshold`,
  `--never-align`, `--sort-rows`; duplicate keys are counted (`duplicate_keys`).

## 3. Display and editing

- **TB-020** (P0): Grid with row numbers (toggle), column headers, cell colouring:
  red important / blue unimportant / replacement / orphan rows; gutter colour spot
  per row; thumbnail; **Hide Same Columns**; **Resize Columns to Fit**; visible
  whitespace; display font commands; Text Details panel (current row's cells, "Auto
  Resize Columns to Fit", "Lock Visible Whitespace").
  *M2 notes (phase 3, T-43…T-50):* the window's Table Compare view: two grids side by side or over-under, each
  side's row labels (toggle `row_numbers`), column heads with the key mark, cells coloured from the core's cell
  states, hatched sides of orphan rows, a formula mark, the thumbnail, sheet tabs with the TB-004 spot, Hide Same
  Columns (`SheetComparison::column_differs`; key columns stay), Resize Columns to Fit (the rows around the first
  visible one), resizable and hideable columns kept in the session's `[view]`, the font commands, Text Details.
  Visible whitespace and the Auto/Lock toggles of the details panel are not built (P1 polish).
- **TB-021** (P0): Display filters: Show All / Differences / Same / None; toggles
  Same / Left Orphans / Right Orphans / Differences; Ignore Unimportant Differences.
  *M2 notes:* `--display all|differences|same|none` and `--ignore-unimportant` for the output, JSON and reports;
  the toggles are the grid's (phase 3: built, through `RowFilter`).
- **TB-022** (P0): Navigation: Next/Previous Difference Cell (Shift+Ctrl+N/P),
  Next/Previous Difference Row (Ctrl+N/P), Next/Previous Difference Sheet, Next/Previous
  Edit, Next/Previous Existing Row (Alt+Down/Up), Find/Replace (Ctrl+F/H; cells),
  Go To row:column / sheet (Ctrl+G), Next/Previous Difference Files (child).
  *M2 notes (phase 3):* difference rows, cells and sheets (`nav::*`, only rows the filter shows) and Go To are built;
  Edit and Existing Row navigation, Find/Replace in cells and the child commands are not (they wait for editing,
  TB-023, and the child links of table views).
- **TB-023** (P1): Editing: Modify cell (F2, in-place edit), Cut/Copy/Paste/Delete,
  Delete Row(s), Delete Column(s), Insert Row Before/After (Shift+Ctrl+Enter /
  Ctrl+Enter), Insert Column Before/After, **Sort…** (selected data, by column(s),
  asc/desc, typed), Copy to Right/Left (rows; Ctrl+R/L), Copy Cells to Right/Left
  (Shift+Ctrl+R/L), Align With (F7: pick row then counterpart; "Align with Marked" /
  "Cancel"), Undo/Redo. Saving writes delimited text with the format's delimiter and
  qualifier ("Data lost during editing because of format" warning when a value cannot be
  represented). Spreadsheet write-back is P2.
- **TB-024** (P0): Status bar: current cell address, cell state, sheet name;
  **Table Compare Info**: difference rows per sheet.
  *M2 notes (phase 3):* built; Info lists the sheets and every sheet's statistics (`SheetStats`), with Copy. Copy
  (Ctrl+C) of a cell, or of a row after a click on its number, is TB-023's only part in M2.
- **TB-025** (P1): Report layouts: side-by-side, summary, interleaved
  (`13-reports.md`).
  *M2 notes (§6.3):* done in phase 2, as text and HTML, plus the XML and JSON dumps; `--report` on `table
  compare`, `table-report` (`data-report`) in scripts.
