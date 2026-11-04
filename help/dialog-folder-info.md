# Folder Compare Info

Session ▸ **Folder Compare Info** (Ctrl+I) shows the statistics of the view's comparison, read when the dialog opens:

- **Pairs**: the file and the folder pairs per state (same, left newer, right newer, different, left orphan, right
  orphan, unknown, error, filtered). The base folders are not counted.
- **Sides**: per side, the files, the folders and the bytes of the regular files.
- **Content comparisons**: the results of the content tests (binary, rules-based), the files the quick tests sufficed
  for, and the rows marked Ignored.

While the comparison still runs the figures are the partial ones, and the dialog says so; open it again for the
current ones. *Ignore Unimportant Differences* is applied as the view shows it.

**Copy** places the table as text on the clipboard. Close or Esc closes the dialog; F1 shows this page.

The same figures are the `stats` of [hikasa folder compare](topic:folder-compare) `--json`.
