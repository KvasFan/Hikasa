# Hex Compare Info

Session ▸ **Hex Compare Info** (Ctrl+I) of a [Hex Compare view](topic:view-hex-compare) shows the figures of the
comparison:

- **Bytes**: each file's size, the same and differing bytes, and the bytes only one side has.
- **Alignment**: the number of difference sections, the mode asked and the mode used, the block size, skew, window and
  budget, whether the alignment ran out of its budget (*degraded*) and whether the section list was cut
  (*sections truncated*).
- **Warnings**, when there are any (an input changed during the comparison, for example).

**Copy** copies the summary line and the result, as `hikasa hex compare` prints them.
