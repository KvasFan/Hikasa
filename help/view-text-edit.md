# The Text Edit view

The Text Edit view shows one text file in one pane: the editor of the [Text Compare view](topic:view-text-compare)
without a comparison. Open it with Tools ▸ **Edit Text File**, Home ▸ New ▸ **Text Edit**, **Open With ▸ Text Edit**
in a folder view's pane or a text view's header, a `text-edit` session, `hikasa gui open --edit <file>…` (a view per
file), or a hit of [Find in Files](topic:panel-find-in-files).

- Everything the editor does in a Text Compare pane works here: the keys, the mouse, Undo and Redo, the clipboard,
  [Save and Save As](topic:dialog-save-questions), the [File Info header](topic:view-text-compare), Word Wrap and the
  display options of one pane. The text has its format's syntax colours (View ▸ **Syntax Highlighting**); the
  header's menu ▸ **Format** reopens the file with another format.
- Search ▸ [Find and Replace](topic:panel-find-replace), [Find in Files](topic:dialog-find-in-files),
  [Go To](topic:dialog-go-to), the bookmarks (Shift+Ctrl+0…9 toggles, Ctrl+0…9 goes) and **Next/Previous Edit**.
- Edit ▸ **Convert File** and **Line Endings** convert the selected lines, or the whole file, as one step.
- View ▸ **Text Details**, **Hex Details** and **Ruler** (see [the details panel](topic:panel-details)).
- Session ▸ **Text Edit Info** (Ctrl+I) shows the file's lines, encoding, line endings, format and load time;
  **Clear Session** empties the view and forgets the file's encoding, format and save target (type a path in the
  header and press Enter to open another file).
- The commands that need two panes (copies to the other side, layouts, alignment overrides, the display filter) are
  not in this view.

Closing the view with unsaved edits asks first ([Unsaved edits](topic:dialog-unsaved-edits)). The view's session type
is `text-edit`: Session ▸ Save Session keeps the file, its format and encoding.
