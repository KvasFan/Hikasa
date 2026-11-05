# Rename

Actions ▸ **Rename** (F2) in the [Folder Compare view](topic:view-folder-compare):

- **One row**: its name becomes editable in place. Enter renames it on the selected sides, Esc keeps the name. A name
  that is taken makes the item fail: nothing is overwritten.
- **More rows**: this dialog. **DOS compatible** takes a mask, prefilled with one that fits every selected name (`*` for
  what differs, `?` for one character); **Regular expressions** take *Old* (matched against each whole name) and *New*
  (`$1` … for the groups). **Rename on**: Left, Right or Both (read-only sides are off).

The preview lists every rename as `a.txt -> a.bak`, as `hikasa folder op rename … --dry-run` prints it; **Start** runs
it after the questions of [Confirm an operation](topic:dialog-confirm). The mode and the masks are remembered. See
[hikasa folder op](topic:folder-op).
