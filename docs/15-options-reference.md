# Program Options Reference

The Options dialog has a searchable category list on the left and pages on the
right; Apply / OK / Cancel. Every setting below maps to a key in `options.toml`
(key names in `snake_case` mirroring the headings).

## Startup
- Load workspace (combo of saved workspaces, empty = Home view).
- On exit, save workspace as (combo).
- When starting with a file comparison, show quick compare dialog ▸ Binary quick
  compare / Rules-based quick compare; Open view automatically if files are different.
- Clipboard history helper: run at login (P2).
- Shell/file-manager integration: include in context menu; per-command position
  (None / Main menu / Submenu) for Compare, Compare Using, Sync, Merge, Edit, Open for
  Compare, View Patch, Compare to Clipboard, Compare to Registry (Windows); hide
  "Select Left" if a selection is already saved. (Portable installs show an
  explanatory message instead.)

## Tabs
- Open sessions in: a new window / a new tab. Open child sessions in: window / tab.
- Open new tab next to active tab; Closing last remaining tab closes the window; Warn
  when closing multiple tabs; Warn when closing multiple windows; Hide tab bar if single
  tab; Show previews for individual tabs in the taskbar (Windows).

## Appearance (Theme, Colors, Fonts)
- Theme mode (Follow OS / Light / Dark); Preview.
- Sub-pages: Folder Views, File Views, Picture Compare (elements listed in
  `14-ui-and-commands.md` §2); font buttons: Folder font (system or custom), Editor
  Font, Listing Font, Hex Font; "Use stripes" per view family; "Use system" for
  selection and log colours.

## Text Editing
- Auto indent; Backspace unindents; Initialize "Text to find" to be current word;
  Show filtered line counts; Number of context lines (default 3;
  `[text_editing] number_of_context_lines`, also the default of `--context` and of the
  patch reports). (Tab stops are per file format.)

## Next Difference
- When loading new files, go to first difference; Go to next difference in files after
  copying to other side; Limit "Next Difference Files" to current folder; When no more
  differences: Wrap around to first difference, Show message panel.

## Backups
- Back up files before copy; Back up files before save.
- Back up "abc.txt" as: `abc.txt~` / `abc~.txt` / `abc.txt.bak` / `~abc.txt` /
  `abc.bak` / `abc.bak.txt` / `abc.txt` in this folder: <path> (created on demand).

## File Operations
- Confirmations: Confirm file copy; Confirm file move; Confirm replacing/deleting
  read-only files; Confirm replacing/deleting system files; Confirm overwriting newer
  files; Confirm replacing files during move; Confirm content compare; Confirm file
  delete; Require explicit side selection in Delete dialog; Confirm file merge.
- Synchronize confirmations: Prompt / Yes to All / No to All.
- Filtered items: Include hidden items by default.

## Archive Types
- Table of archive formats with editable mask per format (empty = disabled); "Add
  Plug-in…" (P2: external archive handler definitions); restart note.

## Commands (Toolbars, Shortcuts, Menus)
- See `14-ui-and-commands.md` §1 (UI-003).

## Open With
- List of external applications: Description, Shortcut, Command line with variables
  `%f` path+name, `%l` current line, `%n` name, `%p` path, `%x` extension, `%b` name
  without extension, `%F` path+name relative to working folder, `%P` relative path;
  `1`/`2` suffixes for first/second file; Working folder (specific, `<parent folder>`,
  `<base folder>`); Path delimiter replacement; Accepts files / Accepts folders;
  Refresh when finished; Multiple instances (per pair when a `%…2` variable is used,
  else per item); Wait for previous instance to finish. New / Edit / Clone / Delete.

## Tweaks
- **Check for Updates**: Check every N days (default 7; policy can disable); last
  checked. Update check downloads a JSON manifest over HTTPS; no telemetry.
- **Editor Display**: Show syntax highlighting on difference lines; Use crosshatching
  for lines beyond the end of file; Right side gutter for left editor (compare only);
  Use orphan color; Extra line spacing (px); Show column line at (0 = off); Dim
  inactive pane by (%); Use different font for merge input panes (+ Font…).
- **File Operations**: Beep after long file operations; Remove read-only flag when
  copying from read-only media; Check for files changed on disk; Automatically reload
  unless changes will be discarded; Use IPv6 when available; Comparison priority
  (Normal / Below normal / Idle; not applied in M1, see `01-architecture.md` §5.1);
  Buffer size for binary compare (bytes; `buffer_size_for_binary_compare`, default
  1 MiB, used between 4 KiB and 64 MiB); Content-test memory budget (MiB;
  `content_memory_budget_mib`, default 1024): the file content that the content tests of
  one folder comparison may hold in memory at the same time, across all worker threads
  (D-25). A test needs `min(left size + right size, budget)` bytes and waits until they
  are free, so memory stays bounded whatever the thread count; the rules-based method
  also compares files above 128 MiB by bytes (warning `rules_size_limit`).
- **Home View**: Sticky session selection.
- **Interface**: Show hints for toolbar buttons; Show keyboard shortcut in hints; ESC
  key closes file views; Single vertical scrollbar; Sticky splitter position; Thumbnail
  on large files: Compress to fit / Allow scrolling.
- **Log Panel**: Include: Operation summaries, Operation details, Warnings, Remote
  protocol trace, Debug messages; Display up to N messages; Automatically save messages
  to file.
- **Metrics**: Max auto-saved sessions; Max most-recently-used items; Milliseconds
  before help hint appears / disappears; Milliseconds before incremental search resets.
- **Name Filter Presets**: list of name + mask (e.g. `Web files = *.htm;*.html;*.css;*.js`).
- **Scripts**: Beep when finished; Close when finished; Shared scripts folder.
- **Shared Sessions File**: path to a settings package whose sessions appear read-only.

## The file in M1 (binding interpretation; contract §1.8.2; D-25, D-31)

`options.toml` lives in the configuration folder (`03-sessions-settings-profiles.md`
§5.1). Every page above is a top-level table, named by its heading in `snake_case`:
`startup`, `tabs`, `appearance`, `text_editing`, `next_difference`, `backups`,
`file_operations`, `archive_types`, `commands`, `open_with`, `tweaks`; Tweaks
sub-pages are sub-tables (`[tweaks.file_operations]`). The file may carry
`schema_version = 1`.

**Keys read in M1** (all other keys are kept as they are; M1 never writes the file):

| Key | Type | Default | Used by |
|---|---|---|---|
| `[text_editing] number_of_context_lines` | integer ≥ 0 | 3 | default `--context` of `text compare` and the context of the patch reports |
| `[tweaks.file_operations] buffer_size_for_binary_compare` | integer (bytes) | 1048576 | binary and CRC content tests (clamped to 4 KiB … 64 MiB) |
| `[tweaks.file_operations] content_memory_budget_mib` | integer (MiB) | 1024 | memory budget of the content tests of one folder comparison (D-25; 0 counts as 1) |

```toml
schema_version = 1

[text_editing]
number_of_context_lines = 5

[tweaks.file_operations]
buffer_size_for_binary_compare = 4194304
content_memory_budget_mib = 512
```

**Loading rules (D-31).** A missing file means the defaults. A `schema_version` newer
than this build supports stops the program with an error (SES-052). Unknown keys inside
the sections above are ignored silently (`--verbose` logs each one); an unknown
top-level section produces the warning `unknown_config_key`. A syntax error, a value of
the wrong type or range in a key of the table, a section written as a plain value
(`text_editing = 5`) and a negative `schema_version` are errors. Keys match the table
structure: a quoted key with dots (`"text_editing.number_of_context_lines" = 9`) is one
unknown key of an unknown section, not the option.
