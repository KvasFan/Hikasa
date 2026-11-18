# hikasa hex dump

`hikasa hex dump <spec>` prints the bytes of one file as rows of hex digits and characters, the way a pane of the
Hex Compare view shows them, and with `--values` what the bytes at the offset mean as numbers and text. It reads only
the bytes it prints, so it works on files of any size; `-` reads standard input, and [archive](topic:archives)
members are read like files.

## Flags

| Flag | Meaning |
|---|---|
| `--offset <addr>` | the first byte (default 0): decimal digits, `0x1F`, `1Fh`, `$1F` (hex), `#31`, `31d` (decimal), `+n`, `end` (the last byte); `_` separators allowed; an offset past the end of the file, or a negative one, is refused (exit 100) |
| `--length <n>` | bytes printed (default 256, at most 16M; `K`, `M` units) |
| `--bytes-per-line <n>` | bytes per row: 1 to 256 (default 16) |
| `--group 1\|2\|4\|8` | bytes between spaces (default 1; groups are separated by two spaces) |
| `--chars ascii\|latin1\|none` | the character column: printable ASCII (default), also Latin-1 letters, or none; other bytes are `.` |
| `--address hex\|decimal` | addresses in hex (default, at least 8 digits) or decimal |
| `--values` | the value panel at the offset |
| `--format <id\|name>` | a format of the hex kind: the file is dumped as its load conversion gives it |
| `--stdin-name <name>` | names standard input, for the format detection |

A file whose name matches an enabled format of the hex kind is dumped through that format's conversion too
([hikasa format](topic:format)). A dump that reaches the end of the file stops there.

## Output

```
00000000  89 50 4e 47 0d 0a 1a 0a 00 00 00 0d 49 48 44 52  .PNG........IHDR
00000010  00 00 01 00 00 00 01 00 08 06 00 00 00 5c 72 a8  .............\r.
```

With `--values` a blank line, `values at 0x0000_0040:` (the offset in hex), then the value panel: a *Little Endian
Values* and a *Big Endian Values* block, each with `Int8`, `UInt8`, `Int16`, `UInt16`, `Int32`, `UInt32`, `Int64`,
`UInt64`, `Float32`, `Float64` and the `UTF-8` character starting there (`U+0041 A`); `n/a` where fewer bytes remain.

`--json` prints `hikasa.hex_dump.v1` ([JSON output](topic:json-output)): the `side` (`spec`, `size`, `mtime`,
`format`, `converted`, `load`), the `layout` (`bytes_per_line`, `group`, `chars`, `address`), `offset`, `length`, the
`rows` (`address` as a number, `hex` as lower-case digits without spaces, `chars`, `null` with `--chars none`) and
`values` (`null` without `--values`; else `offset`, `little` and `big`, each with `i8`, `u8`, `i16`, `u16`, `i32`,
`u32` as numbers, `i64` and `u64` as decimal strings, `f32` and `f64` as numbers or `"NaN"`, `"inf"`, `"-inf"`, and
`utf8`). `--group` and `--address` shape the human output only: with `--json` they are refused (exit 100). The JSON
Schema is `docs/schemas/hex_dump.v1.json`.

## Exit codes

0; 100 for a bad word (before anything is read), a folder, a file that cannot be read, an unknown format or one that
is not of the hex kind, a conversion that fails, a Ctrl-C. See [Exit codes](topic:exit-codes).

## Examples

```
hikasa hex dump firmware.bin --offset 0x1F40 --length 64
hikasa hex dump data.bin --offset 4096 --length 1K
hikasa hex dump header.bin --bytes-per-line 8 --group 4 --chars none --address decimal
hikasa hex dump record.dat --offset 0x40 --length 8 --values --json
```

See [hikasa hex compare](topic:hex-compare), [hikasa hex find](topic:hex-find) and [Contents](topic:index).
