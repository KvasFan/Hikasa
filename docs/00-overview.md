# Hikasa — Product Overview

> Audience: AI coding agents and human contributors implementing Hikasa.
> Status: version 0.1.0 = milestone M1 (core engines, the `hikasa` command line, the
> egui GUI prototype `hikasa-gui`); version 0.1.91 = pre-release of 0.2.0 with M2 waves W0–W5. This document describes the whole product; what each milestone
> delivers, and what M1 still lacks, is in `16-roadmap.md`.

## 1. What Hikasa is

Hikasa is a cross-platform (Linux first, then Windows and macOS) desktop application
for **comparing, merging and synchronizing** folders and files, written in Rust.

It covers the whole comparison workflow in one tool:

1. **Folder level** — compare two folder trees (local, inside archives, on remote
   services, or saved snapshots), reconcile them with copy/move/delete/rename/touch
   operations, synchronize them with presets (update, mirror) and merge three folder
   trees into an output tree.
2. **File level** — specialized *views* for different kinds of content:
   text (2-way compare, 3-way merge, single-pane edit, patch viewer), tables
   (CSV/spreadsheet, cell-by-cell), hex (byte-by-byte), pictures (pixel-by-pixel),
   media tags, executable version info, and registry dumps.
3. **Automation** — a complete command-line interface (`hikasa`) that exposes every
   tool headless with exit codes and JSON output, connects to the GUI over IPC, runs a
   text script language, and produces HTML/text/XML/patch reports.

The code is organised in three layers — **core** libraries (all logic), the **CLI**
(every function as a subcommand) and the **GUI** (a thin front-end over core that the
CLI can drive) — see `01-architecture.md` §0.

The product targets developers, system administrators, QA engineers and anyone who
needs to understand *what changed* between two sets of data and to reconcile it.

## 2. Core concepts (vocabulary used across all documents)

| Term | Meaning |
|------|---------|
| **Session** | A comparison task: the specs (what to compare), the rules (how to compare) and view state (display filter, layout). Sessions can be named, saved, locked, organized in folders and reused. |
| **Session type** | One of: Folder Compare, Folder Merge, Folder Sync, Text Compare, Text Merge, Text Edit, Text Patch, Table Compare, Hex Compare, Picture Compare, Media Compare, Version Compare, Registry Compare. |
| **View** | An open session rendered in a window or a tab. The *Home* view is the launcher/session manager. |
| **Child session** | A file session spawned from a folder session (double-click on a file pair). It reports its result back to the parent. |
| **Spec** | The name of a folder or file on any supported storage: local path, archive member, remote URL, snapshot, clipboard, stdin, HTTP URL. |
| **Base folder** | The top-most folder of one side of a folder view. |
| **Side** | Left, Right, Center (3-way ancestor) and Output (merge target). |
| **Rules** | The session settings that control how a comparison is performed (criteria, importance, alignment, filters). |
| **File format** | A named collection of settings for a kind of file: file mask, view type, conversion (pre-load/pre-save transformation), grammar (syntax elements), tab stops, line weights, encoding defaults. |
| **Grammar** | The set of syntax elements (comment, string, keyword…) of a text file format, used for highlighting and to classify differences as important or unimportant. |
| **Important / unimportant difference** | A difference that matters (red) vs. one that the rules declare irrelevant (blue): whitespace, comments, case, replacements, tolerances. "Ignore Unimportant Differences" hides the latter. |
| **Orphan** | An item that exists on one side only. |
| **Section** | A contiguous range of lines (or rows/bytes) with differences. |
| **Display filter** | A view-level filter that shows/hides items by comparison result (Show All / Differences / Same / Orphans / Left Newer …). |
| **File filter** | Session-level inclusion/exclusion of items by name mask, date, size, attributes, content. |
| **Alignment** | The process of deciding which line/row/item on the left corresponds to which on the right. |
| **Skew tolerance** | How far (in lines) the aligner looks for a match. |
| **Replacement** | A pair "text on left ↔ text on right" declared unimportant (e.g. `apple` ↔ `orange`). |
| **Snapshot** | A compressed, read-only listing of a folder tree (names, sizes, timestamps, attributes, optional CRCs, optional version info, optional expanded archive contents) that can later be compared against a live folder. |
| **Profile** | Named connection settings and credentials for a remote service (FTP/FTPS/SFTP, WebDAV, S3-compatible object storage, cloud drives, Subversion). |
| **Workspace** | The set of all open windows and tabs with their sessions, positions and sizes, saved under a name. |
| **Thumbnail** | The one-pixel-per-line overview strip at the left of file views that shows the pattern of differences. |
| **Touch** | Setting a file's last-modified timestamp. |
| **Tolerance** | In table and picture compare: how much two cells/pixels may differ before the difference counts as important. |

## 3. Non-goals for the first major release

- No cloud-sync service of our own; remote storage support is client-only.
- No plugin ABI for native binaries in 1.0 (external conversion *programs* are supported
  through command lines; a Rust/WASM plugin API is a later milestone).
- No live Windows registry editing in 1.0 (registry *files* are supported; live
  registry is Windows-only and scheduled later).
- No source-control-system (MSSCCI) integration in 1.0.

## 4. Quality bar

- **Correctness first**: a diff tool that reports wrong results is worthless. Every
  comparison engine ships with property-based tests and golden tests.
- **Scale**: folder trees with 1,000,000 entries, text files with 1,000,000 lines and
  binary files of several GB must stay usable (streaming, memory-mapping, background
  work, virtualized rendering).
- **Responsiveness**: the UI thread never blocks on I/O or on comparison work. Every
  long operation is cancellable and shows progress.
- **Safety**: the application never writes to a file the user did not explicitly
  target; destructive operations show a confirmation unless the user disabled it, and
  backups are available for overwrites.
- **Portability**: Linux (X11/Wayland), Windows 10+, macOS 12+. Paths, line endings,
  encodings, timestamps and attributes are handled per platform.
- **Open source only**: every dependency is MIT, Apache-2.0, BSD, MPL-2.0, CC0,
  Unlicense, zlib or GPL/LGPL compatible with our own license (see `02-tech-stack.md`).

## 5. Document map

| File | Content |
|------|---------|
| `01-architecture.md` | Crate layout, layering, threading model, data flow, key abstractions |
| `02-tech-stack.md` | Evaluated crates, decisions, GUI toolkit discussion, license matrix |
| `03-sessions-settings-profiles.md` | Session model, storage, workspaces, program options model, import/export, profiles |
| `04-folder-views.md` | Folder Compare, Folder Merge, Folder Sync |
| `05-text-views.md` | Text Compare, Text Merge, Text Edit, Text Patch, alignment engines, importance |
| `06-table-compare.md` | Table Compare |
| `07-hex-compare.md` | Hex Compare |
| `08-picture-compare.md` | Picture Compare |
| `09-metadata-views.md` | Media Compare, Version Compare, Registry Compare |
| `10-file-formats.md` | File format model, grammar items, conversions, built-in formats |
| `11-vfs-remote-archives.md` | Virtual file system: local, archives, snapshots, FTP/SFTP/WebDAV/S3/cloud, clipboard, HTTP |
| `12-cli-and-scripting.md` | CLI subcommand surface (CLI-first contract), `hikasa gui` IPC client, exit codes, script language |
| `13-reports.md` | Report layouts and outputs |
| `14-ui-and-commands.md` | Window/tab model, Home view, menus, toolbars, display-filter modes, theming |
| `15-options-reference.md` | Every program option page and setting |
| `16-roadmap.md` | Milestones and acceptance criteria |
| `17-agent-workflow.md` | Rules for AI agents working on this repository |
| `reference/commands.md` | Complete command catalogue with default shortcuts |
| `design/m1-contracts.md` | Binding API contracts for milestone M1 |

## 6. Requirement notation

Requirements are written as `**ID** (priority): statement`.

- IDs are prefixed per area (`ARCH`, `SES`, `FC` folder compare, `FM` folder merge,
  `FS` folder sync, `TC` text compare, `TM` text merge, `TE` text edit, `TP` text
  patch, `TB` table, `HX` hex, `PX` picture, `MD` media, `VR` version, `RG` registry,
  `FF` file formats, `VFS`, `CLI`, `SCR` script, `RPT` report, `UI`, `OPT`).
- Priorities: **P0** = required for the first usable release (MVP), **P1** = required
  for 1.0, **P2** = planned after 1.0.
- A requirement is *done* only when it is implemented, covered by tests and documented
  in the user-facing help.
