# hikasa folder sync

`hikasa folder sync <left> <right> <preset>` compares two folders, then makes one side like the other, or both up to date.
A sync is planned first, from the comparison: `--dry-run` prints each row's decision and every item, and changes nothing;
a run does exactly what the plan says.

## Presets

| Flag | Preset | Does |
|---|---|---|
| `--update-right` | Update Right | what is newer or new on the left goes right |
| `--update-left` | Update Left | what is newer or new on the right goes left |
| `--update-both` | Update Both | each side gets what is newer or new on the other |
| `--mirror-right` | Mirror to Right | the right becomes what the left is: it copies, overwrites and **deletes on the right** |
| `--mirror-left` | Mirror to Left | the left becomes what the right is |
| `--custom <MATRIX>` | Custom | your own matrix |

Exactly one is needed, unless `--session` names a folder-sync session, which holds its own. *Update* never deletes and never
overwrites a newer file. A pair that differs with no time difference is a **conflict** under *Update*: nothing says which
side wins, so it is left and marked `<!>`. A mirror overwrites; that is what it is for.

## The matrix

Each row is in one of eight situations, and the matrix says what happens to it:

| Situation | The row is… | Update Right | Update Left | Update Both | Mirror to Right | Mirror to Left |
|---|---|---|---|---|---|---|
| `newer-left` | on both sides, the left newer | copy right | leave | copy right | copy right | copy left |
| `newer-right` | on both sides, the right newer | leave | copy left | copy left | copy right | copy left |
| `added-left` | on the left only | copy right | leave | copy right | copy right | delete left |
| `added-right` | on the right only | leave | copy left | copy left | delete right | copy left |
| `deleted-left` | on the right only | leave | leave | leave | delete right | copy left |
| `deleted-right` | on the left only | leave | leave | leave | copy right | delete left |
| `different` | on both sides, no time difference | leave | leave | leave | copy right | copy left |
| `same` | on both sides, the same | leave | leave | leave | leave | leave |

An item on one side only can be new there or gone from the other: Hikasa cannot tell. `--orphans added` (the default)
reads it as added; `--orphans deleted` as deleted on the other side, so the `deleted-…` rows of the matrix apply.

`--custom` takes `<situation>=<action>` items separated by commas; the actions are `leave`, `copy-right`, `copy-left`,
`delete-left`, `delete-right`. A preset may come first as the base: `--custom mirror-right,different=leave`. Without one, the
base is the session's matrix, else `leave` everywhere. An action must name a side the item is on (`added-left=copy-left` is
refused), and a situation may be given once.

## Overrides

`--override <PATH>=<ACTION>` (repeatable) decides one path below the bases, and a folder decides what lies below it; the
longest path wins: `--override build=leave --override build/keep.txt=copy-right`. An override a row cannot take (a copy from
a side it is not on) is dropped with a `switch_ignored` warning, and so is one that names no row. A path is below the bases:
an empty path, `.` or `..` is refused.

## Folders and empty folders

A folder has no row of its own in the matrix: what happens to it is what happens to its rows. A folder on one side, all of
whose rows are deleted, is deleted too (with `--recycle`, as one item). A folder that keeps a row (a filtered one, one left
by the matrix) stays. An **empty** folder on one side reads the matrix as a file would, but is made or deleted only with
`--create-empty`.

## Options

| Flag | Does |
|---|---|
| `--create-empty` | empty folders are made and deleted too |
| `--within-archives` | archives are folders, and their members are synced (one rebuild and one backup per archive); otherwise an archive is a file |
| `--recycle`, `--recycle-fallback` | deletes go to the recycle bin; with the fallback, a delete the bin refuses becomes permanent |
| `--backup`, `--no-backup` | keep what a copy replaces, as `[backups]` names it |
| `--read-only left\|right\|both` | the rows that would write that side are left, with a `read_only_side` warning; the other side goes on |
| `--just-selection` with `--select`, `--path` | only the selection; without it, `--select` and `--path` are refused, as a sync ignores the selection |
| `--visible` | only the rows `--display` shows |
| `--include-hidden` | the side of a pair a filter hides is synced too; a row hidden on every side never is |
| `--allow-empty-source` | see below |
| `--all-rows` | the output shows every row, `same` ones and folders too |

## What protects your files

- **An empty source.** A sync that would delete on one side while the other has nothing at all is refused (exit 100): an empty
  side is usually a mount that failed. `--allow-empty-source` lets it run; `--yes` does not.
- **A comparison that did not finish deletes nothing.** A sync that deletes is refused on it.
- **What could not be compared is left.** A row that failed, and everything below a folder that failed or was not read, is
  left (`compare_error`), with a `sync_incomplete` warning and exit 100; nothing below it is an orphan.
- **Filters.** A side a filter hides is left, and so is what lies below a folder a filter hides on either side.
- **A plan remembers what it saw.** A file changed since the comparison is not overwritten or deleted.
- **The bases are a fence.** Nothing outside them is written or deleted, and nothing below a followed link; a followed link on
  one side that a mirror deletes goes as the link, never what it leads to.
- **Each row replaces only its own counterpart.** A right file `y` is not replaced by a left folder `y` that Update copies:
  that item fails, and the file stays. A name a delete of the plan frees is free.
- **Order.** Deletes first, then copies to the right, then copies to the left, as one batch.

## Questions

As [folder op](topic:folder-op): one question for the sync, then one per item for a read-only or system target, a newer file
a mirror overwrites, an item that cannot go to the bin. `--yes`, `--confirm yes-to-all|no-to-all`; without a terminal and
without `--yes`, exit 100 and nothing touched.

## Sessions and the GUI

A folder-sync session keeps the options in `[rules.sync]` (`preset`, `orphans`, `create_empty`, `within_archives`,
`recycle`, `recycle_fallback`, `[rules.sync.matrix]`, `[rules.sync.overrides]`). `hikasa session run <REF>` runs it, with
`--dry-run`, `--yes`, `--confirm`, `--allow-empty-source`, `--all-rows`; flags of `folder sync` go on top with `folder sync
--session <REF>`. `folder sync --gui` checks the words here, then asks the GUI for a Folder Sync view, which this version
cannot show yet (exit 100).

## Output

One line per row that is not `same`: `L>R`, `R>L`, `delL`, `delR`, `----` (left, with its reason) or `<!>` (a conflict), the
action and the path; then one line per item and the summary as `folder op`; then `# <n> copies right, <n> copies left, <n>
deletes left, <n> deletes right, <n> conflicts, <n> unresolved`. `--json` prints `hikasa.folder_sync.v1`.

| Outcome | Exit |
|---|---|
| every item done or skipped, every row compared | 0 |
| a failed item, an item that did not run, a stop, a `compare_error` row (also in a dry run) | 100, with `ok: true` |
| a bad word, no preset, nothing selected, unanswered questions, an empty source | 100, nothing written |
| a base that cannot be loaded | 107 |

See also [folder op](topic:folder-op), [folder compare](topic:folder-compare), [session](topic:session) and
[exit codes](topic:exit-codes).
