# File Formats

Tools ▸ File Formats… lists every file format in detection order: the first enabled format whose mask matches a
file's name reads that file. Each change is written at once, as the matching `hikasa format` command writes it
([hikasa format](topic:format)); there is no OK or Cancel.

## The list

- **On**: the format takes part in detection (`format enable`, `format disable`).
- **Name**, **Kind**, **Mask**, and the **Origin**: *built-in*, *user* (a file of yours), or *override* (your file
  replacing a built-in of the same id).
- A format shown in the accent colour is never detected: enabled formats above it cover its whole mask. Its tip
  names them. Disabled formats are muted.
- *Everything Else* is last and fixed: it reads every file no format matches.

## The buttons

- **New ▸** Text, Table, Hex, Picture, Media, Version or Registry: asks for an id and a name, creates the format
  (`format new`), then opens the [format editor](topic:dialog-format-editor).
- **Clone**: the same for a copy of the selected format.
- **Edit…** (or a double click): the [format editor](topic:dialog-format-editor). Editing a built-in writes your
  override file; the built-in itself never changes.
- **Delete**: a user format only, after a confirmation (`format delete`). A built-in is disabled or restored instead.
- **Move Up**, **Move Down**, **Move to Top**, **Move to Bottom**: the detection order (`format move`).
- **Import…**: a format file chosen in the [Browse](topic:dialog-browse) dialog (`format import`); a format of that
  id already there is replaced only after you say so.
- **Export…**: the selected format as a TOML file (`format export`).
- **Restore Built-in** (a built-in with your override) and **Restore All…** (every user format and the order): the
  files to be removed are listed first, and nothing is removed until you confirm (`format restore`).

A message under the list is the command's own when an action is refused. The list follows changes made elsewhere
(another window, the command line, an editor): it is read again when the formats folder changes. A Text Compare or
Text Edit view that uses a changed format offers *Reopen*.

See [Contents](topic:index).
