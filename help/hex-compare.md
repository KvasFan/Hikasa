# hikasa hex compare

`hikasa hex compare <left> <right>` compares two files byte by byte, whatever they hold: executables, images, disk
images, databases. The bytes are aligned first, so an insertion near the start does not make every later byte
differ; the differing runs are listed as **sections**. The [exit code](topic:exit-codes) is the verdict: 1 identical
bytes, 11 the bytes differ, 100 an error; when a side was read through its format's load conversion, 2 the
converted outputs are identical and 13 they differ.

Files of any size are opened without reading them whole: a file up to `--unlocked-max` MiB (default 16, 0 to 1024)
is read into memory and released at once, a larger one is read in pages as needed. `-` reads standard input and
[archive](topic:archives) members are read like files.

## Alignment

| Flag | Meaning |
|---|---|
| `--alignment none` | byte n with byte n: the fastest, for files of the same layout (patched executables, fixed records) |
| `--alignment fast` (default) | content-defined blocks are matched, the gaps between them compared byte by byte; an insertion or a deletion is found at bounded cost, and two patched files of the same length give None's result |
| `--alignment complete` | every gap aligned byte by byte, in windows, within a work budget |
| `--block <bytes>` | Fast's average block: a power of two from 1K to 1M (default 8K) |
| `--skew <bytes>` | the farthest a block may move between the sides (default 16M) |
| `--window <bytes>` | Complete's window: 4K to 16M (default 256K) |
| `--budget <MiB>` | Complete's work budget (default 1024); when it runs out the rest is aligned as Fast aligns it, with the warning `alignment_degraded` |

Sizes are bytes, or a number with `K`, `M`, `G` (`KB`, `KiB`, … too; binary units): `--block 64K`, `--skew 1M`.
The statistics are exact in every mode; the verdict is the binary one (never 12): bytes that differ are a difference.

## Formats

A side whose name matches an enabled format of the *hex* kind is read through that format's load conversion (an
*External* program that turns the file into the bytes to compare); with no such format the raw bytes are compared.
`--format <id|name>` gives both sides a format, `--left-format`/`--right-format` one side (with `--format` the
combination is refused); the format must be of the hex kind ([hikasa format](topic:format)). With a converted side
the raw files are compared first: identical raw files are 1; otherwise the converted outputs decide, 2 when they are
identical, 13 when they differ. `--stdin-name <name>` names standard input for the detection.

## Output

Without a report flag the output is one line per section (`--display differences`, the default), then the summary
line and the verdict line:

```
0x00001F40 L 8  R 8  changed    de ad be ef 00 11 22 33 | 00 00 00 00 00 11 22 33
0x00002000 L —  R 4  right-only — | 01 02 03 04
hex compare: 2 sections, 8 bytes differ, 0 only left, 4 only right (alignment fast)
verdict: binary_different
```

- The address is the left offset (the right one for a section without left bytes), in the larger file's hex width;
  when both sides hold bytes at different offsets it reads `0x…/0x…` (left/right). `L` and `R` are the byte counts of
  each side (`—` for none); the kinds are `changed`, `left-only`, `right-only` and `mixed`.
- `--max-sections <n>` lists at most n sections (default 20, 0 all; then `... N more sections`), `--preview <bytes>`
  shows that many bytes of each side (default 16, 0 to 4096; a longer side ends with `…`).
- `--display all` or `--display same` prints the rows instead, as the side-by-side report shows them (`address hex
  chars ‖ address hex chars`, a `^` under each differing byte), `--display none` only the summary line. The row
  layout: `--bytes-per-line <n>` (1 to 256, default 16), `--group 1|2|4|8` (bytes between spaces), `--chars
  ascii|latin1|none` (the character column), `--address hex|decimal`, `--context <n>` (0 to 64 rows around differing
  rows of the reports). They shape rows only: with the sections of `--display differences` (or `none`) and no report
  they are refused (exit 100).
- `--json` prints `hikasa.hex_compare.v1` ([JSON output](topic:json-output)): both sides (`spec`, `size`, `mtime`, the
  `format`, `converted`, `load`: `memory`, `file`, `temp_cache`), the `alignment` (`mode`, the `requested` one,
  `block`, `skew`, `window`, `budget_mib`, `degraded`), the `verdict`, `stale`, the `stats` (`left_bytes`,
  `right_bytes`, `same_bytes`, `differing_bytes`, `left_only_bytes`, `right_only_bytes`, `sections`) and the listed
  `sections` (`index`, `kind`, `left` and `right` as `{offset, len}` or `null`, `columns`, the `preview` bytes as hex
  text), `sections_shown` and `sections_truncated`. `--max-sections` and `--preview` apply; the row layout flags do
  not, and with `--json` alone they are refused (exit 100). The JSON Schema is `docs/schemas/hex_compare.v1.json`.
- Warnings: `alignment_degraded`; `sections_truncated` (the engine keeps a bounded list of sections: the statistics
  stay exact, the sections past it are not listed); `input_changed` (a file changed while it was compared: the
  result may be stale, `stale: true`).

The rule flags start from the Hex Compare defaults session (`sessions/_defaults/hex-compare.toml`, the defaults above
when it is not there; `hikasa session edit --defaults hex-compare`), as `hikasa session new` does; a flag given here
wins.

## Reports

`--report summary|side-by-side|interleaved` (`--layout`) writes a report instead, as text or HTML (`--html`,
`--html-mono`); side-by-side is the default layout. *Summary* gives the statistics, the alignment and the first
sections with their previews; *side-by-side* the rows of both sides (`--display`, default all; `--context`), differing
bytes marked; *interleaved* each differing row's left row above its right one. `--line-numbers`/`--no-line-numbers`
show or hide the byte addresses; `--title`, `--options`, `--output-options`, `--output-to <file>|clipboard|printer|
browser` (`--output <file>`, never one of the inputs), `--dry-run` as for every report ([hikasa
report](topic:report)). A script writes the same reports with `hex-report` ([hikasa script](topic:script)); a Hex Compare session runs with `hikasa session run`
([hikasa session](topic:session)).

## Errors and exit codes

1 identical bytes, 11 different bytes, 2 and 13 for converted sides as above; 100 for a bad word (before anything is
read), an unknown format or one that is not of the hex kind, a folder given as a side, a file that cannot be read or
ends while it is read (`unexpected_eof`), a conversion that fails, a Ctrl-C.

## In the window

`--gui` opens the comparison in the window's Hex Compare view with the same words (`--wait` waits and exits with its
code, `--no-launch` does not start the window); the words are checked here first. The report flags,
`--max-sections` and `--preview` do not apply with it (exit 100). Until the window has the view it answers
`unsupported_view` (exit 100).

## Examples

```
hikasa hex compare old.bin new.bin
hikasa hex compare a.img b.img --alignment none --max-sections 0 --preview 0
hikasa hex compare v1.exe v2.exe --alignment complete --json
hikasa hex compare a.bin b.bin --report side-by-side --display differences --context 2 --html --output diff.html
```

See [hikasa hex dump](topic:hex-dump), [hikasa hex find](topic:hex-find), [hikasa quick](topic:quick) and
[Contents](topic:index).
