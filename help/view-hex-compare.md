# The Hex Compare view

The Hex Compare view shows two files byte by byte, side by side or one above the other, aligned so that inserted and
deleted runs of bytes show as gaps on the other side. Files of any size open at once: the first screen shows before
the comparison ends, and only the bytes on screen are read.

## Opening

- Two files of a Hex format (see [File Formats](topic:dialog-file-formats)) open in Hex Compare from Home, from a
  folder view's **Open**, from `hikasa gui open` and from `hikasa hex compare --gui` (see
  [hikasa hex compare](topic:hex-compare)). Any two files open here with Home's **Hex Compare** type, Session ▸ **New
  Session** ▸ **Hex Compare**, or a folder view's **Open With** ▸ **Hex Compare**.
- The rows show at once with each byte against the byte at the same offset; the alignment then runs (the status bar
  shows *Aligning… n %*) and the aligned rows replace them in one step, the caret keeping its byte. Esc stops the
  alignment (*Alignment stopped*); the rows stay as they were.
- View ▸ **Alignment** chooses the mode: **None** (byte n with byte n), **Fast** (the default: blocks, small gaps byte
  by byte) or **Complete** (every gap byte by byte within the budget). The mode is a rule of the session; the other
  alignment rules are on the [Alignment tab](topic:dialog-session-settings).

## Reading the panes

- Each row shows the address of its first byte of that side (View ▸ [Layout…](topic:dialog-hex-layout) hides the
  addresses or shows them in decimal), the bytes as two hex digits, and the characters (ASCII, Latin-1 or none).
- Differing bytes are in the important difference colour; bytes of one side only are on the orphan background and the
  other side shows a hatched gap. A row with a difference has the difference background.
- Rows whose pages are still being read show as grey bands for a moment.
- The thumbnail at the right edge marks where the differences are (red) and where one side has bytes the other lacks;
  the frame is the part on screen. Click or drag in it to go there. View ▸ **Thumbnail** hides it.
- The header of each pane shows the file (type another path and press Enter to open it on that side; **…** browses),
  its format (Format ▸ picks *Auto* or a Hex format), its size and time, how it is read (*in memory*, *kept open*,
  *from the temp cache*) and *Converted* when a Hex format converted it. Files dropped on a pane replace that side.

## Caret and selection

- Click a byte or a character; Shift+click or drag selects; a double-click selects the group; a click on an address
  selects the row. The arrow keys, Home, End, Ctrl+Home, Ctrl+End, Page Up and Page Down move the caret; Shift extends
  the selection; Tab switches between the digits and the characters.
- Edit ▸ **Copy** (Ctrl+C) copies the selected bytes of the focus pane as hex pairs (`de ad be ef`), or as characters
  when the caret is in the characters; **Copy as Text** always copies the characters; **Select All** selects every
  byte. Copy takes at most 16 MiB.
- Edit ▸ **Current Byte Address** copies the caret's address.

## Navigating and finding

- Ctrl+N and Ctrl+P go to the next and the previous difference; Alt+Down and Alt+Up to the next and the previous byte
  of the focus side, skipping the gaps.
- View ▸ Show All, Show Differences and Show Same filter the rows (with *Differences*, the context rows of the
  [Layout dialog](topic:dialog-hex-layout) show around them); the status bar counts the hidden rows.
- Ctrl+G opens [Go To](topic:dialog-hex-go-to); Ctrl+F opens the [Find panel](topic:panel-hex-find); F3 and Shift+F3
  find the next and the previous hit.
- In a view opened from a folder comparison, Ctrl+M and Shift+Ctrl+M open the next and the previous pair with
  differences.

## Values and information

- View ▸ **Little Endian Values** and **Big Endian Values** show the [value panel](topic:panel-hex-values) under the
  panes.
- The status bar shows the caret's address on both sides, the byte's state, the selection's length, the alignment,
  the filter and the result. Session ▸ **Hex Compare Info** (Ctrl+I) shows the figures ([Hex Compare
  Info](topic:dialog-hex-info)).
