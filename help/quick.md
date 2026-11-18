# hikasa quick

`hikasa quick <left> <right>` compares two files and prints nothing: the [exit code](topic:exit-codes) is the
result (1, 2, 11, 12, 13, or 100 on error).

## Methods

| Flag | Method |
|---|---|
| `--rules` (default) | rules-based: the file format decides what is important |
| `--binary` | byte by byte |
| `--crc` | CRC-32 checksums only |
| `--size` | sizes only |

`--size` and `--crc` report "binary same" (exit 1) when the sizes or checksums are equal, although the bytes
were not compared one by one.

## Rules-based options

- `--ignore-unimportant` (`--iu`): only unimportant differences count as same (12 becomes 2).
- `--format <id|name>` forces a file format; `--rules-file <toml>` loads text rules.
- `--unimportant <list>`, `--unimportant-text <text>`, `--unimportant-regex <re>` mark what does not matter.
- `--compare-line-endings` makes line endings important; `--stdin-name <name>` names standard input.

## Which engine compares

With `--rules` the format of the pair (the left name's, else the right's) chooses the engine:

- **Text** formats compare line by line with their rules, as [hikasa text compare](topic:text-compare).
- **Table** formats (CSV, workbooks) compare cell by cell, as [hikasa table compare](topic:table-compare).
- **Picture** formats compare by their pixels, as [hikasa picture compare](topic:picture-compare), with the tolerance,
  Auto Scale, tone map and picture size limit of the Picture Compare defaults session
  (`sessions/_defaults/picture-compare.toml`, else the built-in defaults: tolerance 0): a PNG saved again as BMP is 2,
  a JPEG saved again within the tolerance 12. A pair the decoders do not support (DDS, AVIF) is compared byte by byte
  with the warning `fell_back_to_binary`; a picture that does not decode (damaged, too large) is an error (100). A
  picture format with a load conversion (an *External* program) is compared through the program's output, as by
  `picture compare` (`--format` forces one for both sides). A
  file over 512 MiB is compared byte by byte (`rules_size_limit`).
- **Hex** formats compare byte by byte: a hex verdict is the binary one ([hikasa hex compare](topic:hex-compare)).
- Two files with identical bytes are 1 before any engine reads them.

## Example

```
hikasa quick --iu old/main.c new/main.c && echo "no important change"
```

See [hikasa text compare](topic:text-compare) for the differences themselves.
