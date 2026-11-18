# Find (Hex Compare)

Search ▸ **Find…** (Ctrl+F) of a [Hex Compare view](topic:view-hex-compare) opens a one-line panel above the panes.
With up to 64 bytes selected, they become the pattern.

- **Text** finds characters in an encoding (UTF-8, Latin-1, UTF-16 LE, UTF-16 BE); **Match case** distinguishes upper
  and lower case letters.
- **Hex bytes** finds bytes written as hex pairs (`DE AD ?? EF`; `??` matches any byte).
- **Wrap around** continues from the other end of the file; the side chooses the file searched.
- **Find Next** (F3, Enter) searches from after the caret; **Find Previous** (Shift+F3) before it. The hit's bytes are
  selected. *Not found* and *Wrapped to the start* show in the message panel; Esc or the close button stops a search.

A pattern that cannot be searched (empty, more than 4,096 bytes, only `??`, malformed hex, a character the encoding
cannot write) is explained in the panel and its buttons are off. [hikasa hex find](topic:hex-find) searches files and
folders from the command line with the same patterns.
