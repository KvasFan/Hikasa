# Command Line and Scripting

The command line is the **primary, complete interface** of the product
(`01-architecture.md` §0, ARCH-L2). Every tool is a subcommand that runs headless
on the core crates; the GUI is reached through the `gui` subcommand.

## 0. Executables

| Binary | Behaviour |
|--------|-----------|
| `hikasa` | The CLI. Subcommands for every function (§1). Runs headless; never needs the GUI. `hikasa gui …` connects to (or starts) the GUI over IPC. |
| `hikasa-gui` | The desktop application. Single instance per user: a second launch forwards its arguments to the running instance over IPC and exits (unless `--solo`). Accepts the same positional arguments and switches as `hikasa gui` so that double-click, drag & drop and shell integration work without the CLI. |

## 1. Subcommand surface (CLI-first contract)

- **CLI-000** (P0): Every subcommand supports `--json` (machine-readable result on
  stdout, progress and logs on stderr), `--quiet`, `--log <file>`, `--config-dir
  <dir>`, `--no-color`, and documented exit codes (§2). Output in human mode is stable
  enough for scripts (one record per line where applicable).

| Subcommand | Function (core crate) | Notes |
|------------|------------------------|-------|
| `hikasa folder compare <left> <right> [rules…] [--filters] [--expand-all] [--criteria …] [--display …] [--report …]` | `hikasa-folder` | Prints the aligned tree with states; `--json` emits the result model. Exit code reflects the overall state. |
| `hikasa folder sync <left> <right> (--update-left\|--update-right\|--update-both\|--mirror-left\|--mirror-right\|--custom <matrix>) [--dry-run] [--create-empty] [--recycle]` | `hikasa-folder` | `--dry-run` prints the plan; otherwise executes with progress. |
| `hikasa folder merge <left> <right> [--center <ancestor>] --output <dir> [--dry-run]` | `hikasa-folder` + `hikasa-text` | Automatic 3-way merge; conflicts listed; exit 14 when conflicts remain. |
| `hikasa folder op (copy\|move\|delete\|rename\|touch\|attrib\|mkdir\|copyto\|moveto) …` | `hikasa-folder` | File operations on a selection (`--select` masks as in scripts). |
| `hikasa folder snapshot <folder> --output <file.hksnap> [--crc] [--versions] [--expand-archives] [--follow-symlinks] [--include-empty] [--no-filters]` | `hikasa-folder` | |
| `hikasa text compare <left> <right> [--format <name>] [--rules …] [--ignore-unimportant] [--context N] [--report <layout>] [--output <file>]` | `hikasa-text` | Default output: unified diff with importance markers; `--report` for side-by-side/summary/interleaved/patch/statistics/xml. |
| `hikasa text merge <left> <right> [--center <file>] --output <file> [--favor-left\|--favor-right] [--force] [--ignore-unimportant]` | `hikasa-text` | Headless 3-way merge (`--automerge` semantics). |
| `hikasa text patch apply <patch> [--base <dir\|file>] [-p N] [--dry-run]` / `hikasa text patch show <patch>` | `hikasa-text` | |
| `hikasa text convert <file> (--trim-trailing\|--tabs-to-spaces\|--spaces-to-tabs\|--line-endings lf\|crlf\|cr) [--in-place\|--output]` | `hikasa-text` | |
| `hikasa text find <pattern> <folder> [--mask] [--regex] [--case] [--word] [--recursive]` | `hikasa-text` | Find in Files. |
| `hikasa table compare <left> <right> [--format] [--key <cols>] [--sheet …] [--report …]` | `hikasa-table` | |
| `hikasa hex compare <left> <right> [--alignment none\|fast\|complete] [--report …]` | `hikasa-hex` | |
| `hikasa picture compare <left> <right> [--mode tolerance\|range\|blend] [--tolerance N] [--diff-output <png>] [--metadata] [--report …]` | `hikasa-picture` | |
| `hikasa media compare <left> <right>` / `hikasa version compare <left> <right>` / `hikasa registry compare <left> <right>` | `hikasa-metadata` | |
| `hikasa quick <left> <right> [--size\|--crc\|--binary\|--rules]` | all | Quick compare: no output, exit code only (`--qc`). |
| `hikasa report <type> …` | `hikasa-report` | Shared report options (`13-reports.md`); every `compare` subcommand accepts `--report` as a shortcut. |
| `hikasa script run <file> [args…] [--silent] [--stop-on-error]` / `hikasa script check <file>` | `hikasa-script` | Run or syntax-check a script. |
| `hikasa session (list\|show\|new\|edit\|delete\|rename\|lock\|unlock\|export\|import\|run) …` | `hikasa-session` | `run` executes a saved session headless (compare/sync/merge) and prints/reports the result. `edit` takes `--set key=value` pairs. |
| `hikasa workspace (list\|show\|delete\|rename)` | `hikasa-session` | |
| `hikasa format (list\|show\|new\|edit\|delete\|import\|export\|detect <file>\|tokens <file>)` | `hikasa-formats` | `tokens` dumps the grammar tokenization (debugging grammars). |
| `hikasa profile (list\|show\|new\|edit\|delete\|test) …` | `hikasa-session` + `hikasa-vfs` | Secrets prompted interactively or read from `--password-stdin`. |
| `hikasa vfs (ls\|stat\|cat\|cp\|mv\|rm\|mkdir\|touch) <spec>…` | `hikasa-vfs` | Generic access to any back-end (archives, remote, snapshots). |
| `hikasa settings (get\|set\|list\|export\|import\|restore-defaults)` | `hikasa-session` | Program options. |
| `hikasa gui …` | IPC client | See §1.1. |
| `hikasa completions <shell>`, `hikasa help [topic]`, `hikasa version` | | |

### 1.1 `hikasa gui` — connecting to the GUI

- **CLI-G01** (P0): `hikasa gui open <specs…> [view options]` opens a view in the
  running GUI (starting it when needed, unless `--no-launch`). `--wait` blocks until
  the view closes and returns its exit code. All legacy positional forms and switches
  below (`--fv`, `--ro`, `--title1`, `--automerge --reviewconflicts`, …) are accepted
  by `hikasa gui open` and by `hikasa-gui` itself.
- **CLI-G02** (P0): `hikasa gui session <name> [--wait]`, `hikasa gui workspace
  <name>`, `hikasa gui script <file>` (run with the status window), `hikasa gui list`
  (open views), `hikasa gui close <id>`, `hikasa gui ping`, `hikasa gui quit`.
- **CLI-G03** (P0): Any headless subcommand accepts `--gui` to show its result in
  the GUI instead of printing it (e.g. `hikasa text compare a b --gui`).

## 2. Legacy positional forms and switches (accepted by `hikasa gui open` and `hikasa-gui`)

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

- **CLI-004** (P0): Exit codes (shared by all subcommands):

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

## 2b. Return-code semantics for version control

- **CLI-010** (P0): `hikasa gui open left right --wait` returns only after the user
  closes the view; `hikasa quick` and headless `compare` subcommands return
  immediately. Document the recommended configuration for git
  (`difftool`/`mergetool` with `hikasa text merge … --gui --wait` or
  `--automerge --reviewconflicts --mergeoutput`), Mercurial, Subversion and Perforce
  in the user manual.

## 3. Script language

Scripts remain supported as a batch layer on top of the same core; `hikasa script
run` executes them headless and `hikasa gui script` runs them with the status window.

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
