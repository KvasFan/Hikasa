# Sessions, Workspaces, Settings and Profiles

## 1. Session lifecycle

- **SES-001** (P0): A session is created from the Home view (new session of a type,
  with the type's *defaults* as template), from the command line, from drag & drop,
  from a parent folder session (child session), or by opening a saved session.
- **SES-002** (P0): Sessions are auto-saved when a view closes (and periodically while
  open) into an "Auto-saved" tree grouped by day (Today, Yesterday, 2–6 days ago,
  More than 6 days ago). A program option limits the number kept (default 200; 0
  disables). Named sessions are never auto-deleted.
- **SES-003** (P0): "Save Session" stores the session under its name; "Save Session
  As…" prompts for a name and an optional session folder. Names are unique within a
  folder; the dialog offers existing names for overwrite.
- **SES-004** (P0): A named session can be **locked**. A locked session can be used,
  but changes to specs, rules or display filter are not persisted; on close the user is
  asked "Save changes to locked session?" only if they explicitly chose Save.
- **SES-005** (P0): When a named session's primary parameters (specs, rules) change,
  closing the view prompts "Save changes to session?" (unless locked → see above).
- **SES-006** (P0): Session folders organise named sessions in a tree; folders can be
  created, renamed, deleted (with contents) and sessions dragged between them.
- **SES-007** (P1): A *shared sessions file* (a settings package) can be configured;
  its sessions appear in a read-only "Shared" branch.
- **SES-008** (P0): "Clear Session" replaces the view content with an empty session
  of the same type. "Swap Sides" exchanges left and right specs (and everything that
  is side-specific in view state).
- **SES-009** (P1): Folder views keep a navigation history of base-folder pairs
  (Back / Forward).
- **SES-010** (P0): Session display filter state is saved with the session except
  when loading a session from a script (script loads use the script's filter state).

*M2 interpretation (contract `docs/design/m2-contracts.md` §1.2, D-80…D-84, D-90, D-91):*

- **Types and files (D-80, D-81).** The session types of M2 are `folder-compare`, `folder-sync`,
  `text-compare`, `text-edit`, `table-compare`, `hex-compare` and `picture-compare`; the P1 types (`text-merge`,
  `folder-merge`, `media`, `version`, `registry`, …) are refused as `unsupported`. A session is one TOML file
  (§5.3). A session is named by its folders and name joined by `/` (`work/api-v2`); names and folder names
  have 1–100 characters, none of `/ \ : * ? " < > |` or control characters, no leading or trailing space or
  dot, do not start with `_`, are not a Windows device name, and are unique per folder whatever their case
  (SES-003, SES-006: a save or rename onto another case of an existing name is `already_exists`).
- **New and clear (SES-001, SES-008).** A new session copies the defaults of its type (`_defaults/<type>.toml`,
  else the built-in defaults) without lock, favorite, parent or overrides; Clear Session is a new session of the
  same type; Swap Sides exchanges specs, encodings, the parent's relative paths, pair overrides and every `view`
  key ending in `_left`/`_right`.
- **Auto-save (SES-002, D-83).** One file per open view: the first save creates
  `_autosaved/<YYYYMMDD-HHMMSS>-<n>.toml` (local time, `n` the first free counter), later saves of the view
  overwrite it; `saved_at` is RFC 3339 with the local offset. Groups (Today, Yesterday, 2–6 days ago, More than
  6 days ago; a future date counts as Today) come from `saved_at` against the local date of the injected clock.
  After each auto-save the newest `max_auto_saved_sessions` (OPT-019) are kept by `saved_at`, then by name; 0
  saves nothing and removes every auto-saved file. Named sessions are never removed.
- **Lock and close prompts (SES-004, SES-005, D-84).** `dirty` means that specs or rules differ from the loaded
  file (`view` and `display` do not count). Closing an unnamed or auto-saved view auto-saves it (nothing when
  auto-save is off); closing a named one writes its view state silently when it is not dirty, asks "Save changes
  to session?" when it is, and for a locked session writes nothing unless the user chose Save, which asks "Save
  changes to locked session?". A locked session file is never written by auto-save or by close.
- **Locked sessions and other commands (SES-004, D-91).** `hikasa session edit`, `delete` and `rename` refuse a
  locked session unless `--force`; `settings import` does not replace or delete one, and `settings
  restore-defaults --delete-sessions` does not delete one, unless `--force` (the plan shows `skip_locked` and
  the warning `session_locked` names the session).
- **Stored rules (D-82, D-90).** A session file holds the rules that differ from the defaults of its type, plus
  every rule it already held: a value chosen explicitly stays written even when it equals the defaults, so a
  later change of the defaults does not change it. `hikasa session edit --unset <key>` removes a rule, and the
  session follows the defaults for it again. The optional folder rules (time offset, size and date filters) are
  written `false` to mean "off" where the defaults set them. Removing an entry of a map that the defaults hold
  (such as one grammar element's importance) comes with the session settings dialog (W4).
- **Display filter (SES-010).** `[display]` is stored with the session; a session loaded by a script (or opened
  over IPC) has it cleared, so the script's filter state applies.
- **Specs that cannot be stored.** `-` (standard input) and `clipboard://` are never stored; the side stays
  empty, with the warning `spec_not_saved`.

## 2. Session settings dialog (common contract)

- **SES-020** (P0): Every session type has a Session Settings dialog organised in
  tabs (see the per-view documents for tab contents). The first tab is always
  **Specs** (the inputs, an optional description, and for file sessions a "Disable
  editing" switch per session).
- **SES-021** (P0): The dialog's footer has a scope selector:
  - *Use for this view only* (default for unnamed sessions),
  - *Update session defaults* (writes to the type's Defaults),
  - for child sessions additionally: *Use for these files within parent session* and
    *Use for all files within parent session* (stored in the parent folder session as
    per-pair or per-type overrides).
- *M2 interpretation of SES-021 (D-82):* the effective rules of a session are the defaults of its type → for a
  child session, the parent's `type` overrides for the child's type → the parent's `pair` override matching the
  child's relative paths → the child's own changes. "Update session defaults" writes the rules (never the specs)
  to `_defaults/<type>.toml`; the two parent scopes store an override in the parent (a named or auto-saved folder
  session), sparse against the defaults of the child's type; a locked parent keeps the override in memory only.
- **SES-022** (P0): A "Rules" toolbar button opens the dialog directly on the
  comparison/importance tab; "Format" opens the file-format editor for the current pane.
- **SES-023** (P1): Settings pages have a search box that filters visible settings.

## 3. Home view and session management

- **SES-030** (P0): The Home view shows: large buttons for each session type (drop
  targets for files/folders), the Sessions tree (New → Defaults per type; named
  sessions in folders; Auto-saved; Shared), a search box filtering the tree, and a
  **Session Management panel** that edits the selected session's settings in place
  (Save, Save As…, Factory Defaults, Cancel). The panel can be pinned, hidden or
  maximized. A **Web Resources** panel (links to documentation, community) can be shown
  or hidden.
- **SES-031** (P0): Tree context menu: Open, Open as New (copy), Open Sessions (all
  in a folder), New (session or folder), Clear Contents, Delete, Save As…, Rename,
  Edit, Expand All, Collapse All, Favorite (pins to the top).
- **SES-032** (P1): "Sticky session selection" option: the Home view re-selects the
  last session picked from Home instead of the last session updated anywhere.
- **SES-033** (P1): Incremental search in trees and lists (type-ahead) with a
  configurable reset delay.

## 4. Workspaces

- **SES-040** (P0): A workspace = all open windows and tabs with their sessions,
  window geometry, active tab, per-view layout. "Save Workspace As…" names it (or
  overwrites an existing one); "Load Workspace" closes all views and restores it;
  "Manage Workspaces" renames/deletes and assigns a keyboard shortcut to a workspace.
- **SES-041** (P0): Startup options "Load workspace" and "On exit, save workspace as"
  (typically the same name, e.g. `last`) implement "reopen what I had".
- **SES-042** (P0): A workspace name can be given on the command line.
- *M2 interpretation (D-85, D-88):* a workspace stores, per window, its geometry (as `state.toml` `[window]`)
  and active tab, and per tab either a named session reference or an inline session table (an unsaved or
  auto-saved view) plus its view layout; an optional `shortcut`. Loading closes the open views and opens the
  tabs (lazily); a tab whose session no longer exists opens Home, with the warning `session_missing`. On the
  command line, `hikasa gui workspace <name>` and `hikasa gui open <name>` (one argument that is neither an
  existing path nor a session) load a workspace (`12-cli-and-scripting.md` §1).

## 5. Settings storage

- **SES-050** (P0): Location: `$XDG_CONFIG_HOME/hikasa` (Linux),
  `%APPDATA%\Hikasa` (Windows), `~/Library/Application Support/Hikasa` (macOS).
  **Portable mode**: if a file named `hikasa-portable.toml` (may be empty) exists next
  to the executable, all settings live in that directory.
- **SES-051** (P0): Files (all TOML, UTF-8):
  - `options.toml` — program options (`15-options-reference.md`)
  - `state.toml` — window positions, MRU lists, last used session, hint/first-run flags
  - `commands.toml` — per-view-type shortcut/menu/toolbar customizations
  - `theme.toml` — colours and fonts
  - `sessions/<folder>/<name>.toml`, `sessions/_defaults/<type>.toml`, `sessions/_autosaved/…`
  - `formats/<name>.toml` + `formats/order.toml` (ordered list + enabled flags)
  - `profiles/<name>.toml` (no secrets; secrets go to the OS keychain, see §7)
  - `workspaces/<name>.toml`
  - `filters.toml` — name-filter presets
  - `openwith.toml` — "Open With" external applications
  - `archives.toml` — archive type masks
- **SES-052** (P0): Every file carries `schema_version`. Loading newer-major files
  fails with a clear message; older files are migrated in memory and written back on
  next save.
- **SES-053** (P0): Writes are atomic (temp file + rename). Settings changed by another
  running instance or by hand are detected (file watch) and reloaded with a notice
  ("Session changed on disk. Reopen?").

### 5.1 M1 implementation (binding interpretation; contract §1.8.2, §2.12.4, §2.22; D-21, D-25, D-31, D-49)

**Configuration folder.** Chosen in this order (`hikasa_session::resolve_config_dir`):

1. `--config-dir <dir>` (made absolute against the current directory; `.` components are
   dropped);
2. portable mode: the folder of the executable, when `hikasa-portable.toml` exists there;
3. the per-user default: `$XDG_CONFIG_HOME/hikasa` when `XDG_CONFIG_HOME` is an absolute
   path, else `~/.config/hikasa` (Linux and other Unix systems);
   `~/Library/Application Support/Hikasa` (macOS); `%APPDATA%\Hikasa` (Windows). Without
   the needed home or application-data folder the command fails ("cannot determine the
   configuration folder").

A configuration folder that equals the per-user default keeps the default IPC endpoint;
any other folder gets its own endpoint and single-instance lock (D-47,
`01-architecture.md` §5.1).

**Cache folder.** Next to explicit and portable configurations it is `<config>/cache`.
For the per-user configuration it is `$XDG_CACHE_HOME/hikasa` (absolute values only),
else `~/.cache/hikasa` (Linux and other Unix systems), `~/Library/Caches/Hikasa`
(macOS), `%LOCALAPPDATA%\Hikasa\cache` (Windows); when the needed folder is unknown,
`<temp>/hikasa-cache-<uid>` (a per-user name). M1 uses it for one thing: `hikasa gui open -`
spools standard input to `<cache>/stdin/<n>/<name>` (`<name>` is the file name of
`--stdin-name`, else `stdin.txt`; `<n>` is the first free number, so concurrent
invocations never share a file) and sends that absolute path to the GUI, which has
another current directory and no access to the CLI's standard input; with `--wait` the
file and its folder are deleted after the view closes (D-49). Piped content is often
private, so on Unix the cache folder must belong to the user and must not be writable by
others (the temporary-folder fallback must also be a real folder, not a symbolic link),
`stdin/` and `<n>/` are private folders (mode 0700, `<n>` always created fresh), and the
file is created new with mode 0600, never through a symbolic link; a spool that fails
leaves nothing behind. Standard input is read only when a `-` argument is actually used
(`format detect -` needs only the `--stdin-name` hint and never reads it).

`--config-dir ""` is an error (an unset shell variable must not turn the current
directory into the configuration folder).

**Read-only commands never create either folder.** Missing files mean defaults.

**`options.toml` (D-31; revised by D-71 in M2, see §5.2).** A missing file means defaults. A `schema_version` newer than
this build supports is an error (SES-052); an older one is read as it is. Unknown keys
inside the top-level sections of `15-options-reference.md` (`startup`, `tabs`,
`appearance`, `text_editing`, `next_difference`, `backups`, `file_operations`,
`archive_types`, `commands`, `open_with`, `tweaks`) are ignored silently, because M2 writes
the whole file and M1 reads only a few keys; with `--verbose` every ignored key is logged
(a log line, not a warning). An unknown top-level section (usually a typo) produces the
warning `unknown_config_key`. A TOML syntax error, a value of the wrong type or range in
a key that M1 reads, a known section written as a plain value (`text_editing = 5`) and a
negative `schema_version` are errors. Keys match the table structure: a quoted key with
dots (`"text_editing.number_of_context_lines" = 9`) is one unknown key, not the option. M1 never writes `options.toml`, so every key it does not
use is preserved. The keys read in M1 are listed in `15-options-reference.md` ("Keys read
in M1"); among them `[tweaks.file_operations] content_memory_budget_mib`, the memory budget
of content tests in folder comparisons (D-25).

**File formats (SES-051, FF-004).** The registry is the built-in formats in their
canonical order, merged with `formats/*.toml` (sorted by file name; `order.toml` is not a
format). A user format whose id equals a built-in's replaces it in place; a new id is
appended (sorted by id). A user file that cannot be read or parsed, whose id is not its
file name, whose mask does not compile, or whose id another user file took, is skipped
with the warning `user_format_skipped` — the built-ins and every other format stay usable,
and `format list --json` shows the warnings. Unknown keys in a format file produce
`unknown_config_key`. `formats/order.toml` (`schema_version = 1`, then `[[format]] id = "…"
enabled = true|false`) lists the ids first, in its order (unknown ids warn), followed by
the unlisted formats; its enabled flags override the defaults. An id listed twice, or an
`order.toml` from a newer version, is an error. Read-only commands never write the
built-ins into the configuration folder (D-21); that happens only on the GUI's first start
and with the `format` write commands of M2, and never overwrites an existing file.

**Atomic writes (SES-053).** Every configuration and report file is written to a
temporary file in the same folder (created exclusively), flushed to disk and renamed over
the target; on any failure the target is untouched and the temporary file is removed.

### 5.2 The settings store (M2; contract `docs/design/m2-contracts.md` §1.1; D-70…D-78)

**One engine (D-70).** Every program-owned settings file — `options.toml`, `state.toml`, `commands.toml`,
`theme.toml`, `themes/<name>.toml`, `filters.toml`, `openwith.toml`, `archives.toml` — is read and written by
the same engine (`hikasa_session::settings::SettingsFile`):

- **Versions (SES-052).** `schema_version` absent means the current version (1); a negative or non-integer
  value is an error; a newer value is an error for every command. An older file is migrated in memory, step by
  step, and written back in the current version on its next save (never at load). `schema_version = 0` reads as
  version 1.
- **Unknown keys (Q11).** Keys this version does not know are kept and written back with their values (comments
  and layout are not kept). An unknown top-level section warns `unknown_config_key`; unknown keys inside known
  tables are silent (`--verbose` logs them). Unknown keys inside `[[app]]` and `[[preset]]` entries stay with
  their entry. *Interpretation:* `toml` is used without its order-preserving feature, so kept unknown keys are
  written in sorted order after the known keys of their table, and unknown sections after the known ones.
- **Sparse writes.** A key equal to its default is not written, so a later change of a default reaches every
  user; an empty table is not written; a file that would hold only `schema_version` is not created. Known keys
  are written in the order of the schema. Entries of maps and arrays of tables (`[view.<v>.command.<id>]`,
  `[[app]]`, `[[preset]]`) are written sparsely against the defaults of a new entry. Writes are atomic (SES-053).
- **Strict and lenient reading (D-71, revises D-31).** `options.toml` is read completely through the typed model
  of `15-options-reference.md` ("Keys"). A value of the wrong type or out of range is an error that names the
  file and the key, for every command except `hikasa settings`, which reads leniently: the bad value is reported
  as warning `invalid_config_value` and replaced by its default, so `hikasa settings set` repairs the file (any
  save writes the repaired values). The entries of `[[app]]` and `[[preset]]` are never dropped or reordered: a
  bad value inside one is replaced by its default, and an entry that fails a check as a whole (an empty command)
  is kept as it is, with the warning.

**`state.toml` (SES-051).** The program's own memory, not options:

```toml
schema_version = 1
last_session = ""                        # name of the session last used (W2)
[mru]                                    # each list newest first, capped at max_mru_items on push
left = []
right = []
center = []
folders = []
files = []
find = []
replace = []
name_filters = []
workspaces = []
[window]                                 # the last main window (a workspace stores its own)
width = 1200                             # 200..=32767
height = 800                             # 200..=32767
x = -1                                   # -1..=32767; -1: the platform places the window
y = -1
maximized = false
[flags]
first_run_done = false                   # set by the GUI's first start (the D-21 materialization)
hints_seen = []                          # ids of the one-time hints already shown
[dialogs]                                # free tables per dialog id ("remembers last choices"), kept as they are
```

Pushing a value to a list moves an equal entry (same text) to the front and keeps at most
`[tweaks.metrics] max_mru_items` entries.

**Change detection (D-74, SES-053).** A file's stamp is its length, its modification time and the CRC-32 of its
bytes; a missing file has the stamp "absent". `ConfigWatch::poll` compares the stamps of the files it tracks
(every settings file, every scheme file and every `formats/*.toml`) and reports created, changed and removed
files; it holds no timer: the window polls on focus and every 2 s. *Interpretation:* core code does not read
the clock, so the CRC is computed at every poll that finds an unchanged length and time (the files are small);
an edit that keeps the length and the time is still found. A save first compares the stamp on disk with the
stamp of the load: a file changed in between is not overwritten (error, exit 100) unless
`hikasa settings set --force`.

**Keys and the `settings` command (D-73).** `hikasa settings get|set|unset|list` address every value as
`<file>.<path>` (`options.text_editing.auto_indent`, `openwith.app.0.command`, `themes.dark.folder.background`);
see `12-cli-and-scripting.md` §4. `set` refuses a value of the wrong type, out of range or failing a check
(an unknown command id or view in `commands.toml`, a shortcut that does not parse, a duplicate preset name, an
Open With command that does not split) and leaves the file as it was. Setting a value equal to its default
removes the key.

**Restore defaults (D-77; the single-file part of SES-062).** `hikasa settings restore-defaults (--all |
--file <file>…)` deletes settings files so that the next load sees defaults. It is a plan: `--dry-run` prints
it (`delete <path>` / `absent <path>` per file); without `--dry-run` it needs `--yes`. `--file themes` means every
scheme file. Sessions, workspaces and file formats are not touched (`hikasa format restore` restores formats).

### 5.3 Session, workspace and package files (M2; contract §1.2; D-80…D-87)

All three are read and written by the D-70 engine of §5.2 (versions, unknown keys kept, sparse atomic writes).

**Sessions.** `sessions/<folder path>/<name>.toml` (named), `sessions/_defaults/<type>.toml` (one per type),
`sessions/_autosaved/<id>.toml`:

```toml
schema_version = 1
type = "folder-compare"        # always written
description = ""
locked = false
favorite = false               # SES-031 Favorite
saved_at = ""                  # RFC 3339; auto-saved files only
[parent]                       # TC-052 child sessions: session = "<ref>" or "_autosaved/<id>", left/right relative paths
[specs]                        # left, right (text-edit: left only), format, left_encoding, right_encoding
[rules]                        # folder-compare: the TOML of --rules-file; text types: TextRules; others: kept as is
[text_rules]                   # folder types: rules of rules-based content tests
[display]                      # display filter state (SES-010)
[view]                         # view layout and state
[[override]]                   # folder types: scope = "pair" | "type", left, right, type, [override.rules]
```

`[rules]`, `[text_rules]` and `[override.rules]` hold only what differs from the defaults of the type **at save
time**, so a later change of the defaults reaches every session that did not change that rule. `hikasa session
edit --set` addresses keys of this file (`rules.criteria.size=false`); `type`, `locked`, `saved_at` and
`override` are not set that way.

**Workspaces.** `workspaces/<name>.toml` (names as session names, no folders):

```toml
schema_version = 1
shortcut = ""
[[window]]                     # width, height, x, y, maximized, active_tab
[[window.tab]]                 # session = "<ref>"  or  [window.tab.inline] (a session table without schema_version)
[window.tab.view]              # the tab's layout
```

A tab has exactly one of `session` and `inline`, and `active_tab` names one of the tabs (lenient loading resets
it to 0).

**Settings packages.** A `.hikasa-pkg` is a zip (stored or deflate, no encryption) with `manifest.toml` and the
files at their configuration-relative paths (§6).

## 6. Import, export, restore defaults, support package

- **SES-060** (P0): **Export Settings** wizard: choose categories — Program Options;
  Theme, Colors, Fonts; Toolbars, Shortcuts, Menus; Program State; Sessions (pick
  all/some); File Formats (pick); Profiles (pick); System Info (for support) — then
  writes a **settings package** (`.hikasa-pkg`, a zip containing the TOML files and a
  manifest).
- **SES-061** (P0): **Import Settings** wizard: pick a package, choose categories,
  then pick the named items; items whose names collide are shown in red (they will be
  replaced). Options "Delete all existing sessions / file formats / profiles" before
  import. A summary ("12 sessions will be imported, replacing 3 existing") precedes
  Finish. Importing a package given on the command line with `--silent` imports
  everything.
- **SES-062** (P0): **Restore Factory Defaults** reuses the same wizard against the
  built-in defaults.
- *M2 interpretation (D-86, D-87):* the categories of a package are `options` (`options.toml`, `filters.toml`,
  `openwith.toml`, `archives.toml`), `theme` (`theme.toml`, `themes/*.toml`), `commands`, `state`, `sessions`
  (named sessions and `_defaults`, per item; never `_autosaved`), `formats` (per item, and `order.toml`),
  `workspaces` (per item) and `system_info` (`system-info.txt`: version, platform, configuration mode and folder
  with the home folder written `~`, item counts; exported only when asked, never imported); `profiles` is
  reserved (P1). `manifest.toml` holds `schema_version`, `format = "hikasa-pkg"`, `hikasa_version`, `created`
  (the injected clock, RFC 3339) and one `[[item]]` (`category`, `name`, `path`) per file. Export writes the
  zip atomically with entries sorted by path and the `created` time, so the same configuration exported at the
  same instant gives the same bytes. Import is a plan (T12): every entry is validated first — manifest and
  entries agree, each path is allowed for its category, no `..`, absolute or backslash paths, at most 10,000
  entries, 16 MiB per entry and 64 MiB in total uncompressed, each file parses with its schema and none is newer
  than this version (`unsupported`; anything else `invalid_data` naming the entry), session rules and text rules
  (also of overrides and of inline workspace tabs) build and their regular expressions compile, no two entries
  differ only by case, and every manifest category is known — and nothing is written when anything fails. Then the deletes ("delete all existing sessions / file formats / workspaces") run, and the
  items are written one by one with a result each (a failure is recorded and the others continue; exit 100).
  A collision is an item whose target exists, compared without regard to case (the existing spelling is kept):
  it is replaced — except a locked named session, which is kept (`skip_locked`, warning `session_locked`, not
  counted) unless `--force`; `--delete-existing sessions` does not delete locked sessions either (D-91). The summary counts added, replaced and deleted
  per category. `--silent` imports every category and item without deletes (`hikasa settings import --silent`,
  `hikasa gui open <file.hikasa-pkg> --silent`).
- *M2 interpretation of SES-062 (D-87):* restoring factory defaults is the same plan with deletes only: per
  chosen category the user files are deleted (the settings files, the scheme files, `sessions/_defaults/*.toml`,
  every format file and `order.toml`); named sessions and workspaces only with the explicit
  `--delete-sessions` / `--delete-workspaces` choices (`hikasa settings restore-defaults --category …`); a
  locked named session is kept with the warning `session_locked` unless `--force` (D-91).
- **SES-063** (P1): **Support** dialog builds a support package (settings + system
  info + logs) and offers "Explore settings folder".

## 7. Profiles (remote services)

- **SES-070** (P1): Profiles are named connection definitions: FTP (incl. FTPS,
  SFTP), WebDAV, S3-compatible, Subversion (read-only), cloud drives (OAuth: Dropbox,
  OneDrive — P2). A `<default>` FTP profile supplies defaults inherited by named FTP
  profiles.
- **SES-071** (P1): The Profiles dialog lists, creates, clones, edits, deletes profiles;
  each profile type has its settings pages (`11-vfs-remote-archives.md` §5).
- **SES-072** (P1): Passwords are stored only if "Save password" is checked, and then
  in the OS keychain (`keyring`); never in TOML. An admin policy `disable_saved_passwords`
  removes the checkboxes and keeps passwords in memory only.
- **SES-073** (P1): "Browse Using Profile" dialog: pick Local File System, Quick
  Connect (FTP/Subversion with ad-hoc credentials), or a named profile, then browse.

## 8. Admin policies

- **SES-080** (P1): A system-wide policy file (`/etc/hikasa/policy.toml`,
  `%ProgramData%\Hikasa\policy.toml`, `/Library/Application Support/Hikasa/policy.toml`)
  with boolean keys: `disable_check_for_updates`, `disable_remote_profiles`,
  `disable_saved_passwords`. A per-user policy file is honoured as well. Policies hide
  the related UI and refuse the feature at runtime.

## 9. Tabs and windows

- **SES-090** (P0): Multiple windows; each window has a tab bar. Tabs are drag-
  reorderable and draggable between windows; "Move Tab to New Window"; "Duplicate
  Tab"; "Close Other Tabs"; "Merge All Windows" (P1).
- **SES-091** (P0): Options: open new sessions in a new tab or window; open child
  sessions in a new tab or window; open new tab next to active tab; closing last tab
  closes window; hide tab bar if single tab; warn when closing multiple tabs/windows;
  (Windows) show tab previews in taskbar.
- **SES-092** (P0): Closing a view with unsaved edits prompts per file (Save / Discard
  / Cancel).
- **SES-093** (P0): "Home" command (Alt+Home) replaces the current view with the Home
  view; "Sessions" dropdown in the toolbar lists sessions openable in this view.
