# Roadmap and Milestones

Each milestone ends with a tagged release, passing CI on three platforms, and the
acceptance criteria below verified by automated tests where possible.

## M0 — Skeleton (version 0.0.0, done)
- Workspace, crate stubs, specifications, CI config. No functionality.

## M1 — Core engines and performance gate (0.1, released as 0.1.0)

Status: released as 0.1.0 (waves 1–6b, see `CHANGELOG.md`). `[x]` done, `[ ]` open.

- [x] `hikasa-core`, `hikasa-vfs` (local), `hikasa-diff` (standard, Myers/histogram,
  patience, closeness), `hikasa-formats` (model, tokenizer, 10 presets — 21 shipped),
  `hikasa-text` document model, `hikasa-folder` scan/compare (quick tests, CRC,
  binary, rules-based), name filters.
- [x] CLI first: `hikasa quick`, `hikasa text compare` (unified/side-by-side/summary/
  patch reports), `hikasa folder compare` (`--json`), `hikasa format tokens`, with
  exit codes and `--json` (also `format list|show|detect`, `vfs ls|stat|cat`,
  `gui open|ping|list|close|quit`, `completions`, `version`; `docs/12`).
- [x] GUI prototype: one window, Folder Compare + Text Compare views, virtualized text
  panes, IPC server; `hikasa gui open … --wait`. **Performance gate ARCH-GUI-003
  decided here:** egui/eframe (D-56 as amended); all three items met on a desktop
  (`docs/02` §2). Not yet measured on a 2020-class laptop.
- [x] Acceptance: golden diffs; 1M-line scroll test (`gui/hikasa-gui/tests/perf_gate.rs`,
  run by hand); 100k-file folder scan < 5 s on SSD (0.694 s cold, `docs/12` §4).

Release:
- [x] Version 0.1.0, CHANGELOG entry.
- [x] Tagged release `v0.1.0`.
- [x] CI green on Linux, Windows and macOS (the performance gates were measured on
  Linux only).

Still open from M1 (done in M2 wave 0):
- [x] Help topics: `help/` with one topic per CLI subcommand and view, `hikasa help`,
  F1 in the window.

Moved to M2 (done in M2 wave 0):
- [x] Bench job: `bench.yml`, started by hand on a GitHub-hosted runner (25 %
  tolerance); reference figures stay hand measurements on a fixed machine (D-69).
- [x] `cargo xtask commands-doc` generating `docs/reference/commands.md`, with a CI
  staleness check.
- [x] `cargo nextest` in CI.

## M2 — Usable comparison tool (0.2) — all P0 of FC, TC, TE, SES, UI, OPT

Status: in progress on the branch `feat/m2`; plan `docs/design/m2-plan.md` (15 waves),
contract `docs/design/m2-contracts.md`. Wave 6 ran in four parts, all done: W6a (foundations), W6b (operations: every file operation headless, `folder op`), W6c (sync: the matrix, presets and overrides, `folder sync`, folder-sync sessions, `--sync` up to the GUI, gate G7) and W6d (GUI hooks: Copy to Right/Left, Delete and Delete Permanently on one selected row with the confirmation and Delete dialogs, the log and notices, refresh in the view with `renumbered`, Open on an archive in place, FC-012 on load, a child view's result on its row, Disable editing and `--nobackups` in the window; the rest of the Actions menu and the Folder Sync view are W9). Wave 7 (reports) ran in two parts: W7a (core: the request and its vocabulary, interleaved, XML dumps and their schema, HTML wrap and the print block, links, streaming; the W6 defect of `copyto` of an aligned pair) and W7b (the Service and the CLI: `hikasa report text|folder`, one REPORT group of flags on five commands, the targets file, clipboard, printer and browser, linked file reports in a files folder that is swapped in whole, `SOURCE_DATE_EPOCH`, remembered dialog choices, `hikasa.report.v1`, gate G13); the report dialog, *View in Browser* and *Print Preview* are W13's. Wave 8 (scripts) ran in two parts: W8a (the language in `hikasa-script`: lexer, parser, checker with every vocabulary word through the CLI's parsers, `%` expansion, the interpreter over a host in the session layer; every command of docs/12 §3.1 as its CLI command, `attrib` and `folder op attrib`, confirmation and silent mode, the log, the lookup, `hikasa script run|check`, `hikasa.script.v1`, gate G14) and W8b (the window and IPC: `RunScript` with its cwd and environment, `ScriptFinished`, `busy`; `hikasa gui script`, `script run --gui`, `@script` in the shared command line, `--silent` always headless, `--closescript`; the status window and Tools ▸ Run Script…; `hikasa.gui_script.v1`). Wave 9 (the GUI's folder views) ran in three parts: W9a (view: columns, sort, glyphs and cells, the selection, path edits and the browse dialog, Find Filename, Legend, Info, the base commands, Swap, Clear; done) and W9b (operations and sync: every verb of the Actions menu through a dialog or a confirmation that shows the core's preview of its plan, Ignored, Exclude, Compare Contents and its results, Quick Compare, Compare To, Open With, Synchronize, the Folder Sync view with its overrides and Run Sync, its waited outcome over IPC; done), and W9c (drag and drop with core's plan in a prompt and rows between the panes, clipboard file lists with Copy, Paste and Select All, the pane context menus, the file-manager entries, the folder rule tabs of Session Settings with Rules and Name Filters, the Filters edit with presets and Add To Presets; done, reviewed and fixed: W9 is complete). Wave 10 (the text engine) ran in three parts: W10a (editing and recompare: the block line table, the editable document with undo, marks, bookmarks and the editing commands, editability, Align With and Isolate with `text compare --align`/`--isolate`, the traced alignment, chunked rows, the live comparison with its provisional splice and the exact incremental recompare, gate G8; done, G8 partly met, see docs/02), W10b (saves that keep every unchanged line byte-exact as plans of the VFS action `write`, their questions, targets, archive members and backups, conversions and `hikasa text convert`, a format per pane, Ignored lines, the CLI-003 switches `--edit`, `--savetarget`, `--ro*`, `--nobackups`; done) and W10c (search and navigation: find and replace, the Find in Files engine, `hikasa text find` with replace in files as one save plan (`hikasa.text_find.v1`), navigation and details data, Next Difference Files and Copy File and Open Next Difference, Text Edit and text pairs; done: W10 is complete. G8's search, save and cancel rows are met; its edit rows Enter, delete a line, paste 100 lines and Copy Section stay open (a few edits of 20, and most pastes, take the full compare of the 1M-line pair: 1.3–1.6 s at load 6–10, see docs/02 and contract §1.10.1), and `note_edit`'s worst of 20 is about 1 ms (above it under load); G1, G4 and G5 are unchanged). Wave 11 (the GUI's text views) runs in three parts: W11a (editor and save: the live view in the window, caret and selection with word wrap, typing and the IME, the editing commands and undo per pane, the clipboard and the primary selection, Copy to Right/Left and the gutter's copy buttons, Save, Save All, Save As with their questions, the unsaved-edits prompts of every close, the File Info header, the status bar, rule and format changes that keep the edits, gate G16's typing and memory rows; done), W11b (panels, search, navigation, Text Edit: the layouts and display options, the Details panel and ruler, the Find and Replace panels, Find in Files with its results panel, navigation and bookmarks, Ignored and alignment overrides in the window, the session link, the text context menus, the Text Edit view, the `--gui` words, Info, Clear Session and changes on disk; done) and W11c (the text rule tabs of Session Settings with D-317's `"format"` entries, the File Formats manager, the format editor with its grammar items, preview and regex helper, Edit-mode steps block by block, the whole G16 table and ARCH-GUI-003; done: W11 is complete). Version 0.1.93 is a pre-release of 0.2.0 with waves 0–7 and the packages (0.1.92 had waves 0–6, 0.1.91 waves 0–5);
0.2.0 stays reserved for M2 complete. Done: wave 0 (catalogue in core, help topics,
`xtask`, nextest and the docs/bench jobs); wave 1 (settings store: every settings file
typed, versioned and written sparsely with unknown keys kept, change detection by
polling, all `docs/15` keys OPT-001…OPT-022, `hikasa settings get|set|unset|list|
restore-defaults`, the `hikasa format` write commands, colour schemes and Open With
expansion in core; the window reads them from W4); wave 2 (sessions: files, store, defaults, auto-save
rotation, scopes and close prompts in core, `hikasa session …` with `session run`; workspaces and `hikasa
workspace …`; settings packages `hikasa settings export|import`, `restore-defaults --category`; the window
answers `LoadSession`/`LoadWorkspace`, `hikasa gui session|workspace` and the one-argument forms of `gui open`;
the session UI itself comes with W4); wave 3 (VFS mutations: single-item primitives on every back-end with
capabilities, the generic plan/execute layer for copy, move, remove and trash, mkdir and touch with per-item
results, `hikasa vfs cp|mv|rm|mkdir|touch`; the clipboard as `clipboard://` for the CLI and the window, D-92…D-103); wave 4a (GUI shell: several windows and tab moves, layout memory, menus, toolbars and keys from the core's
effective command table, vector icons, dialogs, the theme from `theme.toml`, status bar with jobs, message panel,
log book and panel, notices and polling of settings and input files, drops, D-104…D-119; W4b sessions in the
window: save, lock, close prompts, auto-save, the Session Settings shell, Home with its tree and management panel,
workspaces with lazy restore, the GUI's command line, D-120…D-127; W4c settings dialogs: the Options dialog with the
Toolbars/Shortcuts/Menus page and the theme editor with live preview, the Export, Import and Restore wizards, IPC
`ImportPackage`, D-128…D-134; wave 4 complete; settled memory gates met on glibc, peak RSS 419–429 MiB and the ≥ 60 fps
GPU run still open, docs/02); wave 5a (archives, read: zip, tar, compressed tar and single streams through
`ArchiveVfs`, nesting, the temp cache, FC-015 and FC-025 over archive sides, D-135…D-155, reviewed against hostile input: cache sessions, index memory, modes
on copy-out, rule 12, G4 on the main paths, the scan budget; wave 5b (archives, write: batches, the overlay, rebuilds that
keep what they did not touch as bytes, `create_archive`, the write capabilities, D-148…D-150, the review's D-156…D-158; wave 5 complete); 0.1.91 also brings Ctrl-C in the CLI (D-159) and the theme's named fonts (D-160).

- Sessions (save/load/auto-save/lock/folders/defaults), workspaces, options dialog,
  theming light/dark, commands catalogue with customizable shortcuts.
- Folder Compare complete (display filters, other filters, columns, all file
  operations, sync submenu, legend, log). Text Compare complete (editing, find/replace,
  bookmarks, thumbnail, details, display filters, replacements P1 optional).
- Archives: zip, tar(.gz/.bz2/.xz/.zst). Clipboard and stdin specs. Hex Compare P0.
  Table Compare P0 (delimited/fixed) and, moved from M3 (Table Compare T-1, owner 2026-11-09), TB-004 spreadsheets:
  all sheets of xlsx, xlsm, xlsb, xls and ods. Picture Compare P0.
  *Table Compare progress (contract `design/m2-table-contract.md`):* phase 1, the engine (`hikasa-table`: parser,
  workbook loader, comparison, gate G9), and phase 2, the integration (`hikasa table compare`, reports,
  `hikasa.table_compare.v1`, sessions, `table-report`, the rules-based content test of folders, the *Spreadsheet text*
  conversion, the *Spreadsheet Workbooks* preset with xlsb), and phase 3, the window's grid (`table compare --gui`, the
  Table Compare view, its session tabs and report dialog; Wave 13's table part), are done.
  *Hex and Picture Compare progress (wave 12, contract `design/m2-contracts.md` §1.12):* W12a, the hex engine
  (`hikasa-hex`: lazy documents, the None, Fast and Complete alignments, sections, rows, find, values, the shared chunk
  visitor; gate G10), and W12b, the picture engine (`hikasa-picture`: the decoders, orientation, the tone map, the
  comparison with tolerance, Auto Scale and offset, the renderer, PNG output and clipboard images; gate G11), are done;
  W12c, the integration (`hikasa hex compare|dump|find`, `hikasa picture compare`, the Service calls, hex and picture
  reports and the script commands `hex-report`/`picture-report`, the four JSON schemas, typed `hex-compare` and
  `picture-compare` sessions, `--gui` through IPC, and the Picture content test of folders and `quick --rules`,
  D-344), is done: wave 12 is complete (its review follows). W13a, the Hex Compare view in the window (the row cache,
  panes, Layout, caret and Copy, navigation, Go To, Find, values, thumbnail, headers, status, Info, rule tabs and
  sessions, `hex compare --gui` served), is done; the Picture Compare view is W13b's.
- Scripts: load/filter/expand/select/copy/move/delete/sync/compare/log/option/
  reports; single-instance IPC; `hikasa-cli` wait mode.

## M3 — 1.0 — all P1
- Moved from M2 (D-59, plan Q1): the merge-only parts of TC-015/TC-040/TC-050/TC-052/
  TC-053/FC-004; FC-081 and the snapshot/remote/profile parts of FC-001/TC-002/TC-005;
  FF-030 built-in document converters (but *Spreadsheet text*, M2 by Table Compare T-22); TB-001 HTML extraction
  (spreadsheet extraction is M2, T-1); PX-001 DDS;
  FC-070 ACL and 8.3 options; SES-091 taskbar previews (OPT-004 `show_taskbar_previews`
  is stored only); UI-031 "update available" (OPT-013 is stored only); the FF-027
  third-party highlighter fallback.
- Folder Merge, Folder Sync with Sync Memory, Text Merge (GUI + automerge), Text
  Patch, alignment overrides, replacements, snapshots, remote back-ends (FTP/FTPS/
  SFTP/WebDAV/S3), profiles + keychain, 7z, office/PDF conversions (spreadsheets are M2's, T-1),
  Media/Version/Registry-file views, reports of the P1 views (table reports are M2's, Table Compare §6.3, and so are
  hex side-by-side and interleaved and picture side-by-side, M2 W12c, D-348;
  media, version, registry; XML and linked file reports are M2's, D-195), import/export packages, shell integration, admin policies, accessibility,
  installers, user manual (context help).

## M4 — post-1.0 (P2)
- Cloud drive profiles (OAuth), Subversion, MTP, live Windows registry, RAR/cab/iso/
  deb/rpm/wim/chm, clipboard history helper, webpages panels, plugin API (WASM), theme
  marketplace, spreadsheet write-back, Rust/Go/TOML/YAML/JSON/Markdown grammars polish.

## Definition of done (every milestone)
- `cargo test`, `cargo clippy -D warnings`, `cargo fmt --check`, `cargo deny` green on
  Linux/Windows/macOS.
- Every new command is in `reference/commands.md` and has a help topic.
- CHANGELOG entry; version bumped per semver (0.x during development).
