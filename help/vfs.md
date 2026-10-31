# hikasa vfs

`hikasa vfs` reads and changes any spec the comparisons read, through the same virtual file system.

## Reading

| Command | Result |
|---|---|
| `hikasa vfs ls <spec>` | the items of a folder: kind, size, time, name |
| `hikasa vfs stat <spec>` | the metadata of one item and what its back-end can do |
| `hikasa vfs cat <spec>` | the bytes of a file on stdout (`--json`: base64) |

`ls` and `stat` report symbolic links themselves; `--follow-symlinks` reports their targets.

## Changing

| Command | Result |
|---|---|
| `hikasa vfs cp <src>… <dst>` | copies files; `-r` copies folders with their contents |
| `hikasa vfs mv <src>… <dst>` | moves or renames files and folders |
| `hikasa vfs rm <spec>…` | removes files, links and empty folders; `-r` whole folders; `--trash` to the recycle bin |
| `hikasa vfs mkdir <spec>…` | creates folders; `-p` also their parents (an existing folder is fine) |
| `hikasa vfs touch <spec>…` | sets the modification time: now, `--time <RFC 3339>` or `--reference <spec>`; `--created <RFC 3339>` also the creation time (Windows, macOS); `--create` creates missing files |

- **Targets.** When `<dst>` is an existing folder, each source goes into it under its own name; otherwise one
  source is copied or moved to the new name `<dst>`. Several sources need an existing folder. A `<dst>` that
  ends with `/` must be a folder: `-p` creates it. `mv` to the same name spelt differently (`File.txt` →
  `file.txt` on a case-insensitive disk) renames it. `-` and `clipboard://` cannot be written (`unsupported`).
- **Plans.** Every command first lists everything it would do (whole folder trees included), then does it.
  `--dry-run` prints that plan and changes nothing. While running, an item whose target appeared after the
  plan was made fails ("changed since the plan was made"): nothing is replaced that the plan did not show;
  `--overwrite newer` is decided again, so a target that became newer meanwhile is skipped. When a folder
  cannot be created, or a folder was replaced (for example by a link) after the plan was made, nothing inside
  it is touched: those items are `not run`.
- **Confirmation.** `rm` needs `--yes` (or `--dry-run`), also with `--trash`; without it nothing is touched
  (exit 100). The other commands replace nothing unless `--overwrite` says so. A name ending in `.` or `..`
  (`.`, `..`, `dir/..`) and a file-system root are never removed, moved or trashed ("needs a file name",
  "is a root folder").
- **Existing targets** (`cp`, `mv`): `--overwrite fail` (default: the item fails), `skip`, `always`, or `newer`
  (only when the source is strictly newer). A folder is never removed to make room for a file; folders merge.
  When a replacement fails, the item it would have replaced is left as it was.
  FIFOs, sockets and devices are skipped, never opened.
- **Metadata.** `cp` copies the modification time and the permissions (Windows: the read-only attribute)
  after the content; `--no-preserve` copies neither, `--preserve-created` also the creation time. Owner and
  group are never copied. A target that cannot keep a time or attribute gives one `capability_missing` warning.
- **Links.** Symbolic links are copied as links (`--symlinks skip` skips them), never followed inside a tree;
  `--follow-symlinks` follows the links named on the command line only. `rm`, `mv` and `touch` act on a link
  itself, never on what it points to; copying onto a link replaces the link.
- **Moves** rename where they can (one step, also for whole folders); across file systems or back-ends they
  copy and then remove each item, and keep a source folder whose items could not all be moved. When removing
  a source fails, its copy is removed again, unless the source is already gone: then the copy is kept.
- **Recycle bin.** `rm --trash` moves each named item to the recycle bin as a whole; when that fails the item
  stays where it is: it is never deleted permanently instead. A link is trashed as a link; on Linux a link on
  another file system than the recycle bin is refused (`unsupported`), since it could only be trashed as a copy
  of what it points to.
- **Results.** One line per item, then a summary such as `copied 4 of 4 items (0 skipped, 0 failed)`. With
  `--json` the document is `hikasa.vfs_op.v1`; failed items keep `ok: true` and the exit code is 100.

## Specs

A spec is a local path, `-` for standard input or `clipboard://` for the clipboard (text, or an image as
`clipboard.png`): `vfs cat clipboard://`, `vfs stat clipboard://` and `vfs cp clipboard:// <file>` (it needs a
file name). An empty clipboard is `not_found`; clipboard text (or `HIKASA_CLIPBOARD_FILE`) above 1 GiB, or an
image above 1 GiB of pixels, is `too_large`. Archive members, snapshots and remote specs parse but are not
available yet. See
[Using the command line](topic:cli).
