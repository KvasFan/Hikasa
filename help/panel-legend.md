# Legend

View ▸ **Legend** (Ctrl+Alt+L) shows a panel at the right of the [Folder Compare view](topic:view-folder-compare) that
says what the tree's colours, glyphs and centre marks mean. It is not a dialog: it stays open while you use the tree,
and the view remembers whether it is shown (`[layout] legend`). The command again, or the panel's × button, hides it.

The panel is painted from the same table the tree uses, in the colours of the current theme
(Tools ▸ Options ▸ Colors), so the two always agree.

## Colours

Each item's name takes the colour of its state: same, newer, older, orphan (on one side only), different, unknown
(not compared yet), error, filtered out (listed only while Suppress Filters is on). A folder takes the colour of what it
holds: a folder holding newer items is drawn as newer.

## Name column

- A colour spot alone: a file.
- A folder glyph: a scanned folder.
- A hollow folder: not scanned yet while the comparison runs, after a stop, or an orphan folder left unscanned.
- A filled folder in the *unopened folder* colour: not opened yet, because subfolders are not scanned automatically;
  opening it reads it.
- A red X on a folder: its listing failed on that side.
- An arrow over the glyph: a symbolic link or junction, followed or not.
- A zipper: an archive; Open reads it as a folder.

## Centre column

- An arrow points away from the newer side.
- `=`, `≠`, the orphan halves, `?`, `!`, `·`: the state of the row.
- The marks of a content comparison: binary same or different, rules-based same, unimportant differences only,
  important differences, a failed comparison.
- A small *i*: the row is marked Ignored; its difference counts as same.
- A ring: the item marked with Space, which Open and Compare To pair with the selected item.
