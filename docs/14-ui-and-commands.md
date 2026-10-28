# User Interface: Windows, Menus, Toolbars, Commands, Theming

## 1. Window model

- **UI-001** (P0): Main window = menu bar, tab bar, the active view (its own toolbar,
  path/File Info bar, content, optional panels, status bar). The Home view is the
  default content of a new tab/window. Views remember layout per session type (panel
  visibility, splitter positions when "sticky", column widths, font size).
- **UI-002** (P0): Menus change with the active view type. The **common** menus are:
  - **Session**: New Session ▸ (one entry per type), New Tab (Ctrl+T), New Window,
    Open Session (Shift+Ctrl+O), Load Workspace ▸ (…, Manage Workspaces…), Save
    Workspace As…, [view-specific session commands], Close Tab (Ctrl+W), Exit (Ctrl+Q).
  - **Tools**: Options…, File Formats…, Profiles…, Export Settings…, Import Settings…,
    Restore Factory Defaults…, Save Snapshot…, Edit Text File, View Patch….
  - **Help**: Contents, Context Sensitive Help (F1), Project Website, Check for Updates,
    Support, About.
  - View-type menus **File**, **Edit**, **Actions**, **Search**, **View** as listed in
    `reference/commands.md`.
- **UI-003** (P0): Every command has a stable id, caption, hint ("short|long"),
  category, default shortcut(s), menu/toolbar visibility defaults, and an availability
  predicate. Users can customize shortcuts and menu/toolbar presence per view type
  (Options ▸ Toolbars, Shortcuts, Menus: view selector, search box, command list with
  Menu/Toolbar checkboxes, Shortcut editor with Remove/Default and conflict warning,
  Display filter mode, "Large buttons with text labels", "Show All in Menu",
  "Show All on Toolbar").
  *M2 W1:* the customizations are stored in `commands.toml` (`15-options-reference.md`,
  OPT-011); a shortcut is written in the spelling of `reference/commands.md`
  (`Shift+Ctrl+Alt+Cmd+Key`, a trailing `(context)` kept as text). The effective shortcuts of
  a view are the catalogue defaults of its sections, replaced per command by an override.
  Two commands of one view with the same shortcut (without a context) are a conflict; the
  catalogue itself binds the Copy and Take commands of the file views to the same keys (the
  window resolves them by focus), so `hikasa settings list` warns only about conflicts that
  involve a command whose shortcuts the user changed (*interpretation*); the W4 editor shows
  all of them.
- **UI-004** (P0): Toolbar right-click menu: display-filter mode (DropDown /
  Favorites / Toggles), large buttons with labels, Toolbar Options…, Hide Toolbar.
- **UI-005** (P0): Context menus in panes mirror the relevant Actions/Edit commands;
  the folder tree has side-specific entries; the text gutter has section commands.
- **UI-006** (P0): Keyboard: all commands reachable; arrow navigation in trees/grids;
  Esc closes dialogs and (option) file views; F1 context help opens the manual page of
  the focused element.
  *M2 W0 (D-67):* F1 (`ContextHelp`) shows the help topic of the active view type in a
  resizable panel on the right of the window; Help ▸ Contents (`HelpContents`) shows the
  contents; links open other topics; Esc or the panel's close button hides it. The topics
  are the Markdown files of `help/`, the same ones `hikasa help` prints. Dialogs get their
  own topics in the waves that add them.
- **UI-007** (P0): Hints/tooltips for toolbar buttons with optional shortcut text;
  timing options.
- **UI-008** (P0): Drag & drop from the OS into any path edit, pane or Home button.
- **UI-009** (P1): Accessibility: all custom widgets expose AccessKit nodes (names,
  roles, selection, values); high-contrast mode respected; minimum 4.5:1 contrast in
  default themes.
- **UI-010** (P0): HiDPI per-monitor scaling; icon sets at 100/125/150/200%.

## 2. Theming

- **UI-020** (P0): Theme mode: Follow operating system / Light / Dark. Colour
  schemes are TOML files in `themes/`; the two built-ins are editable through the
  Options ▸ Theme, Colors, Fonts page with live **Preview** in the open view.
- **UI-021** (P0): Folder view elements: default text, background, folder info,
  selection (custom or system), inactive selection background, stripes, log panel,
  filtered-out, unknown, same, newer, older, orphan, different (compare colours), left
  change, right change, mergeable, conflict (merge colours); "Color folders to reflect
  content", "Use gradient for center background", system or custom font.
- **UI-022** (P0): File view elements: default text, background, stripes, edited,
  saved, line numbers font, ignored section, current line (optionally only in over/
  under layout), inactive selection, selection, important difference (text / text
  background / line background), unimportant difference, replacement, orphan, output
  (merge), left/right/conflict change backgrounds, find highlight, gutter, visible
  whitespace, syntax elements (comment, string, keyword, identifier, number, operator,
  directive, …) with bold/italic styles; three fonts: Editor, Listing, Hex.
- **UI-023** (P0): Picture Compare colours: background, tolerance-mode same/similar/
  different, checkerboard transparency.
- **UI-024** (P1): Theme import/export; community schemes.

### 2.1 Theme and scheme files (M2 W1; contract D-78)

`theme.toml` chooses a scheme for light and for dark mode and holds the fonts; the mode
itself is `[appearance] theme_mode` of `options.toml` (`follow_os`, `light`, `dark`):

```toml
schema_version = 1
light_scheme = "light"
dark_scheme = "dark"
[fonts.editor]                # also [fonts.listing] and [fonts.hex]
family = ""                   # "" = the built-in monospace font
size = 13.0                   # 6.0..=72.0
[fonts.folder]
family = ""
size = 13.0
use_system = true
```

A scheme is `themes/<name>.toml` (a name of letters, digits, `_` and `-`). The built-in
schemes `light` and `dark` are part of the program; a file `themes/light.toml` or
`themes/dark.toml` holds only the elements that differ from the built-in, so the built-ins
stay editable without being copied. Any other scheme names its `base` (`light` or `dark`,
default `light`), whose values fill the elements it does not set. A selected scheme that
does not exist is an error (`hikasa settings` and the window fall back to the built-in of
that mode with the warning `invalid_config_value`).

Colours are `#rrggbb` or `#rrggbbaa`, written in lower case. Elements:

- `[folder]` (UI-021): `default_text`, `background`, `folder_info`, `selection`,
  `inactive_selection_background`, `stripes`, `log_panel`, `filtered_out`, `unknown`,
  `same`, `newer`, `older`, `orphan`, `different`, `left_change`, `right_change`,
  `mergeable`, `conflict`; flags `use_system_selection`, `use_system_log_colors`,
  `use_stripes` (true), `color_folders_by_content` (true), `gradient_center_background`.
- `[file]` (UI-022): `default_text`, `background`, `stripes`, `edited`, `saved`,
  `line_numbers`, `ignored`, `current_line`, `inactive_selection`, `selection`,
  `important_text`, `important_text_background`, `important_line_background`,
  `unimportant_text`, `unimportant_text_background`, `unimportant_line_background`,
  `replacement`, `orphan`, `output`, `left_change`, `right_change`, `conflict`,
  `find_highlight`, `gutter`, `whitespace`; flags `current_line_only_over_under`,
  `use_stripes`, `use_system_selection`; `fallback` (exactly 8 colours: a grammar element
  without its own entry gets colour number `(element id − 1) mod 8`, as in M1); and
  `[file.syntax.<element name>]` with `color`, `bold`, `italic` (names in lower case; the
  built-ins define `comment`, `string`, `character`, `keyword`, `identifier`, `number`,
  `operator`, `directive`).
- `[picture]` (UI-023): `background`, `same`, `similar`, `different`, `checker_light`,
  `checker_dark`.

The built-in values are the M1 palettes of the window where an element existed; the
others were chosen for W1. *Interpretation (UI-009):* the elements `background`,
`stripes`, `selection` and `inactive_selection_background` of `[folder]`, and
`background`, `stripes`, `current_line`, `inactive_selection`, `selection`, the important
and unimportant text and line backgrounds, `orphan`, `left_change`, `right_change`,
`conflict`, `find_highlight` and `gutter` of `[file]` are backgrounds of default text;
in both built-ins default text on each of them has a contrast ratio of at least 4.5:1
(a unit test). The window starts using these files in M2 W4.

## 3. Status bar, log, notifications

- **UI-030** (P0): Status bar per view (left: context info; right: comparison state,
  encoding, line ending, insert/overwrite, zoom); global job progress with cancel;
  a message panel ("Difference not found." etc.) that can be suppressed.
- **UI-031** (P0): Non-modal notifications for background events (file changed on
  disk, remote disconnected, update available).

## 4. Dialog inventory (P0 unless noted)

Session Settings (per type), File Formats manager + per-kind editors, Grammar Item,
Replacement Item, Alignment Override, Other Filter Item, Name Filters, Column Handling,
Row Alignment, Sort, Go To, Find/Replace (panels), Find in Files, Find Filename, Copy/
Move confirmation (with preview list), Copy/Move to Folder, Delete, Rename, Attributes
(P1), Touch, New Folder, Exclude, Sync preview, Compare Contents, Compare Results,
Quick Compare, Report (per type) + Print Preview, Options, Open With item, Profiles
manager + editors (P1), Browse for Folder / Browse Using Profile (P1), Workspace
manager, Session Save As, Export/Import/Restore wizards, Snapshot (P1), Support,
About, Check for Updates (P1), Legend, Layout (hex), Archive password prompt (P1),
SSH host key / TLS certificate validation prompts (P1), Confirm close multiple tabs/
windows, "File changed on disk" prompts, Script status window.

## 5. Optional helpers

- **UI-040** (P2): **Clipboard history helper**: a tray application that records text
  clipboard entries (minimum length, max count/size), lists them, "Save As…", "Delete",
  "Open With Text Edit", and a global shortcut (default Ctrl+Alt+C) to compare the two
  most recent entries. Option to start with the desktop session.
