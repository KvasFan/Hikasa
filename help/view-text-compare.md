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

- Ctrl+N and Ctrl+P go to the next and the previous difference section; Shift+Ctrl+N and Shift+Ctrl+P to the next and
  the previous difference inside the lines; Search ▸ **Next/Previous Edit** to your edits. The toolbar's arrows step by
  what View ▸ **Browse By** chooses (difference sections, differences, edits or find hits). When there is nothing
  more, the message line says so (or the window beeps, with `[next_difference] show_message_panel` off). The view
  opens at the first difference (`go_to_first_difference_on_load`).
- Alt+Down and Alt+Up go to the next and the previous line of the pane with the caret, skipping its gaps.
- [Go To](topic:dialog-go-to) (Ctrl+G) moves to a line and column. The bookmarks: Shift+Ctrl+0…9 sets or clears one
  at the caret's line (its digit shows in the line numbers), Ctrl+0…9 goes to it (in the other pane when this one has
  none), Search ▸ **Clear Bookmarks**.
- Alt+Left and Alt+Right (F6, Shift+F6) give the keyboard to the left or the right pane, on the same row.
- [Find and Replace](topic:panel-find-replace) (Ctrl+F, Ctrl+H) and [Find in Files](topic:dialog-find-in-files)
  (Shift+Ctrl+F).
- In a view opened from a folder comparison, Ctrl+M and Shift+Ctrl+M open the next and the previous pair of
  difference files in this view; Ctrl+Alt+R and Ctrl+Alt+L [copy the file and open the next
  pair](topic:dialog-copy-file-next).
- F5 reloads the files and compares them again.
- Ctrl+Num + and Ctrl+Num - change the font size; Ctrl+Num 0 restores it; View ▸ [Display
  Font…](topic:dialog-display-font) picks the family and the size.
- Session ▸ **Swap Sides** exchanges the left and the right file and compares again; **Compare Parent Folders** opens
  a [Folder Compare view](topic:view-folder-compare) of the folders holding the two files.

## Layouts and display

- View ▸ **Side-by-Side Layout** or **Over-Under Layout** (the left pane above the right one; the copy arrows are then
  in each pane's line numbers). Drag the strip between the panes to move it; **Widen Left/Right Pane**
  (Ctrl+Alt+Right/Left) move it 5 %. **Zoom Pane** (F11) shows the pane with the keyboard alone.
- View ▸ **Syntax Highlighting**, **Visible Whitespace** (`·` a space, `→` a tab, `°` another blank, `¶` and `¤` the
  line endings), **Ruler** and the [details panel](topic:panel-details) (**Text Details**, **Hex Details**,
  **Alignment Details**).
- Options ▸ Tweaks: extra line spacing, a column guide, dimming the pane without the keyboard, hatching below the
  shorter file, the orphan colour, one scroll bar, the right-side line numbers of the left pane. The layout and the
  panels are remembered for the next view.

## Marks, overrides and conversions

- View ▸ **Ignored** marks the selected rows (or the current section) as ignored: they count as same and are drawn in
  the ignored colour; again removes the mark. The session keeps them with a check of their text.
- Edit ▸ **Align With…** (F7): select lines in one pane, then click a line of the other pane (or select lines there
  and press Enter) to align them; Esc cancels; on lines already aligned it removes the pair. **Isolate** keeps the
  selected lines out of the alignment; **Clear Alignment Overrides** removes them all. The strip marks the rows.
- Edit ▸ **Convert File** (trim trailing whitespace, leading spaces to tabs, tabs to spaces) and **Line Endings** (to
  Windows/DOS, Unix, Macintosh) convert the selected lines, or the file, as one step (*Converted N lines*).
- Edit ▸ **Compare Selection to Clipboard** compares the selection (or the whole file) with the clipboard's text in a
  new view; File ▸ **Open Clipboard** (Shift+Ctrl+V) shows the clipboard's text in the pane with the caret; File ▸
  **Open File…** (Ctrl+O) opens another file in it.

## Context menus

A right-click in a pane opens its menu (outside the selection the caret moves there first); Shift+F10 opens it at
the caret. The strip and the line numbers have the copy and mark commands; a header has Save, Open With, the file
manager, the encoding, the line endings and the format.

## Changes on disk

A file changed on disk is read again by itself when you have not edited it. With edits, a notice offers **Reload**
(asks about your edits first) and **Keep mine** (the next save asks whether to overwrite). A changed file format
offers **Reopen**. Session ▸ **Text Compare Info** shows the [figures of the view](topic:dialog-text-info);
**Clear Session** empties it: the next files open with the session defaults (no encoding, format or alignment
override of the cleared session). **Reload** keeps the Ignored lines and alignment overrides of now; one beyond the end
of a file that got shorter is dropped with a notice.

## Display filters

- View ▸ Show All, Show Differences, Show Same, Show Context and Show None choose which lines are listed; the
  toggles on the toolbar combine same, orphan and different lines.
- Ignore Unimportant Differences treats unimportant differences as same; Thumbnail shows or hides the
  thumbnail.

The same comparison is available as [hikasa text compare](topic:text-compare); the file format decides what is
unimportant ([hikasa format](topic:format)).

## Reports and other views

Session ▸ **Text Compare Report…** opens the [report dialog](topic:dialog-report) over the view's comparison: the layouts of
`hikasa text compare --report`, the view's display, View in Browser and Print Preview. Session ▸ **Compare Files Using** ▸ opens
the same two files in a Table Compare, Hex Compare or Picture Compare view, in a new tab beside this one; a view opened from a folder comparison keeps its
link to it.
Edits not saved are not in the new view, which compares the files (a message line says so). When a file holds binary
data its header offers **Compare in Hex**, the same as Compare Files Using ▸ Hex Compare.
