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

## Example

```
hikasa quick --iu old/main.c new/main.c && echo "no important change"
```

See [hikasa text compare](topic:text-compare) for the differences themselves.
