# Go To (Hex Compare)

Search ▸ **Go To…** (Ctrl+G) of a [Hex Compare view](topic:view-hex-compare) moves the caret to a byte of one side.

- **Address**: `0x1F40`, `1F40h` and `$1F40` are hexadecimal; `#8000` and `8000d` decimal; bare digits are read in
  the radix chosen below; `+16` and `-16` move from the caret; `end` is the last byte. Underscores are allowed
  (`0x0000_1F40`, as Current Byte Address copies it). The recent addresses are in the list beside the field.
- **Digits are**: hexadecimal or decimal (remembered).
- **Side**: the left or the right file (the focus side by default).

An address past the end goes to the last byte and says so in the message panel. A text that is no address keeps the
dialog open with the reason. `hikasa hex dump --offset` reads the same spellings.
