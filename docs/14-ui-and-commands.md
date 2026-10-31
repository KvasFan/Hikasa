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

*M2 W4a (contract §1.4.2–§1.4.5, D-106…D-111, D-114):*

- **Windows (D-106).** One process owns every window; each window owns its tabs, and a view
  lives in exactly one tab. The first window is drawn in the toolkit's root viewport, the
  others as further viewports of the same frame; closing the first window while others are
  open moves the next one into the root viewport, so the process ends only with its last
  window. Opens from IPC and the command line go to the window focused last
  (`[tabs] open_sessions_in = "window"`: a new window); FC-062 Open follows
  `open_child_sessions_in`.
- **Tabs (D-107).** Tab context menu: New Tab, Duplicate Tab (the same request in a new view
  with its own job), Move Tab to New Window, Move Tab to Window ▸, Close Other Tabs, Close
  Tab. Dragging a tab reorders it inside its bar; releasing it over another window's bar or
  outside every window moves it only where the platform reports window positions (not on
  Wayland). A window that loses its last tab by a move closes. `warn_when_closing_multiple_*`
  prompts precede closing several tabs or windows and Exit.
- **Layout memory (D-108).** `state.toml` keeps `[window]` (the last window closed) and
  `[layout.<view type>]` (W4: `toolbar`, `log_panel`, `log_height`, `font_size`,
  `thumbnail`), written when a window closes and at exit.
- **Effective commands (D-109).** Captions, descriptions (hints), menus, toolbar flags and
  shortcuts come from the core's effective command table (the catalogue with
  `commands.toml` applied; a catalogue entry has an optional `toolbar` flag); the GUI keeps
  per command only its icon, short label, group, availability and context-menu areas. Only
  `implemented` commands appear.
- **Menus (D-110).** Session, File, Edit, Actions, Search, View, Tools, Help; entries grouped
  by catalogue section with separators; the Session menu puts the global entries before
  Close Tab first, the view's entries next, Close Tab and Exit last; View ▸ Display Filter
  lists the `DropDown`/`Favorites` presets and the `Toggles`. The toolbar shows the
  `toolbar` entries as icons and the display-filter control of the view's mode (drop-down
  with toggles, favourites, or toggles); its right-click menu is UI-004.
- **Shortcut dispatch (D-111).** A shortcut with a context, of a command without a menu, or
  without Ctrl/Alt/Cmd on an ordinary key is taken only by its focus area (the folder tree,
  the text panes, a path field); the others by the window, the view's own section first. A
  dialog takes the window's keys (the view under it does not scroll). The toolkit does not
  tell the keypad apart: Ctrl with `=`, `-` or `0` runs a binding of `Ctrl+=`, `Ctrl+-` or
  `Ctrl+0` when there is one, else the keypad's `Ctrl+Num +`, `Ctrl+Num -`, `Ctrl+Num 0`.
- **Hints (UI-007).** Caption, the first shortcut (`show_shortcut_in_hints`) and the
  description, after `hint_show_ms`, hidden after `hint_hide_ms` of hover; every hint of the
  window (tabs, toolbar, panels, notices, dialogs) follows `show_hints` and these times.
- **Icons (D-114).** In-house vector drawings on a 16 × 16 grid painted at the current scale
  (no image files, no icon font).
- **Drops (UI-008).** A Home path field takes the first dropped path (two on the left field
  fill both); a window opens two dropped paths like the command line and explains one or
  three.

*M2 W4b (§1.4.9–§1.4.12):* the Session menu gains New Session ▸ (one item per type the GUI opens), Open Session, Load
Workspace ▸ (saved workspaces, then Manage Workspaces…), Save Workspace As… and, in session views, Go to Home, Save
Session, Save Session As…, Session Settings… and Locked; the toolbar gains the Sessions drop-down; Home's View menu
shows or maximizes the management panel. The sessions tree is a focus area: Enter, Del and F2 work only there
(`Area::SessionTree`). A workspace shortcut applies when no command took the key (D-111 rule 3), and a key without
Ctrl, Alt or Cmd only at the window root.

## 2. Theming

- **UI-020** (P0): Theme mode: Follow operating system / Light / Dark. Colour
  schemes are TOML files in `themes/`; the two built-ins are editable through the
  Options ▸ Theme, Colors, Fonts page with live **Preview** in the open view. *M2 W4c (D-130):* there is no separate
  preview mode: every change of the Appearance pages (mode, schemes, elements, flags, fonts) is applied at once to
  every window from the dialog's working copies; Cancel restores the applied theme, Apply writes `theme.toml` and the
  changed scheme files sparsely. New (name and base), Clone and Delete (not the built-ins) manage schemes; *Reset*
  takes an element back to its base (removed from the file), *Reset all* on a built-in deletes its override file.
  Default text below 4.5:1 on a background is a warning, not an error.
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

*M2 W4a (D-113):* the window's colours come only from the resolved scheme of
`theme.toml` for the theme mode (`follow_os` re-evaluates when the system theme changes),
mapped to the painters through one table (folder elements → the tree, file elements → the
text panes, `log_panel` → the log); the window chrome follows the scheme's base, and the
selection is `[folder] selection` unless `use_system_selection`. The theme is re-applied at
start, when the system theme changes and when a theme file changes on disk — never per
frame. The editor font size sets the text panes (the font commands scale from it and are
remembered per view type), the folder size the tree (`use_system`: the toolkit's default);
listing and hex sizes wait for their views. A non-empty font family is not looked up in
W4: the built-in font is used and one warning notice names the family.

## 3. Status bar, log, notifications

- **UI-030** (P0): Status bar per view (left: context info; right: comparison state,
  encoding, line ending, insert/overwrite, zoom); global job progress with cancel;
  a message panel ("Difference not found." etc.) that can be suppressed.
- **UI-031** (P0): Non-modal notifications for background events (file changed on
  disk, remote disconnected, update available).

*M2 W4a (D-115…D-119):* each window's status bar shows the active view's context text and
its state items (comparison state; format, encodings and line endings for file views) and a
job indicator over the jobs of every view of the window (click: the list, Cancel per view);
each view has a one-line message panel (info, warning, error; a close button; optional "Do
not show again" bound to an option key). One log book per process (OPT-018 filter at
insertion, errors always kept, `display_up_to` entries, optional `save_to_file`, appended in
order; a file that cannot be written stops the saving with one notice) records
finished jobs, warnings with their codes, errors, notices and settings reloads; the folder
views show it in their Log panel (`ShowLog`). A watch thread polls the settings files every
2 s and when a window gains focus (no file-system events): a changed `options.toml` or format
file reloads the service's settings (later comparisons use them), `commands.toml` rebuilds
the menus and keys, a theme file re-applies the theme, each with a notice; the GUI's own
writes raise nothing. The inputs of open text views are watched by length and modification
time: with `automatically_reload` the view reloads (info notice), else a warning notice
offers Reload; a removed input gives a warning. Notices: at most three, newest on top,
information ones hide after 8 s.

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

*M2 W4a (D-112):* dialogs are drawn inside their window, modal to it (a dimmed backdrop
takes the window's input; never a separate native window), one stack per window; Esc is
Cancel, as is the window's close button, Enter the default button, F1 the dialog's help topic
(its panel stands over the dialog). The title bar moves a dialog. A prompt (message, buttons,
default, optional "Do not ask again" bound to an option key) serves the close warnings and
later waves' questions. About is a dialog.

## 5. Optional helpers

- **UI-040** (P2): **Clipboard history helper**: a tray application that records text
  clipboard entries (minimum length, max count/size), lists them, "Save As…", "Delete",
  "Open With Text Edit", and a global shortcut (default Ctrl+Alt+C) to compare the two
  most recent entries. Option to start with the desktop session.
