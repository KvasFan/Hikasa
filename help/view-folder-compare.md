# The Folder Compare view

The Folder Compare view shows two folder trees side by side, aligned by name. Each item's colour tells its
state: same, different, newer or older, orphan (on one side only), filtered, or not compared.

## Base folders

The two fields above the panes hold the left and the right base folder. Type in one: 150 ms after the last key a
list of the matching folders, archives and files below it opens; Up and Down move in it, Tab or Enter puts the item in
the field (a folder with its separator, so you can go on typing), Esc closes it. The ▾ button lists the folders you
compared recently on that side.

Enter, or the ✓ button, accepts both fields: the view compares the new folders (a named session becomes modified).
Esc with the list closed, or ✕, puts the folder back. A folder that does not exist asks whether to create it; No
keeps your text with the message under the field. A file where a folder is needed, or a path that cannot be read,
shows its message there and nothing runs. Only a folder that could be opened enters the recent list.

Session ▸ **Up One Level** ▸ Left Side, Right Side, Both Sides compares the parent folders instead (out of an archive,
the folder holding it). Actions ▸ **Set as Base Folders** compares the selected folder row (or one folder selected on
each side); **Set as Base on Other Side** makes the folder selected on one side the other side's base. Session ▸
**Swap Sides** exchanges the left and the right (with their read-only setting) and compares again; **Compare Parent
Folders** opens a new view of the folders holding the selected row; **Clear Session** (Shift+Ctrl+C) empties both
fields: type or browse two folders and accept to compare.

## Navigating

- Up and Down move the selection; Right and Left open and close a folder while the tree has the focus.
- Enter opens the selected file pair in a [Text Compare view](topic:view-text-compare).
- Ctrl+N and Ctrl+P go to the next and the previous difference.
- Ctrl+F opens [Find Filename](topic:panel-find-filename) above the tree; F3 and Shift+F3 repeat the last search.
  Alt+Down and Alt+Up step to the next and the previous row present on the focused side.
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

## Legend and Info

- View ▸ **Legend** (Ctrl+Alt+L) shows the [Legend](topic:panel-legend) beside the tree: what each colour, glyph and
  centre mark means, in the colours of the current theme. The view remembers whether it is shown.
- Session ▸ **Folder Compare Info** (Ctrl+I) shows the [statistics](topic:dialog-folder-info) of the comparison: the
  pairs per state, per side the files, folders and bytes, the content results; Copy places them on the clipboard.

## Log

View ▸ Log shows the log panel at the bottom: finished comparisons and file operations, warnings (with their codes)
and errors of every window, settings reloads and notices; *Show log* on an operation's notice opens it.
`[tweaks.log_panel]` options choose what is kept, how many entries and an optional file the entries are also written
to. Clear empties it.

The same comparison is available as [hikasa folder compare](topic:folder-compare).
