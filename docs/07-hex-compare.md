# Hex Compare

Compares raw bytes of two files side-by-side or over-under, like a hex editor.

- **HX-001** (P0): Panes show address column (toggle), hex bytes and an ASCII/
  Latin-1 column; bytes per row configurable (Layout dialog: bytes per line, group
  size, show characters). Files are memory-mapped when local; remote/archive files are
  fetched to a temp cache. Multi-GB files must open instantly (lazy rows).
  *M2 notes (§1.12.3, §1.12.9, D-326, D-332):* the hex document reads through the VFS: a local file of at most
  *unlocked max* bytes (16 MiB) is read whole and its handle closed at once; a larger one keeps one read handle for
  positional reads; archive members and other back-ends read the temp cache file; standard input and the clipboard
  are in memory. Pages of 64 KiB go into a fixed LRU cache of 256 pages (16 MiB a document); no mapping outlives a
  call — the bulk passes read through the chunk visitor (`11-vfs-remote-archives.md` §3), which maps local files one
  8 MiB chunk at a time. A short read is an error (`unexpected_eof`), never zero bytes; the (length, modification
  time) stamp of each input is re-checked after every job (`input_changed`). Rows cut the aligned columns every
  *bytes per line* (1–256, default 16; groups of 1, 2, 4 or 8 bytes; characters ASCII 0x20–0x7E, Latin-1 also
  0xA0–0xFF, or none; hex or decimal addresses); an address is that side's offset of its first byte in the row,
  so a gap is empty cells on the other side. Before the alignment lands, the first screen shows None's layout
  from the first pages.
- **HX-002** (P0): Session rules: **Alignment** = None (byte n ↔ byte n), Fast
  (block-hash based, for large files), Complete (full alignment of insertions and
  deletions, byte-level Myers on chunks with skew limit); *Files up to this size are
  loaded without file locks* (N MB; small files stay editable by other programs).
  *M2 notes (§1.12.4–§1.12.7, D-327…D-330):* every mode gives ordered segments (*paired*, *left only*, *right
  only*) that cover both files once; a paired byte is same or different by its bytes, never by a hash. **None**:
  one paired segment of the shorter length and the longer file's tail; its scan finds the differing runs 32 bytes at
  a time. **Fast**: content-defined blocks (a gear hash over each 8 MiB chunk, average `[rules.hex] block`, default
  8 KiB, minimum a quarter, maximum 8 times), aligned by the Standard engine with a skew of `[rules.hex] skew`
  (default 16 MiB) in blocks; a gap whose sides are both at most 64 KiB is aligned byte by byte, a larger gap is
  paired over its shorter side with the rest one-sided at its end (or at its start when that pairs fewer differing
  bytes). **Complete**: the same blocks, then every gap byte by byte with Myers in windows of `[rules.hex] window`
  (default 256 KiB; the byte-level skew is half of it) within `[rules.hex] budget_mib` (default 1,024 MiB, charged
  in gap order; beyond it gaps are laid out as Fast lays them out and the result says `alignment_degraded`). A
  window of unrelated bytes (an 8-byte-gram probe, or fewer equal bytes than an eighth of its shorter side) is laid
  out without Myers. Each layout is the better of its candidates by non-same columns (Myers or position pairing for a
  whole gap; Complete's walk or Fast's layout for a large gap; the block layout or None's for the files), so
  Complete never shows more differing columns than Fast, nor Fast than None; swapping the files mirrors the result.
  **Sections**: maximal runs of non-same columns, runs fewer than 8 same columns apart merged; at most
  `max_sections` (1,048,576) are listed, the statistics stay exact. The **thumbnail** has 4,096 buckets of same,
  different, left-only and right-only column counts.
- **HX-003** (P0): Hex file formats: mask, description, conversion (load/save
  external program) only.
  *M2 notes (§1.12.13, D-336):* a Hex format's External load conversion writes the converted bytes to the temp
  cache and the document reads them (*Converted* in the File Info); the verdict then follows the conversion rows of
  HX-009's note. Detection is by name among enabled Hex formats; no Hex preset ships.
- **HX-004** (P0): Colouring: different bytes red; orphan (inserted/deleted) bytes
  with gaps on the other side; thumbnail; File Info; display font commands.
- **HX-005** (P0): Display filters: Show All / Differences / Same; navigation
  Next/Previous Difference Bytes (Ctrl+N/P), Next/Previous Existing Byte, Go To
  address (Ctrl+G; hex or decimal), Find/Replace (text or hex byte sequence, Ctrl+F/H),
  Find Next/Previous, Next/Previous Difference Files (child).
  *M2 notes (§1.12.9–§1.12.11, D-332…D-334):* the *Differences* filter shows the rows that intersect a section, with
  0–64 context rows; *Same* the others; hidden runs are counted (reports print *N rows hidden*). Next/Previous
  Difference go to section starts (from inside a section, Previous goes to the one before it); past a cut section
  list the next difference is found by scanning. Go To reads `0x1F`, `1Fh`, `$1F` (hex), `#31`, `31d` (decimal), bare
  digits in the dialog's radix, `+n`/`-n` relative to the caret, `end`; past the end it stops at the last byte.
  Find takes text (UTF-8, Latin-1, UTF-16 LE/BE; ASCII letters match either case unless *match case*) or hex bytes
  (`DE AD ?? EF`, `dead??ef`, `0xDE,0xAD`; `??` any byte), at most 4,096 bytes; hits may overlap and a hit across a
  chunk or page edge is found once. Replace waits for editing (HX-006, P1).
- **HX-006** (P1): Editing: typing in overwrite (default) or insert mode (Ins toggles),
  Cut/Copy/Paste/Delete, Select All, Copy to Right/Left (selected or current bytes;
  Ctrl+R/L), Undo/Redo, Save/Save As with backups.
- **HX-007** (P0): **Current Byte Address** status/command (copies the address);
  value interpretation panel: *Little Endian Values* / *Big Endian Values* (int8/16/32/
  64 signed/unsigned, float32/64 at the cursor).
  *M2 notes (§1.12.10, §1.12.12, D-333, D-335):* the address text is `0x0000_1F40` (`_` every 4 hex digits, at least
  8 digits, more for larger files) or plain decimal. The panel shows each type little and big endian (`n/a` when
  fewer bytes remain; NaN and infinities as read), and the UTF-8 character that starts at the cursor when the bytes
  there are a valid sequence.
- **HX-008** (P1): Report layouts: side-by-side, summary, interleaved; option
  "line-numbers" prints byte addresses.
- **HX-009** (P0): Binary quick compare (folder content test and CLI `--qc=binary`)
  shares the chunked comparison code (configurable buffer size, default 1 MiB).
  *M2 notes (§1.12.2, §1.12.8, D-325, D-331):* one chunk visitor in the VFS layer serves the binary comparison, the
  CRC and every hex pass (8 MiB chunks; the buffer size sizes the reads only). `hex compare` answers the binary
  question whatever the alignment: `binary_same` (1) for identical bytes, `binary_different` (11) otherwise — the
  same exit code as the binary quick compare for every mode; with a converted side, raw bytes identical → 1,
  converted outputs identical → `rules_same` (2), else `important_differences` (13); errors 100; never 12.
