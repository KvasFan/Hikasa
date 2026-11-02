# Delete

Actions ▸ Delete (Del) and Delete Permanently (Shift+Del) in the [Folder Compare view](topic:view-folder-compare) delete
the selected row on the sides chosen here. The questions of the plan follow in [Confirm an operation](topic:dialog-confirm);
nothing is deleted before they are answered.

## Sides

- **Left**, **Right**, **Both**: the dialog shows what the row holds on each side. A side where the row has nothing, or
  that is read-only (Disable editing, `--ro`), is greyed.
- A row on one side has that side chosen. A row on both sides has none chosen, and OK stays off until you choose:
  deleting both sides is never a default. With `[file_operations] require_side_selection_in_delete` no side is ever
  chosen for you.
- Deleting a side leaves the other side's item: the row becomes an orphan.

## Recycle bin

- **Use recycle bin if possible** (on for Delete): files and folders go to the system's recycle bin. An item that has
  none (a member of an archive) asks before it is deleted for good.
- **If the recycle bin fails, delete permanently** (off): only then may a failed move to the bin become a permanent
  delete, of what the plan saw. Off, a failure leaves the item where it was.
- **Delete Permanently** has the bin off and greyed: the items are removed.

Esc or Cancel deletes nothing; F1 shows this page. See [hikasa folder op](topic:folder-op) `delete` for the same from the
command line.
