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
shows its message there and nothing runs. Only a folder that could be opened enters the recent list. `-` (standard
input) and `clipboard://` cannot be typed here: give them on the command line. A name that is not valid Unicode is
listed, marked, and cannot be taken into the field.

Session ▸ **Up One Level** ▸ Left Side, Right Side, Both Sides compares the parent folders instead (out of an archive,
the folder holding it). Actions ▸ **Set as Base Folders** compares the selected folder row (or one folder selected on
each side); **Set as Base on Other Side** makes the folder selected on one side the other side's base. Session ▸
**Swap Sides** exchanges the left and the right (with their read-only setting) and compares again; **Compare Parent
Folders** opens a new view of the folders holding the selected row; **Clear Session** (Shift+Ctrl+C) empties both
fields: type or browse two folders and accept to compare. While an operation of the view runs, these commands are off
and an accepted field says *Wait for the current operation to end.*

When the shown columns are wider than a side, the sides scroll sideways together: the bar under the rows, or the
horizontal wheel (Shift and the wheel). Name keeps at least 80 points.

## Navigating

- Up and Down move the selection; Right and Left open and close a folder while the tree has the focus.
- Enter opens the selected file pair in a [Text Compare view](topic:view-text-compare).
- Ctrl+N and Ctrl+P go to the next and the previous difference.
- Ctrl+F opens [Find Filename](topic:panel-find-filename) above the tree; F3 and Shift+F3 repeat the last search.
  Alt+Down and Alt+Up step to the next and the previous row present on the focused side.
- Edit ▸ Expand All and Collapse All; Actions ▸ Open Subfolders and Close Subfolders.
- Ctrl+F5 compares everything again; Stop ends a running comparison.

## File operations

Select one row or more, then, from the Actions menu:

- **Copy to Right** (Ctrl+R), **Copy to Left** (Ctrl+L), **Move to Right**, **Move to Left**, **Copy to Side…** and
  **Move to Side…** (which ask the side), **Copy to Other Side** and **Move to Other Side** (from a pane's context
  menu), **Exchange** (both sides at once): the selected items go to the other side, a folder with what it shows; a file
  there is replaced.
- **Copy to Folder…**, **Move to Folder…**: to a folder outside the comparison ([dialog](topic:dialog-copy-to-folder)).
- **Delete** (Del) and **Delete Permanently** (Shift+Del) ask the sides and the recycle bin in the
  [Delete dialog](topic:dialog-delete).
- **Rename** (F2): one row in place, more rows with a mask ([dialog](topic:dialog-rename)). **Touch…**
  ([dialog](topic:dialog-touch)), **New Folder…** (Ins, [dialog](topic:dialog-new-folder)).
- **Ignored** marks the selected rows Ignored (their differences do not count) and keeps the marks in the session;
  **Exclude** hides them by name ([dialog](topic:dialog-exclude)).
- **Compare Contents** ([dialog](topic:dialog-compare-contents)), **Quick Compare** ([dialog](topic:dialog-quick-compare)),
  **Compare To…** (F7: two selected items, one and the item marked with Space, or one and an item you browse to),
  **Open With** ▸ (a Text Compare view, a Table Compare, [Hex Compare](topic:view-hex-compare) or [Picture
  Compare view](topic:view-picture-compare) of two files, a
  Folder Compare view of two folders, an application of the Open With page of
  [Options](topic:dialog-options), the associated application). More than ten items ask first; the first item that cannot
  be opened stops the rest.
- **Synchronize** ▸ Update Right, Update Left, Update Both, Mirror to Right, Mirror to Left: the
  [sync preview](topic:dialog-sync-preview). Session ▸ **Sync Base Folders** opens a [Folder Sync view](topic:view-folder-sync).

A verb with options opens its dialog, which plans on every change and shows the plan line by line as `hikasa folder op
… --dry-run` prints it; a verb without options shows the same list in its questions. *Include hidden items* is in each.

Each operation is planned first, then its questions are asked in [Confirm an operation](topic:dialog-confirm); Stop
ends one that runs (what ran stays). The log gets its summary and its failures, a notice sums it up, and the rows it
wrote are read again, keeping the open folders and the selected row (a row you select or a folder you close meanwhile
stays so). A side that is read-only (Disable editing: `--ro`, `--ro1`, `--ro2`, or the session's `read_only_left`,
`read_only_right`) is never written: the commands that would are off. It stays read-only through Session Settings,
Save, Save As, Duplicate Tab and workspaces, and a session saved from the view keeps `read_only_left`,
`read_only_right`. `--nobackups` turns backups off for the view's operations; Duplicate Tab and a workspace keep it, a
session does not. An operation, a sync or a content compare of 10 seconds or more beeps at its end with
`beep_after_long_operations`. [hikasa folder op](topic:folder-op) has every operation from the command line.

## Context menus

A right-click on a row opens the menu of that pane (or of the centre column); a row outside the selection becomes the
selection first, on the side you clicked. The menu lists the commands that work on the selection: Open, Open With ▸,
Compare To, Quick Compare; Copy and Move to Other Side (from that side), Copy and Move to Folder, Exchange; Delete,
Delete Permanently, Rename, Touch, New Folder; Compare Contents, Ignored, Exclude, Synchronize ▸; Set as Base Folders,
Set as Base on Other Side, Compare Parent Folders; Copy, Paste, Copy Filename, File Manager ▸; Open and Close Folder and
Subfolders. Unavailable entries are greyed out, among them those that would write a side with *Disable editing* on.
Shift+F10 or the Menu key opens the menu of the focused pane for the current row. A right-click on the column header
lists the columns and *Reset Columns* (the default columns, widths and order).

**File Manager** ▸ *Open Folder in File Manager* opens the folder of the current row (a folder row itself) with your
file manager; *Show in File Manager* shows the item selected in its folder (Explorer, Finder, or your desktop's file
manager; the folder when it cannot select). An item inside an archive shows the archive file.

## Drag and drop, the clipboard

- Drop files or folders from your file manager on a base field or a pane's header: that side compares the first one
  (two on the left field: both sides).
- Drop them on a pane's tree: they go into the folder row under the pointer (or the side's base folder). A prompt asks
  *Copy* or *Move*, with what to do when an item exists there (*Fail*, *Skip*, *Replace if newer*, *Replace*) and the
  plan of either, line by line as `hikasa vfs cp … --dry-run` prints it; the plan you see is the plan that runs, then
  the folder is read again. On a file row they go into its folder; hold Shift as you drop to see the move first. Items
  from a side with *Disable editing* on are only copied (*Move* is off and says why).
- While another program drags, the window often cannot tell where the pointer is. When it cannot, the prompt asks
  which pane's current folder the items go into (the focused pane first) and, for one or two items, offers *Open as
  new comparison* instead.
- Drag selected rows to the other pane: they are copied there (hold Shift when you let go: moved), through the
  operation's preview; onto a folder row of the other pane: Copy to Folder (Move with Shift) into that folder.
- Dropping from other programs works on X11, Windows and macOS, not on Wayland; rows cannot be dragged out of the
  window. Use the clipboard instead: Edit ▸ **Copy** (Ctrl+C) puts the selected items as files your file manager
  pastes (items inside archives are left out); **Paste** (Ctrl+V) brings a file manager's copied files (or text lines
  that are absolute paths or `file://` addresses of this computer) into the pane's current folder through the same
  prompt; **Select All** (Ctrl+A) selects every row shown. Actions ▸ **Copy Filename** copies the selected paths as
  text, one a line.
- On Linux the clipboard's file lists name each item by its real path (a link's target), leave out items that no longer
  exist, and cannot carry a name that is not valid Unicode from another program.
- In a text field (a base field, the Filters edit, Find Filename, a dialog) Ctrl+C, Ctrl+X, Ctrl+V and Ctrl+A edit the
  text.

## Rules and the Filters edit

Session ▸ **Rules…** opens [Session Settings](topic:dialog-session-settings) on its Comparison tab, **Name Filters…** on
its Name Filters tab: what is compared, the name filters, how names align and folders are read, the file formats and the
Ignored items of this session; OK compares again.

The **Filters** field in the toolbar (Session ▸ Filters focuses it) shows the session's name filter: `*.c;*.h` keeps
those files, `-*.bak` hides files, `obj\` and `-obj\` keep and hide folders. Enter compares with what you typed (a
mask that does not parse is explained under the field and nothing runs); Esc puts it back. Its ▾ button lists the
presets and your recent filters, and *Add To Presets…*, which saves the filter under a name of your choice (the
presets are on the Name Filters page of [Options](topic:dialog-options)).

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

## Reports

Session ▸ **Folder Compare Report…** opens the [report dialog](topic:dialog-report) over the tree as it is now (after
copies, deletes, refreshes and excludes): side by side with its columns and links to file reports, summary, list, XML
or JSON, the view's display, *Just selection* of the selected rows.
