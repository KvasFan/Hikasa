# Find Filename

Search ▸ **Find Filename** (Ctrl+F) opens a one-line panel above the tree of the
[Folder Compare view](topic:view-folder-compare). It is not a dialog: the tree stays usable while it is open, and the
view remembers whether it is shown (`[layout] find_panel`).

Type a name or part of one and press Enter (or **Next**) to select the next row whose name matches; **Previous** goes
the other way. The search looks at every row the display filter shows, also inside closed folders: a match inside one
opens the folders above it, and the row is selected on the side whose name matched (both sides when both match) and
scrolled into view.

## Options

- **Match case**: `Report` does not find `report`. Off, names are compared case-insensitively.
- **Whole word**: the text must not be part of a longer word: letters, digits and `_` count as word characters.
- **Regular expression**: the text is a regular expression (case-insensitive unless Match case is on). One that is not
  valid is shown in red in the panel and nothing is searched.
- **Wrap around**: after the last row the search goes on from the first one.
- **Search up**: Next searches towards the top; Previous towards the bottom.
- **Left**, **Right**, **Both sides**: which side's names are looked at.

When no row matches, the message panel says *Filename not found.* (its *Do not show again* turns
`[next_difference] show_message_panel` off).

## Keys

- **F3** (Search ▸ Find Next Filename) and **Shift+F3** (Find Previous Filename) repeat the last search, also with the
  panel closed; with no text yet they open the panel.
- **Esc** in the text field, or the × button, closes the panel.
- **Alt+Down** and **Alt+Up** (Next and Previous Item on This Side) step to the next row present on the focused side.

The texts you searched for are kept, newest first, in the panel's drop-down (`[mru] find`); the options in
`[dialogs.find-filename]` of `state.toml`.
