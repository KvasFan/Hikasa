# The Home view

The Home view is the start of every new tab: the type buttons, a search box, the tree of saved sessions and the
session management panel.

## Starting a comparison

Type or paste two paths in the panel and press Enter or Compare: two folders open a
[Folder Compare view](topic:view-folder-compare), two files a [Text Compare view](topic:view-text-compare), and
the comparison replaces the Home view in its tab. A path that does not exist, or a folder paired with a file, is
reported in the panel. `clipboard://` compares the clipboard's content; `-` works only from the command line.

Click a type button for an empty session of that type. Drop one path on a type button to fill the left side, or
two paths to open them as that type.

## The sessions tree

- *New*: one entry per session type; Session Settings on it edits the defaults of the type.
- Your named sessions in their folders: folders first, then favorites, then the rest, each sorted by name.
- *Auto-saved*: the views you did not save, grouped by day, newest first.

Ctrl+F moves to the search box: it matches names, folder paths and descriptions, and shows the matches with their
folders. Clearing it brings the tree back as it was.

Right-click a row for its commands: Open (Enter, double-click), Open as New (a copy without a name), Open All (every
session directly in a folder), New Session, New Folder, Clear, Delete (Del), Save As, Rename (F2), Session Settings,
Expand All, Collapse All and Favorite. Drag a session onto a folder to move it. A locked session is protected: Clear
and Favorite are off, and Delete, Rename and moving ask first. Deleting or renaming a folder that holds locked
sessions names them first. Open views of a renamed, moved or deleted session follow: a deleted one becomes unnamed.

## The management panel

The panel shows the selected row. For a named session: description, inputs, Locked, Save, Save As…, Factory Defaults
(takes effect when you press Save), Cancel and Open. For an auto-saved session the inputs are read-only: Open, Open
as New and Delete. View ▸ Show Session Management hides the panel; View ▸ Maximize Session Management hides the
tree.

See [Session Settings](topic:dialog-session-settings), [Save Session As](topic:dialog-save-session),
[Workspaces](topic:dialog-workspaces), [hikasa session](topic:session) and [The main window](topic:main-window).
