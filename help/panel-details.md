# The details panel and the ruler

View ▸ **Text Details**, **Hex Details** or **Alignment Details** shows a panel below the panes for the current row
(a second click hides it; drag its top edge to resize it).

- **Text Details**: the row's lines, the left one above the right one, in full. A click puts the caret of that pane
  there: typing edits the line in its pane.
- **Hex Details**: the bytes of each line as a save would write them: the offset, 16 bytes a row in hex and as
  characters (`.` for the others), the line ending's bytes after them; a character the encoding cannot write is
  marked (*written as ?*); a long line is cut at 4,096 bytes (*truncated*).
- **Alignment Details** (Text Compare): the row's characters aligned in two lines of cells, coloured as in the panes.
  Click a cell and type, Backspace or Delete: that side's line is edited at that character; the arrows move by cells.
- **Lock Visible Whitespace** shows spaces and tabs in the panel whatever View ▸ Visible Whitespace says.

View ▸ **Ruler** shows the display columns above each pane: a tick per column, a number every ten, the tab stops,
and the caret's column. See [The Text Compare view](topic:view-text-compare).
