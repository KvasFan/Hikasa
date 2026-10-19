# Roadmap and Milestones

Each milestone ends with a tagged release, passing CI on three platforms, and the
acceptance criteria below verified by automated tests where possible.

## M0 — Skeleton (version 0.0.0, this repository state)
- Workspace, crate stubs, specifications, CI config. No functionality.

## M1 — Core engines and performance gate (0.1)
- `hikasa-core`, `hikasa-vfs` (local), `hikasa-diff` (standard, Myers/histogram,
  patience, closeness), `hikasa-formats` (model, tokenizer, 10 presets),
  `hikasa-text` document model, `hikasa-folder` scan/compare (quick tests, CRC,
  binary, rules-based), name filters.
- CLI `--qc` with exit codes; `text-report` side-by-side/summary/patch to text/HTML.
- GUI prototype: one window, Folder Compare + Text Compare views, virtualized text
  panes. **Performance gate ARCH-GUI-003 decided here.**
- Acceptance: golden diffs; 1M-line scroll test; 100k-file folder scan < 5 s on SSD.

## M2 — Usable comparison tool (0.2) — all P0 of FC, TC, TE, SES, UI, OPT
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
