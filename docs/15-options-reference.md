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

## Keys (M2; OPT ids, contract D-61, D-71, D-72)

Every option has an id `OPT-nnn` (one per page or setting group) and a priority. P0
options are read, validated and written from M2 W1 and take effect in the wave that
implements their feature; P1/P2 options are stored and validated only. Types: `bool`;
`int a..b` (inclusive range); `enum` (the listed strings); `string`; `path` (a string,
empty = unset); `mask` (a mask list as in file formats, empty = disabled). Writes are
sparse: a value equal to its default is not written (D-70). The M1 loading rules above
hold, revised by D-71: a wrong type or range in any key of this table is an error,
except in the `settings` subcommands (warning `invalid_config_value`).

| Id | P | Key (`options.toml`) | Type | Default |
|---|---|---|---|---|
| OPT-001 | P0 | `[startup] load_workspace` | string (workspace name) | `""` (Home view) |
| OPT-001 | P0 | `[startup] on_exit_save_workspace_as` | string | `""` (none) |
| OPT-001 | P0 | `[startup] show_quick_compare_dialog` | bool | false |
| OPT-001 | P0 | `[startup] quick_compare_method` | enum `binary`, `rules_based` | `rules_based` |
| OPT-001 | P0 | `[startup] open_view_if_different` | bool | true |
| OPT-002 | P2 | `[startup] clipboard_helper_at_login` | bool | false |
| OPT-003 | P1 | `[startup.shell_integration] include_in_context_menu` | bool | false |
| OPT-003 | P1 | `[startup.shell_integration] compare`, `compare_using`, `sync`, `merge`, `edit`, `open_for_compare`, `view_patch`, `compare_to_clipboard`, `compare_to_registry` | enum `none`, `main_menu`, `submenu` | `compare` = `main_menu`, `compare_to_registry` = `none`, others `submenu` |
| OPT-003 | P1 | `[startup.shell_integration] hide_select_left_if_saved` | bool | true |
| OPT-004 | P0 | `[tabs] open_sessions_in`, `open_child_sessions_in` | enum `tab`, `window` | `tab` |
| OPT-004 | P0 | `[tabs] open_new_tab_next_to_active` | bool | true |
| OPT-004 | P0 | `[tabs] closing_last_tab_closes_window` | bool | false |
| OPT-004 | P0 | `[tabs] warn_when_closing_multiple_tabs`, `warn_when_closing_multiple_windows` | bool | true |
| OPT-004 | P0 | `[tabs] hide_tab_bar_if_single_tab` | bool | false |
| OPT-004 | P1 | `[tabs] show_taskbar_previews` | bool | false (deferred, D-59) |
| OPT-005 | P0 | `[appearance] theme_mode` | enum `follow_os`, `light`, `dark` | `follow_os` |
| OPT-006 | P0 | `[text_editing] auto_indent`, `backspace_unindents`, `init_find_with_current_word` | bool | true |
| OPT-006 | P0 | `[text_editing] show_filtered_line_counts` | bool | false |
| OPT-006 | P0 | `[text_editing] number_of_context_lines` | int 0..4294967295 | 3 |
| OPT-007 | P0 | `[next_difference] go_to_first_difference_on_load` | bool | true |
| OPT-007 | P0 | `[next_difference] go_to_next_difference_after_copy` | bool | false |
| OPT-007 | P0 | `[next_difference] limit_next_difference_files_to_current_folder` | bool | false |
| OPT-007 | P0 | `[next_difference] wrap_around` | bool | false |
| OPT-007 | P0 | `[next_difference] show_message_panel` | bool | true |
| OPT-008 | P0 | `[backups] back_up_before_copy` | bool | false |
| OPT-008 | P0 | `[backups] back_up_before_save` | bool | true |
| OPT-008 | P0 | `[backups] naming` | enum `name_tilde` (`abc.txt~`), `stem_tilde` (`abc~.txt`), `name_bak` (`abc.txt.bak`), `tilde_name` (`~abc.txt`), `stem_bak` (`abc.bak`), `stem_bak_ext` (`abc.bak.txt`), `folder` (`abc.txt` in `folder`) | `name_bak` |
| OPT-008 | P0 | `[backups] folder` | path (used by `naming = "folder"`; created on demand) | `""` |
| OPT-009 | P0 | `[file_operations] confirm_copy`, `confirm_move`, `confirm_read_only`, `confirm_system`, `confirm_overwrite_newer`, `confirm_replace_during_move`, `confirm_delete`, `confirm_merge` | bool | true |
| OPT-009 | P0 | `[file_operations] confirm_content_compare`, `require_side_selection_in_delete` | bool | false |
| OPT-009 | P0 | `[file_operations] sync_confirmations` | enum `prompt`, `yes_to_all`, `no_to_all` | `prompt` |
| OPT-009 | P0 | `[file_operations] include_hidden_by_default` | bool | true |
| OPT-010 | P0 | archive type masks | `archives.toml` (below) | |
| OPT-011 | P0 | toolbars, shortcuts, menus | `commands.toml` (below) | |
| OPT-012 | P0 | Open With applications | `openwith.toml` (below) | |
| OPT-013 | P1 | `[tweaks.check_for_updates] every_days` | int 0..365 (0 = never) | 7 (deferred, D-59) |
| OPT-014 | P0 | `[tweaks.editor_display] syntax_on_difference_lines`, `crosshatch_beyond_eof`, `use_orphan_color` | bool | true |
| OPT-014 | P0 | `[tweaks.editor_display] right_gutter_for_left_editor` | bool | false |
| OPT-014 | P0 | `[tweaks.editor_display] extra_line_spacing_px` | int 0..20 | 0 |
| OPT-014 | P0 | `[tweaks.editor_display] column_line_at` | int 0..1000 (0 = off) | 0 |
| OPT-014 | P0 | `[tweaks.editor_display] dim_inactive_pane_percent` | int 0..90 | 0 |
| OPT-014 | P1 | `[tweaks.editor_display] different_font_for_merge_inputs` | bool | false |
| OPT-015 | P0 | `[tweaks.file_operations] beep_after_long_operations` | bool | false |
| OPT-015 | P0 | `[tweaks.file_operations] remove_read_only_flag_from_read_only_media`, `check_for_files_changed_on_disk`, `automatically_reload` | bool | true |
| OPT-015 | P1 | `[tweaks.file_operations] use_ipv6` | bool | true |
| OPT-015 | P0 | `[tweaks.file_operations] comparison_priority` | enum `normal`, `below_normal`, `idle` | `normal` |
| OPT-015 | P0 | `[tweaks.file_operations] buffer_size_for_binary_compare` | int 0..4294967295 (bytes; used clamped to 4 KiB … 64 MiB) | 1048576 |
| OPT-015 | P0 | `[tweaks.file_operations] content_memory_budget_mib` | int 0..4294967295 (0 counts as 1) | 1024 |
| OPT-016 | P1 | `[tweaks.home_view] sticky_session_selection` | bool | false |
| OPT-017 | P0 | `[tweaks.interface] show_hints`, `show_shortcut_in_hints`, `sticky_splitter_position` | bool | true |
| OPT-017 | P0 | `[tweaks.interface] esc_closes_file_views`, `single_vertical_scrollbar` | bool | false |
| OPT-017 | P0 | `[tweaks.interface] thumbnail_on_large_files` | enum `compress`, `scroll` | `compress` |
| OPT-018 | P0 | `[tweaks.log_panel] operation_summaries`, `warnings` | bool | true |
| OPT-018 | P0 | `[tweaks.log_panel] operation_details`, `debug_messages` | bool | false |
| OPT-018 | P1 | `[tweaks.log_panel] remote_protocol_trace` | bool | false |
| OPT-018 | P0 | `[tweaks.log_panel] display_up_to` | int 10..100000 | 1000 |
| OPT-018 | P0 | `[tweaks.log_panel] save_to_file` | path (empty = off) | `""` |
| OPT-019 | P0 | `[tweaks.metrics] max_auto_saved_sessions` | int 0..10000 | 200 |
| OPT-019 | P0 | `[tweaks.metrics] max_mru_items` | int 1..100 | 20 |
| OPT-019 | P0 | `[tweaks.metrics] hint_show_ms` | int 0..10000 | 500 |
| OPT-019 | P0 | `[tweaks.metrics] hint_hide_ms` | int 0..60000 (0 = never) | 5000 |
| OPT-019 | P1 | `[tweaks.metrics] incremental_search_reset_ms` | int 100..10000 | 1000 |
| OPT-020 | P0 | name filter presets | `filters.toml` (below) | |
| OPT-021 | P0 | `[tweaks.scripts] beep_when_finished`, `close_when_finished` | bool | false |
| OPT-021 | P0 | `[tweaks.scripts] shared_scripts_folder` | path | `""` |
| OPT-022 | P1 | `[tweaks.shared_sessions_file] path` | path | `""` |

**`archives.toml` (OPT-010).** `schema_version = 1`, then `[types]` with one `mask`
per archive type (empty = disabled): `zip` = `*.zip;*.jar;*.war;*.ear;*.apk`,
`tar` = `*.tar`, `tar_gz` = `*.tar.gz;*.tgz`, `tar_bz2` = `*.tar.bz2;*.tbz;*.tbz2`,
`tar_xz` = `*.tar.xz;*.txz`, `tar_zst` = `*.tar.zst;*.tzst`, `gz` = `*.gz`,
`bz2` = `*.bz2`, `xz` = `*.xz`, `zst` = `*.zst`. A tar family type is tried before its
single-stream type (`x.tar.gz` is `tar_gz`, not `gz`). Restart note: masks apply to
comparisons started after the change.

**`commands.toml` (OPT-011, UI-003).** `schema_version = 1`, then one table per view
type `[view.<view>]` (`home`, `folder-compare`, `folder-sync`, `text-compare`,
`text-edit`, `table-compare`, `hex-compare`, `picture-compare`): `display_filter_mode`
(enum `dropdown`, `favorites`, `toggles`; default `dropdown`), `large_buttons`,
`show_all_in_menu`, `show_all_on_toolbar` (bool, false), and per command
`[view.<view>.command.<Id>]` with `shortcuts` (array of shortcuts in the reference
spelling; replaces the defaults; `[]` removes them), `menu` and `toolbar` (bool). A
command id must exist in a catalogue section of that view; a shortcut must parse.

**`openwith.toml` (OPT-012).** `schema_version = 1`, then `[[app]]` entries:
`description` (string, required), `shortcut` (string), `command` (string, required;
variables below), `working_folder` (`parent`, `base` or a path; default `parent`),
`path_delimiter` (`""` = native, `"/"` or `"\\"`), `accepts_files` (true),
`accepts_folders` (false), `refresh_when_finished` (false), `wait_for_previous`
(false). Variables: `%f` path+name, `%l` current line, `%n` name, `%p` folder, `%x`
extension without the dot, `%b` name without extension, `%F` path+name relative to the
working folder, `%P` path relative to the comparison's base folder; suffix `1` or `2`
for the first or second item, `%%` for `%`. One instance per pair when a `%…2` variable
is used, else one per item. Default: no applications.

**`filters.toml` (OPT-020).** `schema_version = 1`, then `[[preset]]` entries with
`name` (unique, required) and `mask` (required). Defaults: `Source files` =
`*.c;*.cpp;*.h;*.hpp;*.rs;*.py;*.java;*.cs;*.go;*.js;*.ts`, `Web files` =
`*.htm;*.html;*.css;*.js`, `Documents` = `*.txt;*.md;*.rst;*.adoc`.

**`state.toml`, `theme.toml`, `themes/<name>.toml`** are described in
`03-sessions-settings-profiles.md` §5.2 and `14-ui-and-commands.md` §2.1.
