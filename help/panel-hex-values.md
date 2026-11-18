# Values (Hex Compare)

View ▸ **Little Endian Values** and **Big Endian Values** of a [Hex Compare view](topic:view-hex-compare) show the
value panel under the panes: the bytes at the caret of the focus pane read as numbers — signed and unsigned 8, 16, 32
and 64 bits, 32- and 64-bit floating point, and the UTF-8 character that starts there — in little endian, big endian
or both. Each submenu's **Show** shows its block; **Integers in Decimal** and **Integers in Hexadecimal** choose how
integers are written. On a gap the values are those of the side's next byte; *n/a* marks a type longer than the bytes
left.

The panel's context menu copies its lines. `hikasa hex dump --values` prints the same text.
