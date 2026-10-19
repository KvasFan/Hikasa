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
