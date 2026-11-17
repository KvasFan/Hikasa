# The Text Compare view

The Text Compare view shows two text files side by side, aligned line by line. Changed lines are highlighted,
and inside them the changed characters; unimportant differences use a lighter colour. The thumbnail on the
left shows where the differences are.

Both panes are editors: click in a pane and type. The comparison follows every key at once and is redone exactly a
moment after you stop typing (the status bar shows *Recomparing…* when that takes longer than a quarter second).

## Editing

- The caret moves with the arrows, Ctrl+Left and Ctrl+Right (words), Home (first non-blank, then the line's start),
  End, Ctrl+Home, Ctrl+End, Page Up and Page Down; Shift with any of them selects. Ctrl+Up and Ctrl+Down scroll a
  line. Esc collapses the selection; Ins switches **Overwrite Mode** (`OVR` in the status bar).
- With the mouse: a click places the caret, a drag selects, a double click selects a word, a triple click the line
  with its line ending; a click or a drag in the line numbers selects lines. On Linux the selection is also the
  primary selection, and a middle click pastes it.
- Edit ▸ **Undo** (Ctrl+Z) and **Redo** (Ctrl+Y) act on the pane with the caret; typing within a second is one step
  of at most a word. **Cut**, **Copy** and **Paste** use the clipboard; without a selection Copy and Cut take the
  caret's line. Pasted line breaks become the pane's line ending.
- Edit ▸ **Delete Line** (Shift+Ctrl+K), **Delete Word**, **Delete to Start/End of Word** (Ctrl+BkSp, Ctrl+Del),
  **Delete to Start/End of Line**, **Insert Line Before/After** (Shift+Ctrl+Enter, Ctrl+Enter), **Increase/Decrease
  Indent** (Shift+Ctrl+I, Shift+Ctrl+U) and **Select Section** (Ctrl+D).
- Edit ▸ **Copy to Right** (Ctrl+R) and **Copy to Left** (Ctrl+L) copy the selected lines, or the current section,
  to the other pane as one step (Undo in that pane takes it back); **Copy Line to Right/Left** copy the current line.
  The strip between the panes shows arrows at each difference section on screen: a click copies that section.
- A pane that cannot be edited says why in its header (*Read-only*, *Editing disabled*) and in the message line when
  you type. A file with binary data asks once whether to edit it anyway.
- View ▸ **Word Wrap** breaks long lines at the panes' width.

## Saving

File ▸ **Save File** (Ctrl+S) saves the pane with the caret; **Save All Files** saves every modified pane;
**Save File As…** (F12) and **Save File on File System As…** save under a new name. The format's *when saving*
options are applied first, as one step you can undo. Questions about the save (a file changed on disk, a read-only
file, characters the encoding cannot write) are asked one at a time: see [Save questions](topic:dialog-save-questions).
While a save runs, keys typed in its pane wait and are applied after it.

Closing a view, a window or the program with unsaved edits asks first: see [Unsaved edits](topic:dialog-unsaved-edits).

## File Info

View ▸ **File Info** shows a strip above each pane: the file (type another path and press Enter to open it in that
pane), the encoding (*Reopen with Encoding*, *Save with Encoding*, *Byte Order Mark*), the line endings (*To
Windows/DOS*, *To Unix*, *To Macintosh*), the format (pick another: the comparison is redone and your edits stay) and
badges: *Read-only*, *Editing disabled*, *Converted*, *Modified*. The status bar shows the caret's line and column
(the display column in parentheses when tabs make it differ), the row's state, the element under the caret, the
encoding, the line ending and the times.

## Navigating

- Ctrl+N and Ctrl+P go to the next and the previous difference section.
- F5 reloads the files and compares them again.
- Ctrl+Num + and Ctrl+Num - change the font size; Ctrl+Num 0 restores it.
- Session ▸ **Swap Sides** exchanges the left and the right file and compares again; **Compare Parent Folders** opens
  a [Folder Compare view](topic:view-folder-compare) of the folders holding the two files.

## Display filters

- View ▸ Show All, Show Differences, Show Same, Show Context and Show None choose which lines are listed; the
  toggles on the toolbar combine same, orphan and different lines.
- Ignore Unimportant Differences treats unimportant differences as same; Thumbnail shows or hides the
  thumbnail.

The same comparison is available as [hikasa text compare](topic:text-compare); the file format decides what is
unimportant ([hikasa format](topic:format)).
