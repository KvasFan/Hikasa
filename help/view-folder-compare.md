# The Folder Compare view

The Folder Compare view shows two folder trees side by side, aligned by name. Each item's colour tells its
state: same, different, newer or older, orphan (on one side only), filtered, or not compared.

## Navigating

- Up and Down move the selection; Right and Left open and close a folder while the tree has the focus.
- Enter opens the selected file pair in a [Text Compare view](topic:view-text-compare).
- Ctrl+N and Ctrl+P go to the next and the previous difference.
- Edit ▸ Expand All and Collapse All; Actions ▸ Open Subfolders and Close Subfolders.
- Ctrl+F5 compares everything again; Stop ends a running comparison.

## File operations

Select one row, then:

- Actions ▸ **Copy to Right** (Ctrl+R) and **Copy to Left** (Ctrl+L) copy it to the other side, a folder with what it
  shows; a file there is replaced.
- Actions ▸ **Delete** (Del) and **Delete Permanently** (Shift+Del) ask the sides and the recycle bin in the
  [Delete dialog](topic:dialog-delete).

Each operation is planned first, then its questions are asked in [Confirm an operation](topic:dialog-confirm); Stop
ends one that runs (what ran stays). The log gets its summary and its failures, a notice sums it up, and the rows it
wrote are read again, keeping the open folders and the selected row (a row you select or a folder you close meanwhile
stays so). A side that is read-only (Disable editing: `--ro`, `--ro1`, `--ro2`, or the session's `read_only_left`,
`read_only_right`) is never written: the commands that would are off. It stays read-only through Session Settings,
Save, Save As, Duplicate Tab and workspaces, and a session saved from the view keeps `read_only_left`,
`read_only_right`. `--nobackups` turns backups off for the view's operations; Duplicate Tab and a workspace keep it, a
session does not. The other operations of the Actions menu, and selecting
more than one row, come later; [hikasa folder op](topic:folder-op) has them all from the command line.

## Folders and archives

- A folder that was not compared yet (*Automatically scan subfolders* off) is read when you open it (after the
  operation, if one runs).
- `[rules.handling] expand_on_load` opens every folder after a comparison, `expand_differences_only` those with a
  difference below them.
- Open on a closed archive opens it in place, as a folder; once it opened the session remembers it (`open_archives`).
  One that cannot be read gives a warning and is not remembered.
- A Text Compare opened from a row gives its result back to the row once its comparison ends, unless the files changed
  meanwhile.

## Display filters

- View ▸ Display Filter: Show All, Show Differences, Show Same, Show No Orphans, Show Orphans, the newer filters
  and Show None choose which items are listed; the toggles combine categories. The toolbar shows them as a
  drop-down with toggles, as favourites or as toggles (right-click the toolbar to choose).
- The folder modes: Always Show Folders, Compare Files and Folder Structure, Only Compare Files.
- Ignore Unimportant Differences treats unimportant differences as same; Suppress Filters temporarily shows
  filtered items.

## Log

View ▸ Log shows the log panel at the bottom: finished comparisons and file operations, warnings (with their codes)
and errors of every window, settings reloads and notices; *Show log* on an operation's notice opens it.
`[tweaks.log_panel]` options choose what is kept, how many entries and an optional file the entries are also written
to. Clear empties it.

The same comparison is available as [hikasa folder compare](topic:folder-compare).
