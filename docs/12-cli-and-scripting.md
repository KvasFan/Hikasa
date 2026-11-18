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
| `hikasa folder sync <left> <right> (--update-left\|--update-right\|--update-both\|--mirror-left\|--mirror-right\|--custom <matrix>) [--dry-run] [--create-empty] [--recycle]` | `hikasa-folder` | `--dry-run` prints the plan; otherwise executes with progress. *M2 W6c:* the full flag set is in §4 (`folder sync`). |
| `hikasa folder merge <left> <right> [--center <ancestor>] --output <dir> [--dry-run]` | `hikasa-folder` + `hikasa-text` | Automatic 3-way merge; conflicts listed; exit 14 when conflicts remain. |
| `hikasa folder op (copy\|move\|exchange\|delete\|rename\|touch\|mkdir\|copyto\|moveto\|compare\|crc\|exclude\|ignored\|open-with) …` | `hikasa-folder` | File operations on a selection (`--select` masks as in scripts). |
| `hikasa folder snapshot <folder> --output <file.hksnap> [--crc] [--versions] [--expand-archives] [--follow-symlinks] [--include-empty] [--no-filters]` | `hikasa-folder` | |
| `hikasa text compare <left> <right> [--format <name>] [--rules …] [--ignore-unimportant] [--context N] [--report <layout>] [--output <file>] [--align <L>:<R>]… [--isolate left\|right:<a>[-<b>]]… [--left-format <id\|name>] [--right-format <id\|name>] [--ignored left\|right:<a>[-<b>]]…` | `hikasa-text` | Default output: unified diff with importance markers; `--report` for side-by-side/summary/interleaved/patch/statistics/xml. *M2 W10a (D-266):* `--align`/`--isolate` set alignment anchors (TC-013), see §4. *M2 W10b (D-281, D-282):* `--left-format`/`--right-format` give a pane its own format (FF-003), `--ignored` marks ignored lines (TC-024). |
| `hikasa text merge <left> <right> [--center <file>] --output <file> [--favor-left\|--favor-right] [--force] [--ignore-unimportant]` | `hikasa-text` | Headless 3-way merge (`--automerge` semantics). |
| `hikasa text patch apply <patch> [--base <dir\|file>] [-p N] [--dry-run]` / `hikasa text patch show <patch>` | `hikasa-text` | |
| `hikasa text convert <file>… [--trim-trailing] [--tabs-to-spaces\|--spaces-to-tabs] [--tab-size <1-32>] [--line-endings lf\|crlf\|cr] [--encoding <label>] [--bom\|--no-bom] [--lines <a>-<b>] [--format <id\|name>] [--encoding-in <label>] [--unmappable refuse\|replace] [--in-place\|--output <file>] [--backup\|--no-backup] [--force] [--dry-run] [--yes] [--confirm <mode>] [--stdin-name <name>]` | `hikasa-text` | *M2 W10b (D-280, TC-063):* the conversions of the text views on whole files, in the fixed order trim, tabs, line endings, encoding/BOM, written through the save plan (unchanged lines byte-exact); stdout by default, `--in-place` one plan with backups, `--output` one file; see §4. |
| `hikasa text find <pattern> <path>… [--regex] [--case] [--word] [--recursive] [--mask <masks>] [--include-hidden] [--archives files\|opened] [--encoding <label>] [--format <id\|name>] [--convert] [--max-matches <n>] [--max-matches-per-file <n>] [--max-file-size <size>] [--files-with-matches\|--count] [--replace <text> [--dry-run] [--yes] [--confirm …] [--backup\|--no-backup] [--unmappable refuse\|replace]]` | `hikasa-text` | Find in Files (M2 W10c, D-285); `--replace` replaces in every file found, as one save plan. |
| `hikasa table compare <left> <right> [--format] [--key <cols>] [--sheet …] [--report …]` | `hikasa-table` | *M2 (Table Compare phase 2, contract `design/m2-table-contract.md` §6.5):* every sheet of two workbooks (xlsx, xlsm, xlsb, xls, ods), two delimited or fixed-width text tables, or one of each; the flags and output are in §4. |
| `hikasa hex compare <left> <right> [--alignment none\|fast\|complete] [--block <bytes>] [--skew <bytes>] [--window <bytes>] [--budget <MiB>] [--format\|--left-format\|--right-format <id\|name>] [--unlocked-max <MiB>] [--display all\|differences\|same\|none] [--max-sections <n>] [--preview <bytes>] [--bytes-per-line <n>] [--group 1\|2\|4\|8] [--chars ascii\|latin1\|none] [--address hex\|decimal] [--context <n>] [--stdin-name <name>] [REPORT: --report summary\|side-by-side\|interleaved …] [--gui [--wait] [--no-launch]]` | `hikasa-hex` | *M2 W12c (contract §1.12.23, D-346):* the sections, or the rows, then the summary and verdict lines; `--json` `hikasa.hex_compare.v1`; exit 1, 11 (2, 13 with a converted side), 100; see §4. *M2 W13a (D-369):* `--gui` opens the window's Hex Compare view with the same words (no longer `unsupported_view`); `gui open` of two files of a Hex format and `--fv="Hex Compare"` open it too; `--wait` exits with the view's verdict at close (1, 11, 2, 13; 0 without a landed alignment; 100 when a side could not be opened). |
| `hikasa hex dump <spec> [--offset <addr>] [--length <n>] [--bytes-per-line <n>] [--group …] [--chars …] [--address …] [--values] [--format <id\|name>] [--stdin-name <name>]` | `hikasa-hex` | *M2 W12c:* rows of one file as the Hex pane shows them, and the value panel; `--json` `hikasa.hex_dump.v1`; exit 0, 100. |
| `hikasa hex find <pattern> <path>… [--hex] [--encoding utf-8\|latin1\|utf-16le\|utf-16be] [--case] [--recursive] [--mask <masks>] [--include-hidden] [--archives files\|opened] [--max-matches <n>] [--max-matches-per-file <n>] [--files-with-matches\|--count]` | `hikasa-hex` | *M2 W12c (D-334):* `spec:0xOFFSET` per hit; `--json` `hikasa.hex_find.v1`; exit 0 with or without hits, 100. |
| `hikasa picture compare <left> <right> [--mode tolerance\|range\|blend] [--tolerance <0-255>] [--blend <0-100>] [--ignore-unimportant] [--auto-scale] [--offset <dx>,<dy>] [--left-orientation <list>] [--right-orientation <list>] [--tone-map clamp\|reinhard] [--max-megapixels <n>] [--format\|--left-format\|--right-format <id\|name>] [--probe <x>,<y>]… [--diff-output <spec> [--diff-pane difference\|left\|right] [--checkerboard]] [--metadata] [--stdin-name <name>] [REPORT: --report summary\|side-by-side …] [--gui [--wait] [--no-launch]]` | `hikasa-picture` | *M2 W12c (contract §1.12.23, D-346):* a line per side, the statistics, the probes, the verdict line; `--json` `hikasa.picture_compare.v1`; exit 1, 2, 12, 13, 100; `--metadata` is refused (`unsupported`, PX-006 is P1); see §4. |
| `hikasa media compare <left> <right>` / `hikasa version compare <left> <right>` / `hikasa registry compare <left> <right>` | `hikasa-metadata` | |
| `hikasa quick <left> <right> [--size\|--crc\|--binary\|--rules]` | all | Quick compare: no output, exit code only (`--qc`). |
| `hikasa report <type> …` | `hikasa-report` | Shared report options (`13-reports.md`); every `compare` subcommand accepts `--report` as a shortcut. *M2 W7 (D-205):* `report text` and `report folder` (`[<left> <right>] [--session <REF>]`, the rule flags of the compare subcommand, REPORT); the report flags are one group shared with `text compare`, `folder compare` and `session run`; see §4. |
| `hikasa script run <file> [args…] [--silent] [--stop-on-error]` / `hikasa script check <file>` | `hikasa-script` | Run or syntax-check a script. *M2 W8 (D-226, contract §1.8.12):* `script run <FILE\|-> [<ARG>…] [--silent] [--stop-on-error] [--confirm prompt\|yes-to-all\|no-to-all] [--dry-run] [--gui [--wait] [--no-launch] [--closescript]]`, `script check <FILE\|-> [<ARG>…]`; an argument that starts with `-` follows `--`; `--silent` with `--gui` is a usage error; see §4. |
| `hikasa session (list\|show\|new\|edit\|delete\|rename\|lock\|unlock\|export\|import\|run) …` | `hikasa-session` | `run` executes a saved session headless (compare/sync/merge) and prints/reports the result. `edit` takes `--set key=value` pairs. *M2 W2 (§1.2 of the contract, D-80…D-84):* `list [--folder <ref>] [--autosaved] [--defaults]`, `show <ref>` / `show --defaults <type>`, `new <ref> --type <type> [--left] [--right] [--description]`, `edit <ref> (--set <key>=<value>… \| --unset <key>… \| --left \| --right \| --description) [--force]`, `edit --defaults <type> (--set … \| --unset …)`, `delete [--force]`, `rename <ref> <new ref> [--force]`, `lock`, `unlock`, `mkdir <folder>`, `rmdir <folder> [--recursive] [--force]`, `run <ref> [output flags of folder/text compare]`. `run` executes Folder Compare and Text Compare sessions through the same service call as `folder compare` / `text compare` (same output, JSON and exit codes, T11); other types exit 100 `unsupported`. *M2 W6c:* `run` of a Folder Sync session is `folder sync` with the session's specs, rules, `[rules.sync]` and read-only sides, and takes `--dry-run`, `--yes`, `--confirm`, `--allow-empty-source`, `--all-rows`; without a terminal and without `--yes` it exits 100, as `folder sync`. Those flags on another type are refused (exit 100). *M2 W10a:* `run` of a Text Compare session takes `--align` and `--isolate` (§4); on another type they are refused (exit 100). *M2 W12c (D-349):* `run` of a `hex-compare` session is `hex compare` with the session's words (and takes `--max-sections`, `--preview`, `--display`, `--context`), of a `picture-compare` session `picture compare` (and takes `--diff-output`, `--diff-pane`, `--checkerboard`, `--probe`, `--ignore-unimportant`); each set is refused on the other types (exit 100); only `text-edit` sessions stay `unsupported`. `--set` keys are paths inside the session file, parsed like settings keys; `--unset` removes a rule key so the session follows the defaults again (D-90); editing, deleting or renaming a locked session, and `rmdir --recursive` of a folder holding one, needs `--force` (D-91). Session export and import go through settings packages (`settings export --item sessions:<ref>`). |
| `hikasa workspace (list\|show\|delete\|rename)` | `hikasa-session` | *M2 W2 (D-85).* `show` warns `session_missing` for a tab whose session no longer exists. |
| `hikasa format (list\|show\|new\|edit\|delete\|import\|export\|enable\|disable\|move\|restore\|detect <file>\|tokens <file>)` | `hikasa-formats` | `tokens` dumps the grammar tokenization (debugging grammars). The write forms are M2 W1 (§4, `10-file-formats.md` §1.1). |
| `hikasa profile (list\|show\|new\|edit\|delete\|test) …` | `hikasa-session` + `hikasa-vfs` | Secrets prompted interactively or read from `--password-stdin`. |
| `hikasa vfs (ls\|stat\|cat\|cp\|mv\|rm\|mkdir\|touch) <spec>…` | `hikasa-vfs` | Generic access to any back-end (archives, remote, snapshots). *M2 W3 (D-102):* `cp`, `mv`, `rm`, `mkdir`, `touch` over the generic plan/execute layer (`hikasa_vfs::ops`); `--dry-run` prints the plan, `rm` needs `--yes`; see §4. *M2 W5 (D-152):* `vfs ls a.zip`, `a.zip/` and `a.zip/inner` list members (`SpecUse::Folder`), `vfs stat a.zip` is the file and `a.zip/` the root, `vfs cat a.zip/x` the member; `<refused-N>` and `<damaged>` are the listing's entry errors; a copy out of an archive only reads it and skips link members. |
| `hikasa settings (get\|set\|unset\|list\|export\|import\|restore-defaults)` | `hikasa-session` | Every settings file, keys `<file>.<path>` (D-73). `get`/`set`/`unset`/`list`/`restore-defaults` are M2 W1 (§4). *M2 W2 (D-86, D-87):* `export -o <file.hikasa-pkg> [--category <cat>…] [--item <cat>:<name>…]`; `import <file> [--category] [--item] [--delete-existing sessions\|formats\|workspaces] [--force] (--dry-run \| --yes \| --silent)` validates every entry before writing anything and prints the plan (`delete`, `add`/`replace`/`skip_locked`, a summary per category); `restore-defaults --category <cat>… [--delete-sessions [--force]] [--delete-workspaces]` restores factory defaults by category. A locked named session is neither replaced nor deleted by either command unless `--force` (plan action `skip_locked`, warning `session_locked`, D-91). |
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
  the GUI instead of printing it (e.g. `hikasa text compare a b --gui`). *M2 W6c (D-190):* `folder sync --gui [--wait]
  [--no-launch]` checks its words here, then sends `OpenView { kind: folder_sync }` with the sync in
  `OpenOptions::sync`; until the Folder Sync view (W9) the GUI answers `unsupported_view`, "this window cannot show a Folder
  Sync view yet", and the command exits 100. *M2 W9b (D-248, replaces that answer):* the GUI opens the Folder Sync view
  (the words apply over the `folder-sync` defaults' options); `--wait` exits with the view's code: its last Run's (0, or
  100 when an item failed, did not run or the sync was stopped), else 0 when the comparison completed and 100 when it did
  not. `gui list` names the kind `folder_sync`.
- *M1 notes (D-22, D-49, D-50):* `gui ping|list|close|quit` never start the GUI; when
  none is running they exit 100 ("not running"). Only `gui open` and `--gui` start it.
  `gui open -` spools standard input to `<cache>/stdin/<n>/<name>` (the name from
  `--stdin-name`, else `stdin.txt`) and sends that absolute path; with `--wait` the file is
  deleted after the view closes. If the waiting CLI is interrupted (Ctrl-C, a signal), the
  spooled file and its numbered folder stay in `<cache>/stdin/` and may be deleted by hand
  (`gui open` runs no job, so no handler is armed, D-159). *M2 W3 (D-101, revises D-49):* `gui open clipboard://`
  and `--gui` with `clipboard://` send `clipboard://` verbatim; the GUI reads **its own**
  clipboard when it opens the view (the CLI never reads it, so it may run headless); no or an
  empty clipboard in the GUI is answered `invalid_request` (exit 100). Rules files may still not
  be `-` or `clipboard://`, and sessions never store either.
  `--gui` on `text compare` and `folder compare` never compares locally: it sends the same
  specs, rules files, rule flags and view options (`--display`, `--context`, `--folders`,
  `--suppress-filters`, `--expand-all`) to the GUI, after checking every word locally (a
  typo exits 100 before the GUI is contacted); `--wait` and `--no-launch` work as for
  `gui open`. The report flags (`--report`, `--output`, `--html`, …) cannot be combined
  with `--gui`. In M1 `gui session`, `gui workspace` and `gui script` are not available.
- *M2 W2 notes (D-88, D-89):* `gui session <ref> [--wait] [--no-launch]` and `gui workspace <name>
  [--no-launch]` find the session or workspace locally (unknown: exit 100 `not_found`; a session type the
  window cannot show yet: `unsupported`) and send `LoadSession` / `LoadWorkspace`; both print
  `hikasa.gui_open.v1` (a workspace adds `views`, every view it opened). A window of 0.1 answers
  `unsupported_request`, reported as "the running GUI does not support sessions; restart it" (exit 100).
- *M2 W8b notes (D-228, contract §1.8.13):* `gui script <FILE> [<ARG>…] [--wait] [--no-launch] [--stop-on-error]
  [--confirm <MODE>] [--closescript] [--dry-run] [--silent]` locates the script as `script run` does (§1.8.11 of the contract: the
  current folder, then `[tweaks.scripts] shared_scripts_folder`), reads and checks it here (105, 106, and `output-to:-`
  refused: the window has no standard output) before the GUI is contacted, captures the environment variables the
  script references, and sends `RunScript` with the script's absolute path, the arguments, this cwd and those
  variables (a value that is not UTF-8: exit 100, "… cannot be sent to the GUI: not valid UTF-8; use hikasa script
  run"); connecting or launching as `gui open` (102, 103). The window runs it with its status window; exit 0 once it
  started, with `--wait` the script's exit code from `ScriptFinished` (102 when the wait is lost). A window that runs a
  script answers `busy` (exit 100); an older window `unsupported_request`, "the running GUI does not support scripts;
  restart it" (exit 100). `--dry-run` runs it in the window as `script run --dry-run` (a window whose `Hello` does not
  list `run_script.dry_run` would run it for real, so it is refused first, exit 100). `--silent` never goes through a
  GUI: the script runs headless, as `script run --silent` (Ctrl-C included; `--json` prints `hikasa.script.v1`).
  `--json`: `hikasa.gui_script.v1`. *CLI-G03:* `script run … --gui` is `gui script …` (with `--dry-run` too).

## 2. Legacy positional forms and switches (accepted by `hikasa gui open` and `hikasa-gui`)

*M1 note (D-51):* one parser in the session layer (`normalize_legacy_args`,
`parse_open_args`) reads these forms for `hikasa gui open` and for `hikasa-gui`, so both
accept exactly the same command lines. Recognised switches that M1 does not implement
(`--automerge`, `--center`, `--qc`, `--vcs1`, …) are forwarded to the GUI with warning
`switch_ignored`; unknown switches exit 100. *M2 W10b (D-278):* `--edit` and `--savetarget` are
implemented and no longer forwarded.

- **CLI-001** (P0): Switches accept `--name`, `-name` and Windows-style `/name`
  spellings; values with `=`; quote arguments containing spaces. `--help` prints usage.
- *M2 W2 interpretation of CLI-002 (D-88):* one positional argument that is **not an existing path** is
  resolved as a named session (the exact reference, then case-insensitively) → `LoadSession`, else a
  workspace → `LoadWorkspace`, else the M1 error; an existing path always wins over a name. An existing file
  whose name ends in `.hikasa-pkg` (any case) is a settings package: with `--silent` it is imported headless
  (every category and item, no deletes; `hikasa.settings_import.v1`), without it the window shows its import wizard
  (`ImportPackage`, D-127; exit 0, also from `hikasa-gui <file.hikasa-pkg>`, forwarded to a running GUI). `hikasa gui open` and,
  since W4b (D-127), the window's own command line serve these forms: `hikasa-gui <session>` and `hikasa-gui
  <workspace>` open them, or forward `LoadSession`/`LoadWorkspace` to a running GUI; an existing path always wins.
- **CLI-002** (P0): Positional arguments:
  - a **named session** → opens it;
  - a **named workspace** → loads it;
  - **two folders** → Folder Compare; **two files** → the file view chosen by file
    formats (*Table Compare phase 3, T-54:* two files whose format is of the Table kind open Table Compare); **three files** → Text Merge (left, right, center); **four files** → Text
    Merge with output;
  - `@script.txt` → runs the script (`§3`); *M2 W8b (D-228):* in the shared parser the first positional argument
    starting with `@` names the script and every other positional argument is its `%1`… (never a comparison); `@`
    alone is a usage error; a file whose name starts with `@` is `./@name`. `hikasa gui open @s.txt a b [--wait]
    [--closescript]` is `gui script s.txt a b …`; `hikasa-gui @s.txt a b` starts the window with its status window, or,
    with a GUI running, forwards `RunScript` (its cwd and the referenced variables) without waiting and exits 0; with
    `--silent` neither goes through a GUI: the process runs the script headless (`hikasa-gui` opens no window), its log
    stream on stderr, Ctrl-C as for `script run`, and exits with the script's code; a view switch given with a script
    (`--ro`, `--sync`, `--nobackups`, …) is `switch_ignored`, and so is `--silent` with neither a script nor a settings
    package;
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
| `--closescript` | Close the script status window when finished (overrides option). *M2 W8b:* applies to an `@script` (after any run, whatever its exit code); no longer `switch_ignored` (without a script it warns that it applies to a script only). |
| `--silent` | No windows, no task-bar entry; packages import everything; scripts log errors instead of showing dialogs. *M2 W8b:* an `@script` with `--silent` runs headless, never through a GUI; no longer `switch_ignored`. |
| `--edit` | Open a Text Edit view. *M2 W10b (D-278):* each file spec opens a Text Edit view (`ViewKind::TextEdit`, one spec per `OpenView`; two specs, two views; `--wait` waits for every view and exits with the highest code: 0, or 100 when a file failed to load); with `--fv` or `--sync` the last wins (D-190); no longer `switch_ignored`. The window answers `unsupported_view` until W11's Text Edit view. *M2 W11b (D-314):* the window opens the Text Edit views (one per spec); `--ro*`, `--savetarget` and `--nobackups` reach them; `gui list` shows `text_edit`. *W11b review fixes:* `--savetarget` with two `--edit` files or more is refused by `check_open` before the window is contacted (exit 100, `invalid_input`). |
| `--sync` | Open a Folder Sync view for the two folders. *M2 W6c (D-190):* parsed (with `--fv` the last wins); the view is W9's, so the GUI answers `unsupported_view` until then. *M2 W9b (D-248):* the view opens; a waited one exits with its last Run's code (see CLI-G03). |
| `--expandall` | Expand all subfolders in the initial folder comparison. |
| `--filters=<masks>` | Name filter for the initial folder comparison. |
| `--fv=<type>`, `--fileviewer=<type>` | Force the view type: `Text Compare`, `Text Merge`, `Table Compare`, `Hex Compare`, `Media Compare`, `Picture Compare`, `Registry Compare`, `Version Compare`, `Folder Compare`, `Folder Merge`, `Folder Sync`, `Text Edit`, `Text Patch`. |
| `--nobackups` | Do not create backup files. *M2 W6c:* `OpenOptions::no_backups` (additive, left out of the wire while off). *M2 W6d:* the operations of that view keep no backups, whatever `[backups]` says. *M2 W10b (D-278):* also no backup on any save of that view (`LiveOptions::no_backups`). |
| `--qc[=size|crc|binary]`, `--quickcompare[=…]` | Quick compare two files and set the exit code; no type = rules-based. |
| `--ro`, `--readonly` / `--ro1`, `--lro`, `--leftreadonly` / `--ro2`, `--rro`, `--rightreadonly` | Disable editing on all / left / right side. *M2 W6d:* a Folder Compare view turns off the file operations that would write such a side, and the Service refuses them too. *M2 W10b (D-278):* a text view's `CommandLine` block on those panes (`LiveOptions::from_open`); a Text Edit view's file is side 1 (`--ro1`). |
| `--savetarget=<file>` | Save command writes to this file instead of the original. *M2 W10b (D-278):* `OpenOptions::save_target` (additive, left out of the wire when absent), an absolute canonical spec made from the caller's current folder before sending (`save_target_spec`; Windows drive, UNC and `\\?\` spellings; `-` and `clipboard://` exit 100); the saves of a Text Compare's right pane and of a Text Edit view's file write it (the left pane saves in place); with `--wait` the view's outcome is still its verdict; no longer `switch_ignored`. |
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
  - *M2 W12c (D-331, D-339):* `hex compare` 1 `binary_same`, 11 `binary_different`, and with a side read through its
    format's load conversion 2 (the converted outputs identical) or 13 (they differ); never 12. `picture compare` 1
    identical bytes, 2 every pixel same, 12 similar pixels only (2 with `--ignore-unimportant`), 13 a different
    pixel. `hex dump` 0; `hex find` 0 whether or not something matched (D-285's rule). Any error 100; with `--gui` the
    codes of `gui open`. `quick --rules` of a picture pair gives `picture compare`'s codes (11 after the fallback to
    bytes of an unsupported container).
  - A command line that does not parse exits **100**, never 2 (2 means "rules-based
    same"); `--help` and `--version` exit 0. A panic is caught and exits **100**, never
    101 (101 means "conflicts, output not written"). A broken pipe on stdout stops the
    output and keeps the exit code that was computed.
  - *M2, Ctrl-C (D-159):* while a command runs a job (a comparison, `vfs ls`, `cat`, `cp`,
    `mv`, `rm`, `mkdir`, `touch`), the first Ctrl-C cancels it: a rebuild aborts, removes its
    temporary and lock files and leaves the archive as it was; a `vfs` operation reports its
    items as usual, those not reached as `not_run`. The command exits **100**. A second
    Ctrl-C exits 100 at once, as a kill would: a rebuild it stops leaves its files, which the
    next rebuild of that archive removes. A cancelled comparison writes no report, so `--output`
    keeps what it had, and a process started with Ctrl-C ignored (`cmd &` in a script) stays
    that way. A command without a job ends at the first Ctrl-C,
    by the signal; SIGTERM always does.
  - *M2 W10b (D-280):* `text convert`: 0 when every file is converted or skipped (`no_change`, declined), 100 when a
    file fails (the others are still written: `ok: true`), after a Ctrl-C (a file being written stays as it was, the
    files not reached are `not_run`), and for a refused request (nothing to convert, several files to stdout,
    `--output` with several files, an unknown label or format, open questions without a terminal: nothing written,
    `ok: false`).
  - *M2 W10c (D-285):* `text find`: 0 whether or not anything matched (also after `--dry-run`); 100 for an invalid
    pattern or replacement, a missing path, a bad word or limit, an unknown encoding or format, a Ctrl-C, and with
    `--replace` open questions without a terminal or a declined operation (nothing written, `ok: false`) and a file
    that failed (the others are still written: `ok: true`). A broken pipe stops the search and keeps exit 0.

- **CLI-005** (P0): Quick Compare dialog on startup (program option): when launched
  with two files, show a summary dialog (binary or rules-based) instead of the view;
  option "Open view automatically if files are different".
  *M2 W13c notes (D-368):* shown for an open of two files whose kind was inferred (the process's arguments, `hikasa
  gui open a b`, the bare form; `OpenOptions::inferred`, set by `resolve_open`) without view words, with `[startup]
  show_quick_compare_dialog` on; not for `--fv`, `<kind> compare --gui`, `--edit`, view words, two folders, a session,
  a workspace or a script, nor for files opened in a window (a drop, Home's Compare, a folder row, Open With). Both specs with size and time, the method (`quick_compare_method`), `Service::quick`'s result
  and the view *Open View* opens; `open_view_if_different` opens the view at once for a result that is not the same; the
  check writes the option. `--wait`: *Close* answers the quick exit code (1, 2, 11, 12, 13; 100 for an error; no
  verdict, 0, after *Stop*), *Open View* the view's at its close. An error result stays in the dialog; the window's
  close button stops the comparison and closes the dialog.
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

*M2 note (W7, D-196):* the words of `layout:`, `options:` and `output-options:` of `text-report`, `folder-report`
and `file-report` are the report vocabulary of `13-reports.md` ("M2 reports"), which adds `no-line-numbers` and
`display-none` to SCR-001's words; `output-to:` takes a file, `clipboard`, `printer` or `browser` as `--output-to`
does. The interpreter itself is W8's.

*Table Compare phase 2 (contract §6.3):* `table-report` (alias `data-report`) writes a table report: `table-report
[layout:<summary|side-by-side|interleaved|xml|json>] [options:…] [output-options:…] [title:…] (<left> <right> |
<table-compare session>) output-to:<target>`, the words of tables in `13-reports.md` (*Table Compare reports*); it
needs no loaded comparison. The other `*-report` commands of later views stay refused.

*M2 W12c (contract §1.12.25, D-348):* `hex-report [layout:<summary|side-by-side|interleaved>] [options:…]
[output-options:…] [title:…] (<left> <right> | <hex-compare session>) output-to:<target>` and `picture-report
[layout:<summary|side-by-side>] … (<left> <right> | <picture-compare session>) output-to:<target>` write the reports of
`hex compare --report` and `picture compare --report` (the words of *Hex Compare reports* and *Picture Compare
reports* in `13-reports.md`; a picture side-by-side report is HTML: `options:html-color` or `html-mono`); a session of
another type fails the command. `media-report`, `version-report` and `registry-report` stay refused.

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
  completed in <time>". *M2 W8b (D-229):* the minimal window of contract §1.8.14: the script's path, "Command <i> of
  <n>, line <l>: <text>", the current call's progress, the log lines (the last `[tweaks.log_panel] display_up_to`),
  the end line ("Script completed in <time>", "Script stopped at line <n>", "Script failed (exit <code>)"), *Stop*
  while it runs (as Ctrl-C), *Close* after; W6's confirmation dialogs and the stop-on-error Yes/No question over it;
  an end notice and the log panel's entries. Tools ▸ Run Script… (catalogue `RunScript`) checks the script first and
  shows its errors with their positions.

*M2 interpretation (W8, D-214…D-231; contract §1.8).* The language is `hikasa-script`; every command means its CLI
command (`folder op`, `folder sync`, `report`) through the same Service calls (T11).

- **Lexing (§1.8.3):** a BOM is skipped; LF, CRLF and CR end lines; words split at spaces and tabs; `"` quotes any part
  of a word and `""` inside quotes is one `"`; a backslash is literal; `#` where a word could start begins a comment; a
  trailing `&` outside quotes continues the line (comment lines between are skipped, a continuation before a blank
  line or the end is an error); invalid UTF-8 is one error at the first bad byte; 16 MiB at most (105).
- **Expansion (§1.8.4):** after the split, never re-lexed: `%1`…`%9` (`%10` is `%1` then `0`), `%0` the script as
  located, `%%`, `%date%`/`%time%`/`%fn_time%` (one instant per command), `%NAME%` (case-insensitive on Windows only);
  any other `%` is itself. A missing argument or an unset variable is 106 before anything runs. `lt`/`rt` are `left`/
  `right` wherever a side is expected, never inside a path, a mask or a quoted word.
- **State (§1.8.5):** one invisible folder comparison: its settings (from the folder-compare defaults), the last
  `load`, the open folders, the selection, the confirm mode, stop on error, the log. A `load` starts from nothing;
  a session's `[display]` is not applied (SES-010).
- **Commands (§1.8.6):** the table of contract §1.8.6 maps each command to its CLI request; `snapshot`, the table,
  hex, picture, media, version and registry reports are refused with their message ("… not available in this
  version") and the run goes on.
- **Errors (§1.8.7):** 105 the script cannot be read, 106 it does not check (every error with its position, nothing
  runs), 107 a base cannot be loaded (the run ends there), 100 a failed or refused command, a declined question or a
  cancel; stop on error ends at the first failure (on a terminal or in the window it asks first).
- **Confirmation and silent mode (§1.8.8):** `option confirm:` and `--confirm` pick W6's mode; `prompt` asks on a
  terminal or in the window; silent declines with "Attempted to open a dialog when running silently: …"; no terminal
  declines with its own message.
- **The log (§1.8.9):** `<date> <time> <kind> <where> <text>`, kinds `run`, `info`, `detail`, `warning`, `error`; the
  stream on stderr (or the status window) without the date; `log [append:]<file>` a local file.
- **Lookup (§1.8.11):** the script as given, else (relative and absent) below `[tweaks.scripts] shared_scripts_folder`.

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

*M2 note (W8, A8.11):* example 2 ends with exit **107** at line 4, "sftp://deploy@host/var/www: remote folders are not
available in this version" (the log file holds lines 1–4, the `sync` is not run); example 4 ends with exit **107** at
line 3, "…/project.hksnap: snapshots are not available in this version", nothing written. With a folder copy of the
project as its right side, example 4 exits 100: the zip holds the left-only and differing files, and `snapshot` is
refused with its message. Examples 1 and 3 run as written (exit 0).

## 4. The M1 command line (binding interpretation; contract §1.9, §2.21, §4; decisions D-29, D-32, D-35, D-40, D-50)

M1 implements the subcommands below in the `hikasa` binary. Every subcommand is a thin
adapter over the session service: vocabulary words (`--display`, `--criteria`,
`--unimportant`, `--report`, …) are parsed by the core crates, so the CLI, scripts and the
GUI read the same words. Other rows of the table in §1 (sync, merge, patch, table, hex,
picture, script, session, profile, settings, `vfs cp`/`mv`/…) come in later milestones (M2 adds
the `settings`, `session`, `workspace` and `vfs cp`/`mv`/`rm`/`mkdir`/`touch` rows below).

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
| `quick <left> <right>` | `--size` \| `--crc` \| `--binary` \| `--rules` (default); `--ignore-unimportant` (`--iu`); `--format <id\|name>`; `--rules-file <toml>`; `--unimportant <list>`, `--unimportant-text <text>`…, `--unimportant-regex <re>`…, `--compare-line-endings`; `--stdin-name <name>`. Prints nothing; the exit code is the result. `--size` compares only the sizes and `--crc` only the CRC-32 checksums: equal sizes (or checksums) report `binary_same` and exit 1 as specified (contract §2.21) although the bytes were not compared one by one; `--binary` and `--rules` read and compare the contents. *M2 W12c (D-344, contract §1.12.21):* with `--rules` the pair's format kind (D-15: the left name's, else the right's) picks the engine: Table formats the table engine (T-21), **Picture** formats `hikasa_picture::PictureComparator` (the pixels, with the tolerance, Auto Scale, tone map and `max_megapixels` of the Picture Compare defaults session `sessions/_defaults/picture-compare.toml`, else the built-in defaults: a PNG saved again as BMP is 2, a JPEG within the tolerance 12; a container the decoders do not support, such as DDS or AVIF, is compared by bytes with `fell_back_to_binary`; a decode error, `invalid_data` or `too_large`, is exit 100, never a fallback; files over 512 MiB by bytes with `rules_size_limit`), **Hex** formats by bytes (a hex verdict is the binary one, D-331), every other kind the text rules; `--binary`, `--crc`, `--size` are unchanged |
| `text compare <left> <right>` | `--format`; `--encoding`, `--left-encoding`, `--right-encoding <label>`; `--rules-file` (`--rules`); `--algorithm auto\|unaligned\|standard\|myers\|patience\|histogram`, `--skew <n>`, `--closeness`/`--no-closeness`, `--closeness-threshold <‰>`, `--never-align`; `--unimportant`, `--unimportant-text`, `--unimportant-regex`, `--compare-line-endings`, `--orphans-unimportant`; `--ignore-unimportant` (`--iu`); `--context <n>` (default 3, from the options), `--display all\|mismatches\|context\|matches`; `--report unified\|side-by-side\|summary\|patch\|statistics` (default: the annotated unified diff), `--patch-style normal\|context\|unified`, `--html`, `--html-mono`, `--line-numbers`/`--no-line-numbers`, `--width <n>` (160), `--wrap none\|character\|word`, `--title <text>`, `--output <file>`; `--stdin-name`; `--gui`, `--wait`, `--no-launch`. *M2 W10a (D-266, §1.10.8 of the contract):* `--align <L>:<R>` (repeatable; `L` and `R` are 1-based lines or ranges `a-b`: `--align 10:12`, `--align 10-14:12-13`) makes those lines a segment of their own; `--isolate left\|right:<a>[-<b>]` (repeatable) makes lines align with nothing (always deleted or inserted). The words are parsed by core (`parse_anchor`); a malformed word or a line beyond a document is exit 100 (`invalid_input`, before anything is compared); a pair that overlaps or crosses an earlier kept one is dropped with the warning `anchor_dropped`. `report text` and `session run` of a Text Compare session take both flags (with the rule flags, one group, D-205); `session run` of another type and `--gui` refuse them (exit 100). *M2 W10b (D-281, §1.10.16):* `--left-format <id\|name>`, `--right-format <id\|name>` give one file its own format (also `Everything Else`); the other keeps the pair's (D-15); with `--format` exit 100 (`invalid_input`). *M2 W10b (D-282, §1.10.17):* `--ignored left\|right:<a>[-<b>]` (repeatable, 1-based) marks ignored lines: a row with a marked line counts as same for the verdict, the exit code, the statistics, the display and the reports (alignment unchanged); the words are parsed by core (`parse_ignored`); a malformed word or a line beyond a document is exit 100. `report text` and `session run` take `--ignored` (on a session it replaces the session's marks of the side it names); `session run` of another type and `--gui` refuse it, `--gui` also the per-side formats (exit 100). *M2 W11b (D-314, contract §1.11.23):* `--gui` accepts `--left-format`, `--right-format`, `--align`, `--isolate` and `--ignored`: they are checked here (format names, the words' syntax; `--format` with a per-side format is exit 100) before the GUI is contacted and sent as `OpenOptions` `formats`, `anchors` and `ignored`; the window applies them once the files are loaded, and a word whose lines are beyond a document is dropped with a warning notice (the view opens) |
| `folder compare <left> <right>` | `--filters <masks>`; `--rules-file` (`--rules`); `--criteria <list>`; `--size`/`--no-size`, `--timestamps`/`--no-timestamps`, `--tolerance <secs>`, `--ignore-dst`, `--ignore-timezone`, `--filename-case`, `--attributes <letters>`, `--permissions`, `--owner`, `--group`; `--content none\|crc\|binary\|rules`, `--skip-if-quick-same`, `--no-override-quick`; `--text-rules-file <toml>`, `--unimportant`, `--unimportant-text`, `--unimportant-regex` (for `--content rules`); `--case-sensitive`\|`--case-insensitive`, `--align-extensions`, `--align-normalization`; `--cutoff <expr>`, `--size-filter <expr>`, `--include-protected`; `--follow-symlinks`, `--flatten`, `--no-scan-orphans`; `--display <filter>` (default `differences`), `--folders always\|structure\|files-only` (default `structure`), `--ignore-unimportant` (`--iu`), `--suppress-filters`, `--expand-all`; `--report list\|side-by-side\|summary\|json` (default `list`), `--columns <list>`, `--html`, `--html-mono`, `--title`, `--output`; `--gui`, `--wait`, `--no-launch` |
| REPORT (`text compare`, `folder compare`, `session run`, `report text`, `report folder`) | *M2 W7 (D-205, §1.7.15 of the contract):* `--layout <word>` (alias `--report`), `--options <word,…>`…, `--output-options <word,…>`…, `--output-to <target>` (`-`, `clipboard`, `clipboard://`, `printer`, `browser`, or a file; `./printer` is a file), `--output <file>` (as `--output-to <file>`; both are a usage error), `--title <text>`, `--display <word>`, `--folders <mode>`, `--suppress-filters`, `--context <n>`, `--patch-style <s>`, `--columns <list>`, `--html` \| `--html-mono`, `--line-numbers` \| `--no-line-numbers`, `--width <n>`, `--wrap <mode>`, `--lines [left:\|right:]<a>-<b>`, `--select <mask>`…, `--path <rel>`…, `--dry-run`. One group (`ReportArgs`) in the five commands; a flag only one kind takes is refused for the other by the service (exit 100), not by the parser. `--html`, `--html-mono`, `--wrap`, `--patch-style`, `--columns`, `--line-numbers` and `--no-line-numbers` are words of the vocabulary (`13-reports.md`), so they conflict with a different word of their slot; `--display`, `--folders`, `--suppress-filters`, `--context` and `--width` are what an absent word means. Every word is checked before the comparison runs |
| `report text [<left> <right>]` / `report folder [<left> <right>]` | *M2 W7 (D-205):* `--session <REF>` (a Text Compare session for `text`; a Folder Compare or Folder Sync session for `folder`), with the flags on top; the rule flags of `text compare` (`report text`) or of `folder compare` and `--ignore-unimportant` (`report folder`); REPORT, the layout `side-by-side` by default and every other default the compare subcommand's, so `report text a b <flags>` and `text compare a b --report side-by-side <flags>` print the same bytes and exit alike. `--json`: `hikasa.report.v1`, refused with the stdout target (exit 100, "the report and the JSON document cannot share stdout: pass --output-to"). Exit codes: the comparison's; 100 for a refused word, combination or target, a write that fails (target untouched), a stylesheet that cannot be read, `too_large`, a launcher that fails, a cancel; 107 where the compare subcommand gives it. A pair report that fails is a warning (`file_report_failed`) |
| `text convert <file>…` | *M2 W10b (D-280, §1.10.15 of the contract):* at least one conversion flag (`--trim-trailing`, `--tabs-to-spaces` \| `--spaces-to-tabs`, `--line-endings lf\|crlf\|cr`, `--encoding <label>`, `--bom` \| `--no-bom`), else exit 100 ("nothing to convert"); `--tab-size <1-32>` (default the format's), `--lines <a>-<b>` (1-based lines of the file; the trim, tab and line-ending conversions only), `--format <id\|name>`, `--encoding-in <label>`, `--stdin-name <name>` (`-` reads standard input). Each file is loaded as `text compare` loads it and converted in the order trim, tabs, line endings, encoding and BOM (the BOM stays unless asked, and only for an encoding that has one; `--bom` with one that has none fails the file); a file with nothing to change is skipped (`no_change`) and not rewritten in place, while stdout gets its bytes as they are and `--output` a copy (W10b note 54). The conversions run line by line while the result is written: a file is held once (W10b note 60). `--in-place` refuses what is not a regular file before reading it (a FIFO never blocks it, note 65). Stdout holds up to 64 MiB until the file's check at the end of its read (a changed file writes nothing); a larger result streams, and a change found at its end leaves partial output with exit 100 (note 64). A binary file, a *Sorted* or save-less *External* format (FF-031), a cut document and a member of a read-only archive fail their file. Targets: stdout by default (one file; the summary on stderr; `--json` needs `--dry-run` there); `--in-place` (one save plan for every file, backups by `[backups] back_up_before_save` unless `--no-backup`, `--backup` forces them); `--output <file>` (one input; an existing file asks `replace_existing` and is backed up). Questions (§1.10.14): `--yes` (`-y`) answers `read_only_target`, `replace_existing`, `overwrite_backup`, `lossy_rewritten`; `--force` answers `changed_on_disk`; `--confirm prompt\|yes-to-all\|no-to-all` as `folder op`; without a terminal an open question refuses the command before anything is written (exit 100, the counts named). `--unmappable refuse\|replace` (default without a terminal: refuse; a terminal asks; stdout never asks): `replace` writes `?` (warning `unmappable_replaced`), never numeric references. `--dry-run` prints the plan. Human output: `converted <n> lines: <file>` (`would convert …`), `no change: <file>`, `failed: <file>: <reason>`; `--verbose` adds the plan's `write <target> (<bytes> bytes, <encoding>[ BOM], <eol>[, backup <spec>][, via <link>])` lines. Exit 0 when every file is done or skipped, else 100 (also a Ctrl-C); a run that wrote with failed files is `ok: true` with exit 100, a refused request `ok: false`. `--json`: `hikasa.text_convert.v1` |
| `table compare <left> <right>` | *Table Compare phase 2 (contract `design/m2-table-contract.md` §6.5, T-1…T-67):* formats `--format <id\|name>` (both sides), `--left-format`, `--right-format` (each a format of the table kind; by default each side's own name picks its format, so a CSV compares with an xlsx); parser flags (they replace the formats' settings on both sides; workbook sides ignore them) `--type detect\|delimited\|fixed\|spreadsheet`, `--delimiter <d>` (repeatable; a character, several written together, `tab`, `space`, `comma`, `semicolon`, `colon`, `pipe`, `u+XXXX`), `--qualifier <c>\|none`, `--consecutive`, `--trim`, `--widths <w;w;…>`, `--first-line detect\|names\|data`, `--regional system\|default` (the A16 table of the locale's `LC_ALL`/`LC_NUMERIC`/`LC_TIME`/`LANG`; with none of them set, Windows' `HKCU\Control Panel\International` (`LocaleName`, `sDecimal`, `sThousand`, `sShortDate`) and macOS' `AppleLocale`, read once through the host, T-45), `--decimal <c>`, `--thousands <c>\|none`, `--date-order ymd\|dmy\|mdy`, `--date-sep <c>\|any`, `--encoding <label>`, `--ctrl-z`, `--stdin-name`; each parser flag also per side, on top of the both-sides flag: `--left-type`, `--left-delimiter`, …, `--left-ctrl-z` and `--right-…` (T-43); `--type fixed` needs widths (`--widths`, or the format's); the *Comma Separated Values* preset prefers `,` and reads a file the comma does not split with `;`, tab or `\|` when one of them does (T-44), while a given `--delimiter` is used as given and a file whose every row is then one field warns `delimiter_mismatch`; sheets `--sheets by-name\|left-name\|right-name\|position`, `--sheet <L>=<R>` (repeatable; exactly these pairs); columns `--columns auto\|number\|left-name\|right-name`, `--align-column <L>=<R>` (repeatable); rules (every sheet pair; a column a sheet lacks is left out of it with `rule_not_applied`, keys that do not all resolve leave the sheet unkeyed with `keys_not_applied`, a column no sheet has is exit 100, T-30; per-sheet rules of a session are strict in what they write, lenient in what they inherit, T-47) `--key <cols>`, `--unimportant <cols>`, `--text <cols>`, `--number <cols>`, `--ignore-case [<cols>]`, `--ignore-whitespace [<cols>]`, `--tolerance [<cols>=]<n>[%]`, `--date-tolerance [<cols>=]<n>[ms\|s\|m\|h\|d]` (repeatable), `--exact-numbers [<cols>]`, `--hide <cols>`, `--formulas`, `--ignore-hidden-rows`, `--ignore-hidden-columns`; rows `--algorithm unaligned\|standard\|myers\|patience\|histogram`, `--skew <n>`, `--closeness`/`--no-closeness`, `--closeness-threshold <‰>`, `--never-align`, `--sort-rows`; output `--ignore-unimportant` (`--iu`), `--display all\|differences\|same\|none` (default differences; a report's default is all), `--cells differing\|all` (JSON: `--json`, or `--report json` where it is the report word `cells-*`), `--max-rows <n>` (the human output; default 20, 0 every row); an output flag the output does not use is exit 100 (T-46: `--cells` without JSON, `--max-rows` with `--json` or a report, `--width` outside side-by-side and interleaved text, `--line-numbers`/`--no-line-numbers` with summary or json, `--title` with json, `--display` with summary), REPORT's flags of tables (`--report summary\|side-by-side\|interleaved\|xml\|json` (`--layout`), `--options`, `--output-options`, `--html`, `--html-mono`, `--line-numbers`/`--no-line-numbers` (`--no-row-numbers`), `--width`, `--title`, `--output`, `--output-to`, `--dry-run`), `--json`; `--gui` (`--wait`, `--no-launch`) sends the words to the window's Table Compare view (*phase 3, T-52*: `OpenOptions::table`, checked here first; the report flags and `--cells` do not apply, exit 100; the codes are `gui open`'s). Column references (T-12): a header name (`name:<text>` forces one), a 1-based number (`3`, `#3`), column letters (`C`); lists comma-separated, `\,` a comma in a name. Default human output: one line per sheet pair (`= Notes → notes`, `≠ Prices: 2 rows differ, 1 left only, 1 right only; cells: 1 important, 2 unimportant`, `~ …` unimportant only, `< Old (left only)`, `> …`), then the rows the display shows as `  row <L>/<R>: <column>: <left> → <right>` (unimportant cells marked; orphan rows with their cells; columns of `--hide` left out; under `--ignore-unimportant` a changed row with unimportant cells only is `same (unimportant differences)`; at most 20 per sheet unless `--display all` or `--max-rows`, then `… N more rows`), then the summary line (`important differences: 2 of 3 sheets differ; rows: …`; under `--ignore-unimportant` a sheet with unimportant differences only does not differ and the rows with unimportant differences only are counted apart, T-50). Exit codes (T-16): 1 identical bytes, 2 same under the rules, 12 unimportant differences only (2 with `--ignore-unimportant`), 13 important; never 11; 100 for a bad word, an unknown or non-table format, a column or sheet that names nothing (words before anything is read), an input that cannot be read, a damaged workbook (`invalid_data`), a password-protected one (`unsupported`: "the workbook is password-protected; Hikasa does not decrypt workbooks"; the same kind and message when a text view converts a workbook, T-48), a Ctrl-C. `--json`: `hikasa.table_compare.v1`. `session run` of a `table-compare` session runs it with the session's words (`[rules]`, `[rules.parse]`, `[rules.left_parse]`, `[rules.right_parse]`, per-sheet `[[rules.sheet]]`; `[specs] left_encoding`/`right_encoding` each side's), its `[display] filter`, `--ignore-unimportant`, `--display` and REPORT (refusing `--wrap`, `--context` and the other flags of other kinds); `session edit --set rules.<key>=<value>` (and `--defaults table-compare`) takes every table rule key, a list also as one bare word, and checks the words (T-49). `quick --rules` and the rules-based content test of `folder compare` compare table files through the same engine (T-21; two files with identical bytes are `binary_same` before they are read, even when they do not load, T-48) |
| `hex compare <left> <right>` | *M2 W12c (contract §1.12.23, D-346):* alignment `--alignment none\|fast\|complete` (default **fast**), `--block <bytes>` (Fast's average block, a power of two 1K…1M, default 8K), `--skew <bytes>` (≥ 1, default 16M), `--window <bytes>` (Complete's window 4K…16M, default 256K), `--budget <MiB>` (Complete's budget 1…1,048,576, default 1024; exhausted: the rest laid out as Fast lays out large gaps, warning `alignment_degraded`); sizes in bytes or with `K`/`M`/`G`/`KiB`… (binary units, `parse_byte_size`); loads `--unlocked-max <MiB>` (0…1024, default 16: files up to it are read whole and released, larger ones read by pages); formats `--format`, `--left-format`, `--right-format <id\|name>` (of the hex kind; `--format` with a side's own is refused), `--stdin-name`; human output `--display all\|differences\|same\|none` (default differences), `--max-sections <n>` (default 20, 0 all), `--preview <bytes>` (0…4096, default 16); row layout `--bytes-per-line <n>` (1…256, default 16), `--group 1\|2\|4\|8`, `--chars ascii\|latin1\|none`, `--address hex\|decimal`, `--context <n>` (0…64; the rows of `--display all\|same` and the reports); REPORT's flags of hex (`--report summary\|side-by-side\|interleaved` (`--layout`, default side-by-side), `--options`, `--output-options`, `--output-to`, `--output`, `--title`, `--html`, `--html-mono`, `--line-numbers`/`--no-line-numbers`, `--dry-run`; `--display` and `--context` fill the report's display and context words); `--json`; `--gui` (`--wait`, `--no-launch`). Every word is parsed by core (`check_hex_words`) before anything is read. Human output with `--display differences`: one line per listed section `0x00001F40 L 8  R 8  changed    de ad … \| 00 00 …` (the left offset in the larger file's hex width, at least 8 digits; `0x…/0x…` when both sides hold bytes at different offsets; `—` for an empty side; kinds `changed`, `left-only`, `right-only`, `mixed`; a preview longer than `--preview` ends with `…`), then `... N more sections (--max-sections 0 lists all)`; with `all`/`same` the rows of the side-by-side text report; with `none` nothing; then `hex compare: N sections, X bytes differ, Y only left, Z only right (alignment fast)` and `verdict: <content kind>`. Exit codes (D-331): 1 `binary_same`, 11 `binary_different`; with a side read through its format's External load conversion (D-336) the raw inputs first (identical: 1), then the converted outputs: 2 `rules_same`, 13 `important_differences`; never 12; 100 for any error (a bad word, an unknown or non-hex format, a folder, a read error, `unexpected_eof` for a file that ends while read, a conversion that fails, Ctrl-C). Refused (exit 100, before reading): the row layout flags with `--json` alone (they shape the human output and the reports; `--max-sections` and `--preview` shape the JSON), and with `--gui` the report flags, `--max-sections` and `--preview` (T-52's rule); `--gui` checks the words locally, then sends `OpenView` kind `hex_compare`, which the window answers `unsupported_view` (exit 100) until W13. `--json`: `hikasa.hex_compare.v1`. `session run` of a `hex-compare` session runs it with the session's `[rules.hex]`, `[view]` and `[display] filter`, and takes `--max-sections`, `--preview`, `--display`, `--context` and REPORT |
| `hex dump <spec>` | *M2 W12c:* `--offset <addr>` (default 0; Go To's spellings `0x1F`, `1Fh`, `$1F`, `#31`, `31d`, `+n`, `end`, bare digits decimal; past the end: the last byte), `--length <n>` (default 256, at most 16 MiB), the row layout flags `--bytes-per-line`, `--group`, `--chars`, `--address`, `--values` (the value panel at the offset: *Little Endian Values* and *Big Endian Values*, `Int8`…`UInt64`, `Float32`, `Float64`, `UTF-8`, `n/a` where fewer bytes remain), `--format <id\|name>` (a hex format: its load conversion), `--stdin-name`. Human output: `ADDRESS  hex…  chars` rows (groups separated by two spaces; characters `.` outside the set), then a blank line, `values at 0x0000_0000:` and the panel. `--json`: `hikasa.hex_dump.v1`; `--group` and `--address` with `--json` are refused (they shape the human output only). Exit 0; 100 on error |
| `hex find <pattern> <path>…` | *M2 W12c (D-334, D-284's walk):* `--hex` (the pattern is hex bytes: pairs of digits, separators space, `,`, `-`, `:`, `0x` prefixes, `??` any byte), else text in `--encoding utf-8\|latin1\|utf-16le\|utf-16be` (default UTF-8) with ASCII letters case-folded unless `--case` (`--encoding` and `--case` with `--hex` are refused); `--recursive` (`-r`), `--mask <masks>`, `--include-hidden`, `--archives files\|opened` as `text find`'s walk; `--max-matches <n>` (default 100,000; `find_limit_reached`), `--max-matches-per-file <n>` (default 10,000), both ≥ 1; `--files-with-matches` (`-l`) \| `--count` (`-c`). Every file is searched as bytes (no binary skip, no conversion). Human output: `spec:0xOFFSET` per hit (upper-case hex), `spec` (`-l`) or `spec:N` (`-c`) per file with a hit; a file that cannot be read is `skipped: spec: message` on stderr. `--json`: `hikasa.hex_find.v1` (`hits` empty with `-l` and `-c`). Exit 0 whether or not something matched; 100 on error |
| `picture compare <left> <right>` | *M2 W12c (contract §1.12.23, D-346):* rules `--tolerance <0-255>` (default 0), `--ignore-unimportant` (`--iu`), `--auto-scale`, `--offset <dx>,<dy>` (±2²⁰, default `0,0`), `--left-orientation <list>`, `--right-orientation <list>` (`none`, `cw`, `ccw`, `180`, `flip-h`, `flip-v`, `transpose`, `transverse`, aliases `identity`, `cw90`, `90`, `rot180`, `ccw90`, `270`, `fliph`, `flipv`; comma lists composed in order), `--tone-map clamp\|reinhard`, `--max-megapixels <n>` (1…4096 of 2²⁰ pixels, default 256); formats `--format`, `--left-format`, `--right-format <id\|name>` (of the picture kind), `--stdin-name`; the difference image `--mode tolerance\|range\|blend` (default tolerance), `--blend <0-100>` (default 50; only with `--mode blend`), `--diff-output <spec>` (a writable VFS spec replaced atomically, archive members included; `-` stdout; `clipboard://` through `ClipboardProvider::write_image`; printer and browser refused), `--diff-pane difference\|left\|right`, `--checkerboard`; `--probe <x>,<y>` (repeatable, area coordinates); `--metadata`; REPORT's flags of pictures (`--report summary\|side-by-side` (default summary), `--options`, `--output-options`, `--output-to`, `--output`, `--title`, `--html`, `--html-mono`, `--dry-run`); `--json`; `--gui` (`--wait`, `--no-launch`). Human output: `left: <spec> <W>×<H> <decoder> <color>`, `right: …`, `pixels P  same S (s %)  similar …  different D (d %)  max distance M  bounds x,y w×h` (`bounds none`), one `probe x,y: left x,y #rrggbbaa  right x,y #rrggbbaa  distance d <class>` per `--probe` (`—` for a side that does not cover it), `verdict: <content kind>`; with `--diff-output -` stdout holds only the PNG. Exit codes (D-339): 1 identical bytes, 2 every position same (bytes differ), 12 similar pixels only (2 with `--ignore-unimportant`), 13 a different pixel (a position one picture covers counts by its alpha), 100. Refused (exit 100, before reading): `--metadata` (kind `unsupported`: *picture metadata comparison is not available in this version*), `--diff-output -` with `--json` or with a report on stdout, `--checkerboard` or `--diff-pane` without `--diff-output`, `--blend` outside blend mode, `--report side-by-side` without `--html`/`--html-mono` (*picture side-by-side reports are HTML*), a bad word or a non-picture format; with `--gui` the report flags, `--diff-output` and `--probe`. Decode errors: `unsupported` (DDS, AVIF, SVG), `invalid_data`, `too_large` (checked from the header). Warnings `color_profile_ignored`, `aspect_ratio_differs`, and with `--verbose` `frames_ignored`, `orientation_applied`. `--json`: `hikasa.picture_compare.v1`. `--gui` sends `OpenView` kind `picture_compare` (`unsupported_view` until W13); *M2 W13b (D-369):* the window opens its Picture Compare view with the words applied (`--wait`: 1, 2, 12, 13 under the view's tolerance, geometry and Ignore Unimportant at close, 0 without a verdict, 100 for a side that failed to load); `gui open` of two pictures and `--fv="Picture Compare"` open it too. `session run` of a `picture-compare` session runs it with `[rules.picture]` and the `[view]` geometry, and takes `--diff-output`, `--diff-pane`, `--checkerboard`, `--probe`, `--ignore-unimportant` and REPORT |
| `text find <pattern> <path>…` | *M2 W10c (D-284, D-285, §1.10.19 of the contract):* `<pattern>` plain text, or with `--regex` a regular expression (one line at a time: `^`, `$`, `\b` see a line; a pattern that can only match a line break is refused); case ignored unless `--case` (Unicode simple folding); `--word` whole words (the format's word characters count); each `<path>` a file (searched as given, whatever the masks) or a folder (name order, its files then its folders; `--recursive` (`-r`) descends; `-` reads standard input); `--mask <masks>` (FC-050: `*.c;*.h`, `-` excludes, a trailing `/` or `\` makes a folder mask; the host's case rule); `--include-hidden` (a leading `.`, the DOS hidden bit); links followed for files, never for folders; `--archives files\|opened` (default `files`: an archive is binary and skipped; `opened` searches its members, an archive in a folder with `--recursive`); `--encoding <label>` for every file (default: detected per file, D-42), `--format <id\|name>` (default: by name); a file whose format converts it (External, Sorted, Spreadsheet) is skipped `converted` unless `--convert`; binary files skipped; `--max-file-size <size>` (default 128MB, `KB`/`MB`/`GB` binary units; larger files skipped `too_large` before they are read); `--max-matches <n>` (default 100,000: the search stops, warning `find_limit_reached`), `--max-matches-per-file <n>` (default 10,000; the file's `stopped`); 0 is refused. Files are searched in parallel and printed in the walk's order whatever the threads (the stream is the same with 1 or 8). Human output streams `<spec>:<line>:<column>: <preview>` (1-based, columns in characters, the line cut to 400 characters around the hit, control characters escaped); `--files-with-matches` (`-l`): each file with a hit once; `--count` (`-c`): `<spec>:<n>` per file with a hit; `--verbose`: `skipped (<reason>): <spec>` on stderr. A reader that goes away stops the search (exit 0). `--replace <text>` (`$1`, `${1}`, `${name}`, `$$` with `--regex`; an unknown group exit 100 before reading): every file with a hit is read again as the search read it and Replace All is one save plan (§1.10.13: unchanged lines byte-exact, in place through links, archive members by one rebuild, backups by `[backups] back_up_before_save` unless `--no-backup`, `--backup` forces them); a file without an occurrence when read again is `no_change`; a file the search did not reach after `--max-matches` is not touched; `--dry-run` prints the hits, `would replace <n> occurrences: <spec>` per file and the plan's `write` lines; otherwise the operation question *Replace N occurrences in M files?* then the save questions, as `folder op` asks them (`--yes` (`-y`), `--confirm prompt\|yes-to-all\|no-to-all`; without a terminal an open question is exit 100, nothing written); `--unmappable refuse\|replace` (default without a terminal: refuse that file); a file changed since the plan fails its item (`changed since the plan was made`). After a run: `replaced <n> occurrences: <spec>`, `no change: <spec>`, `failed: <spec>: <reason>` (`--verbose` adds the plan). Exit 0 with or without hits; 100 for an invalid pattern or replacement, a missing path, a bad word or limit, an unknown encoding or format, a Ctrl-C, open questions without a terminal and a replace with a failed file (`ok: true`). `--json`: `hikasa.text_find.v1` |
| `format list` / `format show <id\|name>` / `format detect <file>` / `format tokens <file>` | `detect`: `--stdin-name`; `tokens`: `--format`, `--encoding`, `--lines <a>-<b>`, `--stdin-name` |
| `format new <id>` / `edit <id>` / `delete <id>` / `import <path>` / `export <id\|name>` / `enable <id>` / `disable <id>` / `move <id>` / `restore [<id>…]` | *M2 W1 (D-75):* `new`: `--from <id\|name>`, `--name <text>`, `--mask <mask>`, `--kind <kind>`; `edit`: `--from-file <path>` \| `--set <key=value>`…; `import`: `--replace`; `export`: `-o`/`--output <path>`; `move`: one of `--up`, `--down`, `--top`, `--bottom`, `--before <id>`, `--after <id>`; `restore`: `<id>`… \| `--all`, and `--dry-run` \| `--yes`. They write only the files they change (`10-file-formats.md` §1.1) |
| `folder compare` rule flags of W6 | *M2 W6 (D-169…D-172):* `--attrib-filter <EXPR>`, `--unixtype-filter <EXPR>` (`none` is off; a bad word is `invalid_input`, exit 100), `--no-auto-scan`, `--expand-on-load`, `--expand-differences`, repeatable `--ignored <PATH>` (`!<PATH>` an exception), `--format-on <ID>`, `--format-off <ID>`. The rule flags are one group (`FolderRuleArgs`) |
| `folder op <VERB>` | *M2 W6b (D-185):* `[<left> <right>] [--session <REF>]`, the rule flags of `folder compare`, `SELECT` = `--select <MASK>…`, `--path <REL>…` (`left:`, `right:`), `--include-hidden`, `--no-expand`, `--expand <REL>…`, `--display <FILTER>` (default `all`), `--ignore-unimportant`; `CONFIRM` = `--yes` or `--confirm prompt\|yes-to-all\|no-to-all`, `--dry-run`, `--read-only left\|right\|both`. Verbs: `copy`, `move` (`--to`, `--overwrite`, `--no-preserve`, `--preserve-created`, `--symlinks`, `--backup`, `--no-backup`), `exchange`, `delete` (`--side`, `--recycle`, `--recycle-fallback`), `rename` (`--side`; `--name`, `--mask` or `--regex <OLD> <NEW>`), `touch` (`--side`; `--time`, `--now`, `--from-other`), `mkdir` (`--side`, `--in`, `--name`), `copyto`, `moveto` (`--side`, `--target`, `--structure relative\|base\|none`, `--overwrite`), `compare` (`--method`), `crc` (`--side`), `exclude` (`--what items\|type`), `ignored` (`--off`), `open-with` (`--path`, `--app` or `--associated`, `--side`). No `--select` and no `--path` selects nothing and exits 100. Exit 0; 100 for a failed item, a stop, a `compare_error` row (`ok: true`, the failures in `items`), and for a bad word, nothing selected, unanswered questions, `ReadOnlySide` (failure envelope, nothing written); 107 for a base that cannot be loaded. `attrib` of `docs/12`'s older row is FC-075, P1 |
| `folder sync` | *M2 W6c (D-181, D-182, D-185):* `[<left> <right>] [--session <REF>]`, one of `--update-left`, `--update-right`, `--update-both`, `--mirror-left`, `--mirror-right`, `--custom <MATRIX>` (unless `--session` holds a preset; `<situation>=<action>,…` in kebab or snake case, a preset first as the base), `--orphans added\|deleted`, `--create-empty`, `--within-archives`, `--recycle [--recycle-fallback]`, `--override <PATH>=<ACTION>…`, `--just-selection` with `SELECT` (without it `--select` and `--path` are refused), `--visible` (what `--display` shows), `--include-hidden`, `--allow-empty-source`, `--backup`/`--no-backup`, `--all-rows`, the rule flags of `folder compare`, `CONFIRM`, `--gui [--wait] [--no-launch]`. Text: one line per row that is not `same` (`L>R`, `R>L`, `delL`, `delR`, `----` with the reason, `<!>` for a conflict), the items and summary as `folder op`, then `# <n> copies right, <n> copies left, <n> deletes left, <n> deletes right, <n> conflicts, <n> unresolved` (with `, <n> left as ambiguous names (two names align to one)` when a case-insensitive comparison finds two names of one key; `# emptied entirely: <path> on the <side> (the <side> holds no file there)` before the items for a folder whose source holds no file). `--json`: `hikasa.folder_sync.v1` (`counts.ambiguous`, `emptied`). An empty source is one with no file after filters. A folder-compare `--session` needs a preset flag. A folder-sync session with an unknown key in `[specs]` or `[rules]` does not run. Exit 0; 100 for a failed item, a stop, a row the comparison could not tell (also in a dry run; `ok: true`), and for a bad word, no preset, nothing selected, unanswered questions, an empty source (failure envelope, nothing written); 107 for a base that cannot be loaded |
| `folder compare` archive flags | *M2 W5 (D-152):* `--archives files\|opened\|folders` (default `opened`), repeatable `--open-archive <PATH>` (with `files`: `switch_ignored`); the word list is parsed by core (`parse_archive_handling`); the rule keys are `handling.archives` and `open_archives`. Exit codes unchanged; `hikasa.vfs_op.v1` is unchanged until W5b |
| `settings get <key>` / `set <key> <value>` / `unset <key>` / `list [<key>]` / `restore-defaults` | *M2 W1 (D-71, D-73, D-77):* `set`: `--force` (overwrite a file changed since it was read); `list`: `--changed`; `restore-defaults`: `--all` \| `--file <file>`…, and `--dry-run` \| `--yes`. A key is `<file>.<path>`: the file is `options`, `state`, `commands`, `theme`, `themes.<scheme>`, `filters`, `openwith` or `archives`; the path is the TOML key path, a segment that is not a plain name in double quotes, an array position as a number (`openwith.app.0.command`). A key naming a table gives the whole table (`get`, `list`); `set` takes a leaf or a whole array. The value is TOML; for text keys (strings, paths, masks, colours, shortcuts, enumerations) text that is not a TOML string is taken as it is. These commands read `options.toml` leniently (a bad value is the warning `invalid_config_value`), every other command refuses it (`03-sessions-settings-profiles.md` §5.2). An unknown key (also an array position past the end, or a command id that its view does not have) is `not_found` with up to five suggestions; a key the file holds but Hikasa does not know is listed as unknown, `set` refuses it and `unset` removes it |
| `vfs ls <spec>` / `vfs stat <spec>` / `vfs cat <spec>` | `ls`, `stat`: `--follow-symlinks` |
| `vfs cp <src>… <dst>` / `mv <src>… <dst>` / `rm <spec>…` / `mkdir <spec>…` / `touch <spec>…` | *M2 W3 (D-102, D-103):* `cp`: `-r`/`--recursive`, `--overwrite fail\|skip\|always\|newer` (default `fail`), `--symlinks copy\|skip`, `--follow-symlinks` (root arguments only), `--no-preserve`, `--preserve-created`, `-p`/`--parents`, `--dry-run`; `mv`: `--overwrite`, `-p`, `--dry-run` (folders move without `-r`); `rm`: `-r`, `--trash`, and `--dry-run` \| `--yes` (without either: exit 100 `invalid_input`, nothing touched; `--trash` too); `mkdir`: `-p`, `--dry-run`; `touch`: `--time <RFC 3339>` \| `--reference <spec>` (default: now), `--created <RFC 3339>` (needs creation times: Windows, macOS), `--create`, `--dry-run`. Targets: an existing folder receives the sources under their names, else one source gets the new name; several sources need a folder; a trailing separator means a folder (`-p` creates it). Each command plans every item first (whole trees), then executes the plan item by item, re-checking every target (`11-vfs-remote-archives.md` §1) |
| `vfs` writes inside archives | *M2 W5b (D-148…D-150):* `cp`, `mv`, `rm`, `mkdir`, `touch` and `--output` work on members of zip and the tar family, as on files, with one rebuild per operation and archive; a single stream, `--trash` on a member and a change another operation holds are item errors (`unsupported`). `mkdir new.zip` makes an empty archive for a writable mask (`create_archive`, `mkarchive`); `x.gz` stays a plain folder, only the final name counts, and a trailing separator wants a folder. |
| `gui open <args…>` | the forms and switches of §2 (`--wait`, `--no-launch`, `--fv`, `--title1`…`--title4`, `--ro`, `--ro1`, `--ro2`, `--filters`, `--expandall`, `--format`, `--iu`, `--stdin-name`, `--help`); the arguments after `open` reach that parser unchanged, so `hikasa gui open -- -x a.c` opens the files `-x` and `a.c` exactly like `hikasa-gui -- -x a.c`; the global flags may also follow the specs (the same flag given twice with different values exits 100) |
| `gui ping` / `gui list` / `gui close <id>` / `gui quit` | — |
| `script run <FILE\|-> [<ARG>…]` | *M2 W8a (D-226):* `--silent`, `--stop-on-error`, `--confirm prompt\|yes-to-all\|no-to-all`, `--dry-run`; `--gui [--wait] [--no-launch] [--closescript]` (W8b, as `gui script`). stdout: nothing, except a report with `output-to:-`; stderr: the log stream (`run L2 sync update:left->right`, …) and the current call's progress. Exit 0, 100, 105, 106, 107. `--json`: `hikasa.script.v1` |
| `script check <FILE\|-> [<ARG>…]` | *M2 W8a:* runs no command and reads no base; `ok: <n> commands` on stdout, or the error lines `<script>:<line>:<column>: error: <message>` on stderr; with arguments `%n` is checked against them. Exit 0, 105, 106 |
| `gui script <FILE> [<ARG>…]` | *M2 W8b (D-228):* `--wait`, `--no-launch`, `--stop-on-error`, `--confirm <MODE>`, `--closescript`, `--silent` (§1.1 note). Nothing on stdout; exit 0 once started, the script's code with `--wait`. `--json`: `hikasa.gui_script.v1` |
| `folder op attrib` | *M2 W8a (D-220, FC-075's script and CLI form):* `--change <(+\|-)letters>` (`a`, `s`, `h`, `r`), `--side left\|right\|both` (default both), with `SELECT`, `CONFIRM`, `--dry-run`; on Unix modes `+r` clears the write bits and `-r` sets the owner's; letters the back-end cannot set are skipped `unsupported` with one `attrib_unsupported` warning; items `set_attrs` with their `change`. The *Attributes…* dialog stays P1 |
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
list|json --html`) is refused (exit 100); since M2 W7 before anything is compared (it was after the comparison in M1;
the exit code is the same). With `--json`, `text compare` shows
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
| `--dry-run` of a report | `write <file>` or `replace <file>`; with links `mkdir <folder>`, `replace <folder> (an earlier report)` or `mkdir <folder> (instead of <wanted>, which is not this report's)`, then `write <folder>/<file>  <path>` per pair (`skip (<reason>) <path>`), then `would write <n> files`; `mkdir <cache>/print/…` first for the printer and the browser (M2 W7) |
| `format list` | `<position>\t<id>\t<status>\t<name>\t<mask>` |
| `format detect` | `<id or ->\t<name>` |
| `format tokens` | `L<line>:<start>-<end>\t<element>\t<text>` |
| `vfs ls` | `<kind><permissions or attributes> <size> <mtime> <name>[ -> <target>]` |
| `vfs stat` | the `vfs ls` line, then `capabilities: …` |
| `vfs cp`, `mv`, `rm`, `mkdir`, `touch` | one line per plan item: `mkdir <target>`, `exists <target>` (a kept folder), `copy <source> -> <target>`, `replace <source> -> <target>`, `link …`, `rename …`, `move …`, `remove <target>`, `rmdir <target>`, `trash <target>`, `touch <target>`, `create <target>`, `mkarchive <target>` (M2 W5b), `skip (<reason>) <target>`; after execution `failed <word> <target>: <message>` and `not run <word> <target>`; then `<verb> <done> of <planned> items (<skipped> skipped, <failed> failed)` (`copied`, `moved`, `removed`, `created`, `touched`), for a dry run `would <copy\|move\|remove\|create\|touch> <planned> items`, and `cancelled` after a cancelled run. Exit 0, or 100 when an item failed |
| `gui list` | `<view>\t<kind>\t<title>\t<spec>…` |
| `gui ping` | `<server>\t<protocol version>` |
| `settings get` | the TOML value (`3`, `"dark"`, `["a"]`); a table as TOML; `(not set)` for an optional key without a value |
| `settings set`, `settings unset` | `<key> = <value>`, then `(file unchanged)` when nothing was written |
| `settings list` | `<key> = <value>`, with `  # default: <value>` when changed and `  # unknown key (kept)` for a key Hikasa does not know; a shortcut conflict that involves a changed shortcut is a warning on stderr |
| `settings restore-defaults`, `format restore` | `delete <path>` or `absent <path>` per file (also with `--dry-run`) |
| `format new`, `edit`, `delete`, `import`, `enable`, `disable`, `move` | `wrote <path>`, `removed <path>` |
| `format export` | the TOML (with `-o`: `wrote <path>`) |
| `hex compare` | per section `0x00001F40 L 8  R 8  changed    de ad … \| 00 00 …`, then `hex compare: N sections, X bytes differ, Y only left, Z only right (alignment fast)` and `verdict: <kind>` (M2 W12c) |
| `hex dump` | `ADDRESS  hex…  chars`; with `--values` `values at 0x…:` and the value panel |
| `hex find` | `spec:0xOFFSET`; `spec` (`-l`); `spec:N` (`-c`) |
| `picture compare` | `left: spec W×H decoder color`, `right: …`, the statistics line, `probe …` per `--probe`, `verdict: <kind>` |

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
removed}`), then (W2) the session, workspace and settings-package schemas, then (W3)
`hikasa.vfs_op.v1` (`{op, dry_run, cancelled, items: [{action, kind, source, target, size, replaces,
status, reason, error}], summary: {planned, done, skipped, failed, bytes}}`; failed items keep
`ok: true`, `error: null` and exit 100, as `hikasa.settings_import.v1`; M2 W5b adds the action
`create_archive`, and a rebuild that fails turns the items of its archive into failed ones, with its error, and
`not_run` when cancelled); 38 schemas in all, with `hikasa.folder_op.v1`, `hikasa.folder_contents.v1` (W6b) and
`hikasa.folder_sync.v1` (W6c); M2 W7 appends `hikasa.report.v1` (39 schemas): `{type, layout, format, target,
output, replaced, dry_run, cancelled, bytes, verdict, words: {layout, options, output_options}, unused, links:
null|{folder, wanted, renamed, state: new|replace_ours, pairs: [{path, file, status: planned|written|skipped|failed,
reason, error}], counts: {written, skipped, failed}}, clipboard: null|{text_bytes, html_bytes}, opened}`. M2 W8 appends
`hikasa.script.v1` (`{mode, script, args, dry_run, silent, cancelled, stopped_at, errors, commands, counts,
elapsed_s}`, contract §1.8.12) and `hikasa.gui_script.v1` (`{script, args, waited, exit_code, summary: null|{commands,
ok, failed, refused, not_run, stopped_at}}`): 41 schemas. The warnings `nothing_selected`, `attrib_unsupported` and
`log_not_written` are W8a's. M2 W10a adds to `hikasa.text_compare.v1` (additive) `anchors: [{kind: "pair"|"isolate",
left, right, side, lines}]`, the kept anchors of `--align`/`--isolate` with 1-based half-open line ranges (`[10, 15]`
is lines 10–14; `left`/`right` for a pair, `side`/`lines` for an isolated range, `null` otherwise; `[]` without
anchors), and the warning `anchor_dropped` for a pair that overlaps or crosses an earlier one. M2 W10b adds to
`hikasa.text_compare.v1` (additive) `right_format` (a `format` object, present only when the right file has its own
format), `ignored: {left: [[a, b]…], right: […]}` (1-based half-open ranges, always present), rows' `ignored: true` and
sections' `ignored: true` (present only when set) and `stats.ignored_rows`, and the warning `ignored_lines_dropped`
for a session whose marked lines changed. M2 W10b appends `hikasa.text_convert.v1` (42 schemas): `{dry_run, target:
"in_place"|"output"|"stdout", files: [{spec, format, encoding: {from, to}, bom: {from, to}, line_endings: {from, to},
conversions: [{kind: "trim_trailing"|"spaces_to_tabs"|"tabs_to_spaces"|"line_endings"|"encoding"|"bom", lines}],
converted_lines, status: "planned"|"done"|"skipped"|"failed"|"not_run", skip, error, questions, warnings}], plan:
[PlanItemJson], summary: {files, converted_lines, done, skipped, failed, not_run}}`, where a plan item is W3's
(`action` `write` or `skip`, `kind`, `source`, `target`, `size`, `replaces`, `status`, `reason`, `error`) with
`backup`, `expect` (the target's `{kind, size, mtime}` the plan saw, `null` for a new file), `via_link`, `encoding`,
`bom`, `bytes`, `spliced_bytes`, `encoded_lines`; one item per file (a file refused before planning is a failed
`write`, a `no_change` file a `skip`), none for stdout. Its envelope is `hikasa.folder_op.v1`'s: a run that wrote with
failed files is `ok: true` with exit 100 and each file's `error`; a refused request is `ok: false`. The warnings of
saves: `source_touched`, `save_reencoded`, `lossy_line_rewritten`, `unmappable_replaced`; `SkipReason` gains
`no_change`. M2 W10c appends `hikasa.text_find.v1` (43 schemas): `{query: {pattern, regex, match_case, whole_word},
roots: [spec], mask: str|null, recursive, include_hidden, archives: "files"|"opened", files: [{spec, encoding, hits:
[{line, column, len, preview, preview_range: [a, b], truncated}], hit_count, stopped}], skipped: [{spec, reason:
"binary"|"too_large"|"unreadable"|"converted"|"filtered", error}], counts: {files_searched, files_matched, hits,
skipped}, limit_reached, replace: null|{text, dry_run, plan: [PlanItemJson], results: [{spec, replaced, status, skip,
error}], summary: {replaced, done, skipped, failed, not_run}}}`: `files` holds the files with a hit (lines and columns
1-based, columns, lengths and `preview_range` in characters), capped by `--max-matches` as the stream is; the warning
`find_limit_reached` says the search stopped. A replace's envelope is `hikasa.text_convert.v1`'s. Table Compare phase
2 appends `hikasa.table_compare.v1` (44 schemas; JSON Schema `docs/schemas/table_compare.v1.json`, contract §6.3):
`{left, right: {spec, size, mtime, source, sheets, encoding, format}, format: {left, right: {kind, delimiters,
qualifier, consecutive_as_one, trim_whitespace, widths, first_line, regional, encoding, detected}}, options: {sheets,
compare_formulas}, ignore_unimportant, verdict: {result}, stats: {sheets_paired, sheets_left_only, sheets_right_only,
sheets_differing, totals}, sheets: [{name, left, right, renamed, state, alignment: {mode, keys, algorithm, sorted},
columns, stats, duplicates, header, display, rows: [{row, state, importance, left, right, cells: [{column, state,
left, right: {value, type, formula}}]}], hidden_rows}], warnings}`; cell values are display texts. Its warnings:
`duplicate_keys`, `unterminated_qualifier`, `stray_after_qualifier`, `sheet_skipped`, `formula_without_value`,
`rule_not_applied`, `keys_not_applied`, `delimiter_mismatch`, `lossy_decode`, `alignment_degraded`, and with
`--verbose` `merged_cells`. `left.format`/`right.format` is the side's format of the table kind, else `null`.
M2 W12c appends `hikasa.hex_compare.v1`, `hikasa.hex_dump.v1`, `hikasa.hex_find.v1` and `hikasa.picture_compare.v1`
(**48 schemas**; contract §1.12.24, D-347; JSON Schemas `docs/schemas/hex_compare.v1.json`, `hex_dump.v1.json`,
`hex_find.v1.json`, `picture_compare.v1.json`), built by `hikasa-report` (`json/hex.rs`, `json/picture.rs`) from the
models: `hikasa.hex_compare.v1` `{left, right: {spec, size, mtime, format, converted, load:
"memory"|"file"|"temp_cache"}, alignment: {mode, requested, block, skew, window, budget_mib, degraded}, verdict:
{result}, stale, stats: {left_bytes, right_bytes, same_bytes, differing_bytes, left_only_bytes, right_only_bytes,
sections}, sections: [{index, kind: "changed"|"left_only"|"right_only"|"mixed", left: {offset, len}|null, right,
columns, preview: {left, right}}], sections_shown, sections_truncated, warnings}`; `hikasa.hex_dump.v1` `{side, layout:
{bytes_per_line, group, chars, address}, offset, length, rows: [{address, hex, chars|null}], values: null|{offset,
little, big}}` with the values `{i8, u8, i16, u16, i32, u32: number|null, i64, u64: decimal string|null, f32, f64:
number|"NaN"|"inf"|"-inf"|null, utf8}`; `hikasa.hex_find.v1` `{query: {kind: "text"|"bytes", pattern, encoding|null,
match_case}, roots, mask, recursive, include_hidden, archives, files: [{spec, size, hits: [offset], hit_count,
stopped}], skipped: [{spec, reason: "error"|"filtered", error}], counts: {files_searched, files_matched, hits,
skipped}, limit_reached}`; `hikasa.picture_compare.v1` `{left, right: {spec, size, mtime, format, decoder, width,
height, color, bit_depth, has_alpha, frames, orientation|null, tone_map|null, converted}, options: {mode, tolerance,
blend, ignore_unimportant, auto_scale, tone_map, orientation: {left, right}, offset: [dx, dy]}, geometry: {area: {x, y,
width, height}, scaled: "left"|"right"|null, scaled_to: [w, h]|null}, verdict: {result}, stats: {pixels, same,
similar, different, left_only, right_only, same_percent, similar_percent, different_percent, max_distance,
mean_distance, bounds|null}, probes: [{x, y, left, right: {x, y, rgba}|null, distance, class}], diff_output:
null|{target, pane, mode, width, height, checkerboard}, warnings}`. Offsets and counts are JSON numbers, hex text is
lower case without separators, percentages are rounded to 4 decimals, arrays are never `null`; the hex verdicts are
`binary_same`, `binary_different`, `rules_same` and `important_differences` (converted sides), the picture verdicts
`binary_same`, `rules_same`, `unimportant_only` and `important_differences`. The warnings of W12c: `input_changed` (an
input's stamp changed during the job; `stale: true`), `sections_truncated` (the engine's section list was cut; the
statistics stay exact), `color_profile_ignored`, `aspect_ratio_differs`, and with `--verbose` `frames_ignored` and
`orientation_applied`; hex reuses `alignment_degraded`, `hex find` `find_limit_reached`.
`hikasa.vfs_stat.v1` lists the capabilities of M2 W3 too (`mkdir`, `remove`, `rename`,
`set_attrs`, `trash`, `create_symlinks`, `set_created`). TOML values appear as JSON values (a table as an object, a datetime as a string;
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
execute bit is skipped; a relative path is taken relative to the current folder; tests) and,
M2 W3, `HIKASA_CLIPBOARD_FILE` (a file whose text stands in for the clipboard of `clipboard://`:
scripts, headless machines and tests; a missing file means "no clipboard", an empty one an
empty clipboard; M2 W7: `--output-to clipboard` writes a report's HTML, else its text, to that file) and, M2 W7,
`HIKASA_CLIPBOARD_HELPER_FILE` (on Linux a report placed on the clipboard is handed to a helper process, `hikasa` or
`hikasa-gui` itself, that keeps it after the program ended; with this variable the helper writes the report's HTML,
else its text, to that file instead of placing it: scripts, headless machines and tests; a clipboard that cannot be
written is exit 100, `unsupported` without a display, `io` otherwise) and, M2 W7,
`SOURCE_DATE_EPOCH` (the report time, a decimal count of seconds since the epoch shown in UTC, so a report is the same
bytes on every run; a value that does not parse is ignored with the warning `source_date_epoch_invalid`; only reports
read it).
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

**Gate G13 (reports, M2 W7b; contract §1.7.17, D-212).** The same harness (`g13_*` in `perf_gates.rs`) measures
each report against `--layout summary` of the same input in the same run: `report text` of the G1 pair to a file,
display all (side-by-side HTML, interleaved HTML and text, XML), each ≤ the summary + 1.5 s and its peak RSS + 24 MiB;
`report folder` of the warm G2 tree, display all (side-by-side HTML, XML), ≤ the summary + 1.0 s and + 32 MiB; the
G2 tree with about 1,000 differing pairs of 4 KiB text, `include-file-links`, into a new files folder each run,
≤ 5 s end to end and ≤ the summary's RSS + 64 MiB. Measured on 2026-11-03 on the same machine, release build, with
the load average printed beside each figure. **Loaded machine:** load 5–10 (other projects building), so these are
upper figures; the two runs of the text gate are both given.

| Gate | Report | Median (min–max) | Over the summary | Peak RSS (over the summary) | Written | Load (1 min) |
|---|---|---|---|---|---|---|
| G13 text | summary (baseline) | 0.548 s (0.543–0.553); 0.500 s | — | 300 MiB | — | 8.2; 7.9 |
| G13 text | side-by-side HTML | 1.437 s (1.354–1.477); 1.676 s (1.092–2.052) | +0.889 s; +1.176 s | 301 MiB (+0.6) | 218 MB | 9.2; 7.5 |
| G13 text | interleaved HTML | 0.971 s (0.920–1.133); 0.752 s | +0.423 s; +0.252 s | 301 MiB (+0.7) | 128 MB | 8.9; 7.5 |
| G13 text | interleaved text | 1.095 s (1.077–1.331); 1.285 s | +0.547 s; +0.784 s | 301 MiB (+0.9) | 67 MB | 8.9; 7.1 |
| G13 text | XML | 1.070 s (0.982–1.257); 1.330 s | +0.522 s; +0.830 s | 301 MiB (+0.4) | 166 MB | 9.6; 6.7 |
| G13 folder | summary (baseline) | 0.063 s (0.061–0.082) | — | 35 MiB | — | 9.3 |
| G13 folder | side-by-side HTML | 0.180 s (0.174–0.181) | +0.116 s | 37 MiB (+1.6) | 34 MB | 9.3 |
| G13 folder | XML | 0.227 s (0.221–0.230) | +0.163 s | 36 MiB (+1.5) | 45 MB | 10.4 |
| G13 links | 1,027 pairs of 4 KiB, a new files folder | 0.108 s (0.104–0.108) | (summary 0.055 s) | 41 MiB (+5.0) | 1,028 files | 8.6 |

Every row is within its threshold. In the same session G1 and G2 were re-measured unchanged (G1 0.398 s auto,
0.395 s standard, 0.316 s identical, peak RSS 301/301/278 MiB; G2 warm 0.051 s, 35 MiB; load 7–8), and the G4 bench
`cancel_latency` (hikasa-text) gave a worst latency of 3.4–5.5 ms against 100 ms (load 5). The streaming row of
§1.7.17 runs on CI (`core/hikasa-report/tests/stream_memory.rs`, and through the Service
`core/hikasa-session/tests/report_stream.rs`: every text layout of a 50,000-line pair peaks at 0.14–0.71 MB of live
heap writing 0.2–9 MB to a file, folder side-by-side HTML and XML at 0.19 MB).

**Gate G14 (scripts, M2 W8a; contract §1.8.17, D-231).** The same harness (`g14_*` in `perf_gates.rs`): `script check`
of a generated 200,000-line script (every command kind, quotes, continuations, `%` forms) ≤ 0.5 s and peak RSS ≤ 32 MiB
+ 8 × its size; a script's `copyto` of 10,000 small files against `folder op copyto … --select all --yes` in the same
run, ≤ the direct × 1.05 + 0.2 s; a script's `--dry-run` sync of the G2 tree against `folder sync --update-right
--dry-run`, ≤ the direct + 0.2 s; 10,000 `select` commands on a 100-file tree ≤ 2 s. Measured on 2026-11-03, release
build, after the compact checked script (contract §1.8.20 W8a note 22), the load average beside each figure (load 1.5–2.0:
under the "loaded machine" mark of 4).

| Gate | Measure | Median (min–max) | Threshold | Load (1 min) | Result |
|---|---|---|---|---|---|
| G14 parse | `script check`, 200,000 lines, 5.0 MB | 0.118 s (0.116–0.118) | ≤ 0.5 s | 1.8 | pass |
| G14 parse | peak RSS of the same | 29.5 MiB | ≤ 70 MiB (32 MiB + 8 × 5.0 MB) | 1.8 | pass |
| G14 copy | script 3.982 s (3.937–4.030) against direct 4.375 s (3.912–4.707) | — | ≤ 4.793 s | 1.6 | pass |
| G14 sync | script 0.076 s (0.075–0.078) against direct 0.078 s (0.077–0.097) | — | ≤ 0.278 s | 1.8 | pass |
| G14 overhead | 10,000 commands | 0.125 s (0.123–0.125) | ≤ 2 s | 1.8 | pass |

The first W8a measurement of the parse row (0.274 s, peak RSS 199.2 MiB, load 4.1) failed the RSS gate: the checked
script held each word twice (in the command's words and its typed slots) with slots sized for the largest command, about
480 bytes a command, `script check` built the JSON document's command list only to count it (about 330 more), and the
references lexed the whole script at once. A checked script now keeps every word's text once and 12 bytes a word and a
command, and is lexed and checked one line at a time. The copy row is bound by the disk: in the same session one run gave
script 4.800 s (4.462–6.347) against direct 4.119 s (3.954–4.515), over its 4.525 s gate, and the next run passed as in
the table (the first W8a measurement also passed: 4.363 s against 4.535 s); nothing in the copy path changed. In the same session G1, G2,
G4 and G7 were re-measured unchanged: G1 0.395 s auto, 0.388 s standard, 0.305 s identical, peak RSS 301/301/279 MiB
(load 1.5–1.7); G2 warm 0.048 s, 35 MiB (load 1.7); the G4 bench `cancel_latency` (hikasa-text) a worst latency of
3.3–4.8 ms against 100 ms (load 1.5–3.8); G7 copy 1.49 × (374 ms against 250 ms, gate 1.5 ×, load 3.5) and G7 plan
0.106 s (gate 0.5 s; **loaded machine**, load 4.3 at its end). The stream row of §1.8.17 runs on CI
(`core/hikasa-session/tests/script_log.rs`: 500,000 commands log over a million lines within 4 MiB of the same run at
level `none`).
