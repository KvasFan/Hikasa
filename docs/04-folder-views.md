# Folder Views: Folder Compare, Folder Merge, Folder Sync

The three folder session types share one engine (`hikasa-folder`) and one tree
widget. Differences: Folder Merge has three input sides plus an output; Folder Sync
replaces free-form actions with a planned operation per item.

## 1. Specs

- **FC-001** (P0): Two base-folder specs (left, right); Folder Merge adds Center
  (optional ancestor) and **Merge to** (output: Left, Right or Other path); Folder Sync
  has left and right. Any side may be local, archive, snapshot or remote
  (`11-vfs-remote-archives.md`).
- **FC-002** (P0): Path edits above each pane accept typed paths with completion and
  an MRU dropdown; a Browse button opens the folder picker (local or "Browse Using
  Profile"); Enter accepts. Typing a non-existent folder offers to create it.
- **FC-003** (P0): "Disable editing" per session makes every file operation and
  child-session editing read-only.
- **FC-004** (P0): "Up One Level" (Left/Right/Both/All), "Set as Base Folder(s)",
  "Set as Base on Other Side", "Compare Parent Folders", "Merge Base Folders",
  "Sync Base Folders", "Compare Base Folders" (from Sync), "Compare to Output"
  (from Merge: left/center/right vs output) open new sessions or re-root the current one.

## 2. Scanning and alignment

- **FC-010** (P0): Entries of both sides are aligned by name. Options: case-sensitive
  names (per back-end capability, default insensitive on Windows/macOS, sensitive on
  Linux and remote unless the profile says otherwise), **Align filenames with different
  extensions** (`x.txt` ↔ `x.csv`), **Align filenames with different Unicode
  normalization forms** (NFC ↔ NFD).
- **FC-011** (P1): **Alignment overrides** (session Misc tab): rules "align left mask
  with right mask", DOS wildcards or replacement-style regular expressions
  (`(.*)\.gif` ↔ `$1.png`), optionally limited to a relative folder. Manual **Align
  With** (F6): pick an item, then click the counterpart; items' parents must already be
  aligned.
- **FC-012** (P0): Folder handling options: *Automatically scan subfolders in
  background*; *Automatically scan top-level orphan subfolders* (otherwise top-level
  orphans are coloured without scanning); *Expand subfolders when loading session*;
  *Only expand subfolders with differences*. Unscanned folders render "hollow";
  folders not yet opened when auto-scan is off render yellow; folders that failed to
  open carry a red X glyph; symlinks/junctions carry an arrow glyph.
- **FC-013** (P0): **Follow symbolic links** (treat links as their target for
  kind/size/time/attributes) — default off.
- **FC-014** (P0): **Ignore Folder Structure (Flatten)**: compare all files as if in
  one folder (aligned by file name only, path shown in a Path column).
- **FC-015** (P0): Archive handling: *As files always* / *As folders once opened*
  (default) / *As folders always*. Archives show a zipper icon and expand on
  double-click; nested archives are supported.
- **FC-016** (P0): Scanning is cancellable (Stop button), incremental, and
  parallel across sides and subfolders with a bounded worker count per back-end
  (remote back-ends use their profile's connection limit).
- **FC-017** (P1): **Automatic refresh** every N seconds/minutes (session option);
  **Refresh** (F5) rescans open folders; **Full Refresh** (Ctrl+F5) rescans everything;
  **Refresh Selection** (Shift+F5) rescans selected items. Local back-ends also use
  file-system watching to mark stale folders.

## 3. Comparison criteria (Rules → Comparison tab)

Quick tests (no file content needed):

- **FC-020** (P0): Compare file size.
- **FC-021** (P0): Compare timestamps with a second tolerance (default 2 s); *Ignore
  daylight saving difference* (exactly 1 h apart = same); *Ignore timezone
  differences* (any exact multiple of 1 h = same).
- **FC-022** (P0): Compare filename case.
- **FC-023** (P1): Compare file attributes: selectable subset of DOS attributes
  (archive, hidden, read-only, system, compressed, encrypted, …). Default off.
- **FC-024** (P1): Compare Unix metadata: permissions, owner, group.

Tests that require reading files:

- **FC-025** (P0): Compare contents by **CRC** (CRC32; archive-provided values are
  used as-is), **binary** (byte-by-byte, configurable buffer size, memory-mapped
  where possible) or **rules-based** (through the file format engine: ignores
  unimportant differences, line endings, encoding; result is one of *binary same*,
  *rules-based same*, *unimportant differences*, *important differences*).
- **FC-026** (P0): *Skip if quick tests indicate files are the same* (only run content
  tests when quick tests differ).
- **FC-027** (P0): *Override quick test results* (content match wins over timestamp
  mismatch) — default on.
- **FC-028** (P1): Compare versions (executable version info).
- **FC-029** (P0): Content comparisons run (a) on load when rules ask for them,
  (b) on explicit **Compare Contents** (dialog: CRC / Binary / Rules-based, "Show
  results dialog" summary), (c) when a child file session loads a pair.

## 4. Comparison states and colours

- **FC-030** (P0): File states: Unknown, Same, Newer (left/right), Older, Different
  (content), Orphan (left/right), Filtered (shown only under Suppress Filters). Folder
  state aggregates its visible children: Same (black), Contains newer/different
  (red), Contains orphans (purple), Contains older (grey), mixtures, Unknown/scanning,
  Only filtered. A **Legend** window (Ctrl+Alt+L) lists all icon/colour combinations.
- **FC-031** (P0): Default colours: grey = unknown/older, black = same, purple =
  orphan, red = newer/different, teal = filtered-out item shown by Suppress Filters.
  Merge colours: teal = left change, magenta = right change, orange = mergeable,
  red = conflict. All configurable (`15-options-reference.md`).
- **FC-032** (P0): Each row shows a colour spot before the name (visible even when
  selected), the centre column shows the content-comparison result icon when a
  content test ran (binary same, binary different, rules same, similar/unimportant,
  important difference) and the comparison direction arrows.
- **FC-033** (P0): "Ignored" marks selected items so their differences count as same
  (persisted in the session).

## 5. Columns

- **FC-040** (P0): Columns per side: Name, Ext, Size (folder size = sum once scanned),
  Modified, Attributes (DOS letters `A C D E H I L M N O P R S T U Z` or Unix mode
  `-rwxr-xr-x`), CRC (computed on demand), Version (executables), Owner, Group,
  Path (flatten mode); Merge adds Result and Remaining Actions; Sync adds the planned
  action. Column visibility via View ▸ Columns or header context menu; click header
  to sort (both sides sort together), drag to reorder/resize; widths persist per
  session type.
- **FC-041** (P0): When Ext is hidden, read-only/system/hidden flags are shown as
  `(r)`, `(s)`, `(h)` after the name.

## 6. Filters

### 6.1 Name filters (file masks)

- **FC-050** (P0): Toolbar **Filters** edit: semicolon-separated masks; `-` prefix
  excludes; trailing `\` (or `/`) denotes a folder mask; dropdown of **presets**
  (editable in options). Session Settings ▸ Name Filters tab has four lists: Include
  Files, Exclude Files, Include Folders, Exclude Folders; "Clear"; "Add To Presets".
- **FC-051** (P0): Mask syntax: `?` one char, `*` any run, `[az]`, `[a-z]`, `[!az]`,
  `[[]`; trailing `.` means "no extension" (`*.` matches files without extension);
  relative masks `p\f` (f inside folder p), `.\f` (in base folder), `...\f` (any
  depth). Simple masks apply to any item whose parent is included; the base folder is
  always included; folders containing included files are included.
- **FC-052** (P0): **Exclude** action: offers to exclude the selected item(s), or all
  files of that type (`-*.ext`), adding to the session's exclude lists.

**Mask semantics (binding interpretation of FC-050/FC-051; decisions D-07, D-08, D-09).** The mask engine
is `hikasa_core::mask`, the filter evaluation `hikasa_core::filter`; their normative algorithm and test
tables are in `design/m1-contracts.md` §2.2 and §2.3.

- **Syntax (D-07).** A mask list is `;`-separated; items are trimmed of ASCII spaces and tabs, and empty
  items are ignored. A leading `-` excludes. `\` and `/` are both separators; a **trailing** separator marks a
  folder mask (folder masks never match files, file masks never match folders). `?` matches one character,
  `*` any run (including the empty run), `[az]` / `[a-z]` a set or range, `[!az]` a negated set; `]` right
  after `[` or `[!` is literal, `-` first or last in a set is literal (`[-]` matches a literal `-`), `\` and
  `/` inside a set are literal, and only `!` negates (`[^a]` is the set of `^` and `a`). Errors (rejected
  when the filter is parsed, never during a scan): an unclosed `[`, an inverted range such as `[z-a]`, an empty
  component (`a\\b`, a lone `-`), and `.` anywhere but first or `..` anywhere. The four lists of the Name
  Filters tab hold the same items without the `-` prefix (the list says whether they exclude); an item of a
  folder list without a trailing separator is taken as a folder mask. List items are trimmed like toolbar
  items, so the lists write a mask that starts with a blank as a one-character set: the toolbar item `- x`
  excludes the name ` x` (blanks after the `-` belong to the mask) and appears in the Exclude Files list as
  `[ ]x`; toolbar text and lists always describe the same filter. The canonical toolbar text lists include
  files, include folders, exclude files, exclude folders: `*.c;*.h;-*.bak;-obj\;src\` is shown as
  `*.c;*.h;src\;-*.bak;-obj\`.
- **Case and Unicode (D-07).** Case-insensitive matching uses the simple 1:1 lower-case mapping only (no
  locale tailoring, no expansion: `STRASSE` does not match `straße`; `İ`, whose full lower case has two
  characters, maps to itself); a character is in a range `[lo-hi]` when the character itself, its simple lower
  case or its simple upper case is in the range (so `[A-Z]*` matches `zeta` case-insensitively). Names and
  masks are compared in Unicode NFC: a name is normalised before it is matched and a mask before it is parsed,
  so `café.txt` typed in NFC matches a file stored in NFD, and a mask pasted from an NFD name means the same,
  including set members and range bounds (a set holding `e` followed by a combining acute accent is the set
  `[é]`, and a KELVIN SIGN inside a set is the letter `K`). Case folding is applied to the NFC characters, so
  a name and its lower-case spelling match the same masks, except for the rare names where folding changes the
  NFC composition (`I` + combining dot above is `İ`, which keeps its case) or where a character is neither
  lower case nor the upper case of its lower case (title-case letters such as `ǅ` inside a range).
- **Extensions (D-07).** A name has an extension when it contains a `.` that is neither its first nor its
  last character (`.bashrc` and `a.` have none). A mask ending in a literal `.` means "no extension"
  (`*.` matches `Makefile` and `.bashrc`, not `a.out`). A mask ending in `.*` also matches names without an
  extension (`*.*` matches every name; `abc.*` matches `abc` but not `abcd`). The same rules apply to every
  component of a relative mask.
- **Paths (D-08).** A mask without separators matches the item **name** at any depth (`*.c` matches
  `src/util/x.c`). A relative mask (`p\f`) is anchored at the base folder (`src\*.c` matches `src/a.c`, not
  `lib/src/a.c`); `.\f` matches in the base folder only; a leading separator anchors as well (`\docs\`).
  `...` is a path component that matches zero or more folders anywhere in the mask (`...\test\*.py`,
  `src\...\*.rs`); a mask that starts with `...` is not anchored.
- **Folder inclusion (D-09).** Filters are evaluated top-down, per side. Exclusion always wins: an excluded
  folder excludes its whole subtree. An include-folder mask includes the whole subtree of a matching folder
  (`src\` includes `src/a/b.c`). Folders that are only *prefixes* of an anchored include-folder mask are
  traversed but not themselves "included": with `src\core\`, the folder `src` is scanned but its files are
  excluded, while `src/core` and everything below it are included. A simple include-folder mask such as
  `src\` makes every other folder traversable in the same way (a `lib/src` could exist below it). The base
  folder is always included; its files are subject to the file masks only.

### 6.2 Other filters

- **FC-053** (P1): Rules ▸ Other Filters: list of items — *date modified* (older/newer
  than; absolute date-time or N days ago relative to midnight), *size* (smaller/larger
  than N bytes/KB/MB/GB), *content* (containing / not containing text with case, whole
  word, regex), *DOS attribute* (is set / is not set), *Unix file type* (is / is not:
  block, char, FIFO, link, regular, socket). "Exclude protected operating system
  files" (system+hidden) is on by default. When any other filter is active a comment bar
  at the bottom of the view says so and opens the editor.
- **FC-054** (P0): If a filter hides an item on one side but its counterpart is visible,
  both are shown and the hidden one is coloured teal.

### 6.3 Display filters

- **FC-055** (P0): Folder Compare: Show All; Show Differences; Show Same; Show No
  Orphans; Show Differences but No Orphans; Show Orphans; Show Left Newer; Show Right
  Newer; Show Left Newer and Left Orphans; Show Right Newer and Right Orphans; Show Left
  Orphans; Show Right Orphans; Show None (hidden by default). Toggle mode exposes the
  atomic states: Same, Left Orphans, Right Orphans, Left Newer, Right Newer,
  Differences.
- **FC-056** (P0): Folder display filters: *Always Show Folders*, *Compare Files and
  Folder Structure* (folders follow the display filter), *Only Compare Files* (hide
  empty/fully-filtered folders).
- **FC-057** (P0): *Ignore Unimportant Differences* (rules-based "unimportant" counts
  as same); *Suppress Filters* (temporarily show everything; hidden items in teal and
  operable).
- **FC-058** (P1): Toolbar display-filter mode: DropDown (3 buttons + menu), Favorites
  (a row of combination buttons), Toggles (one button per atomic state); configurable
  in options and by right-clicking the toolbar.

## 7. Selection and navigation

- **FC-060** (P0): Click selects on one side; click in the centre column selects both
  sides; Ctrl+click adds; drag-swipe selects a range (crossing the centre selects both
  sides); Shift+Left/Right restricts a selection to one side; Space marks an item for
  a two-item "Open" or "Compare To". Edit ▸ Select All, Select All Files, Select Newer
  (Left/Right/Both), Select Orphans (Left/Right/Both), Invert Selection; Expand All /
  Collapse All; Open/Close Folder; Open/Close Subfolders.
- **FC-061** (P0): Search ▸ Next/Previous Difference (Ctrl+N/P) move to the next item
  that is not "same"; Alt+Up/Down move skipping gaps; **Find Filename** (Ctrl+F; case,
  whole word, regex, wrap, search up, sides), Find Next/Previous (F3/Shift+F3).
- **FC-062** (P0): Double-click / Enter / **Open** opens the item pair in the file
  view chosen by file formats (two selected items anywhere → compare them; one item →
  with its counterpart). **Open With** submenu: a specific view type (Text, Table,
  Hex, Picture, Media, Version, Registry, Text Edit, Text Patch), the associated
  application, or a user-defined external application (`15-options-reference.md` Open
  With). **Quick Compare** shows a dialog with the content-comparison result instead of
  opening a view. **Compare To** (F7) compares two arbitrary items in a new view.
- **FC-063** (P0): "Explorer / file manager" context menu for the selected item
  (platform specific; on Linux: open containing folder, copy path), **Copy Filename**
  copies the selected paths.
- **FC-064** (P0): Status bar: counts of selected items per side, total sizes, and the
  comparison state summary; **Folder Compare Info** (Ctrl+I) dialog shows statistics
  (files/folders per state).

## 8. File operations (Actions menu)

All operations: honour the session's filters by default (folders contribute only
their *visible* contents; if a folder and some of its children are selected, only the
children count), can include hidden items ("Include hidden items by default" option,
overridable in each dialog), run as cancellable jobs with progress and a log, continue
after per-item errors, and optionally back up overwritten files (`15-options-reference.md`
Backups). Confirmation dialogs can be disabled per operation.

- **FC-070** (P0): **Copy to Right / Left** (Ctrl+R / Ctrl+L), **Copy to Side…**
  (prompt), dynamic **Copy to Other Side**; overwrite semantics; preserves timestamps,
  and (per back-end) attributes, Unix mode, creation date (option), NTFS ACLs (option,
  needs privilege), DOS 8.3 aliases (option, Windows).
- **FC-071** (P0): **Move to Right / Left / Side…**, **Exchange** (swap both sides).
- **FC-072** (P0): **Copy to Folder… / Move to Folder…**: target path or archive
  (created if missing); path options *Keep relative folder structure* (minimum common
  structure), *Keep base folder structure*, *Don't keep folder structure*.
- **FC-073** (P0): **Delete** (Del): sides selector (Left/Right/Both; option
  "Require explicit side selection"), *Use recycle bin if possible* (local drives
  only); **Delete Permanently** (Shift+Del).
- **FC-074** (P0): **Rename** (F2): single item inline; multi-item with *DOS
  compatible* mask (`abc?.bak` from computed base mask) or *Regular expressions*
  (old mask / new mask with `$1` groups).
- **FC-075** (P1): **Attributes…**: DOS flags Read-only/Hidden/Archive (tri-state
  checkboxes) or Unix owner/group/mode; folders only (not contents).
- **FC-076** (P0): **Touch…**: copy timestamps from other side, or set to a chosen
  date/time (default: current time or last touch); folders where the OS allows.
- **FC-077** (P0): **New Folder…** (Ins) on one or both sides; a name ending in an
  archive extension creates an archive.
- **FC-078** (P0): **Synchronize ▸ Update Right / Update Left / Update Both / Mirror
  to Right / Mirror to Left** on the current selection (or all when nothing is selected)
  with a preview dialog listing planned copies/deletes; "Just selection"; recycle bin
  option; per-item confirmation mode Prompt / Yes to All / No to All.
- **FC-079** (P1): **File Compare Report…** for selected pairs (`13-reports.md`).
- **FC-080** (P0): Drag & drop: from the OS into a pane (sets base folder or copies
  into the dropped folder, with a prompt), between panes (copy/move), onto the Home
  view buttons.
- **FC-081** (P0): Remote-target quirks: *Touch local files when copying to an FTP
  site* (when the server cannot set times, set the local time to match the uploaded
  file's).
- **FC-082** (P1): After copy/move/delete the affected nodes are re-stat'ed (not a
  full rescan); "Beep after long file operations" option.

## 9. Folder Merge (3-way)

- **FM-001** (P1): Three input panes (left, right, optional center ancestor) and an
  output target. Item states: Unchanged, Same change (both sides changed identically),
  Left add/delete/change, Right add/delete/change, Different change (mergeable), Conflict.
- **FM-002** (P1): Display filters: Show All, Show Changes, Show Conflicts, Show Left
  Changes, Show Right Changes, Show Mergeable, Show Unchanged, Show None; toggles per
  atomic state; *Ignore Same Changes*; *Center Pane* show/hide.
- **FM-003** (P1): Actions: **Merge…** (automatic merge of selected items: copies the
  winning side to the output; files needing a text merge run the text merge engine
  headless — conflicts are reported and left for manual review), **Copy to Output…**,
  Copy to Left/Right/Side, Delete, Rename, Attributes, Touch, Exclude, New Folder,
  Ignored, File Compare Report. Double-click opens a Text Merge child session.
- **FM-004** (P1): Columns **Result** (merge state) and **Remaining Actions**
  (planned action text: No action required, Automatic merge, Manual merge required,
  Copy to left/right, Delete left/right, Take left/center/right, Delete).
- **FM-005** (P1): **Folder Merge Info**: statistics per merge state. "Merge Parent
  Folders" opens a merge of the parents.

## 10. Folder Sync

- **FS-001** (P0): Layout: top = left/right base folders, Operations preset dropdown
  (Update Left, Update Right, Update Both, Mirror to Left, Mirror to Right, Custom),
  Rules and Filters buttons, Accept/Cancel for path edits; bottom = preview tree with the
  planned action per item (copy left→right, copy right→left, delete left, delete right,
  leave alone) and folder summaries ("contains files that will be copied…").
- **FS-002** (P0): Custom operation matrix (Rules ▸ Sync tab): action for each
  situation — file newer on left / newer on right / added on left / added on right /
  deleted on left / deleted on right / files different / files same — each from the
  set {copy left→right, copy right→left, delete left, delete right, leave alone}.
- **FS-003** (P1): **Sync Memory**: remembers the state after each sync (per session)
  to distinguish "added on one side" from "deleted on the other" and to flag
  conflicting changes; "Use Sync Memory", "Clear Memory".
- **FS-004** (P0): Per-item overrides via Actions ▸ Leave Alone / Copy Right to Left /
  Copy Left to Right / Delete Left / Delete Right; **Exclude** removes items from the
  plan. Search ▸ Next/Previous Conflict.
- **FS-005** (P0): **Run Sync** executes the plan (confirmation shows counts), with
  options *Sync empty folders*, *Sync within archives*, *Delete to recycle bin if
  possible*, *Expand subfolders when loading*. Suppress Filters shows items the sync
  will not touch. **Folder Sync Info** shows statistics.
- **FS-006** (P1): Scheduled/unattended sync is done through the script engine
  (`12-cli-and-scripting.md` `sync`), not through the GUI.

## 11. Snapshots

- **FC-090** (P1): **Save Snapshot…** (Tools): path (or current left/right base
  folder); include CRCs, executable versions, expanded archive contents; include empty
  folders; include symbolic-link targets; include items excluded by filters. Output is
  a compressed, read-only `.hksnap` file (zstd-compressed, self-describing binary or
  JSON lines; records name, kind, size, mtime, attrs/mode, owner/group, optional crc32,
  optional version info, link targets, archive children).
- **FC-091** (P1): A snapshot can be loaded as any base folder. Content comparisons
  other than CRC are unavailable against snapshots (files have no content). A snapshot
  remembers its source folder; opening a snapshot alone compares it with the original.

## 12. Log panel

- **FC-100** (P0): View ▸ Log toggles a panel listing operation summaries, details,
  warnings/errors, remote protocol traces and debug messages according to the Log
  options; capped at N messages; optional auto-save to a file.
