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

## M2 — Usable comparison tool (0.2, version 0.2.0) — all P0 of FC, TC, TE, SES, UI, OPT

Status: complete on the branch `feat/m2` as version 0.2.0 (wave 14, 2026-11-18; the tag and the CI run on three
platforms follow the maintainer's push). Plan `docs/design/m2-plan.md` (15 waves, W0–W14), contract
`docs/design/m2-contracts.md` (§1.0–§1.14, acceptance §6.0–§6.14) and `docs/design/m2-table-contract.md`;
0.1.90–0.1.93 were pre-releases. `[x]` done, `[ ]` open; each item was checked against its modules and tests in W14
(contract §1.14.8). Deferred sub-parts name D-59 (plan Q1, the maintainer's answer *a* of 2026-10-28) and are in M3's
list below.

Waves: W0 infrastructure (catalogue in core, help topics, `xtask`, nextest, the docs and bench jobs); W1 settings store;
W2 sessions, workspaces, settings packages; W3 VFS mutations, clipboard, generic copy; W4 the GUI shell (a: windows,
tabs, menus, theming; b: sessions in the window, Home, workspaces; c: the Options dialog and the settings wizards); W5
archives (a: read; b: write); W6 folder operations and the sync engine (a–d); W7 reports; W8 scripts (a: language and
headless runs; b: the window and IPC); W9 the folder views in the window (a: view; b: operations and sync; c: drag,
menus, rules); W10 the text engine (a: editing and recompare; b: save, conversions, formats; c: search and navigation);
W11 the text views in the window (a: editor and save; b: panels, search, Text Edit; c: rules, formats, gates); W12 the
Table (phases 1–2), Hex and Picture engines and their integration; W13 the Table (phase 3), Hex and Picture views, the
report dialog and Quick Compare at startup; W14 release hardening.

- [x] Sessions: save, load, auto-save (on close and periodically, day groups, the limit), lock, session folders,
  defaults per type and scope; workspaces with lazy restore; the Options dialog; theming light/dark with the theme
  editor; the commands catalogue with customizable toolbars, shortcuts and menus.
  *Deferred (D-59):* the snapshot, remote and profile parts of the specs (FC-001, TC-002, TC-005); SES-091 taskbar
  previews (OPT-004 stored only); UI-031 "update available" (OPT-013 stored only).
- [x] Folder Compare complete: display filters, name, attribute and Unix-type filters, columns, every file operation
  of the Actions menu (copy, move, exchange, delete and recycle bin, rename by mask, touch, new folder, Exclude, Open
  With, Compare To; attributes as `folder op attrib` and the script command — the Attributes dialog FC-075 is P1), the
  Synchronize submenu and the Folder Sync view (FS-001/002/004/005, Q2), legend, log.
  *Deferred (D-59):* FC-081; FC-070's ACL and 8.3 options; the merge-only part of FC-004.
- [x] Text Compare complete: editing with undo and the exact live recompare, saves, find/replace and Find in Files,
  bookmarks, thumbnail, details, display filters, layouts, Align With and Isolate, conversions, a format per pane and
  the File Formats manager; the Text Edit view. Replacements stay P1.
  *Deferred (D-59):* the merge-only parts of TC-015, TC-040, TC-050, TC-052, TC-053; the FF-027 third-party highlighter
  fallback; FF-030's built-in document converters (except *Spreadsheet text*, T-22).
- [x] Archives: zip, tar, tar.gz, tar.bz2, tar.xz, tar.zst read (nested too); zip and the tar family written by
  rebuild (Q8). Clipboard (`clipboard://`, text and images) and standard input (`-`) specs.
- [x] Hex Compare P0: the engine (None, Fast, Complete), `hikasa hex compare|dump|find`, the Hex Compare view.
- [x] Table Compare P0 (delimited and fixed widths) and TB-004 workbooks (xlsx, xlsm, xlsb, xls, ods; moved from M3,
  T-1): the engine, `hikasa table compare`, the grid view. *Open:* positive xls and xlsb fixtures (the formats are
  tested through xlsx's path and error cases; no approved writer makes them; `m2-table-contract.md` §10.5).
  *Deferred (D-59):* TB-001 HTML extraction.
- [x] Picture Compare P0: the engine, `hikasa picture compare`, the Picture Compare view. *Deferred (D-59):* PX-001 DDS.
- [x] Scripts: load/filter/expand/select/copy/move/delete/sync/compare/log/option/reports (`hikasa script run|check`,
  the window's status window); single-instance IPC (`RunScript`, sessions, workspaces, every view kind); the wait mode
  `hikasa gui open … --wait` and its manual topic for version control systems (CLI-010, `help/version-control.md`).
- [x] Reports in the window: one report dialog for every view kind, *View in Browser*, *Print Preview* (Q6).

Performance gates (all measured on the release code in W14, `docs/02` *M2 W14: the release run*; Linux desktop,
window rows under Xvfb with the software renderer):

- [x] G1 text (0.43 s through the binary, 303 MiB), G2 folder scan (0.049 s warm), G3 binary compare and CRC (1.59 ×,
  1.62 × the read rate), G4 cancel (≤ 85 ms), G5 GUI enablers, G6 archives, G9 table (0.74 s, 2.6 × input), G12 workspace,
  G13 reports (also from the window's dialog), G14 scripts (after W14's fix of the parse row, 2.17 → 0.11 s).
- [x] G8 editing: every row met, also the paste of 100 lines (worst 34.6–44.6 ms in four runs; above 50 ms only at load
  20–30 in W10).
- [x] G10 hex headless and in the window, except the row below; G11 picture headless and in the window, except the row
  below.
- [x] ARCH-GUI-003: `text_scroll`, `table_scroll`, `views_mixed`, `workspace_restore` met at the full 1,920 × 1,200 screen;
  `two_windows` and `text_edited` met at Xvfb's default screen.
- [ ] G7 copy ≤ 1.5 × a plain copy: 1.52–1.60 × on the loaded machine (the code of 2026-11-08, when it gave 1.41 ×, gives
  1.50–1.70 × today): to be measured on a quiet machine.
- [ ] G11 end to end for 16-bit RGBA pictures above about 100 MB: 472.5 MiB against 384.4 MiB (the decoder's 16-bit
  buffer; known limitation of 0.2.0).
- [ ] Software renderer at the full screen: G10's 1 GiB None → Fast alignment (+46 ms over its bound; renderer threads,
  W13c), `two_windows` frame work (p95 9.7–10.5 ms) and `text_edited` PSS (381.6 MiB in one run of two): the maintainer's
  GPU run decides them.
- G15 (folder view) and G16 (text editing in the window): not re-run in W14 (no code of theirs changed); last met at the
  W9c review and W11c (`docs/02`).

Release:
- [x] Version 0.2.0 in lockstep (workspace, lock, packaging), CHANGELOG entry.
- [x] `cargo fmt --check`, `cargo clippy -D warnings` (Linux and the Windows target), `cargo nextest run --workspace`,
  `cargo test --doc`, `cargo xtask commands-doc --check`, `cargo xtask help-check` green on Linux; `cargo deny check
  licenses bans sources` (as CI runs it) green.
- [ ] `cargo deny check advisories`: RUSTSEC-2026-0543 (`chrono` unmaintained, through `trash`'s `chrono` feature; no
  safe upgrade) needs the maintainer's decision (an ignore entry, or the recycle bin without that feature).
- [x] Every implemented command is in `reference/commands.md` and every subcommand, view and dialog has a help topic.
- [ ] Tagged release `v0.2.0` and its packages (the maintainer).
- [ ] CI green on Linux, Windows and macOS (after the maintainer's push).

For the maintainer (runs no agent can do; contract §1.14.6):
- [ ] The window tests (`gui/hikasa-gui/tests/cli_gui.rs`, `perf_gate.rs`) on Windows and macOS.
- [ ] The window gates on a real GPU: ≥ 60 fps in the scroll, zoom and pan phases, and G10's None → Fast alignment of
  the 1 GiB pair (not met under llvmpipe only).
- [ ] ARCH-GUI-003 on a 2020-class laptop (open since M1).
- [ ] The desktop checks: full screen and pointer capture of the picture view, real clipboards with images (A13.29),
  Wayland window positions and drags between windows, the IME, shortcut capture and the colour pickers (W4).
- [ ] The Windows packages on real Windows (tested under wine only).
- [ ] Positive xls and xlsb workbook fixtures (made by a spreadsheet application, committed as test data).
- [ ] RUSTSEC-2026-0543 (`chrono` through `trash`): an ignore entry in `deny.toml` or another way (above).
- [ ] G7 copy on a quiet machine (1.52–1.60 × on the loaded one; gate 1.5 ×).

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
