# hikasa text find

`hikasa text find <PATTERN> <PATH>…` finds text in files and folders, the way the text views' *Find in Files* does,
and prints each hit as it is found. With `--replace` it replaces every occurrence in the files it found, as one plan:
lines without a hit are written back byte for byte, whatever the encoding.

## What is found

- `PATTERN` is plain text; with `--regex` a regular expression (`$`, `^` and `\b` see one line; a pattern that can only
  match a line break is refused, since a hit never crosses a line end). Case is ignored unless `--case` is given (the
  Unicode letters too). `--word` keeps the hits that are whole words; the format's word characters count as letters.
- Each `PATH` is a file, searched as given, or a folder, searched in name order: its files first as listed, with
  `--recursive` (`-r`) the folders below it too. `-` reads standard input.
- `--mask <masks>` keeps the files whose names match: `*.c;*.h`, a leading `-` excludes (`-*.min.js`), and a mask
  ending in `/` or `\` is a folder (`-build/` skips that folder and what it holds). Case follows the system's file
  names. A file given as a `PATH` is searched whatever the masks.
- Hidden files and folders (a name starting with `.`, the hidden attribute on Windows) are left out unless
  `--include-hidden`. Links to files are followed; links to folders are not (a search never loops).
- `--archives files` (the default) leaves archives out as binary files; `--archives opened` searches their members,
  zip and tar archives and compressed streams, as folders (with `--recursive` for an archive inside a folder).

## How files are read

- Each file is read as `hikasa text compare` reads it: the encoding detected (UTF-8, UTF-16 with or without a BOM,
  legacy encodings), or `--encoding <label>` for every file; the format by the file's name, or `--format <id|name>`.
- Binary files are skipped. A file larger than `--max-file-size` (default 128MB; `KB`, `MB`, `GB` are binary units) is
  skipped before it is read. A file whose format converts it (an *External* format, *Sorted*) is skipped unless
  `--convert`: then its converted text is searched.
- A file or folder that cannot be read is skipped in its place; the search goes on. A `PATH` that does not exist stops
  the command before anything is read.
- Files are searched in parallel on the worker threads, and printed in the order above whatever their number: the
  output is the same with one thread or many. Memory depends on the files being read, never on the number of hits.

## Output

- One line per hit: `<file>:<line>:<column>: <text of the line>`. Lines and columns count from 1, columns in
  characters; a long line is cut to 400 characters around the hit. Control characters are escaped.
- `--files-with-matches` (`-l`): each file with a hit, once. `--count` (`-c`): `<file>:<hits>` for each file with a hit.
- `--verbose` adds the skipped files on stderr: `skipped (<reason>): <file>` (`binary`, `too_large`, `unreadable`,
  `converted`, `filtered` for a link to a folder).
- `--max-matches <n>` stops the search after `n` hits (default 100,000; warning `find_limit_reached`, the rest is not
  searched); `--max-matches-per-file <n>` stops a file after `n` hits (default 10,000).
- `--json` prints `hikasa.text_find.v1` ([JSON output](topic:json-output)): the query, the roots, each file with a hit
  (its encoding, its hits with line, column, length, the preview and the hit's place in it, whether the file stopped
  early), the skipped files with the reason and the error, the counts, `limit_reached`, and the replace.
- When the reader of the output goes away (`| head`), the search stops.

## Replacing

`--replace <TEXT>` replaces every occurrence in each file with a hit; a file the search did not reach (after
`--max-matches`) is not touched, and a file past `--max-matches-per-file` is replaced whole. With `--regex` the
replacement may name groups: `$1`, `${1}`, `${name}`, and `$$` for a dollar sign; a group the pattern does not have is
refused before anything is read. Line breaks in the replacement become the file's own.

- The files are read again as the search read them and written as one plan: each file is rewritten in place, through a
  link when it is one, a zip or tar member by rebuilding its archive once. Each replaced file is backed up first when
  `[backups] back_up_before_save` says so (the default); `--backup` and `--no-backup` decide for this run.
- `--dry-run` prints the hits, then per file `would replace <n> occurrences: <file>` and the plan (`write <file>
  (<bytes> bytes, <encoding>[ BOM], <line endings>[, backup <file>])`); nothing is written.
- Otherwise the question *Replace N occurrences in M files?* is asked on the terminal, then the questions of each file
  (a read-only file, a backup that would replace another file), as [hikasa folder op](topic:folder-op) asks them:
  `[y]es [n]o [a]ll [s]kip all [c]ancel`. `--yes` (`-y`) answers them all yes; `--confirm yes-to-all|no-to-all|prompt`
  sets the mode (`no-to-all` replaces, and skips each file that asks). Without a terminal an open question stops the
  command before anything is written (exit 100, the questions named).
- A file changed on disk since it was read for the plan is not written: its item fails (`changed since the plan was
  made`), and the others are written. A file with no occurrence left when it is read again is not rewritten (`no
  change`).
- A file that cannot be written (binary, a format that converts it without a save program, a member of a compressed
  stream) fails its item. A character the file's encoding cannot write: `--unmappable replace` writes `?` (warning
  `unmappable_replaced`), `--unmappable refuse` fails the file; without the flag a terminal asks and without one the
  file fails.
- After the run: `replaced <n> occurrences: <file>`, `no change: <file>`, `failed: <file>: <reason>`; with `--verbose`
  the plan's lines too.

## Exit codes

0 whether or not anything was found, and after a replace that wrote every file; 100 for a pattern or replacement that
is not valid, a `PATH` that does not exist, a word or limit that is not valid, an unknown encoding or format, a Ctrl-C,
open questions without a terminal, and a replace with a file that failed (the document is then `ok: true`: the others
were written). See [Exit codes](topic:exit-codes).

## Examples

```
hikasa text find TODO src --recursive --mask "*.rs;-target/"
hikasa text find "fn (\w+)_old\b" src -r --regex --case -l
hikasa text find colour docs -r --word --count
hikasa text find "old_(\w+)" src -r --regex --replace 'new_${1}' --dry-run
hikasa text find http:// site -r --mask "*.html" --replace https:// --yes --no-backup
hikasa text find error logs.zip --archives opened --json
```

See [hikasa text compare](topic:text-compare), [hikasa text convert](topic:text-convert) and [Contents](topic:index).
