# The Table Compare view

The Table Compare view shows two tables as grids: CSV and other delimited or fixed-width text, and the sheets of
xlsx, xlsm, xlsb, xls and ods workbooks. A CSV compares against a workbook sheet. Rows are paired by key columns or
aligned like lines of text; the cells of a paired row are compared by their typed values (numbers, dates, booleans,
text) under the session's rules.

## Reading the grid

- The left grid is beside the right one (View ▸ **Side-by-Side Layout**) or above it (**Over-Under Layout**).
- Row numbers are each side's own (the sheet row or the text line); column heads show each side's header name, else
  its letters, and `#1`, `#2` mark the key columns.
- Red cells are important differences, blue cells unimportant ones (within a tolerance, a case or whitespace
  exception, an unimportant column); a row on one side only is coloured on its side and hatched on the other. A small
  corner mark shows a cell holding a formula.
- The sheet tabs at the bottom carry a spot: grey same, blue unimportant differences, red important differences,
  orange a sheet on one side only. Click a tab to show that sheet.
- The thumbnail on the left marks where the differences are; View ▸ **Text Details** shows every cell of the current
  row under the grid.
- The status bar shows the current cell's address, its state and the sheet; Session ▸ **Table Compare Info** (Ctrl+I)
  shows the statistics per sheet ([Table Compare Info](topic:dialog-table-info)).

## Navigating

- Ctrl+N and Ctrl+P go to the next and the previous difference row; Shift+Ctrl+N and Shift+Ctrl+P to the next and
  previous differing cell; Search ▸ **Next Difference Sheet** and **Previous Difference Sheet** to the sheets that
  differ.
- Ctrl+G opens [Go To](topic:dialog-table-goto): a row number, a column and a sheet.
- The arrow keys move the current cell; Page Up and Page Down move by a screen.
- F5 reloads the files and compares them again.

## Display filters and columns

- View ▸ Show All, Show Differences, Show Same and Show None choose which rows are listed; the toolbar toggles combine
  same rows, left and right orphans and changed rows. **Ignore Unimportant Differences** treats unimportant
  differences as same.
- View ▸ **Hide Same Columns** leaves out the columns without a difference (key columns stay); **Resize Columns to
  Fit** widens the columns to their texts. Drag a column head's right edge to resize it; right-click a head to hide
  the column or show the hidden ones again. The session keeps the hidden columns, the widths and the sheet shown.
- Ctrl+C copies the current cell; after a click on a row number it copies the row, its cells separated by tabs.

## Rules and reports

- Session ▸ **Rules…** opens the Session Settings on the [table rule tabs](topic:dialog-session-table): sheets,
  columns (keys, tolerances, unimportant columns), rows and parsing. OK compares again.
- Session ▸ **Swap Sides** exchanges the files; **Compare Parent Folders** opens a [Folder Compare
  view](topic:view-folder-compare) of their folders.
- Session ▸ **Table Compare Report…** writes a report of the comparison ([the report dialog](topic:dialog-table-report)).

The same comparison is available as [hikasa table compare](topic:table-compare); `--gui` opens it in this view. The
formats that read tables are described in [Table formats](topic:table-formats).
