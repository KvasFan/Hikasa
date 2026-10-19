# Command Line and Scripting

## 1. Executables

| Binary | Behaviour |
|--------|-----------|
| `hikasa` | GUI application. Single instance per user: a second invocation forwards its arguments to the running instance and exits immediately (unless `--solo`). |
| `hikasa-cli` (`hkcmp`) | Console helper: forwards to the GUI instance and **waits** until the resulting view closes, then exits with that view's exit code. Use from version-control systems and scripts. Also runs scripts and quick compares headless when no GUI is needed. |

- **CLI-001** (P0): Switches accept `--name`, `-name` and Windows-style `/name`
  spellings; values with `=`; quote arguments containing spaces. `--help` prints usage.
- **CLI-002** (P0): Positional arguments:
  - a **named session** → opens it;
  - a **named workspace** → loads it;
  - **two folders** → Folder Compare; **two files** → the file view chosen by file
    formats; **three files** → Text Merge (left, right, center); **four files** → Text
    Merge with output;
  - `@script.txt` → runs the script (`§3`);
  - a **settings package** → imports it;
  - a **patch file** (`.diff`/`.patch`) → Text Patch view;
  - a **snapshot** alone → compares it with its original folder;
  - `-` → reads stdin into the appropriate view.
  - one folder → Folder Compare with the folder on the left and an empty right side
    (P1).
- **CLI-003** (P0): Switches:

| Switch | Meaning |
|--------|---------|
| `--automerge` | Merge without interaction unless conflicts are found. |
| `--center=<file>` | Explicit merge center file. |
| `--mergeoutput=<file or dir>` | Explicit merge output. |
| `--force` | With `--automerge`: write conflicts with CVS-style markers. |
| `--reviewconflicts` | With `--automerge`: open a Text Merge view if conflicts exist. |
| `--favorleft`, `--favorright` | Take non-conflicting changes from the favoured side without highlighting; favoured side wins unimportant conflicts. |
| `--iu`, `--ignoreunimportant` | Turn on Ignore Unimportant Differences (for `--automerge` and `--qc`). |
| `--closescript` | Close the script status window when finished (overrides option). |
| `--silent` | No windows, no task-bar entry; packages import everything; scripts log errors instead of showing dialogs. |
| `--edit` | Open a Text Edit view. |
| `--sync` | Open a Folder Sync view for the two folders. |
| `--expandall` | Expand all subfolders in the initial folder comparison. |
| `--filters=<masks>` | Name filter for the initial folder comparison. |
| `--fv=<type>`, `--fileviewer=<type>` | Force the view type: `Text Compare`, `Text Merge`, `Table Compare`, `Hex Compare`, `Media Compare`, `Picture Compare`, `Registry Compare`, `Version Compare`, `Folder Compare`, `Folder Merge`, `Folder Sync`, `Text Edit`, `Text Patch`. |
| `--nobackups` | Do not create backup files. |
| `--qc[=size|crc|binary]`, `--quickcompare[=…]` | Quick compare two files and set the exit code; no type = rules-based. |
| `--ro`, `--readonly` / `--ro1`, `--lro`, `--leftreadonly` / `--ro2`, `--rro`, `--rightreadonly` | Disable editing on all / left / right side. |
| `--savetarget=<file>` | Save command writes to this file instead of the original. |
| `--solo` | Force a new instance. |
| `--title1=` … `--title4=` (`--lefttitle`, `--righttitle`, `--centertitle`, `--outputtitle`) | Description shown in the path edit of the respective pane. |
| `--vcs1=` … `--vcs4=` (`--vcsleft`, …) | Version-control path shown in the pane (unless a title is given) and used to pick the file format. |
| `--config-dir=<dir>` | Use an alternative settings directory (also enables portable behaviour). |
| `--log=<file>`, `--verbose` | Diagnostic logging. |

- **CLI-004** (P0): Exit codes:

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Binary same |
| 2 | Rules-based same |
| 11 | Binary differences |
| 12 | Similar (only unimportant differences) |
| 13 | Rules-based differences |
| 14 | Conflicts detected |
| 100 | Unknown error |
| 101 | Conflicts detected, merge output not written |
| 102 | CLI helper unable to wait for the GUI |
| 103 | CLI helper cannot find the GUI executable |
| 105 | Error loading script file |
| 106 | Script syntax error |
| 107 | Script failed to load folders or files |

- **CLI-005** (P0): Quick Compare dialog on startup (program option): when launched
  with two files, show a summary dialog (binary or rules-based) instead of the view;
  option "Open view automatically if files are different".
- **CLI-006** (P1): Shell integration (install-time, optional): Linux file-manager
  context-menu actions (Nautilus/Dolphin/Thunar scripts), Windows Explorer context
  menu (Compare, Compare to <item>, Select Left, Compare Using, Sync, Merge, Select
  Center, Open for Compare, Edit, View Patch, Compare to Clipboard), macOS Finder
  extension; configurable in the Startup options page.

## 2. Return-code semantics for version control

- **CLI-010** (P0): `hikasa-cli left right` returns only after the user closes the
  view; `--qc` returns immediately. Document the recommended configuration for git
  (`difftool`/`mergetool` with `--automerge --reviewconflicts --mergeoutput`),
  Mercurial, Subversion and Perforce in the user manual.

## 3. Script language

A script is a UTF-8 text file; one command per line; `#` starts a comment; blank lines
ignored; commands are case-insensitive; arguments separated by spaces, quoted with `"`
when they contain spaces; a trailing `&` continues the command on the next line.
`%1`…`%9` expand to command-line arguments following the script name (switches
excluded); `%NAME%` expands environment variables (case-sensitive); `%date%`
(yyyy-mm-dd), `%time%` (locale), `%fn_time%` (hh-mm-ss) are dynamic. `lt`/`rt` are
accepted for `left`/`right`.

Scripts manipulate an invisible Folder Compare session: load, filter, expand, select,
act, report. A **Scripting Status window** shows progress and errors unless `--silent`.
Options: beep when finished, close when finished, **shared scripts folder** (searched
when the script is not found in the current folder).

### 3.1 Commands

```
attrib (+|-)<set> [...]                       # set/clear a,s,h,r on the selection (Windows)
beep
collapse all | collapse <path> [...]
compare [CRC|binary|rules-based]              # content-compare the selection once
copy (left->right|right->left)
copyto [left|right|all] [path:(relative|base|none)] <path>
criteria [attrib:<set>] [version] [timestamp[:<n>sec][;IgnoreDST]] [size|CRC|binary|rules-based]
         [timezone:(ignore|(left|right)(+|-)<0..12>)] [follow-symlinks] [ignore-unimportant]
         [owner] [group] [permissions]
delete [recyclebin=(yes|no)] (left|right|all)
expand all | expand <path> [...]
file-report layout:(side-by-side|summary) [options:...] [title:...] output-to:(printer|clipboard|<file>)
         [output-options:...] [<session>|<left> <right>]
filter <masks> | filter cutoff:([<|>](<timestamp>|<n>days)|none) | filter size:([<|>]<n>[KB|MB|GB|TB]|none)
         | filter attrib:((+|-)<set>|none) | filter exclude-protected | filter include-protected
         | filter unixtype:((+|-)<b c l p r s>|none)
folder-report layout:(side-by-side|summary|xml) [options:display-*,column-*,include-file-links] ...
hex-report layout:(side-by-side|summary|interleaved) ...
load <session> | load [create:(all|left|right)] <left> [<right>] | load <default>
log [none|normal|verbose] [[append:]<file>]
media-report layout:(side-by-side|summary) ...
move (left->right|right->left)
moveto [left|right|all] [path:(relative|base|none)] <path>
option stop-on-error | option confirm:(prompt|yes-to-all|no-to-all)
picture-report layout:(side-by-side|summary) ...
registry-report layout:(side-by-side|summary) ...
rename [regexpr <old mask>] <new mask>
select <mask> [...]      # all | [(left|right|all).][(exact|diff|newer|older|orphan|all).][(files|folders|all)] | empty.folders
snapshot [save-crc] [save-version] [expand-archives] [follow-symlinks] [include-empty] [no-filters]
         (left|right|path:<path>) [output:<target>]
sync [visible] [create-empty] (update|mirror):(left->right|right->left|all)
table-report (alias data-report) layout:(side-by-side|summary|interleaved) ...
text-report layout:(side-by-side|summary|interleaved|patch|statistics|xml) [options:...] ...
touch (left->right|right->left) | touch (left|right|all):(now|<timestamp>)
version-report layout:(side-by-side|summary) ...
```

- **SCR-001** (P0): Report option vocabulary (shared): `ignore-unimportant`,
  `display-all|display-mismatches|display-context|display-matches`, folder filter
  flags `display-no-orphans|display-mismatches-no-orphans|display-orphans|
  display-left-newer|display-right-newer|display-left-newer-orphans|
  display-right-newer-orphans|display-left-orphans|display-right-orphans`, columns
  `column-vcs|column-revision|column-version|column-size|column-crc|column-timestamp|
  column-attributes|column-owner|column-group|column-none`, `line-numbers`,
  `strikeout-left-diffs|strikeout-right-diffs`, `patch-normal|patch-context|
  patch-unified`, `include-file-links`. Output options: `print-color|print-mono`,
  `print-portrait|print-landscape`, `wrap-none|wrap-character|wrap-word`,
  `html-color|html-mono|html-custom:<css>`.
- **SCR-002** (P0): Semantics: `load` of a session applies all its settings except
  the display filter; `load` of folders uses the current settings; `expand` is required
  before `select` sees subfolder contents; `sync` ignores the selection; `select`
  masks are OR-ed; folders match a result mask only if all their contents match;
  `diff` means differences other than timestamp; failure to load a base folder
  terminates the script with code 107.
- **SCR-003** (P0): Error handling: syntax errors → code 106 before anything runs;
  runtime file errors are logged and (with `option stop-on-error`) prompt or, when
  silent, abort; modal dialogs requested while silent are logged as errors ("Attempted
  to open a dialog when running silently").
- **SCR-004** (P1): Script status window: log view, progress, Stop button; "Script
  completed in <time>".

### 3.2 Examples (must work as acceptance tests)

```
load "Sync Laptop"
sync update:left->right
```
```
log normal "/var/log/hikasa-sync.txt"
load <default>
filter "*.html;*.htm;*.png;*.jpg"
load "/srv/www" sftp://deploy@host/var/www
sync create-empty mirror:left->right
```
```
text-report layout:side-by-side options:ignore-unimportant,display-context &
  output-to:"%3" output-options:html-color "%1" "%2"
```
```
option confirm:yes-to-all
criteria crc
load "/data/project" "/backups/project.hksnap"
expand all
select left.diff.files left.orphan.files
copyto left path:relative "/backups/changes-%date%.zip"
snapshot save-crc left output:"/backups/project.hksnap"
```
