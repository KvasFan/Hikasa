# hikasa text convert

`hikasa text convert <FILE>…` converts text files the way the text views' conversion commands do: trailing
whitespace, tabs and spaces, line endings, the encoding and the byte-order mark. Lines that do not change are written
back byte for byte, whatever the encoding (also bytes that could not be decoded); only converted lines are encoded.

## Conversions

At least one is needed (else exit 100, "nothing to convert"). They run in this order, whatever the order of the
flags: trim, tabs, line endings, then the encoding and the BOM.

- `--trim-trailing` removes the whitespace at the end of each line (spaces, tabs, no-break spaces and the other
  Unicode white space); line endings stay.
- `--tabs-to-spaces` turns every tab into spaces up to the next tab stop; `--spaces-to-tabs` turns the leading
  spaces that reach a tab stop into tabs (spaces short of a stop stay). Tab stops are counted in columns, a wide
  character taking two. `--tab-size <1-32>` sets them (default: the format's).
- `--line-endings lf|crlf|cr` gives every line that has one that ending; the last line keeps having none.
- `--encoding <label>` writes the file in another encoding (`UTF-8`, `UTF-16LE`, `windows-1252`, `Shift_JIS`, …):
  every line is then encoded. `--bom` and `--no-bom` add or remove the byte-order mark; without them the file keeps
  its BOM when the new encoding has one. `--bom` with an encoding that has no BOM fails that file.
- `--lines <a>-<b>` limits the trim, tab and line-ending conversions to those lines of the file (counted from 1).

## Reading

Each file is read as `hikasa text compare` reads it: the format by its name, or `--format <id|name>`; the encoding
detected, or `--encoding-in <label>`. `-` reads standard input (`--stdin-name <name>` for the format). A file that
cannot be read, a binary one, or one whose format converts it without a way back (*Sorted*, an *External* format
without a save program) fails, and the others go on. A file the conversions do not change is skipped (`no change`)
and never rewritten: its time stays.

## Where the result goes

- Without `--in-place` or `--output`, the converted bytes of the one file go to stdout; the file is untouched and
  the summary line goes to stderr. Several files need `--in-place`.
- `--in-place` rewrites every file, as one plan. Each replaced file is backed up first when
  `[backups] back_up_before_save` says so (the default; `naming` and `folder` name the backup); `--backup` and
  `--no-backup` decide for this run. A failed backup fails that file, which stays as it was.
- `--output <FILE>` (one input) writes the converted file there; an existing file is replaced only after a
  question, and is backed up first.
- A file is written through a symbolic link (the link stays); what is not a regular file is refused. A member of a
  zip or tar archive is written by rebuilding the archive once.

## Questions

A file that changed on disk since it was read, a read-only target, an existing `--output`, a backup that would
replace a file Hikasa did not make there, and converted lines holding characters that could not be decoded are
questions, asked on the terminal (`[y]es [n]o [a]ll [s]kip all [c]ancel`, see
[hikasa folder op](topic:folder-op)):

- `--yes` (`-y`) answers yes to them, except a changed file: `--force` overwrites that one. `--confirm
  prompt|yes-to-all|no-to-all` sets the mode; `no-to-all` skips every file that asks.
- Without a terminal an open question stops the command before anything is written: exit 100, naming the questions
  and their counts.
- A character the new encoding cannot write: `--unmappable replace` writes `?` (warning `unmappable_replaced`),
  `--unmappable refuse` fails the file. Without the flag a terminal asks, and without a terminal the file fails.
  Characters are never written as numeric references.

## Output

- One line per file: `converted <n> lines: <file>` (`would convert …` with `--dry-run`), `no change: <file>`,
  `failed: <file>: <reason>`. With `--verbose` the plan follows: `write <file> (<bytes> bytes, <encoding>[ BOM],
  <line endings>[, backup <file>][, via <link>])`.
- `--dry-run` prints the plan and the counts and writes nothing.
- `--json` prints `hikasa.text_convert.v1` ([JSON output](topic:json-output)): per file its format, the encoding,
  BOM and line endings before and after, each conversion with the lines it changed, its status (`planned`, `done`,
  `skipped`, `failed`, `not_run`), the skip reason, the error, the questions and the warnings; the plan's items
  (`write` with the backup, the bytes, the bytes kept from the file, the lines encoded); the counts. With the stdout
  target `--json` needs `--dry-run`.

## Exit codes

0 when every file was converted or skipped; 100 when a file failed, after a Ctrl-C, and for a command that cannot run
(nothing to convert, several files to stdout, an unknown encoding or format, open questions without a terminal). A
run that wrote some files and failed others is `ok: true` with exit code 100. See [Exit codes](topic:exit-codes).

## Examples

```
hikasa text convert notes.txt --trim-trailing > clean.txt
hikasa text convert src/*.c --spaces-to-tabs --tab-size 4 --in-place
hikasa text convert old.txt --encoding-in windows-1252 --encoding UTF-8 --line-endings lf --output new.txt
hikasa text convert *.txt --line-endings crlf --in-place --dry-run --json
```

See [hikasa text compare](topic:text-compare) and [Contents](topic:index).
