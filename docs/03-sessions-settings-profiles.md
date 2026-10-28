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
