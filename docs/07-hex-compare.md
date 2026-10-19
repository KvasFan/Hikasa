# Hex Compare

Compares raw bytes of two files side-by-side or over-under, like a hex editor.

- **HX-001** (P0): Panes show address column (toggle), hex bytes and an ASCII/
  Latin-1 column; bytes per row configurable (Layout dialog: bytes per line, group
  size, show characters). Files are memory-mapped when local; remote/archive files are
  fetched to a temp cache. Multi-GB files must open instantly (lazy rows).
- **HX-002** (P0): Session rules: **Alignment** = None (byte n ↔ byte n), Fast
  (block-hash based, for large files), Complete (full alignment of insertions and
  deletions, byte-level Myers on chunks with skew limit); *Files up to this size are
  loaded without file locks* (N MB; small files stay editable by other programs).
- **HX-003** (P0): Hex file formats: mask, description, conversion (load/save
  external program) only.
- **HX-004** (P0): Colouring: different bytes red; orphan (inserted/deleted) bytes
  with gaps on the other side; thumbnail; File Info; display font commands.
- **HX-005** (P0): Display filters: Show All / Differences / Same; navigation
  Next/Previous Difference Bytes (Ctrl+N/P), Next/Previous Existing Byte, Go To
  address (Ctrl+G; hex or decimal), Find/Replace (text or hex byte sequence, Ctrl+F/H),
  Find Next/Previous, Next/Previous Difference Files (child).
- **HX-006** (P1): Editing: typing in overwrite (default) or insert mode (Ins toggles),
  Cut/Copy/Paste/Delete, Select All, Copy to Right/Left (selected or current bytes;
  Ctrl+R/L), Undo/Redo, Save/Save As with backups.
- **HX-007** (P0): **Current Byte Address** status/command (copies the address);
  value interpretation panel: *Little Endian Values* / *Big Endian Values* (int8/16/32/
  64 signed/unsigned, float32/64 at the cursor).
- **HX-008** (P1): Report layouts: side-by-side, summary, interleaved; option
  "line-numbers" prints byte addresses.
- **HX-009** (P0): Binary quick compare (folder content test and CLI `--qc=binary`)
  shares the chunked comparison code (configurable buffer size, default 1 MiB).
