# Hikasa M2 — Implementation Plan (version 0.2)

> **Status: plan, not binding.** This file prepares milestone M2 (`docs/16-roadmap.md`, "Usable comparison
> tool"). It is written for the AI agents that implement M2 and for the maintainer who answers §4. The binding
> document is `docs/design/m2-contracts.md`, written next from this plan (outline in §6), in the style of
> `docs/design/m1-contracts.md`. Where this plan and the M2 contract disagree, the contract wins.
>
> Branch: `feat/m2` (from `main` at `v0.1.0`). Sources: `docs/00`–`17`, `docs/reference/commands.md`,
> `docs/design/m1-contracts.md` (decisions D-01…D-58, tenets T1–T11, §6 tests and waves), `CHANGELOG.md` and the
> code under `core/`, `cli/`, `gui/` at `v0.1.0`.

## 0. Rules every M2 agent follows

1. **Layering is binding** (`docs/17` §2.2b, ARCH-L1…L4, ARCH-012/013). Logic lives in `core/`. Every feature is
   a `hikasa` subcommand with `--json`, documented exit codes and tests **before** the GUI uses it. The GUI
   (`gui/hikasa-gui`, egui/eframe 0.37, D-56 as amended) calls core only through
   `hikasa_session::api::Service` and contains no comparison, file-operation, filter, script or settings logic.
2. **M1 contracts stay in force.** Tenets T1–T11 and decisions D-01…D-58 hold unless §5 of this plan lists them as
   revisited; a revision gets a new decision id (D-59 onwards) in the M2 contract, never an edit of the old row.
3. **Requirement ids.** Every PR/commit names the ids it implements (`Implements FC-070, FC-071`). Interpretations
   go into the owning `docs/*.md` in the same change (`docs/17` §1.2). New ids are added, never renumbered.
4. **Independence.** Do not name, copy or imitate any other comparison product, its help text, settings files,
   file extensions or binaries (`docs/17` §5). Hikasa's own extensions: `.hksnap`, `.hikasa-pkg`,
   `.hikasa-session`.
5. **Dependencies.** Rust only, permissive licences only (`docs/02`, `deny.toml`). Nothing outside
   `[workspace.dependencies]` is used until the maintainer approves it (§3) and `docs/02` records it. Features are
   fixed at workspace level (architecture check 3 refuses member-level `features`).
6. **Load limit (maintainer request).** At most **two implementer agents at a time**, on disjoint crates;
   reviewers run after the implementers, then one fixer. The full workspace checks (`fmt`, `clippy -D warnings`,
   `test --workspace`, `doc`, `deny`, the architecture test) run **once per wave**, at its end. No GitHub CI run
   unless the maintainer asks; window tests only under `env -u WAYLAND_DISPLAY xvfb-run -a` (`docs/17` §3.5).

---

## 1. Scope: P0 requirements in M2

M2 = "all P0 of FC, TC, TE, SES, UI, OPT" plus archives, clipboard/stdin, Hex/Table/Picture P0, scripts and the
IPC/wait mode (`docs/16`). The P0 ids of `docs/03`–`15` in these areas are **189**; eight architecture ids
(`docs/01`) are also touched. Status at `v0.1.0`:

- **done** — implemented and tested in M1 (engine, CLI and, where the requirement is visual, the GUI). M2 only
  keeps it working with the new features.
- **partial** — the engine/CLI part exists (M1 traceability, contract §7); M2 completes the rest (usually GUI,
  persistence or a sub-feature). The note says what is missing.
- **new** — nothing exists yet.
- **defer** — P0 text that depends on a P1 feature of M3; proposed for deferral in Q1 (§4).

"W" = the wave of §2 that does the work (core wave / GUI wave).

**Totals:** 38 done · 56 partial · 93 new · 2 deferred (plus sub-parts of partial ids, listed in Q1).

### 1.1 Sessions, workspaces, settings (`docs/03`) — 27 ids

| Id | Subject | Status | W | Missing |
|---|---|---|---|---|
| SES-001 | create session (Home, CLI, drop, child, saved) | new | 2 / 4 | session model and types |
| SES-002 | auto-save on close and periodically, day groups, limit 200 | new | 2 / 4 | |
| SES-003, SES-004, SES-005, SES-006 | save / save as, lock, "save changes?", session folders | new | 2 / 4 | |
| SES-008 | Clear Session, Swap Sides | new | 2 / 9, 11 | |
| SES-010 | display filter saved except for script loads | new | 2 / 8 | |
| SES-020, SES-022 | Session Settings dialog (tabs, Specs first); Rules / Format buttons | new | 4 (+9, 11, 13 tabs) | |
| SES-021 | scope selector (view only / update defaults / child overrides) | new | 2 / 4 | ARCH-041 semantics |
| SES-030 | Home view (type buttons, sessions tree, search, management panel) | partial | 4 | M1 Home has two path fields and Compare only |
| SES-031 | sessions tree context menu | new | 4 | |
| SES-040, SES-041, SES-042 | workspaces; load/save on start/exit; workspace on command line | new | 2 / 4 | |
| SES-050 | configuration folder, portable mode | done | — | |
| SES-051 | the file set (`options`, `state`, `commands`, `theme`, `sessions/`, `formats/`, `workspaces/`, `filters`, `openwith`, `archives`) | partial | 1, 2 | M1 reads `options.toml` (3 keys) and `formats/` only |
| SES-052 | `schema_version`, migration and write-back | partial | 1 | M1 refuses newer files; no migration, no writes |
| SES-053 | atomic writes; change detection with notice | partial | 1 | atomic writes done; detection new (Q4) |
| SES-060, SES-061, SES-062 | export / import settings packages (`.hikasa-pkg`), restore factory defaults | new | 2 / 4 | needs `zip` (Q9) |
| SES-090 | windows, tabs, drag between windows, move/duplicate/close others | partial | 4 | one window with tabs exists |
| SES-091 | tab and window options | new | 1 / 4 | taskbar previews deferred (Q1) |
| SES-092 | unsaved-edits prompt per file | new | 4 / 11 | |
| SES-093 | Home command, Sessions dropdown | new | 4 | |

### 1.2 Folder Compare and Folder Sync (`docs/04`) — 46 FC + 4 FS ids

| Id | Subject | Status | W | Missing |
|---|---|---|---|---|
| FC-001 | base-folder specs on any back-end | partial | 5 | local only; archives in W5; snapshot/remote are P1 (Q1) |
| FC-002 | path edits with completion, MRU, Browse, create missing | new | 1 (MRU) / 9 | browse dialog (Q5) |
| FC-003 | Disable editing per session | new | 6 / 9 | enforced by the Service, not the GUI |
| FC-004 | Up One Level, Set as Base, Compare Parent, Compare/Sync Base | new | 9 | merge entries deferred (Q1) |
| FC-010, FC-013, FC-014, FC-016 | alignment options, follow links, flatten, cancellable parallel scan | done | — | GUI glyphs/Path column in W9 |
| FC-012 | folder handling options and glyphs | partial | 6 / 9 | headless subset done; auto-scan/expand options, hollow/yellow/X/arrow glyphs |
| FC-015 | archives as files / folders once opened / always | new | 5 | |
| FC-020, FC-021, FC-022, FC-026, FC-027 | size, time, name case; skip-if-same; override | done | — | |
| FC-025 | CRC / binary / rules-based content tests | partial | 5 | archive-provided CRC (zip) |
| FC-029 | content compare on load / explicit Compare Contents / child load | partial | 6 / 9 | (a) done; (b) Compare Contents of a selection + results dialog; (c) child session |
| FC-030, FC-031, FC-032 | states, colours, centre icons, Legend | partial | 4 / 9 | colours configurable (theme), Legend window, direction arrows |
| FC-033 | "Ignored" marks persisted in the session | new | 6 / 9 | |
| FC-040, FC-041 | columns (Ext, Attributes, CRC on demand, Owner, Group, Path), sort, `(r)(s)(h)` | partial | 6 / 9 | GUI shows name, size, time only |
| FC-050 | Filters toolbar edit, presets, four lists, Add To Presets | partial | 1 / 9 | engine and CLI done; presets file and GUI |
| FC-051, FC-054, FC-055, FC-056, FC-057 | mask syntax, teal counterpart, display filters, folder modes, Ignore Unimportant / Suppress Filters | done | — | |
| FC-052 | Exclude action | new | 6 / 9 | |
| FC-060 | selection model, Select Newer/Orphans/Invert, expand commands | partial | 6 / 9 | one-row selection exists; selection engine shared with script `select` |
| FC-061 | next/previous difference, Find Filename | partial | 6 / 9 | Find Filename new |
| FC-062 | Open, Open With, Quick Compare, Compare To | partial | 1, 6 / 9 | Open done; Open With (`openwith.toml`), Quick Compare dialog, Compare To |
| FC-063 | file-manager menu, Copy Filename | new | 9 | needs `open` (Q9) |
| FC-064 | status-bar counts, Folder Compare Info | partial | 9 | statistics engine done |
| FC-070, FC-071, FC-072, FC-073, FC-074, FC-076, FC-077 | copy, move/exchange, copy/move to folder, delete (+recycle bin), rename (masks/regex), touch, new folder | new | 6 / 9 | ACL / 8.3 options deferred (Q1) |
| FC-078 | Synchronize submenu with preview | new | 6 / 9 | |
| FC-080 | drag and drop (OS → pane, pane ↔ pane, Home buttons) | new | 9 | |
| FC-081 | touch local files after FTP upload | defer | — | remote back-ends are P1 |
| FC-100 | log panel | new | 6 / 4, 9 | operation log model + panel |
| FS-001, FS-002, FS-004, FS-005 | Folder Sync view, custom matrix, per-item overrides, Run Sync | new | 6 / 9 | Q2 (FS is not named in the roadmap line) |

### 1.3 Text Compare and Text Edit (`docs/05`) — 33 TC + 1 TE ids

| Id | Subject | Status | W | Missing |
|---|---|---|---|---|
| TC-001 | document model, encoding and line endings preserved | partial | 10 / 11 | per-pane encoding override in the GUI; preservation on save |
| TC-002 | load from local, archive, clipboard, stdin (remote/HTTP P1) | partial | 3, 5 | archive members, clipboard |
| TC-003 | binary warning "edit anyway?", characters-per-line limit | partial | 11 | the prompt |
| TC-004 | when editing is disabled | new | 10 / 11 | |
| TC-005 | Save / Save As / backups / `--savetarget` / format save options | new | 10 / 11 | "Using Profile" is P1 |
| TC-010, TC-011, TC-012, TC-014, TC-021, TC-022 | engines, skew/closeness/never-align, important text + weights, inline diff, orphans important, line endings | done | — | Rules tab in W11 |
| TC-013 | Align With / Isolate (in-view; saved anchors are P1) | new | 10 / 11 | anchors in `hikasa-diff` |
| TC-015 | merge conflict definition | defer | — | Text Merge is P1 (M3) |
| TC-020 | Importance tab (unimportant items, element checklist) | partial | 11 | engine done; GUI tab |
| TC-024 | "Ignored" lines; Ignore Unimportant Differences | partial | 10 / 11 | Ignore Unimportant done; Ignored marks new |
| TC-030, TC-031, TC-032, TC-034, TC-035, TC-037 | panes, colouring, thumbnail, display options, File Info header, status bar | partial | 4, 11 | side-by-side panes, thumbnail, gap bars, font size done; over-under, gutter buttons, word wrap, header, status bar, theme colours |
| TC-033 | Details panel (text, hex, alignment), ruler | new | 10 / 11 | |
| TC-040 | display filters | done | — | merge filters deferred with TM (Q1) |
| TC-050 | navigation set, Go To, bookmarks, browse mode | partial | 10 / 11 | section navigation done; merge entries deferred |
| TC-051 | next-difference options | new | 1 / 11 | |
| TC-052 | Next/Previous Difference Files, Copy File and Open Next | new | 2, 6 / 11 | child-session link to the parent folder session |
| TC-053 | pane navigation | new | 11 | merge entries deferred |
| TC-060, TC-061, TC-062 | full editor, incremental recompare, copy section/line | new | 10 / 11 | |
| TC-063 | Convert File (trim, tabs, line endings) | new | 10 / 11 | + `hikasa text convert` |
| TC-064 | Find/Replace, Find in Files, Compare Selection to Clipboard | new | 10 / 11 | + `hikasa text find` |
| TC-065 | Open Clipboard | new | 3 / 11 | |
| TE-001 | Text Edit view (webpages panel is P2) | new | 10 / 11 | |

### 1.4 UI, commands, theming (`docs/14`) and options (`docs/15`) — 15 UI ids + OPT

| Id | Subject | Status | W | Missing |
|---|---|---|---|---|
| UI-001, UI-002 | window model, menus per view type | partial | 4 | Tools/Help menus, per-view menus, layout memory |
| UI-003 | catalogue + shortcut/menu/toolbar customization | partial | 0, 1 / 4 | catalogue exists (in the GUI crate); customization new |
| UI-004, UI-005 | toolbar right-click menu; pane context menus | new | 4 / 9, 11 | |
| UI-006, UI-007 | keyboard, F1 help; hints with shortcut and timing | partial | 0 / 4 | help topics, timing options |
| UI-008 | drag and drop from the OS | new | 4 / 9 | |
| UI-010 | HiDPI, icon sets | partial | 4 | egui scales; icons new (in-house vector icons) |
| UI-020, UI-021, UI-022 | theme mode, theme TOML files, folder/file view elements, live preview | partial | 1 / 4 | light/dark follow the OS with hard-coded palettes |
| UI-023 | Picture Compare colours | new | 4 / 13 | |
| UI-030 | status bar, job progress, message panel | partial | 4 | |
| UI-031 | non-modal notifications | new | 4 | "update available" deferred (Q1) |
| OPT (no ids) | Options dialog pages of `docs/15` | partial | 1 / 4 | 3 keys read in M1; Q3 proposes ids OPT-001… |

### 1.5 Table, Hex, Picture (`docs/06`–`08`) — 11 TB + 7 HX + 7 PX ids

| Id | Subject | Status | W |
|---|---|---|---|
| TB-001, TB-002, TB-003, TB-005 | table formats (delimited/fixed/detect, regional), first line, typed cells | new | 12 / 13 |
| TB-011, TB-012, TB-013 | column alignment and handling (key, unimportant, tolerances), row alignment | new | 12 / 13 |
| TB-020, TB-021, TB-022, TB-024 | grid, display filters, navigation, status/Info | new | 12 / 13 |
| HX-001, HX-002, HX-003, HX-004, HX-005, HX-007 | lazy hex panes, None/Fast/Complete alignment, hex formats, colours, filters/navigation/find, value panel | new | 12 / 13 |
| HX-009 | binary quick compare shares the chunked code | partial | 12 (reuse `hikasa_vfs::compare`) |
| PX-001 … PX-004, PX-007, PX-008, PX-009 | decoders, three panes, tolerance/range/blend, geometry and offset, checkerboard, status/Info, clipboard | new | 12 / 13 |

Spreadsheet and HTML-table extraction (TB-001 conversion kinds, TB-004) and DDS decoding (PX-001) are Q1 items.

### 1.6 VFS, CLI, scripts, reports, file formats (`docs/10`–`13`) — 4 VFS + 10 CLI + 3 SCR + 6 RPT + 15 FF ids

| Id | Subject | Status | W | Missing |
|---|---|---|---|---|
| VFS-001, VFS-010, VFS-020 | Vfs trait and capabilities; spec parsing; CRC primitive | done | — | archive CRC in W5 |
| VFS-002 | archive back-end (zip, tar, gz, bz2, xz, zst; nested; temp cache; rebuild on write; type masks) | new | 5 | Q8, Q9 |
| clipboard and stdin back-ends (`docs/11` table, P0, no id) | | stdin done, clipboard new | 3 | `arboard` (Q9) |
| CLI-000, CLI-001, CLI-004, CLI-G01 | global flags/`--json`, switch spellings, exit codes, `gui open --wait` | done | every wave | each new subcommand must comply; 105/106/107 used from W8 |
| CLI-002 | positional forms | partial | 2, 8 | session name, workspace name, `@script`, settings package; merge/patch/snapshot forms stay P1 |
| CLI-003 | legacy switches | partial | 6, 8, 10 | `--sync`, `--edit`, `--ro*`, `--savetarget`, `--nobackups`, `--silent`, `--closescript`; merge switches stay `switch_ignored` |
| CLI-005 | Quick Compare dialog on startup | new | 13 | |
| CLI-010 | wait-mode documentation for version control | partial | 14 | user manual topic |
| CLI-G02 | `gui session`, `gui workspace`, `gui script` | partial | 2, 8 | M1 has `gui list/close/ping/quit` |
| CLI-G03 | `--gui` on every headless compare | partial | 6, 12 | text and folder done |
| SCR-001 | report option vocabulary | partial | 7 | display words done |
| SCR-002, SCR-003 | script semantics; script errors (105/106/107) | new | 8 | |
| RPT-001 | Report command per session type, script commands, file-report links | partial | 7 / 13 | |
| RPT-002 | outputs: printer, HTML/text to file or clipboard, XML dump, patch | partial | 7 | XML dumps, clipboard, printer (Q6) |
| RPT-003 | report dialog (remembers last choices) | new | 7 / 13 | |
| RPT-004, RPT-010, RPT-011 | header, HTML colour semantics, headless identical output | done | — | extended to new types |
| FF-001 | File Formats dialog | partial | 1 / 11 | registry done; `format new/edit/delete/import/export`; dialog |
| FF-002 | folder sessions override enabled formats | new | 6 / 9 | |
| FF-003 | per-pane format pick, session Format tab | partial | 10 / 11 | revisits D-15 (§5) |
| FF-004 | one TOML per format, first-run materialization, Restore, change detection | partial | 1 | detection, restore |
| FF-020, FF-021, FF-022, FF-025, FF-026, FF-040 | grammar items, tokenizer, presets | done | — | |
| FF-027 | tokens feed colouring, importance, status | partial | 11 | element-under-cursor status; the third-party highlighter fallback is not adopted (Q1) |
| FF-028 | regex insert helper | new | 11 | |
| FF-030, FF-031 | conversions; read-only "Converted" | partial | 11 | None/Sorted/External done; built-in document converters deferred (Q1) |
| FF-032 | encodings | partial | — | DOS code pages other than IBM866 stay unsupported (M1 Appendix C 22) |

### 1.7 Architecture ids touched

| Id | Status | W | Note |
|---|---|---|---|
| ARCH-021 nested archive members | new | 5 | `Spec::ArchiveMember` is reserved since M1 |
| ARCH-031 generic cross-back-end copy | new | 3 | |
| ARCH-040, ARCH-041 defaults sessions, settings scope | new | 2 | |
| ARCH-061 comparison priority | gap | Q10 | |
| ARCH-070 command catalogue | partial | 0 | moves to core (Q7) |
| ARCH-080 rayon + tokio | partial | — | tokio stays deferred (no remote I/O in M2) |
| ARCH-091 `LoadSession`, `LoadWorkspace`, `RunScript` | partial | 2, 8 | additive to IPC protocol v1 (D-46) |

### 1.8 Ambiguities and contradictions found in `docs/03`–`15`

| # | Where | Problem | Proposed reading |
|---|---|---|---|
| A1 | `docs/16` M2 line, `docs/15` | "all P0 of … OPT", but `docs/15` has no ids and no priorities | Q3: add OPT ids; P0 = every page except items marked P1/P2 and the P1 dialogs (Check for Updates) |
| A2 | `docs/16` vs `docs/04` §10 | M2 names "FC" and "sync submenu"; FS-001/002/004/005 are P0 Folder Sync; M3 names "Folder Sync with Sync Memory" (FS-003, P1) | Q2: FS P0 in M2 |
| A3 | TC-015, TC-040, TC-050, TC-052, TC-053, FC-004 | P0 text with Text/Folder Merge entries, while TM-*/FM-* are P1 (M3) | merge entries deferred with TM/FM |
| A4 | FC-001, FC-081, TC-002, TC-005 | P0 text that needs snapshots, remote back-ends or profiles (P1) | those parts deferred |
| A5 | FC-053 (P1) vs `docs/16` "other filters" and the P0 script `filter attrib:` / `filter unixtype:` | size/date/protected exist since M1; attribute and Unix-type filters are needed by SCR | implement attribute and Unix-type filters in W6; the content filter stays P1 |
| A6 | FF-030 (P0) vs `docs/16` M3 | built-in spreadsheet/word-processor/RTF converters are P0 in `docs/10` but M3 in the roadmap | Q1: defer; External conversion covers them |
| A7 | TB-001 vs TB-004 | TB-001 lists spreadsheet/HTML extraction; TB-004 (all sheets) is P1; roadmap says "delimited/fixed" | M2: plain text and External conversion only |
| A8 | PX-001 | `image` 0.25 no longer decodes DDS | Q1: DDS compared by bytes in M2 |
| A9 | RPT-002 | "Printer" output (P0); no permissive, maintained cross-platform printing crate | Q6 |
| A10 | RPT-001 (P0) vs TB-025, HX-008, PX-010 (P1) | every session type needs a Report command, but the table/hex/picture layouts are P1 | M2: the Summary layout for table, hex and picture; side-by-side/interleaved P1 |
| A11 | SES-053 "(file watch)" vs ARCH-083 (P1, `notify`) | detection is P0, the watcher crate P1 | Q4 |
| A12 | UI-031 "update available", Tweaks "Check for Updates" | needs an HTTPS client (not approved); the dialog is P1 | deferred (Q1) |
| A13 | `docs/17` §2.7 vs ARCH-006 (P1) | "all user-visible text goes through the localization table" vs localization P1 | Q13 |
| A14 | `docs/16` DoD "every new command … has a help topic" | GUI command ids (hundreds) or CLI subcommands? | Q13 |
| A15 | `docs/12` §1 `hikasa settings` = "program options" | `commands.toml`, `theme.toml`, `filters.toml`, `openwith.toml`, `archives.toml` have no CLI surface | `settings get/set/list` address every settings file by a dotted key (`commands.text.CopyToRight.shortcut`) |
| A16 | TB-001 "Regional: use system" | no portable std API for the system decimal/thousands separators and date order | the host captures the locale name; a small in-house table maps it; unknown → `.` / `,` / ISO dates |
| A17 | SES-040 window geometry | Wayland does not let clients place windows | sizes restored everywhere, positions where the platform allows |
| A18 | FC-070 NTFS ACLs, 8.3 aliases, creation dates | Windows APIs without an approved safe crate | Q1: defer ACL and 8.3; creation date where std exposes it |
| A19 | FF-027 "syntect fallback" | `docs/02` adopts `syntect`, M1 never added it; highlighting comes from Hikasa grammars | Q1: drop the fallback for M2 |
| A20 | SES-030 "Web Resources panel" | links to documentation and community; no site exists yet | links to the repository's docs; panel hidden by default |

---

## 2. Waves

Fifteen waves (0–14). Each wave: one implementer per crate (at most two at once, disjoint crates), then
reviewers, then one fixer; full checks once at the end. A wave is **done** when its acceptance list passes, its
doc updates are in, `docs/reference/commands.md` is regenerated (from W0) and every new subcommand has a help
topic. Sizes: S ≈ M1 wave 1, M ≈ a single-crate M1 wave, L ≈ M1 wave 3 or 6b.

| Wave | Title | Crates | May run beside | Size |
|---|---|---|---|---|
| 0 | Infrastructure: catalogue in core, xtask, help, CI config | session, cli, gui, `xtask/` | — | M |
| 1 | Settings store | session, formats, cli | — | M |
| 2 | Sessions, workspaces, settings packages | session, cli, gui (IPC handler only) | — | L |
| 3 | VFS mutations, clipboard, generic copy | vfs, session, cli | 4 | M |
| 4 | GUI shell: windows, tabs, menus, shortcuts, theming, options, Home, sessions | gui | 3, then 5 | L |
| 5 | Archives | vfs, folder (FC-015), cli | 4 | L |
| 6 | Folder operations and sync engine | folder, session, cli | 7 | L |
| 7 | Reports: XML dumps, interleaved, file links, `report` subcommand | report, session, cli | 6 | M |
| 8 | Scripts | script, session, cli, gui (status window) | 9 | L |
| 9 | GUI Folder Compare and Folder Sync | gui | 8, then 10 | L |
| 10 | Text engine: editing, save, search, overrides | text, diff, session, cli | 9 | L |
| 11 | GUI Text Compare, Text Edit, File Formats dialog | gui | 12 | L |
| 12 | Table, Hex, Picture engines | table; hex + picture (second agent); report; session; cli | 11 | L |
| 13 | GUI Table/Hex/Picture views, report dialog, Quick Compare dialog | gui | — | L |
| 14 | Release hardening, gates, docs, 0.2.0 | all | — | M |

Order rationale: settings and sessions (W1–2) are the persistence every view needs; the catalogue move and the
help skeleton (W0) unblock shortcut customization, `commands-doc` and the DoD help rule; the GUI shell (W4) comes
early so each later GUI wave only adds views; every GUI wave follows the core wave it renders.

### Wave 0 — Infrastructure (M)

- **Ids:** ARCH-070, UI-003 (data), UI-006 (F1 mapping data); M1 leftovers: help topics, `cargo xtask
  commands-doc`, nextest, CI bench job (definition only).
- **Core:** move the command catalogue *data* from `gui/hikasa-gui/src/commands.rs` to
  `hikasa_session::commands` (id, scope/view type, menu, caption, hint, default shortcuts, menu/toolbar defaults,
  help topic id, status `implemented|planned`; Q7). Availability predicates stay in the GUI view models. Add
  `hikasa_session::help`: the topic registry over `help/*.md` (embedded with `include_str!`).
- **CLI:** `hikasa help [topic]` (text and `--json`, schema `hikasa.help.v1`; unknown topic exits 100 with
  suggestions); `hikasa help --list`.
- **GUI:** reads the catalogue from core; F1 resolves the focused command or view to its topic and shows it in a
  help panel (minimal in-house Markdown subset; Q13).
- **xtask:** new workspace member `xtask/` (binary, only approved third-party dependencies, alias in
  `.cargo/config.toml`): `cargo xtask commands-doc` (writes `docs/reference/commands.md` from the catalogue),
  `cargo xtask commands-doc --check` (fails when stale), `cargo xtask help-check` (every subcommand, view and
  dialog has a topic; every topic link resolves).
- **CI config (not run unless asked):** `cargo nextest run --workspace` replaces `cargo test` in `ci.yml` (doc
  tests stay on `cargo test --doc`); a `commands-doc --check` + `help-check` step; `bench.yml` (Q12).
- **Docs:** OPT ids into `docs/15` (Q3); the interpretations of §1.8 accepted by the maintainer into their docs;
  `docs/02` tooling; `help/` with topics for every M1 subcommand and the two M1 views.
- **Acceptance:** generated `commands.md` equals the hand-maintained file apart from the new status column (no
  command lost); a GUI test checks that every catalogue id the GUI executes exists in core and vice versa for
  `implemented` ids; architecture test extended to `xtask/` (no `unsafe`, manifest table) and to
  `hikasa_session::commands` (check 7: no vocabulary parsing in the GUI); `help-check` green.

### Wave 1 — Settings store (M)

- **Ids:** SES-051, SES-052, SES-053, FF-001 (CLI), FF-004, UI-003 (persistence), UI-020 (theme files), SES-041 and
  SES-091 (options), TC-051 (options), OPT pages (all keys, Q3).
- **Core (`hikasa-session::settings`):** typed, serde-backed models for every file of SES-051 except sessions and
  workspaces: `options.toml` (every page of `docs/15`), `state.toml` (MRU lists, window geometry, last session,
  first-run flags), `commands.toml` (per-view overrides of shortcuts/menu/toolbar, conflict detection),
  `theme.toml` + `themes/*.toml` (built-in Light and Dark; every element of UI-021/022/023), `filters.toml`,
  `openwith.toml` (variables `%f %l %n %p %x %b %F %P` and `1`/`2` suffixes; expansion is a pure function),
  `archives.toml`. One generic `SettingsFile<T>` handles `schema_version`, migration chain (older → current, in
  memory, written back on next save), refusal of newer files, atomic write, unknown-key preservation (Q11), and a
  change stamp (size + mtime + content hash) for detection (Q4). `format new|edit|delete|import|export` write paths
  (`FormatStore`, D-21: only explicit commands and GUI first start materialize).
- **CLI:** `hikasa settings get|set|list|restore-defaults` (dotted keys across all settings files; `--json`
  schemas `hikasa.settings_*.v1`), `hikasa format new|edit|delete|import|export`.
- **Acceptance:** TOML round-trip property test for every config type (T5); migration tests from a hand-written
  v0 fixture; newer-version refusal; unknown keys preserved byte-for-byte in value; atomic-write fault injection;
  change-detection test (external edit → `Changed` event); every `docs/15` key has a default, a type, a range and a
  test; `settings` CLI goldens.

### Wave 2 — Sessions, workspaces, settings packages (L)

- **Ids:** SES-001…006, SES-008, SES-010, SES-021, SES-040, SES-041, SES-042, SES-060, SES-061, SES-062,
  ARCH-040, ARCH-041, ARCH-091 (`LoadSession`, `LoadWorkspace`), CLI-002 (session/workspace/package forms),
  CLI-G02 (`gui session`, `gui workspace`), TC-052 (parent link data).
- **Core:** `Session<K>` per session type (Folder Compare, Folder Sync, Text Compare, Text Edit, Table, Hex,
  Picture; the P1 types are reserved ids) with `specs`, `rules`, `view_state`, `description`, `locked`, child
  overrides; defaults per type (`sessions/_defaults/<type>.toml`); named sessions in folders; auto-saved store
  grouped by day with the limit (the clock is injected, T1); scope resolution (view only / update defaults /
  these files / all files within the parent); `Workspace` (windows, tabs, active tab, per-view layout, geometry;
  restore loads tabs lazily). Settings packages: `.hikasa-pkg` = zip with `manifest.toml` + the TOML files;
  export by category, import plan (collisions, "delete existing" options, summary counts), restore factory
  defaults as an import from the built-ins.
- **CLI:** `hikasa session list|show|new|edit|delete|rename|lock|unlock|export|import|run` (`run` executes a saved
  Folder Compare/Text Compare/Folder Sync session headless; sync `run` waits for W6), `hikasa workspace
  list|show|delete|rename`, `hikasa settings export|import [--silent]`, `hikasa gui session <name> [--wait]`,
  `hikasa gui workspace <name>`; `gui open <session name>` / `<workspace name>` / `<package>` (D-51 parser).
- **GUI (IPC handler only):** answers `LoadSession` and `LoadWorkspace` by opening the stored views (rendering
  arrives with W4).
- **Acceptance:** session TOML goldens per type; round-trip properties; auto-save rotation table test (injected
  dates); lock semantics table (SES-004/005); scope table (SES-021); package export → import round trip;
  import plan goldens; IPC forward-compatibility goldens for the new requests; `session run` exit codes equal
  `folder compare`/`text compare` for the same inputs (T11).

### Wave 3 — VFS mutations, clipboard, generic copy (M)

- **Ids:** ARCH-031, VFS-001 (new capabilities), clipboard back-end, TC-002 (clipboard), TC-065 (data),
  FC-073 (recycle bin primitive), FC-076 (set times primitive).
- **Core:** extend `Vfs` with mutating operations behind capabilities: `mkdir`, `remove` (file / empty folder /
  recursive with per-item results), `rename`, `set_times`, `set_attrs`/`set_mode`, `move_to_trash` (local, via
  `trash`, Q9), `copy` (generic stream copy + metadata, local fast path). Writes never follow a symbolic link
  being removed. `MemoryVfs` implements all of them with fault injection. Clipboard: `hikasa_session::host`
  captures clipboard text/image through `arboard` into the existing `Blob` injection (M1 D-49 path); the VFS stays
  pure of OS clipboard calls.
- **CLI:** `hikasa vfs cp|mv|rm|mkdir|touch` (`--json`, per-item results, exit 0 or 100 when any item failed,
  ARCH-101); `clipboard://` accepted by `quick`, `text compare`, `vfs cat`; `gui open clipboard://` allowed (the
  GUI reads its own clipboard; D-49 revisited).
- **Acceptance:** mutation table tests on `MemoryVfs` and a tempdir (Linux, Windows, macOS paths), fault
  injection mid-copy leaves the target untouched; symlink safety tests; clipboard tests behind an injected
  provider (no real clipboard in CI).

### Wave 4 — GUI shell (L, beside W3 then W5)

- **Ids:** UI-001…008, UI-010, UI-020…022, UI-030, UI-031, SES-001…006 (GUI), SES-020…022 (dialog framework),
  SES-030, SES-031, SES-040, SES-041, SES-090, SES-091, SES-092 (framework), SES-093, SES-060…062 (wizards),
  FC-100 (panel), OPT pages (Options dialog), FC-031 (configurable colours).
- **GUI:** multiple windows as eframe viewports, tab bar with reorder, drag to another window, Move to New
  Window, Duplicate, Close Others; menus/toolbars/context menus/shortcuts generated from the core catalogue
  with `commands.toml` overrides; Options dialog with searchable page list (Apply/OK/Cancel) over the W1 models;
  "Toolbars, Shortcuts, Menus" page (view selector, search, conflicts, Default/Remove); theme mode, theme editor
  with live preview, in-house vector icon set (scale-free, UI-010); status bar with global job progress and
  cancel, message panel, non-modal notifications (settings changed on disk, files changed); Home view (type
  buttons as drop targets, sessions tree with folders/auto-saved/defaults, search, management panel, context
  menu); Session Settings dialog shell (Specs tab, scope footer); workspaces (save/load/manage, startup and exit
  options); Export/Import/Restore wizards; Log panel bound to the session log stream; drag and drop from the OS.
- **Acceptance:** view-model tests through the catalogue (`docs/17` §3.4) for every new command; shortcut
  conflict tests; theme round trip; ARCH-GUI-003 re-measured (unchanged gate) plus: empty window with Options
  open < 120 MiB PSS, two windows with one 1M-line comparison < 400 MB, workspace of 10 tabs shows the first
  screen < 1 s (lazy restore). Window tests under Xvfb only.

### Wave 5 — Archives (L, beside W4)

- **Ids:** VFS-002, ARCH-021, FC-001 (archive sides), FC-015, FC-025 (archive CRC), TC-002 (archive members),
  archive type masks (`archives.toml`, OPT "Archive Types").
- **Core:** `ArchiveVfs` for zip, tar, tar.gz, tar.bz2, tar.xz, tar.zst (and single-file gz/bz2/xz/zst as a
  one-member archive); spec resolution of `…/a.zip/inner/x` by probing path prefixes against the archive masks
  (`Spec::ArchiveMember`, nesting); per-session temp cache with size accounting and cleanup; streaming read for
  tar family; zip CRC exposed as `Entry::crc32` and used by the CRC test as-is; writes by full rebuild to a temp
  file + rename (Q8), CRC updated; "new folder with archive extension" creates an empty archive. Archive entry
  times: DOS times in zip are local wall time — interpreted in the injected time zone (T1). Folder scan honours
  FC-015 handling modes; the zipper glyph flag in the model.
- **CLI:** `vfs ls|stat|cat` on members; `folder compare a.zip b/`; `quick`/`text compare` on members;
  `vfs cp x a.zip/dir/`.
- **Acceptance:** fixtures per format (generated in tests, never downloaded), nested `a.zip/b.tar.gz/c.txt`;
  corrupt/truncated archive → error rows, never panics (fuzz-style proptest on random bytes); path traversal
  members (`../`, absolute, drive letters) refused; zip bomb guard (declared vs streamed size, ratio limit); rebuild
  atomicity under fault injection; gates G6 (§5.2).

### Wave 6 — Folder operations and sync engine (L, beside W7)

- **Ids:** FC-003, FC-012 (options), FC-029 (b, c), FC-033, FC-040 (CRC on demand), FC-052, FC-060 (selection
  engine), FC-061 (find filename), FC-062 (Open With expansion, Compare To), FC-070…FC-074, FC-076…FC-078, FC-100
  (operation log model), FS-001, FS-002, FS-004, FS-005, FF-002, CLI-003 (`--sync`), CLI-G03 (`folder sync
  --gui`), A5 (attribute and Unix-type filters).
- **Core (`hikasa-folder::ops`, `::sync`, `::select`):** every operation is *plan → confirm → execute* (T12,
  §6): a serializable `OpPlan` (items, sides, overwrites, read-only/system/newer conflicts, backups, recycle) built
  from the model and the selection with "visible contents only" semantics, confirmed by the adapter (prompt /
  yes-to-all / no-to-all as data), executed through the VFS with progress, cancellation, per-item results,
  continue-after-error, backups (`docs/15` naming patterns) and affected-subtree refresh. Rename with DOS masks and
  regex groups (`fancy-regex` only where backreferences are needed). Sync: the matrix of FS-002 with the five
  presets of FC-078/FS-001, per-item overrides, exclusions, "sync empty folders", "within archives". Selection
  engine = the script `select` vocabulary (one parser, T11). "Ignored" marks and Disable editing enforced by the
  Service.
- **CLI:** `hikasa folder op copy|move|delete|rename|touch|mkdir|copyto|moveto …` (`--select` masks, `--side`,
  `--recycle`, `--backup`, `--dry-run`, `--yes`); `hikasa folder sync <l> <r> (--update-left|…|--custom <matrix>)
  [--dry-run] [--create-empty] [--recycle]`; `--json` plans and results (`hikasa.folder_op.v1`,
  `hikasa.folder_sync.v1`).
- **Acceptance:** the dry-run plan equals the executed plan (property: executing a plan on `MemoryVfs` and
  re-comparing gives the predicted states); sync matrix table (8 situations × 5 actions); mirror never deletes
  outside the base folders; read-only/system/newer confirmation tables; rename mask table; backup naming table;
  cross-device move = copy + delete with rollback of the copy on failure; gate G7.

### Wave 7 — Reports (M, beside W6)

- **Ids:** RPT-001 (file-report links, script entry points), RPT-002 (XML dump for folder and text, clipboard
  target, printer per Q6), RPT-003 (report options model with remembered choices), SCR-001 (full vocabulary:
  columns, strike-out, patch styles, output options), TP-003 stays done.
- **Core:** XML dump layouts (documented schema in `docs/13`), text interleaved layout with strike-out options,
  folder side-by-side HTML with links to per-pair file reports (generated next to the folder report),
  `ReportRequest` that parses the SCR-001 vocabulary once (T11), clipboard output as a byte payload returned to the
  adapter.
- **CLI:** `hikasa report text|folder …` (the shared options; `--report` on compare subcommands stays the
  shortcut), `--output-to clipboard`.
- **Acceptance:** goldens for every new layout × colour/mono; XML well-formedness and schema test; links resolve;
  byte-identical output CLI vs Service (RPT-011).

### Wave 8 — Scripts (L, beside W9)

- **Ids:** SCR-001, SCR-002, SCR-003, CLI-002 (`@script`), CLI-003 (`--silent`, `--closescript`), CLI-004 (105,
  106, 107), CLI-G02 (`gui script`), ARCH-091 (`RunScript`), SES-010.
- **Core (`hikasa-script`):** lexer and parser (quoting, `&` continuation, `%1…%9`, `%NAME%`, `%date%`,
  `%time%`, `%fn_time%`, `lt`/`rt`), a checked AST (syntax errors before anything runs → 106), and an interpreter
  over an invisible Folder Compare session through the `Service` (load, filter, criteria, expand, collapse,
  select, compare, copy, move, copyto, moveto, delete, rename, touch, attrib, sync, option, log, beep, the report
  commands of W7 and W12). Shared scripts folder lookup. Confirmations follow `option confirm:`; silent mode
  logs refused dialogs as errors. `snapshot` is P1 (refused with a clear error).
- **CLI:** `hikasa script run <file> [args…] [--silent] [--stop-on-error]`, `hikasa script check <file>`.
- **GUI:** `gui script` and `@script` run with a minimal status window (log, progress, Stop; the full SCR-004
  window is P1).
- **Acceptance:** the four examples of `docs/12` §3.2 run as acceptance tests (remote spec in example 2 → 107,
  documented); parser goldens; exit-code table; property: a script's `copy`/`sync` gives the same result as the
  matching `folder op`/`folder sync` command (T11).

### Wave 9 — GUI Folder Compare and Folder Sync (L, beside W8, then W10)

- **Ids:** FC-002, FC-003, FC-004, FC-012 (glyphs), FC-013/014 (glyph, Path column), FC-029, FC-030 (Legend),
  FC-032, FC-033, FC-040, FC-041, FC-050 (Filters edit, presets, Name Filters tab), FC-052, FC-060…FC-064,
  FC-070…FC-078, FC-080, FC-100, FS-001, FS-002, FS-004, FS-005, FF-002, SES-008, SES-020 (folder tabs), UI-005.
- **GUI:** column set with sort, reorder, resize, persisted widths; path edits with completion (VFS listing) and
  MRU; in-house browse dialog over the VFS (Q5); selection model (side, both, swipe, Space mark); Find Filename
  panel; operation dialogs with preview lists that render core plans; Synchronize submenu and preview; Folder
  Sync view (preset dropdown, plan tree, overrides, Run); Legend; Info dialog; drag and drop; context menus;
  file-manager entries.
- **Acceptance:** catalogue-driven view-model tests for every folder command; dialog models render core plans
  unchanged (no recomputation in the GUI, architecture check 7); folder perf gate of M1 (100k files, UI frames
  < 16 ms) still met with all columns; ARCH-GUI-003 unchanged.

### Wave 10 — Text engine: editing, save, search, overrides (L, beside W9)

- **Ids:** TC-001 (save preservation), TC-004, TC-005, TC-013, TC-024 (Ignored), TC-033 (data), TC-050 (data),
  TC-052 (session link), TC-060…TC-064, TE-001 (model), FF-003 (per-pane format; D-15 revisited), CLI-003
  (`--edit`, `--ro*`, `--savetarget`, `--nobackups`).
- **Core:** `EditableDocument` (rope edits, line-table maintenance, edited/saved flags, unbounded per-pane undo
  with copy-section operations as one step, bookmarks), incremental recompare of the edited region with a
  debounce owned by the adapter (TC-061; the engine re-aligns the smallest enclosing range between unchanged
  anchors and splices rows; result identical to a full recompare — property), editability rules (TC-004), save
  through the VFS with encoding/BOM/line-ending preservation, format "when saving" options, backups, artificial
  CPL breaks removed; alignment anchors (Align With / Isolate) in `hikasa-diff`; conversions (TC-063); find /
  replace (plain, regex, whole word, case, sides) and Find in Files over the VFS (cancellable job).
- **CLI:** `hikasa text convert …`, `hikasa text find …` (`hikasa.text_convert.v1`, `hikasa.text_find.v1`);
  `text compare --align <l>:<r>` (anchors, for tests and scripts); `gui open --edit`.
- **Acceptance:** P-EDIT properties (edit + undo = identity; incremental = full recompare); save round trip for
  every encoding/BOM/EOL fixture; conversion goldens; find goldens; gate G8; G1/G5 unchanged.

### Wave 11 — GUI Text Compare, Text Edit, File Formats (L, beside W12)

- **Ids:** TC-001…TC-005, TC-013, TC-020, TC-024, TC-030…TC-037, TC-050…TC-053, TC-060…TC-065, TE-001, FF-001
  (dialog and per-kind editors), FF-003, FF-027, FF-028, FF-031, SES-020 (text tabs), SES-092, UI-005.
- **GUI:** editor in each pane (cursor, selection, insert/overwrite, clipboard, IME text input), gutter copy
  buttons, over-under layout, word wrap, details panel, ruler, File Info header (encoding, line ending, format
  pickers, Converted, Modified), status bar fields, find/replace panels, Find in Files results, bookmarks, Go To,
  Rules/Importance/Format tabs, the Text Edit view, the File Formats manager with grammar item editor and regex
  helper.
- **Acceptance:** catalogue-driven tests; typing latency: a keystroke in a 1M-line comparison repaints within one
  frame and the recompare lands within G8; ARCH-GUI-003 re-measured with an edited document and 1,000 undo steps
  (< 400 MB).

### Wave 12 — Table, Hex, Picture engines (L; two implementers: table | hex + picture)

- **Ids:** TB-001…TB-003, TB-005, TB-011…TB-013, TB-020/021/022/024 (models), HX-001…HX-005, HX-007, HX-009,
  PX-001…PX-004, PX-007…PX-009 (models), RPT-001/A10 (Summary layouts), CLI-G03, D-11 revisited.
- **Core:** `hikasa-table`: in-house delimited/fixed parser with sniffing, qualifiers, consecutive delimiters,
  typed cells (number, date via `jiff`, text) with regional settings, column alignment (by number / name /
  custom), key-based row join, unkeyed rows through `hikasa-diff` engines on row keys, tolerances, unimportant
  columns, statistics. `hikasa-hex`: lazy byte document over the VFS (mmap for local, temp cache otherwise), None /
  Fast (block hashes) / Complete (byte-level imara-diff on chunks with skew limit, budgeted, T6) alignment, value
  interpretation, find (text or hex bytes). `hikasa-picture`: decode through `image` to RGBA8 (tone-map HDR/EXR),
  tolerance / mismatch-range / blend difference images computed tile-wise in parallel (T2), geometry transforms
  and offset as view state, statistics. Folder content test of Table kind uses the table engine (TC-016
  principle); Picture and Hex kinds stay by bytes (D-11 revisited only for Table).
- **CLI:** `hikasa table compare`, `hikasa hex compare`, `hikasa picture compare [--diff-output x.png]`, each with
  `--json`, `--report summary`, `--gui`, exit codes per §6 (1/2/11/12/13 family).
- **Acceptance:** table parser table tests and proptests (quote/escape round trip), key-join goldens; hex
  alignment goldens and the shared chunk compare (HX-009 property: hex verdict = `quick --binary`); picture
  goldens on generated images (tolerance boundaries 0/1/255, transparency, size mismatch with Auto Scale); gates
  G9–G11.

### Wave 13 — GUI Table/Hex/Picture, report dialog, Quick Compare dialog (L)

- **Ids:** TB-020…TB-024 (views), HX-001…HX-007 (views), PX-002…PX-009 (views), UI-023, RPT-001, RPT-003, CLI-005,
  SES-020 (tabs of these types), FC-062 (Open With view types).
- **GUI:** virtualized grid (ARCH-GUI-002), hex panes and layout dialog, picture canvas with three panes, zoom,
  synchronized panning, checkerboard, offset dragging, clipboard copy; the report dialog over the W7 model
  ("View in Browser", "Print Preview" per Q6); Quick Compare dialog at startup.
- **Acceptance:** catalogue-driven tests; GUI gates: table 1M rows × 10 columns scroll ≥ 60 fps < 400 MB; hex
  4 GiB pair first screen < 1 s and scroll ≥ 60 fps; picture 2 × 12 MP + difference < 400 MB, zoom/pan ≥ 60 fps.

### Wave 14 — Release hardening (M)

- **Ids:** CLI-010 (manual topic), DoD items, every gate.
- **Work:** full help coverage (`help-check`), `commands-doc --check`, all gates G1–G11 and the GUI gates measured
  and recorded in the docs (desktop; the 2020-class laptop if the maintainer can provide one), Windows/macOS run
  of the window tests by the maintainer, CHANGELOG 0.2.0, version bump in lockstep, roadmap check-off.
- **Acceptance:** `docs/16` M2 items checked or explicitly deferred with the maintainer's answer to Q1.

---

## 3. New dependencies (approved by the maintainer on 2026-10-28, see §4.1)

Approved: every row below except `uzers` (Q10 = b, owner/group stay numeric). `xz` uses `lzma-rust2`, not
`liblzma` (Q9 = b). `zip` is at 9.x at the time of the answers (the "≥ 4" below is a floor, not the target).
Versions are the current major at the time of writing and must be re-checked when added (`docs/02` §1). Each
row needs a `docs/02` line, a `deny.toml` check and a `[workspace.dependencies]` entry with fixed features.
"In tree" means `Cargo.lock` already contains it through eframe at `v0.1.0`.

| Crate | Version | Licence | Used by | Why | Alternatives considered |
|---|---|---|---|---|---|
| `zip` | current major (≥ 4) | MIT | vfs, session (packages) | read/write zip, CRC per entry, settings packages | in-house zip reader (large, risky); features: deflate, deflate64, bzip2, zstd; no AES until the P1 password prompt |
| `tar` | 0.4 | MIT OR Apache-2.0 | vfs | read/write tar streams | in-house ustar/pax parser (moderate; pax and GNU long names are fiddly) |
| `flate2` | 1 | MIT OR Apache-2.0 | vfs | gzip streams (pure-Rust `miniz_oxide` back-end) | in tree already (via png); `zlib-rs` back-end later if faster |
| `bzip2` | 0.6 | MIT OR Apache-2.0 | vfs | `.bz2`; 0.6 uses a pure-Rust libbz2 port by default | `bzip2-rs` (decode only) |
| `lzma-rust2` | 0.24 | Apache-2.0 | vfs | `.xz` (and `.lzma`) read/write, pure Rust; also the back-end of `zip`'s `lzma`/`xz` features, so one LZMA codec in the tree (**chosen, Q9 = b**) | `liblzma` 0.4 (MIT OR Apache-2.0, bundled C library 0BSD; faster, but C code — rejected by the maintainer), `xz2` (unmaintained), `lzma-rs` (MIT, decode-oriented) |
| `zstd` | 0.13 | MIT (`zstd-sys` MIT OR Apache-2.0; bundled libzstd BSD-3-Clause) | vfs | `.zst` read/write; also the `.hksnap` codec in M3 | `ruzstd` (MIT, pure Rust; encoder limited to the fastest level) |
| `image` | 0.25 (in tree) | MIT OR Apache-2.0 (format crates: MIT/Apache, Zlib, BSD-3-Clause for `exr`) | picture | PX-001 decoders; png/jpeg/gif/bmp/ico/tiff/webp/tga/pnm/qoi/exr/hdr; `rayon` on, AVIF off | per-format crates by hand (more code, same licences); `zune-image` (less mature) |
| `arboard` | 3 (in tree) | MIT OR Apache-2.0 | session host, gui | clipboard text and images for `clipboard://`, TC-065, PX-009 (CLI too, not only the GUI) | egui's clipboard (GUI only, text only); `copypasta` (text only) |
| `trash` | 5 | MIT | vfs | recycle bin on Linux/Windows/macOS (FC-073, FS-005) | in-house freedesktop trash on Linux only; Windows/macOS would need `unsafe` FFI (forbidden) |
| `open` | 5 | MIT | session host | open with the associated application, file-manager reveal (FC-062, FC-063) | `std::process` with `xdg-open`/`open`/`explorer` (quoting pitfalls on Windows) |
| `ctrlc` | 3 | MIT OR Apache-2.0 | cli | graceful cancel of `script run`, `folder sync`, `folder op` (current file finished, plan stops; M1 Appendix C 15) | `signal-hook` (more general, Unix-centric); keep termination (atomic per file, but a sync stops mid-plan without a summary) |
| `thread-priority` | current | MIT OR Apache-2.0 (re-check) | session | ARCH-061 comparison priority on the rayon workers (Q10) | `libc` directly (needs `unsafe`, forbidden); keep the gap |
| `uzers` | 0.12 | MIT | vfs (Unix) | owner/group names for FC-040 (Q10, optional) | parse `/etc/passwd` and `/etc/group` (misses NSS/LDAP); keep numeric ids |

**Not proposed for M2** (decided in this plan unless the maintainer objects): `notify` (Q4: polling instead),
`rfd` (Q5: in-house browser), `csv` (in-house table parser: one parser for every TB-002 option and fixed widths,
one owner of behaviour, T11), `syntect`/`two-face` (A19), `tokio` (no remote I/O in M2), `calamine`,
`docx-rs`, `rtf-parser` (Q1), `kamadak-exif` (PX-006 is P1), Markdown rendering crates (Q13: in-house subset),
`sys-locale` (in tree; the host can read `LANG`/`LC_*` itself — revisit if Windows/macOS locale needs it).

---

## 4. Decisions needed from the maintainer

Each question has a recommended default; silence means the default.

1. **Deferrals to M3.** Defer: merge-only parts of TC-015/TC-040/TC-050/TC-052/TC-053/FC-004; FC-081 and the
   snapshot/remote/profile parts of FC-001/TC-002/TC-005; FF-030 built-in document converters; TB-001
   spreadsheet/HTML extraction; PX-001 DDS; FC-070 ACL and 8.3 options; SES-091 taskbar previews; UI-031 "update
   available"; FF-027 highlighter fallback. (a) defer all **[recommended]** (b) keep the items you name.
2. **Folder Sync P0 (FS-001/002/004/005) in M2?** (a) yes, with the sync engine of W6 **[recommended]** (b) M3
   together with Sync Memory.
3. **Options ids.** (a) add `OPT-001…` ids to `docs/15`, one per page/setting group, with priorities
   **[recommended]** (b) track options by `docs/15` headings only.
4. **Change detection (SES-053, FF-004).** (a) poll the config files' stamp on window focus and every 2 s, no new
   crate **[recommended]** (b) add `notify` + debouncer now (P1 features would reuse it).
5. **Browse dialogs.** (a) in-house VFS browser in egui only — also browses archives **[recommended]** (b) plus
   native dialogs through `rfd` (xdg-portal on Linux) (c) native only.
6. **Printing (RPT-002).** (a) Print / Print Preview = the self-contained HTML report opened in the default
   browser **[recommended]** (b) native printing (no suitable permissive crate; large) (c) no printer target in M2.
7. **Command catalogue.** (a) move the data to `hikasa_session::commands` with every command of
   `reference/commands.md` and a status `implemented|planned`; the generated file shows the status
   **[recommended]** (b) the catalogue lists implemented commands only and the generated file shrinks to them.
8. **Archive writes.** (a) read all P0 formats and write zip and the tar family by full rebuild (temp + rename)
   **[recommended]** (b) read-only archives in M2, writes in M3 (then Copy to Folder into an archive and New Folder
   with an archive extension wait too).
9. **Dependency set (§3).** Approve `zip`, `tar`, `flate2`, `bzip2`, `liblzma`, `zstd`, `image`, `arboard`,
   `trash`, `open`, `ctrlc`? (a) all, with `liblzma` (C library) for xz **[recommended]** (b) all, with the
   pure-Rust `lzma-rust2` for xz (c) strike items.
10. **OS facts (M1 Appendix C 13, 14).** (a) add `thread-priority` and `uzers` (b) add `thread-priority` only;
    owner/group stay numeric **[recommended]** (c) neither; ARCH-061 stays a gap.
11. **TOML written by Hikasa.** (a) program-owned files: unknown keys and sections are preserved, comments and
    layout are not **[recommended]** (b) preserve comments too (needs `toml_edit`).
12. **CI for M2 items.** (a) nextest and the `commands-doc`/`help-check` staleness steps in `ci.yml`; the bench
    job as a manual (`workflow_dispatch`) job on a GitHub-hosted runner with a 25 % tolerance, plus reference
    runs by hand **[recommended]** (b) a self-hosted fixed runner (c) benches stay manual only.
13. **Help topics and localization.** (a) Markdown topics in `help/`: one per CLI subcommand, view and dialog;
    F1 maps each command id to its view's or dialog's topic; English strings centralised in the catalogue and
    help, the localization table stays P1 (ARCH-006) **[recommended]** (b) one topic per GUI command id (c) build
    the localization table in M2.

### 4.1 Maintainer answers (2026-10-28)

The maintainer accepted every recommended default except Q9:

| Q | Answer | Consequence |
|---|---|---|
| 1 | a | all listed items deferred to M3; `docs/16` M3 list gains them in W0 |
| 2 | a | Folder Sync P0 (FS-001/002/004/005) in M2, engine in W6, view in W9 |
| 3 | a | `OPT-001…` ids added to `docs/15` in W0 |
| 4 | a | config change detection by polling (focus + every 2 s); no `notify` |
| 5 | a | in-house VFS browse dialog in egui; no `rfd` |
| 6 | a | Print / Print Preview = HTML report in the default browser |
| 7 | a | catalogue data in `hikasa_session::commands`, status `implemented` or `planned` |
| 8 | a | read every P0 archive format; write zip and the tar family by full rebuild |
| 9 | **b** | all crates of §3 approved, with the pure-Rust **`lzma-rust2`** for xz (no `liblzma`, no C code for LZMA) |
| 10 | b | `thread-priority` only; owner/group stay numeric; no `uzers` |
| 11 | a | unknown keys/sections preserved, comments and layout not; no `toml_edit` |
| 12 | a | nextest + `commands-doc --check`/`help-check` in `ci.yml`; `bench.yml` manual, 25 % tolerance |
| 13 | a | Markdown topics in `help/`, one per subcommand, view and dialog; localization table stays P1 |

These answers become decisions D-59 onwards in `docs/design/m2-contracts.md` §0.6.

---

## 5. Risks, performance gates, M1 decisions to revisit

### 5.1 Risks

| Risk | Mitigation |
|---|---|
| **GUI memory** grows with editors (undo), picture textures, more windows and restored workspaces; the 1M-line peak was already ≈ 365 MiB of 381.5 MiB | undo stores edits, not copies; tabs of a restored workspace load lazily; pictures upload tiles at the displayed zoom; ARCH-GUI-003 re-measured at the end of W4, W9, W11, W13, W14; a regression above the limit stops the wave and is reported (`docs/17` §7.1) |
| ARCH-GUI-003 never verified on a 2020-class laptop | W14 asks the maintainer for a laptop run; until then the desktop figures plus software-renderer frame times are recorded |
| **Data loss** in file operations, sync, archive rebuild, save | T12 plan → confirm → execute; atomic writes; backups; recycle bin; never follow a link being deleted; fault-injection tests on `MemoryVfs`; sync never touches paths outside the two bases; the dry-run plan is the executed plan |
| Multi-window on egui viewports: per-window GPU surfaces, Wayland cannot position windows, drag between windows | measure the second window's cost in W4; restore sizes everywhere and positions where allowed (A17); tab drag implemented as drop on another window's tab bar with "Move Tab to New Window" as fallback |
| Incremental recompare must equal full recompare (TC-061) | property test over random edit scripts; fall back to a full recompare when the dirty range exceeds a budget (T6) |
| Clipboard on headless Linux / Wayland for the CLI | clipboard errors are `Unsupported` with a clear message; tests use an injected provider |
| Archive timestamps (zip DOS time is local wall time) break determinism | interpret with the injected zone (T1); tests fix the zone |
| Malicious archives (traversal, bombs) | member path validation; size-ratio and absolute caps in the temp cache; tests |
| C-built dependencies (`liblzma`, `zstd-sys`) on Windows/macOS | maintainer runs the platform builds before W5 merges; `lzma-rust2`/`ruzstd` are the fallbacks |
| Scope: 93 new + 56 partial ids, GUI waves are large | GUI waves may split into a/b at the reviewer's request; P0 strictly before any P1 (`docs/17` §1.3) |
| Two writers of the configuration (CLI and GUI) | atomic writes + change stamps; the GUI reloads or asks; sessions are per-file so conflicts are per session |
| Architecture test drift (new crates, xtask, new allow-points) | each wave updates checks 2–4 and 7 in the same change; new allow-points only in `session::host` |

### 5.2 Performance gates for M2

All M1 gates stay: G1 (1M-line text < 3 s, RSS bound), G2 (100k-file scan < 5 s cold, < 1.5 s warm), G3 (binary
compare and CRC ≥ 90 % of read throughput), G4 (cancel ≤ 100 ms), G5 (GUI enablers), and the GUI gate
**ARCH-GUI-003: first screen < 1 s, ≥ 60 fps, < 400 MB** — re-measured with the new features in every GUI wave.
New gates (release build, headless unless stated; thresholds to be confirmed in the contract):

| Gate | Wave | Measure | Threshold |
|---|---|---|---|
| G6 archives | 5 | list a zip with 100,000 entries; folder compare of a 10,000-file tree against its zip (CRC) ; CRC of a 1 GB tar.gz member stream | < 1 s; < 2 s; ≤ 1.3 × the raw gzip decode time |
| G7 file operations | 6 | copy 10,000 small files local → local; build the sync plan of a compared 100k-file tree | ≤ 1.5 × a plain recursive copy; < 0.5 s |
| G8 editing | 10 / 11 | one-character edit in a 1M-line comparison: re-aligned rows ready; undo of 1,000 steps | < 50 ms; < 1 s; RSS growth < 10 MiB per 1,000 edits |
| G9 table | 12 / 13 | 1,000,000 rows × 10 columns keyed compare; grid scroll | < 5 s and ≤ 4 × input bytes; ≥ 60 fps < 400 MB |
| G10 hex | 12 / 13 | 4 GiB pair, None alignment: first screen; Fast alignment of 1 GiB; scroll | < 1 s; < 5 s; ≥ 60 fps |
| G11 picture | 12 / 13 | two 24 MP images, tolerance diff; GUI with 2 × 12 MP | < 0.5 s; < 400 MB, ≥ 60 fps while panning |
| G12 workspace | 4 | start with a workspace of 10 tabs (one 1M-line comparison) | first screen < 1 s; memory as for one view plus < 20 MiB per unopened tab |

### 5.3 M1 decisions and contract parts M2 must revisit

| M1 item | Why | M2 action |
|---|---|---|
| D-11 non-text kinds compared by bytes | Table/Hex/Picture engines exist | Table kind uses the table engine in rules-based tests; others stay bytes (new D) |
| D-15 one effective format per comparison (Appendix C 1) | FF-003 per-pane formats | keys from element names; a side without grammar is plain text (new D) |
| D-21 materialize only on GUI first start | `format` write commands arrive (W1) | write commands materialize the touched file only |
| D-31 `options.toml` read-only, unknown keys ignored | M2 writes it | preserve unknown keys (Q11), migration chain |
| D-49 `clipboard://` refused over IPC | `arboard` approved (Q9) | the GUI resolves `clipboard://` itself; stdin spooling unchanged |
| D-51 legacy parser | session, workspace, `@script`, package forms; new switches | extend the same parser; still one owner |
| D-57 `--criteria version` refused | FC-028 stays P1 | unchanged |
| §0.5 dependency table, architecture check 3 | new crates and `xtask` | new table in the M2 contract |
| Architecture check 4 (stubs export nothing) | table, hex, picture, script get APIs | only `hikasa-metadata` stays a stub |
| Architecture check 2 allow-points | auto-save timer, change polling, Ctrl-C handler | timers live in the adapters (cli/gui); `session::host` gains the clipboard and `open` calls; no new core allow-point |
| §1.10 catalogue in `hikasa-gui` | CLI and xtask need it (ARCH-070 "shared with the CLI") | moves to `hikasa-session` (Q7) |
| `Vfs` trait read-only plus `open_write` | file operations | mutating methods behind capabilities (W3) |
| `FolderModel` frozen after a run | operations change the tree | affected-subtree refresh (full FC-082 re-stat stays P1) |
| Appendix C 11, 13, 14, 15, 22 | clipboard, priority, owner names, Ctrl-C, DOS code pages | Q9, Q10; DOS code pages stay unsupported (FF-032 partial) |
| ARCH-061 gap | P0, Tweaks option | Q10 |

---

## 6. Outline of `docs/design/m2-contracts.md`

The contract keeps the M1 structure so agents find things in the same places. It **extends** the M1 contract:
anything not restated stays as in `m1-contracts.md`.

**§0 Ground rules.** M2 scope table (in/out per crate, the Q1 deferrals); tenets T1–T11 restated by reference,
plus **T12 Plan, confirm, execute** (every mutation — file operation, sync, archive rebuild, save, settings
import — is a serializable plan computed by core, confirmed by the adapter, executed by core with per-item
results; `--dry-run` prints the same plan) and **T13 Persisted state is versioned and program-owned**
(`SettingsFile<T>`, migrations, unknown-key preservation, change stamps); conventions; code rules (unchanged plus
`xtask`); §0.5 dependencies per crate with the approved crates and their fixed features; §0.6 decision register
D-59 onwards (every Q answer, every §1.8 interpretation, every §5.3 revision); §0.7 dependency constraints that
remain; §0.8 budgets for G6–G12; §0.9 documentation updates per wave.

**§1 Crate contracts — per wave:**

| Wave | The contract must specify |
|---|---|
| 0 | `hikasa_session::commands` (`CommandId`, `ViewType`, `Menu`, `Shortcut`, `CommandInfo`, status, topic id; the full table as data); `hikasa_session::help` (`Topic`, lookup, list); `hikasa help` flags and `hikasa.help.v1`; the `xtask` commands and the exact `commands.md` layout; CI file changes |
| 1 | `SettingsFile<T>` (load, migrate, save, stamp, change check); the TOML schema of `options`, `state`, `commands`, `theme`/`themes`, `filters`, `openwith`, `archives` with every key, default and range; `%`-variable expansion for Open With; `settings` key grammar and JSON schemas; `format` write commands and their exit codes |
| 2 | `SessionKind` trait and the per-type `Specs`/`Rules`/`ViewState` structs; storage layout and naming rules; auto-save algorithm (day buckets from an injected date); lock and "save changes?" state machine; scope resolution table (ARCH-041); `Workspace` schema; package format (`manifest.toml`, categories, import plan, collision rules); IPC `LoadSession`/`LoadWorkspace` messages; CLI `session`/`workspace`/`settings export|import` flags, JSON schemas, exit codes |
| 3 | the mutating `Vfs` methods, `Capabilities` additions, error variants; generic copy algorithm and metadata application order; trash semantics; clipboard provider trait and host capture; `vfs cp|mv|rm|mkdir|touch` |
| 4 | the GUI's needs only (like M1 §1.10): Service calls per dialog and view, view-model rules tested through the catalogue, window/viewport model, theme application, icon set inventory, notification sources |
| 5 | `ArchiveVfs` (formats, masks, resolution algorithm of nested specs, temp cache policy and limits, rebuild protocol, CRC exposure, time-zone handling, safety checks); folder FC-015 modes |
| 6 | `OpPlan`/`OpItem`/`OpResult`, confirmation model, backup naming, rename mask algorithm, selection vocabulary (shared with scripts), sync matrix and presets, attribute and Unix-type filters, subtree refresh, `folder op`/`folder sync` flags and schemas, exit codes for partial failure |
| 7 | XML dump schemas, interleaved layout, file-report links, `ReportRequest` and the SCR-001 vocabulary parser, clipboard output, `report` subcommand |
| 8 | script grammar (EBNF), variable expansion, command semantics table (SCR-002), error and exit-code rules (SCR-003, 105/106/107), the interpreter's Service calls, IPC `RunScript`, status-window events |
| 9 | GUI needs for the folder views (plan rendering, browse dialog over the VFS, columns, selection) |
| 10 | `EditableDocument`, edit operations, undo model, incremental recompare algorithm and its equivalence property, anchors in `hikasa-diff`, save protocol, conversions, find/replace and Find in Files, per-pane format keys (D-15 revision) |
| 11 | GUI needs for the text views and the File Formats dialog |
| 12 | `hikasa-table` (parser, types, regional settings, alignment, key join, tolerances, result model), `hikasa-hex` (document, alignments with budgets, value panel), `hikasa-picture` (decode, normalization, difference modes, transforms, statistics); summary reports; CLI subcommands, JSON schemas, exit codes; D-11 revision |
| 13 | GUI needs for grid, hex and picture views, report dialog and Quick Compare dialog |
| 14 | release checklist |

**§2 Normative algorithms** (with test tables, as M1 §2): archive spec resolution; zip time interpretation;
sync plan from the matrix; operation conflict rules; rename masks; backup names; script lexing and expansion;
`select` mask semantics (folders match only if all contents match, `diff` excludes timestamp-only); delimited
sniffing; key join with duplicates; numeric/date tolerance; hex Fast alignment; picture tolerance and range
colouring; incremental recompare range selection; auto-save buckets; settings migration.

**§3 Cross-crate flows:** `folder sync --dry-run` → plan → execute; `script run` example 4; `gui session <name>
--wait`; archive member open in Text Compare; save with backup; settings import with collisions.

**§4 JSON shapes and IPC:** every new `hikasa.*.v1` schema with a golden; IPC protocol v1 additions
(`LoadSession`, `LoadWorkspace`, `RunScript`, view kinds Table/Hex/Picture/Text Edit/Folder Sync), forward
compatibility per D-46.

**§5 Concurrency, cancellation, memory:** file-operation jobs, archive temp cache, picture tiling, undo memory,
lazy workspace restore, timers in adapters.

**§6 Tests and waves:** fixtures (archives and images generated in tests), new property families (P-OPS, P-SYNC,
P-ARCH, P-EDIT, P-TABLE, P-HEX, P-PIC, P-SCRIPT, P-SET), architecture checks updated, gates G1–G12 and the GUI
gates, and the wave table of §2 with acceptance criteria A0.x…A14.x.

**§7 Requirement traceability:** every id of §1 with crate::module, verifying test and status.

**Appendices:** invariant catalogue additions; open questions left after §4 is answered.
