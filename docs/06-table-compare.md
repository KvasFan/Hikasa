# Table Compare

Compares tabular data (CSV/TSV/fixed-width text, spreadsheets, HTML tables)
cell-by-cell in two grids that scroll together, side-by-side or over-under.

## 1. Input and parsing (Table file formats)

- **TB-001** (P0): A table format (`10-file-formats.md`) defines: mask, conversion
  (plain text, external program, built-in spreadsheet extraction, built-in HTML-table
  extraction), encoding, Ctrl+Z EOF marker, **Type** (Detect / Delimited / Fixed), and
  **Regional** settings (use system, or explicit decimal separator, thousands
  separator, date order and separator).
- **TB-002** (P0): Delimited: delimiter set (comma, semicolon, tab, space, other),
  text qualifier (`"`), *treat consecutive delimiters as one*, *treat surrounding
  whitespace as part of delimiter*; quoted fields may contain delimiters and doubled
  qualifiers. Fixed: list of column widths (`10;20;8`). Detect: sniff delimiter and
  qualifier from the first lines.
- **TB-003** (P0): *First line contains*: Detect / Column names / Cell data; per-pane
  override from the File Info header.
- **TB-004** (P1): Spreadsheet conversion loads **all sheets** (xlsx, xlsm, xls, ods)
  through `calamine`; each sheet becomes a comparison **sheet** (tab at the bottom);
  HTML conversion extracts each `<table>` as a sheet. Colour spots on sheet tabs show
  the overall state of each sheet (red important / blue unimportant).
- **TB-005** (P0): Cell values are typed: General (auto-detect per column: number,
  date/time, text) or Text (forced), using the regional settings for parsing.

## 2. Session rules

### 2.1 Sheets tab

- **TB-010** (P1): Sheet alignment: Unaligned (by position), Align by left name, Align
  by right name, Custom (Move Up/Down to pair sheets manually; Tidy removes blank rows).
  Comparing two sheets of one file: open the file on both sides and pair sheets manually.

### 2.2 Columns tab (per sheet)

- **TB-011** (P0): Each comparison column shows left column letter + sample, right
  column letter + sample, and handling summary. Column alignment: Unaligned, Align by
  left name, Align by right name, Custom (Move Up/Down, Tidy). Context menu in the grid
  header: Align Columns by Number / by Left Name / by Right Name, Align Left/Right
  Column With…, Hide Column, Unhide Column ▸ (list / All), Resize Column…, Key Column,
  Unimportant Column, Standard Column, Column Handling…
- **TB-012** (P0): **Column Handling** dialog (per column, or "Edit Default"): **Key**
  (participates in row alignment and sort; priority = list order), **Use default**,
  **Type** General/Text, **Unimportant** (differences in this column are unimportant),
  *Text important except for*: Character case, Whitespace; **Date tolerance** (N
  seconds/minutes/hours/days), **Numeric tolerance** (absolute or relative %).

### 2.3 Rows tab (per sheet)

- **TB-013** (P0): If key columns exist, rows align by key (sorted by key, like a
  database join; duplicate keys align in order). Otherwise: Unaligned / Standard /
  Myers / Patience alignment with Skew tolerance, closeness matching and *Never align
  differences* (same engines as text, operating on row content); *Sort rows before
  alignment*; *Use default*; "Edit Default…".

## 3. Display and editing

- **TB-020** (P0): Grid with row numbers (toggle), column headers, cell colouring:
  red important / blue unimportant / replacement / orphan rows; gutter colour spot
  per row; thumbnail; **Hide Same Columns**; **Resize Columns to Fit**; visible
  whitespace; display font commands; Text Details panel (current row's cells, "Auto
  Resize Columns to Fit", "Lock Visible Whitespace").
- **TB-021** (P0): Display filters: Show All / Differences / Same / None; toggles
  Same / Left Orphans / Right Orphans / Differences; Ignore Unimportant Differences.
- **TB-022** (P0): Navigation: Next/Previous Difference Cell (Shift+Ctrl+N/P),
  Next/Previous Difference Row (Ctrl+N/P), Next/Previous Difference Sheet, Next/Previous
  Edit, Next/Previous Existing Row (Alt+Down/Up), Find/Replace (Ctrl+F/H; cells),
  Go To row:column / sheet (Ctrl+G), Next/Previous Difference Files (child).
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
- **TB-025** (P1): Report layouts: side-by-side, summary, interleaved
  (`13-reports.md`).
