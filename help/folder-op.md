# hikasa folder op

`hikasa folder op <VERB> <left> <right>` compares two folders, then runs one file operation on what you select. Every
operation is planned first, from the comparison: `--dry-run` prints the plan and changes nothing, and a run does exactly
what the plan says.

## Verbs

| Verb | Flags | Does |
|---|---|---|
| `copy` | `--to left\|right`, `--overwrite`, `--no-preserve`, `--preserve-created`, `--symlinks copy\|skip`, `--backup`, `--no-backup` | copies to the other side |
| `move` | as `copy` | moves to the other side |
| `exchange` | | swaps both sides; an orphan moves; neither side can be lost at any step |
| `delete` | `--side left\|right\|both`, `--recycle`, `--recycle-fallback` | deletes on the side named |
| `rename` | `--side`; `--name <NEW>`, `--mask <MASK>` or `--regex <OLD> <NEW>` | renames; a rename never replaces |
| `touch` | `--side`; `--time <RFC 3339>`, `--now` or `--from-other` | sets the modification time |
| `mkdir` | `--side`, `--in <REL>`, `--name <NAME>` | makes a folder, or an [archive](topic:archives) by its name; a trailing `/` wants a folder |
| `copyto`, `moveto` | `--side`, `--target <SPEC>`, `--structure relative\|base\|none`, `--overwrite` | copies or moves out of the comparison; a trailing `/` on the target wants a folder |
| `compare` | `--method crc\|binary\|rules` | runs a content test on the selection |
| `crc` | `--side` | computes CRC-32 |
| `exclude` | `--session <REF>`, `--what items\|type`, `--force` | adds the selection to the session's exclude lists |
| `ignored` | `--session <REF>`, `--off`, `--force` | marks the selection Ignored in the session |
| `open-with` | `--path <REL>`, `--app <NAME\|INDEX>` or `--associated`, `--side` | opens with an application of `openwith.toml`; `%n` is the name, given as `./-x` when it starts with `-` |

`--session <REF>` takes the place of the two sides: it gives the specs, the rules, the read-only sides and the Ignored marks,
and flags go on top. All the rule flags of [folder compare](topic:folder-compare) work here.

## What is selected

Without `--select` or `--path` **nothing is selected**: the command exits 100, so a stray command cannot act on a whole tree.

- `--select <MASK>` adds what a mask matches; several add up. A mask is up to three words joined by `.`: a side (`left`, `lt`,
  `right`, `rt`, `all`), a state (`all`, `exact`, `diff`, `newer`, `older`, `orphan`), a kind (`all`, `files`, `folders`):
  `left.diff.files`, `newer`, `all`. `empty.folders` is a folder with no file below it. `diff` means differences other than
  the time. A folder matches a state only when everything shown below it does.
- `--path <REL>` adds one row; `left:a/b`, `right:a/b` name a side, otherwise the verb's. It never names the whole tree: an
  empty path, `.` or `/` is an error, and the whole tree takes `--select all`.
- `--display <FILTER>` says what `--select` sees (default `all`). `--no-expand` makes each folder one row, `--expand <REL>`
  opens only those. `--ignore-unimportant` reads unimportant differences as same.

A selected folder brings the rows that are shown, without what file filters hide; `--include-hidden` brings everything. A
folder and a row below it selected together: only the rows below count. Where something below a folder stays out, the
folder stays too. A folder the comparison did not list (`--no-scan-orphans`, `--no-auto-scan`) is listed when the plan is
made, through the same filters, and its files are planned, stamped and asked about as rows are.

## Questions

A plan asks before it does what cannot be undone: one question for the operation, then one per item for a read-only or
system target, a target newer than its source, a move that replaces, an item that cannot go to the recycle bin, a backup
that would replace a file the plan did not make. A folder that replaces a file asks what a file would, and a change in a
read-only archive asks for the archive. What lies in a folder you answer no to stays with it.

| Flag | Answers |
|---|---|
| `--yes`, `--confirm yes-to-all` | everything yes |
| `--confirm no-to-all` | the operation yes, each item question no: the items without one go on |
| on a terminal | one line per question: `[y]es [n]o [a]ll [s]kip all [c]ancel`; `a` and `s` answer the rest of that class; Ctrl-C cancels |
| no terminal, no `--yes` | exit 100, nothing touched |
| `--dry-run` | nothing asked; each item's conflicts are shown as `[read_only,overwrite_newer]` |

`delete` asks whatever `confirm_delete` says: a script's safety does not depend on a settings file. `--read-only
left|right|both` refuses a command that would write that side (`permission_denied`); `copyto` out of it is fine, and a
`--target` that leads into it, by any spelling or link, is refused too.

## What protects your files

- **A plan remembers what it saw.** A file changed since the comparison is not overwritten or deleted: the item fails
  `changed since the plan was made`, and the others go on.
- **The base folders are a fence.** Nothing outside them is deleted or written, and nothing below a followed link.
- **A failed comparison is no difference.** Rows that failed are skipped (`compare_error`), and the exit code is 100.
- **Backups.** With `--backup`, or `[backups] back_up_before_copy`, what a copy or move replaces goes to a backup first, named
  by `[backups] naming`; the names are in the plan. An archive is backed up once per operation, as a whole.
- **Recycle bin.** `--recycle` sends a whole folder as one item. What has no bin is a question. If the bin fails the item
  fails, unless `--recycle-fallback` says to delete permanently, and then only what the plan saw: a file that came since, or
  changed, stays, and so does its folder.

## Renaming

A `--mask` is split at its last `.`: `?` keeps a character, `*` keeps the rest, anything else is written: `*_old.*` turns
`report.txt` into `report_old.txt`. `--regex` matches the whole name; `$1`, `${1}`, `${name}` and `$$` work in `<NEW>`. A rename
that would take a name in use fails `already_exists`. A chain (`a → b`, `b → c`) is ordered, and a cycle goes through a
hidden name.

## Output

One line per item, in plan order: `L` or `R` for the side, `-` for a target outside the bases, then the summary. `--json` prints
`hikasa.folder_op.v1`; `compare` and `crc` print `hikasa.folder_contents.v1`; `exclude` and `ignored` print
`hikasa.session_write.v1`.

| Outcome | Exit |
|---|---|
| every item done or skipped; a dry run with no item error | 0 |
| a failed item, an item that did not run, a stop, a `compare_error` row, a failed rebuild | 100, with `ok: true` and the failures in `items` |
| nothing selected, a bad word, unanswered questions, a read-only side | 100, nothing written |
| a base that cannot be loaded | 107 |

See also [folder compare](topic:folder-compare), [folder sync](topic:folder-sync), [session](topic:session),
[archives](topic:archives) and [exit codes](topic:exit-codes).
