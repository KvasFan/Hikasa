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
contract `docs/design/m2-contracts.md`. Done: wave 0 (catalogue in core, help topics,
`xtask`, nextest and the docs/bench jobs).

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
