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
contract `docs/design/m2-contracts.md`. Version 0.1.90 is a pre-release of 0.2.0 with waves 0–4 and the packages;
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
GPU run still open, docs/02).

- Sessions (save/load/auto-save/lock/folders/defaults), workspaces, options dialog,
  theming light/dark, commands catalogue with customizable shortcuts.
- Folder Compare complete (display filters, other filters, columns, all file
  operations, sync submenu, legend, log). Text Compare complete (editing, find/replace,
  bookmarks, thumbnail, details, display filters, replacements P1 optional).
- Archives: zip, tar(.gz/.bz2/.xz/.zst). Clipboard and stdin specs. Hex Compare P0.
  Table Compare P0 (delimited/fixed). Picture Compare P0.
- Scripts: load/filter/expand/select/copy/move/delete/sync/compare/log/option/
  reports; single-instance IPC; `hikasa-cli` wait mode.

## M3 — 1.0 — all P1
- Moved from M2 (D-59, plan Q1): the merge-only parts of TC-015/TC-040/TC-050/TC-052/
  TC-053/FC-004; FC-081 and the snapshot/remote/profile parts of FC-001/TC-002/TC-005;
  FF-030 built-in document converters; TB-001 spreadsheet/HTML extraction; PX-001 DDS;
  FC-070 ACL and 8.3 options; SES-091 taskbar previews (OPT-004 `show_taskbar_previews`
  is stored only); UI-031 "update available" (OPT-013 is stored only); the FF-027
  third-party highlighter fallback.
- Folder Merge, Folder Sync with Sync Memory, Text Merge (GUI + automerge), Text
  Patch, alignment overrides, replacements, snapshots, remote back-ends (FTP/FTPS/
  SFTP/WebDAV/S3), profiles + keychain, 7z, spreadsheet/office/PDF conversions,
  Media/Version/Registry-file views, reports for every view incl. XML and linked file
  reports, import/export packages, shell integration, admin policies, accessibility,
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
