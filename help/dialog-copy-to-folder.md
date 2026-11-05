# Copy to Folder, Move to Folder

Actions ▸ **Copy to Folder…** and **Move to Folder…** in the [Folder Compare view](topic:view-folder-compare) copy (or
move) the selected items to a folder outside the comparison.

- **Target folder**: typed, picked from the recent folders, or chosen with **Browse…** ([Browse](topic:dialog-browse)).
  A folder that does not exist is created; an archive name is created as an archive.
- **Keep relative folder structure**: the folders the selected items share are left out (two files in `docs/a` go to
  `<target>/`). **Keep base folder structure**: the whole path below the base (`<target>/docs/a/…`). **Don't keep folder
  structure**: every file directly in the target.
- **Copy from** / **Move from**: Left, Right or Both. A side that is read-only (Disable editing) cannot be moved from.
- **If the target exists**: Fail, Skip, Replace if newer, Replace.
- **Include hidden items**: also the hidden rows below a selected folder.

Every change plans again: the dialog shows *Planning…*, then the counts and one line per item exactly as `hikasa folder
op copyto … --dry-run` prints it (failed items in the error colour, skipped ones muted), the plan's warnings and the
backups of archives. **Start** runs that plan, after the questions of [Confirm an operation](topic:dialog-confirm). The
structure and the overwrite choice are remembered. See [hikasa folder op](topic:folder-op).
