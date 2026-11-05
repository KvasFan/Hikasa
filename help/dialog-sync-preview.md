# Synchronize

Actions ▸ **Synchronize** ▸ **Update Right**, **Update Left**, **Update Both**, **Mirror to Right** (Shift+Ctrl+R),
**Mirror to Left** (Shift+Ctrl+L) in the [Folder Compare view](topic:view-folder-compare) plan a sync of the comparison
as it stands (nothing is read again: a folder the view did not read stays as it is, unresolved).

- **Just selection**: only the selected rows (on when something is selected).
- **Delete to recycle bin if possible**, **If the recycle bin fails, delete permanently**.
- **Confirmations**: Prompt, Yes to All, No to All (preset from `[file_operations] sync_confirmations`). Prompt asks the
  sync's question with its counts, then each item's; Yes to All and No to All answer them all (Run is then the go-ahead).
- **Include hidden items**: off, a sync leaves what a filter hides.

The other options (orphans, empty folders, archives) are the Folder Sync defaults'. Each change plans again; the dialog
lists one line per row that is not the same and each item, as `hikasa folder sync … --dry-run` prints them (conflicts
and unresolved rows highlighted), then the counts. An empty source or deletes on an unfinished comparison show the
reason and Run stays off. **Run** asks the questions and runs the sync as a [Folder Sync view](topic:view-folder-sync)
does. A flat comparison (*Ignore Folder Structure*) offers no sync.
