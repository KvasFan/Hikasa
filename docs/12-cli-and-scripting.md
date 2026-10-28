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
  *M1 note (D-40):* with `--json`, stdout holds exactly one JSON document (also on error);
  progress and logs always go to stderr; `--quiet` suppresses progress and every non-error
  stderr line and never changes stdout or the exit code. The M1 subset of the table below
  and its flags are listed in §4.

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
| `hikasa format (list\|show\|new\|edit\|delete\|import\|export\|enable\|disable\|move\|restore\|detect <file>\|tokens <file>)` | `hikasa-formats` | `tokens` dumps the grammar tokenization (debugging grammars). The write forms are M2 W1 (§4, `10-file-formats.md` §1.1). |
| `hikasa profile (list\|show\|new\|edit\|delete\|test) …` | `hikasa-session` + `hikasa-vfs` | Secrets prompted interactively or read from `--password-stdin`. |
| `hikasa vfs (ls\|stat\|cat\|cp\|mv\|rm\|mkdir\|touch) <spec>…` | `hikasa-vfs` | Generic access to any back-end (archives, remote, snapshots). |
| `hikasa settings (get\|set\|unset\|list\|export\|import\|restore-defaults)` | `hikasa-session` | Every settings file, keys `<file>.<path>` (D-73). `get`/`set`/`unset`/`list`/`restore-defaults` are M2 W1 (§4); `export`/`import` (settings packages) come with W2. |
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
- *M1 notes (D-22, D-49, D-50):* `gui ping|list|close|quit` never start the GUI; when
  none is running they exit 100 ("not running"). Only `gui open` and `--gui` start it.
  `gui open -` spools standard input to `<cache>/stdin/<n>/<name>` (the name from
  `--stdin-name`, else `stdin.txt`) and sends that absolute path; with `--wait` the file is
  deleted after the view closes. If the waiting CLI is interrupted (Ctrl-C, a signal), the
  spooled file and its numbered folder stay in `<cache>/stdin/` and may be deleted by hand
  (the CLI installs no signal handler). `clipboard://` is refused by `gui open` and `--gui`
  (exit 100): specs sent over IPC must be absolute and there is no payload transfer.
  `--gui` on `text compare` and `folder compare` never compares locally: it sends the same
  specs, rules files, rule flags and view options (`--display`, `--context`, `--folders`,
  `--suppress-filters`, `--expand-all`) to the GUI, after checking every word locally (a
  typo exits 100 before the GUI is contacted); `--wait` and `--no-launch` work as for
  `gui open`. The report flags (`--report`, `--output`, `--html`, …) cannot be combined
  with `--gui`. In M1 `gui session`, `gui workspace` and `gui script` are not available.

## 2. Legacy positional forms and switches (accepted by `hikasa gui open` and `hikasa-gui`)

*M1 note (D-51):* one parser in the session layer (`normalize_legacy_args`,
`parse_open_args`) reads these forms for `hikasa gui open` and for `hikasa-gui`, so both
accept exactly the same command lines. Recognised switches that M1 does not implement
(`--automerge`, `--center`, `--qc`, `--vcs1`, …) are forwarded to the GUI with warning
`switch_ignored`; unknown switches exit 100.

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

  *M1 exit codes (D-01, D-02, D-03, D-35):*
  - `quick` and `text compare`: 1 bytes identical, 2 rules-based same, 11 binary content
    that differs (binary content never gets a rules-based verdict), 12 only unimportant
    differences (2 with `--ignore-unimportant`), 13 important differences. **Any** error,
    including an input that cannot be loaded, gives 100; 107 is reserved for base folders.
  - `folder compare`: 13 if any important difference; else 100 when the result is
    incomplete or any item is in state Error or Unknown (an incomplete result cannot claim
    sameness); else 12 when only unimportant differences remain; else 2. A base folder that
    cannot be loaded (missing, not a folder) gives 107; every other error 100.
  - Display filters (`--display`, `--context`, `--folders`, `--suppress-filters`) never
    change the exit code or the summary; `--ignore-unimportant` does, because it changes
    the meaning of "same".
  - `format …`, `vfs …`, `completions`, `version`: 0, or 100 on error; `gui …` see §2b.
  - A command line that does not parse exits **100**, never 2 (2 means "rules-based
    same"); `--help` and `--version` exit 0. A panic is caught and exits **100**, never
    101 (101 means "conflicts, output not written"). A broken pipe on stdout stops the
    output and keeps the exit code that was computed.

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
- *M1 note (D-23):* `gui open --wait` (and `--gui --wait`) exits with the view's code when
  it closes: 1/2/11/12/13 as for `quick` for file views, 2/12/13/100 for folder views
  (a folder view closed while still scanning gives 100), 0 when the view has no verdict,
  100 when it failed to load or when the GUI shut down or dropped the view. **102** means
  the CLI cannot wait for the GUI: the connection is lost while waiting, a launched GUI
  does not become reachable within the retry budget (10 s), or `--wait --no-launch` finds
  no GUI. Without `--wait`, `gui open` exits 0 once the view is open, and `--no-launch`
  with no GUI gives **100**. **103** means no GUI executable was found (`hikasa-gui` next
  to `hikasa`, then on `PATH`). A socket folder that another user controls is refused
  with 100 and nothing is launched.

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

*M1 note (D-57):* `hikasa folder compare --criteria <list>` uses the `criteria`
syntax above: items separated by `,` or spaces; listed criteria are on, unlisted quick
tests and the content method are off. Hikasa adds `filename-case` (FC-022);
`version` (FC-028) is refused in M1. Only the documented spellings are accepted
(`timestamp:3sec`, `timezone:left+2`). The individual flags (`--size`, `--no-timestamps`,
`--content`, `--owner`, …) are applied after `--criteria`, which is applied after
`--rules-file` (rules precedence: defaults ← rules file ← `--criteria` ← flags).

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

## 4. The M1 command line (binding interpretation; contract §1.9, §2.21, §4; decisions D-29, D-32, D-35, D-40, D-50)

M1 implements the subcommands below in the `hikasa` binary. Every subcommand is a thin
adapter over the session service: vocabulary words (`--display`, `--criteria`,
`--unimportant`, `--report`, …) are parsed by the core crates, so the CLI, scripts and the
GUI read the same words. Other rows of the table in §1 (sync, merge, patch, table, hex,
picture, script, session, profile, settings, `vfs cp`/`mv`/…) come in later milestones.

**Global flags** (accepted before or after the subcommand):

| Flag | Meaning |
|---|---|
| `--json` | one JSON document on stdout (§4.2 of the contract), also on error; recognised before a `--` only (after it, `--json` is a file name); `--help` stays text, `--version --json` prints the `hikasa version --json` document |
| `--quiet`, `-q` | no progress, no warnings, no other non-error stderr output |
| `--log <file>` | append diagnostic logs to the file (INFO, DEBUG with `--verbose`; a panic's backtrace) |
| `--verbose` | debug-level logs, including the ignored keys of `options.toml` |
| `--config-dir <dir>` | settings folder (portable behaviour); `""` is refused |
| `--no-color` | no ANSI colours (also when `NO_COLOR` is set, or the output is not a terminal) |
| `--help`, `-h` / `--version`, `-V` | usage / `hikasa <version>` (exit 0), before or after a subcommand; the arguments after `gui open` are the shared parser's (§2), which knows `--help` but not `--version` |

A flag that takes a value (`--config-dir`, `--log`) may be given before and after the
subcommand only with the same value (the same path, relative or absolute); different
values are a usage error (exit 100), wherever they stand.

**Subcommands and flags:**

| Subcommand | Flags |
|---|---|
| `quick <left> <right>` | `--size` \| `--crc` \| `--binary` \| `--rules` (default); `--ignore-unimportant` (`--iu`); `--format <id\|name>`; `--rules-file <toml>`; `--unimportant <list>`, `--unimportant-text <text>`…, `--unimportant-regex <re>`…, `--compare-line-endings`; `--stdin-name <name>`. Prints nothing; the exit code is the result. `--size` compares only the sizes and `--crc` only the CRC-32 checksums: equal sizes (or checksums) report `binary_same` and exit 1 as specified (contract §2.21) although the bytes were not compared one by one; `--binary` and `--rules` read and compare the contents |
| `text compare <left> <right>` | `--format`; `--encoding`, `--left-encoding`, `--right-encoding <label>`; `--rules-file` (`--rules`); `--algorithm auto\|unaligned\|standard\|myers\|patience\|histogram`, `--skew <n>`, `--closeness`/`--no-closeness`, `--closeness-threshold <‰>`, `--never-align`; `--unimportant`, `--unimportant-text`, `--unimportant-regex`, `--compare-line-endings`, `--orphans-unimportant`; `--ignore-unimportant` (`--iu`); `--context <n>` (default 3, from the options), `--display all\|mismatches\|context\|matches`; `--report unified\|side-by-side\|summary\|patch\|statistics` (default: the annotated unified diff), `--patch-style normal\|context\|unified`, `--html`, `--html-mono`, `--line-numbers`/`--no-line-numbers`, `--width <n>` (160), `--wrap none\|character\|word`, `--title <text>`, `--output <file>`; `--stdin-name`; `--gui`, `--wait`, `--no-launch` |
| `folder compare <left> <right>` | `--filters <masks>`; `--rules-file` (`--rules`); `--criteria <list>`; `--size`/`--no-size`, `--timestamps`/`--no-timestamps`, `--tolerance <secs>`, `--ignore-dst`, `--ignore-timezone`, `--filename-case`, `--attributes <letters>`, `--permissions`, `--owner`, `--group`; `--content none\|crc\|binary\|rules`, `--skip-if-quick-same`, `--no-override-quick`; `--text-rules-file <toml>`, `--unimportant`, `--unimportant-text`, `--unimportant-regex` (for `--content rules`); `--case-sensitive`\|`--case-insensitive`, `--align-extensions`, `--align-normalization`; `--cutoff <expr>`, `--size-filter <expr>`, `--include-protected`; `--follow-symlinks`, `--flatten`, `--no-scan-orphans`; `--display <filter>` (default `differences`), `--folders always\|structure\|files-only` (default `structure`), `--ignore-unimportant` (`--iu`), `--suppress-filters`, `--expand-all`; `--report list\|side-by-side\|summary\|json` (default `list`), `--columns <list>`, `--html`, `--html-mono`, `--title`, `--output`; `--gui`, `--wait`, `--no-launch` |
| `format list` / `format show <id\|name>` / `format detect <file>` / `format tokens <file>` | `detect`: `--stdin-name`; `tokens`: `--format`, `--encoding`, `--lines <a>-<b>`, `--stdin-name` |
| `format new <id>` / `edit <id>` / `delete <id>` / `import <path>` / `export <id\|name>` / `enable <id>` / `disable <id>` / `move <id>` / `restore [<id>…]` | *M2 W1 (D-75):* `new`: `--from <id\|name>`, `--name <text>`, `--mask <mask>`, `--kind <kind>`; `edit`: `--from-file <path>` \| `--set <key=value>`…; `import`: `--replace`; `export`: `-o`/`--output <path>`; `move`: one of `--up`, `--down`, `--top`, `--bottom`, `--before <id>`, `--after <id>`; `restore`: `<id>`… \| `--all`, and `--dry-run` \| `--yes`. They write only the files they change (`10-file-formats.md` §1.1) |
| `settings get <key>` / `set <key> <value>` / `unset <key>` / `list [<key>]` / `restore-defaults` | *M2 W1 (D-71, D-73, D-77):* `set`: `--force` (overwrite a file changed since it was read); `list`: `--changed`; `restore-defaults`: `--all` \| `--file <file>`…, and `--dry-run` \| `--yes`. A key is `<file>.<path>`: the file is `options`, `state`, `commands`, `theme`, `themes.<scheme>`, `filters`, `openwith` or `archives`; the path is the TOML key path, a segment that is not a plain name in double quotes, an array position as a number (`openwith.app.0.command`). A key naming a table gives the whole table (`get`, `list`); `set` takes a leaf or a whole array. The value is TOML; for text keys (strings, paths, masks, colours, shortcuts, enumerations) text that is not a TOML string is taken as it is. These commands read `options.toml` leniently (a bad value is the warning `invalid_config_value`), every other command refuses it (`03-sessions-settings-profiles.md` §5.2). An unknown key (also an array position past the end, or a command id that its view does not have) is `not_found` with up to five suggestions; a key the file holds but Hikasa does not know is listed as unknown |
| `vfs ls <spec>` / `vfs stat <spec>` / `vfs cat <spec>` | `ls`, `stat`: `--follow-symlinks` |
| `gui open <args…>` | the forms and switches of §2 (`--wait`, `--no-launch`, `--fv`, `--title1`…`--title4`, `--ro`, `--ro1`, `--ro2`, `--filters`, `--expandall`, `--format`, `--iu`, `--stdin-name`, `--help`); the arguments after `open` reach that parser unchanged, so `hikasa gui open -- -x a.c` opens the files `-x` and `a.c` exactly like `hikasa-gui -- -x a.c`; the global flags may also follow the specs (the same flag given twice with different values exits 100) |
| `gui ping` / `gui list` / `gui close <id>` / `gui quit` | — |
| `completions bash\|elvish\|fish\|powershell\|zsh` | — |
| `version` | — |
| `help [topic…]` | `--list`. *M2 W0 (D-67):* prints a help topic of `help/` (the words joined with `-` name it: `help text compare` → `text-compare`; a subcommand without its own topic shows its parent's); no topic prints the contents; an unknown topic exits 100 with suggestions; `--json` gives `hikasa.help.v1`. Reads no configuration |

`-` names standard input (read once, when a command first needs it; `--stdin-name` gives
it a file name for format detection, D-15). `--output <file>` writes the report through a
temporary file and a rename, so on any error the file is untouched; it prints nothing,
and with `--json` the document still goes to stdout. When `<file>` is a symbolic link, the
file it points to is replaced and the link stays; a FIFO or a device (such as `/dev/null`)
is written to directly instead, which is not atomic, and is never replaced. A layout that
has no HTML form (`--report unified|patch|statistics --html`, `folder compare --report
list|json --html`) is refused (exit 100) once the comparison has run. With `--json`, `text compare` shows
the mismatches by default (`--context n` alone means `context` with n lines). Human
`text compare` reports show all lines (`--report side-by-side`, `summary`) or the
differences with 3 lines of context (the default annotated unified diff and `--report
patch`). `folder compare` shows the differences with the folder structure by default in
every output, human (`list`, `side-by-side`, `summary`) and `--json` alike.

**Human output.** One record per line where applicable, with control and bidirectional
formatting characters in names and file text escaped (`\n`, `\t`, `\u{…}`, a backslash as
`\\`). `quick`, `gui open`, `--gui`, `gui close` and `gui quit` print nothing on stdout.
Every failed command prints one `hikasa: error: …` line on stderr (also with `--quiet`,
e.g. `quick --crc` on a file that cannot be read); warnings are `hikasa: warning: …` lines
(not with `--quiet`):

| Command | Line |
|---|---|
| `folder compare` (list) | `<marker>  <path>[/]`, then `# <n> same, <n> left newer, …` (kept with `--quiet`) |
| `format list` | `<position>\t<id>\t<status>\t<name>\t<mask>` |
| `format detect` | `<id or ->\t<name>` |
| `format tokens` | `L<line>:<start>-<end>\t<element>\t<text>` |
| `vfs ls` | `<kind><permissions or attributes> <size> <mtime> <name>[ -> <target>]` |
| `vfs stat` | the `vfs ls` line, then `capabilities: …` |
| `gui list` | `<view>\t<kind>\t<title>\t<spec>…` |
| `gui ping` | `<server>\t<protocol version>` |
| `settings get` | the TOML value (`3`, `"dark"`, `["a"]`); a table as TOML; `(not set)` for an optional key without a value |
| `settings set`, `settings unset` | `<key> = <value>`, then `(file unchanged)` when nothing was written |
| `settings list` | `<key> = <value>`, with `  # default: <value>` when changed and `  # unknown key (kept)` for a key Hikasa does not know; a shortcut conflict that involves a changed shortcut is a warning on stderr |
| `settings restore-defaults`, `format restore` | `delete <path>` or `absent <path>` per file (also with `--dry-run`) |
| `format new`, `edit`, `delete`, `import`, `enable`, `disable`, `move` | `wrote <path>`, `removed <path>` |
| `format export` | the TOML (with `-o`: `wrote <path>`) |

**JSON (D-29, D-32, D-35, D-40).** Every document is an envelope `{schema, hikasa_version,
ok, exit_code, result, warnings, error}` with the schemas `hikasa.quick.v1`,
`hikasa.text_compare.v1`, `hikasa.folder_compare.v1`, `hikasa.format_list.v1`,
`hikasa.format_show.v1`, `hikasa.format_detect.v1`, `hikasa.format_tokens.v1`,
`hikasa.vfs_list.v1`, `hikasa.vfs_stat.v1`, `hikasa.vfs_cat.v1`, `hikasa.gui_open.v1`,
`hikasa.gui_ping.v1`, `hikasa.gui_list.v1`, `hikasa.gui_close.v1`, `hikasa.gui_quit.v1`,
`hikasa.version.v1`, `hikasa.completions.v1` and, for a command line that does not parse,
`hikasa.usage_error.v1` (its `error.message` is the parser's whole message in one line);
M2 adds `hikasa.help.v1` (`{topic: {id, title, markdown, links} | null, topics: [{id,
title}]}`), appended last to the `schemas` of `hikasa version --json`, then (W1)
`hikasa.settings_get.v1` (`{key, file, path, value, default, is_default, kind, opt}`),
`hikasa.settings_set.v1` (`{key, file, path, old, new, written}`, also for `unset`),
`hikasa.settings_list.v1` (`{entries: [{key, value, default, is_default, kind, opt, unknown}],
conflicts: [{view, shortcut, commands}]}`), `hikasa.settings_restore.v1` (`{dry_run, files:
[{file, path, existed, deleted}]}`), `hikasa.format_write.v1` (`{action, id, written, removed}`),
`hikasa.format_export.v1` (`{id, path, toml}`) and `hikasa.format_restore.v1` (`{dry_run, ids,
removed}`). TOML values appear as JSON values (a table as an object, a datetime as a string;
`null` for an optional key without a value or a key without a default).
`warnings` start with the warnings of the configuration (unknown `options.toml` sections,
skipped user formats), which human mode prints once on stderr; `version` and `completions`
read no configuration (they work with a broken or newer one), so their documents carry no
such warnings. A `quick` whose content comparison could not read a file has `ok: false`,
its `error`, and the result `{"result": "error", "error": …}` (exit 100).
The folder comparison is a flat, pre-order list of the visible items with `path` and
`depth`, not nested objects (D-29): it streams well and works with `jq`. `vfs cat --json`
carries the content as base64 (D-32).

**Environment.** `NO_COLOR` (no colours), `HIKASA_IPC_ENDPOINT` (the IPC socket or pipe
name instead of the per-user endpoint; tests) and `HIKASA_GUI_EXE` (the GUI executable to
start instead of `hikasa-gui` next to `hikasa` or on `PATH`, where a candidate without an
execute bit is skipped; a relative path is taken relative to the current folder; tests).
No usable GUI executable — none found, or one that cannot be started (not executable, a
missing interpreter) — exits 103 with a message naming it.

**Performance (gates G1 and G2 through the binary, contract §6.4).** The ignored test
`cargo test --release -p hikasa-cli --test perf_gates -- --ignored --nocapture` generates
the inputs under the temporary folder and runs the `hikasa` binary (one warm-up run, then
the median of three). G1: two 1,000,000-line C sources (≈ 48 MB each, ≈ 1 % of the lines
changed in clusters), `text compare --report summary`; threshold < 3 s end to end and a
peak RSS ≤ 3 × (L + R bytes) + 64 B × (L + R lines) + 32 MiB (427 MiB here). G2: 100,000
files in 10,000 folders per side, 1 % changed, `folder compare --content none --report
summary` with a warm cache; threshold < 1.5 s. Measured on 2026-10-25 (AMD Ryzen 7 7800X3D,
16 threads, btrfs on dm-crypt, Linux 7.3; release build):

| Gate | Command | Median (min–max) | Peak RSS |
|---|---|---|---|
| G1 | `text compare … --algorithm auto` | 0.434 s (0.433–0.460) | 298 MiB |
| G1 | `text compare … --algorithm standard` | 0.452 s (0.449–0.459) | 298 MiB |
| G1 | `text compare` of an identical pair | 0.355 s (0.342–0.370) | 276 MiB |
| G2 | `folder compare … --report summary` (warm) | 0.047 s (0.046–0.047) | 31 MiB |
| G2 | `folder compare … --json` (warm, for the record) | 0.057 s | — |
| G2 | `folder compare … --content none --report summary` (**cold**, page cache dropped) | 0.694 s (warm in the same run 0.275 s) | — |

The machine was not idle (unrelated builds, load average 5–9): the figures are the run
with a small spread; a disturbed run stayed below 1.3 s for G1. The cold G2 figure (gate < 5 s) was measured on 2026-10-26 by the maintainer with
`sync; echo 3 > /proc/sys/vm/drop_caches` (root) right before the run, on btrfs over NVMe: 0.694 s, exit 13 as
expected (one file in 997 differs). No CI job runs these tests.
