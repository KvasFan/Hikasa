# hikasa hex find

`hikasa hex find <PATTERN> <PATH>…` finds text or bytes in files of any kind and prints the offset of each hit, the
way the Hex Compare view's Find does. Files are read as bytes: nothing is decoded, converted or skipped as binary,
and files of any size are read in passes, never whole.

## What is found

- `PATTERN` is text, encoded as `--encoding utf-8|latin1|utf-16le|utf-16be` says (default UTF-8); ASCII letters match
  either case unless `--case`. With `--hex` it is bytes: hex digits in pairs, with spaces, commas, `-` or `:` between
  bytes and `0x` prefixes allowed (`DE AD ?? EF`, `dead??ef`, `0xDE,0xAD`); `??` matches any byte. `--encoding` and
  `--case` apply to text only (with `--hex` they are refused).
- Each `PATH` is a file, searched as given, or a folder, searched in name order: its files first, with `--recursive`
  (`-r`) the folders below it too. `-` reads standard input.
- `--mask <masks>` keeps the files whose names match (`*.bin;-test_*`; a mask ending in `/` or `\` is a folder), hidden
  files and folders are left out unless `--include-hidden`, and `--archives files` (the default) searches archives as
  bytes while `--archives opened` searches their members, as [hikasa text find](topic:text-find) walks folders.
- `--max-matches <n>` stops the search after n hits (default 100,000; warning `find_limit_reached`, the rest is not
  searched); `--max-matches-per-file <n>` stops a file after n hits (default 10,000). Both are at least 1.

## Output

- One line per hit: `<file>:0x<offset>` (the offset in upper-case hex).
- `--files-with-matches` (`-l`): each file with a hit, once. `--count` (`-c`): `<file>:<hits>` for each file with a hit.
- A file or folder that cannot be read is skipped with a `skipped: <file>: <message>` line on stderr; the search goes
  on.
- `--json` prints `hikasa.hex_find.v1` ([JSON output](topic:json-output)): the `query` (`kind`: `text` or `bytes`,
  `pattern`, `encoding`, `null` for bytes, `match_case`), the `roots`, `mask`, `recursive`, `include_hidden`,
  `archives`; each file with a hit (`spec`, `size`, `hits` as offsets (empty with `--count` and
  `--files-with-matches`), `hit_count`, `stopped` when the per-file limit stopped it); the `skipped` items (`spec`,
  `reason`, `error`); the `counts` (`files_searched`, `files_matched`, `hits`, `skipped`) and `limit_reached`. The JSON
  Schema is `docs/schemas/hex_find.v1.json`.

## Exit codes

0 whether or not anything was found; 100 for a pattern, encoding, mask or limit that is not valid, `--encoding` or
`--case` with `--hex` (before anything is read), a `PATH` that does not exist, a Ctrl-C. See
[Exit codes](topic:exit-codes).

## Examples

```
hikasa hex find --hex "50 4B 03 04" downloads -r --mask "*.bin"
hikasa hex find --hex "DE AD ?? EF" core.dump --max-matches 10
hikasa hex find "Copyright" firmware.img --case
hikasa hex find "name" save.dat --encoding utf-16le --count
hikasa hex find --hex 7f454c46 bin -r -l --json
```

See [hikasa hex compare](topic:hex-compare), [hikasa hex dump](topic:hex-dump) and [Contents](topic:index).
