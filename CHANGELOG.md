# Changelog

## Unreleased

### Added
- **Writes inside archives.** `hikasa vfs cp`, `mv`, `rm`, `mkdir`, `touch` and `--output` work on members of zip and the
  tar family, nested ones too: `vfs cp -p notes.txt a.zip/docs/`, `vfs mv a.zip/old a.zip/new`. An archive is rewritten
  once per operation, to a temporary file next to it, which replaces it in one step. `vfs mkdir new.zip` makes an empty
  archive for each writable mask. Single gz, bz2, xz and zst streams stay read-only.
- **Archives.** zip, tar, `tar.gz`, `tar.bz2`, `tar.xz`, `tar.zst` and single gz, bz2, xz and zst streams work
  wherever a file does: `hikasa vfs ls a.zip`, `vfs cat a.zip/inner/x.txt`, `quick a.zip/x b/x`, `text compare`,
  `folder compare a.zip b/`, nested as `a.zip/b.tar.gz/c.txt`, and copies out with `vfs cp -r a.zip/ out/`. Masks and limits
  are in `archives.toml` (`hikasa help archives`).
- **Archives inside folder comparisons.** `--archives files|opened|folders` and `--open-archive <PATH>`; the default
  leaves results as they were. `--content crc` uses a zip's stored CRC without decoding.
- The window shows a zipper icon on archive rows; Open on a closed one opens it as a folder; two archives open a Folder
  Compare.

### Safety
- **A rewrite cannot lose an archive.** A read error, a full disk, Cancel or a failed rename leave it byte for byte as it
  was, and a move into an archive removes its sources only after the rewrite. An archive that changed since the
  operation was planned is left alone (`changed since the plan was made`). A member cannot be written over the archive
  that holds it, nor a file moved into itself.
- **Two writers cannot overlap.** A rewrite holds a lock next to the archive, so a second one, in any process, fails at
  once with `another operation is changing this archive` instead of losing the first one's change.
- **A rewrite keeps what it did not touch** as bytes, in order: encrypted members and methods it cannot decode, refused and
  link members, devices, sparse files, unknown extra fields, comments and raw names. A member hidden by a later one of the
  same name stays with it: it moves with it, and goes when that member is removed or replaced, with a warning, so what
  was overwritten does not live on.
- **The temporary copy of an archive, and of any file Hikasa saves, is private** until it replaces the file. A kill during a
  rewrite leaves it next to the archive, and the next rewrite of that archive removes it.
- Archives are treated as hostile input: bad names are refused and listed, links are never followed or written to disk,
  damage keeps what lies before it, encrypted members are reported, and bombs stop at the limits (`entries_max`,
  `paths_max_mib`, `depth_max`, `stream_max_gib`, `cache_max_gib`, `ratio_max`, `nesting_max`).
- A member copied out never brings `setuid`, `setgid` or sticky bits, nor more than the umask gives.
- `--json` escapes DEL, C1 controls and bidirectional controls, so a hostile name cannot act on a terminal.

### Data safety
- **A settings file this version cannot read is never overwritten from the Options dialog.** A newer `schema_version` or
  broken TOML used to show defaults, and Overwrite replaced the file with them. The page now says why, the prompt holds
  only Reload and Cancel, and `settings_save` refuses such a file even with `force` (new outcome `Unreadable`). One such
  file also no longer turns the other settings files into defaults at start
  (`Service::settings_snapshot_partial`).
- **Overwrite keeps the unknown keys on disk.** It read them when the dialog opened, so keys added since were lost; it
  now reads the file again first.
- **The window's close button asks** when the Options dialog has unapplied edits, as Esc does.
- **`hikasa settings unset` removes a key kept without being known**, such as a view or command of a newer version, which
  it used to answer `not_found`; `settings get` of a key inside a kept command override shows its value.

### Fixed
- First run: no more "File formats reloaded" notices for the program's own format files.
- Home follows a session deleted or changed by the command line at the next 2 s poll, without a focus change.
- Options search: the page follows the search, and no page shows when nothing matches.
- Dialogs: buttons sit at the bottom of the wizards, lists take the free height, and a dialog moves by its title bar.
- A notice raised while a dialog was open could not be clicked if an earlier notice was on screen, and the help of a
  dialog (F1) could be neither scrolled nor closed: both now stand above the dialog.
- Menu marks sit on the line of their label; the Menu column of the shortcuts page is straight; the log panel says
  "1 entry"; the help of Text Compare puts the thumbnail on the left.

## 0.1.90 (2026-10-31, pre-release)

A pre-release of 0.2.0 with the first part of M2: waves W0–W4 and the packages. 0.2.0 stays reserved for M2 complete;
W5–W14 are not in this version.

### Windows and macOS port of M2
- **Session and workspace lookup:** on a case-insensitive file system (Windows, macOS) `SessionStore::find` and
  `WorkspaceStore::find` answered with the case the user typed; they answer with the name on disk.
- **`LocalVfs` on Windows, as the contract says on every platform:** a path below a file is `not_a_directory` (it was
  `not_found`); `rename` with `replace` replaces an empty folder and a link to a folder, which Windows refuses
  to rename onto (the entry is removed first and put back if the rename fails, so that step is not atomic there);
  `create_symlink` stores `\` for `/` in the target, because a Windows link holding `/` resolves to nothing.
- **Tests:** platform-aware paths and golden normalisation; scenarios that need a read-only folder or a `..` in a spec
  are Unix only, each with its reason.

M2 is planned in `docs/design/m2-plan.md` (the maintainer's answers in §4.1) and specified wave by wave in
`docs/design/m2-contracts.md`. Entries per wave, newest first.

### Brand and packaging
- **Brand:** a logo for light and dark backgrounds (`assets/brand/`), with generated icons; the window now shows it.
- **Packages** (`packaging/`, local scripts, `packaging/README.md`): Linux `.tar.gz`, `.deb`, AppImage and an Arch
  `PKGBUILD`, Windows portable zips and MSI installers for x64 and x86, with `SHA256SUMS` and smoke tests. Every package
  carries `THIRD_PARTY_LICENSES.txt`, generated from the crates compiled into it. Desktop entry, AppStream metainfo and
  the `application/x-hikasa-pkg` MIME type for `.hikasa-pkg` files.
- **Windows:** `hikasa-gui.exe` is a windows-subsystem program in release builds and no longer opens a console;
  external conversions started by the GUI do not open console windows either.

### Wave 4c — Settings dialogs
- **Options dialog** (OPT pages, D-128): Tools ▸ Options…: a searchable page tree (title, label, key path, OPT id),
  every P0 key of `options.toml` on exactly one page, Archive Types, Open With and Name Filter Presets; live checks with
  each file's schema, Apply / OK / Cancel; only changed files are written, sparsely, as `hikasa settings set` writes
  them, keeping unknown and hidden keys. A file changed on disk meanwhile asks Overwrite / Reload / Cancel, and a notice
  offers Reload or Keep my edits as soon as it is seen.
- **Toolbars, Shortcuts, Menus** (UI-003, OPT-011, D-129): per view type: Menu and Toolbar check boxes, shortcuts by
  capture (Add, Remove, Default), display filter mode, large buttons, show-all flags, Reset View; conflicts shown live,
  never blocking; overrides of commands not implemented yet are kept. "Toolbar Options…" opens it from the toolbar's
  right-click menu.
- **Theme editor** (UI-020…023, FC-031, D-130): theme mode, the light and dark scheme, every element of `[folder]`,
  `[file]` (syntax elements and the fallback cycle) and `[picture]`, the flags and the four fonts, with Reset, New, Clone,
  Delete and Reset All; every change shows at once in every window, Cancel restores; contrast warnings below 4.5:1.
- **Wizards** (SES-060…062, D-131): Export, Import (per-item actions, locked sessions, delete choices, a summary of the
  exact choices) and Restore Factory Defaults, over the same calls as `hikasa settings`.
- **Packages on the command line** (D-127): `hikasa-gui x.hikasa-pkg` and `hikasa gui open x.hikasa-pkg` show the import
  wizard (new IPC request `ImportPackage`; forwarded to a running GUI; exit 0); `--silent` still imports without a window.
- **Core:** `CommandOverrides::{set_shortcuts, set_in_menu, set_on_toolbar, view_mut, reset_view, prune_view}`,
  `settings::form` (one key of a typed schema as a form field), scheme elements by name, `theme::selected_scheme`,
  `State::{dialog_text, set_dialog_text}`, `Service::gui_import_package`. `ProgramOptions`, `Export`, `Import` and
  `Restore` are implemented.
- **Dialogs:** a drop while a dialog is open no longer reaches the view under it; text fields stand out from the
  dialog's background.
- **Review fixes.** `commands.toml`: a view or command id this build does not know, and the override of a planned
  command with an unreadable shortcut, are kept in the file with a warning, in every load mode and through `settings
  set` (they used to be errors or dropped). The Options dialog checks only the files it would write; OK closes only when
  every file of the run succeeded; a failed read-back after the writes no longer reads as a failed save
  (`SettingsSaveDto::reread_error`); resetting the last element of a built-in keeps a file with unknown keys; Esc leaves
  open popups first and asks before discarding; Apply takes or refuses an open Open With item, whose shortcut is a capture
  field. The Import wizard shows its package on every step and never ignores a second one; wizard paths must be
  absolute. The OS close button and Exit wait for a running import, restore or Apply, and an exit nobody can refuse
  waits for the write. `text_scroll` and `two_windows` report the median of three runs after idle frames.

### Wave 4b — Sessions in the window
- **Sessions of views** (SES-001…006, D-120, D-121): every comparison view has a session: Save, Save As, Locked,
  close prompts ("Save changes to session?", "Save changes to locked session?") on every close path (tab, window,
  exit, Go to Home, the Sessions drop-down, Load Workspace; Cancel stops them), auto-save on close, at exit and every
  300 s while the view changes. `hikasa gui close` and `gui quit` never ask. A pair opened from a saved folder session
  is its child (TC-052).
- **Session Settings dialog** (SES-020…022, D-122): the Specs tab (inputs with recent paths, description, format,
  encodings), the scope selector and the tab registry the rule tabs of later waves join.
- **Home view** (SES-030, SES-031, SES-093, D-124): type buttons as drop targets, search, the sessions tree (New,
  folders, favorites, four auto-saved groups) with its context menu (13 new `Tree*` catalogue commands) and the
  management panel. `HomeCompare` accepts `clipboard://`.
- **Workspaces** (SES-040…042, D-125, D-126): Save Workspace As, Load Workspace, Manage Workspaces (rename, delete,
  shortcuts with conflicts shown), `[startup] load_workspace` and `on_exit_save_workspace_as`, lazy restore: a tab
  starts when first activated, and `gui list` shows the pending ones.
- **Command line** (CLI-002, SES-042, D-127): `hikasa-gui <session>` and `hikasa-gui <workspace>`, forwarded as
  `LoadSession`/`LoadWorkspace` to a running GUI.
- **Keeping changes safe:** Yes at a close holds the tab until the save returned; a failed save keeps it open with the
  reason and stops the close, Exit included. Renames, moves and deletes in Home reach open views and pending tabs; a
  Save refuses a file that is no longer the session the view loaded. Auto-save keeps a parent's overrides, works in an
  idle or minimized window, and asks at the close when its file is locked.
- **Locked sessions in folders** (D-91): `hikasa session rmdir --recursive` takes `--force`, and refuses a folder
  holding locked sessions without it; deleting or renaming such a folder in Home names them first.
- **Service calls** (§1.4.15): `session_load`, `session_load_autosaved`, `session_defaults`, `session_save`,
  `session_save_defaults`, `session_autosave`, `session_delete_autosaved`, `session_for_view`, `session_view_of`,
  `session_resolve_parent`, `session_apply_scope`, `session_rename_folder`, `session_folder_contents`, `session_prune_autosaved`, `workspace_load`, `inline_table`.

### Wave 4a — GUI shell
- **Windows and tabs** (UI-001, SES-090, SES-091, D-106…D-108): several windows in one process (Session ▸ New
  Window); the tab context menu (New Tab, Duplicate Tab, Move Tab to New Window, Move Tab to Window ▸, Close Other
  Tabs, Close Tab); tabs drag to reorder and, where the system reports window positions, into another window or
  out into a new one; the `[tabs]` options (where opens go, new tab placement, closing the last tab, hiding a single
  tab's bar, the warnings before closing several tabs or windows). `state.toml` remembers the last window's size
  and, per view type, `[layout.<view type>]` (toolbar, log panel and its height, font size, thumbnail).
- **Commands from core** (UI-002…UI-007, D-109…D-111): `hikasa_session::commands::effective` merges the catalogue
  with `commands.toml`; menus (grouped by section, View ▸ Display Filter), toolbars (in-house vector icons, large
  buttons, the display filter as a drop-down, favourites or toggles; right-click menu), hints with the shortcut
  and their timing options, and shortcut dispatch by focus area all follow it. New catalogue commands
  `TabDuplicate`, `TabMoveToNewWindow`, `TabMoveToWindow`, `TabCloseOthers` and a `toolbar` flag; `WindowNew`,
  `ShowToolbar` and `ShowLog` are implemented.
- **Dialogs** (D-112): modal inside their window (Esc, Enter, F1); prompts with "Do not ask again"; About is a
  dialog.
- **Theme** (UI-020…022, FC-031, D-113): the colours come from the scheme of `theme.toml` (light/dark, following
  the system), re-applied when a theme file changes; the editor and folder font sizes apply.
- **Status bar, jobs, messages, log** (UI-030, FC-100, D-115, D-116): state items per view, a job indicator with
  Cancel per view, a message line per view, and one log book per process (OPT-018 filter, capacity, optional
  file) shown by the folder views' Log panel.
- **Changes on disk** (UI-031, SES-053, D-117…D-119): a watch thread polls the settings files every 2 s and on
  focus; changed options and format files reload the service's settings (`Service::reload_settings`; later
  comparisons use them), `commands.toml` and theme files re-apply, each with a notice; the inputs of open text
  views are watched (reload automatically or offer Reload; a removed input is reported).
- **Drag and drop** (UI-008): files dropped on a Home path field fill it; two dropped on a window are compared.
- Core: `Service::settings_snapshot`, `settings_save`, `reload_settings`, `state_update`, `log_append`,
  `input_stamp`; `Service::options()` and `registry()` return snapshots (`Arc`); `State::layout`;
  `hikasa_session::log` and `hikasa_session::watch`. Performance mode `HIKASA_GUI_BENCH=two_windows`.
  `Service::loaded_settings()` (options, ignored keys and registry as one snapshot; reloads are serialized);
  `Service::ignored_option_keys()` returns a `Vec<String>` refreshed by each reload; the spooled copy of standard
  input is not watched; a refining catalogue entry without a shortcut takes the shared one.

### Wave 3 — VFS mutations, clipboard, generic copy
- **Mutating VFS** (VFS-001, ARCH-030, D-92…D-99): every back-end gains single-item primitives — create a folder,
  remove, rename, set times, set attributes, create a symbolic link, move to the recycle bin, an optional fast
  copy — each behind a new capability (`mkdir`, `remove`, `rename`, `set_attrs`, `trash`, `create_symlinks`,
  `set_created`; `set_times` is now used). Writes never follow the final symbolic link; new error kind
  `not_empty`.
- **Generic copy, move, remove** (ARCH-031, D-95…D-98, D-103): `hikasa_vfs::ops` plans every operation completely
  (whole trees, in execution order) before the first write, then executes it item by item with per-item results
  (`planned`, `done`, `skipped`, `failed`, `not_run`), re-checking each target. Copies are atomic per file (local
  files up to 64 MiB in one call, `copy_file_range` on Linux), keep times and permissions, copy links as links and follow overwrite
  policies `fail` (default), `skip`, `always`, `newer`; moves rename where they can, else copy and remove per item
  with rollback; the recycle bin never falls back to a permanent delete. Review fixes: `.`/`..` names and roots
  (also through `..`) are never operated on; a failed replacement (a link or folder over a file) leaves the old
  item; nothing runs below a folder that failed or was swapped for a link after planning (`not_run`), and
  removals re-check folder identity; `newer` is re-checked at execution; a move keeps its copy when the source
  vanished; case-only `mv` renames; local copies of read-only files work; planning reports `scanning` progress
  and can be cancelled; a link on another file system than the recycle bin is refused.
- **`hikasa vfs cp|mv|rm|mkdir|touch`** (D-102): `--dry-run` prints the plan; `rm` needs `--yes`; schema
  `hikasa.vfs_op.v1` (35 schemas); failed items keep `ok: true` and exit 100.
- **Clipboard** (TC-002, TC-065 data, D-100, D-101): `clipboard://` reads the clipboard's text, or an image as an
  in-house PNG named `clipboard.png`, once per command and only when given; `HIKASA_CLIPBOARD_FILE` stands in for
  it (empty → `not_found`, above 1 GiB → `too_large`). `gui open clipboard://` and `--gui` now send it to the
  window, which reads its own clipboard. Writing to `-` or `clipboard://` is `unsupported`.
- New dependencies `trash` (recycle bin, `hikasa-vfs`) and `arboard` (clipboard, `hikasa-session`), each confined
  to one file by the architecture test.

### Wave 2 — sessions, workspaces, settings packages
- **Sessions** (SES-001…006, SES-008, SES-010, D-80…D-84): one TOML file per session (`sessions/<folder>/<name>.toml`,
  defaults per type in `sessions/_defaults/`, auto-saved views in `sessions/_autosaved/`), read and written by the
  settings engine. Rules are stored sparse against the type's defaults, so a later change of the defaults reaches
  every session that did not change that rule; Folder and Text Compare rules are typed (the TOML of
  `--rules-file`), the other types keep theirs as they are until their waves. Names are portable (D-80) and unique
  per folder in any case; `-` and `clipboard://` are never stored (warning `spec_not_saved`, a new code).
  Auto-save rotation with day groups and the `max_auto_saved_sessions` limit, the scope table of the session
  settings dialog (view, defaults, these files / all files within the parent) and the close-prompt table are pure
  core functions with table tests.
- **`hikasa session list|show|new|edit|delete|rename|lock|unlock|mkdir|rmdir|run`**: `session run` runs a Folder or
  Text Compare session through the same service call and output as `folder compare` / `text compare` (same JSON,
  same exit codes); schemas `hikasa.session_list.v1`, `hikasa.session_show.v1`, `hikasa.session_write.v1`.
- **Workspaces** (SES-040…042, D-85): `workspaces/<name>.toml` with windows, tabs (a named session or an inline
  one) and layouts; `hikasa workspace list|show|delete|rename` (`hikasa.workspace_list.v1`,
  `hikasa.workspace_show.v1`, `hikasa.workspace_write.v1`); a tab whose session is gone opens Home with warning
  `session_missing` (a new code).
- **Settings packages** (SES-060…062, D-86, D-87): `hikasa settings export` writes a `.hikasa-pkg` (a zip with
  `manifest.toml`; deterministic: same configuration and time, same bytes); `hikasa settings import` validates
  every entry (paths, sizes, schemas) before writing anything, then prints or executes the plan (`--dry-run`,
  `--yes`, `--silent`, `--delete-existing`); `settings restore-defaults --category` restores factory defaults by
  category. Schemas `hikasa.settings_export.v1`, `hikasa.settings_import.v1`. New dependency `zip` (pure-Rust
  deflate only).
- **The window opens sessions and workspaces over IPC** (ARCH-091, D-89): `LoadSession` and `LoadWorkspace` are
  answered by the window; `hikasa gui session <ref> [--wait]`, `hikasa gui workspace <name>`, and `hikasa gui
  open <name>` resolves one argument that is not an existing path as a session, then a workspace (CLI-002, D-88); a
  `.hikasa-pkg` with `--silent` is imported. A window of 0.1 is reported as "does not support sessions; restart it".
- **Explicit rules stay written** (D-90): a session file keeps every rule it already held, even one that now
  equals the defaults, so a later change of the defaults does not change an explicit choice; `hikasa session edit
  --unset <key>` removes a rule (also with `--defaults`). Unknown rule keys are kept. The folder size and date
  filters and the time offset can be written `false` ("off") in a session or a `--rules-file`.
- **Locked sessions are protected** (SES-004, D-91): `session delete` and `session rename` need `--force`;
  `settings import` does not replace or delete a locked session and `settings restore-defaults
  --delete-sessions` does not delete one unless `--force` (new flag on both; plan action `skip_locked`, new
  warning code `session_locked`); auto-save never writes a locked file.
- **Stricter package import**: every session's rules and text rules are built and their regular expressions
  compiled before anything is written; names are matched without regard to case (two entries differing only by
  case are refused; an existing file keeps its spelling); an unknown category in the manifest is refused.
- Help topics `hikasa session` and `hikasa workspace`; `hikasa settings` and `hikasa gui` extended.

### Wave 1 — settings store
- **One engine for every settings file** (D-70, T13): `options.toml`, `state.toml`, `commands.toml`,
  `theme.toml`, `themes/<name>.toml`, `filters.toml`, `openwith.toml` and `archives.toml` are typed
  (`hikasa_session::settings`), carry `schema_version` with a migration chain (older files are written back on
  the next save, newer ones refused), keep unknown keys, are written sparsely (only values that differ from the
  defaults) and atomically, and refuse to overwrite a file changed on disk since it was read (`--force`).
- **Every option of `docs/15`** (OPT-001…OPT-022, D-71): `options.toml` is read completely; a wrong type or a
  value out of range is an error naming the file and the key for every command except `hikasa settings`, which
  warns `invalid_config_value` (a new warning code) and uses the default so that `settings set` repairs the file.
  Entries of `[[app]]` and `[[preset]]` are never dropped or reordered by that repair.
- **`hikasa settings get|set|unset|list|restore-defaults`** (D-73, D-77) with keys `<file>.<path>`, suggestions
  for unknown keys, shortcut-conflict warnings, a `--dry-run` plan and `--yes` for deleting files; schemas
  `hikasa.settings_get.v1`, `hikasa.settings_set.v1`, `hikasa.settings_list.v1`, `hikasa.settings_restore.v1`.
- **`hikasa format new|edit|delete|import|export|enable|disable|move|restore`** (D-75): they write only the files
  they change; a built-in can be disabled or restored, not deleted; the id `order` is reserved; schemas `hikasa.format_write.v1`,
  `hikasa.format_export.v1`, `hikasa.format_restore.v1`.
- **Change detection** (D-74): `ConfigWatch` polls length, time and CRC-32 of every settings and format file.
- **Themes** (D-78): built-in `light` and `dark` schemes (the M1 palettes) with every element of UI-021…023,
  editable by override files, custom schemes with a `base`; contrast of default text ≥ 4.5:1 is tested.
- **Shortcuts, Open With, archive types, filter presets** in core: the shortcut spelling of the command
  reference (every catalogue shortcut parses), per-view customization and conflicts (UI-003); pure Open With
  command expansion with quoting (D-76); archive type masks (OPT-010); name filter presets (OPT-020).
- Help topic `hikasa settings`; `hikasa format` extended. The window takes the sections of its view types from
  `hikasa_session::commands::view_sections`.

### Wave 0 — infrastructure
- **Command catalogue in core** (D-65): every command of the reference (458 in 14 sections) is data in
  `core/hikasa-session/catalogue/commands.toml`, read by `hikasa_session::commands`, with a status
  `implemented` (the 60 the window executes) or `planned`. `docs/reference/commands.md` is generated from it by
  `cargo xtask commands-doc` and gains a Status column; without it the file is the M1 one (D-66).
- **Help topics** (D-67): 17 Markdown topics in `help/` (contents, command-line basics, exit codes, JSON output,
  one per subcommand, the main window and the three views). `hikasa help [topic…]`, `hikasa help --list`, schema
  `hikasa.help.v1` (appended to `hikasa version --json`); an unknown topic exits 100 with suggestions.
- **The window**: Help ▸ Contents and Help ▸ Context Sensitive Help (F1) open the topics in a resizable panel on
  the right; links open other topics; Esc closes it. Tests check that the window executes exactly the
  `implemented` catalogue entries, with the catalogue's shortcuts.
- **Tooling** (D-68, D-69): the `xtask` workspace member (`cargo xtask commands-doc [--check]`, `cargo xtask
  help-check`); CI runs `cargo nextest run` (latency tests one at a time, retried on CI) plus doc tests, and a
  `docs` job runs the two xtask checks; `bench.yml` runs the benches and the G1/G2 gates by hand.
- Decisions D-59…D-69 record the maintainer's answers (xz through the pure-Rust `lzma-rust2`; no `uzers`).

## 0.1.0 — 2026-10-26 (M1)

M1: the core engines, the `hikasa` command line and the GUI prototype (`docs/16-roadmap.md`). Entries per wave,
newest first.

### Windows and macOS port fixes
- Opening a folder as a file is `is_a_directory` on Windows too (`LocalVfs::open_read`, `--log`; Windows reports
  it as "access denied"), and the mapped binary compare and CRC fall back for a folder instead of failing.
- macOS (and every Unix without `/proc`): the effective uid is found also when several threads capture the host
  facts at once; a shared probe file name made all but one report the uid as unknown, and the IPC socket folder
  check then refused the endpoint (`InsecureEndpoint`).
- A path flag given twice (`--config-dir cfg` and `--config-dir <cwd>/cfg`) is the same path also when it does not
  exist yet behind a symbolic link (macOS `/var` → `/private/var`).
- IPC on Windows: a client's connect to a busy pipe waits at most the call's timeout (`IpcError::Timeout`, exit
  100) instead of forever, and a server shut down right after it started no longer leaves its accept thread
  blocked (the self-connect stays open until the accept thread ends; contract §1.8.7 item 13).
- The repository checks files out byte for byte (`.gitattributes`: `* -text`), so goldens with CRLF or LF line
  endings stay exact on Windows; CI runs `cargo test --workspace --no-fail-fast`. CLI goldens mask the
  platform's file metadata, case sensitivity and capability lists outside Linux (contract §6.1).

### M1 final review fixes
- `hikasa-gui` binds its IPC endpoint and opens the window before it writes the first-run format files
  (`FormatStore::materialize`, D-21); the files are written on a background thread after the first frame, so
  disk flushes on a slow or busy disk can no longer delay the window or make `gui open --wait` miss its launch
  budget (unit tests with a blocking fake task and for the startup path; `tests/cli_gui.rs` checks that the
  files appear after the launch).
- Window tests run on a virtual X display only: `env -u WAYLAND_DISPLAY xvfb-run -a …` (`docs/17` §3.5);
  `tests/cli_gui.rs` and `tests/perf_gate.rs` remove `WAYLAND_DISPLAY` from the processes they start whenever
  `DISPLAY` is set. `docs/02` §2: the long non-ASCII scroll run was on the real display, not under Xvfb.
- CI no longer installs GTK/WebKit packages (eframe needs none to build); the `--gui` help text no longer shows
  an internal decision id; README, `docs/00` and `docs/16` describe the implemented state and the open release
  items; crate descriptions say what each crate contains in M1; contract §0.4 rule 3 (`anyhow` only in the
  GUI binary) and §6.4, `docs/02` and `docs/17` (CI bench job, `cargo xtask commands-doc` and nextest are M2
  items); `docs/12` on `quick --size` and the folder report defaults.

### Wave 6b — the GUI on egui
- `hikasa-gui` moved from Dioxus 0.7 to **egui 0.37 with eframe** (D-56 as amended, ARCH-GUI-001): the Dioxus
  prototype met the first screen and the frame rate but not the ARCH-GUI-003 memory limit (598 MiB PSS, 456 MiB
  GUI RSS), so the maintainer moved the whole GUI. The view models, the command catalogue, the IPC handler and
  single instance, the D-23 close codes, Home's rules and FC-062 are unchanged; the page, its script and styles
  (`assets/`) and the page ↔ Rust request channel are gone.
- One native window, repainted on demand (the mailbox wakes it with `request_repaint`; no polling thread): menu
  bar, tabs, toolbar (with the display-filter and folder-mode lists, the toggles and Ignore unimportant) and
  status bar generated from the catalogue; light and dark colours follow the system. Keyboard shortcuts are
  taken at the window root, the focus-bound keys (Enter, Left, Right, arrows, pages) by the folder tree while it
  has the focus; Ctrl with `+`/`=`, `-` and `0` stand for the keypad keys of the font-size commands.
- *Text Compare*: the panes are custom-painted for the visible rows only (line numbers, important, unimportant
  and orphan backgrounds, hatched absent sides, inline highlights, syntax colours, gap bars that expand on a
  click, the cursor row), with a thumbnail (click or drag to jump), a scroll bar, wheel and keyboard scrolling;
  the scroll position is a row index, so million-row views scroll exactly. *Folder Compare*: the tree is painted
  the same way (colour spots, names, sizes, times, centre marks with tooltips, disclosure triangles, selection,
  hover), updates while scanning, and opens a pair with Enter or a double-click.
- ARCH-GUI-003 re-measured 2026-10-26 (desktop, Radeon RX 7900 XTX on Vulkan, 120 Hz display; `docs/02` §2): first
  screen 0.51 s after the process start (coloured 0.70 s); 119.6–119.9 fps in all three scroll phases (UI work
  per frame ≤ 5 ms, p95 ≤ 3.4 ms); memory 275 MiB PSS / 281 MiB RSS after scrolling (peak ≈ 365 MiB) against
  381.5 MiB — **all three items met** on this machine (not yet on a 2020-class laptop). An empty window takes
  76 MiB PSS (Dioxus: 168). Folder scan with every folder open: 100,020 files per side in 0.21 s with UI work
  ≤ 3.3 ms per frame; 400,080 files with a warm cache ≤ 11.8 ms per frame.
- Tests: 39 unit tests (the mailbox's notifier, egui key → catalogue shortcut, UTF-16 columns → layout
  sections replace the Dioxus request-query and ticker tests); the window test runs under Xvfb (with
  `env -u WAYLAND_DISPLAY`, see the final review fixes); the benchmark
  summary now reports per phase the frame work, the frame intervals and eframe's CPU time per frame.
- Review fixes (wave 6b verification):
  - The benchmark reads PSS and RSS from one `smaps_rollup` snapshot and only the peak from `VmHWM` (the RSS came
    from the approximate `VmRSS`, and the unit test that compared it with the PSS failed now and then); memory
    re-measured on the same desktop: 275 MiB PSS / 281 MiB RSS, peak ≈ 365 MiB (`docs/02` §2).
  - Text panes: the UTF-16 column table of a row is built in one pass, so long non-ASCII lines no longer cost
    quadratic time per frame (a 4,096-column regression test).
  - `hikasa-text` gains `TextComparison::next_section_from` and `TextDisplayFilter::toggles`/`toggled` (additive,
    contract §1.5.7); the GUI no longer re-derives the importance rule of section navigation or the filter →
    toggles table. Toggling a category in *Show Context* now keeps the context rows (only switching *Same* off
    removes them); *Same* shows as checked there.
  - Names with control or bidirectional characters are escaped in tab captions (`escape_name`, as `gui list`
    prints them), pane headers, the folder base strip and error strips (`escape_controls`). The text panes show
    C1 controls, bidirectional formatting characters, zero-width characters, LS, PS and BOM as `\u{…}` escapes.
  - A job wakes the window after its body returned and the window repaints at once until the result is
    published, so a finished comparison no longer waits for the 100 ms poll.
  - Closed views release their data (a unit test); the memory a long-running window keeps after many
    open/close cycles is glibc's retention of freed heap, bounded and documented in `docs/02` §2.
  - README, `docs/02`, `docs/05` and the contract no longer name Dioxus as the toolkit or an approved crate.

### Wave 6 — the GUI prototype (Dioxus, replaced in wave 6b)
- `hikasa-gui`: the desktop window on Dioxus 0.7 (D-56, ARCH-GUI-001) over the session service (contract §1.10,
  §3.7); it contains no comparison logic (ARCH-L3, architecture check 7). One window with tabs:
  - *Home*: two path fields and Compare, resolved like the command line (`resolve_open`, CLI-002) on a job
    thread; the paths are passed verbatim (spaces are legal in names), and `-` and `clipboard://` are refused
    with an inline message (the window never reads standard input).
  - *Folder Compare*: a virtualised tree (only the rows on screen exist in the page) with per-side colour spots
    and names (FC-031, D-55), size and time columns, centre state marks, open/close folders, Expand All and
    Collapse All, the 13 display filter presets, the six toggles and the three folder modes (FC-055, FC-056),
    Ignore Unimportant Differences and Suppress Filters (FC-057), next/previous difference (FC-061), Stop, Full
    Refresh, and Open (Enter or double-click) of a file pair in a Text Compare tab with the exact paths of both
    sides (FC-062). The tree is rebuilt from the live model while the scan runs (at most every 66 ms).
  - *Text Compare*: side-by-side panes drawn on a canvas (ARCH-GUI-002): the page asks for the slice of rows it
    is about to draw and the Rust side lays out only that slice (line numbers, important, unimportant and orphan
    colours, inline highlights, syntax colours per grammar element, gap bars that expand on click); a
    thumbnail (TC-032), next/previous difference section (TC-050), display filters with Show Context and the
    toggles (TC-040), Ignore Unimportant Differences, font size, Reload. The documents appear unaligned as soon
    as they are loaded (`CompareObserver::loaded`), the aligned comparison replaces them.
- Command catalogue (ARCH-070, UI-003): the ids of `docs/reference/commands.md` (plus `HomeCompare`), captions,
  hints, default shortcuts, menu and toolbar placement, radio groups and availability predicates; the menu bar,
  the toolbar and the keyboard are generated from it.
- IPC server and single instance (ARCH-090…092, §4.4): `OpenView` (text and folder views, `--wait`),
  `ListViews`, `CloseView`, `Ping`, `Shutdown`; a closed view reports its D-23 code through
  `ViewOutcome::close`; quitting (IPC, Exit, the window's close button) closes every view with reason
  `shutdown` and its real code, then shuts the server down; a second `hikasa-gui a b` forwards its request to
  the running instance and exits 0 (`--solo` runs alone).
- Tests: 32 headless unit tests drive the view models through the catalogue (the enabled state of every
  command, navigation, filters, slices, FC-062, Home, the D-23 outcomes) and the IPC handler with a real
  in-process server and client; `tests/cli_gui.rs` (ignored: it opens a window) runs the real `hikasa` against
  the real `hikasa-gui` (`gui open --wait` codes of a text and a folder view, `gui ping/list/close/quit`,
  second-instance forwarding); `tests/perf_gate.rs` (ignored) measures ARCH-GUI-003 and the folder scan through
  the scripted benchmark mode (`HIKASA_GUI_BENCH=scroll|folder`).
- ARCH-GUI-003 measured 2026-10-26 on a desktop (Ryzen 7 7800X3D, WebKitGTK 2.54, 35 visible rows), not on the
  2020-class laptop the requirement names (`docs/02` §2 "M1 wave 6"): first screen 0.43 s after the process
  start (aligned and coloured 0.60 s); scrolling a 1,000,000-line comparison at 60.2 fps in all three phases, at
  the page's 60 Hz frame-clock ceiling; memory 598 MiB PSS for the GUI and its WebKit processes and 456 MiB RSS
  for the GUI process alone, against 400 MB = 381.5 MiB, so the memory limit is **not met** (an empty Home view
  already takes 168 MiB PSS). Reported to the maintainer; the toolkit is not switched (17-agent-workflow §7.1).
  Folder scan with CRC and *Show All*, every folder opened: 100,020 files per side in 0.26 s (UI-thread work
  ≤ 2.5 ms), 400,080 in 0.68 s with 7 rebuilds while scanning (≤ 8.9 ms), 1,000,200 in 1.64 s with 18 rebuilds
  (up to 21 ms with 1.1 million rows open, so frames are missed at that size).
- Wave-6 review fixes: Home no longer blocks the window on `-` (it read standard input on the UI thread);
  Stop while a folder comparison prepares stops it (no scan starts); the folder-view verdict and close code come
  from the core (`ViewOutcome::for_folder`, `PreparedFolderCompare::view_verdict`); a view whose `OpenView`
  arrives while quitting reports `shutdown` with 100 (`ViewOutcome::close_unopened`); the Text Compare status bar
  is visible again and the canvases are sized in whole device pixels (no per-frame rescaling); plain Enter, Left
  and Right reach focused buttons, lists and check boxes (the tree takes them while it has the focus); the
  next/previous-difference state is memoised per rebuild and selection; `gui list` shows the tab caption from the
  start; two empty files say so; the benchmark reports PSS and RSS in MiB against 381.5 MiB and the page frames
  while scanning; architecture check 7 no longer mistakes `&'static str` for a `static` item (contract §1.8.7
  item 37); `docs/reference/commands.md` lists `HomeCompare` and the Enter/Right/Left defaults.
- `docs/02-tech-stack.md`: the D-56 approval and the ARCH-GUI-003 measurements.

### Wave 5 — the `hikasa` command line
- `hikasa-cli`: the `hikasa` binary over the session service (contract §1.9): `quick`, `text compare`,
  `folder compare`, `format list|show|detect|tokens`, `vfs ls|stat|cat`, `gui open|ping|list|close|quit`,
  `--gui` on the compare commands, `completions` (bash, elvish, fish, powershell, zsh) and `version`; the global
  flags `--json`, `--quiet`, `--log`, `--verbose`, `--config-dir`, `--no-color`.
- Exit codes per CLI-004 and contract §2.21: usage errors and caught panics exit 100 (never 2 or 101), a broken
  pipe keeps the computed code; `gui open --wait` returns the view's code, 102 when it cannot wait, 103 without
  a GUI executable (D-01, D-02, D-03, D-22, D-23, D-35).
- One JSON document per invocation on stdout (also on error, `hikasa.usage_error.v1` for a command line that does
  not parse); progress and logs on stderr; `--output` reports written atomically (D-29, D-32, D-40).
- Vocabulary words are parsed by the core crates only; `gui open` uses the shared legacy parser (D-51, D-57).
- Integration tests for every subcommand with goldens for every JSON schema. The launch and wait paths run in
  the `cli_gui_launch` test target (`harness = false`), whose own binary is the scripted GUI when
  `HIKASA_TEST_FAKE_GUI=1`; no test GUI is built or installed as a program.
- Gates G1 and G2 through the binary (`cargo test --release -p hikasa-cli --test perf_gates -- --ignored`;
  not run in CI). Measured 2026-10-25 (Ryzen 7 7800X3D, release): G1 (2 × 1M lines, `--report summary`)
  0.43 s auto, 0.45 s standard, 0.36 s identical, peak RSS 298 MiB (bound 427 MiB; threshold 3 s); G2 (100k files
  per side, `--content none`) 0.05 s warm (threshold 1.5 s) and 0.694 s cold with dropped caches, measured
  2026-10-26 on btrfs over NVMe (threshold 5 s; `docs/12` §4).
- Wave-5 review fixes: `gui open -- -x a.c` reaches the shared parser with its `--` (D-51); `--json` counts
  only before `--`, a legacy `-json` selects the usage-error document when the `gui open` arguments do not
  parse, and that document keeps clap's whole message; `--version` prints `hikasa <version>` after a subcommand
  too (with `--json` the version document); a global value flag repeated with different values at different
  positions exits 100; `quick --crc|--binary` on an unreadable file reports the error on stderr and with
  `ok: false` in JSON; one error line per failed stdout write; the completions golden checks the field order.
- `hikasa-session`: a GUI executable that exists but cannot be started exits 103 (`ApiError::GuiNotStartable`,
  was 100 "ipc I/O"); `find_gui_exe` resolves a relative `HIKASA_GUI_EXE` against the current folder and skips
  candidates without an execute bit.
- `hikasa-vfs`: writing to a symbolic link replaces the file it points to and keeps the link; a FIFO or a
  device (`/dev/null`) is written to directly and never replaced (`--output`; contract §1.2.7 item 12).
- `docs/12-cli-and-scripting.md`: the M1 flag set, the wave-5 decisions and the measured gates; contract
  §1.9.3 (accepted additions and deviations of wave 5), §1.9.2 and §6.1 amended; `docs/11` §1 local writes.

### Wave 4 — reports and the session service
- `hikasa-report`: text reports (the annotated unified diff, normal, context and unified patches, side-by-side
  text and self-contained HTML, the summary, the statistics CSV) and folder reports (status list, side-by-side
  text and HTML, summary, the `hikasa.folder_compare.v1` JSON document); patches are byte-exact in every style
  (checked with GNU `patch` and `git apply`), with BOM and encoding warnings, escaping, grapheme-aware column
  widths and RPT-004 headers.
- `hikasa-session`: host capture, configuration and cache folders, options, `FormatStore` (built-ins, user
  formats, `order.toml`), the service API shared by the CLI and the GUI (requests, jobs, the panic boundary),
  IPC protocol v1 with framing, client and server, one endpoint per user and configuration, and the
  architecture test (checks 1–7).
- Review fixes (78 findings from break, conformance and consumer reviews; 71 fixed with tests, 7 rejected with
  reasons, contract §1.8.7): the IPC client verifies the socket folder's owner and permissions, dropped view
  waiters close as `abandoned`, shutdown releases waiters, writes never block the GUI, per-call deadlines,
  bounded frames and queues; `--config-dir` and the endpoint override reach a launched GUI (`HIKASA_GUI_EXE`
  for tests); a private, symlink-safe standard-input spool; pool panics never abort; the effective
  ignore-unimportant setting reaches folder JSON and reports; adapter API for the request ↔ view mapping, lazy
  standard input, vocabulary parsers, the D-23 close rules and escaping helpers.
- `hikasa-core`: `WarningCode::PatchEncoding`. Workspace at the end of the wave: 983 tests.

### Wave 3 — text and folder comparison
- `hikasa-text`: `TextDocument` (chunked, cancellable decoding; encodings, line endings, size limits), text
  rules, the compare engine (an identity fast path equal to the general path for every grammar, bounded Auto
  alignment, closeness pairing with work budgets), importance classification of differences, sections,
  statistics, display filters, the thumbnail, a progressive observer, and quick compare by rules that agrees
  with the full comparison (external conversions on every path).
- `hikasa-folder`: folder rules and criteria, a parallel cancellable scanner with name-only prefilter and
  re-stat, name alignment, the live `FolderModel` with O(1) counters, states and aggregates, display and folder
  display filters, flatten mode, symbolic link loop handling, content tests within a byte budget through
  injected comparators, and verdicts.
- Adversarial and conformance reviews per crate; accepted deviations in contract §1.5.7 and §1.6.8. Gates G1,
  G2 (warm), G4 and G5 pass. Workspace at the end of the wave: 724 tests.

### Wave 2 — file access, alignment and file formats
- `hikasa-vfs`: entry model, the `Vfs` trait, `LocalVfs`, `MemoryVfs` with fault injection, standard-input and
  clipboard blobs, the router, name-only prefilter and re-stat, streaming and memory-mapped CRC32 and binary
  comparison with scoped parallel chunks (results independent of the thread count). The memory-map module is
  the only `unsafe` code in the libraries.
- `hikasa-diff`: interner, the Unaligned, Standard, Myers, Histogram and Patience alignment engines, a bounded
  Standard fallback (`alignment_degraded`), closeness matching with cell budgets, never-align, inline
  differences and helper-thread cancellation.
- `hikasa-formats`: the format model and its TOML form, the grammar tokenizer (linear on long lines,
  cancellable inside a line), regular-expression engine selection, the registry with ordering and shadowing,
  21 original presets, encoding and binary detection, Sorted and External conversions.
- `hikasa-core`: `WarningCode::AlignmentDegraded`. Every crate had an adversarial and a conformance review;
  accepted deviations are recorded in the contract. Workspace at the end of the wave: 519 tests.

### Wave 1 — `hikasa-core`
- Sides, spec parsing and canonical form (local, standard input, clipboard, remote and HTTP schemes), error
  kinds and classification, cancellation tokens, progress sinks with per-phase throttling, job ids,
  `FileTime` (strict RFC 3339), case folding, the file-mask engine and `NameFilter`, the content comparator
  contract, warnings and exit codes (ARCH-020, VFS-010, FC-050, FC-051, FC-021, CLI-004).
- The architecture test (checks 1, 2, 4, 5), property suites, golden tables and the `mask_match` benchmark.
  Review: 22 findings from two reviewers, 20 fixed with regression tests, 1 rejected per contract §1.1.3.

### Preparation
- Workspace dependencies, lints (`unsafe_code` denied, `missing_docs`), MSRV 1.97 and the CI workflow (fmt,
  clippy and tests on Linux, Windows and macOS, cargo-deny).
- `docs/design/m1-contracts.md`: the binding M1 API contract (crate APIs, normative algorithms, JSON and IPC
  shapes, concurrency and error model, test plan, wave acceptance criteria, decision register).

## 0.0.0 — 2026-10-19
- Initial specification and workspace skeleton. No functionality.
- Restructured workspace into three layers (`core/`, `cli/`, `gui/`); CLI-first rule:
  every function is a `hikasa` subcommand, the GUI is reached via `hikasa gui` (IPC).
