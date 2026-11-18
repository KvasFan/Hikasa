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
without a save program) fails, and the others go on. With `--in-place`, a file the conversions do not change is
skipped (`no change`) and never rewritten: its time stays. To stdout such a file still comes out, byte for byte as it
is, and `--output` gets a copy of it.

A file is read once and held once: the conversions run line by line while the result is written, so a large file
takes about its own size in memory, whatever the conversions. With `--in-place`, what is not a regular file (a FIFO,
a device) is refused before it is read; to stdout or `--output` a pipe is read as its content.

## Where the result goes

- Without `--in-place` or `--output`, the converted bytes of the one file go to stdout; the file is untouched and
  the summary line goes to stderr. Several files need `--in-place`. Lines kept from the file are checked against the
  file as it was read when the reading ends: up to 64 MiB of output is held until then, so a file that changed
  meanwhile writes nothing (exit 100). A larger result is written as it is made, and a change found at the end
  leaves the part already written on stdout (still exit 100): do not use it.
- `--in-place` rewrites every file, as one plan. Each replaced file is backed up first when
  `[backups] back_up_before_save` says so (the default; `naming` and `folder` name the backup); `--backup` and
  `--no-backup` decide for this run. A failed backup fails that file, which stays as it was.
- `--output <FILE>` (one input) writes the converted file there; an existing file is replaced only after a
  question, and is backed up first.
- A file is written through a symbolic link (the link stays); what is not a regular file is refused. A member of a
  zip or tar archive is written by rebuilding the archive once; it keeps its place and its header (owner, mode).
- A rewritten file stays the same file: its other hard links, owner, group, ACLs and extended attributes stay with
  it. When a rename would lose them (another hard link, another owner, a folder Hikasa cannot write), the file is
  written in place, after its backup was copied; a failure puts the old content back.

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
